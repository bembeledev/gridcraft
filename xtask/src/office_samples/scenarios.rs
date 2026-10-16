//! The scenarios: which of Microsoft's samples run, what each check expects, and the steps.
//!
//! [`SCENARIOS`] is the table: each check is either `MustPass` (GridCraft supports it: a failure
//! fails the run) or `ExpectedUnsupported` (GridCraft doesn't support it yet: the check confirms
//! it fails cleanly, and says so if it starts working). The scripts sent to the pages
//! (`office.eval`) are the constants below; read each sample's HTML and JavaScript for the ids.

use serde_json::{Value as Json, json};

use super::{Abort, Expect, Observed, Run};

/// One check of a scenario.
pub struct CheckDef {
    pub id: &'static str,
    pub expect: Expect,
    pub what: &'static str,
}

/// One sample.
pub struct Scenario {
    pub id: &'static str,
    /// The sample's folder in the samples repository.
    pub sample: &'static str,
    /// Its manifest that points at officedev.github.io; `None`: not hosted (skipped).
    pub manifest: Option<&'static str>,
    pub about: &'static str,
    pub checks: &'static [CheckDef],
    pub run: fn(&mut Run) -> Result<(), Abort>,
}

const fn must(id: &'static str, what: &'static str) -> CheckDef {
    CheckDef { id, expect: Expect::MustPass, what }
}
const fn unsupported(id: &'static str, what: &'static str) -> CheckDef {
    CheckDef { id, expect: Expect::ExpectedUnsupported, what }
}

/// Every sample loads, opens its pane and runs GridCraft's Office.js.
const LOAD: CheckDef = must("load", "the manifest loads as an Office add-in");
const PANE: CheckDef = must("pane", "the task pane opens and the library reports ready");
const OFFICE: CheckDef = must("office", "Office.context.host is Excel and Office is GridCraft's");
const NO_MS: CheckDef = must("no-ms-office-js", "Microsoft's office.js never ran (its script got the empty script)");

pub const SCENARIOS: &[Scenario] = &[
    Scenario {
        id: "excel-hello-world",
        sample: "Samples/hello-world/excel-hello-world",
        manifest: Some("Samples/hello-world/excel-hello-world/manifest.xml"),
        about: "One button writes 'Hello world!' into A1",
        checks: &[LOAD, PANE, OFFICE, NO_MS, must("writes-a1", "the button writes 'Hello world!' into A1")],
        run: hello_world,
    },
    Scenario {
        id: "excel-add-in-commands",
        sample: "Samples/office-add-in-commands/excel",
        manifest: Some("Samples/office-add-in-commands/excel/manifest.xml"),
        about: "A ribbon button opens the pane (Highlight fills the selection yellow); a menu item runs an ExecuteFunction command",
        checks: &[
            LOAD,
            PANE,
            OFFICE,
            NO_MS,
            must("highlight", "the pane's Highlight button fills the selection yellow"),
            must("execute-function", "the ExecuteFunction menu item writes its button id into the selection"),
        ],
        run: add_in_commands,
    },
    Scenario {
        id: "save-custom-settings",
        sample: "Samples/office-add-in-save-custom-settings",
        manifest: Some("Samples/office-add-in-save-custom-settings/manifest.xml"),
        about: "Saves a setting in the document's property bag, then reads it back in a reopened pane",
        checks: &[
            LOAD,
            PANE,
            OFFICE,
            NO_MS,
            must("save", "Save stores the setting (settings.saveAsync)"),
            must("read-back", "the setting reads back after closing and reopening the pane"),
        ],
        run: save_custom_settings,
    },
    Scenario {
        id: "keyboard-shortcuts",
        sample: "Samples/office-keyboard-shortcuts",
        manifest: Some("Samples/office-keyboard-shortcuts/manifest.xml"),
        about: "Shared-runtime add-in whose actions are bound to keyboard shortcuts (ExtendedOverrides)",
        checks: &[
            LOAD,
            PANE,
            OFFICE,
            NO_MS,
            must("page-starts", "the page's onReady runs (its Office.actions.associate calls don't throw)"),
            must("action-add-new", "the AddNew action adds a worksheet"),
            must("action-run-action", "the RunAction action colours the selection"),
            unsupported("shortcuts", "keyboard shortcuts (Office.actions.replaceShortcuts, ExtendedOverrides)"),
            unsupported("show-taskpane", "Office.addin.showAsTaskpane (shared runtime)"),
        ],
        run: keyboard_shortcuts,
    },
    Scenario {
        id: "first-run-experience-tutorial",
        sample: "Samples/tutorials/first-run-experience-tutorial",
        manifest: None,
        about: "Not hosted: its manifest points at a dev server on https://localhost:3000",
        checks: &[],
        run: not_hosted,
    },
    Scenario {
        id: "custom-functions-azure",
        sample: "Excel-custom-functions/AzureFunction",
        manifest: Some("Excel-custom-functions/AzureFunction/CustomFunctionProject/manifest.xml"),
        about: "Custom functions calling an Azure function; its pane highlights the selection",
        checks: CUSTOM_FUNCTION_CHECKS,
        run: custom_functions,
    },
    Scenario {
        id: "custom-functions-batching",
        sample: "Excel-custom-functions/Batching",
        manifest: Some("Excel-custom-functions/Batching/manifest.xml"),
        about: "Custom functions that batch their calls to a remote service",
        checks: CUSTOM_FUNCTION_CHECKS,
        run: custom_functions,
    },
    Scenario {
        id: "custom-functions-storage",
        sample: "Excel-custom-functions/Storage",
        manifest: Some("Excel-custom-functions/Storage/manifest.xml"),
        about: "Custom functions sharing data with the pane through OfficeRuntime.storage",
        checks: CUSTOM_FUNCTION_CHECKS,
        run: custom_functions,
    },
    Scenario {
        id: "custom-functions-web-worker",
        sample: "Excel-custom-functions/web-worker",
        manifest: Some("Excel-custom-functions/web-worker/manifest.xml"),
        about: "Custom functions computed in a web worker",
        checks: CUSTOM_FUNCTION_CHECKS,
        run: custom_functions,
    },
];

