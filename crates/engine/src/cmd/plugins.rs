//! Plugins (Home › Add-ins): load and unload out-of-process plugins, list them, and run their
//! commands. Their functions are called from formulas (see `gridcraft_plugin`).

use std::sync::Arc;

use gridcraft_core::RangeRef;
use serde_json::{Value as Json, json};

use gridcraft_plugin::install::Fetched;
use gridcraft_plugin::requires::MAX_DEPTH;
use gridcraft_plugin::version::{Range, Version};

use super::{CommandSpec, always, bad, bool_param, cmd, str_param};
use crate::{Result, Session, UiRequest};

/// Most selected cells whose values a plugin command receives.
const MAX_CONTEXT_CELLS: u64 = 100_000;
/// Most defined names a plugin command receives (real models can carry tens of thousands).
const MAX_CONTEXT_NAMES: usize = 10_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "plugins.list",
            "Plugins",
            [],
            None,
            "{} → [{id, name, version, description, dir, status, state, blockedBy, requires, requiredBy, functions, commands}]. `state`: loaded, running, failed, blocked (a requirement isn't met: `blockedBy` [{id?, kind, need, have?, message}]) or disabled",
            always,
            |_, _| Ok(list())
        ),
        cmd!(
            query "plugins.load",
            "Load Plugin",
            [],
            None,
            "{path, force?} → the plugin. `path`: a folder holding plugin.json, or the manifest. Replacing a plugin with a version outside the range a turned-on plugin requires fails unless `force` (that plugin is then blocked)",
            always,
            load
        ),
        cmd!(
            query "plugins.unload",
            "Unload Plugin",
            [],
            None,
            "{id} → {unloaded, dependentsBlocked}. The plugins that require it are blocked, and listed",
            always,
            unload
        ),
        cmd!(
            query "plugins.install",
            "Install Plugin from GitHub",
            [],
            None,
            "{url (or github), dir?, version?, withRequirements?, force?} → the plugin, with `release`, `missing` (its required and optional add-ins that aren't installed or are out of range) and, with `withRequirements`, `installed` and `failed`. Downloads the plugin in a GitHub repository (the release zip for this platform, else its source; with `version`, a range, the newest release whose tag is in it) into `dir` (default: the app's plugins folder) and loads it. `withRequirements` (default false) also installs the missing required add-ins that name `github`. A version outside the range a turned-on plugin requires fails unless `force`",
            always,
            install
        ),
        cmd!(
            query "plugins.installMissing",
            "Install Missing Add-ins",
            [],
            None,
            "{id, dir?, force?} → {id, installed: [{id, name, version, release}], failed: [{id, error}], missing, satisfied}. Installs the missing or out-of-range required add-ins of plugin `id` that name `github` (the newest release in range), and theirs in turn",
            always,
            install_missing
        ),
        cmd!(
            query "plugins.requirements",
            "Add-in Requirements",
            [],
            None,
            "{id} → {id, satisfied, blockedBy, tree: [{id, name, need, have, state, optional, why, github, children}]}. `state` `missing` for an add-in that isn't installed",
            always,
            requirements
        ),
        cmd!(
            query "plugins.setEnabled",
            "Turn Plugin On or Off",
            [],
            None,
            "{id, enabled} → the plugin, with `dependentsBlocked` (the plugins that require it, now blocked). Off, a plugin stays listed but its functions, commands, ribbon and menu items are gone; remembered in the preferences",
            always,
            set_enabled
        ),
        cmd!(query "plugins.openHomepage", "Plugin Documentation", [], None, "{id} → {url}. Opens the plugin's web page (its manifest's `homepage`) in the browser", always, open_homepage),
        cmd!(
            "plugins.run",
            "Run Plugin Command",
            [],
            None,
            "{plugin, command, params?} → {ran, message?, result?, open?, dialog?, collab?}. The plugin's commands run as one undoable step; `open` (a file) and `dialog` (protocol 1.2) and `collab.join` (a live session, protocol 2) are returned for the caller, and the interface performs them when a button, menu item or plugin dialog ran the command",
            always,
            run
        ),
    ]
}

/// `home.addIns` and `plugins.list`.
pub fn list() -> Json {
    Json::Array(gridcraft_plugin::list().iter().map(|p| p.info()).collect())
}

