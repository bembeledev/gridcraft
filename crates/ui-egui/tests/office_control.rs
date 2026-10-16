//! The control channel's Office add-in methods (`office.panes`, `office.open`, `office.close`,
//! `office.eval`) against a fake web view host: the answer plumbing, errors and timeouts. The
//! real web views are checked on macOS and Windows by `cargo xtask office-samples`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::Receiver;

use gridcraft_engine::Session;
use gridcraft_ui_egui::control::{self, ControlRequest, Outcome};
use gridcraft_ui_egui::office_pane::{self, EvalReply, EvalRouter, OfficeHost};
use gridcraft_ui_egui::{Services, SheetApp};
use serde_json::{Value as Json, json};

/// Plugins are global: one test at a time loads them.
static PLUGINS: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Default)]
struct Web {
    shown: Vec<String>,
    closed: Vec<String>,
    inbox: Vec<(String, String)>,
    evals: Vec<(String, String, EvalReply)>,
}

struct FakeHost(Rc<RefCell<Web>>);

impl OfficeHost for FakeHost {
    fn show(&mut self, key: &str, _url: &str, _rect: egui::Rect, _ppp: f32) {
        self.0.borrow_mut().shown.push(key.into());
    }
    fn hide(&mut self, _key: &str) {}
    fn close(&mut self, key: &str) {
        self.0.borrow_mut().closed.push(key.into());
    }
    fn load_hidden(&mut self, _key: &str, _url: &str) {}
    fn take_messages(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.0.borrow_mut().inbox)
    }
    fn reply(&mut self, _key: &str, _json: &str) {}
    fn call(&mut self, _key: &str, _entry: &str, _json: &str) {}
    fn eval(&mut self, key: &str, script: &str, reply: EvalReply) {
        self.0.borrow_mut().evals.push((key.into(), script.into(), reply));
    }
}

/// A host that can't run scripts (the trait's default `eval`).
struct NoScripts;

impl OfficeHost for NoScripts {
    fn show(&mut self, _key: &str, _url: &str, _rect: egui::Rect, _ppp: f32) {}
    fn hide(&mut self, _key: &str) {}
    fn close(&mut self, _key: &str) {}
    fn load_hidden(&mut self, _key: &str, _url: &str) {}
    fn take_messages(&mut self) -> Vec<(String, String)> {
        Vec::new()
    }
    fn reply(&mut self, _key: &str, _json: &str) {}
    fn call(&mut self, _key: &str, _entry: &str, _json: &str) {}
}

const MANIFEST: &str = r#"<OfficeApp xmlns="http://schemas.microsoft.com/office/appforoffice/1.1" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="TaskPaneApp">
  <Id>cccccccc-0000-4000-8000-0000000000e1</Id><Version>1.0</Version><ProviderName>Tests</ProviderName><DefaultLocale>en-US</DefaultLocale>
  <DisplayName DefaultValue="Eval Test"/><Hosts><Host Name="Workbook"/></Hosts>
  <DefaultSettings><SourceLocation DefaultValue="https://tests.example/eval.html"/></DefaultSettings>
  <Permissions>ReadWriteDocument</Permissions></OfficeApp>"#;

fn app_with(host: Option<Box<dyn OfficeHost>>) -> SheetApp {
    let mut s = Session::new();
    s.new_workbook();
    SheetApp::new(s, Services { office: host, ..Default::default() })
}

fn done(o: Outcome) -> Json {
    match o {
        Outcome::Done(v) => v,
        _ => panic!("expected an immediate answer"),
    }
}

/// Loads the test add-in: its plugin id. Unloaded by the caller.
fn load() -> String {
    let m = gridcraft_engine::plugin::office::parse_manifest(MANIFEST, "gridcraft").unwrap();
    let id = m.id.clone();
    gridcraft_engine::plugin::install(m, std::env::temp_dir()).unwrap();
    id
}

/// Sends `method` through the control channel's queue as the server does, and polls once.
fn request(app: &mut SheetApp, ctx: &egui::Context, method: &str, params: Json) -> Receiver<Json> {
    let (tx, rx) = std::sync::mpsc::channel();
    let (req, reply) = ControlRequest::new(method, params);
    tx.send(req).unwrap();
    app.control_rx = Some(rx);
    control::poll(app, ctx);
    reply
}

