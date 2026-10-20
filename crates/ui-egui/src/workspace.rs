//! Desktop folder explorer. Directory reads are bounded and run off the UI thread; workbooks
//! are opened only on selection, through the existing file commands.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::mpsc::{Receiver, TryRecvError};

use egui::{RichText, Ui, vec2};
use serde::Serialize;
use serde_json::{Value as Json, json};

use crate::{SheetApp, icons::Icon, theme, widgets};

#[cfg(not(target_arch = "wasm32"))]
const MAX_DIRECTORY_ENTRIES: usize = 10_000;
const MAX_CACHED_ENTRIES: usize = 50_000;
const MAX_DEPTH: usize = 32;
const MAX_PENDING: usize = 4;

#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    pub path: String,
    pub name: String,
    pub directory: bool,
}

#[derive(Default)]
struct Directory {
    entries: Vec<Entry>,
    truncated: bool,
    error: Option<String>,
}

type ReadResult = Result<(String, Directory), String>;
type OpenResult = Result<gridcraft_engine::cmd::file::PreparedOpen, String>;
struct OpenRequest {
    interactive: bool,
    path: String,
    #[cfg(not(target_arch = "wasm32"))]
    params: Json,
    active_uid: Option<u64>,
    sheet: Option<String>,
}
struct Opening {
    path: String,
    receiver: Receiver<OpenResult>,
}

struct CloseBatch {
    remaining: VecDeque<u64>,
    waiting: Option<u64>,
    restore: Option<u64>,
}

struct Source {
    uid: u64,
    save_path: Option<String>,
    canonical: String,
}

#[derive(Default)]
pub struct Workspace {
    root: Option<String>,
    directories: BTreeMap<String, Directory>,
    expanded: HashSet<String>,
    pending_root: Option<Receiver<ReadResult>>,
    pending: HashMap<String, Receiver<ReadResult>>,
    error: Option<String>,
    /// Imported/encrypted files deliberately have no save path. Remember their source by uid
    /// so selecting them again keeps the same document, including unsaved changes.
    sources: HashMap<String, Source>,
    requested_file: Option<OpenRequest>,
    opening: Option<Opening>,
    sheets: BTreeMap<String, Result<Vec<String>, String>>,
    pending_sheets: HashMap<String, Receiver<Result<Vec<String>, String>>>,
    closing: Option<CloseBatch>,
}

impl Workspace {
    pub fn root(&self) -> Option<&str> {
        self.root.as_deref()
    }

    pub fn busy(&self) -> bool {
        self.pending_root.is_some()
            || !self.pending.is_empty()
            || !self.pending_sheets.is_empty()
            || self.requested_file.is_some()
            || self.opening.is_some()
    }

    pub fn inspect(&self) -> Json {
        json!({
            "root": self.root, "loading": self.busy(), "error": self.error,
            "closingTabs": self.closing.as_ref().map(|b| json!({"remaining": b.remaining, "waiting": b.waiting})),
            "openingFile": self.requested_file.as_ref().map(|r| &r.path),
            "files": self.sheets.iter().map(|(path, sheets)| json!({
                "path": path, "expanded": self.expanded.contains(path),
                "sheets": sheets.as_ref().ok(), "error": sheets.as_ref().err(),
            })).collect::<Vec<_>>(),
            "directories": self.directories.iter().map(|(path, dir)| json!({
                "path": path, "expanded": self.expanded.contains(path),
                "entries": dir.entries, "truncated": dir.truncated, "error": dir.error,
            })).collect::<Vec<_>>(),
        })
    }

    fn open(&mut self, path: &str) -> Result<Json, String> {
        if path.trim().is_empty() {
            return Err("workspace.open requires a folder path".into());
        }
        self.pending_root = Some(read_async(path)?);
        self.requested_file = None;
        self.error = None;
        Ok(json!({"loading": true}))
    }

    fn contains(&self, path: &str, directory: bool) -> bool {
        self.directories.values().any(|d| d.entries.iter().any(|e| e.path == path && e.directory == directory))
    }

