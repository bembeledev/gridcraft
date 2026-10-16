//! Office add-ins' task panes: the right-hand panel an Office add-in's page shows in, and the
//! messages between that page and the workbook.
//!
//! The page itself is a web view the host draws over the panel ([`OfficeHost`], the desktop
//! app's `wry` web view; none in tests or where there's no web view). The page runs GridCraft's
//! Office.js (`crates/office-js`), which sends each `context.sync()` as
//! `{"id": n, "actions": [...]}`; [`pump`] runs it with `office.sync` and answers
//! `{"id": n, "results"|"error": …}`. Native web views are drawn above the window's content, so
//! the page is hidden while the File tab or a dialog is open.
//!
//! The library also sends events: `{"event": "ready"}` once it has started in a page (function
//! commands wait for it), and `{"event": "functionDone", "call": n, "error"?}` when a function
//! command's function calls `event.completed()` (or fails). The host calls into the page with
//! `__craftOfficeRunFunction({"call", "name", "control"})` and `__craftOfficeEvent({"type"})`
//! (`documentSelectionChanged`).
//!
//! Tests and agents look into the pages through the control channel: `office.panes` lists them
//! ([`list`]) and `office.eval` runs a script in one ([`eval`]): the host runs [`eval_script`],
//! which posts `{"craftEval": n, "ok", "json"|"error"}` back once the script's value (awaited if
//! it is a Promise) is known; [`EvalRouter`] hands those answers to the waiting caller.

use egui::{Ui, vec2};
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::theme::{self, Tokens};

/// Shows add-in pages: implemented by the desktop app with its web view.
pub trait OfficeHost {
    /// Shows the page for `key` (loading `url` the first time) over `rect`, in points.
    fn show(&mut self, key: &str, url: &str, rect: egui::Rect, pixels_per_point: f32);
    /// Hides the page (it keeps its state).
    fn hide(&mut self, key: &str);
    /// Closes the page.
    fn close(&mut self, key: &str);
    /// Loads `url` for `key` without showing it (a function file's page, for `ExecuteFunction`
    /// commands); nothing if that page is already there.
    fn load_hidden(&mut self, key: &str, url: &str);
    /// The messages pages sent since the last call: (key, JSON).
    fn take_messages(&mut self) -> Vec<(String, String)>;
    /// Answers the page for `key` (`__craftOfficeReply(json)`).
    fn reply(&mut self, key: &str, json: &str);
    /// Calls the library's `globalThis.<entry>(json)` in the page for `key`
    /// (`__craftOfficeRunFunction`, `__craftOfficeEvent`).
    fn call(&mut self, key: &str, entry: &str, json: &str);
    /// Runs `script` in the page for `key` (wrapped by [`eval_script`]; [`EvalRouter`] does the
    /// bookkeeping) and calls `reply` once, with the JSON text of its value or an error. A host
    /// that can't run scripts answers an error.
    fn eval(&mut self, key: &str, script: &str, reply: EvalReply) {
        let _ = (key, script);
        reply(Err("this host can't run scripts in add-in pages".into()));
    }
}

/// Where an `office.eval` answer goes: called once with the value's JSON text, or an error.
pub type EvalReply = Box<dyn FnOnce(Result<String, String>) + Send>;

/// Longest JSON answer `office.eval` passes on.
pub const MAX_EVAL_RESULT: usize = 1024 * 1024;
/// Longest script `office.eval` runs.
pub const MAX_EVAL_SCRIPT: usize = 256 * 1024;
/// Most `office.eval` calls waiting at once (more are refused).
const MAX_EVALS: usize = 256;