fn load(s: &mut Session, p: &Json) -> Result<Json> {
    const ID: &str = "plugins.load";
    let path = str_param(p, "path").filter(|x| !x.trim().is_empty()).ok_or_else(|| bad(ID, "missing `path`"))?;
    let path = std::path::Path::new(path);
    let (manifest, _) = gridcraft_plugin::read_manifest(path).map_err(|e| bad(ID, e))?;
    check_conflicts(ID, &manifest, bool_param(p, "force").unwrap_or(false))?;
    load_from(s, ID, path)
}

/// Refuses a version outside the range a turned-on plugin requires, unless `force`.
fn check_conflicts(id: &str, manifest: &gridcraft_plugin::Manifest, force: bool) -> Result<()> {
    let conflicts = gridcraft_plugin::conflicts(manifest);
    if conflicts.is_empty() || force {
        return Ok(());
    }
    let what: Vec<&str> = conflicts.iter().map(|c| c.message.as_str()).collect();
    Err(bad(id, format!("{}. Pass `force: true` to install it anyway (those plugins are then blocked)", what.join("; "))))
}

/// The plugins folder: `dir`, else the app's.
fn plugins_root(id: &str, p: &Json) -> Result<std::path::PathBuf> {
    match str_param(p, "dir").filter(|x| !x.trim().is_empty()) {
        Some(d) => Ok(std::path::PathBuf::from(d)),
        None => gridcraft_plugin::install::install_dir().ok_or_else(|| bad(id, "no plugins folder: pass `dir`")),
    }
}

fn install(s: &mut Session, p: &Json) -> Result<Json> {
    const ID: &str = "plugins.install";
    let url = str_param(p, "url")
        .or_else(|| str_param(p, "github"))
        .filter(|x| !x.trim().is_empty())
        .ok_or_else(|| bad(ID, "missing `url` (or `github`)"))?;
    let root = plugins_root(ID, p)?;
    let force = bool_param(p, "force").unwrap_or(false);
    let range = match str_param(p, "version") {
        Some(v) => Some(Range::parse(v).map_err(|e| bad(ID, e))?),
        None => None,
    };
    let fetched = gridcraft_plugin::install::fetch(url, range.as_ref()).map_err(|e| bad(ID, e))?;
    let mut fetch = |github: &str, range: &Range| gridcraft_plugin::install::fetch(github, Some(range));
    install_fetched(s, ID, fetched, &root, force, bool_param(p, "withRequirements").unwrap_or(false), &mut fetch)
}

/// Downloads a plugin for [`install_fetched`] and [`install_requirements`]: `(github, range)`.
pub type Fetcher<'a> = dyn FnMut(&str, &Range) -> std::result::Result<Fetched, String> + 'a;

/// Installs a downloaded plugin into `root` and loads it (`plugins.install`, after the
/// download): refused when its version is outside the range a turned-on plugin requires, unless
/// `force`. With `with_requirements`, its missing required add-ins that name `github` are
/// installed too, through `fetch`.
pub fn install_fetched(
    s: &mut Session,
    id: &str,
    fetched: Fetched,
    root: &std::path::Path,
    force: bool,
    with_requirements: bool,
    fetch: &mut Fetcher<'_>,
) -> Result<Json> {
    check_conflicts(id, &fetched.manifest, force)?;
    let dir = fetched.unpack(root).map_err(|e| bad(id, e))?;
    let mut info = load_from(s, id, &dir)?;
    let plugin = fetched.manifest.id.clone();
    info["release"] = json!(fetched.release);
    if with_requirements {
        let (installed, failed) = install_requirements(s, &plugin, root, false, fetch);
        info["installed"] = Json::Array(installed);
        info["failed"] = Json::Array(failed);
        // Its own state may have changed since `load_from`.
        if let Some(p) = gridcraft_plugin::get(&plugin) {
            info["state"] = json!(p.state());
            info["blockedBy"] = json!(p.blocked_by());
        }
    }
    info["missing"] = json!(gridcraft_plugin::missing(&plugin));
    Ok(info)
}