    fn toggle_file(&mut self, path: &str) -> Result<Json, String> {
        if !self.contains(path, false) {
            return Err("file is not in the explorer".into());
        }
        if self.expanded.remove(path) {
            return Ok(json!({"expanded": false}));
        }
        if !self.sheets.contains_key(path) && !self.pending_sheets.contains_key(path) {
            if self.pending_sheets.len() >= MAX_PENDING {
                return Err("Please wait for the current sheets to finish loading.".into());
            }
            self.pending_sheets.insert(path.into(), sheets_async(path)?);
        }
        self.expanded.insert(path.into());
        Ok(json!({"expanded": true}))
    }

    /// Open documents are authoritative, including unsaved sheet renames and undo.
    fn sync_sheets(&mut self, session: &gridcraft_engine::Session) {
        let paths: Vec<_> = self
            .directories
            .values()
            .flat_map(|d| &d.entries)
            .filter(|e| !e.directory)
            .filter_map(|e| session.documents().iter().find(|d| self.matches(&e.path, d)).map(|d| (e.path.clone(), d)))
            .collect();
        for (path, doc) in paths {
            let same = self
                .sheets
                .get(&path)
                .and_then(|r| r.as_ref().ok())
                .is_some_and(|names| names.iter().map(String::as_str).eq(doc.wb.sheets.iter().map(|s| s.name.as_str())));
            if !same {
                self.sheets.insert(path.clone(), Ok(doc.wb.sheets.iter().map(|s| s.name.clone()).collect()));
            }
            self.pending_sheets.remove(&path);
        }
    }

