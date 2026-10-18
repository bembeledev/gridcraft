//! Data › Queries & Connections › Workbook Links (Excel 365's pane) and File › Info › Edit Links
//! to Files (the classic dialog): the workbooks this one links to (`[1]Prices!B2`), their status,
//! and what to do with them: update their values from the source file, change the source, open
//! it, break the link, and the workbook's startup prompt.
//!
//! Reading a source workbook needs a file system, which the host provides ([`LinkFiles`] in
//! [`Session::link_files`]): the desktop app, the CLI and MCP read files; the browser has none,
//! so there the commands that read sources are disabled and links keep the values saved with
//! them. Opening a workbook never reads its links' sources: like Excel, it shows the values saved
//! with the links and disables their automatic update until the user enables it (Enable Content
//! on the security warning) or updates them.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gridcraft_model::links::{MAX_LINK_CELLS, book_from_source};
use gridcraft_model::{ExternalBook, UpdateLinks, Workbook};
use serde::Serialize;
use serde_json::{Value as Json, json};

use super::{CommandSpec, bad, cmd, edit, has_doc, str_param};
use crate::{DocState, EngineError, Result, Session};

/// Reads a linked workbook's file.
pub type ReadSource = Arc<dyn Fn(&Path) -> std::result::Result<Vec<u8>, String> + Send + Sync>;

/// How the host reads linked workbooks' files: the file system where there is one, nothing in the
/// browser. A session starts with the file system ([`LinkFiles::native`]); hosts and tests can
/// replace it.
#[derive(Clone)]
pub struct LinkFiles(pub Option<ReadSource>);

impl LinkFiles {
    /// The file system (`None` on wasm, which has none).
    pub fn native() -> LinkFiles {
        #[cfg(not(target_arch = "wasm32"))]
        {
            /// Files larger than this aren't read as a link's source.
            const MAX_SOURCE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
            LinkFiles(Some(Arc::new(|path: &Path| {
                let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
                if meta.len() > MAX_SOURCE_BYTES {
                    return Err("files larger than 2 GB aren't supported".into());
                }
                std::fs::read(path).map_err(|e| e.to_string())
            })))
        }
        #[cfg(target_arch = "wasm32")]
        {
            LinkFiles(None)
        }
    }
    /// No file access: links keep the values saved with them.
    pub fn none() -> LinkFiles {
        LinkFiles(None)
    }
    pub fn available(&self) -> bool {
        self.0.is_some()
    }
}

impl Default for LinkFiles {
    fn default() -> Self {
        LinkFiles::native()
    }
}

impl std::fmt::Debug for LinkFiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.available() { "LinkFiles(available)" } else { "LinkFiles(none)" })
    }
}

/// A link's status, as Edit Links › Status and the Workbook Links pane show it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkStatus {
    /// Not checked yet in this session.
    #[default]
    Unknown,
    /// The values are those of the source file.
    Ok,
    /// The source file isn't there (or can't be read).
    SourceNotFound,
    /// The source has no sheet a reference names.
    WorksheetNotFound,
    /// The source file has other values than the link.
    ValuesNotUpdated,
    /// The source can't be read as a workbook.
    Undetermined,
    /// The link names no file (a DDE or OLE link kept from the file).
    NotApplicable,
}

impl LinkStatus {
    pub fn label(self) -> &'static str {
        match self {
            LinkStatus::Unknown => "Unknown",
            LinkStatus::Ok => "OK",
            LinkStatus::SourceNotFound => "Error: Source not found",
            LinkStatus::WorksheetNotFound => "Error: Worksheet not found",
            LinkStatus::ValuesNotUpdated => "Warning: Values not updated",
            LinkStatus::Undetermined => "Error: Undetermined",
            LinkStatus::NotApplicable => "N/A",
        }
    }
    pub fn is_error(self) -> bool {
        matches!(self, LinkStatus::SourceNotFound | LinkStatus::WorksheetNotFound | LinkStatus::Undetermined)
    }
}

/// What this session knows about one linked file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LinkFileState {
    pub status: LinkStatus,
    /// When values were last read from it (Unix time, ms).
    pub refreshed: Option<f64>,
    /// Why the status is an error (`os error 2`, the missing sheets).
    pub detail: Option<String>,
}

