//! Plugin protocol 2.2 end to end: `functions/call` carries the calling workbook (`document`),
//! `documentOpened` arrives before an opened workbook's first calculation, multi-line form fields,
//! explicit blank results (`{"empty": true}`), literal text in `range.setValues`, and
//! `host/functions/invalidate` (during a command and on the push channel). The plugin is this
//! test binary started again with `--serve-plugin` (no libtest harness, so standard output is the
//! protocol's alone).
#![allow(clippy::disallowed_methods)] // the clock, off wasm: waiting for the push channel

use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use serde_json::{Value as Json, json};

const PLUGIN: &str = "org.gridcraft.test-protocol-21";

fn main() {
    if std::env::args().any(|a| a == "--serve-plugin") {
        serve();
        return;
    }
    let dir = setup();
    let tests: [(&str, fn(&std::path::Path)); 7] = [
        ("initialize_announces_protocol_2_2", initialize_announces_protocol_2_2),
        ("functions_know_their_workbook", functions_know_their_workbook),
        ("document_opened_comes_before_the_first_calculation", document_opened_comes_before_the_first_calculation),
        ("explicit_blanks_show_blank_and_null_shows_0", explicit_blanks_show_blank_and_null_shows_0),
        ("literal_values_stay_text", literal_values_stay_text),
        ("multiline_fields_are_validated_and_returned", multiline_fields_are_validated_and_returned),
        ("invalidated_functions_are_recalculated", invalidated_functions_are_recalculated),
    ];
    for (name, t) in tests {
        t(&dir);
        println!("test {name} ... ok");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- the plugin

fn send(out: &Arc<Mutex<std::io::Stdout>>, msg: &Json) {
    let mut o = out.lock().unwrap();
    let _ = writeln!(o, "{msg}").and_then(|_| o.flush());
}

/// The plugin side.
/// - `T.DOC()`: `id|path|name` of the calling workbook (`-` without one).
/// - `T.GATE()`: 1 when `documentOpened` named the calling workbook's file, else `#VALUE!`.
/// - `T.BLANK(kind)`: `null`, `{"empty": true}`, or a 2×2 array mixing them.
/// - `T.COUNT()`: how many times it was called (results are kept: only invalidation calls again).
/// - commands: `invalidate` (asks the host to invalidate `T.COUNT` during the command),
///   `pushLater` (asks it on the push channel, after answering), `form` (a dialog with code).
fn serve() {
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let out = Arc::new(Mutex::new(std::io::stdout()));
    let mut init = Json::Null;
    let mut opened: Vec<String> = Vec::new();
    let mut count = 0u32;
    let mut next = 0u32;
    while let Some(Ok(line)) = lines.next() {
        let Ok(msg) = serde_json::from_str::<Json>(&line) else { continue };
        // Answers to the plugin's own push-channel requests.
        if msg.get("method").is_none() {
            continue;
        }
        let Some(id) = msg.get("id").cloned() else { break };
        let p = msg["params"].clone();
        let method = msg["method"].as_str().unwrap_or("").to_string();
        let answer = match method.as_str() {
            "initialize" => {
                init = p.clone();
                json!({"result": {"protocolVersion": 1}})
            }
            "events/documentOpened" => {
                // Takes a moment: the opening calculation waits for it.
                std::thread::sleep(Duration::from_millis(300));
                opened.push(p["path"].as_str().unwrap_or("").to_string());
                json!({"result": {}})
            }
            "functions/call" => {
                let doc = &p["document"];
                let value = match p["name"].as_str().unwrap_or("") {
                    "T.DOC" if doc.is_null() => json!("-"),
                    "T.DOC" => json!(format!(
                        "{}|{}|{}",
                        doc["id"].as_str().unwrap_or("?"),
                        doc["path"].as_str().unwrap_or(""),
                        doc["name"].as_str().unwrap_or("?")
                    )),
                    "T.GATE" => match doc["path"].as_str() {
                        Some(path) if opened.iter().any(|o| o == path) => json!(1),
                        _ => json!({"error": "#VALUE!"}),
                    },
                    "T.BLANK" => match p["args"][0].as_str().unwrap_or("") {
                        "empty" => json!({"empty": true}),
                        "mix" => json!([[{"empty": true}, null], [1, {"empty": true}]]),
                        _ => Json::Null,
                    },
                    "T.COUNT" => {
                        count += 1;
                        json!(count)
                    }
                    _ => json!({"error": "#NAME?"}),
                };
                json!({"result": {"value": value}})
            }
            "commands/run" => match p["command"].as_str().unwrap_or("") {
                "hello" => json!({"result": {"result": {"init": init, "document": p["context"]["document"]}}}),
                "invalidate" => {
                    next += 1;
                    let hid = format!("h{next}");
                    send(
                        &out,
                        &json!({"jsonrpc": "2.0", "id": hid, "method": "host/functions/invalidate", "params": {"names": ["t.count", "NOT.MINE"]}}),
                    );
                    let mut answer = Json::Null;
                    for line in lines.by_ref() {
                        let m: Json = serde_json::from_str(&line.unwrap_or_default()).unwrap_or(Json::Null);
                        if m["id"] == json!(hid) {
                            answer = m;
                            break;
                        }
                    }
                    json!({"result": {"result": answer}})
                }
                "pushLater" => {
                    let out = out.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(200));
                        send(&out, &json!({"jsonrpc": "2.0", "id": "push1", "method": "host/functions/invalidate", "params": {}}));
                    });
                    json!({"result": {}})
                }
                "form" => json!({"result": {"dialog": {
                    "kind": "form", "title": "Run", "command": "hello",
                    "fields": [
                        {"id": "code", "label": "Code", "value": "x <- 1\ny <- 2", "multiline": true, "code": true, "rows": 99},
                        {"id": "notes", "label": "Notes", "multiline": true},
                        {"id": "n", "label": "N", "kind": "number", "multiline": true},
                        {"id": "one", "label": "One"}
                    ]
                }}}),
                other => json!({"error": {"code": -32000, "message": format!("unknown command {other}")}}),
            },
            _ => json!({"result": {}}),
        };
        let mut reply = json!({"jsonrpc": "2.0", "id": id});
        if let (Some(r), Json::Object(a)) = (reply.as_object_mut(), answer) {
            r.extend(a);
        }
        send(&out, &reply);
    }
}