/// The custom-function samples need the shared runtime and custom functions, which GridCraft
/// doesn't have: their `<Requirements>` block them. If that changes, the pane must work.
const CUSTOM_FUNCTION_CHECKS: &[CheckDef] = &[
    unsupported("load", "the add-in loads (its manifest requires SharedRuntime / CustomFunctionsRuntime)"),
    PANE,
    OFFICE,
    NO_MS,
    unsupported("custom-functions", "custom functions (CustomFunctions.associate)"),
];

// ---------------------------------------------------------------- the scripts

/// What the page says about its `Office`.
pub const INFO: &str = "({host: Office.context.host, osf: typeof OSF, info: globalThis.__craftOfficeInfo || null})";

pub const HELLO_READY: &str = "typeof document.getElementById('helloButton').onclick === 'function'";
pub const HELLO_CLICK: &str = "document.getElementById('helloButton').click(); true";

pub const COMMANDS_READY: &str =
    "document.getElementById('app-body').style.display === 'flex' && typeof document.getElementById('run').onclick === 'function'";
pub const COMMANDS_CLICK: &str = "document.getElementById('run').click(); true";

pub const SETTING_KEY: &str = "gridcraftSampleKey";
pub const SETTING_VALUE: &str = "saved-by-gridcraft";
pub const SETTINGS_READY: &str = "typeof document.getElementById('saveSetting').onclick === 'function'";
pub const SETTINGS_SAVE: &str = "document.getElementById('setName').value = 'gridcraftSampleKey'; document.getElementById('setValue').value = 'saved-by-gridcraft'; document.getElementById('saveSetting').click(); true";
pub const SETTINGS_BANNER: &str = "document.getElementById('bannerText').innerText";
pub const SETTINGS_STORED: &str = "Office.context.document.settings.get('gridcraftSampleKey')";
pub const SETTINGS_GET: &str = "document.getElementById('getName').value = 'gridcraftSampleKey'; document.getElementById('getSetting').click(); true";