/// A document's links in this session (not saved: Excel checks again on every open).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LinkSession {
    /// Automatic update of links is disabled (the security warning shows): set when a workbook
    /// with links is opened, cleared by Enable Content or an update the user asks for.
    pub updates_disabled: bool,
    /// By the link's path as stored, lower-case.
    pub files: HashMap<String, LinkFileState>,
}

/// One link, as the pane and the dialog list it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkRow {
    /// `[n]` in formulas.
    pub index: usize,
    /// The file's name (`Source.xlsx`).
    pub name: String,
    /// The path as the link stores it (absolute, or relative to this workbook's folder).
    pub path: String,
    /// Where GridCraft looks for the file.
    pub location: String,
    /// `Worksheet` for a link to a workbook file; `Other` for a link without one (DDE, OLE).
    #[serde(rename = "type")]
    pub kind: &'static str,
    /// Edit Links › Update: links to workbooks update automatically (`A`).
    pub update: &'static str,
    pub status: LinkStatus,
    pub status_text: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// When values were last read from the source in this session (Unix time, ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_refreshed: Option<f64>,
    /// The linked workbook's sheets the link knows.
    pub sheets: Vec<String>,
}

fn key(path: &str) -> String {
    path.replace('/', "\\").to_lowercase()
}

/// The folder relative link paths start from: the one the workbook was read from, else the one
/// it was saved to.
pub fn base_dir(d: &DocState) -> Option<PathBuf> {
    d.wb.source_dir.clone().or_else(|| d.path.as_deref().and_then(crate::io::source_dir))
}

/// Where a link's file is: an absolute path as it is, a relative one from `base`.
pub fn resolve(path: &str, base: Option<&Path>) -> PathBuf {
    let absolute = path.starts_with(['\\', '/']) || path.as_bytes().get(1) == Some(&b':');
    let native = if cfg!(windows) { path.to_string() } else { path.replace('\\', "/") };
    match base {
        Some(b) if !absolute => b.join(native),
        _ => PathBuf::from(native),
    }
}

/// The workbook's links, for the pane and the dialog.
pub fn rows(d: &DocState) -> Vec<LinkRow> {
    let base = base_dir(d);
    d.wb.external_books
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let state = d.links.files.get(&key(&b.path)).cloned().unwrap_or_default();
            let status = if b.path.is_empty() { LinkStatus::NotApplicable } else { state.status };
            LinkRow {
                index: i + 1,
                name: if b.path.is_empty() { format!("[{}]", i + 1) } else { b.file_name().to_string() },
                path: b.path.clone(),
                location: if b.path.is_empty() { String::new() } else { resolve(&b.path, base.as_deref()).to_string_lossy().into_owned() },
                kind: if b.path.is_empty() { "Other" } else { "Worksheet" },
                update: "A",
                status,
                status_text: status.label(),
                detail: state.detail,
                last_refreshed: state.refreshed,
                sheets: b.sheets.iter().map(|s| s.name.clone()).collect(),
            }
        })
        .collect()
}

/// Whether opening `wb` disables its links' automatic update (the security warning): it has
/// links to files, and its startup prompt doesn't say never to update them.
pub fn blocked_on_open(wb: &Workbook) -> bool {
    wb.update_links != UpdateLinks::Never && wb.external_books.iter().any(|b| !b.path.is_empty())
}

// ---------------------------------------------------------------- enabled

fn has_links(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    if s.active().is_some_and(|d| !d.wb.external_books.is_empty()) { Ok(()) } else { Err("this workbook has no links to other workbooks".into()) }
}

/// The reason commands that read source files are off where there is no file system.
pub const NO_FILES: &str = "updating links reads the linked workbooks' files, and there is no file system here";