/// The script a host runs for `office.eval` number `id`: evaluates `script` in the page's global
/// scope as the web inspector's console does (the value of its last statement; a Promise is
/// awaited) and posts `{"craftEval": id, "ok": true, "json": "<value as JSON>"}` or
/// `{"craftEval": id, "ok": false, "error": "…"}` through the page's channel to the host. A page
/// whose Content-Security-Policy forbids `eval` answers an error.
pub fn eval_script(id: u64, script: &str) -> String {
    let quoted = serde_json::to_string(script).unwrap_or_else(|_| "\"\"".into());
    format!(
        "(function(){{var post=function(m){{var s=JSON.stringify(m);var c=globalThis.__craftOffice;\
if(c&&c.post){{c.post(s);}}else if(window.ipc&&window.ipc.postMessage){{window.ipc.postMessage(s);}}}};\
Promise.resolve().then(function(){{return (0,eval)({quoted});}}).then(function(v){{var t;\
try{{t=JSON.stringify(v===undefined?null:v);}}catch(e){{throw new Error(\"the value isn't JSON: \"+e);}}\
post({{craftEval:{id},ok:true,json:t===undefined?\"null\":t}});}}).catch(function(e){{\
post({{craftEval:{id},ok:false,error:String(e&&e.stack?e.stack:e).slice(0,4000)}});}});}})();"
    )
}

/// An answer [`eval_script`] posted: its id and the value's JSON text or the error.
pub fn eval_answer(text: &str) -> Option<(u64, Result<String, String>)> {
    if !text.contains("craftEval") {
        return None;
    }
    let msg: Json = serde_json::from_str(text).ok()?;
    let id = msg.get("craftEval")?.as_u64()?;
    if msg.get("ok").and_then(Json::as_bool) == Some(true) {
        let json = msg.get("json").and_then(Json::as_str).unwrap_or("null");
        if json.len() > MAX_EVAL_RESULT {
            return Some((id, Err(format!("the value is larger than {} KiB", MAX_EVAL_RESULT / 1024))));
        }
        Some((id, Ok(json.to_string())))
    } else {
        let e: String = msg.get("error").and_then(Json::as_str).unwrap_or("the script failed").chars().take(4000).collect();
        Some((id, Err(e)))
    }
}

/// A host's `office.eval` calls waiting for their page's answer.
#[derive(Default)]
pub struct EvalRouter {
    next: u64,
    waiting: Vec<(u64, String, EvalReply)>,
}

impl EvalRouter {
    /// Registers a call to the page for `key`: the script to run there, or `None` (and `reply`
    /// has its error) when too many are waiting.
    pub fn start(&mut self, key: &str, script: &str, reply: EvalReply) -> Option<String> {
        if self.waiting.len() >= MAX_EVALS {
            reply(Err("too many scripts are waiting for add-in pages".into()));
            return None;
        }
        self.next = self.next.wrapping_add(1);
        self.waiting.push((self.next, key.to_string(), reply));
        Some(eval_script(self.next, script))
    }
    /// Answers the calls whose answers are among `messages` (from the page they were sent to),
    /// and returns the other messages.
    pub fn route(&mut self, messages: Vec<(String, String)>) -> Vec<(String, String)> {
        let mut rest = Vec::with_capacity(messages.len());
        for (key, text) in messages {
            match eval_answer(&text) {
                Some((id, answer)) => {
                    if let Some(i) = self.waiting.iter().position(|(n, k, _)| *n == id && *k == key) {
                        let (_, _, reply) = self.waiting.remove(i);
                        reply(answer);
                    }
                }
                None => rest.push((key, text)),
            }
        }
        rest
    }
    /// The page for `key` is gone: its calls fail with `why`.
    pub fn fail(&mut self, key: &str, why: &str) {
        let (gone, keep): (Vec<_>, Vec<_>) = self.waiting.drain(..).partition(|(_, k, _)| k == key);
        self.waiting = keep;
        for (_, _, reply) in gone {
            reply(Err(why.to_string()));
        }
    }
    /// Calls still waiting.
    pub fn len(&self) -> usize {
        self.waiting.len()
    }
    pub fn is_empty(&self) -> bool {
        self.waiting.is_empty()
    }
}

/// An `office.eval` the control channel is waiting on.
struct PendingEval {
    token: u64,
    reply: std::sync::mpsc::Sender<crate::control::ControlResponse>,
    deadline_ms: f64,
}

/// The control channel's `office.eval` calls: their answers arrive from the host, maybe from
/// another thread, into `done`; [`finish_evals`] passes them on, or gives up at the deadline.
#[derive(Default)]
pub struct Evals {
    token: u64,
    pending: Vec<PendingEval>,
    done: std::sync::Arc<std::sync::Mutex<Vec<(u64, Result<String, String>)>>>,
}

