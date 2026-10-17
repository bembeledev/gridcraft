//! `cargo xtask office-samples`: Microsoft's Office add-in samples, loaded from their hosted
//! pages (officedev.github.io) into the real desktop app's task pane, driven through the control
//! channel (docs/officejs.md, "Testing with Microsoft's samples").
//!
//! For each scenario ([`scenarios::SCENARIOS`]) a fresh app starts (`--control <port>`, its own
//! settings folder, a blank workbook); the driver loads the sample's manifest, opens its pane,
//! checks that the page runs GridCraft's Office.js (not Microsoft's), clicks the sample's buttons
//! with `office.eval`, and checks the workbook with engine queries. It writes screenshots,
//! `results.json` and `summary.md` (a table for `$GITHUB_STEP_SUMMARY`) into `--out`. A failing
//! scenario never stops the others; an app that crashes is reported, and the next scenario gets a
//! fresh one. The samples are never copied into the repository: CI clones them at a pinned commit.

// The driver runs on desktops only (never wasm), so it reads the clock.
#![allow(clippy::disallowed_methods)]

pub mod app;
pub mod scenarios;

use std::path::{Path, PathBuf};

use serde_json::{Value as Json, json};

use app::{App, CallError, FakeLauncher, Launcher, ProcessLauncher};
use scenarios::{SCENARIOS, Scenario};

pub const USAGE: &str = "\
usage: cargo xtask office-samples --app <gridcraft binary> --samples <Office-Add-in-samples checkout> --out <dir>
                                  [--office-js <dir>] [--only id,id] [--dry-run] [--list]

  --app        the desktop app to start (with --control <port>, on a fresh settings folder)
  --samples    a checkout of https://github.com/OfficeDev/Office-Add-in-samples (manifests are read
               from it; the pages load from officedev.github.io)
  --out        screenshots, results.json, summary.md, app logs
  --office-js  the built Office.js library (cargo xtask office-js), passed as GRIDCRAFT_OFFICE_JS
  --only       run these scenarios only
  --dry-run    drive a fake app instead (no web view, no network): checks the driver itself
  --list       print the scenarios and their expectations
Exit status 1 if a scenario failed.";

/// What a check expects of GridCraft.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expect {
    /// Supported: it has to work.
    MustPass,
    /// Not supported yet: it has to fail cleanly (an error the page can handle, a blocked add-in).
    ExpectedUnsupported,
}

/// What a check saw.
#[derive(Clone, Debug, PartialEq)]
pub enum Observed {
    Works(String),
    /// Failed the way an unsupported feature fails (NotImplemented, blocked).
    Unsupported(String),
    /// Anything else: a wrong result, a timeout, an exception.
    Broken(String),
}

/// A check's verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pass,
    Fail,
    ExpectedUnsupported,
    /// Expected unsupported, but it worked: the table needs updating (not a failure).
    UnexpectedlyWorks,
    /// Not reached (an earlier step failed or the app crashed).
    NotRun,
    /// Not applicable (the add-in was blocked as expected, or the sample isn't hosted).
    Skipped,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Pass => "pass",
            Status::Fail => "FAIL",
            Status::ExpectedUnsupported => "expected unsupported",
            Status::UnexpectedlyWorks => "works (expected unsupported)",
            Status::NotRun => "not run",
            Status::Skipped => "skipped",
        }
    }
}

pub fn verdict(expect: Expect, seen: &Observed) -> Status {
    match (expect, seen) {
        (Expect::MustPass, Observed::Works(_)) => Status::Pass,
        (Expect::MustPass, _) => Status::Fail,
        (Expect::ExpectedUnsupported, Observed::Works(_)) => Status::UnexpectedlyWorks,
        (Expect::ExpectedUnsupported, Observed::Unsupported(_)) => Status::ExpectedUnsupported,
        (Expect::ExpectedUnsupported, Observed::Broken(_)) => Status::Fail,
    }
}

/// Stops a scenario.
#[derive(Debug)]
pub enum Abort {
    /// The app crashed or hung.
    Crashed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CheckResult {
    pub id: String,
    pub what: String,
    pub expect: Expect,
    pub status: Status,
    pub detail: String,
}

/// One scenario's outcome.
#[derive(Clone, Debug, PartialEq)]
pub struct ScenarioResult {
    pub id: String,
    pub sample: String,
    pub about: String,
    pub status: Status,
    pub checks: Vec<CheckResult>,
    pub crashed: Option<String>,
    pub screenshots: Vec<String>,
    pub seconds: f64,
}

/// A scenario in progress: the app, and what has been seen.
pub struct Run<'a> {
    app: &'a mut dyn App,
    scenario: &'static Scenario,
    samples: Option<&'a Path>,
    out: &'a Path,
    poll_ms: u64,
    checks: Vec<CheckResult>,
    screenshots: Vec<String>,
    skipped: Option<String>,
    diagnostics: usize,
}