/// Installs `plugin`'s missing or out-of-range required add-ins that name `github` (the newest
/// release in range, through `fetch`), then theirs in turn, at most [`MAX_DEPTH`] deep and 64 in
/// all. Returns what was installed (`{id, name, version, release}`) and what failed (`{id, error}`).
pub fn install_requirements(s: &mut Session, plugin: &str, root: &std::path::Path, force: bool, fetch: &mut Fetcher<'_>) -> (Vec<Json>, Vec<Json>) {
    const ID: &str = "plugins.installMissing";
    const MAX_INSTALLS: usize = 64;
    let (mut installed, mut failed) = (Vec::new(), Vec::new());
    let mut seen: Vec<String> = vec![plugin.to_string()];
    let mut queue: std::collections::VecDeque<(String, usize)> = std::collections::VecDeque::from([(plugin.to_string(), 0)]);
    while let Some((pid, depth)) = queue.pop_front() {
        for m in gridcraft_plugin::missing(&pid).into_iter().filter(|m| !m.optional) {
            if seen.contains(&m.id) || installed.len() + failed.len() >= MAX_INSTALLS {
                continue;
            }
            seen.push(m.id.clone());
            let Some(github) = m.github.clone() else { continue };
            let outcome = (|| -> std::result::Result<Json, String> {
                let range = Range::parse(&m.version)?;
                let fetched = fetch(&github, &range)?;
                let got = &fetched.manifest;
                if got.id != m.id {
                    return Err(format!("github.com/{github} holds {}, not {}", got.id, m.id));
                }
                let fits = Version::parse(&got.version).map(|v| range.matches(&v)).unwrap_or(range.is_any());
                if !fits {
                    return Err(format!("{} {} from github.com/{github} isn't in {}", m.name, got.version, range));
                }
                check_conflicts(ID, got, force).map_err(|e| e.to_string())?;
                let dir = fetched.unpack(root)?;
                load_from(s, ID, &dir).map_err(|e| e.to_string())?;
                Ok(json!({"id": m.id, "name": got.name, "version": got.version, "release": fetched.release}))
            })();
            match outcome {
                Ok(done) => {
                    installed.push(done);
                    if depth + 1 < MAX_DEPTH {
                        queue.push_back((m.id.clone(), depth + 1));
                    }
                }
                Err(e) => failed.push(json!({"id": m.id, "error": e})),
            }
        }
    }
    (installed, failed)
}

fn install_missing(s: &mut Session, p: &Json) -> Result<Json> {
    const ID: &str = "plugins.installMissing";
    let id = str_param(p, "id").ok_or_else(|| bad(ID, "missing `id`"))?;
    if gridcraft_plugin::get(id).is_none() {
        return Err(bad(ID, format!("no plugin `{id}` is loaded")));
    }
    let root = plugins_root(ID, p)?;
    let mut fetch = |github: &str, range: &Range| gridcraft_plugin::install::fetch(github, Some(range));
    Ok(install_missing_with(s, id, &root, bool_param(p, "force").unwrap_or(false), &mut fetch))
}

/// `plugins.installMissing` with a given downloader (tests use one without a network).
pub fn install_missing_with(s: &mut Session, id: &str, root: &std::path::Path, force: bool, fetch: &mut Fetcher<'_>) -> Json {
    let (installed, failed) = install_requirements(s, id, root, force, fetch);
    json!({
        "id": id,
        "installed": installed,
        "failed": failed,
        "missing": gridcraft_plugin::missing(id),
        "satisfied": gridcraft_plugin::get(id).is_some_and(|p| !p.is_blocked()),
    })
}

fn requirements(_: &mut Session, p: &Json) -> Result<Json> {
    const ID: &str = "plugins.requirements";
    let id = str_param(p, "id").ok_or_else(|| bad(ID, "missing `id`"))?;
    gridcraft_plugin::requirements(id).map_err(|e| bad(ID, e))
}

/// The plugins in `ids` that are blocked now.
fn now_blocked(ids: &[String]) -> Vec<String> {
    ids.iter().filter(|d| gridcraft_plugin::get(d).is_some_and(|p| p.is_blocked())).cloned().collect()
}

