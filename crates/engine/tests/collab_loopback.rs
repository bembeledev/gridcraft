//! End to end through craft-collab's loopback transport: two GridCraft processes (this test
//! binary started twice, `--peer share` and `--peer join`) co-edit through
//! `craft-collab-loopback` and its relay on 127.0.0.1. Needs the built plugin: set
//! `CRAFT_COLLAB_LOOPBACK` to the `craft-collab-loopback` executable (craft-collab's
//! `cargo build -p craft-collab-loopback`); without it the test says so and passes.
#![allow(clippy::disallowed_methods)] // the clock, off wasm

use std::time::{Duration, Instant};

use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use serde_json::{Value as Json, json};

const PLUGIN: &str = "org.craft.collab.loopback";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--peer") {
        let role = args.get(i + 1).cloned().unwrap_or_default();
        let dir = args.get(i + 2).cloned().unwrap_or_default();
        std::process::exit(match peer(&role, &dir) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("peer {role}: {e}");
                1
            }
        });
    }
    let Some(exe) = std::env::var_os("CRAFT_COLLAB_LOOPBACK") else {
        println!("test loopback_end_to_end ... skipped (set CRAFT_COLLAB_LOOPBACK)");
        return;
    };
    let dir = std::env::temp_dir().join(format!("gridcraft-loopback-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = json!({
        "id": PLUGIN, "name": "Live (Loopback)", "run": {"command": std::path::PathBuf::from(&exe).display().to_string()},
        "commands": [{"id": "share", "label": "Share"}, {"id": "join", "label": "Join"}, {"id": "dismiss", "label": "Close"}],
        "collab": {"transport": "loopback", "label": "Live (Loopback)"},
    });
    std::fs::write(dir.join("plugin.json"), manifest.to_string()).unwrap();
    let me = std::env::current_exe().unwrap();
    let port = 39_000 + (std::process::id() % 900) as u16;
    let spawn = |role: &str| {
        std::process::Command::new(&me)
            .args(["--peer", role, &dir.display().to_string()])
            .env("CRAFT_COLLAB_LOOPBACK_PORT", port.to_string())
            .spawn()
            .unwrap()
    };
    let mut a = spawn("share");
    std::thread::sleep(Duration::from_millis(800));
    let mut b = spawn("join");
    let (ra, rb) = (a.wait().unwrap(), b.wait().unwrap());
    let _ = std::fs::remove_dir_all(&dir);
    assert!(ra.success() && rb.success(), "the peers failed: share {ra}, join {rb}");
    println!("test loopback_end_to_end ... ok");
}

fn value(s: &Session, cell: &str) -> Value {
    s.active().map(|d| d.wb.active().map(|sh| sh.value(CellRef::parse(cell).unwrap_or_default())).unwrap_or_default()).unwrap_or_default()
}

fn peer(role: &str, dir: &str) -> Result<(), String> {
    gridcraft_engine::plugin::load(std::path::Path::new(dir))?;
    let mut s = Session::new();
    s.new_workbook();
    if role == "share" {
        s.run("cell.set", json!({"cell": "A1", "input": "from share"}))?;
    }
    let r = s.run("plugins.run", json!({"plugin": PLUGIN, "command": role, "params": {"room": "gridcraft-e2e", "name": role}}))?;
    let mut join = r.get("collab").and_then(|c| c.get("join")).cloned().ok_or(format!("no collab.join in {r}"))?;
    join["plugin"] = json!(PLUGIN);
    s.run("collab.join", join)?;
    let start = Instant::now();
    let (want_cell, want) = if role == "share" { ("B1", "from join") } else { ("A1", "from share") };
    let mut wrote = false;
    while start.elapsed() < Duration::from_secs(30) {
        s.collab_poll(start.elapsed().as_millis() as u64 + 1_000);
        if role == "join" && !wrote && value(&s, "A1") == Value::Text("from share".into()) {
            s.run("cell.set", json!({"cell": "B1", "input": "from join"}))?;
            wrote = true;
        }
        if value(&s, want_cell) == Value::Text(want.into()) {
            // Linger so the other side gets what we sent.
            let until = Instant::now() + Duration::from_secs(2);
            while Instant::now() < until {
                s.collab_poll(start.elapsed().as_millis() as u64 + 1_000);
                std::thread::sleep(Duration::from_millis(20));
            }
            let status = s.run("collab.status", Json::Null)?;
            eprintln!("peer {role}: ok, {status}");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(format!("timed out: {want_cell} is {:?}; status {}", value(&s, want_cell), s.run("collab.status", Json::Null).unwrap_or_default()))
}