/// The gist of a page's diagnostics: its state, the library's `onReady` bookkeeping, what failed.
pub fn short_diagnosis(v: &Json, file: &str) -> String {
    let info = v.get("info").unwrap_or(&Json::Null);
    let lib = info.get("libraryState").unwrap_or(&Json::Null);
    let mut parts = vec![format!("readyState {}", v.get("readyState").and_then(Json::as_str).unwrap_or("?"))];
    if info.is_null() {
        parts.push("no GridCraft bootstrap in the page".into());
    } else {
        parts.push(format!("library {}", if info.get("library") == Some(&json!(true)) { "started" } else { "not started" }));
        if let Some(e) = info.get("startError").and_then(Json::as_str) {
            parts.push(format!("start error {e}"));
        }
        parts.push(format!(
            "onReady {}/{} done{}",
            lib.get("onReadyDone").and_then(Json::as_u64).unwrap_or(0),
            lib.get("onReadyCalls").and_then(Json::as_u64).unwrap_or(0),
            lib.get("onReadyError").and_then(Json::as_str).map(|e| format!(" (callback threw: {e})")).unwrap_or_default()
        ));
        let failed: Vec<&str> = info
            .get("scripts")
            .and_then(Json::as_array)
            .into_iter()
            .flatten()
            .filter(|s| s.get("state") == Some(&json!("failed")))
            .filter_map(|s| s.get("src").and_then(Json::as_str))
            .collect();
        if !failed.is_empty() {
            parts.push(format!("scripts failed: {}", failed.join(", ")));
        }
        let problems: Vec<String> = info
            .get("problems")
            .and_then(Json::as_array)
            .into_iter()
            .flatten()
            .filter_map(Json::as_str)
            .take(3)
            .map(|p| p.chars().take(200).collect())
            .collect();
        if !problems.is_empty() {
            parts.push(format!("problems: {}", problems.join(" | ")));
        }
    }
    parts.push(format!("details in diagnostics/{file}"));
    parts.join("; ")
}

/// Polls happen every 250 ms of real time; a timeout is that many polls.
const POLL_MS: u64 = 250;

impl Run<'_> {
    /// A control-channel call: `Err(Abort)` if the app is gone, else its answer.
    pub fn call(&mut self, method: &str, params: Json) -> Result<Result<Json, String>, Abort> {
        match self.app.call(method, params) {
            Ok(v) => Ok(Ok(v)),
            Err(CallError::Answer(e)) => Ok(Err(e)),
            Err(CallError::Crashed(e)) => Err(Abort::Crashed(e)),
        }
    }

    /// `office.eval` in `pane`.
    pub fn eval(&mut self, pane: &str, script: &str) -> Result<Result<Json, String>, Abort> {
        self.call("office.eval", json!({"pane": pane, "script": script, "timeoutMs": 10_000}))
    }

    /// Calls `f` until it answers `Some`, at most `ms` milliseconds of polls.
    pub fn wait<T>(&mut self, ms: u64, mut f: impl FnMut(&mut Self) -> Result<Option<T>, Abort>) -> Result<Option<T>, Abort> {
        let polls = (ms / POLL_MS).max(1);
        for i in 0..polls {
            if let Some(v) = f(self)? {
                return Ok(Some(v));
            }
            if i + 1 < polls && self.poll_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(self.poll_ms));
            }
        }
        Ok(None)
    }

    /// The sample's manifest, in the samples checkout (as written when there is none: dry runs).
    pub fn manifest_path(&self) -> Option<String> {
        let m = self.scenario.manifest?;
        Some(match self.samples {
            Some(dir) => dir.join(m).display().to_string(),
            None => m.to_string(),
        })
    }

    /// Records what check `id` saw (a later record of the same check replaces it).
    pub fn record(&mut self, id: &str, seen: Observed) {
        let (expect, what) = match self.scenario.checks.iter().find(|c| c.id == id) {
            Some(c) => (c.expect, c.what.to_string()),
            None => (Expect::MustPass, format!("(check `{id}` isn't in the scenario table)")),
        };
        let status = if self.scenario.checks.iter().any(|c| c.id == id) { verdict(expect, &seen) } else { Status::Fail };
        let detail = match seen {
            Observed::Works(d) | Observed::Unsupported(d) | Observed::Broken(d) => d,
        };
        self.checks.retain(|c| c.id != id);
        self.checks.push(CheckResult { id: id.to_string(), what, expect, status, detail });
    }

    /// The checks not recorded yet are skipped, for `why`.
    pub fn skip_rest(&mut self, why: &str) {
        self.skipped = Some(why.to_string());
    }

    /// What the page `pane` says when something didn't happen: the whole answer goes to
    /// `diagnostics/<scenario>-<n>.json`, a short version is returned for the report.
    pub fn diagnose(&mut self, pane: &str) -> Result<String, Abort> {
        let v = match self.eval(pane, scenarios::DIAGNOSE)? {
            Ok(v) => v,
            Err(e) => return Ok(format!("no diagnostics ({e})")),
        };
        let dir = self.out.join("diagnostics");
        let n = self.diagnostics;
        self.diagnostics += 1;
        let file = format!("{}-{n}.json", self.scenario.id);
        if std::fs::create_dir_all(&dir).is_ok() {
            let _ = std::fs::write(dir.join(&file), serde_json::to_string_pretty(&v).unwrap_or_default());
        }
        Ok(short_diagnosis(&v, &file))
    }

    /// A screenshot into `--out`; a failure is only noted.
    pub fn screenshot(&mut self, name: &str) -> Result<(), Abort> {
        let file = format!("{}-{name}.png", self.scenario.id);
        let path = self.out.join("screenshots").join(&file);
        match self.call("ui.screenshot", json!({"path": path.display().to_string()}))? {
            Ok(_) => self.screenshots.push(format!("screenshots/{file}")),
            Err(e) => eprintln!("office-samples: {}: screenshot {name}: {e}", self.scenario.id),
        }
        Ok(())
    }
}