/// Loads the plugin in `path` (a folder or its manifest) and recalculates the formulas using it
/// and those of the plugins it unblocked or blocked.
pub fn load_from(s: &mut Session, id: &str, path: &std::path::Path) -> Result<Json> {
    let before = activity();
    let plugin = gridcraft_plugin::load(path).map_err(|e| bad(id, e))?;
    // A plugin turned off stays off when it's loaded again.
    if s.prefs.disabled_plugins.contains(&plugin.manifest.id) {
        let _ = gridcraft_plugin::set_enabled(&plugin.manifest.id, false);
    }
    let mut names: Vec<String> = plugin.manifest.functions.iter().map(|f| f.name.clone()).collect();
    names.extend(changed_functions(&before));
    recalc_users(s, &names);
    Ok(plugin.info())
}

/// Which plugins are active (turned on and not blocked), to see whose functions came or went.
fn activity() -> Vec<(String, bool)> {
    gridcraft_plugin::list().iter().map(|p| (p.manifest.id.clone(), p.active())).collect()
}

/// The functions of the plugins whose activity changed since `before`.
fn changed_functions(before: &[(String, bool)]) -> Vec<String> {
    gridcraft_plugin::list()
        .iter()
        .filter(|p| before.iter().find(|(id, _)| *id == p.manifest.id).is_none_or(|(_, was)| *was != p.active()))
        .flat_map(|p| p.manifest.functions.iter().map(|f| f.name.clone()).collect::<Vec<_>>())
        .collect()
}

fn unload(s: &mut Session, p: &Json) -> Result<Json> {
    let id = str_param(p, "id").ok_or_else(|| bad("plugins.unload", "missing `id`"))?;
    let plugin = gridcraft_plugin::get(id).ok_or_else(|| bad("plugins.unload", format!("no plugin `{id}` is loaded")))?;
    let dependents = gridcraft_plugin::dependents(id);
    let before = activity();
    gridcraft_plugin::unload(id);
    let mut names: Vec<String> = plugin.manifest.functions.iter().map(|f| f.name.clone()).collect();
    names.extend(changed_functions(&before));
    recalc_users(s, &names);
    Ok(json!({"unloaded": id, "dependentsBlocked": now_blocked(&dependents)}))
}

fn set_enabled(s: &mut Session, p: &Json) -> Result<Json> {
    const ID: &str = "plugins.setEnabled";
    let id = str_param(p, "id").ok_or_else(|| bad(ID, "missing `id`"))?;
    let on = bool_param(p, "enabled").ok_or_else(|| bad(ID, "missing `enabled` (true or false)"))?;
    let plugin = gridcraft_plugin::get(id).ok_or_else(|| bad(ID, format!("no plugin `{id}` is loaded")))?;
    let dependents = if on { Vec::new() } else { gridcraft_plugin::dependents(id) };
    let before = activity();
    let changed = gridcraft_plugin::set_enabled(id, on).map_err(|e| bad(ID, e))?;
    s.prefs.disabled_plugins.retain(|x| x != id);
    if !on {
        s.prefs.disabled_plugins.push(id.to_string());
    }
    if changed {
        let mut names: Vec<String> = plugin.manifest.functions.iter().map(|f| f.name.clone()).collect();
        names.extend(changed_functions(&before));
        recalc_users(s, &names);
    }
    let mut info = plugin.info();
    info["dependentsBlocked"] = json!(now_blocked(&dependents));
    Ok(info)
}

fn open_homepage(s: &mut Session, p: &Json) -> Result<Json> {
    const ID: &str = "plugins.openHomepage";
    let id = str_param(p, "id").ok_or_else(|| bad(ID, "missing `id`"))?;
    let plugin = gridcraft_plugin::get(id).ok_or_else(|| bad(ID, format!("no plugin `{id}` is loaded")))?;
    let url = plugin.manifest.homepage.trim();
    if url.is_empty() {
        return Err(bad(ID, format!("plugin {id} has no web page")));
    }
    s.ui_requests.push(UiRequest::OpenUrl(url.to_string()));
    Ok(json!({"url": url}))
}

/// Turns off the loaded plugins the preferences list as off (at startup, after loading them).
pub fn apply_prefs(prefs: &crate::Prefs) {
    for id in &prefs.disabled_plugins {
        if gridcraft_plugin::get(id).is_some()
            && let Err(e) = gridcraft_plugin::set_enabled(id, false)
        {
            log::warn!("plugin {id}: {e}");
        }
    }
    // Say which plugins can't start, and why.
    for (id, why) in gridcraft_plugin::refresh().blocked {
        log::warn!("plugin {id} is blocked: {}", why.iter().map(|b| b.message.as_str()).collect::<Vec<_>>().join("; "));
    }
}