pub const SHORTCUTS_READY: &str = "document.getElementById('app-body').style.display === 'flex'";
pub const SHORTCUTS_ADD_NEW: &str = "__craftOfficeRunFunction(JSON.stringify({call: 0, name: 'AddNew', control: 'gridcraft-test'})); true";
pub const SHORTCUTS_RUN_ACTION: &str = "__craftOfficeRunFunction(JSON.stringify({call: 0, name: 'RunAction', control: 'gridcraft-test'})); true";
pub const SHORTCUTS_REPLACE: &str = "Office.actions.replaceShortcuts({}).then(function () { return 'supported'; }, function (e) { return 'unsupported: ' + (e && (e.code || e.message)); })";
pub const SHORTCUTS_SHOW: &str = "Office.addin.showAsTaskpane().then(function () { return 'supported'; }, function (e) { return 'unsupported: ' + (e && (e.code || e.message)); })";

pub const CUSTOM_FUNCTIONS: &str =
    "typeof CustomFunctions === 'object' && typeof CustomFunctions.associate === 'function' ? 'supported' : 'unsupported: no CustomFunctions'";

/// RunAction's colours (the sample cycles through them).
const ACTION_COLOURS: &[&str] = &["FFFFFF", "C7CC7A", "7560BA", "9DD9D2", "FFE1A8", "E26D5C"];

// ---------------------------------------------------------------- the steps

/// How long a page may take to load from GitHub Pages and start the library.
const READY_MS: u64 = 60_000;
/// How long a click may take to show in the workbook.
const EFFECT_MS: u64 = 15_000;

/// Loads the sample, opens its pane and checks the page's `Office`: the pane's key, or `None`
/// when a check failed (recorded) or the add-in was blocked as expected (the rest skipped).
fn open_sample(run: &mut Run, command: Option<&str>) -> Result<Option<(String, String)>, Abort> {
    let Some(manifest) = run.manifest_path() else { return Ok(None) };
    let loaded = run.call("plugins.load", json!({"path": manifest}))?;
    let plugin = match loaded {
        Ok(info) => {
            let id = info.get("id").and_then(Json::as_str).unwrap_or("").to_string();
            if info.get("state").and_then(Json::as_str) == Some("blocked") {
                let why: Vec<String> = info
                    .get("blockedBy")
                    .and_then(Json::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|b| b.get("message").and_then(Json::as_str))
                    .map(str::to_string)
                    .collect();
                let why = format!("blocked: {}", why.join("; "));
                run.record("load", Observed::Unsupported(why.clone()));
                run.skip_rest(&why);
                return Ok(None);
            }
            run.record("load", Observed::Works(format!("{id} ({})", info.get("name").and_then(Json::as_str).unwrap_or(""))));
            id
        }
        Err(e) => {
            run.record("load", Observed::Broken(e));
            return Ok(None);
        }
    };
    let mut params = json!({"plugin": plugin});
    if let Some(c) = command {
        params["command"] = json!(c);
    }
    let pane = match run.call("office.open", params)? {
        Ok(r) => r.get("pane").and_then(Json::as_str).map(str::to_string),
        Err(e) => {
            run.record("pane", Observed::Broken(format!("office.open: {e}")));
            return Ok(None);
        }
    };
    let Some(pane) = pane else {
        run.record("pane", Observed::Broken("office.open opened no pane".into()));
        return Ok(None);
    };
    if !wait_ready(run, &pane, "pane")? {
        return Ok(None);
    }
    check_office(run, &pane)?;
    Ok(Some((plugin, pane)))
}

/// Waits until `office.panes` says the page `key` is ready; records `check`.
fn wait_ready(run: &mut Run, key: &str, check: &str) -> Result<bool, Abort> {
    let started = std::time::Instant::now();
    let mut last = Json::Null;
    let ready = run.wait(READY_MS, |run| {
        let panes = run.call("office.panes", json!({}))?.unwrap_or(Json::Null);
        let ready =
            panes.as_array().into_iter().flatten().any(|p| p.get("key").and_then(Json::as_str) == Some(key) && p.get("ready") == Some(&json!(true)));
        last = panes;
        Ok(ready.then_some(()))
    })?;
    match ready {
        Some(()) => {
            run.record(check, Observed::Works(format!("ready after {:.1} s", started.elapsed().as_secs_f64())));
            Ok(true)
        }
        None => {
            run.record(check, Observed::Broken(format!("{key} not ready after {} s; panes: {}", READY_MS / 1000, clip(&last.to_string()))));
            Ok(false)
        }
    }
}

