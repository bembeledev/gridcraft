#![cfg(not(target_arch = "wasm32"))]
use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::{
    Session,
    core::{CellRef, Value},
};
use gridcraft_ui_egui::{Services, SheetApp};
use serde_json::{Value as Json, json};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
struct Fixture {
    root: PathBuf,
    file: String,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!("ck-patch-data-gridcraft-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(root.join("protobuf")).unwrap();
        let root = root.canonicalize().unwrap();
        let file = root.join("protobuf/ItemData.xlsx").to_string_lossy().into_owned();
        let mut s = Session::new();
        s.new_workbook();
        s.execute("sheet.rename", json!({"name": "Item"})).unwrap();
        s.execute("range.setValues", json!({"range": "A1", "values": [["All;Key", "All", "All"], ["Long", "Bool", "TString<Item.[Id]>"], ["Id", "Enabled", "Name"], [1, true, "First"], [1, false, "Second"]]})).unwrap();
        std::fs::write(&file, gridcraft_engine::io::save_bytes(&s.active().unwrap().wb, &file).unwrap()).unwrap();
        std::fs::write(root.join("BuildName"), "v1").unwrap();
        Self { root, file }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn harness() -> egui_kittest::Harness<'static, SheetApp> {
    let mut s = Session::new();
    s.new_workbook();
    egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(s, Services::default()),
    )
}
fn settle(h: &mut egui_kittest::Harness<'static, SheetApp>) {
    h.run_steps(2);
    for _ in 0..5000 {
        if !h.state().workspace.busy() && !h.state().patch_data.busy() {
            break;
        }
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(!h.state().workspace.busy() && !h.state().patch_data.busy());
    h.run_steps(3);
}
fn open(h: &mut egui_kittest::Harness<'static, SheetApp>, f: &Fixture) {
    h.state_mut().run("workspace.open", json!({"path": f.root})).unwrap();
    settle(h);
    h.state_mut().open_path(&f.file);
    settle(h);
}
fn request(h: &egui_kittest::Harness<'static, SheetApp>, cell: &str) -> Json {
    let d = h.state().session.active().unwrap();
    json!({"uid": d.uid, "revision": d.revision, "sheet": d.sheet_index(), "cell": cell})
}
#[test]
fn automatic_ck_profile_diagnostics_jump_and_unsaved_refresh() {
    let f = Fixture::new();
    let before = std::fs::read(&f.file).unwrap();
    let mut h = harness();
    open(&mut h, &f);
    let info = h.state_mut().run("patch.inspect", json!({})).unwrap();
    assert_eq!(info["enabled"], true);
    assert_eq!(info["stale"], false, "opening a saved file must not invalidate the disk index");
    let issues = info["report"]["issues"].as_array().unwrap();
    assert!(issues.iter().any(|i| i["code"] == "ck.tstring"));
    let issue = issues.iter().find(|i| i["code"] == "key.duplicate").unwrap();
    h.state_mut().run("patch.jump", issue.clone()).unwrap();
    settle(&mut h);
    assert_eq!(h.state().session.active().unwrap().selection.active.a1(), "A5");
    h.state_mut().run("cell.set", json!({"cell": "A5", "input": "2"})).unwrap();
    assert_eq!(h.state_mut().run("patch.inspect", json!({})).unwrap()["stale"], true);
    h.state_mut().run("patch.refresh", json!({})).unwrap();
    settle(&mut h);
    let info = h.state_mut().run("patch.inspect", json!({})).unwrap();
    assert_eq!(info["stale"], false);
    assert!(!info["report"]["issues"].as_array().unwrap().iter().any(|i| i["code"] == "key.duplicate"));
    assert_eq!(std::fs::read(&f.file).unwrap(), before);
    h.get_by_label("CK 데이터");
    h.get_by_label("헤더 3칸 적용");
}
#[test]
fn literal_edit_precision_atomic_header_undo_and_stale_guard() {
    let f = Fixture::new();
    let mut h = harness();
    open(&mut h, &f);
    let mut p = request(&h, "A4");
    p["value"] = json!("9223372036854775807");
    h.state_mut().run("patch.apply", p.clone()).unwrap();
    assert_eq!(h.state().session.active().unwrap().wb.active().unwrap().value(CellRef::new(3, 0)), Value::from("9223372036854775807"));
    assert!(h.state_mut().run("patch.apply", p).is_err());
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.active().unwrap().wb.active().unwrap().value(CellRef::new(3, 0)), Value::Number(1.0));
    let mut p = request(&h, "C4");
    p["kind"] = json!("schema");
    p["flags"] = json!("All;NotBlank");
    p["type"] = json!("OptimizedTString<Item.[Id]>");
    p["name"] = json!("Title");
    h.state_mut().run("patch.apply", p).unwrap();
    assert_eq!(h.state().session.active().unwrap().wb.active().unwrap().value(CellRef::new(2, 2)).display(), "Title");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    let sh = h.state().session.active().unwrap().wb.active().unwrap();
    assert_eq!(sh.value(CellRef::new(0, 2)).display(), "All");
    assert_eq!(sh.value(CellRef::new(1, 2)).display(), "TString<Item.[Id]>");
    assert_eq!(sh.value(CellRef::new(2, 2)).display(), "Name");
}
#[test]
fn unrelated_folders_remain_generic_and_failed_index_is_not_success() {
    let f = Fixture::new();
    let root = f.root.join("unrelated");
    std::fs::create_dir(&root).unwrap();
    let mut h = harness();
    h.state_mut().run("workspace.open", json!({"path": root})).unwrap();
    settle(&mut h);
    assert_eq!(h.state_mut().run("patch.inspect", json!({})).unwrap()["enabled"], false);
    assert!(h.state_mut().run("patch.refresh", json!({})).is_err());
    h.state_mut().run("patch.profile", json!({"mode": "ck"})).unwrap();
    settle(&mut h);
    let info = h.state_mut().run("patch.inspect", json!({})).unwrap();
    assert_eq!(info["report"]["complete_context"], false);
    assert!(!info["notes"].as_array().unwrap().is_empty());
    let mut p = request(&h, "A1");
    p["value"] = json!("42");
    assert!(h.state_mut().run("patch.apply", p).is_err());
}
#[test]
fn excluded_and_broken_files_are_not_silently_accepted() {
    let f = Fixture::new();
    std::fs::write(f.root.join("protobuf/BrokenData.xlsx"), b"invalid").unwrap();
    std::fs::write(f.root.join("protobuf/_IgnoredData.xlsx"), b"invalid").unwrap();
    let mut h = harness();
    open(&mut h, &f);
    let info = h.state_mut().run("patch.inspect", json!({})).unwrap();
    assert_eq!(info["report"]["complete_context"], false);
    assert_eq!(info["files"].as_array().unwrap().len(), 1);
    let notes = info["notes"].to_string();
    assert!(notes.contains("BrokenData.xlsx"));
    assert!(!notes.contains("_IgnoredData.xlsx"));
}

