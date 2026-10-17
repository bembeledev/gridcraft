//! The craft-live client (`live`) compiled to wasm, as the web build runs it: random Yjs client
//! ids from the browser's crypto, the fragment, joining, the y-websocket framing, seeding from
//! file bytes and the upload a `save` asks for. Run with
//! `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner cargo test -p gridcraft-engine --target wasm32-unknown-unknown --test live_wasm`
//! (Node 19 or later, which has `globalThis.crypto`).
#![cfg(target_arch = "wasm32")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use gridcraft_engine::Session;
use gridcraft_engine::live::wire::Frame;
use gridcraft_engine::live::{Action, HttpResponse, LiveClient, LiveConfig, Phase};
use serde_json::json;
use wasm_bindgen_test::wasm_bindgen_test;

const FRAGMENT: &str = "#craft-live=1&ws=wss%3A%2F%2Flive.test%2Fws%2Fr1&room=r1&token=t0k&user=%7B%22id%22%3A%22ada%22%2C%22name%22%3A%22Ada%22%2C%22color%22%3A%22%23d8553b%22%7D&role=edit&seed=1&file=https%3A%2F%2Flive.test%2Fapi%2Fv1%2Frooms%2Fr1%2Ffile&name=Budget.xlsx&format=xltx&api=https%3A%2F%2Flive.test%2Fapi%2Fv1&lang=de";

#[wasm_bindgen_test]
fn client_ids_are_random_in_the_browser() {
    let a = gridcraft_engine::collab_core::new_client_id();
    let b = gridcraft_engine::collab_core::new_client_id();
    assert_ne!(a, b);
    let mut buf = [0u8; 32];
    gridcraft_engine::collab_core::random_bytes(&mut buf).unwrap();
    assert!(buf.iter().any(|b| *b != 0));
}

#[wasm_bindgen_test]
fn seed_connect_and_save_in_wasm() {
    let cfg = LiveConfig::from_fragment(FRAGMENT).unwrap().unwrap();
    assert_eq!(cfg.user.name, "Ada");
    let mut s = Session::new();
    let mut live = LiveClient::new(cfg);
    live.start(&mut s, 1);
    assert_eq!(live.phase(), Phase::Loading);
    let get = live.take_actions().into_iter().find_map(|a| if let Action::Http(r) = a { Some(r) } else { None }).unwrap();
    assert_eq!(get.method, "GET");
    // The file: a workbook written here.
    let mut src = Session::new();
    src.new_workbook();
    src.execute("cell.set", json!({"cell": "A1", "input": "Item"})).unwrap();
    let bytes = gridcraft_engine::io::save_bytes_as(&src.doc().unwrap().wb, "Budget.xlsx", gridcraft_engine::io::FileKind::Xlsx).unwrap();
    live.on_http(&mut s, get.id, Ok(HttpResponse { status: 200, body: bytes }));
    assert_eq!(live.phase(), Phase::Live);
    let acts = live.take_actions();
    let conn = acts
        .iter()
        .find_map(|a| {
            if let Action::Connect { conn, url } = a {
                assert_eq!(url, "wss://live.test/ws/r1?token=t0k");
                Some(*conn)
            } else {
                None
            }
        })
        .unwrap();
    assert!(acts.iter().any(|a| matches!(a, Action::Shell(m) if m["type"] == "craft-live:loaded")));
    live.on_open(conn);
    live.on_frame(&mut s, conn, &Frame::Control(json!({"type": "welcome", "seed": true, "seeded": false, "modified": false})).encode());
    live.on_frame(&mut s, conn, &Frame::Sync1(vec![0]).encode());
    live.poll(&mut s, 2);
    let sent: Vec<Frame> =
        live.take_actions().into_iter().filter_map(|a| if let Action::Send { frame, .. } = a { Frame::decode(&frame).ok() } else { None }).collect();
    assert!(matches!(&sent[0], Frame::Control(v) if v["type"] == "hello"));
    // The whole seed answers the room's empty state vector.
    assert!(sent.iter().any(|f| matches!(f, Frame::Sync2(u) if u.len() > 20)));
    // Read-only until the room has the seed.
    assert_eq!(gridcraft_engine::collab::read_only(&s).as_deref(), Some("Joining…"));
    live.on_frame(&mut s, conn, &Frame::Control(json!({"type": "seeded", "modified": false})).encode());
    assert_eq!(gridcraft_engine::collab::read_only(&s), None);
    let save = json!({"type": "save", "id": 1, "mode": "upload", "name": "Budget.xltx", "format": "xltx", "url": "https://live.test/api/v1/rooms/r1/file?save=1"});
    live.on_frame(&mut s, conn, &Frame::Control(save).encode());
    let put = live.take_actions().into_iter().find_map(|a| if let Action::Http(r) = a { Some(r) } else { None }).unwrap();
    assert_eq!(put.method, "PUT");
    assert!(put.headers.iter().any(|(k, v)| k == "X-Craft-File-Name" && v == "Budget.xltx"));
    assert!(put.body.unwrap().starts_with(b"PK"));
}
