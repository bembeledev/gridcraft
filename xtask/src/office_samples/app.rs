//! The app the driver talks to: the real desktop app over its control channel
//! (`--control <port>`, docs/control-protocol.md), or a fake one for `--dry-run` and the tests.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value as Json, json};

/// Why a call didn't answer a result.
#[derive(Clone, Debug, PartialEq)]
pub enum CallError {
    /// The app answered an error.
    Answer(String),
    /// The app is gone (exited, crashed) or stopped answering.
    Crashed(String),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::Answer(e) => write!(f, "{e}"),
            CallError::Crashed(e) => write!(f, "the app crashed: {e}"),
        }
    }
}

/// A running app.
pub trait App {
    /// One control-channel request: its `result`, or why there's none.
    fn call(&mut self, method: &str, params: Json) -> Result<Json, CallError>;
    /// Quits the app; a note if it had already died.
    fn shutdown(&mut self) -> Option<String>;
}

/// Starts apps: a fresh one per scenario (and after a crash).
pub trait Launcher {
    fn launch(&mut self, run: usize) -> Result<Box<dyn App>, String>;
    /// Milliseconds to sleep between polls (0 for the fake app).
    fn poll_ms(&self) -> u64;
}

// ---------------------------------------------------------------- the real app

/// How long the app may take to open its control channel.
const START_TIMEOUT: Duration = Duration::from_secs(90);
/// The control server gives up on the UI after 60 s; a little more here.
const CALL_TIMEOUT: Duration = Duration::from_secs(75);

/// Starts the desktop app with a control port, on a profile of its own.
pub struct ProcessLauncher {
    pub app: PathBuf,
    pub out: PathBuf,
    /// `GRIDCRAFT_OFFICE_JS` for the app (the built library), if given.
    pub office_js: Option<PathBuf>,
}

/// A free localhost port (the OS picks it; the app binds it right after).
fn free_port() -> Result<u16, String> {
    let l = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("no free port: {e}"))?;
    l.local_addr().map(|a| a.port()).map_err(|e| e.to_string())
}

impl Launcher for ProcessLauncher {
    fn launch(&mut self, run: usize) -> Result<Box<dyn App>, String> {
        let port = free_port()?;
        // A fresh settings folder: the app keeps preferences, logs and plugins there.
        let profile = self.out.join("profiles").join(format!("run-{run}"));
        std::fs::create_dir_all(&profile).map_err(|e| format!("{}: {e}", profile.display()))?;
        let log_path = self.out.join("logs").join(format!("app-run-{run}.log"));
        if let Some(dir) = log_path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let log = std::fs::File::create(&log_path).map_err(|e| format!("{}: {e}", log_path.display()))?;
        let log_err = log.try_clone().map_err(|e| e.to_string())?;
        let mut cmd = Command::new(&self.app);
        cmd.arg("--control")
            .arg(port.to_string())
            .env("GRIDCRAFT_NO_PREFS", "1")
            .env("HOME", &profile)
            .env("APPDATA", &profile)
            .env("XDG_CONFIG_HOME", &profile)
            .env_remove("GRIDCRAFT_CONTROL_PORT")
            .env_remove("GRIDCRAFT_PLUGIN_PATH")
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err));
        if let Some(dir) = &self.office_js {
            cmd.env("GRIDCRAFT_OFFICE_JS", dir);
        }
        let child = cmd.spawn().map_err(|e| format!("{}: {e}", self.app.display()))?;
        let mut app = Process { child, conn: None, next_id: 0, log: log_path, dead: None };
        app.connect(SocketAddr::from(([127, 0, 0, 1], port)))?;
        Ok(Box::new(app))
    }
    fn poll_ms(&self) -> u64 {
        250
    }
}

struct Process {
    child: Child,
    conn: Option<(BufReader<TcpStream>, TcpStream)>,
    next_id: u64,
    log: PathBuf,
    dead: Option<String>,
}

