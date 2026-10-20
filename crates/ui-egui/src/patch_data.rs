//! Opt-in CK authoring panel. Disk indexing and validation run on a bounded background worker.
//! No writes occur here: edits and navigation use ordinary GridCraft commands.
use crate::{SheetApp, theme};
use egui::RichText;
#[cfg(not(target_arch = "wasm32"))]
use gridcraft_engine::patch_data as data;
use gridcraft_engine::{
    core::CellRef,
    model::Workbook,
    patch_data::{
        Context, Report,
        schema::{self, Type},
        validate::Catalog,
    },
};
use serde_json::{Value as Json, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        mpsc::{Receiver, TryRecvError},
    },
};

type Stamp = Vec<(u64, u64)>;
#[derive(Clone, Default, serde::Serialize)]
struct Progress {
    stage: String,
    discovered: usize,
    processed: usize,
    failed: usize,
    current: String,
    rows: usize,
}
struct Job {
    receiver: Receiver<Result<Loaded, String>>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
#[cfg(not(target_arch = "wasm32"))]
fn progress(state: &Arc<std::sync::Mutex<Progress>>, update: impl FnOnce(&mut Progress)) {
    if let Ok(mut p) = state.lock() {
        update(&mut p);
    }
}
#[cfg(not(target_arch = "wasm32"))]
fn cancelled(cancel: &std::sync::atomic::AtomicBool) -> Result<(), String> {
    if cancel.load(std::sync::atomic::Ordering::Relaxed) { Err("검사가 취소되었습니다.".into()) } else { Ok(()) }
}

struct Loaded {
    context: Context,
    report: Report,
    catalog: Arc<Catalog>,
    stamp: Stamp,
}
#[derive(Default)]
pub struct State {
    root_seen: Option<String>,
    mode: String,
    pub visible: bool,
    enabled: bool,
    pending: Option<Job>,
    progress: Arc<std::sync::Mutex<Progress>>,
    loaded: Option<Loaded>,
    error: Option<String>,
    version: String,
    search: String,
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
// Opening a saved workbook is navigation, not a data edit. Index the saved cached values
// unless a document has edits, or an explicitly opened decrypted/imported model supplies it.
fn needs_overlay(app: &SheetApp, doc: &gridcraft_engine::DocState) -> bool {
    (doc.is_dirty() || doc.path.is_none()) && app.workspace.source_path(doc).is_some()
}
fn stamp(app: &SheetApp) -> Stamp {
    app.session.documents().iter().filter(|d| needs_overlay(app, d)).map(|d| (d.uid, d.revision)).collect()
}
impl State {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn inspect(&self) -> Json {
        json!({"enabled": self.enabled, "mode": self.mode, "visible": self.visible, "loading": self.busy(), "error": self.error,
            "progress": self.progress.lock().ok().map(|p| p.clone()), "version": self.version, "report": self.loaded.as_ref().map(|l| &l.report),
            "notes": self.loaded.as_ref().map(|l| &l.context.notes), "files": self.loaded.as_ref().map(|l| l.context.files.keys().collect::<Vec<_>>())})
    }
}

fn detected(root: &str) -> bool {
    let path = std::path::Path::new(root);
    let project = if path.file_name().is_some_and(|n| n == "protobuf") { path.parent().unwrap_or(path) } else { path };
    project.file_name().and_then(|s| s.to_str()).is_some_and(|s| s.starts_with("ck-patch-data") || s.starts_with("patch-data-ck"))
}

pub fn poll(app: &mut SheetApp, ctx: &egui::Context) {
    let root = app.workspace.root().map(str::to_string);
    if root != app.patch_data.root_seen {
        app.patch_data = State {
            root_seen: root.clone(),
            enabled: root.as_deref().is_some_and(detected),
            visible: root.as_deref().is_some_and(detected),
            ..Default::default()
        };
        if app.patch_data.enabled
            && let Err(e) = refresh(app)
        {
            app.patch_data.error = Some(e);
        }
    }
    let result = app.patch_data.pending.as_ref().and_then(|rx| match rx.receiver.try_recv() {
        Ok(v) => Some(v),
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => Some(Err("규칙 검사 작업이 중단되었습니다. 다시 검사하세요.".into())),
    });
    if let Some(result) = result {
        app.patch_data.pending = None;
        match result {
            Ok(loaded) => {
                app.patch_data.loaded = Some(loaded);
                app.patch_data.error = None;
            }
            Err(e) => app.patch_data.error = Some(e),
        }
    }
    if app.patch_data.busy() {
        ctx.request_repaint_after(std::time::Duration::from_millis(80));
    }
}

fn refresh(app: &mut SheetApp) -> Result<Json, String> {
    if !app.patch_data.enabled {
        return Err("CK 규칙을 먼저 활성화하세요.".into());
    }
    if app.patch_data.busy() {
        return Err("검사가 끝난 후 다시 실행하세요.".into());
    }
    let root = app.workspace.root().ok_or("데이터 폴더를 먼저 여세요.")?.to_string();
    let overrides = app
        .session
        .documents()
        .iter()
        .filter(|d| needs_overlay(app, d))
        .filter_map(|d| app.workspace.source_path(d).map(|p| (p, d.wb.clone())))
        .collect();
    app.patch_data.pending = Some(start(root, overrides, stamp(app), app.patch_data.version.clone(), app.patch_data.progress.clone())?);
    app.patch_data.error = None;
    app.patch_data.page = 0;
    Ok(json!({"loading": true}))
}

#[cfg(not(target_arch = "wasm32"))]
fn start(
    root: String,
    overrides: BTreeMap<String, Arc<Workbook>>,
    stamp: Stamp,
    version: String,
    state: Arc<std::sync::Mutex<Progress>>,
) -> Result<Job, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    progress(&state, |p| *p = Progress { stage: "파일 찾기".into(), ..Default::default() });
    std::thread::Builder::new()
        .name("gridcraft-ck-rules".into())
        .spawn(move || {
            let result = load(&root, overrides, stamp, &version, &state, &worker_cancel);
            let _ = tx.send(result);
        })
        .map_err(|e| e.to_string())?;
    Ok(Job { receiver: rx, cancel })
}
#[cfg(target_arch = "wasm32")]
fn start(_: String, _: BTreeMap<String, Arc<Workbook>>, _: Stamp, _: String, _: Arc<std::sync::Mutex<Progress>>) -> Result<Job, String> {
    Err("폴더 규칙 인덱스는 데스크톱에서 사용할 수 있습니다.".into())
}

#[cfg(not(target_arch = "wasm32"))]
fn load(
    root: &str,
    mut overrides: BTreeMap<String, Arc<Workbook>>,
    stamp: Stamp,
    version: &str,
    state: &Arc<std::sync::Mutex<Progress>>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Loaded, String> {
    use std::{
        fs,
        io::{BufReader, BufWriter, Read, Seek, Write},
        path::Path,
    };
    let project = Path::new(root);
    let dir = if project.join("protobuf").is_dir() { project.join("protobuf") } else { project.to_path_buf() };
    let project = if dir.file_name().is_some_and(|s| s == "protobuf") { dir.parent().unwrap_or(&dir) } else { project };
    let build_name = fs::File::open(project.join("BuildName"))
        .ok()
        .and_then(|f| {
            let mut s = String::new();
            f.take(4096).read_to_string(&mut s).ok().map(|_| s.trim().to_string())
        })
        .unwrap_or_default();
    let mut context = Context { root: Some(dir.to_string_lossy().into()), complete: true, build_name, ..Default::default() };
    let mut paths = Vec::new();
    for item in fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        cancelled(cancel)?;
        match item {
            Ok(item) => {
                if item.file_name().to_str().is_some_and(schema::eligible_file) {
                    match item.file_type() {
                        Ok(kind) if kind.is_file() => paths.push(item.path()),
                        Ok(_) => {}
                        Err(e) => {
                            context.complete = false;
                            context.notes.push(format!("{}: {e}", item.path().display()));
                        }
                    }
                }
            }
            Err(e) => {
                context.complete = false;
                context.notes.push(e.to_string());
            }
        }
    }
    paths.sort();
    progress(state, |p| {
        p.discovered = paths.len();
        p.stage = "파일 읽기".into();
    });
    // Disk-backed, three passes: metadata, version-filtered keys, then cross-file validation.
    // Only one workbook, small row chunks and reference sets are resident at a time.
    let mut spool = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut catalog = Catalog::default();
    {
        let mut writer = BufWriter::new(&mut spool);
        for path in paths {
            cancelled(cancel)?;
            let name = path.to_string_lossy().to_string();
            progress(state, |p| p.current.clone_from(&name));
            let result = if let Some(wb) = overrides.remove(&name) {
                Ok((wb, Vec::new()))
            } else {
                gridcraft_engine::io::read_file(&name)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| data::read_xlsx(&bytes))
                    .map(|(wb, warnings)| (Arc::new(wb), warnings))
            };
            match result {
                Ok((wb, warnings)) => {
                    for warning in warnings {
                        context.notes.push(format!("{name}: {warning}"));
                        context.complete = false;
                    }
                    let mut headers = Vec::new();
                    data::visit_chunks(&name, &wb, |chunk| {
                        cancelled(cancel)?;
                        if chunk.rows.is_empty() {
                            headers.push(chunk.clone());
                        }
                        data::validate::add_metadata(&mut catalog, &chunk);
                        serde_json::to_writer(&mut writer, &chunk).map_err(|e| e.to_string())?;
                        writer.write_all(b"\n").map_err(|e| e.to_string())
                    })?;
                    context.files.insert(name, headers);
                }
                Err(e) => {
                    context.complete = false;
                    context.notes.push(format!("{name}: {e}"));
                    progress(state, |p| p.failed += 1);
                }
            }
            progress(state, |p| p.processed += 1);
        }
        writer.flush().map_err(|e| e.to_string())?;
    }
    if context.files.is_empty() {
        context.complete = false;
        context.notes.push("최상위 *Data.xlsx 파일이 없습니다. protobuf 폴더 또는 그 상위 프로젝트를 선택하세요.".into());
    }
    let target = if version == "@build" { context.build_name.as_str() } else { version };
    progress(state, |p| {
        p.stage = "참조 인덱스".into();
        p.current.clear();
    });
    spool.rewind().map_err(|e| e.to_string())?;
    for chunk in serde_json::Deserializer::from_reader(BufReader::new(&mut spool)).into_iter::<data::DataSheet>() {
        cancelled(cancel)?;
        data::validate::add_keys(&mut catalog, &chunk.map_err(|e| e.to_string())?, Some(target));
    }
    let catalog = Arc::new(catalog);
    let mut validator = data::validate::Validator::new(catalog.clone(), Some(target.into()), context.complete);
    progress(state, |p| p.stage = "전체 행 검사".into());
    spool.rewind().map_err(|e| e.to_string())?;
    for chunk in serde_json::Deserializer::from_reader(BufReader::new(&mut spool)).into_iter::<data::DataSheet>() {
        cancelled(cancel)?;
        let chunk = chunk.map_err(|e| e.to_string())?;
        validator.push(&chunk);
        progress(state, |p| {
            p.rows += chunk.rows.len();
            p.current = chunk.file;
        });
    }
    progress(state, |p| {
        p.stage = "완료".into();
        p.current.clear();
    });
    Ok(Loaded { context, report: validator.report, catalog, stamp })
}