#[test]
fn panes_open_list_eval_and_close() {
    let _one = PLUGINS.lock().unwrap_or_else(|e| e.into_inner());
    let id = load();
    let web = Rc::new(RefCell::new(Web::default()));
    let mut app = app_with(Some(Box::new(FakeHost(web.clone()))));
    let ctx = egui::Context::default();

    // office.open runs the add-in's default task pane command and answers the pane's key.
    let opened = done(control::handle(&mut app, &ctx, "office.open", &json!({"plugin": id})));
    assert_eq!(opened["ok"], true, "{opened}");
    let key = opened["result"]["pane"].as_str().unwrap().to_string();
    assert!(key.starts_with(&id), "{key}");
    let panes = done(control::handle(&mut app, &ctx, "office.panes", &json!({})));
    let pane = &panes["result"][0];
    assert_eq!((pane["key"].as_str(), pane["plugin"].as_str(), pane["kind"].as_str()), (Some(key.as_str()), Some(id.as_str()), Some("pane")));
    assert_eq!(pane["url"], "https://tests.example/eval.html");
    assert_eq!(pane["ready"], false, "not ready before the library says so");
    assert_eq!(pane["title"], "Eval Test");
    web.borrow_mut().inbox.push((key.clone(), json!({"event": "ready"}).to_string()));
    office_pane::pump(&mut app);
    let panes = done(control::handle(&mut app, &ctx, "office.panes", &json!({})));
    assert_eq!(panes["result"][0]["ready"], true);

    // office.eval is answered when the page answers: the plugin id names its pane.
    let reply = request(&mut app, &ctx, "office.eval", json!({"pane": id, "script": "Office.context.host"}));
    assert!(reply.try_recv().is_err(), "no answer before the page's");
    let (to, script, answer) = web.borrow_mut().evals.pop().unwrap();
    assert_eq!((to.as_str(), script.as_str()), (key.as_str(), "Office.context.host"));
    // The page answers from another thread (the web view's callback).
    std::thread::spawn(move || answer(Ok("{\"host\":\"Excel\"}".into()))).join().unwrap();
    control::poll(&mut app, &ctx);
    assert_eq!(reply.try_recv().unwrap(), json!({"ok": true, "result": {"host": "Excel"}}));

    // A script that throws answers an error.
    let reply = request(&mut app, &ctx, "office.eval", json!({"pane": key, "script": "nope()"}));
    let (_, _, answer) = web.borrow_mut().evals.pop().unwrap();
    answer(Err("ReferenceError: nope is not defined".into()));
    control::poll(&mut app, &ctx);
    let r = reply.try_recv().unwrap();
    assert_eq!(r["ok"], false);
    assert!(r["error"].as_str().unwrap().contains("ReferenceError"));

    // An answer that isn't JSON, or too large, is an error too.
    let reply = request(&mut app, &ctx, "office.eval", json!({"pane": key, "script": "1"}));
    let (_, _, answer) = web.borrow_mut().evals.pop().unwrap();
    answer(Ok("{broken".into()));
    control::poll(&mut app, &ctx);
    assert!(reply.try_recv().unwrap()["error"].as_str().unwrap().contains("isn't JSON"));
    let reply = request(&mut app, &ctx, "office.eval", json!({"pane": key, "script": "1"}));
    let (_, _, answer) = web.borrow_mut().evals.pop().unwrap();
    answer(Ok(format!("\"{}\"", "x".repeat(office_pane::MAX_EVAL_RESULT + 1))));
    control::poll(&mut app, &ctx);
    assert!(reply.try_recv().unwrap()["error"].as_str().unwrap().contains("larger than"));

    // Bad requests answer at once.
    for (params, error) in [
        (json!({"pane": key}), "missing `script`"),
        (json!({"pane": "office.nobody", "script": "1"}), "no such pane"),
        (json!({"pane": key, "script": "x".repeat(office_pane::MAX_EVAL_SCRIPT + 1)}), "too large"),
    ] {
        let r = done(control::handle(&mut app, &ctx, "office.eval", &params));
        assert!(r["error"].as_str().unwrap().contains(error), "{r}");
    }
    let r = done(control::handle(&mut app, &ctx, "office.open", &json!({})));
    assert!(r["error"].as_str().unwrap().contains("plugin"));

    // office.close closes the pane: the host closes its page.
    let r = done(control::handle(&mut app, &ctx, "office.close", &json!({"pane": key})));
    assert_eq!(r["result"]["closed"], json!(key));
    assert!(web.borrow().closed.contains(&key));
    assert_eq!(done(control::handle(&mut app, &ctx, "office.panes", &json!({})))["result"], json!([]));
    assert!(done(control::handle(&mut app, &ctx, "office.close", &json!({"pane": key})))["error"].is_string());
    gridcraft_engine::plugin::unload(&id);
}