fn can_read_sources(s: &Session) -> std::result::Result<(), String> {
    has_links(s)?;
    if s.link_files.available() { Ok(()) } else { Err(NO_FILES.into()) }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "data.workbookLinks",
            "Workbook Links",
            ["Data", "Queries & Connections"],
            None,
            "{check?: bool (default true: read each source to show its status)} → {links: [{index, name, path, location, type, update, status, statusText, lastRefreshed?, sheets, formulas}], startupPrompt, updatesDisabled}; opens the Workbook Links pane",
            has_links,
            workbook_links
        ),
        cmd!(
            query "data.editLinks",
            "Edit Links to Files",
            ["File", "Info"],
            None,
            "→ {links, startupPrompt, updatesDisabled}; opens the Edit Links dialog",
            has_links,
            edit_links
        ),
        cmd!(
            noundo "data.checkLinkStatus",
            "Check Status",
            ["Data", "Queries & Connections", "Workbook Links"],
            None,
            "{link?: n | path | file name (default: every link)} → {links}",
            can_read_sources,
            check_status
        ),
        cmd!(
            "data.refreshLinks",
            "Update Values",
            ["Data", "Queries & Connections", "Workbook Links"],
            None,
            "{link?: n | path | file name (default: every link)} → {updated, errors, links}: reads the values from the source files",
            can_read_sources,
            refresh_links
        ),
        cmd!(
            "data.changeLinkSource",
            "Change Source…",
            ["Data", "Queries & Connections", "Workbook Links"],
            None,
            "{link: n | path | file name, path: new source file} → {links}; without `path` opens the Change Source dialog",
            can_read_sources,
            change_source
        ),
        cmd!(
            noundo "data.openLinkSource",
            "Open Source",
            ["Data", "Queries & Connections", "Workbook Links"],
            None,
            "{link: n | path | file name} → the opened workbook (file.open)",
            can_read_sources,
            open_source
        ),
        cmd!(
            "data.breakLink",
            "Break Link",
            ["Data", "Queries & Connections", "Workbook Links"],
            None,
            "{link?: n | path | file name (default: every link)} → {formulas, removed, kept, links}: formulas that use the link become their values",
            has_links,
            break_link
        ),
        cmd!(
            "data.linkStartupPrompt",
            "Startup Prompt…",
            ["Data", "Queries & Connections", "Workbook Links"],
            None,
            "{mode?: userSet (let users choose) | never (don't display the alert and don't update) | always (don't display the alert and update)} → {mode}; without `mode` opens the Startup Prompt dialog",
            has_doc,
            startup_prompt
        ),
        cmd!(
            "data.enableLinkUpdates",
            "Enable Content",
            ["Data", "Queries & Connections", "Workbook Links"],
            None,
            "→ {updated, errors, links}: enables the automatic update of links disabled when the workbook was opened, and updates them",
            can_read_sources,
            enable_updates
        ),
    ]
}

// ---------------------------------------------------------------- helpers

/// The links (0-based) a call names with `link`: its number (`1` or `"[1]"` for `[1]`), its path
/// or its file name. Without `link`, every link (`all`) or an error.
fn targets(wb: &Workbook, p: &Json, id: &str, all: bool) -> Result<Vec<usize>> {
    let books = &wb.external_books;
    let n = match p.get("link") {
        None | Some(Json::Null) if all => return Ok((0..books.len()).collect()),
        None | Some(Json::Null) => return Err(bad(id, "missing `link` (its number, path or file name)")),
        Some(Json::Number(n)) => n.as_u64().and_then(|n| usize::try_from(n).ok()),
        Some(Json::String(t)) => {
            let t = t.trim();
            let number = t.strip_prefix('[').and_then(|x| x.strip_suffix(']')).unwrap_or(t);
            match number.parse::<usize>() {
                Ok(n) => Some(n),
                Err(_) => books
                    .iter()
                    .position(|b| !b.path.is_empty() && key(&b.path) == key(t))
                    .or_else(|| books.iter().position(|b| !b.path.is_empty() && b.file_name().eq_ignore_ascii_case(t)))
                    .map(|i| i + 1),
            }
        }
        Some(other) => return Err(bad(id, format!("`link` must be a number, path or file name, not {other}"))),
    };
    match n {
        Some(n) if n >= 1 && n <= books.len() => Ok(vec![n - 1]),
        _ => Err(bad(id, format!("no such link: {}", p.get("link").map(|v| v.to_string()).unwrap_or_default()))),
    }
}

/// Whether a formula has no value saved with it: the source was written by a program that
/// doesn't save values, so it is calculated before its values are read.
fn needs_calc(wb: &Workbook) -> bool {
    wb.sheets.iter().any(|sh| sh.cells.iter().any(|(_, c)| c.formula.is_some() && c.value.is_empty()))
}

/// Link `i`'s new contents read from its source, with the link's status after reading.
struct Read {
    book: Option<ExternalBook>,
    status: LinkStatus,
    detail: Option<String>,
}