/// `office` and `no-ms-office-js`.
fn check_office(run: &mut Run, pane: &str) -> Result<(), Abort> {
    match run.eval(pane, INFO)? {
        Ok(v) => {
            let host = v.get("host").and_then(Json::as_str).unwrap_or("");
            let ours = v.pointer("/info/officeIsCraft") == Some(&json!(true));
            let detail = format!("host {host:?}, GridCraft's Office: {ours}");
            run.record("office", if host == "Excel" && ours { Observed::Works(detail) } else { Observed::Broken(detail) });
            let osf = v.get("osf").and_then(Json::as_str).unwrap_or("?");
            let blocked: Vec<&str> =
                v.pointer("/info/blockedScripts").and_then(Json::as_array).into_iter().flatten().filter_map(Json::as_str).collect();
            let detail = format!("blocked: [{}]; typeof OSF: {osf}", blocked.join(", "));
            run.record(
                "no-ms-office-js",
                if osf == "undefined" && v.get("info").is_some_and(Json::is_object) { Observed::Works(detail) } else { Observed::Broken(detail) },
            );
        }
        Err(e) => {
            run.record("office", Observed::Broken(format!("office.eval: {e}")));
            run.record("no-ms-office-js", Observed::Broken("not checked (office.eval failed)".into()));
        }
    }
    Ok(())
}

/// Waits until `script` answers `true` in the page.
fn wait_true(run: &mut Run, pane: &str, script: &str, ms: u64) -> Result<Result<(), String>, Abort> {
    let mut last = String::new();
    let done = run.wait(ms, |run| {
        Ok(match run.eval(pane, script)? {
            Ok(Json::Bool(true)) => Some(()),
            Ok(v) => {
                last = v.to_string();
                None
            }
            Err(e) => {
                last = e;
                None
            }
        })
    })?;
    Ok(done.ok_or_else(|| format!("`{script}` wasn't true after {} s (last: {})", ms / 1000, clip(&last))))
}

/// Runs `script` and records `check` as broken if it fails: whether it ran.
fn click(run: &mut Run, pane: &str, script: &str, check: &str) -> Result<bool, Abort> {
    match run.eval(pane, script)? {
        Ok(_) => Ok(true),
        Err(e) => {
            run.record(check, Observed::Broken(format!("the click failed: {e}")));
            Ok(false)
        }
    }
}

/// Waits until `cell.get` of `cell` satisfies `ok`; the last answer.
fn wait_cell(run: &mut Run, cell: &str, ms: u64, ok: impl Fn(&Json) -> bool) -> Result<Result<Json, Json>, Abort> {
    let mut last = Json::Null;
    let done = run.wait(ms, |run| {
        let v = run.call("cell.get", json!({"cell": cell}))?.unwrap_or(Json::Null);
        let hit = ok(&v);
        last = v;
        Ok(hit.then_some(()))
    })?;
    Ok(if done.is_some() { Ok(last) } else { Err(last) })
}

fn fill_has(cell: &Json, colour: &str) -> bool {
    cell.pointer("/style/fill").is_some_and(|f| f.to_string().to_ascii_uppercase().contains(colour))
}

fn clip(s: &str) -> String {
    let mut out: String = s.chars().take(300).collect();
    if out.len() < s.len() {
        out.push('…');
    }
    out
}

fn hello_world(run: &mut Run) -> Result<(), Abort> {
    let Some((_, pane)) = open_sample(run, None)? else { return Ok(()) };
    if let Err(e) = wait_true(run, &pane, HELLO_READY, EFFECT_MS)? {
        run.record("writes-a1", Observed::Broken(e));
        return run.screenshot("pane");
    }
    if click(run, &pane, HELLO_CLICK, "writes-a1")? {
        let got = wait_cell(run, "A1", EFFECT_MS, |c| c.get("value") == Some(&json!("Hello world!")))?;
        run.record(
            "writes-a1",
            match got {
                Ok(_) => Observed::Works("A1 = \"Hello world!\"".into()),
                Err(c) => Observed::Broken(format!("A1 is {}", c.get("value").cloned().unwrap_or(Json::Null))),
            },
        );
    }
    run.screenshot("after-click")
}