/// Recalculates the open workbooks with formulas that call one of `names`, so their cells pick
/// up a plugin that just arrived (or show `#NAME?` for one that left).
fn recalc_users(s: &mut Session, names: &[String]) {
    recalc_users_with(s, names, false);
}

/// [`recalc_users`] for functions a plugin invalidated (protocol 2.2): a workbook without unsaved
/// changes stays so (its formulas' results changed, not what the person wrote).
pub(crate) fn recalc_users_keeping_saved(s: &mut Session, names: &[String]) {
    recalc_users_with(s, names, true);
}

fn recalc_users_with(s: &mut Session, names: &[String], keep_saved: bool) {
    if names.is_empty() {
        return;
    }
    let calls: Vec<String> = names.iter().map(|n| format!("{}(", n.to_ascii_uppercase())).collect();
    for d in s.docs.iter_mut() {
        let uses = d.wb.sheets.iter().any(|sh| {
            sh.cells.iter().any(|(_, cell)| cell.formula.as_ref().is_some_and(|f| calls.iter().any(|c| f.text.to_ascii_uppercase().contains(c))))
        });
        if uses {
            let clean = keep_saved && !d.is_dirty();
            let doc = d.calling_document();
            let (calc, wb) = (&mut d.calc, Arc::make_mut(&mut d.wb));
            gridcraft_calc::external::with_document(Some(doc), || calc.recalc_all(wb));
            if clean {
                d.saved = d.wb.clone();
            }
        }
    }
}

/// An absolute form of `path` (as given when it can't be made absolute).
pub fn absolute_path(path: &str) -> String {
    std::path::absolute(path).map(|p| p.display().to_string()).unwrap_or_else(|_| path.to_string())
}

/// `context.document` (protocol 1.2): the local file the workbook was opened from or last saved
/// to (`null` for a new, never-saved one), its name, and whether it has unsaved changes.
pub fn document_json(d: &crate::DocState) -> Json {
    json!({"path": d.path.as_deref().map(absolute_path), "name": d.display_title(), "dirty": d.is_dirty(), "id": d.session_id()})
}

/// The context every plugin request about the workbook gets: its title, active sheet, active
/// cell, selection and `document` (macros get this much; commands get [`context`]).
pub fn brief_context(s: &Session) -> Json {
    let Some(d) = s.active() else { return json!({}) };
    let sel = &d.selection;
    let range = sel.ranges.last().copied().unwrap_or(RangeRef::cell(sel.active));
    json!({
        "workbook": d.display_title(),
        "sheet": d.wb.active().map(|sh| sh.name.clone()).unwrap_or_default(),
        "active": sel.active.to_string(),
        "selection": range.to_string(),
        "document": document_json(d),
    })
}

/// Sends a document event (protocol 1.2) to the plugins that declared it, after it happened. It
/// never waits for them: they answer on the events thread, and the interface shows what they say.
pub fn emit(event: &str, path: &str, format: Option<&str>, id: Option<&str>) {
    gridcraft_plugin::events::dispatch(event, event_params(path, format, id));
}

/// An event's params: the file's absolute path and name, the format saved, and the document's
/// session id (protocol 2.2, as in `context.document` and `functions/call`).
fn event_params(path: &str, format: Option<&str>, id: Option<&str>) -> Json {
    let path = absolute_path(path);
    let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.clone());
    let mut params = json!({"path": path, "name": name});
    if let Some(f) = format {
        params["format"] = json!(f.to_ascii_lowercase());
    }
    if let Some(id) = id {
        params["id"] = json!(id);
    }
    params
}

/// Protocol 2.2: `documentOpened` for a workbook just read from `path`, before its formulas are
/// calculated. Waits for each plugin that declared it (at most 10 s each), so a plugin that
/// decides what its functions may do per workbook knows of it before the first call.
pub fn emit_opened_before_calc(path: &str, id: &str) {
    let wait = gridcraft_plugin::events::OPENED_BEFORE_CALC_TIMEOUT;
    gridcraft_plugin::events::deliver_now(gridcraft_plugin::events::DOCUMENT_OPENED, event_params(path, None, Some(id)), wait);
}

