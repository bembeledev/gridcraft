//! IME composition at the grid level.
//!
//! With a CJK IME, the first keystroke of a composition must reach the input method, not be
//! written straight into the cell as raw text. The grid publishes `PlatformOutput::ime` while
//! it owns input on an editable cell, so the OS delivers `ImeEvent`s; a `Commit` opens the
//! in-cell editor with the finished text. Plain ASCII keeps using `Event::Text` (no regression).

use egui::{Event, ImeEvent};
use gridcraft_engine::Session;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

/// A harness over a fresh workbook with `B2` active, stepped a few frames so the grid owns input.
fn fixture() -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    session.execute("selection.set", json!({"cell": "B2"})).unwrap();
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .with_step_dt(1.0 / 60.0)
        .with_os(egui::os::OperatingSystem::Mac)
        .build_ui_state(
            |ui, app: &mut SheetApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            SheetApp::new(session, Default::default()),
        );
    h.run_steps(4);
    h
}

/// The text the in-cell editor currently holds, or `None` when no editor is open.
fn editor_text(h: &Harness) -> Option<String> {
    h.state().editor.as_ref().map(|ed| ed.text.clone())
}

/// A committed composition opens the editor with its text and leaves the caret after it.
#[test]
fn ime_commit_opens_editor_with_committed_text() {
    let mut h = fixture();
    h.input_mut().events.push(Event::Ime(ImeEvent::Commit("张".to_string())));
    h.step();
    let ed = h.state().editor.as_ref().expect("IME commit must open the in-cell editor");
    assert_eq!(ed.text, "张", "committed IME text should seed the editor");
    assert_eq!(ed.caret, ed.text.chars().count(), "caret should sit after the committed text");
}

/// A preedit is not text yet: it must not open the editor on its own, only the commit does.
#[test]
fn ime_preedit_does_not_seed_editor() {
    let mut h = fixture();
    h.input_mut().events.push(Event::Ime(ImeEvent::Preedit { text: "zh".to_string(), active_range_chars: Some(0..2) }));
    h.step();
    assert!(h.state().editor.is_none(), "a preedit must not seed the editor on its own");
    h.input_mut().events.push(Event::Ime(ImeEvent::Commit("张".to_string())));
    h.step();
    assert_eq!(editor_text(&h).as_deref(), Some("张"), "commit after a preedit opens the editor");
}

/// Plain ASCII never composes, so `Event::Text` still seeds the editor exactly as before.
#[test]
fn ascii_text_still_seeds_editor_no_regression() {
    let mut h = fixture();
    h.input_mut().events.push(Event::Text("abc".to_string()));
    h.step();
    assert_eq!(editor_text(&h).as_deref(), Some("abc"), "ASCII typing must still seed the editor");
}
