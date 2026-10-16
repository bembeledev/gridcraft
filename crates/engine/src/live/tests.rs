//! The craft-live client against a small fake craft-live room (written from craft-live's
//! design document §4–§6 and §9): framing, the fragment, seeding, joining, converging, saving,
//! reseeding, a lost lease, close codes and reconnecting, viewers, closing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;

use serde_json::{Value as Json, json};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{ReadTxn, Transact};

use super::wire::Frame;
use super::*;
use crate::core::CellRef;

const API: &str = "https://live.test/api/v1";
const ROOM: &str = "nc:cloud.test:288";

fn config(user: &str, role: &str, seed: bool) -> LiveConfig {
    LiveConfig::from_json(&json!({
        "ws": "wss://live.test/ws/nc%3Acloud.test%3A288",
        "room": ROOM,
        "token": format!("tok-{user}"),
        "user": {"id": user, "name": format!("{user} name"), "color": "#2E9E5B"},
        "role": role,
        "seed": seed,
        "file": format!("{API}/rooms/nc%3Acloud.test%3A288/file"),
        "name": "Budget.xlsx",
        "format": "xlsx",
        "api": API,
        "lang": "en",
    }))
    .unwrap()
}

/// A workbook file: A1 "Item", B1 "Cost", B2 12.
fn budget_file(name: &str) -> Vec<u8> {
    let mut s = Session::new();
    s.new_workbook();
    for (c, v) in [("A1", "Item"), ("B1", "Cost"), ("B2", "12")] {
        s.execute("cell.set", json!({"cell": c, "input": v})).unwrap();
    }
    crate::io::save_bytes_as(&s.doc().unwrap().wb, name, crate::io::FileKind::Xlsx).unwrap()
}

fn input(s: &Session, uid: u64, cell: &str) -> String {
    let d = s.documents().iter().find(|d| d.uid == uid).unwrap();
    d.wb.sheets[0].input_text(CellRef::parse(cell).unwrap())
}

// ---------------------------------------------------------------- the fake room

struct Conn {
    client: usize,
    conn: u64,
    token: String,
    synced: bool,
    pending: Option<Vec<u8>>,
    editor: bool,
}

#[derive(Default)]
struct Room {
    doc: yrs::Doc,
    seeded: bool,
    dirty: bool,
    /// The token holding the seed lease, and the connection it's bound to once that connects.
    lease: Option<String>,
    lease_conn: Option<(usize, u64)>,
    conns: Vec<Conn>,
    /// (client, conn, message) to deliver.
    out: VecDeque<(usize, u64, Vec<u8>)>,
    next_save: u64,
    /// Uploads received: (file name header, bytes).
    uploads: Vec<(String, Vec<u8>)>,
    /// Answer uploads with this status.
    upload_status: u16,
    /// Answer `POST prepare` with this.
    prepare: Json,
    /// The file `GET` gives.
    file: Vec<u8>,
    controls_in: Vec<Json>,
    hellos: Vec<u64>,
}

impl Room {
    fn new(file: Vec<u8>, lease: Option<&str>) -> Room {
        Room { file, lease: lease.map(str::to_string), upload_status: 200, prepare: json!({"seed": false, "seeded": false}), ..Room::default() }
    }

    fn send(&mut self, i: usize, f: &Frame) {
        if let Some(c) = self.conns.get(i) {
            self.out.push_back((c.client, c.conn, f.encode()));
        }
    }

    fn broadcast(&mut self, f: &Frame, except: Option<usize>, synced_only: bool) {
        for i in 0..self.conns.len() {
            if Some(i) != except && (!synced_only || self.conns[i].synced) {
                self.send(i, f);
            }
        }
    }

    fn sv(&self) -> Vec<u8> {
        self.doc.transact().state_vector().encode_v1()
    }

    fn diff(&self, sv: &[u8]) -> Vec<u8> {
        let sv = yrs::StateVector::decode_v1(sv).unwrap_or_default();
        self.doc.transact().encode_diff_v1(&sv)
    }

    fn empty(&self) -> bool {
        self.doc.transact().state_vector().is_empty()
    }

    fn join(&mut self, client: usize, conn: u64, token: &str, editor: bool) {
        let seeder = self.lease.as_deref() == Some(token) && self.lease_conn.is_none_or(|c| c == (client, conn));
        if seeder {
            self.lease_conn = Some((client, conn));
        }
        let synced = self.seeded || seeder;
        self.conns.push(Conn { client, conn, token: token.into(), synced, pending: None, editor });
        let i = self.conns.len() - 1;
        let welcome = json!({"type": "welcome", "room": ROOM, "seeded": self.seeded, "seed": seeder && !self.seeded, "modified": self.dirty});
        self.send(i, &Frame::Control(welcome));
        if synced {
            let sv = self.sv();
            self.send(i, &Frame::Sync1(sv));
        }
        self.peers();
    }

    fn leave(&mut self, client: usize, conn: u64) {
        let Some(i) = self.conns.iter().position(|c| c.client == client && c.conn == conn) else { return };
        self.conns.remove(i);
        if self.lease_conn == Some((client, conn)) && !self.seeded {
            self.lease = None;
            self.lease_conn = None;
            self.broadcast(
                &Frame::Control(json!({"type": "reseed", "message": "The person loading the file left before it was ready."})),
                None,
                false,
            );
        }
        self.peers();
    }