    fn toggle(&mut self, path: &str) -> Result<Json, String> {
        if !self.contains(path, true) {
            return Err("folder is not in the explorer".into());
        }
        if self.expanded.remove(path) {
            return Ok(json!({"expanded": false}));
        }
        let depth = self.root.as_ref().and_then(|root| std::path::Path::new(path).strip_prefix(root).ok()).map(|p| p.components().count());
        if depth.is_none_or(|n| n >= MAX_DEPTH) {
            return Err(format!("Folder nesting is limited to {MAX_DEPTH} levels. Open this folder as a workspace instead."));
        }
        if !self.directories.contains_key(path) && !self.pending.contains_key(path) {
            if self.pending.len() >= MAX_PENDING {
                return Err("Please wait for the current folders to finish loading.".into());
            }
            self.pending.insert(path.to_string(), read_async(path)?);
        }
        self.expanded.insert(path.to_string());
        Ok(json!({"expanded": true}))
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        if let Some(result) = self.pending_root.as_ref().and_then(receive) {
            self.pending_root = None;
            match result {
                Ok((path, dir)) => {
                    self.root = Some(path.clone());
                    self.directories.clear();
                    self.pending.clear();
                    self.expanded.clear();
                    self.sheets.clear();
                    self.pending_sheets.clear();
                    self.expanded.insert(path.clone());
                    self.directories.insert(path, dir);
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
        let finished: Vec<_> = self.pending.iter().filter_map(|(path, rx)| receive(rx).map(|r| (path.clone(), r))).collect();
        for (path, result) in finished {
            self.pending.remove(&path);
            let dir = match result {
                Ok((_, dir)) => dir,
                Err(error) => Directory { error: Some(error), ..Default::default() },
            };
            let count = self.directories.values().map(|d| d.entries.len()).sum::<usize>();
            if count.saturating_add(dir.entries.len()) > MAX_CACHED_ENTRIES {
                self.expanded.remove(&path);
                self.error = Some("Explorer limit reached. Open a subfolder as a workspace, or refresh to clear the list.".into());
            } else {
                self.directories.insert(path, dir);
            }
        }
        let finished: Vec<_> = self.pending_sheets.iter().filter_map(|(path, rx)| receive(rx).map(|r| (path.clone(), r))).collect();
        for (path, result) in finished {
            self.pending_sheets.remove(&path);
            self.sheets.insert(path, result);
        }
        if self.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    pub(crate) fn remember(&mut self, path: &str, session: &gridcraft_engine::Session) {
        self.sources.retain(|_, source| session.documents().iter().any(|d| d.uid == source.uid && d.path == source.save_path));
        if let Some(doc) = session.active() {
            #[cfg(not(target_arch = "wasm32"))]
            let canonical = std::fs::canonicalize(path).ok().and_then(|p| p.to_str().map(str::to_string)).unwrap_or_else(|| path.into());
            #[cfg(target_arch = "wasm32")]
            let canonical = path.to_string();
            self.sources.insert(path.to_string(), Source { uid: doc.uid, save_path: doc.path.clone(), canonical });
        }
    }

    pub(crate) fn source_path(&self, doc: &gridcraft_engine::DocState) -> Option<String> {
        self.sources
            .values()
            .find(|source| source.uid == doc.uid && source.save_path == doc.path)
            .map(|source| source.canonical.clone())
            .or_else(|| doc.path.clone())
    }

    fn matches(&self, path: &str, doc: &gridcraft_engine::DocState) -> bool {
        doc.path.as_deref() == Some(path) || self.sources.get(path).is_some_and(|source| source.uid == doc.uid && source.save_path == doc.path)
    }

    fn rows(&self) -> Vec<Row<'_>> {
        let mut rows = Vec::new();
        if let Some(root) = &self.root {
            self.append_rows(root, 0, &mut rows);
        }
        rows
    }

    fn append_rows<'a>(&'a self, path: &str, depth: usize, rows: &mut Vec<Row<'a>>) {
        if depth >= MAX_DEPTH {
            return;
        }
        let Some(dir) = self.directories.get(path) else {
            rows.push(Row::Message("Loading…", depth));
            return;
        };
        if let Some(error) = &dir.error {
            rows.push(Row::Message(error, depth));
        } else if dir.entries.is_empty() {
            rows.push(Row::Message("No spreadsheets in this folder", depth));
        }
        for entry in &dir.entries {
            rows.push(Row::Entry(entry, depth));
            if self.expanded.contains(&entry.path) {
                if entry.directory {
                    self.append_rows(&entry.path, depth + 1, rows);
                } else {
                    match self.sheets.get(&entry.path) {
                        Some(Ok(names)) if names.is_empty() => rows.push(Row::Message("No worksheets", depth + 1)),
                        Some(Ok(names)) => {
                            for name in names {
                                rows.push(Row::Sheet(&entry.path, name, depth + 1));
                            }
                        }
                        Some(Err(error)) => rows.push(Row::Message(error, depth + 1)),
                        None => rows.push(Row::Message("Loading…", depth + 1)),
                    }
                }
            }
        }
        if dir.truncated {
            rows.push(Row::Message("Folder limit reached; open a smaller folder", depth));
        }
    }
}

/// One decoder at a time. Rapid selections coalesce to the latest file, and the current
/// document remains editable while parsing and recalculation run off the UI thread.
pub fn poll_open(app: &mut SheetApp, ctx: &egui::Context) {
    if let Err(error) = advance_close(app) {
        app.alert(error);
    }
    app.workspace.sync_sheets(&app.session);
    if app.workspace.requested_file.as_ref().is_some_and(|r| r.active_uid != app.session.active().map(|d| d.uid)) {
        app.workspace.requested_file = None;
    }
    // Never steal an unfinished cell edit when the result arrives.
    let result = if app.editor.is_none() && app.text_box_editor.is_none() && app.dialog.is_none() && app.message.is_none() {
        app.workspace.opening.as_ref().and_then(|job| match job.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("Workbook loading was interrupted.".into())),
        })
    } else {
        None
    };
    if let Some(result) = result {
        let path = app.workspace.opening.take().map(|job| job.path).unwrap_or_default();
        if app.workspace.requested_file.as_ref().is_some_and(|r| r.path == path) {
            let request = app.workspace.requested_file.take();
            let interactive = request.as_ref().is_some_and(|r| r.interactive);
            let sheet = request.and_then(|r| r.sheet);
            match result {
                Ok(prepared) => match gridcraft_engine::cmd::file::finish_open(&mut app.session, prepared) {
                    Ok(result) => {
                        app.after_open(Some(&path), &result);
                        app.selected_chart = None;
                        app.after_engine();
                        if let Some(sheet) = sheet {
                            app.run_or_alert("sheet.activate", json!({"sheet": sheet}));
                        }
                    }
                    Err(e) => app.alert(e.to_string()),
                },
                Err(error) if interactive && error.contains(gridcraft_engine::io::PASSWORD_REQUIRED) => {
                    app.open_dialog("openPassword", json!({"path":path}))
                }
                Err(error) => {
                    app.workspace.error = Some(error.clone());
                    if interactive {
                        app.alert(error);
                    }
                }
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    if app.workspace.opening.is_none()
        && let Some(request) = &app.workspace.requested_file
    {
        let path = request.path.clone();
        let params = request.params.clone();
        let (tx, receiver) = std::sync::mpsc::channel();
        match std::thread::Builder::new().name("gridcraft-open".into()).spawn(move || {
            let result = gridcraft_engine::cmd::file::prepare_open(&params).map_err(|e| e.to_string());
            let _ = tx.send(result);
        }) {
            Ok(_) => app.workspace.opening = Some(Opening { path, receiver }),
            Err(error) => {
                app.workspace.requested_file = None;
                app.alert(error.to_string());
            }
        }
    }
    if app.workspace.busy() {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

fn receive<T>(rx: &Receiver<Result<T, String>>) -> Option<Result<T, String>> {
    match rx.try_recv() {
        Ok(result) => Some(result),
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => Some(Err("Folder reader stopped. Try refreshing the explorer.".into())),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read_async(path: &str) -> Result<Receiver<ReadResult>, String> {
    let path = path.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("gridcraft-explorer".into())
        .spawn(move || {
            let _ = tx.send(read_directory(&path));
        })
        .map_err(|e| format!("Could not start folder reader: {e}"))?;
    Ok(rx)
}

#[cfg(target_arch = "wasm32")]
fn read_async(_: &str) -> Result<Receiver<ReadResult>, String> {
    Err("Folder workspaces are available in the desktop app.".into())
}

#[cfg(not(target_arch = "wasm32"))]
fn read_directory(path: &str) -> ReadResult {
    let root = std::fs::canonicalize(path).map_err(|e| format!("Cannot open folder {path}: {e}"))?;
    let root_name = root.to_str().ok_or("Folder path is not valid Unicode")?.to_string();
    let reader = std::fs::read_dir(&root).map_err(|e| format!("Cannot read folder {path}: {e}"))?;
    let mut dir = Directory::default();
    for (i, result) in reader.enumerate() {
        if i >= MAX_DIRECTORY_ENTRIES {
            dir.truncated = true;
            break;
        }
        let item = result.map_err(|e| format!("Cannot read folder {path}: {e}"))?;
        let kind = item.file_type().map_err(|e| format!("Cannot read file in {path}: {e}"))?;
        // Omit symlink entries rather than expanding cycles in the folder tree.
        if kind.is_symlink() || (!kind.is_dir() && !kind.is_file()) {
            continue;
        }
        let Some(name) = item.file_name().to_str().map(str::to_string) else { continue };
        if name.starts_with('.') || name.starts_with("~$") {
            continue;
        }
        let Some(path) = item.path().to_str().map(str::to_string) else { continue };
        if kind.is_file() && !is_spreadsheet(&name) {
            continue;
        }
        dir.entries.push(Entry { path, name, directory: kind.is_dir() });
    }
    dir.entries.sort_by_cached_key(|e| (!e.directory, e.name.to_lowercase(), e.name.clone()));
    Ok((root_name, dir))
}

#[cfg(not(target_arch = "wasm32"))]
fn is_spreadsheet(name: &str) -> bool {
    std::path::Path::new(name).extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(e.to_ascii_lowercase().as_str(), "xlsx" | "xlsm" | "xltx" | "xltm" | "xls" | "xlsb" | "ods" | "csv" | "tsv" | "txt" | "json")
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn sheets_async(path: &str) -> Result<Receiver<Result<Vec<String>, String>>, String> {
    let path = path.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("gridcraft-sheet-list".into())
        .spawn(move || {
            let _ = tx.send(gridcraft_engine::io::sheet_names(&path).map_err(|e| e.to_string()));
        })
        .map_err(|e| e.to_string())?;
    Ok(rx)
}

#[cfg(target_arch = "wasm32")]
fn sheets_async(_: &str) -> Result<Receiver<Result<Vec<String>, String>>, String> {
    Err("Folder workspaces are available in the desktop app.".into())
}

enum Row<'a> {
    Sheet(&'a str, &'a str, usize),
    Entry(&'a Entry, usize),
    Message(&'a str, usize),
}

/// Commands require explicit paths; only `open_dialog("openFolder", …)` invokes the picker.
pub fn command(app: &mut SheetApp, id: &str, p: &Json) -> Option<Result<Json, String>> {
    let path = || p.get("path").and_then(Json::as_str).ok_or_else(|| "missing `path`".to_string());
    Some(match id {
        "workspace.open" => path().and_then(|path| {
            let result = app.workspace.open(path)?;
            app.ui.explorer_visible = true;
            app.backstage = None;
            Ok(result)
        }),
        "workspace.refresh" => match app.workspace.root.clone() {
            Some(path) => app.workspace.open(&path),
            None => Err("No folder is open.".into()),
        },
        "workspace.close" => {
            // Open documents and their edits stay available in the document tabs.
            let sources = std::mem::take(&mut app.workspace.sources);
            app.workspace = Workspace { sources, ..Default::default() };
            Ok(Json::Null)
        }
        "workspace.closeTabs" => close_tabs(app, p),
        "workspace.toggleFolder" => path().and_then(|path| app.workspace.toggle(path)),
        "workspace.toggleFile" => path().and_then(|path| {
            app.workspace.sync_sheets(&app.session);
            app.workspace.toggle_file(path)
        }),
        "workspace.openSheet" => path().and_then(|path| {
            let sheet = p.get("sheet").and_then(Json::as_str).ok_or("missing `sheet`")?;
            let result = app.run("workspace.openFile", p.clone())?;
            if let Some(request) = &mut app.workspace.requested_file {
                request.sheet = Some(sheet.into());
            } else {
                app.run("sheet.activate", json!({"sheet": sheet}))?;
            }
            app.workspace.expanded.insert(path.into());
            Ok(result)
        }),
        "workspace.openFile" => path().and_then(|path| {
            if !app.workspace.contains(path, false) {
                return Err("file is not in the explorer".into());
            }
            if !app.commit_edit(0, 0, false, false) {
                return Err("Finish the current cell edit before switching files.".into());
            }
            app.commit_text_box_edit()?;
            let index = app.session.documents().iter().position(|d| app.workspace.matches(path, d)).or_else(|| existing_path(app, path));
            if let Some(index) = index {
                app.workspace.requested_file = None;
                app.run("window.activate", json!({"index": index}))?;
                app.workspace.remember(path, &app.session);
                app.backstage = None;
                app.selected_chart = None;
                app.notice = None;
                Ok(json!({"index": index, "alreadyOpen": true}))
            } else {
                if !p.get("background").and_then(Json::as_bool).unwrap_or(false) {
                    let result = app.run("file.open", json!({"path": path, "password": p.get("password")}))?;
                    app.after_open(Some(path), &result);
                    app.selected_chart = None;
                    return Ok(result);
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    app.workspace.requested_file = Some(OpenRequest {
                        interactive: p.get("interactive").and_then(Json::as_bool).unwrap_or(false),
                        path: path.into(),
                        params: json!({"path":path,"password":p.get("password")}),
                        active_uid: app.session.active().map(|d| d.uid),
                        sheet: None,
                    });
                    Ok(json!({"loading": true, "path": path}))
                }
                #[cfg(target_arch = "wasm32")]
                {
                    Err("Local folder opening is available on desktop.".into())
                }
            }
        }),
        "workspace.inspect" => Ok(app.workspace.inspect()),
        "view.explorer" => {
            app.ui.explorer_visible = p.get("on").and_then(Json::as_bool).unwrap_or(!app.ui.explorer_visible);
            Ok(json!({"on": app.ui.explorer_visible}))
        }
        _ => return None,
    })
}

pub fn available(app: &SheetApp) -> bool {
    app.services.pick_folder.is_some() || app.workspace.root.is_some() || app.workspace.busy()
}

/// Resolve aliases once when opening, never during painting. For example, macOS /tmp and
/// /private/tmp name the same file; a file already open via either spelling must keep its edits.
#[cfg(not(target_arch = "wasm32"))]
fn existing_path(app: &SheetApp, path: &str) -> Option<usize> {
    let requested = std::path::Path::new(path);
    app.session.documents().iter().position(|d| d.path.as_deref().and_then(|p| std::fs::canonicalize(p).ok()).is_some_and(|p| p == requested))
}

#[cfg(target_arch = "wasm32")]
fn existing_path(_: &SheetApp, _: &str) -> Option<usize> {
    None
}

pub fn show(app: &mut SheetApp, ui: &mut Ui) {
    if !available(app) || !app.ui.explorer_visible {
        return;
    }
    let t = theme::Tokens::get(ui.ctx());
    let lang = app.ui.language;
    let mut action: Option<(&str, Json)> = None;
    let mut pick = false;
    egui::Panel::left("workspace_explorer")
        .default_size(240.0)
        .size_range(180.0..=420.0)
        .resizable(true)
        .frame(egui::Frame::NONE.fill(t.window).inner_margin(10))
        .show(ui, |ui| {
            if app.dialog.is_some() || app.message.is_some() {
                ui.disable();
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new(lang.tr("Explorer")).strong().color(t.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::icon_button(ui, Icon::Close, t.text_dim, lang.tr("Hide Explorer"), vec2(22.0, 24.0)).clicked() {
                        action = Some(("view.explorer", json!({"on": false})));
                    }
                    if app.workspace.root.is_some()
                        && widgets::icon_button(ui, Icon::OpenFolder, t.text_dim, lang.tr("Open Folder…"), vec2(22.0, 24.0)).clicked()
                    {
                        pick = true;
                    }
                });
            });
            ui.separator();
            if app.workspace.root.is_some() {
                ui.horizontal(|ui| {
                    let root = app.workspace.root.as_deref().unwrap_or_default();
                    let name = std::path::Path::new(root).file_name().and_then(|n| n.to_str()).unwrap_or(root);
                    ui.add(egui::Label::new(RichText::new(name).strong()).truncate()).on_hover_text(root);
                });
                ui.horizontal(|ui| {
                    if ui.small_button(lang.tr("Refresh")).clicked() {
                        action = Some(("workspace.refresh", json!({})));
                    }
                    if ui.small_button(lang.tr("Close Folder")).clicked() {
                        action = Some(("workspace.close", json!({})));
                    }
                    if ui.small_button("CK 규칙").clicked() {
                        action = Some(("view.patchData", json!({"on": true})));
                    }
                    if app.workspace.busy() {
                        ui.spinner();
                    }
                });
                if let Some(request) = &app.workspace.requested_file {
                    let name = std::path::Path::new(&request.path).file_name().and_then(|p| p.to_str()).unwrap_or(&request.path);
                    ui.add(egui::Label::new(format!("Loading {name}…")).truncate());
                }
                ui.add_space(8.0);
            } else {
                ui.add_space(18.0);
                ui.label(lang.tr("Open a folder to browse your spreadsheets."));
                ui.add_space(8.0);
                if ui.button(lang.tr("Open Folder…")).clicked() {
                    pick = true;
                }
                if app.workspace.busy() {
                    ui.spinner();
                }
            }
            if let Some(error) = &app.workspace.error {
                ui.colored_label(t.danger, error);
            }
            let active = app.session.active();
            let rows = app.workspace.rows();
            egui::ScrollArea::vertical().id_salt("explorer_files").auto_shrink([false, false]).show_rows(ui, 26.0, rows.len(), |ui, visible| {
                for row in rows.get(visible).unwrap_or_default() {
                    match row {
                        Row::Entry(entry, depth) => {
                            let selected = active.is_some_and(|d| app.workspace.matches(&entry.path, d));
                            let dirty = app.session.documents().iter().any(|d| d.is_dirty() && app.workspace.matches(&entry.path, d));
                            ui.horizontal(|ui| {
                                ui.add_space(*depth as f32 * 14.0);
                                let expanded = app.workspace.expanded.contains(&entry.path);
                                let disclosure = ui.add_sized([18.0, 26.0], egui::Button::new(if expanded { "▾" } else { "▸" }).frame(false));
                                disclosure.widget_info(|| {
                                    egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), format!("Toggle: {}", entry.name))
                                });
                                if disclosure.clicked() {
                                    action = Some((
                                        if entry.directory { "workspace.toggleFolder" } else { "workspace.toggleFile" },
                                        json!({"path": entry.path}),
                                    ));
                                }
                                let label = format!("{}{}", entry.name, if dirty { " •" } else { "" });
                                let response =
                                    ui.add_sized([ui.available_width(), 26.0], egui::Button::selectable(selected, "").left_text(label).truncate());
                                response.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::WidgetType::Button,
                                        ui.is_enabled(),
                                        selected,
                                        format!("{}: {}", if entry.directory { "Folder" } else { "File" }, entry.name),
                                    )
                                });
                                if response.on_hover_text(&entry.path).clicked() {
                                    action = Some((
                                        if entry.directory { "workspace.toggleFolder" } else { "workspace.openFile" },
                                        json!({"path": entry.path, "background": true, "interactive": true}),
                                    ));
                                }
                            });
                        }
                        Row::Sheet(path, name, depth) => {
                            let selected = active.is_some_and(|d| app.workspace.matches(path, d) && d.wb.active().is_some_and(|s| s.name == *name));
                            ui.horizontal(|ui| {
                                ui.add_space(*depth as f32 * 14.0 + 18.0);
                                let response =
                                    ui.add_sized([ui.available_width(), 26.0], egui::Button::selectable(selected, "").left_text(*name).truncate());
                                response.widget_info(|| {
                                    egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), selected, format!("Sheet: {name}"))
                                });
                                if response.on_hover_text(*name).clicked() {
                                    action =
                                        Some(("workspace.openSheet", json!({"path": path, "sheet": name, "background": true, "interactive": true})));
                                }
                            });
                        }
                        Row::Message(message, depth) => {
                            ui.horizontal(|ui| {
                                ui.add_space(*depth as f32 * 14.0);
                                ui.add(egui::Label::new(RichText::new(lang.tr(message)).small().color(t.text_dim)).truncate())
                                    .on_hover_text(*message);
                            });
                        }
                    }
                }
            });
        });
    if pick {
        app.open_dialog("openFolder", json!({}));
    }
    if let Some((id, params)) = action
        && let Err(error) = app.run(id, params.clone())
    {
        if id == "workspace.openFile" && error.contains(gridcraft_engine::io::PASSWORD_REQUIRED) {
            app.open_dialog("openPassword", params);
        } else {
            app.alert(error);
        }
    }
}