/// The app is quitting: `documentClosed` for every open workbook that has a file, waiting at most
/// 5 s for the plugins (best effort).
pub fn quitting(s: &Session) {
    for d in s.documents() {
        if let Some(p) = &d.path {
            emit(gridcraft_plugin::events::DOCUMENT_CLOSED, p, None, Some(&d.session_id()));
        }
    }
    gridcraft_plugin::events::wait_idle(gridcraft_plugin::events::timeout(gridcraft_plugin::events::DOCUMENT_CLOSED));
}

/// Checks the file a plugin command asked to open: absolute, an existing regular file, with an
/// extension GridCraft opens.
fn check_open(o: &gridcraft_plugin::dialog::OpenRequest) -> std::result::Result<(), String> {
    let path = std::path::Path::new(&o.path);
    if !path.is_absolute() {
        return Err(format!("can't open {}: the path isn't absolute", o.path));
    }
    let opens = crate::io::FileKind::from_path(&o.path).is_some_and(|k| k != crate::io::FileKind::Html);
    if !opens {
        return Err(format!("can't open {}: GridCraft doesn't open this kind of file", o.path));
    }
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() => Ok(()),
        Ok(_) => Err(format!("can't open {}: it isn't a file", o.path)),
        Err(e) => Err(format!("can't open {}: {e}", o.path)),
    }
}

/// What a plugin command sees of the workbook: its title, the active sheet, the selection and,
/// for a selection up to 100,000 cells, its values (rows of values, as functions get them) and
/// formulas (rows of formula text, `null` where a cell has none); plus every sheet (index, name,
/// visibility, used range) and up to 10,000 defined names (name, scope, hidden, refersTo).
fn context(s: &Session) -> Json {
    let Some(d) = s.active() else { return json!({}) };
    let sel = &d.selection;
    let range = sel.ranges.last().copied().unwrap_or(RangeRef::cell(sel.active));
    let mut ctx = brief_context(s);
    if let Some(sh) = d.wb.active()
        && range.count() <= MAX_CONTEXT_CELLS
    {
        let rows: Vec<Json> = (range.start.row..=range.end.row)
            .map(|r| {
                Json::Array(
                    (range.start.col..=range.end.col)
                        .map(|c| gridcraft_plugin::values::to_json(&sh.value(gridcraft_core::CellRef::new(r, c))))
                        .collect(),
                )
            })
            .collect();
        ctx["values"] = Json::Array(rows);
        // Formula text (`=A1*2`) in the same shape, `null` for cells without one, so plugins can
        // tell inputs from calculations (colour coding, auditing).
        let formulas: Vec<Json> = (range.start.row..=range.end.row)
            .map(|r| {
                Json::Array(
                    (range.start.col..=range.end.col)
                        .map(|c| {
                            sh.cell(gridcraft_core::CellRef::new(r, c))
                                .and_then(|cell| cell.formula.as_ref())
                                .map_or(Json::Null, |f| Json::String(format!("={}", f.text)))
                        })
                        .collect(),
                )
            })
            .collect();
        ctx["formulas"] = Json::Array(formulas);
    }
    // The workbook's shape, as `document.inspect` reports it, so a plugin can act beyond the
    // selection (unhide every sheet, list sheets, clean up names, trim used ranges).
    let wb = &d.wb;
    ctx["activeSheet"] = json!(wb.active_sheet);
    ctx["sheets"] = Json::Array(
        wb.sheets
            .iter()
            .enumerate()
            .map(|(i, sh)| {
                json!({
                    "index": i,
                    "name": sh.name,
                    "visibility": format!("{:?}", sh.visibility),
                    "usedRange": sh.used_range().map(|r| r.a1()),
                })
            })
            .collect(),
    );
    ctx["names"] = Json::Array(
        wb.names
            .iter()
            .take(MAX_CONTEXT_NAMES)
            .map(|n| json!({"name": n.name, "scope": n.scope, "hidden": n.hidden, "refersTo": format!("={}", n.formula)}))
            .collect(),
    );
    if wb.names.len() > MAX_CONTEXT_NAMES {
        ctx["namesTruncated"] = json!(true);
    }
    ctx
}