impl Evals {
    pub fn waiting(&self) -> bool {
        !self.pending.is_empty()
    }
}

/// How long a function command may run before GridCraft stops waiting for its
/// `event.completed()` and says so.
pub const FUNCTION_TIMEOUT_MS: f64 = 5.0 * 60.0 * 1000.0;

/// A function command waiting for its page, or for its `event.completed()`.
#[derive(Clone, Debug, PartialEq)]
pub struct FunctionCall {
    pub id: u64,
    /// The function page.
    pub key: String,
    pub plugin: String,
    pub function: String,
    /// The manifest's control id (`event.source.id`).
    pub control: String,
    /// When it was asked for, and when it was handed to the page (ms, [`crate::now_ms`]).
    pub asked_ms: f64,
    pub sent_ms: Option<f64>,
}

/// An open task pane.
#[derive(Clone, Debug, PartialEq)]
pub struct OfficePane {
    /// The add-in (plugin id).
    pub plugin: String,
    pub title: String,
    pub url: String,
    /// Identifies the page to the host: the add-in, and its pane id or page.
    pub key: String,
}

/// The add-ins' panes and function pages.
#[derive(Default)]
pub struct Panes {
    pub open: Vec<OfficePane>,
    /// Which of `open` shows.
    pub current: usize,
    /// Pages running a function command (no pane): key, add-in, function file URL.
    pub function_pages: Vec<(String, String, String)>,
    /// Function commands not finished yet.
    pub calls: Vec<FunctionCall>,
    next_call: u64,
    /// Pages whose library has started (`{"event": "ready"}`).
    pub ready: std::collections::BTreeSet<String>,
    /// The selection last told to the pages (document, sheet, selection).
    selection: Option<(u64, usize, gridcraft_engine::Selection)>,
    /// Keys shown last frame, to hide them when they stop showing.
    shown: Option<String>,
    /// `office.eval` calls waiting for their page.
    pub evals: Evals,
}

/// Most function commands waiting at once (more are refused).
const MAX_CALLS: usize = 100;

fn key(plugin: &str, pane: Option<&str>, url: &str) -> String {
    format!("{plugin}#{}", pane.unwrap_or(url))
}

/// Opens (or brings forward) an add-in's task pane.
pub fn open(app: &mut SheetApp, plugin: String, title: String, url: String, pane: Option<String>) {
    let k = key(&plugin, pane.as_deref(), &url);
    match app.office.open.iter().position(|p| p.key == k) {
        Some(i) => {
            if let Some(p) = app.office.open.get_mut(i) {
                p.url = url;
            }
            app.office.current = i;
        }
        None => {
            app.office.open.push(OfficePane { plugin, title, url, key: k });
            app.office.current = app.office.open.len() - 1;
        }
    }
}

/// Runs an add-in's function command in a page with no pane: loads its function file (hidden)
/// and, once the library there is ready, calls the function with `event.source.id` = `control`.
pub fn run_function(app: &mut SheetApp, plugin: String, url: String, function: String, control: String) {
    let k = key(&plugin, Some("functions"), &url);
    let Some(host) = app.services.office.as_mut() else {
        app.message = Some(("Office Add-in".into(), "Office add-ins need the GridCraft desktop app on macOS or Windows.".into()));
        return;
    };
    if app.office.calls.len() >= MAX_CALLS {
        app.toast = Some((format!("{function}: too many add-in commands are still running"), crate::now_ms()));
        return;
    }
    host.load_hidden(&k, &url);
    if !app.office.function_pages.iter().any(|(fk, ..)| *fk == k) {
        app.office.function_pages.push((k.clone(), plugin.clone(), url.clone()));
    }
    app.office.next_call = app.office.next_call.wrapping_add(1);
    let id = app.office.next_call;
    app.office.calls.push(FunctionCall { id, key: k, plugin, function, control, asked_ms: crate::now_ms(), sent_ms: None });
    send_calls(app);
}

/// Hands waiting function calls to the pages that are ready.
fn send_calls(app: &mut SheetApp) {
    let Some(host) = app.services.office.as_mut() else { return };
    let now = crate::now_ms();
    for c in app.office.calls.iter_mut().filter(|c| c.sent_ms.is_none() && app.office.ready.contains(&c.key)) {
        let call = json!({"call": c.id, "name": c.function, "control": c.control}).to_string();
        host.call(&c.key, "__craftOfficeRunFunction", &call);
        c.sent_ms = Some(now);
    }
}

