//! Linked workbooks in the interface: the security warning on open (Enable Content), the
//! Workbook Links pane (Data › Queries & Connections), File › Info › Edit Links to Files and the
//! dialogs it opens, in English and German.
#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};

use egui::os::OperatingSystem;
use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::Queryable;
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_ui_egui::SheetApp;
use gridcraft_ui_egui::i18n::Language;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn folder(name: &str) -> PathBuf {
    #[allow(clippy::disallowed_methods)] // native-only test: a unique temp dir name
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("gridcraft-ui-links-{name}-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn source(path: &Path, a1: f64) {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("cell.set", json!({"cell": "A1", "input": a1.to_string()})).unwrap();
    s.execute("file.saveAs", json!({"path": path})).unwrap();
}

/// `Dest.xlsx` in `dir`, reading `Source.xlsx` there (A1 = 10 when it was saved), then the source
/// changed to 12. Returns a session that has just opened `Dest.xlsx`.
fn opened(dir: &Path) -> Session {
    source(&dir.join("Source.xlsx"), 10.0);
    let mut s = Session::new();
    s.new_workbook();
    s.execute("file.saveAs", json!({"path": dir.join("Dest.xlsx")})).unwrap();
    s.execute("cell.set", json!({"cell": "A1", "input": "=[Source.xlsx]Sheet1!A1"})).unwrap();
    s.execute("cell.set", json!({"cell": "A2", "input": "=[Source.xlsx]Sheet1!A1*2"})).unwrap();
    s.execute("data.refreshLinks", json!({})).unwrap();
    s.execute("file.save", json!({})).unwrap();
    source(&dir.join("Source.xlsx"), 12.0);
    let mut s = Session::new();
    s.execute("file.open", json!({"path": dir.join("Dest.xlsx")})).unwrap();
    s
}

fn harness(session: Session) -> Harness {
    let mut h = egui_kittest::Harness::builder().with_os(OperatingSystem::Nix).with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        SheetApp::new(session, Default::default()),
    );
    h.run_steps(4);
    h
}

fn value(h: &Harness, cell: &str) -> Value {
    h.state().session.active().unwrap().wb.active().unwrap().value(CellRef::parse(cell).unwrap())
}

fn click(h: &mut Harness, label: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, label).click();
    h.run_steps(4);
}

fn shows(h: &Harness, label: &str) -> bool {
    h.query_all_by_label(label).next().is_some()
}

fn dialog(h: &Harness) -> Option<String> {
    h.state().dialog.as_ref().map(|d| d.name().to_string())
}