fn run(s: &mut Session, p: &Json) -> Result<Json> {
    const ID: &str = "plugins.run";
    let plugin = str_param(p, "plugin").ok_or_else(|| bad(ID, "missing `plugin`"))?;
    let command = str_param(p, "command").ok_or_else(|| bad(ID, "missing `command`"))?;
    let params = p.get("params").cloned().unwrap_or_else(|| json!({}));
    // The plugin may read and write the workbook while it works (`host/sync`, protocol 1.1); that
    // and its answer's commands are one step (below).
    let before_host = s.active().map(|d| (d.uid, d.wb.clone(), d.selection.clone(), d.undo.len()));
    let journal_at = s.journal.len();
    let ctx = context(s);
    let reply = gridcraft_plugin::run_command_with(plugin, command, params, ctx, &mut |m, p| super::developer::host_call(s, plugin, m, p));
    s.journal.truncate(journal_at);
    if let Some((uid, wb, sel, undo_len)) = before_host
        && let Some(d) = s.docs.iter_mut().find(|d| d.uid == uid)
    {
        d.undo.truncate(undo_len);
        if reply.is_err() {
            d.wb = wb;
            d.selection = sel;
            d.calc.rebuild(&d.wb);
        }
    }
    let reply = reply.map_err(|e| bad(ID, e))?;
    // A file the plugin asks to open must be one GridCraft can open; otherwise the command fails
    // before anything changed.
    if let Some(o) = &reply.open {
        check_open(o).map_err(|e| bad(ID, format!("{plugin} › {command}: {e}")))?;
    }
    // An Office add-in's command opens its task pane or calls its function: nothing runs here.
    if let Some(office) = reply.office {
        let title = gridcraft_plugin::get(plugin).map(|p| p.manifest.name.clone()).unwrap_or_default();
        s.ui_requests.push(match &office {
            gridcraft_plugin::office::OfficeCommand::ShowTaskpane { url, taskpane_id } => {
                UiRequest::OfficePane { plugin: plugin.to_string(), title, url: url.clone(), pane: taskpane_id.clone() }
            }
            gridcraft_plugin::office::OfficeCommand::ExecuteFunction { function, url, control } => {
                UiRequest::OfficeFunction { plugin: plugin.to_string(), url: url.clone(), function: function.clone(), control: control.clone() }
            }
        });
        return Ok(json!({"ran": 0, "office": office}));
    }
    if let Some((id, _)) = reply.commands.iter().find(|(id, _)| id.starts_with("plugins.")) {
        return Err(bad(ID, format!("a plugin command can't run `{id}`")));
    }
    // The engine commands run as one step: their own undo and journal entries are dropped (this
    // command's entry covers them), and if one fails the workbook goes back to how it was.
    let before = s.active().map(|d| (d.uid, d.wb.clone(), d.selection.clone(), d.undo.len()));
    let journal_len = s.journal.len();
    let mut failure = None;
    for (id, params) in &reply.commands {
        if let Err(e) = s.execute(id, params.clone()) {
            failure = Some(format!("{id}: {e}"));
            break;
        }
    }
    s.journal.truncate(journal_len);
    if let Some((uid, wb, sel, undo_len)) = before
        && let Some(d) = s.docs.iter_mut().find(|d| d.uid == uid)
    {
        d.undo.truncate(undo_len);
        if failure.is_some() {
            d.wb = wb;
            d.selection = sel;
            d.calc.rebuild(&d.wb);
        }
    }
    if let Some(why) = failure {
        return Err(bad(ID, format!("{plugin} › {command} failed, nothing was changed: {why}")));
    }
    if let Some(m) = &reply.message {
        s.ui_requests.push(UiRequest::Message(m.clone()));
    }
    let mut out = json!({"ran": reply.commands.len()});
    if let Some(m) = reply.message {
        out["message"] = json!(m);
    }
    if !reply.result.is_null() {
        out["result"] = reply.result;
    }
    // Protocol 1.2: returned for the caller. The interface opens the file and shows the dialog
    // when one of its buttons ran the command; programmatic callers act on them themselves.
    if let Some(o) = &reply.open {
        out["open"] = o.to_json();
    }
    if let Some(d) = &reply.dialog {
        out["dialog"] = d.to_json();
    }
    // Protocol 2: a room to join or start, for the caller (the interface runs `collab.join`).
    if let Some(j) = &reply.collab_join {
        out["collab"] = json!({"join": j});
    }
    Ok(out)
}
