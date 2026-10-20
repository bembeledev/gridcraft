use gridcraft_engine::{
    Session,
    core::{CellRef, Value},
    patch_data::{self as data, Config, Request},
};
use serde_json::{Value as Json, json};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};
fn session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("sheet.rename", json!({"name":"Item"})).unwrap();
    s
}
fn overlay(s: &Session) -> Result<Json, String> {
    let books = BTreeMap::from([("/project/protobuf/ItemData.xlsx".into(), s.active().unwrap().wb.clone())]);
    serde_json::to_value(data::overlays("/project", &books, &AtomicBool::new(false))?).map_err(|e| e.to_string())
}
#[test]
fn snapshot_preserves_physical_positions_literal_integers_and_formula_results() {
    let mut s = session();
    s.execute("range.setValues", json!({"range":"B3","values":[["'9007199254740993",true],["=2+3","'=literal"]]})).unwrap();
    let o = overlay(&s).unwrap();
    assert_eq!(o[0]["file"], "ItemData.xlsx");
    assert_eq!(o[0]["sheets"][0]["rows"][0][0], json!({"kind":"empty"}));
    assert_eq!(o[0]["sheets"][0]["rows"][2][1], json!({"kind":"string","value":"9007199254740993"}));
    assert_eq!(o[0]["sheets"][0]["rows"][2][2], json!({"kind":"bool","value":true}));
    assert_eq!(o[0]["sheets"][0]["rows"][3][1], json!({"kind":"float","value":5.0}));
    assert_eq!(o[0]["sheets"][0]["rows"][3][2], json!({"kind":"string","value":"=literal"}));
    s.execute("sheet.rename", json!({"name":"Item_Transpose"})).unwrap();
    assert_eq!(overlay(&s).unwrap()[0]["sheets"][0]["rows"], o[0]["sheets"][0]["rows"]);
}
#[test]
fn snapshot_refuses_formula_errors_and_unavailable_results_instead_of_sending_empty() {
    let mut s = session();
    s.execute("cell.set", json!({"cell":"C7","input":"=1/0"})).unwrap();
    assert!(overlay(&s).unwrap_err().contains("C7"));
    s.execute("cell.set", json!({"cell":"C7","input":"=1"})).unwrap();
    let doc = s.active_mut().unwrap();
    std::sync::Arc::make_mut(&mut doc.wb).sheet_mut(0).unwrap().cells.get_mut(CellRef::parse("C7").unwrap()).unwrap().value = Value::Empty;
    assert!(overlay(&s).unwrap_err().contains("수식 결과"));
}
#[test]
fn snapshot_limits_dense_allocation_and_includes_spill_followers() {
    let mut s = session();
    s.execute("cell.set", json!({"cell":"C1048576","input":"1"})).unwrap();
    assert!(overlay(&s).unwrap_err().contains("2,000,000"));
    let mut s = session();
    s.execute("cell.set", json!({"cell":"B2","input":"=SEQUENCE(2,2)"})).unwrap();
    let o = overlay(&s).unwrap();
    assert_eq!(o[0]["sheets"][0]["rows"][2][2], json!({"kind":"float","value":4.0}));
    s.execute("sheet.rename", json!({"name":"_Ignored"})).unwrap();
    assert_eq!(overlay(&s).unwrap()[0]["sheets"][0]["rows"], json!([]));
}
#[test]
fn snapshot_keeps_raw_date_serials_including_1904_workbooks() {
    let mut s = session();
    s.execute("cell.set", json!({"cell":"B4","input":"45000.5"})).unwrap();
    s.execute("home.numberFormat", json!({"range":"B4","code":"yyyy-mm-dd hh:mm:ss"})).unwrap();
    for system in [gridcraft_engine::core::DateSystem::D1900, gridcraft_engine::core::DateSystem::D1904] {
        std::sync::Arc::make_mut(&mut s.active_mut().unwrap().wb).date_system = system;
        assert_eq!(overlay(&s).unwrap()[0]["sheets"][0]["rows"][3][1], json!({"kind":"date_time","value":45000.5}));
    }
}
#[test]
fn navigation_does_not_invent_missing_coordinates_or_allow_path_traversal() {
    let mut l = data::Location { file: Some("ItemData.xlsx".into()), row: Some(3), ..Default::default() };
    assert!(l.cell().is_none());
    l.column = Some(1);
    assert_eq!(l.cell().unwrap().a1(), "B4");
    assert_eq!(l.path("/project/protobuf").unwrap(), "/project/protobuf/ItemData.xlsx");
    l.row = Some(u32::MAX);
    assert!(l.cell().is_none());
    for path in ["../secret", "/tmp/secret", "..\\secret", ".."] {
        l.file = Some(path.into());
        assert!(l.path("/project").is_none());
    }
    assert!(data::workbook_file("/project", "/elsewhere/ItemData.xlsx").is_none());
    assert!(data::workbook_file("/project", "/project/protobuf/Notes.xlsx").is_none());
    let cwd = std::env::current_dir().unwrap();
    assert_eq!(data::workbook_file(cwd.to_str().unwrap(), "protobuf/ItemData.xlsx"), Some("ItemData.xlsx".into()));
    assert_eq!(data::project_root("."), cwd.canonicalize().unwrap());
}
#[test]
fn missing_id_map_is_an_error_and_programmatic_calls_never_open_dialogs() {
    let mut s = session();
    assert!(s.execute("patch.validate", json!({})).is_err());
    assert!(s.execute("patch.validate", json!({"project":"ck","root":"/project","config":{}})).is_err());
    assert!(Request::new("/project", &Config::default(), "rev".into(), vec![]).is_err());
}

