//! Folder-to-workbook navigation uses real temporary files and the normal egui click path.
#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::{
    Session,
    core::{CellRef, Value},
};
use gridcraft_ui_egui::{Services, SheetApp};
use serde_json::json;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!("gridcraft-explorer-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    fn file(&self, name: &str, value: &str) -> String {
        let path = self.0.join(name);
        std::fs::write(&path, value).unwrap();
        path.to_str().unwrap().into()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn harness(services: Services) -> egui_kittest::Harness<'static, SheetApp> {
    let mut session = Session::new();
    session.new_workbook();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(session, services),
    );
    h.run_steps(4);
    h
}

fn settle(h: &mut egui_kittest::Harness<'static, SheetApp>) {
    h.run_steps(2);
    for _ in 0..5000 {
        if !h.state().workspace.busy() {
            break;
        }
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(!h.state().workspace.busy(), "folder scan did not finish");
    h.run_steps(4);
}

fn open(h: &mut egui_kittest::Harness<'static, SheetApp>, dir: &Path) {
    h.state_mut().run("workspace.open", json!({"path": dir})).unwrap();
    settle(h);
}

fn a1(h: &egui_kittest::Harness<'static, SheetApp>) -> Value {
    h.state().session.active().unwrap().wb.active().unwrap().value(CellRef::new(0, 0))
}

#[test]
fn picker_lists_supported_files_and_lazy_subfolders_without_opening_them() {
    let dir = TempDir::new();
    dir.file("zeta.CSV", "1\n");
    dir.file("Alpha.xlsx", "not opened until selected");
    dir.file("readme.md", "hidden from the spreadsheet list");
    dir.file("~$locked.xlsx", "temporary Excel lock file");
    std::fs::create_dir(dir.0.join("Reports")).unwrap();
    std::fs::create_dir(dir.0.join(".git")).unwrap();
    dir.file("Reports/nested.tsv", "2\n");
    let picked = dir.0.to_str().unwrap().to_string();
    let mut h = harness(Services { pick_folder: Some(Box::new(move || Some(picked.clone()))), ..Default::default() });
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Open Folder…").click();
    settle(&mut h);
    let state = h.state().workspace.inspect();
    let listings = state["directories"].as_array().unwrap();
    assert_eq!(listings.len(), 1, "subfolders are not scanned until expanded");
    let names: Vec<_> = listings[0]["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Reports", "Alpha.xlsx", "zeta.CSV"]);
    assert_eq!(h.state().session.documents().len(), 1, "folder opening does not load every workbook");
    h.get_by_label("Folder: Reports").click();
    settle(&mut h);
    h.get_by_label("File: nested.tsv").click();
    settle(&mut h);
    assert_eq!(a1(&h), Value::Number(2.0));
    assert!(h.state().grid.cells_rect.unwrap().left() > 180.0, "the grid is beside the explorer");
}

#[test]
fn selecting_files_commits_the_draft_and_preserves_unsaved_values_and_undo() {
    let dir = TempDir::new();
    let first = dir.file("first.csv", "1\n");
    dir.file("second.csv", "2\n");
    let mut h = harness(Services::default());
    open(&mut h, &dir.0);
    h.get_by_label("File: first.csv").click();
    settle(&mut h);
    let uid = h.state().session.active().unwrap().uid;
    h.state_mut().begin_edit(Some("99".into()), false);
    h.get_by_label("File: second.csv").click();
    settle(&mut h);
    assert_eq!(a1(&h), Value::Number(2.0));
    h.get_by_label("File: first.csv").click();
    settle(&mut h);
    assert_eq!(h.state().session.active().unwrap().uid, uid);
    assert_eq!(a1(&h), Value::Number(99.0));
    assert_eq!(h.state().session.documents().len(), 2);
    assert!(h.state().session.active().unwrap().is_dirty());
    assert_eq!(std::fs::read_to_string(first).unwrap(), "1\n", "switching never saves implicitly");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(a1(&h), Value::Number(1.0));
    h.state_mut().begin_edit(Some("77".into()), false);
    h.state_mut().open_path(dir.0.join("second.csv").to_str().unwrap());
    h.state_mut().run("workspace.openFile", json!({"path": dir.0.join("first.csv")})).unwrap();
    assert_eq!(a1(&h), Value::Number(77.0), "opening an individual file also commits the previous draft");
    h.state_mut().run("workspace.close", json!({})).unwrap();
    assert_eq!(h.state().session.documents().len(), 2, "closing the folder retains open documents");
}

