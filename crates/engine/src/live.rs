//! craft-live: GridCraft's web build as a live editor in Nextcloud Office.
//!
//! craft-live (a WOPI editor that replaces Collabora Online) frames the web build with
//! `index.html#craft-live=1&ws=…&room=…&token=…` and expects the app to speak its **embed
//! contract** (craft-live `docs/design.md` §9): say hello to the shell, seed the room from the
//! file or join it, speak y-websocket (protocol 2, Appendix B: lib0 framing) plus craft-live's
//! control messages (type 100), and upload the file bytes when the room asks for them.
//!
//! [`LiveClient`] is that contract without any I/O, so it runs (and is tested) natively: the
//! caller performs its [`Action`]s (open a WebSocket, send a frame, an HTTP request, a message to
//! the shell) and feeds back what happens ([`LiveClient::on_open`], [`LiveClient::on_frame`],
//! [`LiveClient::on_close`], [`LiveClient::on_http`], [`LiveClient::on_shell`]), and calls
//! [`LiveClient::poll`] between frames. The workbook side is the engine's own co-editing room
//! ([`crate::collab`]), served in memory: every committed command is one Yjs update, exactly as
//! with a transport plugin. The browser build drives it with `web_sys::WebSocket` and `fetch`
//! (`apps/gridcraft-web/src/live.rs`).

use std::collections::BTreeMap;

use gridcraft_plugin::push::{Incoming, Pushed};
use serde_json::{Value as Json, json};

use crate::Session;
use crate::collab::User;

/// The transport id of craft-live rooms (served in memory: no plugin).
pub const TRANSPORT: &str = "craft-live";
/// What the session's popover calls the transport.
pub const TRANSPORT_LABEL: &str = "Nextcloud";
/// The embed contract version this build speaks (`craft-live=1`).
pub const CONTRACT: &str = "1";
/// Largest WebSocket message handled (craft-live sends at most 16 MiB of update plus framing).
pub const MAX_MESSAGE: usize = 17 << 20;
/// Reconnecting waits this long first, doubling up to [`BACKOFF_MAX_MS`].
const BACKOFF_MIN_MS: u64 = 1_000;
const BACKOFF_MAX_MS: u64 = 30_000;
/// Closing with unsaved changes waits this long for the room's save.
const CLOSE_SAVE_MS: u64 = 10_000;
/// Peers kept from a `peers` message.
const MAX_PEERS: usize = 1_000;

/// y-websocket messages (protocol 2, Appendix B) and craft-live's control messages.
pub mod wire {
    use serde_json::Value as Json;

    /// One WebSocket message.
    #[derive(Clone, Debug, PartialEq)]
    pub enum Frame {
        /// `0/0`: a state vector.
        Sync1(Vec<u8>),
        /// `0/1`: the update answering a state vector.
        Sync2(Vec<u8>),
        /// `0/2`: an update.
        Update(Vec<u8>),
        /// `1`: a y-protocols awareness update.
        Awareness(Vec<u8>),
        /// `2/0`: permission denied, with the reason.
        Denied(String),
        /// `3`: asks for everyone's awareness.
        QueryAwareness,
        /// `100`: craft-live control, a JSON object.
        Control(Json),
    }

    fn var(out: &mut Vec<u8>, mut n: u64) {
        loop {
            let b = (n & 0x7f) as u8;
            n >>= 7;
            if n == 0 {
                out.push(b);
                return;
            }
            out.push(b | 0x80);
        }
    }

    fn var_bytes(out: &mut Vec<u8>, d: &[u8]) {
        var(out, d.len() as u64);
        out.extend_from_slice(d);
    }

    fn read_var(d: &[u8], pos: &mut usize) -> Result<u64, String> {
        let mut n = 0u64;
        let mut shift = 0u32;
        loop {
            let b = *d.get(*pos).ok_or("truncated number")?;
            *pos += 1;
            n |= u64::from(b & 0x7f).checked_shl(shift).ok_or("number too large")?;
            if b & 0x80 == 0 {
                return Ok(n);
            }
            shift += 7;
            if shift > 63 {
                return Err("number too large".into());
            }
        }
    }