// ---------------------------------------------------------------- helpers

fn setup() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("gridcraft-engine-protocol-22-{}", std::process::id()));
    let pdir = dir.join(PLUGIN);
    std::fs::create_dir_all(&pdir).unwrap();
    let exe = std::env::current_exe().unwrap();
    let manifest = json!({
        "id": PLUGIN, "name": "Protocol 2.2", "run": {"command": exe.display().to_string(), "args": ["--serve-plugin"]},
        "functions": [
            {"name": "T.DOC", "maxArgs": 1}, {"name": "T.GATE", "maxArgs": 0},
            {"name": "T.BLANK", "minArgs": 1, "maxArgs": 1}, {"name": "T.COUNT", "maxArgs": 1}
        ],
        "commands": [{"id": "hello", "label": "Hello"}, {"id": "invalidate", "label": "Invalidate"},
                     {"id": "pushLater", "label": "Push"}, {"id": "form", "label": "Form"}],
        "events": ["documentOpened"],
    });
    std::fs::write(pdir.join("plugin.json"), manifest.to_string()).unwrap();
    gridcraft_engine::plugin::load(&pdir).unwrap();
    dir
}

fn session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s
}

fn run(s: &mut Session, command: &str, params: Json) -> Result<Json, String> {
    s.run("plugins.run", json!({"plugin": PLUGIN, "command": command, "params": params}))
}

fn set(s: &mut Session, cell: &str, input: &str) {
    s.run("cell.set", json!({"cell": cell, "input": input})).unwrap();
}

fn value(s: &Session, cell: &str) -> Value {
    s.active().unwrap().wb.active().unwrap().value(CellRef::parse(cell).unwrap())
}