#[test]
fn refresh_failed_folder_and_deleted_file_do_not_destroy_the_active_document() {
    let dir = TempDir::new();
    let first = dir.file("first.csv", "7\n");
    let missing = dir.file("missing.csv", "8\n");
    let mut h = harness(Services::default());
    open(&mut h, &dir.0);
    h.state_mut().run("workspace.openFile", json!({"path": first})).unwrap();
    let uid = h.state().session.active().unwrap().uid;
    std::fs::remove_file(missing.clone()).unwrap();
    assert!(h.state_mut().run("workspace.openFile", json!({"path": missing})).is_err());
    h.state_mut().run("workspace.open", json!({"path": dir.0.join("nonexistent")})).unwrap();
    settle(&mut h);
    assert_eq!(h.state().workspace.root(), dir.0.to_str());
    assert!(h.state().workspace.inspect()["error"].as_str().is_some());
    dir.file("added.csv", "9\n");
    h.state_mut().run("workspace.refresh", json!({})).unwrap();
    settle(&mut h);
    assert_eq!(h.state().session.active().unwrap().uid, uid);
    assert_eq!(a1(&h), Value::Number(7.0));
    assert!(h.query_by_label("File: missing.csv").is_none());
    assert!(h.query_by_label("File: added.csv").is_some());
}

#[test]
fn commands_do_not_open_pickers_and_cancelled_folder_selection_keeps_the_workspace() {
    let dir = TempDir::new();
    let count = Arc::new(AtomicUsize::new(0));
    let capture = count.clone();
    let mut h = harness(Services {
        pick_folder: Some(Box::new(move || {
            capture.fetch_add(1, Ordering::Relaxed);
            None
        })),
        ..Default::default()
    });
    for id in ["workspace.open", "workspace.openFile", "workspace.toggleFolder", "workspace.refresh"] {
        assert!(h.state_mut().run(id, json!({})).is_err());
    }
    assert_eq!(count.load(Ordering::Relaxed), 0);
    open(&mut h, &dir.0);
    h.state_mut().open_dialog("open", json!({}));
    assert_eq!(count.load(Ordering::Relaxed), 1, "the default desktop open action picks a folder");
    assert_eq!(h.state().workspace.root(), dir.0.to_str());
    h.state_mut().run("view.explorer", json!({"on": false})).unwrap();
    h.run_steps(3);
    h.get_by_label("Explorer").click();
    h.run_steps(3);
    assert!(h.state().ui.explorer_visible);
}

#[test]
fn encrypted_source_reuses_its_unsaved_document_without_a_save_path() {
    let dir = TempDir::new();
    let mut s = Session::new();
    s.new_workbook();
    s.run("cell.set", json!({"cell": "A1", "input": "42"})).unwrap();
    let bytes = gridcraft_engine::io::save_bytes(&s.active().unwrap().wb, "Secret.xlsx").unwrap();
    let secret = dir.0.join("Secret.xlsx");
    std::fs::write(&secret, gridcraft_xlsx::testing::encrypt_workbook(&bytes, "test-password")).unwrap();
    let second = dir.file("second.csv", "2\n");
    let mut h = harness(Services::default());
    open(&mut h, &dir.0);
    assert!(h.state_mut().run("workspace.openFile", json!({"path": secret})).is_err());
    assert!(h.state().dialog.is_none(), "programmatic opening returns the password error without a dialog");
    h.state_mut().run("workspace.openFile", json!({"path": secret, "password": "test-password"})).unwrap();
    assert!(h.state().session.active().unwrap().path.is_none());
    let uid = h.state().session.active().unwrap().uid;
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "123"})).unwrap();
    h.state_mut().run("workspace.openFile", json!({"path": second})).unwrap();
    h.state_mut().run("workspace.openFile", json!({"path": secret})).unwrap();
    assert_eq!(h.state().session.active().unwrap().uid, uid);
    assert_eq!(a1(&h), Value::Number(123.0));
    assert_eq!(h.state().session.documents().len(), 2);
}