#[cfg(unix)]
mod transport {
    use super::*;
    use std::{
        io::Write,
        os::unix::fs::PermissionsExt,
        sync::{Arc, atomic::Ordering},
        time::Duration,
    };
    fn response() -> Json {
        json!({"event":"result","request_id":"revision","revision":"revision","validator":{"protocol_version":1,"package_version":"fixture","source_revision":"fixture"},"report":{"valid":false,"stages":[{"stage":"raw","status":"passed"},{"stage":"parse","status":"failed"},{"stage":"versions","status":"skipped"},{"stage":"validation","status":"skipped"}],"diagnostics":[{"code":"from.patcher","severity":"error","message":"exact worker message","location":{"file":"ItemData.xlsx","sheet":"Item","table":null,"row":3,"column":1},"related":[]}],"total_diagnostics":1,"truncated":false,"simulated_new_ids":0,"filtered_rows":0,"effective_build_name":null}})
    }
    fn run_response(value: Json, exit: u8) -> Result<data::Checked, String> {
        let mut executable = tempfile::NamedTempFile::new().unwrap();
        writeln!(executable, "#!/bin/sh\ncat >/dev/null\ncat <<'RESULT'\n{value}\nRESULT\nexit {exit}").unwrap();
        executable.as_file().set_permissions(std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = Config { executable: executable.path().to_string_lossy().into(), id_map_directory: "/id-map".into(), ..Default::default() };
        let req = Request::new("/project", &config, "revision".into(), vec![]).unwrap();
        data::check(&config, &req, &AtomicBool::new(false), |_| {})
    }
    #[test]
    fn report_is_exactly_the_workers_report_and_protocol_failures_never_fall_back() {
        let response = response();
        let checked = run_response(response.clone(), 1).unwrap();
        assert_eq!(serde_json::to_value(&checked.report).unwrap(), response["report"]);
        for field in ["request_id", "revision"] {
            let mut bad = response.clone();
            bad[field] = json!("stale");
            assert!(run_response(bad, 1).is_err());
        }
        let mut bad = response.clone();
        bad["validator"]["protocol_version"] = json!(2);
        assert!(run_response(bad, 1).is_err());
        let mut bad = response.clone();
        bad["report"]["valid"] = json!(true);
        assert!(run_response(bad, 0).is_err());
        assert!(run_response(response, 0).is_err());
        assert!(run_response(json!({"broken":true}), 0).is_err());
    }
    #[test]
    fn cancellation_terminates_and_reaps_the_child() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("worker");
        let pidfile = dir.path().join("pid");
        std::fs::write(&script, format!("#!/bin/sh\necho $$ > '{}'\nexec sleep 60\n", pidfile.display())).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = Config { executable: script.to_string_lossy().into(), id_map_directory: "/id-map".into(), ..Default::default() };
        let req = Request::new("/project", &config, "revision".into(), vec![]).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let other = cancel.clone();
        let join = std::thread::spawn(move || data::check(&config, &req, &other, |_| {}));
        for _ in 0..100 {
            if pidfile.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let pid = std::fs::read_to_string(&pidfile).unwrap();
        cancel.store(true, Ordering::Relaxed);
        assert!(join.join().unwrap().unwrap_err().contains("취소"));
        assert!(!std::process::Command::new("kill").args(["-0", pid.trim()]).stderr(std::process::Stdio::null()).status().unwrap().success());
    }
}
