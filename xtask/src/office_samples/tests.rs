use super::app::{FakeLauncher, parse_answer};
use super::*;

fn out_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gridcraft-office-samples-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("screenshots")).unwrap();
    dir
}

fn all() -> Vec<&'static Scenario> {
    SCENARIOS.iter().collect()
}

fn status_of<'a>(results: &'a [ScenarioResult], id: &str) -> &'a ScenarioResult {
    results.iter().find(|r| r.id == id).unwrap()
}

#[test]
fn the_table_is_consistent() {
    let mut ids = std::collections::BTreeSet::new();
    for s in SCENARIOS {
        assert!(ids.insert(s.id), "scenario ids are unique: {}", s.id);
        let mut checks = std::collections::BTreeSet::new();
        for c in s.checks {
            assert!(checks.insert(c.id), "{}: check ids are unique: {}", s.id, c.id);
        }
        match s.manifest {
            Some(m) => {
                assert!(m.starts_with(s.sample), "{}: the manifest is in the sample's folder", s.id);
                assert!(m.ends_with("manifest.xml") && !m.contains("localhost"), "{}: the hosted manifest", s.id);
                assert!(!s.checks.is_empty());
            }
            None => assert!(s.checks.is_empty(), "{}: a sample that isn't hosted has no checks", s.id),
        }
    }
    // What the task asked for: hosted samples must pass; the unsupported features say so.
    let expect = |s: &str, c: &str| SCENARIOS.iter().find(|x| x.id == s).unwrap().checks.iter().find(|x| x.id == c).unwrap().expect;
    assert_eq!(expect("excel-hello-world", "writes-a1"), Expect::MustPass);
    assert_eq!(expect("excel-add-in-commands", "execute-function"), Expect::MustPass);
    assert_eq!(expect("save-custom-settings", "read-back"), Expect::MustPass);
    assert_eq!(expect("keyboard-shortcuts", "shortcuts"), Expect::ExpectedUnsupported);
    assert_eq!(expect("custom-functions-batching", "custom-functions"), Expect::ExpectedUnsupported);
    assert_eq!(expect("custom-functions-storage", "load"), Expect::ExpectedUnsupported);
}

/// The scenario table names the hosted manifests of a samples checkout, when
/// `GRIDCRAFT_OFFICE_SAMPLES` names one (CI's driver run reads them anyway).
#[test]
fn hosted_manifests_exist_in_a_local_checkout() {
    let Some(dir) = std::env::var_os("GRIDCRAFT_OFFICE_SAMPLES").map(PathBuf::from) else {
        return;
    };
    for s in SCENARIOS {
        if let Some(m) = s.manifest {
            let text = std::fs::read_to_string(dir.join(m)).unwrap_or_else(|e| panic!("{m}: {e}"));
            assert!(text.contains("https://officedev.github.io/Office-Add-in-samples/"), "{m} is the hosted variant");
            assert!(text.contains("Workbook"), "{m} is for Excel");
        }
    }
}

#[test]
fn verdicts() {
    assert_eq!(verdict(Expect::MustPass, &Observed::Works("x".into())), Status::Pass);
    assert_eq!(verdict(Expect::MustPass, &Observed::Unsupported("x".into())), Status::Fail);
    assert_eq!(verdict(Expect::MustPass, &Observed::Broken("x".into())), Status::Fail);
    assert_eq!(verdict(Expect::ExpectedUnsupported, &Observed::Unsupported("x".into())), Status::ExpectedUnsupported);
    assert_eq!(verdict(Expect::ExpectedUnsupported, &Observed::Works("x".into())), Status::UnexpectedlyWorks);
    assert_eq!(verdict(Expect::ExpectedUnsupported, &Observed::Broken("TypeError".into())), Status::Fail, "unsupported must fail cleanly");
}