/// Runs one scenario on `app`: its result, and whether the app is still usable.
pub fn run_scenario(app: &mut dyn App, scenario: &'static Scenario, samples: Option<&Path>, out: &Path, poll_ms: u64) -> ScenarioResult {
    let started = std::time::Instant::now();
    let mut run = Run { app, scenario, samples, out, poll_ms, checks: Vec::new(), screenshots: Vec::new(), skipped: None, diagnostics: 0 };
    let mut crashed = None;
    if scenario.manifest.is_none() {
        run.skip_rest(scenario.about);
    } else if let Err(Abort::Crashed(e)) = (scenario.run)(&mut run) {
        crashed = Some(e);
    }
    // Checks in the table's order; those never recorded are skipped or not run.
    let mut checks = Vec::new();
    for def in scenario.checks {
        match run.checks.iter().position(|c| c.id == def.id) {
            Some(i) => checks.push(run.checks.remove(i)),
            None => checks.push(CheckResult {
                id: def.id.into(),
                what: def.what.into(),
                expect: def.expect,
                status: if run.skipped.is_some() && crashed.is_none() { Status::Skipped } else { Status::NotRun },
                detail: match (&crashed, &run.skipped) {
                    (Some(e), _) => format!("the app crashed: {e}"),
                    (None, Some(why)) => why.clone(),
                    (None, None) => "an earlier step failed".into(),
                },
            }),
        }
    }
    checks.append(&mut run.checks); // Checks missing from the table (already FAIL).
    let status = if scenario.manifest.is_none() {
        Status::Skipped
    } else if crashed.is_some() || checks.iter().any(|c| c.status == Status::Fail) {
        Status::Fail
    } else if checks.iter().any(|c| matches!(c.status, Status::ExpectedUnsupported | Status::Skipped)) {
        Status::ExpectedUnsupported
    } else {
        Status::Pass
    };
    ScenarioResult {
        id: scenario.id.into(),
        sample: scenario.sample.into(),
        about: scenario.about.into(),
        status,
        checks,
        crashed,
        screenshots: run.screenshots,
        seconds: started.elapsed().as_secs_f64(),
    }
}