    fn read_bytes<'a>(d: &'a [u8], pos: &mut usize) -> Result<&'a [u8], String> {
        let n = usize::try_from(read_var(d, pos)?).map_err(|_| "length too large")?;
        let end = pos.checked_add(n).ok_or("length too large")?;
        let out = d.get(*pos..end).ok_or("truncated payload")?;
        *pos = end;
        Ok(out)
    }

    impl Frame {
        pub fn encode(&self) -> Vec<u8> {
            let mut o = Vec::new();
            match self {
                Frame::Sync1(d) => {
                    o.extend_from_slice(&[0, 0]);
                    var_bytes(&mut o, d);
                }
                Frame::Sync2(d) => {
                    o.extend_from_slice(&[0, 1]);
                    var_bytes(&mut o, d);
                }
                Frame::Update(d) => {
                    o.extend_from_slice(&[0, 2]);
                    var_bytes(&mut o, d);
                }
                Frame::Awareness(d) => {
                    o.push(1);
                    var_bytes(&mut o, d);
                }
                Frame::Denied(r) => {
                    o.extend_from_slice(&[2, 0]);
                    var_bytes(&mut o, r.as_bytes());
                }
                Frame::QueryAwareness => o.push(3),
                Frame::Control(v) => {
                    var(&mut o, 100);
                    var_bytes(&mut o, v.to_string().as_bytes());
                }
            }
            o
        }

        /// Decodes one message; an unknown type or a damaged message is an error (and ignored).
        pub fn decode(d: &[u8]) -> Result<Frame, String> {
            if d.len() > super::MAX_MESSAGE {
                return Err(format!("a message of {} bytes is too large", d.len()));
            }
            let mut p = 0;
            Ok(match read_var(d, &mut p)? {
                0 => {
                    let step = read_var(d, &mut p)?;
                    let b = read_bytes(d, &mut p)?.to_vec();
                    match step {
                        0 => Frame::Sync1(b),
                        1 => Frame::Sync2(b),
                        2 => Frame::Update(b),
                        n => return Err(format!("unknown sync step {n}")),
                    }
                }
                1 => Frame::Awareness(read_bytes(d, &mut p)?.to_vec()),
                2 => {
                    // y-websocket's auth message: 0 is "permission denied" (the only one).
                    read_var(d, &mut p)?;
                    Frame::Denied(String::from_utf8_lossy(read_bytes(d, &mut p)?).chars().take(300).collect())
                }
                3 => Frame::QueryAwareness,
                100 => {
                    let v: Json = serde_json::from_slice(read_bytes(d, &mut p)?).map_err(|e| format!("control isn't JSON: {e}"))?;
                    if !v.is_object() {
                        return Err("control isn't an object".into());
                    }
                    Frame::Control(v)
                }
                n => return Err(format!("unknown message type {n}")),
            })
        }
    }
}

use wire::Frame;

// ---------------------------------------------------------------- configuration

/// The embed configuration (§9.1): from the page's fragment, or the shell's `craft-live:config`.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveConfig {
    /// The room's WebSocket URL (`wss://live.example.com/ws/nc%3A…`).
    pub ws: String,
    pub room: String,
    /// The craft-live session token (secret): `?token=` on the WebSocket, `Bearer` on the API.
    pub token: String,
    /// The person, from Nextcloud.
    pub user: User,
    /// `role=view`: the workbook is read-only.
    pub view_only: bool,
    /// This client seeds the room from the file.
    pub seed: bool,
    /// Where seeders `GET` the file.
    pub file: String,
    /// The file's name (`Budget.xlsx`).
    pub name: String,
    /// Its extension: the format to save in.
    pub format: String,
    /// The API base (`https://live.example.com/api/v1`).
    pub api: String,
    /// Nextcloud's interface language (may be empty).
    pub lang: String,
}

/// Decodes `%XX` (and `+` as a space, as form encoding does).
pub fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        let hex = |j: usize| b.get(j).and_then(|h| char::from(*h).to_digit(16));
        match c {
            b'%' if hex(i + 1).is_some() && hex(i + 2).is_some() => {
                out.push((hex(i + 1).unwrap_or(0) * 16 + hex(i + 2).unwrap_or(0)) as u8);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encodes everything but RFC 3986's unreserved characters.
pub fn pct_encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { char::from(b).to_string() } else { format!("%{b:02X}") }).collect()
}

/// `scheme://host[:port]` of a URL (lower case), for same-origin checks.
pub fn origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty() && !scheme.is_empty()).then(|| format!("{}://{}", scheme.to_ascii_lowercase(), host.to_ascii_lowercase()))
}

fn clip(s: &str, n: usize) -> String {
    s.trim().chars().filter(|c| !c.is_control()).take(n).collect()
}

impl LiveConfig {
    /// The configuration in a page fragment (`#craft-live=1&ws=…`). `None` when the fragment
    /// isn't craft-live's; an error when it is but can't be used (the shell's config may still
    /// arrive).
    pub fn from_fragment(fragment: &str) -> Option<Result<LiveConfig, String>> {
        let f = fragment.strip_prefix('#').unwrap_or(fragment);
        let mut map = serde_json::Map::new();
        for kv in f.split('&').take(64) {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            let (k, v) = (pct_decode(k), pct_decode(v));
            if v.len() > 16_384 {
                continue;
            }
            let value = match k.as_str() {
                "user" => serde_json::from_str(&v).unwrap_or(Json::String(v)),
                "seed" => json!(v == "1" || v.eq_ignore_ascii_case("true")),
                _ => Json::String(v),
            };
            map.entry(k).or_insert(value);
        }
        let version = map.get("craft-live")?.as_str().unwrap_or("").to_string();
        if version != CONTRACT {
            return Some(Err(format!("this GridCraft speaks craft-live embed contract {CONTRACT}, not {version}")));
        }
        Some(LiveConfig::from_json(&Json::Object(map)))
    }