impl Process {
    fn connect(&mut self, addr: SocketAddr) -> Result<(), String> {
        let started = Instant::now();
        loop {
            if let Some(why) = self.exited() {
                return Err(format!("the app exited before its control channel opened ({why}); see {}", self.log.display()));
            }
            if let Ok(s) = TcpStream::connect_timeout(&addr, Duration::from_secs(1)) {
                s.set_read_timeout(Some(CALL_TIMEOUT)).map_err(|e| e.to_string())?;
                let read = s.try_clone().map_err(|e| e.to_string())?;
                self.conn = Some((BufReader::new(read), s));
                return Ok(());
            }
            if started.elapsed() > START_TIMEOUT {
                let _ = self.child.kill();
                return Err(format!("no control channel on {addr} after {} s; see {}", START_TIMEOUT.as_secs(), self.log.display()));
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// How the process ended, if it has.
    fn exited(&mut self) -> Option<String> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(format!("exit status {status}")),
            Ok(None) => None,
            Err(e) => Some(e.to_string()),
        }
    }

    fn died(&mut self, what: String) -> CallError {
        // Give a crashing process a moment to finish dying, so the report says how it ended.
        for _ in 0..20 {
            if let Some(why) = self.exited() {
                let msg = format!("{what} ({why}); see {}", self.log.display());
                self.dead = Some(msg.clone());
                self.conn = None;
                return CallError::Crashed(msg);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        let msg = format!("{what} (still running: killed); see {}", self.log.display());
        self.dead = Some(msg.clone());
        self.conn = None;
        CallError::Crashed(msg)
    }
}

impl App for Process {
    fn call(&mut self, method: &str, params: Json) -> Result<Json, CallError> {
        if let Some(d) = &self.dead {
            return Err(CallError::Crashed(d.clone()));
        }
        self.next_id += 1;
        let line = json!({"id": self.next_id, "method": method, "params": params}).to_string();
        let Some((reader, writer)) = self.conn.as_mut() else { return Err(CallError::Crashed("not connected".into())) };
        if writeln!(writer, "{line}").and_then(|()| writer.flush()).is_err() {
            return Err(self.died(format!("{method}: the control channel closed")));
        }
        let mut answer = String::new();
        match reader.read_line(&mut answer) {
            Ok(0) => return Err(self.died(format!("{method}: the control channel closed"))),
            Ok(_) => {}
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                return Err(self.died(format!("{method}: no answer in {} s (hung)", CALL_TIMEOUT.as_secs())));
            }
            Err(e) => return Err(self.died(format!("{method}: {e}"))),
        }
        parse_answer(&answer)
    }

    fn shutdown(&mut self) -> Option<String> {
        if let Some(d) = self.dead.take() {
            return Some(d);
        }
        let _ = self.call("app.quit", json!({}));
        for _ in 0..100 {
            if self.exited().is_some() {
                return None;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        None
    }
}

/// A control-channel answer line: its `result`, or its `error`.
pub fn parse_answer(line: &str) -> Result<Json, CallError> {
    let v: Json = serde_json::from_str(line.trim()).map_err(|e| CallError::Answer(format!("unreadable answer: {e}")))?;
    if v.get("ok").and_then(Json::as_bool) == Some(true) {
        Ok(v.get("result").cloned().unwrap_or(Json::Null))
    } else {
        Err(CallError::Answer(v.get("error").and_then(Json::as_str).unwrap_or("error").to_string()))
    }
}

// ---------------------------------------------------------------- the fake app

/// Starts [`FakeApp`]s.
#[derive(Default)]
pub struct FakeLauncher {
    /// The fake crashes on this call number of the run with this index (tests).
    pub crash: Option<(usize, usize)>,
    /// The samples' effects never happen (tests of failure reports).
    pub broken: bool,
}

impl Launcher for FakeLauncher {
    fn launch(&mut self, run: usize) -> Result<Box<dyn App>, String> {
        let crash_at = self.crash.filter(|(r, _)| *r == run).map(|(_, n)| n);
        Ok(Box::new(FakeApp { crash_at, broken: self.broken, ..FakeApp::default() }))
    }
    fn poll_ms(&self) -> u64 {
        0
    }
}

/// A stand-in for the desktop app that knows what each sample's scripts do (they are the
/// constants in `scenarios`): enough to run every scenario without a web view.
#[derive(Default)]
pub struct FakeApp {
    calls: usize,
    crash_at: Option<usize>,
    crashed: bool,
    broken: bool,
    /// Loaded add-ins: id, manifest path, blocked by.
    plugins: Vec<(String, String, Option<String>)>,
    /// Open panes: key, plugin, polls until ready.
    panes: Vec<(String, String, u32)>,
    function_page: Option<(String, String)>,
    cells: std::collections::BTreeMap<String, (String, String)>,
    selection: String,
    sheets: usize,
    settings: Option<String>,
    banner: String,
}

impl FakeApp {
    fn cell(&mut self, a: &str) -> &mut (String, String) {
        self.cells.entry(a.to_string()).or_default()
    }

    fn eval(&mut self, script: &str) -> Result<Json, CallError> {
        use super::scenarios as s;
        if script == s::INFO {
            return Ok(json!({"host": "Excel", "osf": "undefined",
                "info": {"bootstrap": true, "library": true, "officeIsCraft": true, "blockedScripts": ["https://officeapis.public.onecdn.static.microsoft/1/office.js"]}}));
        }
        if self.broken {
            return Ok(json!(false));
        }
        let first = self.selection.split(':').next().unwrap_or("A1").to_string();
        match script {
            s::HELLO_READY | s::COMMANDS_READY | s::SETTINGS_READY | s::SHORTCUTS_READY => Ok(json!(true)),
            s::HELLO_CLICK => {
                self.cell("A1").0 = "Hello world!".into();
                Ok(json!(true))
            }
            s::COMMANDS_CLICK => {
                for a in fake_range(&self.selection) {
                    self.cell(&a).1 = "#FFFF00".into();
                }
                Ok(json!(true))
            }
            s::SETTINGS_SAVE => {
                self.settings = Some(s::SETTING_VALUE.into());
                self.banner = format!("Saved setting KEY: {} VALUE: {}", s::SETTING_KEY, s::SETTING_VALUE);
                Ok(json!(true))
            }
            s::SETTINGS_BANNER => Ok(json!(self.banner)),
            s::SETTINGS_STORED => Ok(json!(self.settings)),
            s::SETTINGS_GET => {
                self.banner = format!("Retrieved setting information KEY: {} VALUE: {}", s::SETTING_KEY, self.settings.clone().unwrap_or_default());
                Ok(json!(true))
            }
            s::SHORTCUTS_ADD_NEW => {
                self.sheets += 1;
                Ok(json!(true))
            }
            s::SHORTCUTS_RUN_ACTION => {
                self.cell(&first).1 = "#C7CC7A".into();
                Ok(json!(true))
            }
            s::SHORTCUTS_REPLACE | s::SHORTCUTS_SHOW | s::CUSTOM_FUNCTIONS => Ok(json!("unsupported: NotImplemented")),
            _ => Err(CallError::Answer(format!("the fake app doesn't know the script {script:?}"))),
        }
    }
}

/// The cells of `B2:C3` (two letters and digits at most: enough for the fake).
fn fake_range(r: &str) -> Vec<String> {
    let parse = |a: &str| -> Option<(u8, u32)> {
        let col = *a.as_bytes().first()?;
        Some((col, a.get(1..)?.parse().ok()?))
    };
    let (from, to) = r.split_once(':').unwrap_or((r, r));
    let (Some((c1, r1)), Some((c2, r2))) = (parse(from), parse(to)) else { return Vec::new() };
    let mut out = Vec::new();
    for c in c1..=c2 {
        for row in r1..=r2 {
            out.push(format!("{}{row}", char::from(c)));
        }
    }
    out
}

impl App for FakeApp {
    fn call(&mut self, method: &str, params: Json) -> Result<Json, CallError> {
        if self.crashed {
            return Err(CallError::Crashed("the fake app crashed".into()));
        }
        self.calls += 1;
        if self.crash_at == Some(self.calls) {
            self.crashed = true;
            return Err(CallError::Crashed("the fake app crashed (as asked)".into()));
        }
        if self.sheets == 0 {
            self.sheets = 1;
            self.selection = "A1".into();
        }
        let p = |k: &str| params.get(k).and_then(Json::as_str).unwrap_or("").to_string();
        match method {
            "document.inspect" => {
                let sheets: Vec<Json> = (1..=self.sheets).map(|i| json!({"name": format!("Sheet{i}")})).collect();
                Ok(json!({"title": "Book1", "sheets": sheets, "selection": self.selection}))
            }
            "plugins.load" => {
                let path = p("path");
                if !Path::new(&path).is_file() && Path::new(&path).is_absolute() {
                    return Err(CallError::Answer(format!("{path}: No such file or directory")));
                }
                let id = format!("office.fake-{}", self.plugins.len() + 1);
                let blocked = path.contains("custom-functions").then(|| "Needs SharedRuntime 1.1, which GridCraft doesn't support".to_string());
                let state = if blocked.is_some() { "blocked" } else { "loaded" };
                let blocked_by: Vec<Json> = blocked.iter().map(|m| json!({"kind": "feature", "message": m})).collect();
                self.plugins.push((id.clone(), path, blocked));
                Ok(json!({"id": id, "name": "Fake add-in", "state": state, "blockedBy": blocked_by, "kind": "office"}))
            }
            "office.open" => {
                let plugin = p("plugin");
                if !self.plugins.iter().any(|(id, _, b)| *id == plugin && b.is_none()) {
                    return Err(CallError::Answer(format!("no plugin {plugin}")));
                }
                let key = format!("{plugin}#pane");
                if !self.panes.iter().any(|(k, ..)| *k == key) {
                    self.panes.push((key.clone(), plugin, 2));
                }
                Ok(json!({"ran": 0, "pane": key}))
            }
            "office.close" => {
                let pane = p("pane");
                self.panes.retain(|(k, ..)| *k != pane);
                Ok(json!({"closed": pane}))
            }
            "office.panes" => {
                let mut out = Vec::new();
                for (key, plugin, polls) in &mut self.panes {
                    *polls = polls.saturating_sub(1);
                    out.push(json!({"key": key, "plugin": plugin, "url": "https://fake.example/pane.html", "kind": "pane", "visible": true, "ready": *polls == 0}));
                }
                if let Some((key, plugin)) = &self.function_page {
                    out.push(json!({"key": key, "plugin": plugin, "url": "https://fake.example/fn.html", "kind": "function", "visible": false, "ready": true}));
                }
                Ok(Json::Array(out))
            }
            "office.eval" => {
                let pane = p("pane");
                if !self.panes.iter().any(|(k, _, polls)| *k == pane && *polls == 0) {
                    return Err(CallError::Answer("no such pane (see `office.panes`)".into()));
                }
                self.eval(&p("script"))
            }
            "selection.set" => {
                self.selection = p("range");
                Ok(json!({}))
            }
            "plugins.run" => {
                let plugin = p("plugin");
                if !self.broken && p("command") == "itemExecuteFunction" {
                    let first = self.selection.split(':').next().unwrap_or("A1").to_string();
                    self.cell(&first).0 = "ExecuteFunction works. Button ID=itemExecuteFunction".into();
                    self.function_page = Some((format!("{plugin}#functions"), plugin));
                }
                Ok(json!({"ran": 0}))
            }
            "cell.get" => {
                let a = p("cell");
                let (value, fill) = self.cells.get(&a).cloned().unwrap_or_default();
                let style = if fill.is_empty() { json!({}) } else { json!({"fill": {"pattern": "Solid", "fg": fill}}) };
                Ok(json!({"cell": a, "value": if value.is_empty() { Json::Null } else { json!(value) }, "style": style}))
            }
            "ui.screenshot" => {
                let path = p("path");
                // A 1×1 PNG is enough to show where the picture would be.
                std::fs::write(&path, FAKE_PNG).map_err(|e| CallError::Answer(e.to_string()))?;
                Ok(json!({"path": path, "width": 1, "height": 1}))
            }
            "app.quit" => Ok(Json::Null),
            other => Err(CallError::Answer(format!("unknown method `{other}`"))),
        }
    }
    fn shutdown(&mut self) -> Option<String> {
        self.crashed.then(|| "the fake app had crashed".to_string())
    }
}

/// A transparent 1×1 PNG.
const FAKE_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
    0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];