/// Reads link `i` of `wb` from its file (relative to `base`), or from `source` instead.
fn read_source(files: &LinkFiles, wb: &Workbook, base: Option<&Path>, i: usize, source: Option<&Path>) -> Read {
    let fail = |status, detail: String| Read { book: None, status, detail: Some(detail) };
    let Some(book) = wb.external_books.get(i) else { return fail(LinkStatus::Undetermined, "no such link".into()) };
    if book.path.is_empty() && source.is_none() {
        return Read { book: None, status: LinkStatus::NotApplicable, detail: None };
    }
    let Some(read) = files.0.as_ref() else { return fail(LinkStatus::Unknown, NO_FILES.into()) };
    let path = source.map(Path::to_path_buf).unwrap_or_else(|| resolve(&book.path, base));
    let bytes = match read(&path) {
        Ok(b) => b,
        Err(e) => return fail(LinkStatus::SourceNotFound, e),
    };
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| book.file_name().to_string());
    let mut src = match crate::io::read_link_source(&name, &bytes) {
        Ok(wb) => wb,
        Err(e) => return fail(LinkStatus::Undetermined, e.to_string()),
    };
    if needs_calc(&src) {
        gridcraft_calc::Calc::new().recalc_all(&mut src);
    }
    match book_from_source(&src, &wb.link_needs(i + 1), book, MAX_LINK_CELLS) {
        Ok((new, missing)) if missing.is_empty() => Read { book: Some(new), status: LinkStatus::Ok, detail: None },
        Ok((new, missing)) => Read { book: Some(new), status: LinkStatus::WorksheetNotFound, detail: Some(missing.join(", ")) },
        Err(e) => fail(LinkStatus::Undetermined, e),
    }
}

/// Whether two links hold the same sheets, values and names.
fn same_values(a: &ExternalBook, b: &ExternalBook) -> bool {
    a.sheets == b.sheets && a.names == b.names
}

fn listing(s: &Session) -> Result<Json> {
    let d = s.doc()?;
    let mut links = serde_json::to_value(rows(d)).map_err(|e| EngineError::Other(e.to_string()))?;
    if let Some(list) = links.as_array_mut() {
        for (i, row) in list.iter_mut().enumerate() {
            row["formulas"] = json!(d.wb.link_formula_count(i + 1));
        }
    }
    Ok(json!({"links": links, "startupPrompt": d.wb.update_links.as_str(), "updatesDisabled": d.links.updates_disabled}))
}