fn keytips(h: &mut Harness, seq: &str) {
    let key = |k: Key, pressed: bool| Event::Key { key: k, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    h.input_mut().events.extend([key(Key::F10, true), key(Key::F10, false)]);
    h.run_steps(2);
    for ch in seq.chars() {
        let k = Key::from_name(&ch.to_string()).unwrap();
        h.input_mut().events.extend([key(k, true), Event::Text(ch.to_ascii_lowercase().to_string()), key(k, false)]);
        h.run_steps(2);
    }
    h.run_steps(4);
}

#[test]
fn opening_a_linked_workbook_shows_the_security_warning_and_enable_content_updates() {
    let dir = folder("bar");
    let mut h = harness(opened(&dir));
    assert_eq!(value(&h, "A1"), Value::Number(10.0), "opening reads nothing");
    let bars = gridcraft_ui_egui::message_bar::shown(h.state(), &h.ctx);
    assert_eq!(bars.len(), 1);
    assert!(shows(&h, "Security Warning"));
    assert!(shows(&h, "Automatic update of links has been disabled"));
    click(&mut h, "Enable Content");
    assert_eq!((value(&h, "A1"), value(&h, "A2")), (Value::Number(12.0), Value::Number(24.0)));
    assert!(gridcraft_ui_egui::message_bar::shown(h.state(), &h.ctx).is_empty(), "the warning goes once content is enabled");
    assert!(!shows(&h, "Enable Content"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_security_warning_speaks_german() {
    let dir = folder("de");
    let mut h = harness(opened(&dir));
    h.state_mut().ui.language = Language::De;
    h.run_steps(4);
    assert!(shows(&h, "Sicherheitswarnung"));
    assert!(shows(&h, "Die automatische Aktualisierung von Links wurde deaktiviert"));
    click(&mut h, "Inhalt aktivieren");
    assert_eq!(value(&h, "A1"), Value::Number(12.0));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_workbook_links_keytip_opens_the_pane_with_a_card_per_link() {
    let dir = folder("pane");
    let mut h = harness(opened(&dir));
    keytips(&mut h, "AK");
    assert_eq!(h.state().grid.pane.as_deref(), Some("workbookLinks"));
    // The pane checks each source when it opens: the file has newer values.
    assert!(shows(&h, "Source.xlsx"));
    assert!(shows(&h, "Warning: Values not updated"));
    click(&mut h, "Refresh");
    assert_eq!(value(&h, "A1"), Value::Number(12.0), "{:?} {:?}", h.state().message, h.state().toast);
    assert!(shows(&h, "OK"));
    // Break Links asks first, then turns the formulas into values.
    click(&mut h, "Break Links");
    assert_eq!(dialog(&h).as_deref(), Some("breakLinks"));
    h.state_mut().grid.pane = None;
    h.run_steps(2);
    click(&mut h, "Break Links");
    assert_eq!(dialog(&h), None);
    let sh = h.state().session.active().unwrap().wb.active().unwrap().clone();
    assert!(sh.cell(CellRef::parse("A1").unwrap()).unwrap().formula.is_none());
    assert_eq!(value(&h, "A2"), Value::Number(24.0));
    assert!(h.state().session.active().unwrap().wb.external_books.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn edit_links_from_file_info_checks_status_and_sets_the_startup_prompt() {
    let dir = folder("edit");
    let mut h = harness(opened(&dir));
    h.state_mut().open_backstage(gridcraft_ui_egui::backstage::Page::Info);
    h.run_steps(3);
    h.get_by_label("Edit Links to Files").click();
    h.run_steps(4);
    assert_eq!(dialog(&h).as_deref(), Some("editLinks"));
    for label in ["Source", "Type", "Update", "Status", "Source.xlsx", "Worksheet", "Unknown"] {
        assert!(shows(&h, label), "{label}");
    }
    click(&mut h, "Check Status");
    assert!(shows(&h, "Warning: Values not updated"));
    click(&mut h, "Update Values");
    assert_eq!(value(&h, "A1"), Value::Number(12.0));
    // Startup Prompt… opens over Edit Links and returns to it.
    click(&mut h, "Startup Prompt…");
    assert_eq!(dialog(&h).as_deref(), Some("linkStartupPrompt"));
    h.get_by_label("Don't display the alert and don't update automatic links").click();
    h.run_steps(3);
    click(&mut h, "   OK   ");
    assert_eq!(h.state().session.active().unwrap().wb.update_links.as_str(), "never");
    assert_eq!(dialog(&h).as_deref(), Some("editLinks"), "back to Edit Links");
    click(&mut h, "  Close  ");
    assert_eq!(dialog(&h), None);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn edit_links_speaks_german() {
    let dir = folder("dede");
    let mut s = opened(&dir);
    s.execute("data.editLinks", json!({})).unwrap();
    let mut h = harness(s);
    h.state_mut().ui.language = Language::De;
    h.state_mut().open_dialog("editLinks", json!({}));
    h.run_steps(4);
    for label in [
        "Verknüpfungen bearbeiten",
        "Werte aktualisieren",
        "Quelle ändern…",
        "Quelle öffnen",
        "Verknüpfung löschen",
        "Status prüfen",
        "Eingabeaufforderung beim Start…",
    ] {
        assert!(shows(&h, label), "{label}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}