    /// The configuration as `craft-live:config` gives it (`seed` a boolean, `user` an object).
    pub fn from_json(v: &Json) -> Result<LiveConfig, String> {
        let s = |k: &str| v.get(k).and_then(Json::as_str).map(str::trim).unwrap_or("").to_string();
        let ws = s("ws");
        if !(ws.starts_with("ws://") || ws.starts_with("wss://")) {
            return Err("`ws` isn't a WebSocket URL".into());
        }
        let room = clip(&s("room"), 256);
        if room.is_empty() {
            return Err("`room` is missing".into());
        }
        let token = s("token");
        if token.is_empty() || token.len() > 1024 {
            return Err("`token` is missing".into());
        }
        let user_v = match v.get("user") {
            Some(Json::String(t)) => serde_json::from_str(t).unwrap_or(Json::Null),
            Some(u) => u.clone(),
            None => Json::Null,
        };
        let us = |k: &str| user_v.get(k).and_then(Json::as_str).map(|t| clip(t, 128)).unwrap_or_default();
        let id = us("id");
        let name = us("name");
        let color = us("color");
        let user = User {
            name: if name.is_empty() { if id.is_empty() { "Guest".into() } else { id.clone() } } else { name },
            id: if id.is_empty() { "guest".into() } else { id },
            color: if crate::collab::valid_color(&color) { color.to_ascii_uppercase() } else { "#3B7DD8".into() },
        };
        let seed = match v.get("seed") {
            Some(Json::Bool(b)) => *b,
            Some(Json::String(t)) => t == "1" || t.eq_ignore_ascii_case("true"),
            Some(Json::Number(n)) => n.as_u64() == Some(1),
            _ => false,
        };
        let view_only = s("role") == "view";
        let seed = seed && !view_only;
        let file = s("file");
        let api = s("api").trim_end_matches('/').to_string();
        let http = |u: &str| u.starts_with("http://") || u.starts_with("https://");
        if seed && !http(&file) {
            return Err("`file` isn't an http(s) URL".into());
        }
        if !api.is_empty() && !http(&api) {
            return Err("`api` isn't an http(s) URL".into());
        }
        let name = {
            let n = clip(&s("name"), 255);
            let n = n.rsplit(['/', '\\']).next().unwrap_or("").to_string();
            if n.is_empty() { "Book.xlsx".to_string() } else { n }
        };
        let format = {
            let f = clip(&s("format"), 16).trim_start_matches('.').to_ascii_lowercase();
            if f.is_empty() { std::path::Path::new(&name).extension().and_then(|e| e.to_str()).unwrap_or("xlsx").to_ascii_lowercase() } else { f }
        };
        Ok(LiveConfig { ws, room, token, user, view_only, seed, file, name, format, api, lang: clip(&s("lang"), 16) })
    }

    /// The URL to open the room's WebSocket with.
    pub fn ws_url(&self) -> String {
        let sep = if self.ws.contains('?') { '&' } else { '?' };
        format!("{}{sep}token={}", self.ws, pct_encode(&self.token))
    }

    fn bearer(&self) -> (String, String) {
        ("Authorization".into(), format!("Bearer {}", self.token))
    }
}

// ---------------------------------------------------------------- the client

/// An HTTP request for the caller to make.
#[derive(Clone, Debug, PartialEq)]
pub struct HttpRequest {
    /// Hand it back with the answer ([`LiveClient::on_http`]).
    pub id: u64,
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

/// An HTTP answer.
#[derive(Clone, Debug, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// What the caller does for the client.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Open the room's WebSocket (binary frames); its events carry `conn`.
    Connect {
        conn: u64,
        url: String,
    },
    /// Send a binary message on connection `conn` (drop it if that connection is gone).
    Send {
        conn: u64,
        frame: Vec<u8>,
    },
    /// Close connection `conn`.
    Disconnect {
        conn: u64,
        code: u16,
        reason: String,
    },
    Http(HttpRequest),
    /// Post this object to the shell (`window.parent`, same origin).
    Shell(Json),
    /// A status-bar message (`level`: info, warning or error).
    Notice {
        level: String,
        text: String,
    },
    /// A message box: something the person must know (the session ended).
    Alert(String),
}