/// Records what reading the sources found.
fn record(s: &mut Session, reads: &[(String, LinkStatus, Option<String>, bool)]) -> Result<()> {
    let now = gridcraft_core::date::unix_now_ms();
    let d = s.doc_mut()?;
    for (path, status, detail, refreshed) in reads {
        if path.is_empty() {
            continue;
        }
        let state = d.links.files.entry(key(path)).or_default();
        state.status = *status;
        state.detail = detail.clone();
        if *refreshed {
            state.refreshed = now;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- commands

fn workbook_links(s: &mut Session, p: &Json) -> Result<Json> {
    if p.get("check").and_then(Json::as_bool).unwrap_or(true) && s.link_files.available() {
        check(s, &(0..s.doc()?.wb.external_books.len()).collect::<Vec<_>>())?;
    }
    s.ui_requests.push(crate::UiRequest::Dialog("workbookLinks".into(), json!({})));
    listing(s)
}

fn edit_links(s: &mut Session, _: &Json) -> Result<Json> {
    s.ui_requests.push(crate::UiRequest::Dialog("editLinks".into(), json!({})));
    listing(s)
}

fn check(s: &mut Session, which: &[usize]) -> Result<()> {
    let d = s.doc()?;
    let base = base_dir(d);
    let reads: Vec<(String, LinkStatus, Option<String>, bool)> = which
        .iter()
        .filter_map(|&i| {
            let book = d.wb.external_books.get(i)?;
            let r = read_source(&s.link_files, &d.wb, base.as_deref(), i, None);
            // Readable, but the link holds other values than the file: Update Values would change them.
            let status = match &r.book {
                Some(new) if r.status == LinkStatus::Ok && !same_values(new, book) => LinkStatus::ValuesNotUpdated,
                _ => r.status,
            };
            Some((book.path.clone(), status, r.detail, false))
        })
        .collect();
    record(s, &reads)
}

fn check_status(s: &mut Session, p: &Json) -> Result<Json> {
    let which = targets(&s.doc()?.wb, p, "data.checkLinkStatus", true)?;
    check(s, &which)?;
    listing(s)
}

/// Reads the sources of links `which` and puts their values in the workbook (one undo step).
fn update(s: &mut Session, which: &[usize]) -> Result<Json> {
    let d = s.doc()?;
    let base = base_dir(d);
    let reads: Vec<(usize, String, Read)> = which
        .iter()
        .filter_map(|&i| Some((i, d.wb.external_books.get(i)?.path.clone(), read_source(&s.link_files, &d.wb, base.as_deref(), i, None))))
        .collect();
    let changed: Vec<(usize, ExternalBook)> = reads
        .iter()
        .filter_map(|(i, _, r)| {
            let new = r.book.as_ref()?;
            let old = d.wb.external_books.get(*i)?;
            (!same_values(new, old)).then(|| (*i, new.clone()))
        })
        .collect();
    if !changed.is_empty() {
        edit(s, |cx| {
            let books = Arc::make_mut(&mut cx.wb.external_books);
            for (i, b) in changed {
                if let Some(slot) = books.get_mut(i) {
                    *slot = b;
                }
            }
            cx.structural = true;
            Ok(())
        })?;
    }
    let updated = reads.iter().filter(|(_, _, r)| r.book.is_some()).count();
    let errors: Vec<Json> = reads
        .iter()
        .filter(|(_, _, r)| r.status.is_error())
        .map(|(i, path, r)| json!({"index": i + 1, "path": path, "status": r.status, "statusText": r.status.label(), "detail": r.detail}))
        .collect();
    let recorded: Vec<(String, LinkStatus, Option<String>, bool)> =
        reads.iter().map(|(_, path, r)| (path.clone(), r.status, r.detail.clone(), r.book.is_some())).collect();
    record(s, &recorded)?;
    if updated > 0 {
        s.doc_mut()?.links.updates_disabled = false;
    }
    if !errors.is_empty() {
        let names: Vec<String> = reads
            .iter()
            .filter(|(_, _, r)| r.status.is_error())
            .map(|(_, path, r)| {
                format!("{}: {}", Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), r.status.label())
            })
            .collect();
        s.ui_requests.push(crate::UiRequest::Message(format!(
            "Some links couldn't be updated; they keep the values they had.\n\n{}\n\nUse Change Source… to pick the file, or Check Status to try again.",
            names.join("\n")
        )));
    }
    let mut out = listing(s)?;
    out["updated"] = json!(updated);
    out["errors"] = Json::Array(errors);
    Ok(out)
}

fn refresh_links(s: &mut Session, p: &Json) -> Result<Json> {
    let which = targets(&s.doc()?.wb, p, "data.refreshLinks", true)?;
    update(s, &which)
}

fn enable_updates(s: &mut Session, _: &Json) -> Result<Json> {
    s.doc_mut()?.links.updates_disabled = false;
    let all: Vec<usize> = (0..s.doc()?.wb.external_books.len()).collect();
    update(s, &all)
}

fn change_source(s: &mut Session, p: &Json) -> Result<Json> {
    const ID: &str = "data.changeLinkSource";
    let i = *targets(&s.doc()?.wb, p, ID, false)?.first().ok_or_else(|| bad(ID, "missing `link`"))?;
    let Some(new_path) = str_param(p, "path").map(str::trim).filter(|t| !t.is_empty()).map(str::to_string) else {
        let row = rows(s.doc()?).into_iter().nth(i);
        s.ui_requests.push(crate::UiRequest::Dialog("changeLinkSource".into(), json!({"link": i + 1, "path": row.map(|r| r.location)})));
        return listing(s);
    };
    let d = s.doc()?;
    let base = base_dir(d);
    let file = resolve(&new_path, base.as_deref());
    // Read the new source first: a file that can't be read leaves the link as it was.
    let r = read_source(&s.link_files, &d.wb, base.as_deref(), i, Some(&file));
    if r.book.is_none() {
        let why = r.detail.unwrap_or_else(|| r.status.label().to_string());
        return Err(EngineError::Other(format!("We can't use '{new_path}' as the source: {why}")));
    }
    // Another link already reads that file: its references join it (Excel merges them).
    let same = |b: &ExternalBook| !b.path.is_empty() && key(&resolve(&b.path, base.as_deref()).to_string_lossy()) == key(&file.to_string_lossy());
    let other = d.wb.external_books.iter().enumerate().position(|(j, b)| j != i && same(b));
    let files = s.link_files.clone();
    let (status, detail) = edit(s, |cx| {
        let target = match other {
            Some(j) => {
                cx.wb.merge_link(i + 1, j + 1);
                if j > i { j - 1 } else { j }
            }
            None => {
                if let Some(b) = Arc::make_mut(&mut cx.wb.external_books).get_mut(i) {
                    b.path = new_path.clone();
                    b.kept = None;
                }
                i
            }
        };
        // The values come from the new file, for what this workbook reads from it now.
        let r = read_source(&files, &cx.wb, base.as_deref(), target, Some(&file));
        if let (Some(book), Some(slot)) = (r.book, Arc::make_mut(&mut cx.wb.external_books).get_mut(target)) {
            *slot = book;
        }
        cx.structural = true;
        Ok((r.status, r.detail))
    })?;
    record(s, &[(new_path, status, detail, true)])?;
    s.doc_mut()?.links.updates_disabled = false;
    listing(s)
}

fn open_source(s: &mut Session, p: &Json) -> Result<Json> {
    const ID: &str = "data.openLinkSource";
    let d = s.doc()?;
    let i = *targets(&d.wb, p, ID, false)?.first().ok_or_else(|| bad(ID, "missing `link`"))?;
    let book = d.wb.external_books.get(i).ok_or_else(|| bad(ID, "no such link"))?;
    if book.path.is_empty() {
        return Err(EngineError::Other("This link names no workbook file.".into()));
    }
    let path = resolve(&book.path, base_dir(d).as_deref());
    let path = path.to_str().ok_or_else(|| EngineError::Other("The source's path can't be opened.".into()))?.to_string();
    s.execute("file.open", json!({"path": path}))
}

fn break_link(s: &mut Session, p: &Json) -> Result<Json> {
    let which: BTreeSet<usize> = targets(&s.doc()?.wb, p, "data.breakLink", true)?.into_iter().map(|i| i + 1).collect();
    let report = edit(s, |cx| {
        let r = cx.wb.break_links(&which);
        cx.structural = true;
        Ok(r)
    })?;
    let mut out = listing(s)?;
    out["formulas"] = json!(report.formulas);
    out["removed"] = json!(report.removed);
    out["kept"] = json!(report.kept);
    Ok(out)
}

fn startup_prompt(s: &mut Session, p: &Json) -> Result<Json> {
    let Some(mode) = str_param(p, "mode") else {
        let current = s.doc()?.wb.update_links.as_str();
        s.ui_requests.push(crate::UiRequest::Dialog("linkStartupPrompt".into(), json!({"mode": current})));
        return Ok(json!({"mode": current}));
    };
    let mode = UpdateLinks::parse(mode).ok_or_else(|| bad("data.linkStartupPrompt", "`mode` is userSet, never or always"))?;
    edit(s, |cx| {
        cx.wb.update_links = mode;
        Ok(json!({"mode": mode.as_str()}))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_start_from_the_workbook_folder() {
        let base = PathBuf::from("base");
        assert_eq!(resolve("Source.xlsx", Some(&base)), base.join("Source.xlsx"));
        // Windows keeps native separators; other hosts normalise imported Windows
        // paths to forward slashes. An absolute drive path must never gain `base`.
        let drive = if cfg!(windows) { r"C:\links\Source.xlsx" } else { "C:/links/Source.xlsx" };
        assert_eq!(resolve(r"C:\links\Source.xlsx", Some(&base)), PathBuf::from(drive));
        let nested = if cfg!(windows) { r"links\Source.xlsx" } else { "links/Source.xlsx" };
        assert_eq!(resolve(r"links\Source.xlsx", Some(&base)), base.join(nested));
        assert_eq!(resolve("/srv/Source.xlsx", Some(&base)), PathBuf::from("/srv/Source.xlsx"));
        assert_eq!(resolve("Source.xlsx", None), PathBuf::from("Source.xlsx"));
    }

    #[test]
    fn statuses_read_as_excel_names_them() {
        assert_eq!(LinkStatus::SourceNotFound.label(), "Error: Source not found");
        assert_eq!(LinkStatus::ValuesNotUpdated.label(), "Warning: Values not updated");
        assert_eq!(serde_json::to_value(LinkStatus::WorksheetNotFound).unwrap(), json!("worksheetNotFound"));
    }
}