fn text(s: &Session, cell: &str) -> String {
    match value(s, cell) {
        Value::Text(t) => t.to_string(),
        other => format!("{other:?}"),
    }
}

// ---------------------------------------------------------------- tests

fn initialize_announces_protocol_2_2(_dir: &std::path::Path) {
    let mut s = session();
    let r = run(&mut s, "hello", json!({})).unwrap();
    let host = &r["result"]["init"]["host"];
    assert_eq!(host["protocol"], "2.2");
    for f in ["functionDocument", "openedBeforeCalc", "multiline", "emptyResult", "literalValues", "invalidate"] {
        assert!(host["features"].as_array().unwrap().contains(&json!(f)), "{f} is announced");
    }
}

fn functions_know_their_workbook(dir: &std::path::Path) {
    let mut s = session();
    let id = s.active().unwrap().session_id();
    set(&mut s, "A1", "=T.DOC()");
    assert_eq!(text(&s, "A1"), format!("{id}||Book1"));
    // Commands see the same id.
    assert_eq!(run(&mut s, "hello", json!({})).unwrap()["result"]["document"]["id"], json!(id));

    // Saved, the workbook keeps its id and gains its path; the same arguments are asked again
    // (a result is kept per workbook and file).
    let path = dir.join("Doc.xlsx").display().to_string();
    s.run("file.save", json!({"path": path})).unwrap();
    set(&mut s, "A2", "=T.DOC()");
    assert_eq!(text(&s, "A2"), format!("{id}|{path}|Doc.xlsx"));

    // Another workbook is another document, even with the same formula.
    s.run("file.new", json!({})).unwrap();
    let other = s.active().unwrap().session_id();
    assert_ne!(other, id);
    set(&mut s, "A1", "=T.DOC()");
    assert!(text(&s, "A1").starts_with(&format!("{other}||")), "{}", text(&s, "A1"));
}

fn document_opened_comes_before_the_first_calculation(dir: &std::path::Path) {
    // A workbook whose formula works only once the plugin heard it was opened.
    let path = dir.join("Gate.xlsx").display().to_string();
    let mut s = session();
    set(&mut s, "A1", "=T.GATE()");
    assert_eq!(value(&s, "A1"), Value::Error(gridcraft_engine::core::CellError::Value), "not opened: refused");
    s.run("file.save", json!({"path": path})).unwrap();

    let mut t = Session::new();
    let started = Instant::now();
    t.run("file.open", json!({"path": path})).unwrap();
    assert!(started.elapsed() >= Duration::from_millis(250), "the opening calculation waited for the plugin's answer");
    assert_eq!(value(&t, "A1"), Value::Number(1.0), "documentOpened came before the first calculation");
    assert!(!t.active().unwrap().is_dirty(), "the opening calculation leaves the workbook unmodified");
}

fn explicit_blanks_show_blank_and_null_shows_0(_dir: &std::path::Path) {
    let mut s = session();
    set(&mut s, "A1", "=T.BLANK(\"null\")");
    assert_eq!(value(&s, "A1"), Value::Number(0.0), "null shows 0, as Excel shows a function returning Empty");
    set(&mut s, "A2", "=T.BLANK(\"empty\")");
    assert_eq!(value(&s, "A2"), Value::Empty, "{{\"empty\": true}} shows blank");
    set(&mut s, "B2", "=A2+1");
    assert_eq!(value(&s, "B2"), Value::Number(1.0), "and reads as an empty cell");
    set(&mut s, "A3", "=IFERROR(T.BLANK(\"empty\"),1)");
    assert_eq!(value(&s, "A3"), Value::Number(0.0), "only a formula that is the call itself keeps the blank");
    // Cached: the same blank again.
    set(&mut s, "A4", "=T.BLANK(\"empty\")");
    assert_eq!(value(&s, "A4"), Value::Empty);
    // In an array, explicit blanks stay blank and nulls are 0.
    set(&mut s, "D1", "=T.BLANK(\"mix\")");
    assert_eq!(value(&s, "D1"), Value::Empty);
    assert_eq!(value(&s, "E1"), Value::Number(0.0));
    assert_eq!(value(&s, "D2"), Value::Number(1.0));
    assert_eq!(value(&s, "E2"), Value::Empty);
}