#[cfg(unix)]
#[test]
fn symlink_directories_are_not_followed() {
    let dir = TempDir::new();
    std::os::unix::fs::symlink(&dir.0, dir.0.join("loop")).unwrap();
    let mut h = harness(Services::default());
    open(&mut h, &dir.0);
    assert!(h.query_by_label("Folder: loop").is_none());
    assert!(!h.state().workspace.busy());
}

#[test]
fn invalid_cell_draft_blocks_file_switch_and_dirty_tab_close_keeps_the_prompt_target() {
    let dir = TempDir::new();
    let first = dir.file("first.csv", "1\n");
    let second = dir.file("second.csv", "2\n");
    let mut h = harness(Services::default());
    open(&mut h, &dir.0);
    h.state_mut().run("workspace.openFile", json!({"path": first})).unwrap();
    h.state_mut().run("data.validation", json!({"range": "A1", "type": "whole", "operator": "between", "formula1": "1", "formula2": "10"})).unwrap();
    let uid = h.state().session.active().unwrap().uid;
    h.state_mut().begin_edit(Some("99".into()), false);
    assert!(h.state_mut().run("workspace.openFile", json!({"path": second})).is_err());
    assert_eq!(h.state().session.active().unwrap().uid, uid);
    assert_eq!(h.state().editor.as_ref().unwrap().text, "99");
    h.state_mut().cancel_edit();
    h.state_mut().message = None;
    h.run_steps(3);
    h.get_by_label("Close first.csv").click();
    h.run_steps(3);
    assert_eq!(h.state().dialog.as_ref().unwrap().name(), "saveChanges");
    h.get_by_label("File: second.csv").click();
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().uid, uid, "the save prompt must stay bound to the document being closed");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().uid, uid);
}

#[test]
fn save_as_does_not_redirect_the_original_explorer_entry() {
    let dir = TempDir::new();
    let first = dir.file("first.csv", "1\n");
    let mut h = harness(Services::default());
    open(&mut h, &dir.0);
    h.state_mut().run("workspace.openFile", json!({"path": first})).unwrap();
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "99"})).unwrap();
    h.state_mut().run("file.saveAs", json!({"path": dir.0.join("copy.xlsx")})).unwrap();
    h.state_mut().run("workspace.openFile", json!({"path": first})).unwrap();
    assert_eq!(a1(&h), Value::Number(1.0));
    assert_eq!(h.state().session.documents().len(), 2);
}

#[cfg(unix)]
#[test]
fn file_opened_via_a_path_alias_reuses_its_document() {
    let dir = TempDir::new();
    let first = dir.file("first.csv", "1\n");
    let alias = TempDir::new();
    std::os::unix::fs::symlink(&dir.0, alias.0.join("linked")).unwrap();
    let mut h = harness(Services::default());
    h.state_mut().open_path(alias.0.join("linked/first.csv").to_str().unwrap());
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "55"})).unwrap();
    let uid = h.state().session.active().unwrap().uid;
    open(&mut h, &dir.0);
    h.state_mut().run("workspace.openFile", json!({"path": first})).unwrap();
    assert_eq!(h.state().session.active().unwrap().uid, uid);
    assert_eq!(a1(&h), Value::Number(55.0));
    assert_eq!(h.state().session.documents().len(), 1);
}