    fn peers(&mut self) {
        for i in 0..self.conns.len() {
            let others: Vec<Json> = self
                .conns
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, c)| json!({"user": {"id": c.token}, "role": "edit", "kind": "web"}))
                .collect();
            self.send(i, &Frame::Control(json!({"type": "peers", "peers": others})));
        }
    }

    fn on_frame(&mut self, client: usize, conn: u64, data: &[u8]) {
        let Some(i) = self.conns.iter().position(|c| c.client == client && c.conn == conn) else { return };
        match Frame::decode(data).unwrap() {
            Frame::Sync1(sv) => {
                if self.conns[i].synced {
                    let d = self.diff(&sv);
                    self.send(i, &Frame::Sync2(d));
                } else {
                    self.conns[i].pending = Some(sv);
                }
            }
            Frame::Sync2(u) | Frame::Update(u) => {
                if !self.conns[i].editor {
                    return;
                }
                let was_empty = self.empty();
                self.doc.transact_mut().apply_update(yrs::Update::decode_v1(&u).unwrap()).unwrap();
                if !self.seeded {
                    if !was_empty || self.empty() {
                        return;
                    }
                    self.seeded = true;
                    self.lease = None;
                    self.lease_conn = None;
                    for j in 0..self.conns.len() {
                        if !self.conns[j].synced {
                            self.conns[j].synced = true;
                            let d = match self.conns[j].pending.take() {
                                Some(sv) => self.diff(&sv),
                                None => self.diff(&[0]),
                            };
                            self.send(j, &Frame::Sync2(d));
                            let sv = self.sv();
                            self.send(j, &Frame::Sync1(sv));
                        }
                    }
                    self.broadcast(&Frame::Control(json!({"type": "seeded", "modified": false})), None, false);
                    return;
                }
                if !self.dirty {
                    self.dirty = true;
                    self.broadcast(&Frame::Control(json!({"type": "modified", "modified": true})), None, false);
                }
                self.broadcast(&Frame::Update(u), Some(i), true);
            }
            Frame::Awareness(a) => self.broadcast(&Frame::Awareness(a), Some(i), false),
            Frame::Control(v) => {
                match v["type"].as_str() {
                    Some("hello") => self.hellos.push(v["clientId"].as_u64().unwrap_or(0)),
                    Some("saveNow") => self.ask_save(),
                    _ => {}
                }
                self.controls_in.push(v);
            }
            _ => {}
        }
    }

    fn ask_save(&mut self) {
        let Some(i) = self.conns.iter().position(|c| c.editor) else { return };
        self.next_save += 1;
        let id = self.next_save;
        let save = json!({"type": "save", "id": id, "mode": "upload", "name": "Budget.xlsx", "format": "xlsx",
            "url": format!("{API}/rooms/nc%3Acloud.test%3A288/file?save={id}"), "autosave": false});
        self.send(i, &Frame::Control(save));
    }

    /// An HTTP request from a client.
    fn http(&mut self, r: &HttpRequest) -> HttpResponse {
        let ok = |body: Vec<u8>| HttpResponse { status: 200, body };
        assert!(r.headers.iter().any(|(k, v)| k == "Authorization" && v.starts_with("Bearer tok-")), "{r:?}");
        match r.method {
            "GET" => ok(self.file.clone()),
            "PUT" => {
                let name = r.headers.iter().find(|(k, _)| k == "X-Craft-File-Name").map(|(_, v)| pct_decode(v)).unwrap_or_default();
                self.uploads.push((name, r.body.clone().unwrap_or_default()));
                if self.upload_status != 200 {
                    return HttpResponse { status: self.upload_status, body: br#"{"error":"Nextcloud said no"}"#.to_vec() };
                }
                self.dirty = false;
                self.broadcast(&Frame::Control(json!({"type": "saved", "at": 1, "modified": false})), None, false);
                ok(br#"{"saved":true,"version":"2"}"#.to_vec())
            }
            "POST" => {
                let p = self.prepare.clone();
                if p["seed"] == true {
                    let token = r.headers.iter().find(|(k, _)| k == "Authorization").map(|(_, v)| v.trim_start_matches("Bearer ").to_string());
                    self.lease = token;
                    self.lease_conn = None;
                }
                ok(p.to_string().into_bytes())
            }
            m => panic!("unexpected {m}"),
        }
    }
}

// ---------------------------------------------------------------- clients

struct Client {
    s: Session,
    live: LiveClient,
    conn: Option<u64>,
    shell: Vec<Json>,
    notices: Vec<String>,
    alerts: Vec<String>,
    sent: Vec<Frame>,
    disconnects: Vec<(u64, u16)>,
    /// Requests the test answers itself instead of the room.
    hold_http: bool,
    held: Vec<HttpRequest>,
}

struct World {
    room: Room,
    clients: Vec<Client>,
    now: u64,
}

impl World {
    fn new(room: Room) -> World {
        World { room, clients: vec![], now: 1_000 }
    }

    fn add(&mut self, cfg: LiveConfig) -> usize {
        let mut s = Session::new();
        let mut live = LiveClient::new(cfg);
        live.start(&mut s, self.now);
        self.clients.push(Client {
            s,
            live,
            conn: None,
            shell: vec![],
            notices: vec![],
            alerts: vec![],
            sent: vec![],
            disconnects: vec![],
            hold_http: false,
            held: vec![],
        });
        self.clients.len() - 1
    }

    /// Runs everyone until nothing moves.
    fn settle(&mut self) {
        for _ in 0..200 {
            let mut moved = false;
            for i in 0..self.clients.len() {
                moved |= self.step(i);
            }
            while let Some((ci, conn, data)) = self.room.out.pop_front() {
                moved = true;
                let c = &mut self.clients[ci];
                if c.conn == Some(conn) {
                    c.live.on_frame(&mut c.s, conn, &data);
                }
            }
            if !moved {
                return;
            }
        }
        panic!("never settled");
    }

    fn step(&mut self, i: usize) -> bool {
        let now = self.now;
        let c = &mut self.clients[i];
        c.live.poll(&mut c.s, now);
        let actions = c.live.take_actions();
        let moved = !actions.is_empty();
        for a in actions {
            let c = &mut self.clients[i];
            match a {
                Action::Connect { conn, url } => {
                    assert!(url.starts_with("wss://live.test/ws/") && url.contains("?token=tok-"), "{url}");
                    if let Some(old) = c.conn.take() {
                        self.room.leave(i, old);
                    }
                    c.conn = Some(conn);
                    let token = c.live.cfg.token.clone();
                    let editor = !c.live.cfg.view_only;
                    c.live.on_open(conn);
                    self.room.join(i, conn, &token, editor);
                }
                Action::Send { conn, frame } => {
                    if c.conn == Some(conn) {
                        c.sent.push(Frame::decode(&frame).unwrap());
                        self.room.on_frame(i, conn, &frame);
                    }
                }
                Action::Disconnect { conn, code, .. } => {
                    c.disconnects.push((conn, code));
                    if c.conn == Some(conn) {
                        c.conn = None;
                        self.room.leave(i, conn);
                    }
                }
                Action::Http(r) => {
                    if c.hold_http {
                        c.held.push(r);
                    } else {
                        let answer = self.room.http(&r);
                        let c = &mut self.clients[i];
                        c.live.on_http(&mut c.s, r.id, Ok(answer));
                    }
                }
                Action::Shell(m) => c.shell.push(m),
                Action::Notice { text, .. } => c.notices.push(text),
                Action::Alert(t) => c.alerts.push(t),
            }
        }
        moved
    }

    /// The server drops client `i`'s connection with `code`.
    fn drop_conn(&mut self, i: usize, code: u16) {
        let c = &mut self.clients[i];
        let conn = c.conn.take().unwrap();
        self.room.leave(i, conn);
        c.live.on_close(&mut c.s, conn, code, "");
    }

    fn c(&mut self, i: usize) -> &mut Client {
        &mut self.clients[i]
    }

    fn doc(&self, i: usize) -> u64 {
        self.clients[i].live.doc().unwrap()
    }

    fn fp(&self, i: usize) -> String {
        let c = &self.clients[i];
        let d = c.s.documents().iter().find(|d| Some(d.uid) == c.live.doc()).unwrap();
        crate::collab::fingerprint(&d.wb)
    }

    fn set(&mut self, i: usize, cell: &str, v: &str) {
        self.c(i).s.execute("cell.set", json!({"cell": cell, "input": v})).unwrap();
    }
}

fn shell_types(c: &Client) -> Vec<String> {
    c.shell.iter().map(|m| m["type"].as_str().unwrap_or("").to_string()).collect()
}

// ---------------------------------------------------------------- framing and configuration

#[test]
fn frames_round_trip() {
    let frames = [
        Frame::Sync1(vec![1, 2, 3]),
        Frame::Sync2(vec![]),
        Frame::Update(vec![9; 300]),
        Frame::Awareness(vec![1, 0, 0]),
        Frame::Denied("no".into()),
        Frame::QueryAwareness,
        Frame::Control(json!({"type": "hello", "clientId": 5, "can": ["upload"]})),
    ];
    for f in frames {
        assert_eq!(Frame::decode(&f.encode()), Ok(f.clone()));
    }
    // lib0: a 300-byte payload has a two-byte length; control is type 100 then a varstring.
    assert_eq!(&Frame::Update(vec![9; 300]).encode()[..4], &[0, 2, 0xAC, 0x02]);
    assert_eq!(Frame::Control(json!({"type": "bye"})).encode(), [&[100u8, 14][..], br#"{"type":"bye"}"#].concat());
}

#[test]
fn damaged_frames_are_errors_never_panics() {
    for bad in [&[][..], &[0], &[0, 0], &[0, 0, 5, 1], &[0, 7, 0], &[9], &[100, 3, b'[', b']', b' '], &[100, 2, b'{'], &[0xff; 12]] {
        assert!(Frame::decode(bad).is_err(), "{bad:?}");
    }
    let mut x: u64 = 0x1234_5678;
    for len in 0..400usize {
        let data: Vec<u8> = (0..len % 40)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect();
        let _ = Frame::decode(&data);
    }
    // A length larger than the message.
    assert!(Frame::decode(&[0, 1, 0xff, 0xff, 0xff, 0xff, 0x0f]).is_err());
}

#[test]
fn the_fragment_is_read_as_craft_live_writes_it() {
    // As craft-live's `embed_fragment` builds it: strings as they are, user as JSON, 1/0.
    let user = pct_encode(r##"{"id":"ada","name":"Ada Lovelace","color":"#d8553b"}"##);
    let frag = format!(
        "#craft-live=1&ws={}&room={}&token=abc123&user={user}&role=edit&seed=1&file={}&name={}&format=xlsx&api={}&lang=it",
        pct_encode("wss://live.example.com/ws/nc%3Acloud.example.com%3A288"),
        pct_encode("nc:cloud.example.com:288"),
        pct_encode("https://live.example.com/api/v1/rooms/nc%3Acloud.example.com%3A288/file"),
        pct_encode("Budget 2026.xlsx"),
        pct_encode("https://live.example.com/api/v1"),
    );
    let c = LiveConfig::from_fragment(&frag).unwrap().unwrap();
    assert_eq!(c.ws, "wss://live.example.com/ws/nc%3Acloud.example.com%3A288");
    assert_eq!(c.room, "nc:cloud.example.com:288");
    assert_eq!(c.user, User { id: "ada".into(), name: "Ada Lovelace".into(), color: "#D8553B".into() });
    assert!(c.seed && !c.view_only);
    assert_eq!((c.name.as_str(), c.format.as_str(), c.lang.as_str()), ("Budget 2026.xlsx", "xlsx", "it"));
    assert_eq!(c.ws_url(), "wss://live.example.com/ws/nc%3Acloud.example.com%3A288?token=abc123");
    // Not craft-live's, another version, unusable.
    assert!(LiveConfig::from_fragment("#sample=sales").is_none());
    assert!(LiveConfig::from_fragment("").is_none());
    assert!(LiveConfig::from_fragment("#craft-live=2&ws=wss://x").unwrap().is_err());
    assert!(LiveConfig::from_fragment("#craft-live=1&ws=https://x&room=r&token=t").unwrap().is_err());
    assert!(LiveConfig::from_fragment("#craft-live=1&ws=wss://x&room=r").unwrap().is_err());
    // A viewer never seeds; a bad user still gives a usable person.
    let v = LiveConfig::from_fragment("#craft-live=1&ws=wss://x/ws/r&room=r&token=t&role=view&seed=1&user=%7Bbroken").unwrap().unwrap();
    assert!(v.view_only && !v.seed);
    assert_eq!((v.user.id.as_str(), v.user.name.as_str(), v.user.color.as_str()), ("guest", "Guest", "#3B7DD8"));
    assert_eq!((v.name.as_str(), v.format.as_str()), ("Book.xlsx", "xlsx"));
    // The shell's config: the same object (`seed` a boolean, `user` an object).
    let j = LiveConfig::from_json(
        &json!({"ws": "ws://l/ws/r", "room": "r", "token": "t", "seed": false, "user": {"id": "u", "name": "N"}, "name": "a/b/Plan.ods"}),
    )
    .unwrap();
    assert_eq!((j.name.as_str(), j.format.as_str()), ("Plan.ods", "ods"));
}

// ---------------------------------------------------------------- seeding, joining, saving

#[test]
fn seed_join_converge_and_save() {
    let mut w = World::new(Room::new(budget_file("Budget.xlsx"), Some("tok-ada")));
    let ada = w.add(config("ada", "edit", true));
    // The seeder downloads the file first.
    assert_eq!(w.c(ada).live.phase(), Phase::Loading);
    w.settle();
    assert!(w.room.seeded, "the seed reached the room");
    assert_eq!(w.c(ada).live.phase(), Phase::Live);
    assert!(shell_types(w.c(ada)).contains(&"craft-live:loaded".to_string()));
    // hello first, with the Yjs client id, then sync step 1.
    let sent = &w.c(ada).sent;
    assert!(matches!(&sent[0], Frame::Control(v) if v["type"] == "hello" && v["can"] == json!(["upload"]) && v["clientId"].as_u64().unwrap() > 0));
    assert!(sent.iter().any(|f| matches!(f, Frame::Sync1(_))));
    assert_eq!(w.c(ada).s.documents().len(), 1);
    assert_eq!(w.c(ada).s.active().unwrap().title, "Budget.xlsx");
    let a_doc = w.doc(ada);
    assert_eq!(input(&w.c(ada).s, a_doc, "B2"), "12");

    let bob = w.add(config("bob", "edit", false));
    w.settle();
    let b_doc = w.doc(bob);
    assert_eq!(input(&w.c(bob).s, b_doc, "A1"), "Item");
    assert_eq!(w.fp(ada), w.fp(bob));
    assert!(shell_types(w.c(bob)).contains(&"craft-live:loaded".to_string()));
    assert_eq!(w.c(bob).s.active().unwrap().title, "Budget.xlsx");

    // One update per command, both ways.
    let before = w.c(ada).sent.iter().filter(|f| matches!(f, Frame::Update(_))).count();
    w.set(ada, "C3", "=B2*2");
    w.settle();
    assert_eq!(w.c(ada).sent.iter().filter(|f| matches!(f, Frame::Update(_))).count(), before + 1);
    assert_eq!(input(&w.c(bob).s, b_doc, "C3"), "=B2*2");
    w.set(bob, "A4", "Total");
    w.settle();
    assert_eq!(input(&w.c(ada).s, a_doc, "A4"), "Total");
    assert_eq!(w.fp(ada), w.fp(bob));
    // The room is modified; the shell hears it.
    assert!(w.c(bob).shell.iter().any(|m| m["type"] == "craft-live:modified" && m["modified"] == true));
    // Each knows the other through awareness (the title bar's circles).
    let peers = w.c(ada).s.active_room().unwrap().peers();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].user.name, "bob name");
    // craft-live:peers lists the others and yourself.
    let p = w.c(ada).shell.iter().rev().find(|m| m["type"] == "craft-live:peers").unwrap().clone();
    assert!(p["peers"].as_array().unwrap().iter().any(|x| x["self"] == true && x["user"]["id"] == "ada"));

    // Bob presses Save: the room asks an editor for the file, which uploads it.
    w.c(bob).live.save_now();
    w.settle();
    let (name, bytes) = w.room.uploads.last().unwrap().clone();
    assert_eq!(name, "Budget.xlsx");
    let (wb, _) = crate::io::open_bytes(&name, &bytes).unwrap();
    assert_eq!(wb.sheets[0].input_text(CellRef::parse("C3").unwrap()), "=B2*2");
    assert_eq!(wb.sheets[0].input_text(CellRef::parse("A4").unwrap()), "Total");
    assert!(w.c(bob).shell.iter().any(|m| m["type"] == "craft-live:saved" && m["success"] == true));
    assert!(w.c(ada).shell.iter().rev().find(|m| m["type"] == "craft-live:modified").is_some_and(|m| m["modified"] == false));
    // The saved room makes the workbook unmodified.
    assert!(!w.c(ada).s.active().unwrap().is_dirty());
    let lines = w.c(ada).live.summary();
    assert!(lines.iter().any(|l| l == "All changes saved in Nextcloud"), "{lines:?}");
}

#[test]
fn a_seeder_is_read_only_until_the_room_has_the_seed() {
    let mut w = World::new(Room::new(budget_file("Budget.xlsx"), Some("tok-ada")));
    let ada = w.add(config("ada", "edit", true));
    // The file arrives, the seed is built and the connection opens; the room hasn't answered.
    w.step(ada);
    w.step(ada);
    assert_eq!(w.c(ada).live.phase(), Phase::Live);
    assert_eq!(crate::collab::read_only(&w.c(ada).s).as_deref(), Some("Joining…"));
    w.settle();
    assert_eq!(crate::collab::read_only(&w.c(ada).s), None);
    assert!(w.room.seeded && !w.room.dirty);
    w.set(ada, "A2", "first edit");
    w.settle();
    assert!(w.room.dirty, "an edit after the seed is an unsaved change");
}

#[test]
fn a_joiner_waits_read_only_until_the_seed_arrives() {
    let mut w = World::new(Room::new(budget_file("Budget.xlsx"), Some("tok-ada")));
    let bob = w.add(config("bob", "edit", false));
    w.settle();
    assert_eq!(crate::collab::read_only(&w.c(bob).s).as_deref(), Some("Joining…"));
    assert!(w.c(bob).s.execute("cell.set", json!({"cell": "A1", "input": "x"})).is_err());
    assert!(!shell_types(w.c(bob)).contains(&"craft-live:loaded".to_string()));
    let ada = w.add(config("ada", "edit", true));
    w.settle();
    assert_eq!(crate::collab::read_only(&w.c(bob).s), None);
    assert_eq!(w.fp(ada), w.fp(bob));
    assert!(shell_types(w.c(bob)).contains(&"craft-live:loaded".to_string()));
}

#[test]
fn reseed_when_the_seeder_left() {
    let mut w = World::new(Room::new(budget_file("Budget.xlsx"), Some("tok-gone")));
    let bob = w.add(config("bob", "edit", false));
    w.settle();
    // The seeder left: the room says reseed; prepare gives Bob the lease.
    w.room.prepare = json!({"seed": true, "seeded": false});
    w.room.lease = None;
    w.room.broadcast(&Frame::Control(json!({"type": "reseed", "message": "The person loading the file left before it was ready."})), None, false);
    w.settle();
    assert!(w.room.seeded);
    let b_doc = w.doc(bob);
    assert_eq!(input(&w.c(bob).s, b_doc, "B1"), "Cost");
    assert_eq!(w.c(bob).s.documents().len(), 1);
    assert_eq!(crate::collab::read_only(&w.c(bob).s), None);
    // Its first connection was closed for the seeding one.
    assert!(!w.c(bob).disconnects.is_empty());
}

#[test]
fn a_seeder_whose_lease_was_taken_joins_instead() {
    // Someone else seeded while Ada's file was downloading: Ada's seed would duplicate it.
    let mut w = World::new(Room::new(budget_file("Budget.xlsx"), Some("tok-ada")));
    let carl = {
        let mut c = config("carl", "edit", true);
        c.token = "tok-ada".into(); // shares the lease for the test: seeds first
        w.add(c)
    };
    w.settle();
    assert!(w.room.seeded);
    let room_state = w.room.sv();
    let ada = w.add(config("ada", "edit", true));
    w.settle();
    let a_doc = w.doc(ada);
    assert_eq!(input(&w.c(ada).s, a_doc, "A1"), "Item");
    assert_eq!(w.fp(ada), w.fp(carl));
    // One copy of the content in the room, not two: Ada's own seed never went out.
    assert_eq!(w.room.sv(), room_state);
    assert_eq!(w.c(ada).s.documents().len(), 1);
    assert!(w.c(ada).notices.iter().any(|n| n.contains("joining their session")));
}

#[test]
fn close_codes_reconnect_or_end() {
    let mut w = World::new(Room::new(budget_file("Budget.xlsx"), Some("tok-ada")));
    let ada = w.add(config("ada", "edit", true));
    let bob = w.add(config("bob", "edit", false));
    w.settle();
    // A dropped connection reconnects after 1 s, then 2 s…; offline edits merge on return.
    w.drop_conn(bob, 1006);
    w.set(bob, "D1", "offline edit");
    w.settle();
    assert!(w.c(bob).conn.is_none());
    assert!(w.c(bob).live.summary().iter().any(|l| l.starts_with("Offline: reconnecting")));
    w.now += 1_000;
    w.settle();
    assert!(w.c(bob).conn.is_some());
    let a_doc = w.doc(ada);
    assert_eq!(input(&w.c(ada).s, a_doc, "D1"), "offline edit");
    assert_eq!(w.fp(ada), w.fp(bob));
    // A connection that fails to open waits twice as long next time.
    w.drop_conn(bob, 1006);
    w.now += 1_000;
    let c = w.c(bob);
    c.live.poll(&mut c.s, 2_000 + 1_000);
    let conn = c.live.take_actions().iter().find_map(|a| if let Action::Connect { conn, .. } = a { Some(*conn) } else { None }).unwrap();
    c.live.on_close(&mut c.s, conn, 1006, "");
    w.now += 1_500;
    w.settle();
    assert!(w.c(bob).conn.is_none(), "2 s this time");
    w.now += 1_000;
    w.settle();
    assert!(w.c(bob).conn.is_some());
    // 4401: the session expired. No reconnecting; read-only; the person is told.
    w.drop_conn(bob, 4401);
    w.now += 60_000;
    w.settle();
    assert!(w.c(bob).conn.is_none());
    assert_eq!(w.c(bob).live.phase(), Phase::Ended);
    assert!(w.c(bob).alerts.iter().any(|a| a.contains("Open the file again")));
    assert!(w.c(bob).s.execute("cell.set", json!({"cell": "A1", "input": "late"})).is_err());
    // 4403: refused.
    w.drop_conn(ada, 4403);
    assert_eq!(w.c(ada).live.phase(), Phase::Ended);
}

#[test]
fn the_control_expired_ends_the_session() {
    let mut w = World::new(Room::new(budget_file("Budget.xlsx"), Some("tok-ada")));
    let ada = w.add(config("ada", "edit", true));
    w.settle();
    w.room.broadcast(&Frame::Control(json!({"type": "expired"})), None, false);
    w.settle();
    assert_eq!(w.c(ada).live.phase(), Phase::Ended);
    assert!(crate::collab::read_only(&w.c(ada).s).is_some());
}

#[test]
fn saving_writes_the_files_own_format_or_lets_the_server_copy() {
    let mut w = World::new(Room::new(budget_file("Plan.xltx"), Some("tok-ada")));
    let ada = w.add(config("ada", "edit", true));
    w.settle();
    let save = |w: &mut World, name: &str, format: &str| {
        let url = format!("{API}/rooms/r/file?save=9");
        w.room.out.push_back((
            0,
            w.clients[0].conn.unwrap(),
            Frame::Control(json!({"type": "save", "id": 9, "mode": "upload", "name": name, "format": format, "url": url})).encode(),
        ));
        w.settle();
        w.room.uploads.last().unwrap().clone()
    };
    let (name, bytes) = save(&mut w, "Plan.xltx", "xltx");
    assert_eq!(name, "Plan.xltx");
    assert!(bytes.starts_with(b"PK"));
    let (name, _) = save(&mut w, "Budget.xlsm", "xlsm");
    assert_eq!(name, "Budget.xlsm");
    // GridCraft doesn't write ODS: an .xlsx, and craft-live saves it as a copy.
    let (name, bytes) = save(&mut w, "Plan.ods", "ods");
    assert_eq!(name, "Plan.xlsx");
    assert!(crate::io::open_bytes(&name, &bytes).is_ok());
    // An upload the room refuses is reported back.
    w.room.upload_status = 409;
    save(&mut w, "Budget.xlsx", "xlsx");
    assert!(w.room.controls_in.iter().any(|c| c["type"] == "saveResult" && c["ok"] == false && c["error"].as_str().unwrap().contains("409")));
    // The token never goes to another origin; WebDAV mode isn't ours.
    let n = w.room.uploads.len();
    w.room.out.push_back((
        ada,
        w.clients[ada].conn.unwrap(),
        Frame::Control(json!({"type": "save", "id": 10, "mode": "upload", "name": "B.xlsx", "url": "https://evil.test/x"})).encode(),
    ));
    w.room.out.push_back((
        ada,
        w.clients[ada].conn.unwrap(),
        Frame::Control(json!({"type": "save", "id": 11, "mode": "webdav", "etag": "x"})).encode(),
    ));
    w.settle();
    assert_eq!(w.room.uploads.len(), n);
    assert!(w.room.controls_in.iter().any(|c| c["type"] == "saveResult" && c["id"] == 10 && c["ok"] == false));
    assert!(w.room.controls_in.iter().any(|c| c["type"] == "saveResult" && c["id"] == 11 && c["ok"] == false));
}

#[test]
fn a_viewer_reads_and_never_saves() {
    let mut w = World::new(Room::new(budget_file("Budget.xlsx"), Some("tok-ada")));
    let ada = w.add(config("ada", "edit", true));
    let vic = w.add(config("vic", "view", false));
    w.settle();
    assert_eq!(w.fp(ada), w.fp(vic));
    assert!(crate::collab::read_only(&w.c(vic).s).unwrap().contains("view"));
    assert!(w.c(vic).s.execute("cell.set", json!({"cell": "A1", "input": "x"})).is_err());
    w.c(vic).live.save_now();
    w.settle();
    assert!(!w.room.controls_in.iter().any(|c| c["type"] == "saveNow"));
    let lines = w.c(vic).live.summary();
    assert!(lines.iter().any(|l| l.contains("can view")), "{lines:?}");
}

#[test]
fn closing_saves_first_then_tells_the_shell() {
    let mut w = World::new(Room::new(budget_file("Budget.xlsx"), Some("tok-ada")));
    let ada = w.add(config("ada", "edit", true));
    w.settle();
    w.set(ada, "A9", "unsaved");
    w.settle();
    assert_eq!(w.c(ada).live.modified(), Some(true));
    let c = w.c(ada);
    c.live.close(&mut c.s, true);
    w.settle();
    assert_eq!(w.room.uploads.len(), 1, "the room saved before the person left");
    let c = w.c(ada);
    assert_eq!(c.live.phase(), Phase::Closed);
    assert!(shell_types(c).contains(&"craft-live:close".to_string()));
    assert!(c.sent.iter().any(|f| matches!(f, Frame::Control(v) if v["type"] == "bye")));
    assert!(c.conn.is_none());

    // Nextcloud closes (shell → app): no echo back; a save that never comes gives up after 10 s.
    let mut w = World::new(Room::new(budget_file("Budget.xlsx"), Some("tok-ada")));
    let ada = w.add(config("ada", "edit", true));
    w.settle();
    w.set(ada, "A9", "unsaved");
    w.settle();
    w.c(ada).hold_http = true;
    let c = w.c(ada);
    c.live.on_shell(&mut c.s, &json!({"type": "craft-live:close"}));
    w.settle();
    assert_eq!(w.c(ada).live.phase(), Phase::Closing);
    w.now += 10_000;
    w.settle();
    assert_eq!(w.c(ada).live.phase(), Phase::Closed);
    assert!(!shell_types(w.c(ada)).contains(&"craft-live:close".to_string()));
}

#[test]
fn closing_the_workbook_closes_the_document_in_nextcloud() {
    let mut w = World::new(Room::new(budget_file("Budget.xlsx"), Some("tok-ada")));
    let ada = w.add(config("ada", "edit", true));
    w.settle();
    w.c(ada).s.execute("file.close", json!({"force": true})).unwrap();
    w.settle();
    assert!(shell_types(w.c(ada)).contains(&"craft-live:close".to_string()));
    assert_eq!(w.c(ada).live.phase(), Phase::Closed);
}

#[test]
fn a_file_that_cant_be_had_is_an_error_for_the_shell() {
    let mut w = World::new(Room::new(b"PK\x03\x04\x14\x00 a damaged package".to_vec(), Some("tok-ada")));
    let ada = w.add(config("ada", "edit", true));
    w.settle();
    let c = w.c(ada);
    assert_eq!(c.live.phase(), Phase::Ended);
    assert!(shell_types(c).contains(&"craft-live:error".to_string()));
    assert!(!shell_types(c).contains(&"craft-live:loaded".to_string()));

    // A 0-byte file is a new, empty workbook.
    let mut w = World::new(Room::new(vec![], Some("tok-ada")));
    let ada = w.add(config("ada", "edit", true));
    w.settle();
    assert_eq!(w.c(ada).live.phase(), Phase::Live);
    w.set(ada, "A1", "first");
    w.settle();
    assert!(w.room.seeded);

    // An HTTP error.
    let mut w = World::new(Room::new(vec![], Some("tok-ada")));
    let ada = w.add(config("ada", "edit", true));
    w.c(ada).hold_http = true;
    w.settle();
    let r = w.c(ada).held.pop().unwrap();
    let c = w.c(ada);
    c.live.on_http(&mut c.s, r.id, Ok(HttpResponse { status: 401, body: br#"{"error":"Nextcloud refused the session"}"#.to_vec() }));
    assert_eq!(c.live.phase(), Phase::Ended);
    assert!(c.live.ended().unwrap().contains("Nextcloud refused the session"));
}

#[test]
fn hostile_messages_never_crash() {
    let mut w = World::new(Room::new(budget_file("Budget.xlsx"), Some("tok-ada")));
    let ada = w.add(config("ada", "edit", true));
    w.settle();
    let conn = w.clients[ada].conn.unwrap();
    let junk: Vec<Vec<u8>> = vec![
        vec![0, 2, 3, 1, 2, 3],
        vec![0, 1, 1, 0xff],
        vec![1, 4, 9, 9, 9, 9],
        Frame::Control(json!({"type": "save"})).encode(),
        Frame::Control(json!({"type": "save", "id": "x", "mode": "upload", "url": 5})).encode(),
        Frame::Control(json!({"type": "peers", "peers": "no"})).encode(),
        Frame::Control(json!({"type": "notice", "level": "<b>", "text": "\u{0}x"})).encode(),
        Frame::Control(json!({"type": "welcome", "seed": "maybe"})).encode(),
        Frame::Control(json!({"type": 7})).encode(),
        vec![100, 1, b'x'],
        vec![2, 0, 2, 0xff, 0xfe],
    ];
    for j in junk {
        let c = w.c(ada);
        c.live.on_frame(&mut c.s, conn, &j);
        w.settle();
    }
    // Messages for a connection that's gone are ignored.
    let c = w.c(ada);
    c.live.on_frame(&mut c.s, conn + 7, &Frame::Control(json!({"type": "expired"})).encode());
    c.live.on_close(&mut c.s, conn + 7, 4401, "");
    c.live.on_http(&mut c.s, 999, Err("?".into()));
    assert_eq!(c.live.phase(), Phase::Live);
    w.set(ada, "A1", "still fine");
    w.settle();
}

/// The desktop app writes the format craft-live asks for in `collab/snapshot {file: true, format}`.
#[test]
fn desktop_snapshot_writes_the_asked_format() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("cell.set", json!({"cell": "A1", "input": "x"})).unwrap();
    s.collab_join_in_memory(json!({"room": "nc:h:1", "mode": "share", "title": "Plan.xltx"})).unwrap();
    let _ = s.collab_take_outgoing();
    let ask = |s: &mut Session, id: u64, format: Option<&str>| {
        let mut params = json!({"room": "nc:h:1", "file": true});
        if let Some(f) = format {
            params["format"] = json!(f);
        }
        let req = Pushed {
            plugin: crate::collab::IN_MEMORY.into(),
            incoming: Incoming::Message(json!({"jsonrpc": "2.0", "id": id, "method": "collab/snapshot", "params": params})),
        };
        s.collab_poll_pushed(0, vec![req]);
        s.collab_take_outgoing().into_iter().find(|m| m["id"] == id).unwrap()["result"].clone()
    };
    let r = ask(&mut s, 1, Some("xltx"));
    assert_eq!((r["name"].as_str(), r["format"].as_str()), (Some("Book1.xltx"), Some("xltx")));
    let r = ask(&mut s, 2, Some("ods"));
    assert_eq!((r["name"].as_str(), r["format"].as_str()), (Some("Book1.xlsx"), Some("xlsx")));
    let r = ask(&mut s, 3, None);
    assert_eq!((r["name"].as_str(), r["format"].as_str()), (Some("Book1.xlsx"), Some("xlsx")));
    let bytes = gridcraft_collab::b64_decode(r["file"].as_str().unwrap()).unwrap();
    assert!(crate::io::open_bytes("Book1.xlsx", &bytes).is_ok());
}

#[test]
fn live_file_bytes_names_and_formats() {
    let wb = gridcraft_model::Workbook::new();
    let n = |name: &str, f: &str| crate::io::live_file_bytes(&wb, name, f).map(|(n, f, _)| (n, f)).unwrap();
    assert_eq!(n("BUDGET.XLSX", "xlsx"), ("BUDGET.XLSX".into(), "xlsx".into()));
    assert_eq!(n("Plan.xltx", "XLTX"), ("Plan.xltx".into(), "xltx".into()));
    assert_eq!(n("Plan.ods", "ods"), ("Plan.xlsx".into(), "xlsx".into()));
    assert_eq!(n("Plan", ".xlsm"), ("Plan.xlsm".into(), "xlsm".into()));
    assert_eq!(n("", "csv"), ("Book.xlsx".into(), "xlsx".into()));
}

#[test]
fn helpers() {
    assert_eq!(pct_decode("a%20b+c%2Bd%zz%4"), "a b c+d%zz%4");
    assert_eq!(pct_encode("Budget 2026 (€).xlsx"), "Budget%202026%20%28%E2%82%AC%29.xlsx");
    assert_eq!(origin("https://Live.Test:8443/api/v1?x"), Some("https://live.test:8443".into()));
    assert_eq!(origin("nope"), None);
    assert_eq!(LiveClient::hello()["type"], "craft-live:hello");
}