/// Where the client is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Not started yet.
    Starting,
    /// Fetching the file to seed the room.
    Loading,
    /// In the room.
    Live,
    /// Closing: waiting for the room's save first.
    Closing,
    /// Closed by the person or the shell.
    Closed,
    /// The session ended (expired, refused, or the document couldn't be opened).
    Ended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Link {
    Down,
    Connecting,
    Open,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Purpose {
    SeedFile,
    Upload(u64),
    Prepare,
}

/// GridCraft's side of the craft-live embed contract (see the module docs).
pub struct LiveClient {
    pub cfg: LiveConfig,
    phase: Phase,
    ended: Option<String>,
    link: Link,
    conn: u64,
    retry_at: Option<u64>,
    backoff: u64,
    now: u64,
    actions: Vec<Action>,
    http: BTreeMap<u64, Purpose>,
    next_http: u64,
    incoming: Vec<Pushed>,
    /// The workbook's `DocState::uid` once in the room.
    doc: Option<u64>,
    client_id: Option<u64>,
    /// This client seeded the room and the room hasn't confirmed it yet.
    seeding: bool,
    loaded: bool,
    /// The room's unsaved state (control `modified`, `saved`, `seeded`).
    modified: Option<bool>,
    /// The person (or Nextcloud) asked for a save: report its outcome to the shell.
    save_asked: bool,
    /// An upload is in flight.
    uploading: bool,
    peers: Vec<Json>,
    /// Closing tells the shell (the person closed, not the shell).
    close_tells_shell: bool,
    close_deadline: Option<u64>,
    /// Messages that couldn't be decoded, for diagnostics.
    pub dropped: u64,
}

fn rpc(method: &str, params: Json) -> Pushed {
    Pushed { plugin: TRANSPORT.into(), incoming: Incoming::Message(json!({"jsonrpc": "2.0", "method": method, "params": params})) }
}

fn shell(kind: &str, mut fields: Json) -> Action {
    if let Some(o) = fields.as_object_mut() {
        o.insert("type".into(), json!(kind));
    }
    Action::Shell(fields)
}

impl LiveClient {
    pub fn new(cfg: LiveConfig) -> LiveClient {
        LiveClient {
            cfg,
            phase: Phase::Starting,
            ended: None,
            link: Link::Down,
            conn: 0,
            retry_at: None,
            backoff: BACKOFF_MIN_MS,
            now: 0,
            actions: Vec::new(),
            http: BTreeMap::new(),
            next_http: 1,
            incoming: Vec::new(),
            doc: None,
            client_id: None,
            seeding: false,
            loaded: false,
            modified: None,
            save_asked: false,
            uploading: false,
            peers: Vec::new(),
            close_tells_shell: false,
            close_deadline: None,
            dropped: 0,
        }
    }

    /// `craft-live:hello`, which the app posts to the shell as soon as it runs.
    pub fn hello() -> Json {
        json!({"type": "craft-live:hello", "contract": CONTRACT, "app": "gridcraft", "version": env!("CARGO_PKG_VERSION")})
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }
    /// Why the session ended, when it did.
    pub fn ended(&self) -> Option<&str> {
        self.ended.as_deref()
    }
    /// The workbook in the room.
    pub fn doc(&self) -> Option<u64> {
        self.doc
    }
    pub fn online(&self) -> bool {
        self.link == Link::Open
    }
    /// The room's unsaved state, as the room last said.
    pub fn modified(&self) -> Option<bool> {
        self.modified
    }
    /// The others in the room (control `peers`).
    pub fn peers(&self) -> &[Json] {
        &self.peers
    }

    /// What the caller must do now.
    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    /// Starts: seeds the room from the file (`seed=1`) or joins it.
    pub fn start(&mut self, s: &mut Session, now: u64) {
        if self.phase != Phase::Starting {
            return;
        }
        self.now = now;
        s.collab_serve_in_memory(TRANSPORT);
        // Notes, comments and chat messages are signed with the Nextcloud name.
        s.prefs.user_name = self.cfg.user.name.clone();
        if self.cfg.seed {
            self.load_file();
        } else {
            self.join(s, "join");
        }
    }

    fn request(&mut self, purpose: Purpose, method: &'static str, url: String, mut headers: Vec<(String, String)>, body: Option<Vec<u8>>) {
        let id = self.next_http;
        self.next_http += 1;
        self.http.insert(id, purpose);
        headers.insert(0, self.cfg.bearer());
        self.actions.push(Action::Http(HttpRequest { id, method, url, headers, body }));
    }

    fn notice(&mut self, level: &str, text: impl Into<String>) {
        self.actions.push(Action::Notice { level: level.into(), text: text.into() });
    }

    fn send(&mut self, f: &Frame) {
        if self.link == Link::Open {
            self.actions.push(Action::Send { conn: self.conn, frame: f.encode() });
        }
    }

    fn control(&mut self, v: Json) {
        self.send(&Frame::Control(v));
    }

    fn load_file(&mut self) {
        self.phase = Phase::Loading;
        let url = self.cfg.file.clone();
        self.request(Purpose::SeedFile, "GET", url, vec![], None);
    }

    fn connect(&mut self) {
        if self.link != Link::Down {
            self.disconnect(1000, "reconnecting");
        }
        self.conn += 1;
        self.link = Link::Connecting;
        self.retry_at = None;
        self.actions.push(Action::Connect { conn: self.conn, url: self.cfg.ws_url() });
    }

    fn disconnect(&mut self, code: u16, reason: &str) {
        if self.link != Link::Down {
            self.actions.push(Action::Disconnect { conn: self.conn, code, reason: reason.into() });
            self.link = Link::Down;
            self.push_state("offline");
        }
    }

    fn push_state(&mut self, state: &str) {
        if self.doc.is_some() {
            self.incoming.push(rpc("collab/status", json!({"room": self.cfg.room, "state": state})));
        }
    }

    /// Our room in the session.
    fn room_index(&self, s: &Session) -> Option<usize> {
        s.collab.rooms.iter().position(|r| r.room == self.cfg.room && r.plugin == TRANSPORT)
    }

    /// Keeps only `uid` open (the web build edits one document here).
    fn only_document(s: &mut Session, uid: u64) {
        for i in (0..s.documents().len()).rev() {
            if s.documents().get(i).is_some_and(|d| d.uid != uid) {
                s.close_document(i);
            }
        }
        if let Some(i) = s.documents().iter().position(|d| d.uid == uid) {
            s.set_active(i);
        }
    }

    /// Joins the room in `mode` (`share`: from the active workbook; `join`: a new one).
    fn join(&mut self, s: &mut Session, mode: &str) {
        let params = json!({
            "plugin": TRANSPORT,
            "room": self.cfg.room,
            "mode": mode,
            "user": self.cfg.user,
            "role": if self.cfg.view_only { "view" } else { "edit" },
            "title": self.cfg.name,
        });
        match s.collab_join_in_memory(params) {
            Ok(r) => {
                self.client_id = r.get("clientId").and_then(Json::as_u64);
                let uid = s.active().map(|d| d.uid);
                self.doc = uid;
                if let Some(uid) = uid {
                    Self::only_document(s, uid);
                    if let Some(d) = s.active_mut() {
                        d.title = self.cfg.name.clone();
                    }
                }
                // `collab/joined` and the first sync step 1 go when the connection opens.
                let _ = s.collab_take_outgoing();
                self.seeding = mode == "share";
                if self.seeding {
                    // Read-only until the room has the seed: an edit made meanwhile would ride in
                    // the seed, which the room takes as the file itself (nothing to save).
                    self.set_joining(s, true);
                }
                self.phase = Phase::Live;
                if self.seeding {
                    self.loaded_now();
                }
                self.connect();
            }
            Err(e) => self.fail(&format!("GridCraft couldn't join the document: {e}")),
        }
    }

    /// Leaves the room without a trace (its messages are dropped) and closes its workbook.
    fn drop_room(&mut self, s: &mut Session) {
        if self.room_index(s).is_some() {
            let _ = s.execute("collab.leave", json!({"room": self.cfg.room}));
            let _ = s.collab_take_outgoing();
        }
        let _ = s.take_ui_requests();
        if let Some(uid) = self.doc.take()
            && let Some(i) = s.documents().iter().position(|d| d.uid == uid)
        {
            s.close_document(i);
        }
        self.incoming.clear();
        self.seeding = false;
    }

    /// Someone else's document is (or will be) the room's: join it instead of our seed.
    fn become_joiner(&mut self, s: &mut Session) {
        self.disconnect(1000, "joining instead");
        self.drop_room(s);
        self.notice("info", "Someone else opened the file first: joining their session");
        self.join(s, "join");
    }

    /// The room has no document and this client holds the lease: load the file and seed it.
    fn reseed(&mut self, s: &mut Session) {
        self.disconnect(1000, "seeding");
        self.drop_room(s);
        self.loaded = false;
        self.load_file();
    }

    fn loaded_now(&mut self) {
        if !self.loaded {
            self.loaded = true;
            self.actions.push(shell("craft-live:loaded", json!({})));
        }
    }

    /// The document can't be opened or the session can't go on.
    fn fail(&mut self, why: &str) {
        if !self.loaded {
            self.actions.push(shell("craft-live:error", json!({"message": why})));
        }
        self.end(why);
    }

    /// Ends the session: no reconnecting; the workbook stays, read-only.
    fn end(&mut self, why: &str) {
        if matches!(self.phase, Phase::Ended | Phase::Closed) {
            return;
        }
        self.phase = Phase::Ended;
        self.ended = Some(why.to_string());
        self.retry_at = None;
        self.disconnect(1000, "ended");
        self.actions.push(Action::Alert(why.to_string()));
    }

    fn set_joining(&self, s: &mut Session, joining: bool) {
        if let Some(i) = self.room_index(s)
            && let Some(r) = s.collab.rooms.get_mut(i)
        {
            r.joining = joining;
        }
    }

    fn make_read_only(&self, s: &mut Session) {
        if let Some(i) = self.room_index(s)
            && let Some(r) = s.collab.rooms.get_mut(i)
        {
            r.view_only = true;
        }
    }

    // ---- events

    /// The WebSocket `conn` opened.
    pub fn on_open(&mut self, conn: u64) {
        if conn != self.conn || self.link != Link::Connecting {
            return;
        }
        self.link = Link::Open;
        self.backoff = BACKOFF_MIN_MS;
        let hello = json!({"type": "hello", "clientId": self.client_id.unwrap_or(0), "can": ["upload"], "client": format!("gridcraft-web/{}", env!("CARGO_PKG_VERSION"))});
        self.control(hello);
        // The room goes online: the engine sends its sync step 1 and awareness.
        self.push_state("online");
    }

    /// The WebSocket `conn` closed (or failed to open) with `code`.
    pub fn on_close(&mut self, s: &mut Session, conn: u64, code: u16, reason: &str) {
        if conn != self.conn || self.link == Link::Down {
            return;
        }
        self.link = Link::Down;
        self.push_state("offline");
        match code {
            4401 => {
                self.make_read_only(s);
                self.end("Your editing session has ended. Open the file again from Nextcloud to go on editing.");
            }
            4403 => {
                self.make_read_only(s);
                let why = if reason.trim().is_empty() { "craft-live refused the connection".to_string() } else { clip(reason, 300) };
                self.end(&format!("This document can't be edited here: {why}"));
            }
            _ => match self.phase {
                Phase::Live => {
                    self.retry_at = Some(self.now.saturating_add(self.backoff));
                    self.backoff = (self.backoff * 2).min(BACKOFF_MAX_MS);
                }
                Phase::Closing => self.finish_close(s),
                _ => {}
            },
        }
    }

    /// A binary message arrived on `conn`.
    pub fn on_frame(&mut self, s: &mut Session, conn: u64, data: &[u8]) {
        if conn != self.conn || self.link != Link::Open {
            return;
        }
        let frame = match Frame::decode(data) {
            Ok(f) => f,
            Err(e) => {
                self.dropped += 1;
                log::debug!("craft-live: dropped a message: {e}");
                return;
            }
        };
        let room = self.cfg.room.clone();
        let b64 = gridcraft_collab::b64_encode;
        match frame {
            Frame::Sync1(sv) => self.incoming.push(rpc("collab/sync1", json!({"room": room, "stateVector": b64(&sv)}))),
            Frame::Sync2(u) => self.incoming.push(rpc("collab/sync2", json!({"room": room, "update": b64(&u)}))),
            Frame::Update(u) => self.incoming.push(rpc("collab/update", json!({"room": room, "update": b64(&u)}))),
            Frame::Awareness(a) => self.incoming.push(rpc("collab/awareness", json!({"room": room, "update": b64(&a)}))),
            Frame::Denied(why) => self.notice("error", format!("craft-live refused: {why}")),
            Frame::QueryAwareness => {}
            Frame::Control(v) => self.on_control(s, &v),
        }
    }

    fn on_control(&mut self, s: &mut Session, v: &Json) {
        let b = |k: &str| v.get(k).and_then(Json::as_bool);
        let text = |k: &str, n: usize| v.get(k).and_then(Json::as_str).map(|t| clip(t, n)).unwrap_or_default();
        match v.get("type").and_then(Json::as_str).unwrap_or("") {
            "welcome" => {
                let (seed, seeded) = (b("seed") == Some(true), b("seeded") == Some(true));
                if let Some(m) = b("modified") {
                    self.set_modified(m);
                }
                if self.seeding {
                    if seeded && !seed {
                        // The room has someone else's document: ours would duplicate it.
                        self.become_joiner(s);
                    } else if !seed {
                        // The lease went (we were away too long): ask for it again.
                        self.ask_prepare();
                    }
                } else if seed && !seeded && !self.cfg.view_only {
                    // The room's lease is ours: load the file and seed.
                    self.reseed(s);
                }
            }
            "peers" => {
                self.peers = v.get("peers").and_then(Json::as_array).map(|a| a.iter().take(MAX_PEERS).cloned().collect()).unwrap_or_default();
                self.post_peers();
            }
            "seeded" => {
                if self.seeding {
                    self.set_joining(s, false);
                }
                self.seeding = false;
                self.set_modified(b("modified").unwrap_or(false));
            }
            "modified" => self.set_modified(b("modified").unwrap_or(true)),
            "save" => self.on_save(s, v),
            "saved" => {
                let m = b("modified").unwrap_or(false);
                self.set_modified(m);
                if !m
                    && let Some(uid) = self.doc
                    && let Some(d) = s.docs.iter_mut().find(|d| d.uid == uid)
                {
                    d.saved = d.wb.clone();
                }
                if self.save_asked {
                    self.save_asked = false;
                    self.actions.push(shell("craft-live:saved", json!({"success": true})));
                    self.notice("info", "Saved in Nextcloud");
                }
                if self.phase == Phase::Closing {
                    self.finish_close(s);
                }
            }
            "saveFailed" => {
                let why = text("message", 300);
                self.notice(
                    "error",
                    if why.is_empty() { "Saving in Nextcloud failed".to_string() } else { format!("Saving in Nextcloud failed: {why}") },
                );
                if self.save_asked {
                    self.save_asked = false;
                    self.actions.push(shell("craft-live:saved", json!({"success": false})));
                }
                if self.phase == Phase::Closing {
                    self.finish_close(s);
                }
            }
            "notice" => {
                let level = match v.get("level").and_then(Json::as_str) {
                    Some(l @ ("info" | "warning" | "error")) => l.to_string(),
                    _ => "info".into(),
                };
                let t = text("text", 500);
                if !t.is_empty() {
                    self.notice(&level, t);
                }
            }
            "reseed" => {
                let why = text("message", 300);
                if !why.is_empty() {
                    self.notice("warning", why);
                }
                self.ask_prepare();
            }
            "expired" => {
                self.make_read_only(s);
                self.end("Your editing session has ended. Open the file again from Nextcloud to go on editing.");
            }
            _ => {}
        }
    }

    fn ask_prepare(&mut self) {
        if self.cfg.view_only || self.cfg.api.is_empty() {
            return;
        }
        let url = format!("{}/prepare", self.cfg.api);
        self.request(Purpose::Prepare, "POST", url, vec![], None);
    }

    fn set_modified(&mut self, m: bool) {
        if self.modified != Some(m) {
            self.modified = Some(m);
            self.actions.push(shell("craft-live:modified", json!({"modified": m})));
        }
    }

    fn post_peers(&mut self) {
        let mut list = self.peers.clone();
        let mut me = json!({"user": self.cfg.user, "role": if self.cfg.view_only { "view" } else { "edit" }, "self": true});
        if let Some(id) = self.client_id {
            me["clientId"] = json!(id);
        }
        list.push(me);
        self.actions.push(shell("craft-live:peers", json!({"peers": list})));
    }

    /// `save {id, mode, url, name, format}`: the file bytes in the file's format, uploaded.
    fn on_save(&mut self, s: &mut Session, v: &Json) {
        let id = v.get("id").and_then(Json::as_u64).unwrap_or(0);
        let fail = |me: &mut LiveClient, why: &str| me.control(json!({"type": "saveResult", "id": id, "ok": false, "error": why}));
        if v.get("mode").and_then(Json::as_str) != Some("upload") {
            return fail(self, "this client saves by upload only");
        }
        if self.cfg.view_only {
            return fail(self, "a viewer can't save");
        }
        let url = v.get("url").and_then(Json::as_str).unwrap_or("").to_string();
        // The session token goes only to craft-live itself.
        let ours = origin(&self.cfg.api).or_else(|| origin(&self.cfg.ws.replacen("ws", "http", 1)));
        if origin(&url).is_none() || origin(&url) != ours {
            return fail(self, "the upload address isn't craft-live's");
        }
        let name = v.get("name").and_then(Json::as_str).map(|n| clip(n, 255)).filter(|n| !n.is_empty()).unwrap_or_else(|| self.cfg.name.clone());
        let format = v.get("format").and_then(Json::as_str).map(|f| clip(f, 16)).filter(|f| !f.is_empty()).unwrap_or_else(|| self.cfg.format.clone());
        let Some(wb) = self.doc.and_then(|uid| s.docs.iter().find(|d| d.uid == uid)).map(|d| d.wb.clone()) else {
            return fail(self, "the workbook isn't open");
        };
        match crate::io::live_file_bytes(&wb, &name, &format) {
            Ok((name, _written, bytes)) => {
                let mime = match std::path::Path::new(&name).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
                    Some("xlsm") => "application/vnd.ms-excel.sheet.macroEnabled.12",
                    Some("xltx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.template",
                    Some("xltm") => "application/vnd.ms-excel.template.macroEnabled.12",
                    _ => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                };
                self.uploading = true;
                let headers = vec![("X-Craft-File-Name".into(), pct_encode(&name)), ("Content-Type".into(), mime.into())];
                self.request(Purpose::Upload(id), "PUT", url, headers, Some(bytes));
            }
            Err(e) => fail(self, &format!("GridCraft couldn't write the file: {e}")),
        }
    }

    /// The answer to request `id` (`Err`: it never got one).
    pub fn on_http(&mut self, s: &mut Session, id: u64, answer: Result<HttpResponse, String>) {
        let Some(purpose) = self.http.remove(&id) else { return };
        match purpose {
            Purpose::SeedFile => {
                if self.phase != Phase::Loading {
                    return;
                }
                match answer {
                    Ok(r) if r.status == 200 => self.seed_from(s, &r.body),
                    Ok(r) => self.fail(&format!("Nextcloud didn't give {} (HTTP {}): {}", self.cfg.name, r.status, api_error(&r.body))),
                    Err(e) => self.fail(&format!("{} couldn't be downloaded: {e}", self.cfg.name)),
                }
            }
            Purpose::Upload(save) => {
                self.uploading = false;
                let why = match answer {
                    Ok(r) if r.status == 200 => None,
                    Ok(r) => Some(format!("HTTP {}: {}", r.status, api_error(&r.body))),
                    Err(e) => Some(e),
                };
                if let Some(why) = why {
                    self.control(json!({"type": "saveResult", "id": save, "ok": false, "error": clip(&why, 300)}));
                }
            }
            Purpose::Prepare => match answer {
                Ok(r) if r.status == 200 => {
                    let v: Json = serde_json::from_slice(&r.body).unwrap_or(Json::Null);
                    if v.get("seed").and_then(Json::as_bool) == Some(true) {
                        self.reseed(s);
                    } else if self.seeding && v.get("seeded").and_then(Json::as_bool) != Some(true) {
                        // Someone else holds the lease now: wait for their document.
                        self.become_joiner(s);
                    }
                }
                Ok(r) => self.notice("warning", format!("craft-live: {}", api_error(&r.body))),
                Err(e) => self.notice("warning", format!("craft-live can't be reached: {e}")),
            },
        }
    }

    /// The file's bytes: open them as File › Open does, and seed the room (`collab.share`).
    fn seed_from(&mut self, s: &mut Session, bytes: &[u8]) {
        let opened = if bytes.is_empty() {
            // A 0-byte file is a new, empty workbook.
            s.new_workbook();
            Ok(())
        } else {
            s.execute("file.open", json!({"name": self.cfg.name, "base64": crate::io::base64_encode(bytes)})).map(|_| ())
        };
        if let Err(e) = opened {
            return self.fail(&format!("GridCraft can't open {}: {e}", self.cfg.name));
        }
        if let Some(d) = s.active_mut() {
            d.title = self.cfg.name.clone();
            d.path = None;
            d.saved = d.wb.clone();
        }
        self.join(s, "share");
    }

    /// A message from the shell (`craft-live:*`).
    pub fn on_shell(&mut self, s: &mut Session, msg: &Json) {
        match msg.get("type").and_then(Json::as_str).unwrap_or("") {
            "craft-live:save" => {
                // The shell has told the server already: show progress.
                if !self.cfg.view_only {
                    self.save_asked = true;
                    self.notice("info", "Saving in Nextcloud…");
                }
            }
            "craft-live:close" => self.close(s, false),
            "craft-live:getPeers" => self.post_peers(),
            _ => {}
        }
    }

    /// The person pressed Save: the room saves now.
    pub fn save_now(&mut self) {
        if self.cfg.view_only {
            self.notice("warning", "You can view this workbook, not save it");
            return;
        }
        if self.phase != Phase::Live {
            return;
        }
        if self.link == Link::Open {
            self.save_asked = true;
            self.control(json!({"type": "saveNow"}));
            self.notice("info", "Saving in Nextcloud…");
        } else {
            self.notice("warning", "Offline: your changes are saved when the connection is back");
        }
    }

    /// The person (or, with `tell_shell` false, the shell) closes the document. Unsaved changes
    /// are saved first (up to 10 s): once everyone has gone nobody can produce the file.
    pub fn close(&mut self, s: &mut Session, tell_shell: bool) {
        self.close_tells_shell |= tell_shell;
        match self.phase {
            Phase::Closing => {}
            Phase::Live if self.modified == Some(true) && !self.cfg.view_only && self.link == Link::Open => {
                self.phase = Phase::Closing;
                self.close_deadline = Some(self.now.saturating_add(CLOSE_SAVE_MS));
                self.control(json!({"type": "saveNow"}));
                self.notice("info", "Saving in Nextcloud before closing…");
            }
            _ => self.finish_close(s),
        }
    }

    fn finish_close(&mut self, s: &mut Session) {
        if self.phase == Phase::Closed {
            return;
        }
        self.control(json!({"type": "bye"}));
        self.phase = Phase::Closed;
        self.retry_at = None;
        self.close_deadline = None;
        self.disconnect(1000, "closed");
        self.make_read_only(s);
        if self.close_tells_shell {
            self.actions.push(shell("craft-live:close", json!({})));
        }
    }

    /// Between frames: runs the room (what arrived, the engine's answers and this person's
    /// edits and selection go out), reconnects when due, and notices a closed workbook. Returns
    /// whether the interface should repaint.
    pub fn poll(&mut self, s: &mut Session, now: u64) -> bool {
        self.now = now;
        let pushed = std::mem::take(&mut self.incoming);
        let mut changed = !pushed.is_empty();
        changed |= s.collab_poll_pushed(now, pushed);
        for m in s.collab_take_outgoing() {
            let p = &m["params"];
            if p.get("room").and_then(Json::as_str).is_some_and(|r| r != self.cfg.room) {
                continue;
            }
            let bytes = |k: &str| p.get(k).and_then(Json::as_str).and_then(|t| gridcraft_collab::b64_decode(t).ok());
            let frame = match m.get("method").and_then(Json::as_str).unwrap_or("") {
                "collab/sync1" => bytes("stateVector").map(Frame::Sync1),
                "collab/sync2" => bytes("update").map(Frame::Sync2),
                "collab/update" => bytes("update").map(Frame::Update),
                "collab/awareness" => bytes("update").map(Frame::Awareness),
                _ => None,
            };
            if let Some(f) = frame {
                self.send(&f);
            }
        }
        match self.phase {
            Phase::Live => {
                let room = self.room_index(s).and_then(|i| s.collab.rooms.get(i));
                match room {
                    Some(r) if !r.joining => self.loaded_now(),
                    Some(_) => {}
                    // The workbook was closed or the room left (File › Close, Leave): close.
                    None => {
                        self.doc = None;
                        self.close_tells_shell = true;
                        self.finish_close(s);
                        changed = true;
                    }
                }
                if self.link == Link::Down && self.retry_at.is_some_and(|t| now >= t) {
                    self.connect();
                    changed = true;
                }
            }
            Phase::Closing if self.close_deadline.is_some_and(|t| now >= t) => {
                self.finish_close(s);
                changed = true;
            }
            _ => {}
        }
        changed || !self.actions.is_empty()
    }

    /// Lines for the session's popover: the file, the person, the saved state, the connection.
    pub fn summary(&self) -> Vec<String> {
        let mut lines = vec![format!("{} in Nextcloud", self.cfg.name)];
        lines.push(format!("{} · {}", self.cfg.user.name, if self.cfg.view_only { "can view" } else { "can edit" }));
        let saved = match self.phase {
            Phase::Ended => self.ended.clone(),
            Phase::Closed => Some("Closed".into()),
            Phase::Closing => Some("Saving before closing…".into()),
            Phase::Loading => Some(format!("Loading {}…", self.cfg.name)),
            _ if self.uploading => Some("Saving…".into()),
            _ if self.cfg.view_only => None,
            _ => match self.modified {
                Some(true) => Some("Unsaved changes: Nextcloud gets them automatically".into()),
                Some(false) => Some("All changes saved in Nextcloud".into()),
                None => None,
            },
        };
        lines.extend(saved);
        if matches!(self.phase, Phase::Live | Phase::Closing) {
            match self.link {
                Link::Down => {
                    if let Some(t) = self.retry_at {
                        lines.push(format!("Offline: reconnecting in {} s", t.saturating_sub(self.now).div_ceil(1000).max(1)));
                    }
                }
                Link::Connecting => lines.push("Connecting…".into()),
                Link::Open => {}
            }
        }
        lines
    }
}

/// The `error` of a craft-live JSON error body, or the body's start.
fn api_error(body: &[u8]) -> String {
    let v: Json = serde_json::from_slice(body).unwrap_or(Json::Null);
    match v.get("error").or_else(|| v.get("message")).and_then(Json::as_str) {
        Some(e) => clip(e, 300),
        None => clip(&String::from_utf8_lossy(body.get(..300.min(body.len())).unwrap_or_default()), 300),
    }
}

#[cfg(test)]
mod tests;