/// Runs `scenarios`, each on a fresh app from `launcher`.
pub fn run_all(launcher: &mut dyn Launcher, scenarios: &[&'static Scenario], samples: Option<&Path>, out: &Path) -> Vec<ScenarioResult> {
    let mut results = Vec::new();
    for (n, scenario) in scenarios.iter().enumerate() {
        eprintln!("office-samples: {} …", scenario.id);
        if scenario.manifest.is_none() {
            results.push(run_scenario(&mut NoApp, scenario, samples, out, 0));
            continue;
        }
        let result = match launcher.launch(n + 1) {
            Ok(mut app) => {
                let mut r = run_scenario(app.as_mut(), scenario, samples, out, launcher.poll_ms());
                if let Some(note) = app.shutdown()
                    && r.crashed.is_none()
                {
                    r.crashed = Some(note);
                    r.status = Status::Fail;
                }
                r
            }
            Err(e) => {
                let mut r = run_scenario(&mut NoApp, scenario, samples, out, 0);
                r.status = Status::Fail;
                r.crashed = Some(format!("the app didn't start: {e}"));
                for c in &mut r.checks {
                    c.status = Status::NotRun;
                    c.detail = "the app didn't start".into();
                }
                r
            }
        };
        eprintln!("office-samples: {}: {}", scenario.id, result.status.label());
        results.push(result);
    }
    results
}

/// For scenarios that need no app (not hosted) and apps that didn't start.
struct NoApp;

impl App for NoApp {
    fn call(&mut self, _method: &str, _params: Json) -> Result<Json, CallError> {
        Err(CallError::Crashed("no app".into()))
    }
    fn shutdown(&mut self) -> Option<String> {
        None
    }
}

// ---------------------------------------------------------------- reports

fn expect_label(e: Expect) -> &'static str {
    match e {
        Expect::MustPass => "must pass",
        Expect::ExpectedUnsupported => "expected unsupported",
    }
}