fn literal_values_stay_text(_dir: &std::path::Path) {
    let mut s = session();
    let values = json!([["123", "=A1", "1/2", "'x", "TRUE", "", 5]]);
    s.run("range.setValues", json!({"range": "A1", "values": values, "literal": true})).unwrap();
    assert_eq!(value(&s, "A1"), Value::Text("123".into()));
    assert_eq!(value(&s, "B1"), Value::Text("=A1".into()));
    assert!(s.active().unwrap().wb.active().unwrap().cell(CellRef::parse("B1").unwrap()).unwrap().formula.is_none());
    assert_eq!(value(&s, "C1"), Value::Text("1/2".into()));
    assert_eq!(value(&s, "D1"), Value::Text("'x".into()));
    assert_eq!(value(&s, "E1"), Value::Text("TRUE".into()));
    assert_eq!(value(&s, "F1"), Value::Empty);
    assert_eq!(value(&s, "G1"), Value::Number(5.0), "numbers stay numbers");
    // Without it, strings are read as typed.
    s.run("range.setValues", json!({"range": "A2", "values": [["123", "=A2+1"]]})).unwrap();
    assert_eq!(value(&s, "A2"), Value::Number(123.0));
    assert_eq!(value(&s, "B2"), Value::Number(124.0));
    // One undo step.
    s.run("edit.undo", json!({})).unwrap();
    s.run("edit.undo", json!({})).unwrap();
    assert_eq!(value(&s, "A1"), Value::Empty);
}

fn multiline_fields_are_validated_and_returned(_dir: &std::path::Path) {
    let mut s = session();
    let r = run(&mut s, "form", json!({})).unwrap();
    let f = &r["dialog"]["fields"];
    assert_eq!(f[0]["multiline"], true);
    assert_eq!(f[0]["code"], true);
    assert_eq!(f[0]["rows"], 30, "rows are bounded");
    assert_eq!(f[0]["value"], "x <- 1\ny <- 2");
    assert_eq!(f[1]["multiline"], true);
    assert_eq!(f[1]["code"], false);
    assert_eq!(f[1]["rows"], 8);
    assert!(f[2].get("multiline").is_none(), "only text fields are multi-line");
    assert!(f[3].get("multiline").is_none());
}

fn invalidated_functions_are_recalculated(_dir: &std::path::Path) {
    let mut s = session();
    set(&mut s, "A1", "=T.COUNT()");
    let first = value(&s, "A1");
    let Value::Number(n) = first else { panic!("a count: {first:?}") };
    set(&mut s, "B1", "x");
    s.run("formulas.calculateNow", json!({})).unwrap_or_default();
    assert_eq!(value(&s, "A1"), Value::Number(n), "kept: not called again");

    // During a command: the plugin's own names only; recalculated after the step.
    s.run("file.save", json!({"path": std::env::temp_dir().join(format!("gc-inv-{}.xlsx", std::process::id())).display().to_string()})).unwrap();
    let r = run(&mut s, "invalidate", json!({})).unwrap();
    assert_eq!(r["result"]["result"], json!({"functions": ["T.COUNT"]}));
    assert_eq!(value(&s, "A1"), Value::Number(n + 1.0), "recalculated after the command");
    assert!(!s.active().unwrap().is_dirty(), "a saved workbook stays saved");

    // On the push channel, any time: recalculated when the app polls.
    run(&mut s, "pushLater", json!({})).unwrap();
    let start = Instant::now();
    while value(&s, "A1") != Value::Number(n + 2.0) && start.elapsed() < Duration::from_secs(20) {
        s.collab_poll(start.elapsed().as_millis() as u64 + 1);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(value(&s, "A1"), Value::Number(n + 2.0), "recalculated after the push-channel request");
    let _ = std::fs::remove_file(std::env::temp_dir().join(format!("gc-inv-{}.xlsx", std::process::id())));
}
