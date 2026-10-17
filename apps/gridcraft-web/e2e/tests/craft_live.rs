//! The web build's live client (`gridcraft_engine::live::LiveClient`) against a real craft-live
//! server and craft-live's fake Nextcloud (its `testkit`): the editor page Nextcloud opens, the
//! fragment and configuration in it, seeding from the WOPI file, a second person joining and
//! converging, edits both ways, Save reaching the WOPI host as a workbook GridCraft reads back,
//! a viewer, and closing after the room saved. The browser's WebSocket and `fetch` are played by
//! tokio-tungstenite and reqwest; everything GridCraft decides is the code the web build runs.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use craft_live::config::Config;
use craft_live_testkit::FakeNextcloud;
use futures_util::{SinkExt, StreamExt};
use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_engine::live::{Action, HttpResponse, LiveClient, LiveConfig, Phase};
use serde_json::{Value as Json, json};
use tokio_tungstenite::tungstenite::Message;

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("gridcraft-e2e-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A workbook as Nextcloud holds it: A1 "Item", B1 "Cost", B2 12.
fn budget() -> Vec<u8> {
    let mut s = Session::new();
    s.new_workbook();
    for (c, v) in [("A1", "Item"), ("B1", "Cost"), ("B2", "12")] {
        s.execute("cell.set", json!({"cell": c, "input": v})).unwrap();
    }
    gridcraft_engine::io::save_bytes_as(&s.doc().unwrap().wb, "Budget.xlsx", gridcraft_engine::io::FileKind::Xlsx).unwrap()
}

fn cell(wb: &gridcraft_engine::model::Workbook, a1: &str) -> String {
    wb.sheets[0].input_text(CellRef::parse(a1).unwrap())
}

/// One browser tab running the web build in craft-live mode.
struct Tab {
    s: Session,
    live: LiveClient,
    ws: Option<(u64, Ws)>,
    shell: Vec<Json>,
    notices: Vec<String>,
    alerts: Vec<String>,
    http: reqwest::Client,
    t0: Instant,
}

impl Tab {
    fn new(cfg: LiveConfig) -> Tab {
        Tab {
            s: Session::new(),
            live: LiveClient::new(cfg),
            ws: None,
            shell: vec![],
            notices: vec![],
            alerts: vec![],
            http: reqwest::Client::new(),
            t0: Instant::now(),
        }
    }

    fn now(&self) -> u64 {
        self.t0.elapsed().as_millis() as u64 + 1
    }

    /// One frame: what arrived, then the client, then its actions.
    async fn frame(&mut self) {
        if self.live.phase() == Phase::Starting {
            let now = self.now();
            self.live.start(&mut self.s, now);
        }
        let mut closed = None;
        if let Some((conn, ws)) = &mut self.ws {
            loop {
                match tokio::time::timeout(Duration::from_millis(3), ws.next()).await {
                    Ok(Some(Ok(Message::Binary(b)))) => self.live.on_frame(&mut self.s, *conn, &b),
                    Ok(Some(Ok(Message::Close(c)))) => {
                        closed = Some((*conn, c.map(|c| (u16::from(c.code), c.reason.to_string())).unwrap_or((1005, String::new()))));
                        break;
                    }
                    Ok(Some(Ok(_))) => {}
                    Ok(Some(Err(e))) => {
                        closed = Some((*conn, (1006, e.to_string())));
                        break;
                    }
                    Ok(None) => {
                        closed = Some((*conn, (1006, "gone".into())));
                        break;
                    }
                    Err(_) => break,
                }
            }
        }
        if let Some((conn, (code, reason))) = closed {
            self.ws = None;
            self.live.on_close(&mut self.s, conn, code, &reason);
        }
        let now = self.now();
        self.live.poll(&mut self.s, now);
        for a in self.live.take_actions() {
            match a {
                Action::Connect { conn, url } => match tokio_tungstenite::connect_async(url.as_str()).await {
                    Ok((ws, _)) => {
                        self.ws = Some((conn, ws));
                        self.live.on_open(conn);
                    }
                    Err(e) => self.live.on_close(&mut self.s, conn, 1006, &e.to_string()),
                },
                Action::Send { conn, frame } => {
                    if let Some((c, ws)) = &mut self.ws
                        && *c == conn
                    {
                        let _ = ws.send(Message::Binary(frame)).await;
                    }
                }
                Action::Disconnect { conn, .. } => {
                    if self.ws.as_ref().is_some_and(|(c, _)| *c == conn)
                        && let Some((_, mut ws)) = self.ws.take()
                    {
                        let _ = ws.close(None).await;
                    }
                }
                Action::Http(r) => {
                    let method = reqwest::Method::from_bytes(r.method.as_bytes()).unwrap();
                    let mut req = self.http.request(method, &r.url);
                    for (k, v) in &r.headers {
                        req = req.header(k, v);
                    }
                    if let Some(b) = r.body.clone() {
                        req = req.body(b);
                    }
                    let answer = match req.send().await {
                        Ok(resp) => {
                            let status = resp.status().as_u16();
                            Ok(HttpResponse { status, body: resp.bytes().await.map(|b| b.to_vec()).unwrap_or_default() })
                        }
                        Err(e) => Err(e.to_string()),
                    };
                    self.live.on_http(&mut self.s, r.id, answer);
                }
                Action::Shell(m) => self.shell.push(m),
                Action::Notice { text, .. } => self.notices.push(text),
                Action::Alert(t) => self.alerts.push(t),
            }
        }
    }

    /// The document is shown and can be edited (a seeder waits for the room to take the seed).
    fn ready(&self) -> bool {
        self.loaded() && self.live.online() && gridcraft_engine::collab::read_only(&self.s).is_none()
    }

    fn loaded(&self) -> bool {
        self.shell.iter().any(|m| m["type"] == "craft-live:loaded")
    }

    fn wb(&self) -> std::sync::Arc<gridcraft_engine::model::Workbook> {
        let uid = self.live.doc().unwrap();
        self.s.documents().iter().find(|d| d.uid == uid).unwrap().wb.clone()
    }

    fn fp(&self) -> String {
        gridcraft_engine::collab::fingerprint(&self.wb())
    }

    fn set(&mut self, a1: &str, v: &str) {
        self.s.execute("cell.set", json!({"cell": a1, "input": v})).unwrap();
    }
}

/// Runs every tab's frames until `done` holds (or fails after 15 s).
async fn until(tabs: &mut [&mut Tab], what: &str, done: impl Fn(&[&mut Tab]) -> bool) {
    let end = Instant::now() + Duration::from_secs(15);
    while !done(tabs) {
        if Instant::now() >= end {
            for t in tabs.iter() {
                eprintln!("{:?} {:?}\n  shell {:?}\n  notices {:?}\n  alerts {:?}", t.live.phase(), t.live.summary(), t.shell, t.notices, t.alerts);
            }
            panic!("timed out waiting for: {what}");
        }
        for t in tabs.iter_mut() {
            t.frame().await;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

struct Env {
    fake: FakeNextcloud,
    public_url: String,
    http: reqwest::Client,
    file: u64,
    _server: craft_live::Running,
}

impl Env {
    async fn new(name: &str, bytes: &[u8]) -> Env {
        let fake = FakeNextcloud::start().await;
        let data = temp(name);
        let port = free_port();
        let apps = data.join("apps-gridcraft");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::write(apps.join("index.html"), "<!doctype html><title>GridCraft</title>").unwrap();
        let mut cfg = Config {
            listen: format!("127.0.0.1:{port}"),
            public_url: format!("http://127.0.0.1:{port}"),
            data_dir: data.join("data"),
            wopi_hosts: vec![fake.base()],
            proof_keys: true,
            ..Config::default()
        };
        cfg.apps.insert("gridcraft".into(), apps);
        // Saves happen when asked (Save, closing), so the test sees each one.
        cfg.autosave.updates = 100_000;
        cfg.autosave.idle_secs = 3600;
        cfg.autosave.max_secs = 3600;
        cfg.autosave.last_editor_idle_secs = 3600.0;
        cfg.validate().unwrap();
        let public_url = cfg.public_url.clone();
        let server = craft_live::start(cfg).await.unwrap();
        let http = reqwest::Client::new();
        let xml = http.get(format!("{public_url}/hosting/discovery")).send().await.unwrap().text().await.unwrap();
        fake.trust_discovery(&xml);
        let file = fake.add_file("ada", "/Budget.xlsx", bytes);
        Env { fake, public_url, http, file, _server: server }
    }

    /// Opens the editor page as Nextcloud does and reads the app frame's fragment from it.
    async fn open(&self, user: &str, write: bool) -> LiveConfig {
        let token = self.fake.token(user, &format!("{user} name"), self.file, write);
        let src = self.fake.wopi_src(self.file);
        let pct = gridcraft_engine::live::pct_encode;
        let url = format!("{}/edit/gridcraft?WOPISrc={}&lang=en&closebutton=1", self.public_url, pct(&src));
        let r = self.http.post(url).form(&[("access_token", token.as_str()), ("access_token_ttl", "0")]).send().await.unwrap();
        assert_eq!(r.status(), 200);
        let html = r.text().await.unwrap();
        // The app frame's `src`: index.html#craft-live=1&… (HTML-escaped in the attribute).
        let src = html.split("<iframe id=\"app\" src=\"").nth(1).and_then(|r| r.split('"').next()).unwrap().replace("&amp;", "&");
        let fragment = src.split_once('#').unwrap().1;
        let from_fragment = LiveConfig::from_fragment(fragment).unwrap().unwrap();
        // The shell's `craft-live:config` carries the same.
        let cfg: Json =
            serde_json::from_str(&html.split("id=\"craft-live-config\">").nth(1).unwrap().split("</script>").next().unwrap().replace("<\\/", "</"))
                .unwrap();
        let from_config = LiveConfig::from_json(&cfg["embed"]).unwrap();
        assert_eq!(from_fragment, from_config);
        from_fragment
    }
}

#[test]
fn gridcraft_edits_live_in_nextcloud_through_craft_live() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(2).build().unwrap();
    rt.block_on(async {
        let original = budget();
        let env = Env::new("main", &original).await;

        // Ada opens the file: she seeds the room from the WOPI file.
        let cfg = env.open("ada", true).await;
        assert!(cfg.seed && !cfg.view_only);
        assert_eq!((cfg.name.as_str(), cfg.format.as_str()), ("Budget.xlsx", "xlsx"));
        let mut ada = Tab::new(cfg);
        until(&mut [&mut ada], "Ada's document", |t| t[0].ready()).await;
        assert_eq!(cell(&ada.wb(), "B2"), "12");

        // Bob joins and gets the same workbook.
        let cfg = env.open("bob", true).await;
        assert!(!cfg.seed);
        let mut bob = Tab::new(cfg);
        until(&mut [&mut ada, &mut bob], "Bob's document", |t| t[1].ready()).await;
        assert_eq!(ada.fp(), bob.fp());
        assert_eq!(cell(&bob.wb(), "A1"), "Item");

        // Edits both ways.
        ada.set("C3", "=B2*2");
        until(&mut [&mut ada, &mut bob], "Ada's edit at Bob's", |t| cell(&t[1].wb(), "C3") == "=B2*2").await;
        bob.set("A4", "Total");
        until(&mut [&mut ada, &mut bob], "Bob's edit at Ada's", |t| cell(&t[0].wb(), "A4") == "Total").await;
        assert_eq!(ada.fp(), bob.fp());
        until(&mut [&mut ada, &mut bob], "the room modified", |t| t.iter().all(|t| t.live.modified() == Some(true))).await;
        // Each sees the other (awareness: the title bar's circles).
        until(&mut [&mut ada, &mut bob], "presence", |t| t[0].s.active_room().is_some_and(|r| r.peers().iter().any(|p| p.user.name == "bob name")))
            .await;

        // Bob presses Save: craft-live asks an editor for the file and PutFile writes it.
        bob.live.save_now();
        until(&mut [&mut ada, &mut bob], "the save", |t| t[1].shell.iter().any(|m| m["type"] == "craft-live:saved")).await;
        assert!(bob.shell.iter().any(|m| m["type"] == "craft-live:saved" && m["success"] == true), "{:?}", bob.shell);
        let f = env.fake.file(env.file);
        assert!(f.version > 1);
        let (wb, _) = gridcraft_engine::io::open_bytes("Budget.xlsx", &f.bytes).unwrap();
        assert_eq!(cell(&wb, "C3"), "=B2*2");
        assert_eq!(cell(&wb, "A4"), "Total");
        assert_eq!(cell(&wb, "B1"), "Cost");
        assert!(env.fake.log().iter().any(|l| l.starts_with("PutFile")), "{:?}", env.fake.log());
        until(&mut [&mut ada, &mut bob], "saved state", |t| t.iter().all(|t| t.live.modified() == Some(false))).await;

        // A viewer reads along and can't edit.
        let cfg = env.open("vic", false).await;
        assert!(cfg.view_only && !cfg.seed);
        let mut vic = Tab::new(cfg);
        until(&mut [&mut ada, &mut bob, &mut vic], "Vic's document", |t| t[2].loaded()).await;
        assert_eq!(vic.fp(), ada.fp());
        assert!(vic.s.execute("cell.set", json!({"cell": "A1", "input": "x"})).is_err());

        // Ada edits and closes: the room saves before she goes, then the shell hears close.
        ada.set("A5", "last word");
        until(&mut [&mut ada, &mut bob, &mut vic], "the last edit", |t| cell(&t[1].wb(), "A5") == "last word" && t[0].live.modified() == Some(true))
            .await;
        ada.live.close(&mut ada.s, true);
        until(&mut [&mut ada, &mut bob, &mut vic], "Ada closed", |t| t[0].live.phase() == Phase::Closed).await;
        assert!(ada.shell.iter().any(|m| m["type"] == "craft-live:close"));
        let (wb, _) = gridcraft_engine::io::open_bytes("Budget.xlsx", &env.fake.file(env.file).bytes).unwrap();
        assert_eq!(cell(&wb, "A5"), "last word");
        assert!(ada.alerts.is_empty() && bob.alerts.is_empty(), "{:?} {:?}", ada.alerts, bob.alerts);
        // Bob goes on editing with Vic.
        bob.set("A6", "after Ada");
        until(&mut [&mut bob, &mut vic], "Bob's edit at Vic's", |t| cell(&t[1].wb(), "A6") == "after Ada").await;
    });
}

#[test]
fn an_ods_file_saves_as_a_copy() {
    // GridCraft opens .ods but doesn't write it: craft-live keeps the .xlsx beside the file.
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(2).build().unwrap();
    rt.block_on(async {
        let env = Env::new("ods", &[]).await;
        {
            let mut st = env.fake.st();
            let f = st.files.get_mut(&env.file).unwrap();
            f.path = "/Plan.ods".into();
        }
        let cfg = env.open("ada", true).await;
        assert_eq!(cfg.format, "ods");
        let mut ada = Tab::new(cfg);
        until(&mut [&mut ada], "Ada's document", |t| t[0].ready()).await;
        ada.set("A1", "from GridCraft");
        until(&mut [&mut ada], "modified", |t| t[0].live.modified() == Some(true)).await;
        ada.live.save_now();
        until(&mut [&mut ada], "a copy", |t| t[0].notices.iter().any(|n| n.contains(".xlsx"))).await;
        let st = env.fake.st();
        assert!(st.files.get(&env.file).unwrap().bytes.is_empty(), "the .ods itself is untouched");
        let copy = st.files.values().find(|f| f.path.ends_with(".xlsx")).expect("a copy");
        let (wb, _) = gridcraft_engine::io::open_bytes("copy.xlsx", &copy.bytes).unwrap();
        assert_eq!(cell(&wb, "A1"), "from GridCraft");
    });
}