#[test]
fn an_eval_the_page_never_answers_times_out() {
    let _one = PLUGINS.lock().unwrap_or_else(|e| e.into_inner());
    let id = load();
    let web = Rc::new(RefCell::new(Web::default()));
    let mut app = app_with(Some(Box::new(FakeHost(web.clone()))));
    let ctx = egui::Context::default();
    let opened = done(control::handle(&mut app, &ctx, "office.open", &json!({"plugin": id})));
    let key = opened["result"]["pane"].as_str().unwrap().to_string();
    let Outcome::OfficeEval { pane, script, timeout_ms } =
        control::handle(&mut app, &ctx, "office.eval", &json!({"pane": key, "script": "new Promise(() => {})", "timeoutMs": 1}))
    else {
        panic!("office.eval waits for the page");
    };
    assert_eq!(timeout_ms, 100.0, "clamped to at least 100 ms");
    let (tx, rx) = std::sync::mpsc::channel();
    office_pane::eval(&mut app, &pane, &script, timeout_ms, tx);
    office_pane::finish_evals(&mut app, gridcraft_ui_egui::now_ms());
    assert!(rx.try_recv().is_err(), "still waiting");
    assert!(app.office.evals.waiting());
    office_pane::finish_evals(&mut app, gridcraft_ui_egui::now_ms() + 1000.0);
    let r = rx.try_recv().unwrap();
    assert_eq!(r["ok"], false);
    assert!(r["error"].as_str().unwrap().contains("timeout"), "{r}");
    assert!(!app.office.evals.waiting());
    // The page answering late changes nothing.
    let (_, _, answer) = web.borrow_mut().evals.pop().unwrap();
    answer(Ok("1".into()));
    office_pane::finish_evals(&mut app, gridcraft_ui_egui::now_ms());
    assert!(rx.try_recv().is_err());
    // The longest timeout is below the control server's 60 s.
    let Outcome::OfficeEval { timeout_ms, .. } =
        control::handle(&mut app, &ctx, "office.eval", &json!({"pane": key, "script": "1", "timeoutMs": 1e9}))
    else {
        panic!()
    };
    assert!(timeout_ms < 60_000.0);
    gridcraft_engine::plugin::unload(&id);
}

#[test]
fn hosts_without_scripts_or_web_views_answer_errors() {
    let _one = PLUGINS.lock().unwrap_or_else(|e| e.into_inner());
    let id = load();
    for host in [Some(Box::new(NoScripts) as Box<dyn OfficeHost>), None] {
        let none = host.is_none();
        let mut app = app_with(host);
        let ctx = egui::Context::default();
        let _ = control::handle(&mut app, &ctx, "office.open", &json!({"plugin": id}));
        let reply = request(&mut app, &ctx, "office.eval", json!({"pane": id, "script": "1"}));
        control::poll(&mut app, &ctx);
        let r = reply.try_recv().unwrap();
        assert_eq!(r["ok"], false);
        let e = r["error"].as_str().unwrap();
        assert!(if none { e.contains("desktop app") } else { e.contains("can't run scripts") }, "{e}");
    }
    gridcraft_engine::plugin::unload(&id);
}

#[test]
fn the_router_matches_answers_to_their_page_and_call() {
    let answers: std::sync::Arc<std::sync::Mutex<Vec<(u32, Result<String, String>)>>> = Default::default();
    let to = |n: u32| -> EvalReply {
        let a = answers.clone();
        Box::new(move |r| a.lock().unwrap().push((n, r)))
    };
    let mut router = EvalRouter::default();
    let s1 = router.start("p#1", "1 + 1", to(1)).unwrap();
    let s2 = router.start("p#2", "document.title", to(2)).unwrap();
    assert!(s1.contains("craftEval:1") && s1.contains("\"1 + 1\""), "{s1}");
    assert!(s2.contains("craftEval:2"));
    assert!(s1.contains("__craftOffice") && s1.contains("window.ipc.postMessage"), "answers through the page's channel");
    assert_eq!(router.len(), 2);
    let rest = router.route(vec![
        ("p#1".into(), json!({"id": 3, "actions": []}).to_string()),
        // Another page can't answer for this one.
        ("p#2".into(), json!({"craftEval": 1, "ok": true, "json": "9"}).to_string()),
        ("p#1".into(), json!({"craftEval": 1, "ok": true, "json": "2"}).to_string()),
        ("p#1".into(), json!({"event": "ready"}).to_string()),
    ]);
    assert_eq!(rest.len(), 2, "the pages' own messages pass through");
    assert_eq!(answers.lock().unwrap().as_slice(), &[(1, Ok("2".to_string()))]);
    router.fail("p#2", "closed");
    assert_eq!(answers.lock().unwrap().last(), Some(&(2, Err("closed".to_string()))));
    assert!(router.is_empty());
    // Errors and oversized values.
    assert_eq!(
        office_pane::eval_answer(&json!({"craftEval": 5, "ok": false, "error": "TypeError: x"}).to_string()),
        Some((5, Err("TypeError: x".into())))
    );
    let big = json!({"craftEval": 6, "ok": true, "json": "1".repeat(office_pane::MAX_EVAL_RESULT + 1)}).to_string();
    assert!(matches!(office_pane::eval_answer(&big), Some((6, Err(e))) if e.contains("larger")));
    assert_eq!(office_pane::eval_answer("{\"id\": 1}"), None);
    assert_eq!(office_pane::eval_answer("not json craftEval"), None);
    // A script with quotes and newlines stays one string literal.
    let s = office_pane::eval_script(7, "a = \"x\";\n'</script>'");
    assert!(s.contains(r#"(0,eval)("a = \"x\";\n'</script>'")"#), "{s}");
}