#[test]
fn a_dry_run_passes_every_hosted_scenario_and_reports_the_unsupported_ones() {
    let out = out_dir("dry");
    let results = run_all(&mut FakeLauncher::default(), &all(), None, &out);
    assert_eq!(results.len(), SCENARIOS.len());
    for id in ["excel-hello-world", "excel-add-in-commands", "save-custom-settings"] {
        let r = status_of(&results, id);
        assert_eq!(r.status, Status::Pass, "{id}: {:?}", r.checks);
        assert!(r.checks.iter().all(|c| c.status == Status::Pass));
        assert!(!r.screenshots.is_empty());
        for s in &r.screenshots {
            assert!(out.join(s).is_file(), "{s}");
        }
    }
    let k = status_of(&results, "keyboard-shortcuts");
    assert_eq!(k.status, Status::ExpectedUnsupported, "{:?}", k.checks);
    assert!(k.checks.iter().filter(|c| c.expect == Expect::MustPass).all(|c| c.status == Status::Pass));
    assert_eq!(k.checks.iter().find(|c| c.id == "shortcuts").unwrap().status, Status::ExpectedUnsupported);
    let cf = status_of(&results, "custom-functions-web-worker");
    assert_eq!(cf.status, Status::ExpectedUnsupported);
    assert_eq!(cf.checks[0].status, Status::ExpectedUnsupported);
    assert!(cf.checks[0].detail.contains("blocked"), "{}", cf.checks[0].detail);
    assert!(cf.checks[1..].iter().all(|c| c.status == Status::Skipped));
    assert_eq!(status_of(&results, "first-run-experience-tutorial").status, Status::Skipped);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_crash_is_reported_and_the_next_scenario_gets_a_fresh_app() {
    let out = out_dir("crash");
    let chosen: Vec<&'static Scenario> = SCENARIOS.iter().filter(|s| ["excel-hello-world", "excel-add-in-commands"].contains(&s.id)).collect();
    // The first app crashes on its fifth call (after the pane opened).
    let mut launcher = FakeLauncher { crash: Some((1, 5)), broken: false };
    let results = run_all(&mut launcher, &chosen, None, &out);
    let first = &results[0];
    assert_eq!(first.status, Status::Fail);
    assert!(first.crashed.as_deref().unwrap_or("").contains("crashed"), "{:?}", first.crashed);
    assert!(first.checks.iter().any(|c| c.status == Status::NotRun && c.detail.contains("crashed")));
    assert_eq!(results[1].status, Status::Pass, "{:?}", results[1].checks);
    let md = summary_markdown(&results, "Test OS", None);
    assert!(md.contains("| excel-hello-world | Test OS | FAIL | app crashed:"), "{md}");
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn failures_say_what_was_seen() {
    let out = out_dir("broken");
    let chosen: Vec<&'static Scenario> = SCENARIOS.iter().filter(|s| s.id == "excel-hello-world").collect();
    let results = run_all(&mut FakeLauncher { crash: None, broken: true }, &chosen, None, &out);
    let r = &results[0];
    assert_eq!(r.status, Status::Fail);
    let c = r.checks.iter().find(|c| c.id == "writes-a1").unwrap();
    assert_eq!(c.status, Status::Fail);
    assert!(c.detail.contains("wasn't true"), "{}", c.detail);
    // What the page knew goes into the report and a file.
    assert!(c.detail.contains("onReady 1/1 done (callback threw: x is null)") && c.detail.contains("TypeError"), "{}", c.detail);
    assert!(out.join("diagnostics/excel-hello-world-0.json").is_file());
    assert_eq!(r.checks.iter().find(|c| c.id == "pane").unwrap().status, Status::Pass);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn reports_are_json_and_a_markdown_table() {
    let out = out_dir("reports");
    let results = run_all(&mut FakeLauncher::default(), &all(), None, &out);
    let json = results_json(&results, "macOS aarch64", Some("2fb9262ce3802c8497d3b148a9bed7129a675da7"));
    assert_eq!(json["passed"], true);
    assert_eq!(json["scenarios"][0]["id"], "excel-hello-world");
    assert_eq!(json["scenarios"][0]["status"], "pass");
    assert_eq!(json["scenarios"][0]["checks"][0]["expect"], "must pass");
    let md = summary_markdown(&results, "macOS aarch64", Some("2fb9262ce3802c8497d3b148a9bed7129a675da7"));
    assert!(md.starts_with("## Office add-in samples on macOS aarch64"));
    assert!(md.contains("| Scenario | Platform | Result | Details |"));
    assert!(md.contains("| excel-hello-world | macOS aarch64 | pass | 5 checks passed |"), "{md}");
    assert!(md.contains("| keyboard-shortcuts | macOS aarch64 | expected unsupported | shortcuts: expected unsupported"), "{md}");
    assert!(md.contains("Office-Add-in-samples@2fb9262ce3"));
    assert_eq!(cell("a|b\nc"), "a\\|b c");
    let list = list_markdown();
    assert!(list.contains("| custom-functions-storage |") && list.contains("expected unsupported"));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn arguments() {
    let o = parse_args(&["--app", "a", "--samples", "s", "--out", "o", "--only", "excel-hello-world,keyboard-shortcuts"]).unwrap();
    assert_eq!(o.app, Some(PathBuf::from("a")));
    assert_eq!(o.only, ["excel-hello-world", "keyboard-shortcuts"]);
    assert!(parse_args(&["--out", "o"]).unwrap_err().contains("--dry-run"));
    assert!(parse_args(&["--out", "o", "--dry-run"]).unwrap().dry_run);
    assert!(parse_args(&["--dry-run"]).unwrap_err().contains("--out"));
    assert!(parse_args(&["--list"]).unwrap().list);
    assert!(parse_args(&["--out", "o", "--dry-run", "--only", "nope"]).unwrap_err().contains("no scenario"));
    assert!(parse_args(&["--app"]).unwrap_err().contains("needs a value"));
}

#[test]
fn control_answers() {
    assert_eq!(parse_answer(r#"{"id":1,"ok":true,"result":{"a":1}}"#), Ok(json!({"a": 1})));
    assert_eq!(parse_answer(r#"{"id":1,"ok":false,"error":"no such pane"}"#), Err(CallError::Answer("no such pane".into())));
    assert!(matches!(parse_answer("garbage"), Err(CallError::Answer(_))));
}

/// The whole command in dry-run mode writes its files.
#[test]
fn the_command_writes_results_and_summary() {
    let out = out_dir("command");
    let out_s = out.display().to_string();
    assert_eq!(run(&["--dry-run", "--out", &out_s, "--only", "excel-hello-world"]), Ok(true));
    let json: Json = serde_json::from_str(&std::fs::read_to_string(out.join("results.json")).unwrap()).unwrap();
    assert_eq!(json["scenarios"].as_array().unwrap().len(), 1);
    assert!(json["platform"].as_str().unwrap().ends_with("(dry run)"));
    assert!(std::fs::read_to_string(out.join("summary.md")).unwrap().contains("excel-hello-world"));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn fills_read_in_both_representations() {
    use scenarios::{fill_rgb, next_action_colour};
    // As the Windows run reported it: 16776960 is #FFFF00.
    let yellow = json!({"style": {"fill": {"bg": "Auto", "fg": {"Rgb": 16776960}, "pattern": "Solid"}}});
    assert_eq!(fill_rgb(&yellow).as_deref(), Some("FFFF00"));
    assert_eq!(fill_rgb(&json!({"style": {"fill": {"fg": "#c7cc7a", "pattern": "Solid"}}})).as_deref(), Some("C7CC7A"));
    assert_eq!(fill_rgb(&json!({"style": {"fill": {"fg": {"Rgb": 1}, "pattern": "None"}}})), None);
    assert_eq!(fill_rgb(&json!({"style": {}})), None);
    // RunAction's cycle: a blank cell reads #FFFFFF, so the next is #C7CC7A (13094010).
    assert_eq!(next_action_colour(None), "C7CC7A");
    assert_eq!(format!("{:06X}", 13094010), "C7CC7A");
    assert_eq!(next_action_colour(Some("C7CC7A")), "7560BA");
    assert_eq!(next_action_colour(Some("E26D5C")), "FFFFFF");
    assert_eq!(next_action_colour(Some("123456")), "FFFFFF");
}
