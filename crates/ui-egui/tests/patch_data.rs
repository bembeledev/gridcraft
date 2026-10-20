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
        s.execute("range.setValues", json!({"range": "A1", "values": [["ALL;KEY", "ALL", "ALL"], ["int", "bool", "string"], ["Id", "Enabled", "Name"], [1, true, "First"], [1, false, "Second"]]})).unwrap();
        std::fs::write(&file, gridcraft_engine::io::save_bytes(&s.active().unwrap().wb, &file).unwrap()).unwrap();
        std::fs::write(root.join("BuildName"), "v1").unwrap();
        let mut meta = Session::new();
        meta.new_workbook();
        meta.execute("sheet.rename", json!({"name":"EnumList"})).unwrap();
        meta.execute(
            "range.setValues",
            json!({"range":"A1","values":[["ALL","ALL","ALL"],["string","string","int"],["EnumType","EnumName","EnumValue"]]}),
        )
        .unwrap();
        let metadata = root.join("protobuf/MetaData.xlsx");
        std::fs::write(&metadata, gridcraft_engine::io::save_bytes(&meta.active().unwrap().wb, metadata.to_str().unwrap()).unwrap()).unwrap();
        std::fs::create_dir(root.join("id-map")).unwrap();
        std::fs::write(root.join("id-map/PatchDataIdMap.csv"), "stringId,intId,status,note\nexisting,1000,LIVE,\n").unwrap();
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
fn configure(h: &mut egui_kittest::Harness<'static, SheetApp>, f: &Fixture) {
    let executable = std::env::var("GRIDCRAFT_TEST_CK_VALIDATOR").expect("set GRIDCRAFT_TEST_CK_VALIDATOR to the real patcher worker");
    h.state_mut().run("patch.configure", json!({"executable":executable,"id_map_directory":f.root.join("id-map"),"tc_id_prefix":"TC_"})).unwrap();
    h.state_mut().run("patch.refresh", json!({})).unwrap();
    settle(h);
}
fn inspect(h: &mut egui_kittest::Harness<'static, SheetApp>) -> Json {
    h.state_mut().run("patch.inspect", json!({})).unwrap()
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
    assert_eq!(sh.value(CellRef::new(0, 2)).display(), "ALL");
    assert_eq!(sh.value(CellRef::new(1, 2)).display(), "string");
    assert_eq!(sh.value(CellRef::new(2, 2)).display(), "Name");
}
#[test]
fn missing_configuration_never_runs_local_checks_and_settings_are_persisted() {
    let f = Fixture::new();
    let mut h = harness();
    open(&mut h, &f);
    let info = inspect(&mut h);
    assert!(info["error"].as_str().unwrap().contains("ID 맵"));
    assert_eq!(info["report"], Json::Null);
    assert_eq!(info["backend"], "ck-validator");
    h.state_mut().run("patch.configure", json!({"executable":"/no/such/worker","id_map_directory":f.root.join("id-map")})).unwrap();
    let restored: gridcraft_ui_egui::UiState = serde_json::from_value(serde_json::to_value(&h.state().ui).unwrap()).unwrap();
    assert_eq!(restored.patch_validators.values().next().unwrap().executable, "/no/such/worker");
    h.state_mut().run("patch.refresh", json!({})).unwrap();
    settle(&mut h);
    assert!(inspect(&mut h)["error"].as_str().unwrap().contains("실행 실패"));
    assert_eq!(inspect(&mut h)["report"], Json::Null);
    h.state_mut().run("patch.profile", json!({"mode":"off"})).unwrap();
    assert!(!h.state().patch_data.busy());
}
#[test]
#[ignore = "requires the real ck-patcher ck-validator; set GRIDCRAFT_TEST_CK_VALIDATOR"]
fn real_patcher_reports_unsaved_edits_and_preserves_workbooks_and_id_map() {
    let f = Fixture::new();
    let before = std::fs::read(&f.file).unwrap();
    let id_before = std::fs::read(f.root.join("id-map/PatchDataIdMap.csv")).unwrap();
    let mut h = harness();
    open(&mut h, &f);
    configure(&mut h, &f);
    let info = inspect(&mut h);
    assert_eq!(info["error"], Json::Null, "{info}");
    assert_eq!(info["report"]["valid"], false);
    let index = info["report"]["diagnostics"].as_array().unwrap().iter().position(|d| d["code"] == "key.duplicate").unwrap();
    h.state_mut().run("patch.jump", json!({"index":index})).unwrap();
    settle(&mut h);
    assert_eq!(h.state().session.active().unwrap().selection.active.a1(), "A5");
    h.state_mut().run("cell.set", json!({"cell":"A5","input":"2"})).unwrap();
    assert_eq!(inspect(&mut h)["stale"], true);
    h.state_mut().run("patch.refresh", json!({})).unwrap();
    settle(&mut h);
    let info = inspect(&mut h);
    assert_eq!(info["report"]["valid"], true, "{info}");
    assert_eq!(info["stale"], false);
    assert_eq!(std::fs::read(&f.file).unwrap(), before);
    assert_eq!(std::fs::read(f.root.join("id-map/PatchDataIdMap.csv")).unwrap(), id_before);
    // The engine/CLI command uses the same external process and carries unsaved edits too.
    let report = h.state_mut().run("patch.validate", json!({"project":"ck","root":f.root,"config":info["config"]})).unwrap();
    assert_eq!(report["report"], info["report"]);
}
#[test]
#[ignore = "requires the real ck-patcher ck-validator; set GRIDCRAFT_TEST_CK_VALIDATOR"]
fn real_patcher_failures_and_version_stage_are_not_presented_as_success() {
    let f = Fixture::new();
    let mut h = harness();
    open(&mut h, &f);
    configure(&mut h, &f);
    h.state_mut().run("patch.version", json!({"name":"@build"})).unwrap();
    settle(&mut h);
    let info = inspect(&mut h);
    assert_eq!(info["report"]["valid"], false);
    assert_eq!(info["report"]["stages"][2]["status"], "failed");
    assert_eq!(info["report"]["stages"][3]["status"], "skipped");
    std::fs::write(f.root.join("protobuf/BrokenData.xlsx"), b"broken").unwrap();
    std::fs::write(f.root.join("protobuf/_IgnoredData.xlsx"), b"broken").unwrap();
    h.state_mut().run("patch.refresh", json!({})).unwrap();
    settle(&mut h);
    let info = inspect(&mut h);
    assert_eq!(info["report"], Json::Null);
    assert!(info["error"].as_str().unwrap().contains("BrokenData.xlsx"));
    assert!(!info["error"].as_str().unwrap().contains("_IgnoredData.xlsx"));
    std::fs::remove_file(f.root.join("protobuf/BrokenData.xlsx")).unwrap();
    std::fs::remove_file(f.root.join("id-map/PatchDataIdMap.csv")).unwrap();
    h.state_mut().run("patch.refresh", json!({})).unwrap();
    settle(&mut h);
    assert_eq!(inspect(&mut h)["report"], Json::Null);
    assert!(inspect(&mut h)["error"].is_string());
}
#[test]
#[ignore = "requires the real ck-patcher ck-validator; set GRIDCRAFT_TEST_CK_VALIDATOR"]
fn real_patcher_inflight_edits_are_discarded_and_cancel_stops_the_job() {
    let f = Fixture::new();
    let mut h = harness();
    open(&mut h, &f);
    configure(&mut h, &f);
    h.state_mut().run("patch.refresh", json!({})).unwrap();
    h.state_mut().run("cell.set", json!({"cell":"A5","input":"2"})).unwrap();
    settle(&mut h);
    let info = inspect(&mut h);
    assert_eq!(info["report"], Json::Null);
    assert!(info["error"].as_str().unwrap().contains("문서가 변경"));
    h.state_mut().run("patch.refresh", json!({})).unwrap();
    h.state_mut().run("patch.cancel", json!({})).unwrap();
    settle(&mut h);
    assert_eq!(inspect(&mut h)["report"], Json::Null);
    h.state_mut().run("patch.refresh", json!({})).unwrap();
    h.state_mut().run("patch.profile", json!({"mode":"off"})).unwrap();
    settle(&mut h);
    assert_eq!(inspect(&mut h)["enabled"], false);
    assert_eq!(inspect(&mut h)["report"], Json::Null);
}
#[test]
#[ignore = "requires the real ck-patcher ck-validator; set GRIDCRAFT_TEST_CK_VALIDATOR"]
fn diagnostics_are_hidden_by_default_and_can_be_reopened_without_rechecking() {
    let f = Fixture::new();
    let mut h = harness();
    open(&mut h, &f);
    configure(&mut h, &f);
    let report = h.state_mut().run("patch.inspect", json!({})).unwrap()["report"].clone();
    assert!(!h.state().ui.patch_diagnostics_visible);
    assert!(h.query_by_label("수준").is_none());
    let grid_height = h.state().grid.cells_rect.unwrap().height();
    h.get_by_label("진단 열기").click();
    h.run_steps(4);
    h.get_by_label("수준");
    assert!(
        h.state().grid.cells_rect.unwrap().height() <= grid_height - 100.0,
        "before={grid_height}, after={}",
        h.state().grid.cells_rect.unwrap().height()
    );
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
#[ignore = "requires the real ck-patcher ck-validator; set GRIDCRAFT_TEST_CK_VALIDATOR"]
fn diagnostic_location_click_selects_and_reveals_cell_in_another_document() {
    let f = Fixture::new();
    let mut source = Session::new();
    source.execute("file.open", json!({"path": f.file})).unwrap();
    source.execute("cell.set", json!({"cell": "C2", "input": "string"})).unwrap();
    let rows: Vec<_> = (6..1000).map(|id| json!([id, true, "ok"])).collect();
    source.execute("range.setValues", json!({"range": "A6", "values": rows})).unwrap();
    source.execute("range.setValues", json!({"range": "A1000", "values": [[1, true, "ok"]]})).unwrap();
    source.execute("file.save", json!({})).unwrap();
    let before = std::fs::read(&f.file).unwrap();
    let mut h = harness();
    open(&mut h, &f);
    configure(&mut h, &f);
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