pub fn command(app: &mut SheetApp, id: &str, p: &Json) -> Option<Result<Json, String>> {
    Some(match id {
        "patch.profile" => {
            let mode = p.get("mode").and_then(Json::as_str).unwrap_or("auto");
            if !["auto", "ck", "off"].contains(&mode) {
                return Some(Err("mode: auto | ck | off".into()));
            }
            app.patch_data = State {
                root_seen: app.workspace.root().map(str::to_string),
                mode: mode.into(),
                enabled: mode == "ck" || (mode == "auto" && app.workspace.root().is_some_and(detected)),
                visible: mode != "off",
                ..Default::default()
            };
            if app.patch_data.enabled { refresh(app) } else { Ok(app.patch_data.inspect()) }
        }
        "patch.refresh" => refresh(app),
        "patch.inspect" => {
            let mut info = app.patch_data.inspect();
            info["stale"] = json!(app.patch_data.loaded.as_ref().is_some_and(|l| l.stamp != stamp(app)));
            Ok(info)
        }
        "patch.version" => {
            if app.patch_data.busy() {
                return Some(Err("검사가 끝난 후 버전을 변경하세요.".into()));
            }
            app.patch_data.version = p.get("name").and_then(Json::as_str).unwrap_or_default().to_string();
            refresh(app)
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
    let file = p.get("file").and_then(Json::as_str).ok_or("file이 필요합니다.")?;
    if !app.patch_data.loaded.as_ref().is_some_and(|l| l.context.files.contains_key(file)) {
        return Err("검사 인덱스에 없는 파일입니다.".into());
    }
    if !app.commit_edit(0, 0, false, false) {
        return Err("셀 편집을 먼저 완료하세요.".into());
    }
    if let Some(index) = app.session.documents().iter().position(|d| app.workspace.source_path(d).as_deref() == Some(file)) {
        app.run("window.activate", json!({"index": index}))?;
    } else {
        app.open_path(file);
    }
    if app.session.active().and_then(|d| app.workspace.source_path(d)).as_deref() != Some(file) {
        return Err("파일을 열지 못했습니다.".into());
    }
    app.run("sheet.activate", json!({"sheet": p.get("sheet")}))?;
    let result = app.run("selection.set", json!({"cell": p.get("cell")}))?;
    app.selected_chart = None;
    app.grid.reveal_moving_end = false;
    app.grid.ensure_visible = true;
    Ok(result)
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
        || !app.workspace.source_path(d).is_some_and(|path| app.patch_data.loaded.as_ref().is_some_and(|l| l.context.files.contains_key(&path)))
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

pub fn show(app: &mut SheetApp, ui: &mut egui::Ui) {
    if !app.patch_data.visible && !app.ui.patch_diagnostics_visible {
        return;
    }
    sync_draft(app);
    let t = theme::Tokens::get(ui.ctx());
    let stale = app.patch_data.loaded.as_ref().is_some_and(|l| l.stamp != stamp(app));
    let file = app.session.active().and_then(|d| app.workspace.source_path(d));
    let eligible = file.as_ref().is_some_and(|p| app.patch_data.loaded.as_ref().is_some_and(|l| l.context.files.contains_key(p)));
    let ignored = app.session.active().and_then(|d| d.wb.active()).is_some_and(|s| s.name.starts_with('_'));
    let mut action: Option<(&str, Json)> = None;
    let mut reset = false;
    if app.patch_data.visible {
        egui::Panel::right("ck_rules").default_size(340.0).size_range(280.0..=540.0).resizable(true)
        .frame(egui::Frame::NONE.fill(t.window).inner_margin(12)).show(ui, |ui| {
        if app.dialog.is_some() || app.message.is_some() { ui.disable(); }
        ui.horizontal(|ui| {
            ui.heading("CK 데이터");
            if ui.small_button("숨기기").clicked() { action = Some(("view.patchData", json!({"on": false}))); }
        });
        if !app.patch_data.enabled {
            ui.label("CK 패처용 폴더에서만 사용하는 편집 도구입니다.");
            if ui.button("이 폴더에 CK 규칙 적용").clicked() { action = Some(("patch.profile", json!({"mode": "ck"}))); }
            return;
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new("프로필 · CK").color(t.accent).strong());
            if ui.small_button("끄기").clicked() { action = Some(("patch.profile", json!({"mode": "off"}))); }
            if ui.add_enabled(!app.patch_data.busy(), egui::Button::new("다시 검사")).clicked() { action = Some(("patch.refresh", json!({}))); }
            if app.patch_data.busy() { ui.spinner(); }
        });
        if ui.button(if app.ui.patch_diagnostics_visible { "진단 닫기" } else { "진단 열기" }).clicked() { action = Some(("view.patchDiagnostics", json!({}))); }
        if app.patch_data.busy() && let Ok(p) = app.patch_data.progress.lock() {
            ui.small(format!("{} · {}/{} 파일 · {}행", p.stage, p.processed, p.discovered, p.rows));
        }
        if let Some(error) = &app.patch_data.error { ui.label(error); }
        if stale { ui.label(RichText::new("수정 내용이 있습니다. 다시 검사하세요.").color(t.accent)); }
        let version_before = app.patch_data.version.clone();
        ui.add_enabled_ui(!app.patch_data.busy(), |ui| { egui::ComboBox::from_id_salt("ck_version").selected_text(if version_before.is_empty() { "버전 필터 없음" } else { &version_before }).width(240.0).show_ui(ui, |ui| {
            ui.selectable_value(&mut app.patch_data.version, String::new(), "버전 필터 없음");
            if let Some(loaded) = &app.patch_data.loaded {
                if !loaded.context.build_name.is_empty() { ui.selectable_value(&mut app.patch_data.version, "@build".into(), format!("BuildName · {}", loaded.context.build_name)); }
                for name in loaded.catalog.versions.keys() { ui.selectable_value(&mut app.patch_data.version, name.clone(), name); }
            }
        });
        });
        if app.patch_data.version != version_before { action = Some(("patch.version", json!({"name": app.patch_data.version}))); }
        ui.small("protobuf 최상위 *Data.xlsx · _ / ~ 파일 제외");
        ui.separator();
        egui::ScrollArea::vertical().id_salt("ck_editor_scroll").show(ui, |ui| {
            if !eligible || ignored {
                ui.label(if ignored { "_로 시작하는 시트는 패처에서 제외됩니다." } else { "검사 대상 파일을 탐색기에서 선택하세요. 다른 파일은 CK 데이터로 처리하지 않습니다." });
            } else if let Some(draft) = &mut app.patch_data.draft {
                ui.label(RichText::new(format!("{} · {}", draft.cell.a1(), if draft.name.is_empty() { "이름 없는 필드" } else { &draft.name })).strong());
                ui.small(if draft.transposed { "전치 시트: A열 플래그 / B열 타입 / C열 필드명" } else { "1행 플래그 / 2행 타입 / 3행 필드명 / 4행부터 데이터" });
                ui.label("플래그");
                draft.dirty |= ui.text_edit_singleline(&mut draft.flags).changed();
                ui.collapsing("플래그 선택", |ui| {
                    for flag in schema::FLAGS {
                        let mut flags = schema::flags(&draft.flags);
                        let mut on = schema::has(&flags, flag);
                        if ui.checkbox(&mut on, *flag).changed() {
                            flags.retain(|f| !f.eq_ignore_ascii_case(flag));
                            if on { flags.push((*flag).into()); }
                            draft.flags = flags.join("; "); draft.dirty = true;
                        }
                    }
                    ui.small("참조: FKey.TableName · ZipWith.FieldName · Client.Name");
                });
                if draft.flags.trim().is_empty() { ui.label("플래그가 비어 있어 내보내기에서 제외되는 열입니다."); }
                for error in schema::flag_errors(&draft.flags) { ui.small(error); }
                ui.label("타입");
                draft.dirty |= ui.text_edit_singleline(&mut draft.typ).changed();
                egui::ComboBox::from_id_salt("ck_type_presets").selected_text("타입 예시 선택").show_ui(ui, |ui| {
                    for name in schema::TYPES.iter().copied().chain(["List<Int>", "List<String>", "List<IntList>", "Enum<TypeName>", "OptimizedTString<Table.[Id]>", "NullableTString<Table.[Id]>"]) {
                        if ui.selectable_label(false, name).clicked() { draft.typ = name.into(); draft.dirty = true; }
                    }
                });
                match Type::parse(&draft.typ) {
                    Ok(typ) => {
                        ui.small(typ.hint());
                        if typ.base == "TString" && !typ.list { ui.label("CK에서는 일반 TString을 사용할 수 없습니다."); }
                    }
                    Err(error) if !schema::has(&schema::flags(&draft.flags), "Design") => { ui.small(error); }
                    _ => {}
                }
                ui.label("필드명");
                draft.dirty |= ui.text_edit_singleline(&mut draft.name).changed();
                let params = json!({"uid": draft.uid, "revision": draft.revision, "sheet": draft.sheet, "cell": draft.cell.a1(), "kind": "schema", "flags": draft.flags, "type": draft.typ, "name": draft.name});
                if ui.button("헤더 3칸 적용").clicked() { action = Some(("patch.apply", params)); }
                if (if draft.transposed { draft.cell.col } else { draft.cell.row }) >= 3 {
                    ui.separator();
                    ui.label("셀 값");
                    draft.dirty |= ui.add(egui::TextEdit::multiline(&mut draft.value).desired_rows(4).desired_width(f32::INFINITY)).changed();
                    if let Ok(typ) = Type::parse(&draft.typ) {
                        if typ.base == "Bool" && !typ.list {
                            ui.horizontal(|ui| { for value in ["TRUE", "FALSE"] { if ui.button(value).clicked() { draft.value = value.into(); draft.dirty = true; } } });
                        }
                        if let Some(loaded) = &app.patch_data.loaded {
                            let flags = schema::flags(&draft.flags);
                            // Borrow the source; only enumerate matches when the popup is open.
                            let enums = if typ.base == "Enum" { typ.argument.as_ref().and_then(|a| loaded.catalog.enums.get(a)) } else { None };
                            let keys = if typ.base != "Enum" { schema::argument(&flags, "FKey").and_then(|table| loaded.catalog.keys.get(table)) } else { None };
                            let ids = (typ.base != "Enum" && schema::argument(&flags, "FKey").is_none() && schema::has(&flags, "StringIdReference")).then_some(&loaded.catalog.ids);
                            let count = enums.map(|v| v.len()).or_else(|| keys.or(ids).map(|v| v.len())).unwrap_or(0);
                            if count > 0 {
                                ui.small("후보 검색 (선택하면 입력란에 반영)");
                                ui.text_edit_singleline(&mut app.patch_data.search);
                                egui::ComboBox::from_id_salt("ck_value_candidates").selected_text(format!("{count}개 참조 후보")).show_ui(ui, |ui| {
                                    let query = app.patch_data.search.to_lowercase();
                                    let candidates = enums.into_iter().flat_map(|v| v.keys()).chain(keys.or(ids).into_iter().flat_map(|v| v.iter()));
                                    for candidate in candidates.filter(|s| query.is_empty() || s.to_lowercase().contains(&query)).take(100) {
                                        if ui.selectable_label(false, candidate.as_str()).clicked() {
                                            if typ.list && !draft.value.trim().is_empty() { draft.value.push('\n'); draft.value.push_str(candidate); } else { draft.value = candidate.clone(); }
                                            draft.dirty = true;
                                        }
                                    }
                                });
                            }
                            ui.collapsing("Alias 참고", |ui| {
                                for (name, value) in loaded.catalog.aliases.iter().take(50) { ui.add(egui::Label::new(format!("{name} → {value}")).truncate()); }
                            });
                        }
                    }
                    if ui.button("셀 값 적용").clicked() { action = Some(("patch.apply", json!({"uid": draft.uid, "revision": draft.revision, "sheet": draft.sheet, "cell": draft.cell.a1(), "value": draft.value}))); }
                    ui.small("텍스트로 저장하여 긴 정수·ID를 보존합니다. 수식은 수식 입력줄에서 편집하세요.");
                }
                if draft.dirty && ui.button("입력 취소 · 현재 선택 다시 읽기").clicked() { reset = true; }
            }
            ui.separator();
            ui.collapsing("검사 범위와 상태", |ui| {
                ui.label("로컬 작성 검사: 스키마, 기본 값, 키, 참조, 버전, Alias, ZipWith.");
                ui.label("날짜·기간 변환, ID 맵 번호 배정, protobuf 생성 및 게시 검증은 패처에서 수행하세요.");
                if let Some(l) = &app.patch_data.loaded {
                    ui.label(format!("{}개 파일 · {}행 포함 · {}행 버전 제외", l.context.files.len(), l.report.included_rows, l.report.excluded_rows));
                    if !l.context.complete { ui.label("일부 파일/참조를 읽지 못했습니다. 검사 완료로 간주하지 마세요."); }
                    for note in &l.context.notes { ui.label(note); }
                }
            });
        });
    });
    }
    if app.ui.patch_diagnostics_visible
        && let Some(l) = &app.patch_data.loaded
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
                ui.horizontal(|ui| {
                    if ui.small_button("진단 닫기").clicked() {
                        action = Some(("view.patchDiagnostics", json!({"on": false})));
                    }
                    ui.strong(format!("CK 검사 · {}건{}", l.report.total_issues, if stale { " · 이전 결과" } else { "" }));
                    if l.report.truncated {
                        ui.label("전체 행 검사 완료 · 처음 2,000건 표시");
                    }
                    let pages = l.report.issues.len().div_ceil(25).max(1);
                    app.patch_data.page = app.patch_data.page.min(pages.saturating_sub(1));
                    if ui.add_enabled(app.patch_data.page > 0, egui::Button::new("이전")).clicked() {
                        app.patch_data.page = app.patch_data.page.saturating_sub(1);
                    }
                    ui.label(format!("{} / {pages}", app.patch_data.page + 1));
                    if ui.add_enabled(app.patch_data.page + 1 < pages, egui::Button::new("다음")).clicked() {
                        app.patch_data.page += 1;
                    }
                });
                if !l.report.complete_context {
                    ui.label("일부 파일/참조가 누락된 검사입니다. 오른쪽 검사 범위를 확인하세요.");
                }
                if l.report.issues.is_empty() {
                    ui.label("현재 검사 범위에서 발견된 문제가 없습니다. 게시 전 패처 검증이 필요합니다.");
                }
                egui::ScrollArea::both().id_salt("ck_issues_scroll").show(ui, |ui| {
                    egui::Grid::new("ck_issues_table").striped(true).num_columns(4).min_col_width(80.0).show(ui, |ui| {
                        ui.strong("수준");
                        ui.strong("파일");
                        ui.strong("위치");
                        ui.strong("내용");
                        ui.end_row();
                        for issue in l.report.issues.iter().skip(app.patch_data.page.saturating_mul(25)).take(25) {
                            ui.label(if issue.severity == "error" { "오류" } else { "확인 필요" });
                            ui.label(std::path::Path::new(&issue.file).file_name().and_then(|s| s.to_str()).unwrap_or(&issue.file));
                            if ui.link(format!("{}!{}", issue.sheet, issue.cell)).clicked() {
                                action = Some(("patch.jump", json!({"file": issue.file, "sheet": issue.sheet, "cell": issue.cell})));
                            }
                            ui.label(&issue.message);
                            ui.end_row();
                        }
                    });
                });
            });
    }
    if reset {
        app.patch_data.draft = None;
    }
    if let Some((id, params)) = action {
        match app.run(id, params) {
            Ok(_) => {
                if id == "patch.apply" {
                    app.patch_data.draft = None;
                }
            }
            Err(error) => app.patch_data.error = Some(error),
        }
    }
}
