//! CK authoring UI; all diagnostics come exclusively from the patcher's ck-validator.
use crate::{SheetApp, theme};
use egui::RichText;
#[cfg(not(target_arch = "wasm32"))]
use gridcraft_engine::patch_data::Request;
use gridcraft_engine::{
    core::CellRef,
    model::Workbook,
    patch_data::{self as data, Checked, Config, Location},
};
use serde_json::{Value as Json, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, TryRecvError},
    },
};

type Stamp = Vec<(u64, u64, bool, String)>;
struct Job {
    receiver: Receiver<Result<Loaded, String>>,
    cancel: Arc<AtomicBool>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
struct Loaded {
    checked: Checked,
    stamp: Stamp,
}
#[derive(Default)]
pub struct State {
    root_seen: Option<String>,
    mode: String,
    pub visible: bool,
    enabled: bool,
    pending: Option<Job>,
    progress: Arc<std::sync::Mutex<String>>,
    loaded: Option<Loaded>,
    error: Option<String>,
    config: Config,
    config_draft: Config,
    page: usize,
    draft: Option<Draft>,
}
struct Draft {
    uid: u64,
    revision: u64,
    sheet: usize,
    cell: CellRef,
    flags: String,
    typ: String,
    name: String,
    value: String,
    transposed: bool,
    dirty: bool,
}
fn needs_overlay(app: &SheetApp, doc: &gridcraft_engine::DocState) -> bool {
    (doc.is_dirty() || doc.path.is_none())
        && app.workspace.source_path(doc).is_some_and(|p| app.workspace.root().is_some_and(|r| data::workbook_file(r, &p).is_some()))
}
fn stamp(app: &SheetApp) -> Stamp {
    app.session
        .documents()
        .iter()
        .filter_map(|d| {
            let path = app.workspace.source_path(d)?;
            data::workbook_file(app.workspace.root()?, &path)?;
            Some((d.uid, d.revision, d.is_dirty(), path))
        })
        .collect()
}
fn stale(app: &SheetApp) -> bool {
    let Some(loaded) = &app.patch_data.loaded else { return false };
    let now = stamp(app);
    loaded.stamp.iter().any(|old| now.iter().find(|n| n.0 == old.0).map_or(old.2, |n| n != old))
        || now.iter().any(|n| n.2 && !loaded.stamp.iter().any(|old| old.0 == n.0))
}
impl State {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn inspect(&self) -> Json {
        json!({"enabled": self.enabled, "mode": self.mode, "visible": self.visible, "loading": self.busy(), "error": self.error,
            "progress": {"stage": self.progress.lock().ok().map(|p| p.clone())}, "config": self.config,
            "backend": "ck-validator", "validator": self.loaded.as_ref().map(|l| &l.checked.validator),
            "report": self.loaded.as_ref().map(|l| &l.checked.report)})
    }
}
fn detected(root: &str) -> bool {
    data::project_root(root).file_name().and_then(|s| s.to_str()).is_some_and(|s| s.starts_with("ck-patch-data") || s.starts_with("patch-data-ck"))
}
fn config_key(root: &str) -> String {
    data::project_root(root).to_string_lossy().into_owned()
}
fn reset(app: &mut SheetApp, mode: &str) {
    let root = app.workspace.root().map(str::to_string);
    let config = root.as_ref().and_then(|r| app.ui.patch_validators.get(&config_key(r))).cloned().unwrap_or_else(Config::from_environment);
    let enabled = mode == "ck" || (mode == "auto" && root.as_deref().is_some_and(detected));
    app.patch_data =
        State { root_seen: root, mode: mode.into(), enabled, visible: enabled, config: config.clone(), config_draft: config, ..Default::default() };
}
pub fn poll(app: &mut SheetApp, ctx: &egui::Context) {
    if app.workspace.root() != app.patch_data.root_seen.as_deref() {
        reset(app, "auto");
        if app.patch_data.enabled
            && let Err(e) = refresh(app)
        {
            app.patch_data.error = Some(e);
        }
    }
    let result = app.patch_data.pending.as_ref().and_then(|job| match job.receiver.try_recv() {
        Ok(v) => Some(v),
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => Some(Err("패처 검사 작업이 중단되었습니다. 다시 검사하세요.".into())),
    });
    if let Some(result) = result {
        app.patch_data.pending = None;
        match result {
            Ok(loaded) => {
                app.patch_data.loaded = Some(loaded);
                app.patch_data.error = None;
                if stale(app) {
                    app.patch_data.loaded = None;
                    app.patch_data.error = Some("검사 중 문서가 변경되었습니다. 다시 검사하세요.".into());
                }
            }
            Err(error) => {
                app.patch_data.loaded = None;
                app.patch_data.error = Some(error);
            }
        }
    }
    if app.patch_data.busy() {
        ctx.request_repaint_after(std::time::Duration::from_millis(80));
    }
}
fn refresh(app: &mut SheetApp) -> Result<Json, String> {
    if !app.patch_data.enabled {
        return Err("CK 프로필을 먼저 활성화하세요.".into());
    }
    if app.patch_data.busy() {
        return Err("검사를 취소하거나 끝난 후 다시 실행하세요.".into());
    }
    if !app.commit_edit(0, 0, false, false) {
        return Err("셀 편집을 먼저 완료하세요.".into());
    }
    let root = app.workspace.root().ok_or("데이터 폴더를 먼저 여세요.")?.to_string();
    app.patch_data.config.check()?;
    let overrides = app
        .session
        .documents()
        .iter()
        .filter(|d| needs_overlay(app, d))
        .filter_map(|d| app.workspace.source_path(d).map(|p| (p, d.wb.clone())))
        .collect();
    let snapshot = stamp(app);
    app.patch_data.loaded = None;
    app.patch_data.pending = Some(start(root, overrides, snapshot, app.patch_data.config.clone(), app.patch_data.progress.clone())?);
    app.patch_data.error = None;
    app.patch_data.page = 0;
    Ok(json!({"loading": true, "backend": "ck-validator"}))
}
#[cfg(not(target_arch = "wasm32"))]
fn start(
    root: String,
    overrides: BTreeMap<String, Arc<Workbook>>,
    stamp: Stamp,
    config: Config,
    progress: Arc<std::sync::Mutex<String>>,
) -> Result<Job, String> {
    let (tx, receiver) = std::sync::mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    if let Ok(mut p) = progress.lock() {
        *p = "snapshot".into();
    }
    std::thread::Builder::new()
        .name("gridcraft-ck-validator".into())
        .spawn(move || {
            let run = || -> Result<Loaded, String> {
                let overlays = data::overlays(&root, &overrides, &worker_cancel)?;
                let revision = serde_json::to_string(&stamp).map_err(|e| e.to_string())?;
                let request = Request::new(&root, &config, revision, overlays)?;
                let checked = data::check(&config, &request, &worker_cancel, |stage| {
                    if let Ok(mut p) = progress.lock() {
                        *p = stage.into();
                    }
                })?;
                if let Ok(mut p) = progress.lock() {
                    *p = "complete".into();
                }
                Ok(Loaded { checked, stamp })
            };
            let _ = tx.send(run());
        })
        .map_err(|e| e.to_string())?;
    Ok(Job { receiver, cancel })
}
#[cfg(target_arch = "wasm32")]
fn start(_: String, _: BTreeMap<String, Arc<Workbook>>, _: Stamp, _: Config, _: Arc<std::sync::Mutex<String>>) -> Result<Job, String> {
    Err("ck-validator 검사는 데스크톱에서 사용할 수 있습니다.".into())
}
fn configure(app: &mut SheetApp, p: &Json) -> Result<Json, String> {
    let config: Config = serde_json::from_value(p.clone()).map_err(|e| e.to_string())?;
    config.check()?;
    let root = app.workspace.root().ok_or("데이터 폴더를 먼저 여세요.")?;
    app.ui.patch_validators.insert(config_key(root), config.clone());
    app.patch_data.pending = None;
    app.patch_data.loaded = None;
    app.patch_data.error = None;
    app.patch_data.config = config.clone();
    app.patch_data.config_draft = config;
    Ok(app.patch_data.inspect())
}
pub fn command(app: &mut SheetApp, id: &str, p: &Json) -> Option<Result<Json, String>> {
    Some(match id {
        "patch.profile" => {
            let mode = p.get("mode").and_then(Json::as_str).unwrap_or("auto");
            if !["auto", "ck", "off"].contains(&mode) {
                return Some(Err("mode: auto | ck | off".into()));
            }
            reset(app, mode);
            if app.patch_data.enabled { refresh(app) } else { Ok(app.patch_data.inspect()) }
        }
        "patch.configure" => configure(app, p),
        "patch.refresh" => refresh(app),
        "patch.cancel" => {
            app.patch_data.pending = None;
            app.patch_data.loaded = None;
            app.patch_data.error = Some("검사가 취소되었습니다.".into());
            Ok(json!({"cancelled": true}))
        }
        "patch.inspect" => {
            let mut info = app.patch_data.inspect();
            info["stale"] = json!(stale(app));
            Ok(info)
        }
        "patch.version" => {
            let name = p.get("name").and_then(Json::as_str).unwrap_or_default();
            let mut config = app.patch_data.config.clone();
            config.version_filtering = !name.is_empty();
            config.build_name = (!name.is_empty() && name != "@build").then(|| name.to_string());
            let value = serde_json::to_value(config).map_err(|e| e.to_string());
            value.and_then(|p| configure(app, &p)).and_then(|_| refresh(app))
        }
        "view.patchData" => {
            app.patch_data.visible = p.get("on").and_then(Json::as_bool).unwrap_or(!app.patch_data.visible);
            Ok(json!({"on": app.patch_data.visible}))
        }
        "view.patchDiagnostics" => {
            app.ui.patch_diagnostics_visible = p.get("on").and_then(Json::as_bool).unwrap_or(!app.ui.patch_diagnostics_visible);
            Ok(json!({"on": app.ui.patch_diagnostics_visible}))
        }
        "patch.jump" => jump(app, p),
        "patch.apply" => apply(app, p),
        _ => return None,
    })
}
fn jump(app: &mut SheetApp, p: &Json) -> Result<Json, String> {
    let index = p.get("index").and_then(Json::as_u64).and_then(|n| usize::try_from(n).ok()).ok_or("진단 index가 필요합니다.")?;
    let issue = app.patch_data.loaded.as_ref().and_then(|l| l.checked.report.diagnostics.get(index)).ok_or("패처 진단을 찾지 못했습니다.")?;
    let loc = if let Some(index) = p.get("related").and_then(Json::as_u64).and_then(|n| usize::try_from(n).ok()) {
        issue.related.get(index).ok_or("관련 위치를 찾지 못했습니다.")?
    } else {
        &issue.location
    };
    let file = loc.path(app.workspace.root().ok_or("데이터 폴더가 없습니다.")?).ok_or("진단에 파일 위치가 없습니다.")?;
    let sheet = loc.sheet.clone();
    let cell = loc.cell();
    if !app.commit_edit(0, 0, false, false) {
        return Err("셀 편집을 먼저 완료하세요.".into());
    }
    if let Some(index) = app.session.documents().iter().position(|d| app.workspace.source_path(d).as_deref() == Some(&file)) {
        app.run("window.activate", json!({"index":index}))?;
    } else {
        app.open_path(&file);
    }
    if app.session.active().and_then(|d| app.workspace.source_path(d)).as_deref() != Some(&file) {
        return Err("파일을 열지 못했습니다.".into());
    }
    if let Some(sheet) = sheet {
        app.run("sheet.activate", json!({"sheet": sheet}))?;
        if let Some(cell) = cell {
            app.run("selection.set", json!({"cell":cell.a1()}))?;
            app.selected_chart = None;
            app.grid.reveal_moving_end = false;
            app.grid.ensure_visible = true;
        }
    }
    Ok(json!({"file":file}))
}
fn apply(app: &mut SheetApp, p: &Json) -> Result<Json, String> {
    if !app.patch_data.enabled {
        return Err("CK 프로필이 비활성화되어 있습니다.".into());
    }
    let d = app.session.active().ok_or("문서가 없습니다.")?;
    if p.get("uid").and_then(Json::as_u64) != Some(d.uid) || p.get("revision").and_then(Json::as_u64) != Some(d.revision) {
        return Err("문서가 변경되었습니다. 선택을 다시 불러온 후 적용하세요.".into());
    }
    let sheet = p.get("sheet").and_then(Json::as_u64).and_then(|v| usize::try_from(v).ok()).ok_or("sheet가 필요합니다.")?;
    let cell = p.get("cell").and_then(Json::as_str).and_then(CellRef::parse).ok_or("cell이 필요합니다.")?;
    let sh = d.wb.sheet(sheet).ok_or("시트가 없습니다.")?;
    if sh.name.starts_with('_')
        || !app.workspace.source_path(d).is_some_and(|path| app.workspace.root().is_some_and(|root| data::workbook_file(root, &path).is_some()))
    {
        return Err("CK 검사 대상 파일/시트에서만 적용할 수 있습니다.".into());
    }
    let transposed = sh.name.ends_with("_Transpose");
    if app.editor.is_some() {
        return Err("격자 셀 편집을 먼저 완료하세요.".into());
    }
    if p.get("kind").and_then(Json::as_str) == Some("schema") {
        let text = |key| p.get(key).and_then(Json::as_str).unwrap_or_default();
        let raw = [text("flags"), text("type"), text("name")];
        if raw.iter().any(|s| s.len() > 4096) {
            return Err("헤더가 너무 깁니다.".into());
        }
        let values = raw.map(|s| format!("'{s}"));
        let (start, values) = if transposed {
            (CellRef::new(cell.row, 0), json!([values]))
        } else {
            (CellRef::new(0, cell.col), json!([[values[0]], [values[1]], [values[2]]]))
        };
        app.run("range.setValues", json!({"sheet": sheet, "range": start.a1(), "values": values}))
    } else {
        let value = p.get("value").and_then(Json::as_str).ok_or("value가 필요합니다.")?;
        if value.len() > 32767 {
            return Err("셀 입력은 32,767바이트 이내여야 합니다.".into());
        }
        // Literal input preserves 64-bit integers, IDs, leading = and raw JSON. Formulas remain
        // available in the ordinary formula bar, never inferred by this typed data editor.
        app.run("cell.set", json!({"sheet": sheet, "cell": cell.a1(), "input": format!("'{value}")}))
    }
}

fn sync_draft(app: &mut SheetApp) {
    let Some(doc) = app.session.active() else { return };
    let cell = doc.selection.active;
    if app
        .patch_data
        .draft
        .as_ref()
        .is_some_and(|d| d.dirty || (d.uid == doc.uid && d.revision == doc.revision && d.sheet == doc.sheet_index() && d.cell == cell))
    {
        return;
    }
    let Some(sheet) = doc.wb.active() else { return };
    let transposed = sheet.name.ends_with("_Transpose");
    let header = |r| sheet.value(if transposed { CellRef::new(cell.row, r) } else { CellRef::new(r, cell.col) }).display();
    app.patch_data.draft = Some(Draft {
        uid: doc.uid,
        revision: doc.revision,
        sheet: doc.sheet_index(),
        cell,
        flags: header(0),
        typ: header(1),
        name: header(2),
        value: sheet.value(cell).display(),
        transposed,
        dirty: false,
    });
}

fn stage_label(stage: &str) -> &str {
    match stage {
        "snapshot" => "미저장 수정 전달",
        "load" => "데이터·ID 맵 읽기",
        "raw" => "시트 검사",
        "parse" => "스키마 읽기",
        "versions" => "버전 검사",
        "validation" => "패처 규칙 검사",
        "complete" => "완료",
        _ => stage,
    }
}
fn status_label(status: &str) -> &str {
    match status {
        "passed" => "통과",
        "failed" => "실패",
        "skipped" => "미실행",
        _ => status,
    }
}
fn location_label(loc: &Location) -> String {
    let sheet = loc.sheet.as_deref().or(loc.table.as_deref()).unwrap_or("위치 정보 없음");
    match loc.cell() {
        Some(cell) if loc.sheet.is_some() => format!("{sheet}!{}", cell.a1()),
        _ => sheet.into(),
    }
}
pub fn show(app: &mut SheetApp, ui: &mut egui::Ui) {
    if !app.patch_data.visible && !app.ui.patch_diagnostics_visible {
        return;
    }
    sync_draft(app);
    let t = theme::Tokens::get(ui.ctx());
    let stale = stale(app);
    let root = app.workspace.root().unwrap_or_default().to_string();
    let eligible = app.session.active().and_then(|d| app.workspace.source_path(d)).is_some_and(|p| data::workbook_file(&root, &p).is_some());
    let ignored = app.session.active().and_then(|d| d.wb.active()).is_some_and(|s| s.name.starts_with('_'));
    let mut action: Option<(&str, Json)> = None;
    let mut reset_draft = false;
    if app.patch_data.visible {
        egui::Panel::right("ck_rules").default_size(340.0).size_range(280.0..=540.0).resizable(true)
            .frame(egui::Frame::NONE.fill(t.window).inner_margin(12)).show(ui, |ui| {
            if app.dialog.is_some() || app.message.is_some() { ui.disable(); }
            ui.horizontal(|ui| { ui.heading("CK 데이터"); if ui.small_button("숨기기").clicked() { action = Some(("view.patchData", json!({"on":false}))); } });
            if !app.patch_data.enabled {
                ui.label("CK 패처용 폴더에서 사용하는 편집·검사 도구입니다.");
                if ui.button("이 폴더에 CK 규칙 적용").clicked() { action = Some(("patch.profile", json!({"mode":"ck"}))); }
                return;
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new("패처 검사").color(t.accent).strong());
                if ui.small_button("끄기").clicked() { action = Some(("patch.profile", json!({"mode":"off"}))); }
                if ui.add_enabled(!app.patch_data.busy(), egui::Button::new("다시 검사")).clicked() { action = Some(("patch.refresh", json!({}))); }
                if app.patch_data.busy() { ui.spinner(); if ui.small_button("취소").clicked() { action = Some(("patch.cancel", json!({}))); } }
            });
            if ui.button(if app.ui.patch_diagnostics_visible { "진단 닫기" } else { "진단 열기" }).clicked() { action = Some(("view.patchDiagnostics", json!({}))); }
            if app.patch_data.busy() && let Ok(stage) = app.patch_data.progress.lock() { ui.label(stage_label(&stage)); }
            if let Some(error) = &app.patch_data.error { ui.label(RichText::new(error).color(t.accent)); }
            if stale { ui.label(RichText::new("수정 전 결과입니다. 다시 검사하세요.").color(t.accent)); }
            egui::ScrollArea::vertical().id_salt("ck_editor_scroll").show(ui, |ui| {
                egui::CollapsingHeader::new("패처 검사 설정").default_open(app.patch_data.config.check().is_err()).show(ui, |ui| {
                    let draft = &mut app.patch_data.config_draft;
                    ui.label("검사 프로그램 (ck-validator)"); ui.text_edit_singleline(&mut draft.executable);
                    ui.label("ID 맵 폴더 (PatchDataIdMap.csv)"); ui.text_edit_singleline(&mut draft.id_map_directory);
                    ui.label("TC ID 접두사"); ui.text_edit_singleline(&mut draft.tc_id_prefix);
                    ui.checkbox(&mut draft.version_filtering, "버전 필터 적용");
                    let mut name = draft.build_name.clone().unwrap_or_default();
                    ui.label("대상 버전 (비우면 BuildName 사용)");
                    if ui.text_edit_singleline(&mut name).changed() { draft.build_name = (!name.is_empty()).then_some(name); }
                    ui.small("패처와 같은 ID 맵·버전·접두사를 지정하세요. 설정은 이 폴더에 기억됩니다.");
                    if ui.button("설정 적용").clicked() { action = Some(("patch.configure", json!(draft))); }
                });
                if let Some(loaded) = &app.patch_data.loaded {
                    ui.separator();
                    let report = &loaded.checked.report;
                    ui.strong(if stale { "이전 검사 결과" } else if report.valid { "패처 검사 통과" } else { "패처 검사 실패" });
                    egui::Grid::new("ck_stages").striped(true).show(ui, |ui| {
                        for stage in &report.stages { ui.label(stage_label(&stage.stage)); ui.label(status_label(&stage.status)); ui.end_row(); }
                    });
                    ui.small(format!("버전 제외 {}행 · 신규 ID 시뮬레이션 {}개", report.filtered_rows, report.simulated_new_ids));
                    ui.small(format!("적용 버전: {}", report.effective_build_name.as_deref().unwrap_or("없음")));
                    ui.collapsing("검사 프로그램 버전", |ui| { ui.label(format!("{} · protocol {}", loaded.checked.validator.package_version, loaded.checked.validator.protocol_version)); ui.label(&loaded.checked.validator.source_revision); });
                }
                ui.small("검사 결과는 패처에서 가져옵니다. 실패한 단계 이후 검사는 미실행으로 표시됩니다.");
                ui.small("원본 파일·ID 맵은 변경하지 않습니다. 게시·바이너리 생성은 패처에서 수행하세요.");
                ui.separator();
                if !eligible || ignored { ui.label("protobuf 폴더의 데이터 파일을 선택하세요."); }
                else if let Some(draft) = &mut app.patch_data.draft {
                    ui.strong(format!("{} · {}", draft.cell.a1(), draft.name));
                    ui.small(if draft.transposed { "전치 시트: A열 플래그 / B열 타입 / C열 필드명" } else { "1행 플래그 / 2행 타입 / 3행 필드명" });
                    ui.label("플래그"); draft.dirty |= ui.text_edit_singleline(&mut draft.flags).changed();
                    ui.label("타입"); draft.dirty |= ui.text_edit_singleline(&mut draft.typ).changed();
                    ui.label("필드명"); draft.dirty |= ui.text_edit_singleline(&mut draft.name).changed();
                    if ui.button("헤더 3칸 적용").clicked() { action = Some(("patch.apply", json!({"uid":draft.uid,"revision":draft.revision,"sheet":draft.sheet,"cell":draft.cell.a1(),"kind":"schema","flags":draft.flags,"type":draft.typ,"name":draft.name}))); }
                    if (if draft.transposed { draft.cell.col } else { draft.cell.row }) >= 3 {
                        ui.label("셀 값"); draft.dirty |= ui.add(egui::TextEdit::multiline(&mut draft.value).desired_rows(4).desired_width(f32::INFINITY)).changed();
                        if ui.button("셀 값 적용").clicked() { action = Some(("patch.apply", json!({"uid":draft.uid,"revision":draft.revision,"sheet":draft.sheet,"cell":draft.cell.a1(),"value":draft.value}))); }
                        ui.small("긴 정수·ID는 텍스트로 보존합니다. 수식은 수식 입력줄에서 편집하세요.");
                    }
                    if draft.dirty && ui.button("입력 취소 · 현재 선택 다시 읽기").clicked() { reset_draft = true; }
                }
            });
        });
    }
    if app.ui.patch_diagnostics_visible
        && let Some(loaded) = &app.patch_data.loaded
    {
        egui::Panel::bottom("ck_diagnostics")
            .default_size(190.0)
            .size_range(100.0..=400.0)
            .resizable(true)
            .frame(egui::Frame::NONE.fill(t.window).inner_margin(8))
            .show(ui, |ui| {
                if app.dialog.is_some() || app.message.is_some() {
                    ui.disable();
                }
                let report = &loaded.checked.report;
                ui.horizontal(|ui| {
                    if ui.small_button("진단 닫기").clicked() {
                        action = Some(("view.patchDiagnostics", json!({"on":false})));
                    }
                    ui.strong(format!("패처 진단 · {}건{}", report.total_diagnostics, if stale { " · 이전 결과" } else { "" }));
                    if report.truncated {
                        ui.label(format!("패처에서 전달한 {}건 표시", report.diagnostics.len()));
                    }
                    let pages = report.diagnostics.len().div_ceil(25).max(1);
                    app.patch_data.page = app.patch_data.page.min(pages.saturating_sub(1));
                    if ui.add_enabled(app.patch_data.page > 0, egui::Button::new("이전")).clicked() {
                        app.patch_data.page = app.patch_data.page.saturating_sub(1);
                    }
                    ui.label(format!("{} / {pages}", app.patch_data.page + 1));
                    if ui.add_enabled(app.patch_data.page + 1 < pages, egui::Button::new("다음")).clicked() {
                        app.patch_data.page += 1;
                    }
                });
                if report.diagnostics.is_empty() {
                    ui.label(if report.valid {
                        "패처 검사에서 발견된 문제가 없습니다."
                    } else {
                        "검사 단계 상태를 확인하세요."
                    });
                }
                egui::ScrollArea::both().id_salt("ck_issues_scroll").show(ui, |ui| {
                    egui::Grid::new("ck_issues_table").striped(true).num_columns(4).min_col_width(80.0).show(ui, |ui| {
                        for header in ["수준", "파일", "위치", "내용"] {
                            ui.strong(header);
                        }
                        ui.end_row();
                        for (index, issue) in report.diagnostics.iter().enumerate().skip(app.patch_data.page.saturating_mul(25)).take(25) {
                            ui.label(if issue.severity == "error" { "오류" } else { &issue.severity });
                            ui.label(issue.location.file.as_deref().unwrap_or("—"));
                            let label = location_label(&issue.location);
                            if issue.location.path(&root).is_some() {
                                if ui.link(label).clicked() {
                                    action = Some(("patch.jump", json!({"index":index})));
                                }
                            } else {
                                ui.label(label);
                            }
                            ui.vertical(|ui| {
                                ui.label(&issue.message).on_hover_text(&issue.code);
                                for (related, loc) in issue.related.iter().enumerate() {
                                    if loc.path(&root).is_some()
                                        && ui.link(format!("관련: {} / {}", loc.file.as_deref().unwrap_or_default(), location_label(loc))).clicked()
                                    {
                                        action = Some(("patch.jump", json!({"index":index,"related":related})));
                                    }
                                }
                            });
                            ui.end_row();
                        }
                    });
                });
            });
    }
    if reset_draft {
        app.patch_data.draft = None;
    }
    if let Some((id, p)) = action {
        match app.run(id, p) {
            Ok(_) => {
                if id == "patch.apply" {
                    app.patch_data.draft = None;
                }
            }
            Err(e) => app.patch_data.error = Some(e),
        }
    }
}