#[test]
fn index_checks_every_file_beyond_previous_256_file_limit() {
    let f = Fixture::new();
    let bytes = std::fs::read(&f.file).unwrap();
    for i in 0..260 {
        std::fs::write(f.root.join(format!("protobuf/Copy{i:03}Data.xlsx")), &bytes).unwrap();
    }
    let mut h = harness();
    h.state_mut().run("workspace.open", json!({"path":f.root})).unwrap();
    settle(&mut h);
    let info = h.state_mut().run("patch.inspect", json!({})).unwrap();
    assert_eq!(info["files"].as_array().unwrap().len(), 261);
    assert_eq!(info["progress"]["discovered"], 261);
    assert_eq!(info["progress"]["processed"], 261);
    assert_eq!(info["progress"]["failed"], 0);
    assert_eq!(info["report"]["complete_context"], true);
    assert_eq!(info["report"]["included_rows"], 522);
}

#[test]
fn switching_profile_cancels_inflight_index_and_never_adopts_old_results() {
    let f = Fixture::new();
    let mut h = harness();
    h.state_mut().run("workspace.open", json!({"path":f.root})).unwrap();
    settle(&mut h);
    h.state_mut().run("patch.refresh", json!({})).unwrap();
    assert!(h.state().patch_data.busy());
    h.state_mut().run("patch.profile", json!({"mode":"off"})).unwrap();
    settle(&mut h);
    let info = h.state_mut().run("patch.inspect", json!({})).unwrap();
    assert_eq!(info["enabled"], false);
    assert_eq!(info["report"], Json::Null);
}