fn add_in_commands(run: &mut Run) -> Result<(), Abort> {
    // The ribbon button: the manifest's default SourceLocation points at a page that isn't there.
    let Some((plugin, pane)) = open_sample(run, Some("Contoso.TaskpaneButton"))? else { return Ok(()) };
    let _ = run.call("selection.set", json!({"range": "B2:C3"}))?;
    match wait_true(run, &pane, COMMANDS_READY, EFFECT_MS)? {
        Err(e) => run.record("highlight", Observed::Broken(e)),
        Ok(()) => {
            if click(run, &pane, COMMANDS_CLICK, "highlight")? {
                let got = wait_cell(run, "C3", EFFECT_MS, |c| fill_has(c, "FFFF00"))?;
                let b2 = run.call("cell.get", json!({"cell": "B2"}))?.unwrap_or(Json::Null);
                run.record(
                    "highlight",
                    match got {
                        Ok(_) if fill_has(&b2, "FFFF00") => Observed::Works("B2:C3 filled #FFFF00".into()),
                        Ok(_) => Observed::Broken(format!("C3 is yellow, B2's fill is {}", b2.pointer("/style/fill").cloned().unwrap_or(Json::Null))),
                        Err(c) => Observed::Broken(format!("C3's fill is {}", c.pointer("/style/fill").cloned().unwrap_or(Json::Null))),
                    },
                );
            }
        }
    }
    run.screenshot("highlight")?;
    // The menu item: ExecuteFunction in the hidden function page.
    let _ = run.call("selection.set", json!({"range": "E5"}))?;
    match run.call("plugins.run", json!({"plugin": plugin, "command": "itemExecuteFunction"}))? {
        Err(e) => run.record("execute-function", Observed::Broken(format!("plugins.run: {e}"))),
        Ok(_) => {
            let want = json!("ExecuteFunction works. Button ID=itemExecuteFunction");
            let got = wait_cell(run, "E5", READY_MS, |c| c.get("value") == Some(&want))?;
            let pages = run.call("office.panes", json!({}))?.unwrap_or(Json::Null);
            let function_ready =
                pages.as_array().into_iter().flatten().any(|p| p.get("kind") == Some(&json!("function")) && p.get("ready") == Some(&json!(true)));
            run.record(
                "execute-function",
                match got {
                    Ok(_) => Observed::Works(format!("E5 = {want} (function page ready: {function_ready})")),
                    Err(c) => {
                        Observed::Broken(format!("E5 is {} (function page ready: {function_ready})", c.get("value").cloned().unwrap_or(Json::Null)))
                    }
                },
            );
        }
    }
    run.screenshot("execute-function")
}

fn save_custom_settings(run: &mut Run) -> Result<(), Abort> {
    let Some((plugin, pane)) = open_sample(run, None)? else { return Ok(()) };
    if let Err(e) = wait_true(run, &pane, SETTINGS_READY, EFFECT_MS)? {
        run.record("save", Observed::Broken(e));
        return run.screenshot("pane");
    }
    if !click(run, &pane, SETTINGS_SAVE, "save")? {
        return run.screenshot("pane");
    }
    let banner = wait_text(run, &pane, SETTINGS_BANNER, "Saved setting")?;
    let stored = run.eval(&pane, SETTINGS_STORED)?;
    match (banner, stored) {
        (Ok(b), Ok(v)) if v == json!(SETTING_VALUE) => run.record("save", Observed::Works(b)),
        (Ok(b), v) => run.record("save", Observed::Broken(format!("{b}, but settings.get answers {v:?}"))),
        (Err(e), _) => run.record("save", Observed::Broken(e)),
    }
    run.screenshot("saved")?;
    // Close the pane, open it again: a new page, which reads the settings from GridCraft.
    if let Err(e) = run.call("office.close", json!({"pane": pane}))? {
        run.record("read-back", Observed::Broken(format!("office.close: {e}")));
        return Ok(());
    }
    let pane = match run.call("office.open", json!({"plugin": plugin}))? {
        Ok(r) => r.get("pane").and_then(Json::as_str).unwrap_or("").to_string(),
        Err(e) => {
            run.record("read-back", Observed::Broken(format!("office.open: {e}")));
            return Ok(());
        }
    };
    if !wait_ready(run, &pane, "read-back")? {
        return run.screenshot("reopened");
    }
    if let Err(e) = wait_true(run, &pane, SETTINGS_READY, EFFECT_MS)? {
        run.record("read-back", Observed::Broken(e));
        return run.screenshot("reopened");
    }
    if click(run, &pane, SETTINGS_GET, "read-back")? {
        let want = format!("VALUE: {SETTING_VALUE}");
        let got = wait_text(run, &pane, SETTINGS_BANNER, &want)?;
        run.record(
            "read-back",
            match got {
                Ok(b) => Observed::Works(b),
                Err(e) => Observed::Broken(e),
            },
        );
    }
    run.screenshot("read-back")
}