/// Queue stable document identities, never mutable tab indices. Each dirty workbook gets
/// its normal save prompt; cancel, failed save, or a cancelled picker stops the remaining batch.
fn close_tabs(app: &mut SheetApp, p: &Json) -> Result<Json, String> {
    if app.dialog.is_some() || app.message.is_some() || app.workspace.closing.is_some() {
        return Err("Finish the current dialog before closing tabs.".into());
    }
    let uid = p.get("uid").and_then(Json::as_u64).ok_or("missing `uid`")?;
    let anchor = app.session.documents().iter().position(|d| d.uid == uid).ok_or("This tab is no longer open.")?;
    let scope = p.get("scope").and_then(Json::as_str).unwrap_or("one");
    if !["one", "others", "right", "all"].contains(&scope) {
        return Err("scope: one | others | right | all".into());
    }
    if !app.commit_edit(0, 0, false, false) {
        return Err("Finish the current cell edit before closing tabs.".into());
    }
    app.commit_text_box_edit()?;
    let targets: Vec<_> = app
        .session
        .documents()
        .iter()
        .enumerate()
        .filter(|(i, _)| match scope {
            "one" => *i == anchor,
            "others" => *i != anchor,
            "right" => *i > anchor,
            _ => true,
        })
        .map(|(_, d)| d)
        .collect();
    if !p.get("interactive").and_then(Json::as_bool).unwrap_or(false) && targets.iter().any(|d| d.is_dirty()) {
        return Err("Some tabs have unsaved changes. Save them before closing, or use the tab menu.".into());
    }
    let count = targets.len();
    let restore = if scope == "others" { Some(uid) } else { app.session.active().map(|d| d.uid) };
    app.workspace.closing = Some(CloseBatch { remaining: targets.into_iter().map(|d| d.uid).collect(), waiting: None, restore });
    app.workspace.requested_file = None;
    advance_close(app)?;
    Ok(json!({"requested": count, "pending": app.workspace.closing.is_some()}))
}