/// Stops waiting for function commands asked for more than [`FUNCTION_TIMEOUT_MS`] before
/// `now_ms`, and says which one didn't finish.
pub fn expire_calls(app: &mut SheetApp, now_ms: f64) {
    let (late, keep): (Vec<FunctionCall>, Vec<FunctionCall>) = app.office.calls.drain(..).partition(|c| now_ms - c.asked_ms > FUNCTION_TIMEOUT_MS);
    app.office.calls = keep;
    if let Some(c) = late.first() {
        let name = gridcraft_engine::plugin::get(&c.plugin).map(|p| p.manifest.name.clone()).unwrap_or_else(|| c.plugin.clone());
        let what = if c.sent_ms.is_some() { "didn't finish (it never called event.completed())" } else { "couldn't start (its page didn't load)" };
        app.toast = Some((format!("{name}: {} {what}", c.function), now_ms));
    }
}

/// An event from a page's library (not a `context.sync()`): true if `msg` was one.
fn event(app: &mut SheetApp, key: &str, msg: &Json) -> bool {
    let Some(kind) = msg.get("event").and_then(Json::as_str) else { return false };
    match kind {
        "ready" => {
            app.office.ready.insert(key.to_string());
            send_calls(app);
        }
        "functionDone" => {
            let id = msg.get("call").and_then(Json::as_u64);
            if let Some(i) = app.office.calls.iter().position(|c| Some(c.id) == id && c.key == key) {
                let c = app.office.calls.remove(i);
                if let Some(e) = msg.get("error").and_then(Json::as_str) {
                    let e: String = e.chars().take(300).collect();
                    app.toast = Some((format!("{}: {e}", c.function), crate::now_ms()));
                }
            }
        }
        _ => {}
    }
    true
}

/// Tells the ready pages that the selection changed (`Office.EventType.DocumentSelectionChanged`).
fn selection_events(app: &mut SheetApp) {
    let now = app.session.active().map(|d| (d.uid, d.wb.active_sheet, d.selection.clone()));
    if now == app.office.selection {
        return;
    }
    let first = app.office.selection.is_none();
    app.office.selection = now;
    if first || app.office.ready.is_empty() {
        return;
    }
    let Some(host) = app.services.office.as_mut() else { return };
    let ev = json!({"type": "documentSelectionChanged"}).to_string();
    for k in &app.office.ready {
        host.call(k, "__craftOfficeEvent", &ev);
    }
}

/// The panes and function pages, for `office.panes`: `{key, plugin, url, title, kind: "pane" |
/// "function", visible, ready, current}`.
pub fn list(app: &SheetApp) -> Json {
    let o = &app.office;
    let panes = o.open.iter().enumerate().map(|(i, p)| {
        json!({
            "key": p.key, "plugin": p.plugin, "url": p.url, "title": p.title, "kind": "pane",
            "visible": o.shown.as_deref() == Some(p.key.as_str()), "ready": o.ready.contains(&p.key), "current": i == o.current,
        })
    });
    let functions = o.function_pages.iter().map(|(k, plugin, url)| {
        json!({"key": k, "plugin": plugin, "url": url, "title": "", "kind": "function", "visible": false, "ready": o.ready.contains(k), "current": false})
    });
    Json::Array(panes.chain(functions).collect())
}

/// The page `pane` names: a key from [`list`], or an add-in (plugin id): its pane showing, else
/// its first pane, else its function page.
pub fn find_page(app: &SheetApp, pane: &str) -> Option<String> {
    let o = &app.office;
    if o.open.iter().any(|p| p.key == pane) || o.function_pages.iter().any(|(k, ..)| k == pane) {
        return Some(pane.to_string());
    }
    let current = o.open.get(o.current).filter(|p| p.plugin == pane);
    current
        .or_else(|| o.open.iter().find(|p| p.plugin == pane))
        .map(|p| p.key.clone())
        .or_else(|| o.function_pages.iter().find(|(_, p, _)| p == pane).map(|(k, ..)| k.clone()))
}