#[test]
fn background_open_keeps_live_edits_and_coalesces_rapid_selections() {
    let dir = TempDir::new();
    let first = dir.file("first.csv", "1\n");
    let second = dir.file("second.csv", "2\n");
    let mut h = harness(Services::default());
    open(&mut h, &dir.0);
    let uid = h.state().session.active().unwrap().uid;
    h.state_mut().run("cell.set", json!({"cell":"A1","input":"77"})).unwrap();
    let r = h.state_mut().run("workspace.openFile", json!({"path":first,"background":true})).unwrap();
    assert_eq!(r["loading"], true);
    h.run_steps(1);
    h.state_mut().begin_edit(Some("99".into()), false);
    for _ in 0..20 {
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(h.state().session.active().unwrap().uid, uid);
    assert_eq!(h.state().editor.as_ref().unwrap().text, "99");
    h.state_mut().cancel_edit();
    h.state_mut().run("workspace.openFile", json!({"path":second,"background":true})).unwrap();
    settle(&mut h);
    assert_eq!(a1(&h), Value::Number(2.0));
    assert_eq!(h.state().session.documents().len(), 2, "superseded file does not open a third tab");
    h.state_mut().run("window.activate", json!({"index":0})).unwrap();
    assert_eq!(a1(&h), Value::Number(77.0));
    assert!(h.state().session.active().unwrap().is_dirty());
}

#[test]
fn background_open_reports_errors_without_programmatic_dialogs() {
    let dir = TempDir::new();
    let path = dir.file("deleted.csv", "1");
    let mut h = harness(Services::default());
    open(&mut h, &dir.0);
    std::fs::remove_file(&path).unwrap();
    h.state_mut().run("workspace.openFile", json!({"path":path,"background":true})).unwrap();
    settle(&mut h);
    assert!(h.state().workspace.inspect()["error"].is_string());
    assert!(h.state().dialog.is_none() && h.state().message.is_none());
}

#[test]
fn file_children_are_lazy_and_selecting_a_sheet_opens_it_in_background() {
    let dir = TempDir::new();
    let path = dir.0.join("Book.xlsx").to_string_lossy().into_owned();
    let mut session = Session::new();
    session.new_workbook();
    session.execute("sheet.rename", json!({"name": "First"})).unwrap();
    session.execute("home.insertSheet", json!({"name": "Second"})).unwrap();
    session.execute("cell.set", json!({"cell": "A1", "input": "42"})).unwrap();
    session.execute("sheet.activate", json!({"sheet": "First"})).unwrap();
    std::fs::write(&path, gridcraft_engine::io::save_bytes(&session.active().unwrap().wb, &path).unwrap()).unwrap();
    let original = std::fs::read(&path).unwrap();
    let mut h = harness(Services::default());
    open(&mut h, &dir.0);
    assert!(h.state().workspace.inspect()["files"].as_array().unwrap().is_empty());
    h.get_by_label("Toggle: Book.xlsx").click();
    settle(&mut h);
    h.get_by_label("Sheet: First");
    h.get_by_label("Sheet: Second");
    assert_eq!(h.state().session.documents().len(), 1);
    assert!(h.state().session.active().unwrap().path.is_none(), "expansion must not open a workbook");
    h.get_by_label("Sheet: Second").click();
    settle(&mut h);
    assert_eq!(h.state().session.active().unwrap().wb.active().unwrap().name, "Second");
    assert_eq!(a1(&h), Value::Number(42.0));
    h.state_mut().run("sheet.rename", json!({"name": "Renamed"})).unwrap();
    h.run_steps(3);
    h.get_by_label("Sheet: Renamed");
    assert!(h.query_by_label("Sheet: Second").is_none());
    h.get_by_label("Sheet: First").click();
    settle(&mut h);
    assert_eq!(h.state().session.active().unwrap().wb.active().unwrap().name, "First");
    assert_eq!(h.state().session.documents().len(), 1, "sheet selection reuses the document");
    assert!(h.state().session.active().unwrap().is_dirty());
    h.get_by_label("Toggle: Book.xlsx").click();
    h.run_steps(3);
    assert!(h.query_by_label("Sheet: First").is_none());
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

#[test]
fn sheet_preview_errors_stay_in_explorer_and_refresh_retries() {
    let dir = TempDir::new();
    let path = dir.file("Bad.xlsx", "PK\u{3}\u{4}broken");
    let mut h = harness(Services::default());
    open(&mut h, &dir.0);
    h.state_mut().run("workspace.toggleFile", json!({"path": path})).unwrap();
    settle(&mut h);
    assert!(h.state().workspace.inspect()["files"][0]["error"].is_string());
    assert!(h.state().message.is_none());
    let mut session = Session::new();
    session.new_workbook();
    std::fs::write(&path, gridcraft_engine::io::save_bytes(&session.active().unwrap().wb, &path).unwrap()).unwrap();
    h.state_mut().run("workspace.refresh", json!({})).unwrap();
    settle(&mut h);
    h.state_mut().run("workspace.toggleFile", json!({"path": path})).unwrap();
    settle(&mut h);
    assert_eq!(h.state().workspace.inspect()["files"][0]["sheets"], json!(["Sheet1"]));
}

fn three_tabs(h: &mut egui_kittest::Harness<'static, SheetApp>, dir: &TempDir) -> Vec<u64> {
    for name in ["a.csv", "b.csv", "c.csv"] {
        dir.file(name, "1\n");
    }
    open(h, &dir.0);
    for name in ["a.csv", "b.csv", "c.csv"] {
        h.state_mut().run("file.open", json!({"name": name, "base64": gridcraft_engine::io::base64_encode(b"1\n")})).unwrap();
    }
    h.run_steps(4);
    h.state().session.documents().iter().map(|d| d.uid).collect()
}

#[test]
fn tab_context_menu_targets_clicked_tab_and_middle_click_closes() {
    let dir = TempDir::new();
    let mut h = harness(Services::default());
    let uids = three_tabs(&mut h, &dir);
    h.get_by_label("b.csv").click_secondary();
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().uid, uids[2], "opening the menu must not switch documents");
    h.get_by_label("Close Tabs to the Right").click();
    h.run_steps(4);
    assert_eq!(h.state().session.documents().iter().map(|d| d.uid).collect::<Vec<_>>(), uids[..2]);
    h.get_by_label("a.csv").click_secondary();
    h.run_steps(3);
    h.get_by_label("Close Other Tabs").click();
    h.run_steps(4);
    assert_eq!(h.state().session.active().unwrap().uid, uids[0]);
    assert_eq!(h.state().session.documents().len(), 1);
    h.get_by_label("a.csv").click_button(egui::PointerButton::Middle);
    h.run_steps(4);
    assert!(h.state().session.documents().iter().all(|d| !uids.contains(&d.uid)));
    assert!(!h.state().session.active().unwrap().is_dirty(), "last close returns to a fresh blank workbook");
}

#[test]
fn dirty_tab_batch_stops_on_cancel_and_save_continues_by_uid() {
    let dir = TempDir::new();
    let saved = dir.0.join("saved.xlsx");
    let target = saved.to_string_lossy().into_owned();
    let mut h = harness(Services { pick_save: Some(Box::new(move |_| Some(target.clone()))), ..Default::default() });
    let uids = three_tabs(&mut h, &dir);
    for index in [0, 1] {
        h.state_mut().run("window.activate", json!({"index": index})).unwrap();
        h.state_mut().run("cell.set", json!({"cell": "A1", "input": "7"})).unwrap();
    }
    h.state_mut().run("window.activate", json!({"index": 2})).unwrap();
    h.state_mut().run("workspace.closeTabs", json!({"uid": uids[2], "scope": "all", "interactive": true})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().uid, uids[0]);
    h.get_by_label("Don't Save").click();
    h.run_steps(4);
    assert_eq!(h.state().session.active().unwrap().uid, uids[1]);
    h.get_by_label("Cancel").click();
    h.run_steps(4);
    assert_eq!(h.state().session.documents().iter().map(|d| d.uid).collect::<Vec<_>>(), uids[1..]);
    assert!(h.state().workspace.inspect()["closingTabs"].is_null());
    h.state_mut().run("workspace.closeTabs", json!({"uid": uids[1], "scope": "all", "interactive": true})).unwrap();
    h.run_steps(3);
    h.get_by_label("Save").click();
    h.run_steps(5);
    assert!(h.state().session.documents().iter().all(|d| !uids.contains(&d.uid)));
    assert!(!h.state().session.active().unwrap().is_dirty(), "last close returns to a fresh blank workbook");
    let (wb, _) = gridcraft_engine::io::open_bytes("saved.xlsx", &std::fs::read(saved).unwrap()).unwrap();
    assert_eq!(wb.active().unwrap().value(CellRef::new(0, 0)), Value::Number(7.0));
    assert_eq!(std::fs::read_to_string(dir.0.join("a.csv")).unwrap(), "1\n");
}

#[test]
fn cancelled_save_picker_and_programmatic_dirty_close_leave_tabs_open() {
    let dir = TempDir::new();
    let mut h = harness(Services { pick_save: Some(Box::new(|_| None)), ..Default::default() });
    let uids = three_tabs(&mut h, &dir);
    h.state_mut().run("window.activate", json!({"index": 0})).unwrap();
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "9"})).unwrap();
    assert!(h.state_mut().run("workspace.closeTabs", json!({"uid": uids[0], "scope": "all"})).is_err());
    assert!(h.state().dialog.is_none());
    assert_eq!(h.state().session.documents().len(), 3);
    h.state_mut().run("workspace.closeTabs", json!({"uid": uids[0], "scope": "all", "interactive": true})).unwrap();
    h.run_steps(3);
    h.get_by_label("Save").click();
    h.run_steps(5);
    assert_eq!(h.state().session.documents().len(), 3);
    assert!(h.state().session.active().unwrap().is_dirty());
    assert!(h.state().workspace.inspect()["closingTabs"].is_null());
}