pub fn results_json(results: &[ScenarioResult], platform: &str, samples_commit: Option<&str>) -> Json {
    json!({
        "platform": platform,
        "samplesCommit": samples_commit,
        "passed": !results.iter().any(|r| r.status == Status::Fail),
        "scenarios": results.iter().map(|r| json!({
            "id": r.id,
            "sample": r.sample,
            "about": r.about,
            "status": r.status.label(),
            "crashed": r.crashed,
            "seconds": (r.seconds * 10.0).round() / 10.0,
            "screenshots": r.screenshots,
            "checks": r.checks.iter().map(|c| json!({
                "id": c.id, "what": c.what, "expect": expect_label(c.expect), "status": c.status.label(), "detail": c.detail,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

/// Markdown table cells: no pipes or line breaks.
fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

/// The summary: one row per scenario, then each scenario's checks.
pub fn summary_markdown(results: &[ScenarioResult], platform: &str, samples_commit: Option<&str>) -> String {
    let mut md = format!("## Office add-in samples on {platform}\n\n");
    if let Some(c) = samples_commit {
        md.push_str(&format!(
            "Samples: [OfficeDev/Office-Add-in-samples@{short}](https://github.com/OfficeDev/Office-Add-in-samples/tree/{c})\n\n",
            short = c.get(..10).unwrap_or(c)
        ));
    }
    md.push_str("| Scenario | Platform | Result | Details |\n|---|---|---|---|\n");
    for r in results {
        let mut details: Vec<String> = Vec::new();
        if let Some(e) = &r.crashed {
            details.push(format!("app crashed: {e}"));
        }
        for c in r.checks.iter().filter(|c| !matches!(c.status, Status::Pass | Status::Skipped)) {
            details.push(format!("{}: {} ({})", c.id, c.status.label(), c.detail));
        }
        if details.is_empty() {
            details.push(format!("{} checks passed", r.checks.len()));
        }
        if r.checks.is_empty() && r.status == Status::Skipped {
            details = vec![r.about.clone()];
        }
        md.push_str(&format!("| {} | {} | {} | {} |\n", cell(&r.id), cell(platform), r.status.label(), cell(&details.join("; "))));
    }
    for r in results.iter().filter(|r| !r.checks.is_empty()) {
        md.push_str(&format!(
            "\n### {} ({})\n\n{}\n\n| Check | Expect | Result | Detail |\n|---|---|---|---|\n",
            r.id,
            r.status.label(),
            cell(&r.about)
        ));
        for c in &r.checks {
            md.push_str(&format!(
                "| {}: {} | {} | {} | {} |\n",
                cell(&c.id),
                cell(&c.what),
                expect_label(c.expect),
                c.status.label(),
                cell(&c.detail)
            ));
        }
        if !r.screenshots.is_empty() {
            md.push_str(&format!("\nScreenshots (in the artifact): {}\n", r.screenshots.join(", ")));
        }
    }
    md
}

/// `--list`: the table.
pub fn list_markdown() -> String {
    let mut md = String::from("| Scenario | Manifest | Check | Expect |\n|---|---|---|---|\n");
    for s in SCENARIOS {
        if s.checks.is_empty() {
            md.push_str(&format!("| {} | {} | | skipped: {} |\n", s.id, s.manifest.unwrap_or("(not hosted)"), s.about));
        }
        for c in s.checks {
            md.push_str(&format!("| {} | {} | {}: {} | {} |\n", s.id, s.manifest.unwrap_or(""), c.id, c.what, expect_label(c.expect)));
        }
    }
    md
}

// ---------------------------------------------------------------- the command

#[derive(Debug, Default, PartialEq)]
pub struct Options {
    pub app: Option<PathBuf>,
    pub samples: Option<PathBuf>,
    pub out: Option<PathBuf>,
    pub office_js: Option<PathBuf>,
    pub only: Vec<String>,
    pub dry_run: bool,
    pub list: bool,
}

pub fn parse_args(args: &[&str]) -> Result<Options, String> {
    let mut o = Options::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| it.next().map(|v| v.to_string()).ok_or_else(|| format!("{name} needs a value\n\n{USAGE}"));
        match *a {
            "--app" => o.app = Some(value(a)?.into()),
            "--samples" => o.samples = Some(value(a)?.into()),
            "--out" => o.out = Some(value(a)?.into()),
            "--office-js" => o.office_js = Some(value(a)?.into()),
            "--only" => o.only.extend(value(a)?.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty())),
            "--dry-run" => o.dry_run = true,
            "--list" => o.list = true,
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument `{other}`\n\n{USAGE}")),
        }
    }
    if !o.list {
        if o.out.is_none() {
            return Err(format!("missing --out\n\n{USAGE}"));
        }
        if !o.dry_run && (o.app.is_none() || o.samples.is_none()) {
            return Err(format!("missing --app or --samples (or use --dry-run)\n\n{USAGE}"));
        }
    }
    for id in &o.only {
        if !SCENARIOS.iter().any(|s| s.id == id) {
            return Err(format!("no scenario `{id}` (see --list)"));
        }
    }
    Ok(o)
}

pub fn platform() -> String {
    let os = match std::env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    };
    format!("{os} {}", std::env::consts::ARCH)
}

fn samples_commit(dir: &Path) -> Option<String> {
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(["rev-parse", "HEAD"]).output().ok()?;
    let sha = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !sha.is_empty()).then_some(sha)
}

/// Runs the driver; `Ok(false)` if a scenario failed.
pub fn run(args: &[&str]) -> Result<bool, String> {
    let o = parse_args(args)?;
    if o.list {
        print!("{}", list_markdown());
        return Ok(true);
    }
    let out = o.out.clone().ok_or("missing --out")?;
    std::fs::create_dir_all(out.join("screenshots")).map_err(|e| format!("{}: {e}", out.display()))?;
    let out = std::fs::canonicalize(&out).unwrap_or(out);
    let samples = o.samples.as_ref().map(|s| std::fs::canonicalize(s).unwrap_or_else(|_| s.clone()));
    if let Some(s) = &samples
        && !s.join("Samples").is_dir()
    {
        return Err(format!("{}: not a checkout of OfficeDev/Office-Add-in-samples (no Samples folder)", s.display()));
    }
    let chosen: Vec<&'static Scenario> = SCENARIOS.iter().filter(|s| o.only.is_empty() || o.only.iter().any(|id| id == s.id)).collect();
    let mut launcher: Box<dyn Launcher> = if o.dry_run {
        Box::new(FakeLauncher::default())
    } else {
        let app = o.app.clone().ok_or("missing --app")?;
        let app = std::fs::canonicalize(&app).map_err(|e| format!("{}: {e}", app.display()))?;
        Box::new(ProcessLauncher {
            app,
            out: out.clone(),
            office_js: o.office_js.as_ref().map(|d| std::fs::canonicalize(d).unwrap_or_else(|_| d.clone())),
        })
    };
    let results = run_all(launcher.as_mut(), &chosen, samples.as_deref(), &out);
    let mut platform = platform();
    if o.dry_run {
        platform.push_str(" (dry run)");
    }
    let commit = samples.as_deref().and_then(samples_commit);
    let json = results_json(&results, &platform, commit.as_deref());
    let text = serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?;
    std::fs::write(out.join("results.json"), text).map_err(|e| format!("results.json: {e}"))?;
    let md = summary_markdown(&results, &platform, commit.as_deref());
    std::fs::write(out.join("summary.md"), &md).map_err(|e| format!("summary.md: {e}"))?;
    println!("{md}");
    println!("office-samples: results in {}", out.display());
    Ok(!results.iter().any(|r| r.status == Status::Fail))
}

#[cfg(test)]
mod tests;