/// Waits until the text `script` answers contains `want`.
fn wait_text(run: &mut Run, pane: &str, script: &str, want: &str) -> Result<Result<String, String>, Abort> {
    let mut last = String::new();
    let done = run.wait(EFFECT_MS, |run| {
        Ok(match run.eval(pane, script)? {
            Ok(v) => {
                last = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
                last.contains(want).then_some(())
            }
            Err(e) => {
                last = e;
                None
            }
        })
    })?;
    Ok(if done.is_some() { Ok(last) } else { Err(format!("no {want:?} after {} s (last: {})", EFFECT_MS / 1000, clip(&last))) })
}

/// A probe of an API GridCraft lacks: the script answers (maybe through a Promise)
/// `supported` or `unsupported: …`; throwing instead is broken.
fn probe(run: &mut Run, pane: &str, check: &str, script: &str) -> Result<(), Abort> {
    let observed = match run.eval(pane, script)? {
        Ok(v) => match v.as_str() {
            Some("supported") => Observed::Works("supported".into()),
            Some(s) if s.starts_with("unsupported") => Observed::Unsupported(s.to_string()),
            _ => Observed::Broken(format!("unexpected answer {v}")),
        },
        Err(e) => Observed::Broken(format!("threw instead of failing cleanly: {e}")),
    };
    run.record(check, observed);
    Ok(())
}

fn keyboard_shortcuts(run: &mut Run) -> Result<(), Abort> {
    let Some((_, pane)) = open_sample(run, None)? else { return Ok(()) };
    let started = wait_true(run, &pane, SHORTCUTS_READY, EFFECT_MS)?;
    run.record(
        "page-starts",
        match &started {
            Ok(()) => Observed::Works("#app-body shows".into()),
            Err(e) => Observed::Broken(e.clone()),
        },
    );
    // The actions run as a shortcut would run them: the library calls the associated function.
    let sheets = |run: &mut Run| -> Result<usize, Abort> {
        Ok(run.call("document.inspect", json!({}))?.ok().and_then(|d| d.get("sheets").and_then(Json::as_array).map(Vec::len)).unwrap_or(0))
    };
    let before = sheets(run)?;
    if click(run, &pane, SHORTCUTS_ADD_NEW, "action-add-new")? {
        let mut now = before;
        let added = run.wait(EFFECT_MS, |run| {
            now = sheets(run)?;
            Ok((now > before).then_some(()))
        })?;
        run.record(
            "action-add-new",
            match added {
                Some(()) => Observed::Works(format!("{before} → {now} worksheets")),
                None => Observed::Broken(format!("still {now} worksheets")),
            },
        );
    }
    let _ = run.call("selection.set", json!({"range": "B2"}))?;
    if click(run, &pane, SHORTCUTS_RUN_ACTION, "action-run-action")? {
        let got = wait_cell(run, "B2", EFFECT_MS, |c| ACTION_COLOURS.iter().any(|col| fill_has(c, col)))?;
        run.record(
            "action-run-action",
            match got {
                Ok(c) => Observed::Works(format!("B2's fill is {}", c.pointer("/style/fill").cloned().unwrap_or(Json::Null))),
                Err(c) => Observed::Broken(format!("B2's fill is {}", c.pointer("/style/fill").cloned().unwrap_or(Json::Null))),
            },
        );
    }
    probe(run, &pane, "shortcuts", SHORTCUTS_REPLACE)?;
    probe(run, &pane, "show-taskpane", SHORTCUTS_SHOW)?;
    run.screenshot("pane")
}

fn custom_functions(run: &mut Run) -> Result<(), Abort> {
    let Some((_, pane)) = open_sample(run, None)? else { return Ok(()) };
    probe(run, &pane, "custom-functions", CUSTOM_FUNCTIONS)?;
    run.screenshot("pane")
}

fn not_hosted(_run: &mut Run) -> Result<(), Abort> {
    Ok(())
}