#[test]
fn save_prompt_cannot_discard_a_different_document_after_external_activation() {
    let dir = TempDir::new();
    let mut h = harness(Services::default());
    let uids = three_tabs(&mut h, &dir);
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "8"})).unwrap();
    h.state_mut().run("workspace.closeTabs", json!({"uid": uids[2], "interactive": true})).unwrap();
    h.state_mut().run("window.activate", json!({"index": 0})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Don't Save").is_none());
    h.get_by_label("Cancel").click();
    h.run_steps(4);
    assert_eq!(h.state().session.documents().len(), 3);
    assert!(h.state().session.documents()[2].is_dirty());
}

#[test]
fn failed_save_stops_batch_without_closing_later_tabs() {
    let dir = TempDir::new();
    let path = dir.0.join("missing/saved.xlsx").to_string_lossy().into_owned();
    let mut h = harness(Services { pick_save: Some(Box::new(move |_| Some(path.clone()))), ..Default::default() });
    let uids = three_tabs(&mut h, &dir);
    h.state_mut().run("window.activate", json!({"index": 0})).unwrap();
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "9"})).unwrap();
    h.state_mut().run("workspace.closeTabs", json!({"uid": uids[0], "scope": "all", "interactive": true})).unwrap();
    h.run_steps(3);
    h.get_by_label("Save").click();
    h.run_steps(4);
    assert!(h.state().message.is_some());
    h.state_mut().message = None;
    h.run_steps(4);
    assert_eq!(h.state().session.documents().iter().map(|d| d.uid).collect::<Vec<_>>(), uids);
    assert!(h.state().session.active().unwrap().is_dirty());
    assert!(h.state().workspace.inspect()["closingTabs"].is_null());
}