#[test]
fn diagnostics_are_hidden_by_default_and_can_be_reopened_without_rechecking() {
    let f = Fixture::new();
    let mut h = harness();
    open(&mut h, &f);
    let report = h.state_mut().run("patch.inspect", json!({})).unwrap()["report"].clone();
    assert!(!h.state().ui.patch_diagnostics_visible);
    assert!(h.query_by_label("수준").is_none());
    let grid_height = h.state().grid.cells_rect.unwrap().height();
    h.get_by_label("진단 열기").click();
    h.run_steps(4);
    h.get_by_label("수준");
    assert!(h.state().grid.cells_rect.unwrap().height() < grid_height - 100.0);
    assert!(h.state().ui.patch_diagnostics_visible);
    let restored: gridcraft_ui_egui::UiState = serde_json::from_value(serde_json::to_value(&h.state().ui).unwrap()).unwrap();
    assert!(restored.patch_diagnostics_visible);
    h.state_mut().run("view.patchData", json!({"on": false})).unwrap();
    h.run_steps(4);
    h.get_by_label("진단 닫기").click();
    h.run_steps(4);
    assert!(!h.state().ui.patch_diagnostics_visible);
    assert!(h.query_by_label("수준").is_none());
    h.state_mut().run("view.patchData", json!({"on": true})).unwrap();
    h.run_steps(4);
    h.get_by_label("진단 열기").click();
    h.run_steps(4);
    assert_eq!(h.state_mut().run("patch.inspect", json!({})).unwrap()["report"], report);
    h.state_mut().run("view.patchDiagnostics", json!({"on": false})).unwrap();
    h.state_mut().run("patch.refresh", json!({})).unwrap();
    settle(&mut h);
    assert!(!h.state().ui.patch_diagnostics_visible, "refresh must not reopen a dismissed panel");
    assert!(h.query_by_label("수준").is_none());
}

#[test]
fn diagnostic_location_click_selects_and_reveals_cell_in_another_document() {
    let f = Fixture::new();
    let mut source = Session::new();
    source.execute("file.open", json!({"path": f.file})).unwrap();
    source.execute("cell.set", json!({"cell": "C2", "input": "String"})).unwrap();
    let rows: Vec<_> = (6..1000).map(|id| json!([id, true, "ok"])).collect();
    source.execute("range.setValues", json!({"range": "A6", "values": rows})).unwrap();
    source.execute("range.setValues", json!({"range": "A1000", "values": [[1, true, "ok"]]})).unwrap();
    source.execute("file.save", json!({})).unwrap();
    let before = std::fs::read(&f.file).unwrap();
    let mut h = harness();
    open(&mut h, &f);
    let uid = h.state().session.active().unwrap().uid;
    h.state_mut().run("view.patchDiagnostics", json!({"on": true})).unwrap();
    h.state_mut().view_mut().unwrap().scroll = egui::vec2(2000.0, 50_000.0);
    h.state_mut().run("file.new", json!({})).unwrap();
    h.state_mut().run("cell.set", json!({"cell": "B2", "input": "keep this edit"})).unwrap();
    h.run_steps(4);
    h.get_by_label("Item!A1000").click();
    h.run_steps(5);
    let app = h.state();
    let doc = app.session.active().unwrap();
    let target = CellRef::parse("A1000").unwrap();
    assert_eq!(doc.uid, uid);
    assert_eq!(doc.wb.active().unwrap().name, "Item");
    assert_eq!(doc.selection.active, target);
    assert!(doc.selection.is_single_cell());
    let geo = app.grid.geo.as_ref().unwrap();
    let rect = geo.cell_rect(doc.wb.active().unwrap(), target);
    assert!(geo.cells.contains_rect(rect), "diagnostic cell must be visible: {rect:?} in {:?}", geo.cells);
    assert_eq!(app.session.documents().len(), 2);
    assert!(app.session.documents().iter().any(|d| d.uid != uid && d.is_dirty()));
    assert_eq!(std::fs::read(&f.file).unwrap(), before);
    h.get_by_label("Item!A5").click();
    h.run_steps(5);
    let app = h.state();
    let doc = app.session.active().unwrap();
    let target = CellRef::parse("A5").unwrap();
    assert_eq!(doc.selection.active, target);
    let geo = app.grid.geo.as_ref().unwrap();
    assert!(geo.cells.contains_rect(geo.cell_rect(doc.wb.active().unwrap(), target)));
}