/// Runs `script` in the page `key` for the control channel; `reply` gets
/// `{"ok": true, "result": <value>}` or `{"ok": false, "error"}` from [`finish_evals`], at the
/// latest `timeout_ms` from now.
pub fn eval(app: &mut SheetApp, key: &str, script: &str, timeout_ms: f64, reply: std::sync::mpsc::Sender<crate::control::ControlResponse>) {
    let Some(host) = app.services.office.as_mut() else {
        let _ = reply.send(json!({"ok": false, "error": "Office add-ins need the GridCraft desktop app on macOS or Windows"}));
        return;
    };
    if app.office.evals.pending.len() >= MAX_EVALS {
        let _ = reply.send(json!({"ok": false, "error": "too many scripts are waiting for add-in pages"}));
        return;
    }
    app.office.evals.token = app.office.evals.token.wrapping_add(1);
    let token = app.office.evals.token;
    app.office.evals.pending.push(PendingEval { token, reply, deadline_ms: crate::now_ms() + timeout_ms });
    let done = app.office.evals.done.clone();
    host.eval(
        key,
        script,
        Box::new(move |answer| {
            let mut q = done.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            q.push((token, answer));
        }),
    );
}

/// Passes on the `office.eval` answers that arrived, and fails the calls past their deadline.
pub fn finish_evals(app: &mut SheetApp, now_ms: f64) {
    let evals = &mut app.office.evals;
    if evals.pending.is_empty() {
        return;
    }
    let arrived = std::mem::take(&mut *evals.done.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
    for (token, answer) in arrived {
        let Some(i) = evals.pending.iter().position(|p| p.token == token) else { continue };
        let p = evals.pending.remove(i);
        let reply = match answer {
            Ok(text) if text.len() > MAX_EVAL_RESULT => {
                json!({"ok": false, "error": format!("the value is larger than {} KiB", MAX_EVAL_RESULT / 1024)})
            }
            Ok(text) => match serde_json::from_str::<Json>(&text) {
                Ok(v) => json!({"ok": true, "result": v}),
                Err(e) => json!({"ok": false, "error": format!("the page's answer isn't JSON: {e}")}),
            },
            Err(e) => json!({"ok": false, "error": e}),
        };
        let _ = p.reply.send(reply);
    }
    evals.pending.retain(|p| {
        if now_ms < p.deadline_ms {
            return true;
        }
        let _ = p.reply.send(
            json!({"ok": false, "error": "the page didn't answer before the timeout (still loading, navigated away, or the script never settled)"}),
        );
        false
    });
}

/// Closes the pane `key` (`office.close`): false if no pane has that key.
pub fn close_key(app: &mut SheetApp, key: &str) -> bool {
    match app.office.open.iter().position(|p| p.key == key) {
        Some(i) => {
            close(app, i);
            true
        }
        None => false,
    }
}

/// Closes the pane at `i`.
fn close(app: &mut SheetApp, i: usize) {
    if i >= app.office.open.len() {
        return;
    }
    let pane = app.office.open.remove(i);
    if let Some(host) = app.services.office.as_mut() {
        host.close(&pane.key);
    }
    app.office.ready.remove(&pane.key);
    if app.office.shown.as_deref() == Some(pane.key.as_str()) {
        app.office.shown = None;
    }
    app.office.current = app.office.current.min(app.office.open.len().saturating_sub(1));
}

/// Answers the pages' messages (each `context.sync()` is one `office.sync`), takes their events,
/// tells them of selection changes, and gives up on function commands that never finished.
pub fn pump(app: &mut SheetApp) {
    let Some(host) = app.services.office.as_mut() else { return };
    let messages = host.take_messages();
    for (k, text) in messages {
        if let Ok(msg) = serde_json::from_str::<Json>(&text)
            && event(app, &k, &msg)
        {
            continue;
        }
        let plugin = app
            .office
            .open
            .iter()
            .find(|p| p.key == k)
            .map(|p| p.plugin.clone())
            .or_else(|| app.office.function_pages.iter().find(|(fk, ..)| *fk == k).map(|(_, p, _)| p.clone()));
        let answer = answer(app, plugin.as_deref(), &text);
        if let Some(host) = app.services.office.as_mut() {
            host.reply(&k, &answer.to_string());
        }
    }
    selection_events(app);
    if !app.office.calls.is_empty() {
        expire_calls(app, crate::now_ms());
    }
}

/// The answer to one message from an add-in's page.
fn answer(app: &mut SheetApp, plugin: Option<&str>, text: &str) -> Json {
    let msg: Json = serde_json::from_str(text).unwrap_or(Json::Null);
    let id = msg.get("id").cloned().unwrap_or(Json::Null);
    let Some(plugin) = plugin else {
        return json!({"id": id, "error": {"code": "GeneralException", "message": "this page's add-in is closed"}});
    };
    let actions = msg.get("actions").cloned().unwrap_or(Json::Array(Vec::new()));
    let out = match app.session.execute("office.sync", json!({"addin": plugin, "actions": actions})) {
        Ok(out) => out,
        Err(e) => json!({"error": {"code": "GeneralException", "message": e.to_string()}}),
    };
    app.after_engine();
    let mut reply = json!({"id": id});
    if let (Some(r), Json::Object(o)) = (reply.as_object_mut(), out) {
        r.extend(o);
    }
    reply
}

/// The task pane: the add-in's title (a tab per open pane) and its page.
pub fn show(app: &mut SheetApp, ui: &mut Ui) {
    // Plugins that were unloaded or turned off take their panes with them.
    let gone: Vec<usize> = app
        .office
        .open
        .iter()
        .enumerate()
        .filter(|(_, p)| !gridcraft_engine::plugin::get(&p.plugin).is_some_and(|q| q.active()))
        .map(|(i, _)| i)
        .collect();
    for i in gone.into_iter().rev() {
        close(app, i);
    }
    let covered = app.backstage.is_some() || app.dialog.is_some();
    let current = app.office.open.get(app.office.current).cloned();
    let Some(pane) = current else {
        hide_shown(app);
        return;
    };
    let t = Tokens::get(ui.ctx());
    let max = (ui.available_width() / 2.0).max(320.0);
    let mut page_rect = None;
    egui::Panel::right("office_pane").default_size(340.0).max_size(max).resizable(true).frame(egui::Frame::NONE.fill(t.ribbon).inner_margin(8)).show(
        ui,
        |ui| {
            ui.horizontal(|ui| {
                if app.office.open.len() > 1 {
                    let titles: Vec<(usize, String)> = app.office.open.iter().enumerate().map(|(i, p)| (i, p.title.clone())).collect();
                    for (i, title) in titles {
                        if ui.selectable_label(i == app.office.current, title).clicked() {
                            app.office.current = i;
                        }
                    }
                } else {
                    ui.label(egui::RichText::new(&pane.title).font(theme::ui_bold(15.0)));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::widgets::icon_button(ui, crate::icons::Icon::Close, t.text_dim, "Close", vec2(22.0, 22.0)).clicked() {
                        let i = app.office.current;
                        close(app, i);
                    }
                });
            });
            ui.separator();
            if app.services.office.is_some() {
                let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
                page_rect = Some(rect);
            } else {
                ui.label("Office add-ins need the GridCraft desktop app on macOS or Windows.");
                ui.add_space(4.0);
                ui.label(egui::RichText::new(&pane.url).weak());
            }
        },
    );
    // The pane may have been closed above.
    let Some(pane) = app.office.open.get(app.office.current).cloned() else {
        hide_shown(app);
        return;
    };
    let ppp = ui.ctx().pixels_per_point();
    match (page_rect, covered) {
        (Some(rect), false) => {
            if app.office.shown.as_deref() != Some(pane.key.as_str()) {
                hide_shown(app);
            }
            if let Some(host) = app.services.office.as_mut() {
                host.show(&pane.key, &pane.url, rect, ppp);
            }
            app.office.shown = Some(pane.key);
        }
        _ => hide_shown(app),
    }
}

/// Hides the page while the File tab covers the window (the pane isn't drawn then).
pub fn covered(app: &mut SheetApp) {
    hide_shown(app);
}

fn hide_shown(app: &mut SheetApp) {
    if let Some(k) = app.office.shown.take()
        && let Some(host) = app.services.office.as_mut()
    {
        host.hide(&k);
    }
}