fn advance_close(app: &mut SheetApp) -> Result<(), String> {
    let Some(mut batch) = app.workspace.closing.take() else { return Ok(()) };
    if app.dialog.is_some() || app.message.is_some() {
        app.workspace.closing = Some(batch);
        return Ok(());
    }
    if let Some(uid) = batch.waiting.take()
        && app.session.documents().iter().any(|d| d.uid == uid)
    {
        return Ok(()); // Prompt dismissed without closing its document: stop here.
    }
    if app.editor.is_some() || app.text_box_editor.is_some() {
        return Ok(());
    }
    while let Some(uid) = batch.remaining.pop_front() {
        let Some(index) = app.session.documents().iter().position(|d| d.uid == uid) else { continue };
        app.run("window.activate", json!({"index": index}))?;
        app.selected_chart = None;
        let result = app.run("file.close", json!({}))?;
        if result.get("closed").and_then(Json::as_bool) != Some(true) {
            batch.waiting = Some(uid);
            app.workspace.closing = Some(batch);
            return Ok(());
        }
    }
    if let Some(index) = batch.restore.and_then(|uid| app.session.documents().iter().position(|d| d.uid == uid)) {
        app.run("window.activate", json!({"index": index}))?;
    }
    Ok(())
}

/// Documents remain open when changing folders. Tabs make unsaved work reachable at all times.
pub fn document_tabs(app: &mut SheetApp, ui: &mut Ui) {
    if !available(app) {
        return;
    }
    let t = theme::Tokens::get(ui.ctx());
    let lang = app.ui.language;
    let mut activate = None;
    let mut close = None;
    egui::Panel::top("workspace_documents").exact_size(34.0).frame(egui::Frame::NONE.fill(t.tab_bar)).show(ui, |ui| {
        if app.dialog.is_some() || app.message.is_some() {
            ui.disable();
        }
        egui::ScrollArea::horizontal().id_salt("document_tabs_scroll").show(ui, |ui| {
            ui.horizontal(|ui| {
                if !app.ui.explorer_visible && ui.button(lang.tr("Explorer")).clicked() {
                    app.run_or_alert("view.explorer", json!({"on": true}));
                }
                let count = app.session.documents().len();
                for (index, doc) in app.session.documents().iter().enumerate() {
                    ui.push_id(doc.uid, |ui| {
                        let title = format!("{}{}", doc.display_title(), if doc.is_dirty() { " •" } else { "" });
                        let tab =
                            ui.selectable_label(app.session.active_index() == index, title).on_hover_text(doc.path.as_deref().unwrap_or(&doc.title));
                        if tab.clicked() {
                            activate = Some(index);
                        }
                        if tab.clicked_by(egui::PointerButton::Middle) {
                            close = Some((doc.uid, "one"));
                        }
                        tab.context_menu(|ui| {
                            for (scope, label, enabled) in [
                                ("one", "Close Tab", true),
                                ("others", "Close Other Tabs", count > 1),
                                ("right", "Close Tabs to the Right", index + 1 < count),
                                ("all", "Close All Tabs", true),
                            ] {
                                if ui.add_enabled(enabled, egui::Button::new(lang.tr(label))).clicked() {
                                    close = Some((doc.uid, scope));
                                    ui.close();
                                }
                            }
                        });
                        let response = widgets::icon_button(ui, Icon::Close, t.text_dim, lang.tr("Close Tab"), vec2(18.0, 24.0));
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), format!("Close {}", doc.display_title()))
                        });
                        if response.clicked() {
                            close = Some((doc.uid, "one"));
                        }
                        ui.separator();
                    });
                }
            });
        });
    });
    if let Some((uid, scope)) = close {
        app.run_or_alert("workspace.closeTabs", json!({"uid": uid, "scope": scope, "interactive": true}));
    } else if let Some(index) = activate
        && app.commit_edit(0, 0, false, false)
    {
        match app.run("window.activate", json!({"index": index})) {
            Ok(_) => {
                app.selected_chart = None;
            }
            Err(error) => app.alert(error),
        }
    }
}
