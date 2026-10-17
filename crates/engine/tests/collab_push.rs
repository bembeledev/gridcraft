//! Protocol 2 end to end through a real plugin process: the push channel and co-editing. The
//! transport is this test binary started again with `--serve-transport` (no libtest harness, so
//! standard output is the protocol's alone). It relays between two rooms, `relay:a` and
//! `relay:b`, as a network would between two people: what the app sends for one arrives at the
//! other, with `from`. Two sessions in this process each join one room.
//!
//! Checked: `initialize` with `push`/`collab`; a plugin command answering `collab.join`; notifications
//! the transport sends on its own (status, relayed sync and updates) reaching each session's
//! loop; the transport's own requests (`collab/snapshot` with the file, `host/log`) answered;
//! the joiner getting the document; edits both ways; per-user undo travelling; leaving.
#![allow(clippy::disallowed_methods)] // the clock, off wasm

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::time::{Duration, Instant};

use gridcraft_engine::Session;
use gridcraft_engine::collab;
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_engine::plugin::push::{self, Incoming};
use serde_json::{Value as Json, json};

const PLUGIN: &str = "org.gridcraft.test-relay";

fn main() {
    if std::env::args().any(|a| a == "--serve-transport") {
        serve();
        return;
    }
    let dir = std::env::temp_dir().join(format!("gridcraft-engine-relay-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = std::env::current_exe().unwrap();
    let manifest = json!({
        "id": PLUGIN, "name": "Relay", "run": {"command": exe.display().to_string(), "args": ["--serve-transport"]},
        "commands": [{"id": "share", "label": "Share"}],
        "collab": {"transport": "relay", "label": "Live (test relay)"},
    });
    std::fs::write(dir.join("plugin.json"), manifest.to_string()).unwrap();
    gridcraft_engine::plugin::load(&dir).unwrap();
    two_sessions_edit_through_the_transport();
    println!("test two_sessions_edit_through_the_transport ... ok");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- the transport

fn serve() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    let mut send = |m: Json| {
        let _ = writeln!(out, "{m}");
        let _ = out.flush();
    };
    let mut clients: HashMap<String, u64> = HashMap::new();
    let mut asked_snapshot = false;
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Json>(&line) else { continue };
        let method = msg["method"].as_str().unwrap_or("").to_string();
        let p = msg["params"].clone();
        if let Some(id) = msg.get("id").filter(|_| !method.is_empty()) {
            // The app's requests.
            let result = match method.as_str() {
                "initialize" => json!({"protocolVersion": 1, "host": p["host"]}),
                "commands/run" => {
                    json!({"collab": {"join": {"room": "relay:a", "mode": "share", "user": {"id": "u-a", "name": "Ann", "color": "#3B7DD8"}}}})
                }
                _ => json!({}),
            };
            // A notification of our own in the middle of a request: it must reach the app's loop.
            if method == "commands/run" {
                send(json!({"jsonrpc": "2.0", "method": "collab/status", "params": {"room": "relay:none", "state": "online"}}));
            }
            send(json!({"jsonrpc": "2.0", "id": id, "result": result}));
            continue;
        }
        if method.is_empty() {
            // An answer to one of our requests.
            if msg["id"] == "snap" {
                let r = &msg["result"];
                let ok =
                    r["update"].as_str().is_some_and(|u| !u.is_empty()) && r["file"].as_str().is_some_and(|f| f.len() > 100) && r["name"].is_string();
                send(
                    json!({"jsonrpc": "2.0", "method": "collab/status", "params": {"room": "relay:a", "state": "online", "message": format!("snapshot ok: {ok}")}}),
                );
            }
            continue;
        }
        if method == "shutdown" {
            break;
        }
        let room = p["room"].as_str().unwrap_or("").to_string();
        let other = match room.as_str() {
            "relay:a" => "relay:b",
            "relay:b" => "relay:a",
            _ => continue,
        };
        match method.as_str() {
            "collab/joined" => {
                clients.insert(room.clone(), p["clientId"].as_u64().unwrap_or(0));
                send(json!({"jsonrpc": "2.0", "method": "collab/status", "params": {"room": room, "state": "connecting"}}));
                send(json!({"jsonrpc": "2.0", "method": "collab/status", "params": {"room": room, "state": "online", "peers": []}}));
                send(json!({"jsonrpc": "2.0", "id": 7, "method": "host/log", "params": {"text": format!("joined {room}")}}));
            }
            "collab/sync1" | "collab/sync2" | "collab/update" | "collab/awareness" => {
                let mut q = p.clone();
                q["room"] = json!(other);
                q["from"] = json!(clients.get(&room).copied().unwrap_or(0));
                if method == "collab/sync2" {
                    q.as_object_mut().unwrap().remove("to");
                }
                send(json!({"jsonrpc": "2.0", "method": method, "params": q}));
                if method == "collab/update" && room == "relay:a" && !asked_snapshot {
                    asked_snapshot = true;
                    send(json!({"jsonrpc": "2.0", "id": "snap", "method": "collab/snapshot", "params": {"room": "relay:a", "file": true}}));
                }
            }
            "collab/leave" => {
                send(
                    json!({"jsonrpc": "2.0", "method": "collab/status", "params": {"room": other, "state": "online", "peers": [], "message": "the other left"}}),
                );
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------- the test

/// Hands each session the messages for its room.
fn pump(a: &mut Session, b: &mut Session, now: u64) {
    let mut for_a = Vec::new();
    let mut for_b = Vec::new();
    for p in push::take() {
        let room = match &p.incoming {
            Incoming::Message(m) => m["params"]["room"].as_str().unwrap_or("relay:a").to_string(),
            _ => "relay:a".into(),
        };
        if room == "relay:b" { for_b.push(p) } else { for_a.push(p) }
    }
    a.collab_poll_pushed(now, for_a);
    b.collab_poll_pushed(now, for_b);
}

fn wait(a: &mut Session, b: &mut Session, what: &str, mut done: impl FnMut(&Session, &Session) -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        let now = start.elapsed().as_millis() as u64 + 1_000;
        pump(a, b, now);
        if done(a, b) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

fn value(s: &Session, cell: &str) -> Value {
    s.active().unwrap().wb.active().unwrap().value(CellRef::parse(cell).unwrap())
}

fn two_sessions_edit_through_the_transport() {
    let mut a = Session::new();
    a.new_workbook();
    a.execute("cell.set", json!({"cell": "A1", "input": "Item"})).unwrap();
    a.execute("cell.set", json!({"cell": "B1", "input": "=2*21"})).unwrap();
    // A plugin command answers `collab.join`: returned to the caller, who joins.
    let r = a.run("plugins.run", json!({"plugin": PLUGIN, "command": "share"})).unwrap();
    let mut join = r["collab"]["join"].clone();
    assert_eq!(join["room"], "relay:a", "{r}");
    join["plugin"] = json!(PLUGIN);
    a.run("collab.join", join).unwrap();
    let mut b = Session::new();
    b.run("collab.join", json!({"plugin": PLUGIN, "room": "relay:b", "mode": "join", "user": {"id": "u-b", "name": "Bob", "color": "#D8513B"}}))
        .unwrap();
    assert_eq!(collab::read_only(&b).as_deref(), Some("Joining…"));
    assert!(b.execute("cell.set", json!({"cell": "C1", "input": "too early"})).is_err(), "read-only while joining");

    wait(&mut a, &mut b, "the joiner to get the document", |_, b| collab::read_only(b).is_none() && value(b, "B1") == Value::Number(42.0));
    assert_eq!(value(&b, "A1"), Value::Text("Item".into()));

    // Edits both ways.
    a.execute("cell.set", json!({"cell": "A2", "input": "from a"})).unwrap();
    b.execute("cell.set", json!({"cell": "A3", "input": "from b"})).unwrap();
    b.execute("home.insertRows", json!({"rows": "1:1"})).unwrap();
    wait(&mut a, &mut b, "the edits to cross", |a, b| {
        value(a, "A4") == Value::Text("from b".into())
            && value(b, "A3") == Value::Text("from a".into())
            && value(a, "A2") == Value::Text("Item".into())
    });
    // The transport asked for a snapshot with the file; the answer was complete.
    wait(&mut a, &mut b, "the snapshot answer", |a, _| a.active_room().and_then(|r| r.message.clone()).as_deref() == Some("snapshot ok: true"));
    // Awareness: each sees the other.
    wait(&mut a, &mut b, "awareness", |a, b| {
        a.active_room().is_some_and(|r| r.peers().iter().any(|p| p.user.name == "Bob"))
            && b.active_room().is_some_and(|r| r.peers().iter().any(|p| p.user.name == "Ann"))
    });

    // Selections far below the materialized rows, and on a fresh sheet with no row or column
    // ids at all, reach the other side (as block ids).
    let sel_of = |b: &Session| b.active_room().and_then(|r| r.peers().into_iter().find(|p| p.user.name == "Ann")).and_then(|p| p.selection);
    a.execute("selection.set", json!({"range": "B200:C210"})).unwrap();
    let far = gridcraft_engine::core::RangeRef::parse("B200:C210").unwrap();
    wait(&mut a, &mut b, "a selection past the last row", |_, b| sel_of(b) == Some((0, vec![far])));
    a.execute("home.insertSheet", json!({"name": "Fresh"})).unwrap();
    let fresh = |s: &Session| s.active().unwrap().wb.sheets.iter().position(|sh| sh.name == "Fresh");
    wait(&mut a, &mut b, "the new sheet", |_, b| fresh(b).is_some());
    a.execute("sheet.activate", json!({"sheet": fresh(&a).unwrap()})).unwrap();
    a.execute("selection.set", json!({"range": "C3:D5"})).unwrap();
    let on_fresh = gridcraft_engine::core::RangeRef::parse("C3:D5").unwrap();
    wait(&mut a, &mut b, "a selection on an empty sheet", |_, b| sel_of(b) == fresh(b).map(|i| (i, vec![on_fresh])));
    a.execute("sheet.activate", json!({"sheet": 0})).unwrap();

    // Per-user undo: Bob's undo removes only his row insert.
    b.execute("edit.undo", json!({})).unwrap();
    wait(&mut a, &mut b, "the undo", |a, _| value(a, "A3") == Value::Text("from b".into()) && value(a, "A2") == Value::Text("from a".into()));
    assert_eq!(collab::fingerprint(&a.active().unwrap().wb), collab::fingerprint(&b.active().unwrap().wb));

    // Leaving: the workbook stays, edits stay local.
    a.run("collab.leave", json!({})).unwrap();
    wait(&mut a, &mut b, "the leave", |_, b| b.active_room().and_then(|r| r.message.clone()).as_deref() == Some("the other left"));
    a.execute("cell.set", json!({"cell": "D1", "input": "alone"})).unwrap();
    assert_eq!(value(&a, "A1"), Value::Text("Item".into()));
    for _ in 0..20 {
        pump(&mut a, &mut b, 99_000);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(value(&b, "D1"), Value::Empty);
}
