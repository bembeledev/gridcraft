//! Synthetic, in-memory XLSX lifecycle checks: no Office-authored fixtures or filesystem I/O.
use std::io::{Cursor, Read, Write};

use gridcraft_core::{CellError, CellRef, Value};
use gridcraft_engine::Session;
use gridcraft_model::{CalcMode, Cell, Formula, Workbook};
use serde_json::{Value as Json, json};

/// Build an in-memory workbook with cached, unsupported, stale and missing results.
fn fixture() -> Json {
    let mut wb = Workbook::new();
    let sheet = wb.sheet_mut(0).unwrap();
    sheet.set_value(CellRef::parse("A1").unwrap(), Value::Number(10.0));
    for (at, text, value) in [
        ("B1", "GETPIVOTDATA(\"Suma\",A1)", Value::Number(1250.25)),
        ("C1", "B1*2", Value::Number(2500.5)),
        ("D1", "A1*3", Value::Empty),
        ("E1", "C1+D1", Value::Empty),
        ("F1", "A1+1", Value::Number(999.0)),
    ] {
        let mut cell = Cell::formula(Formula::new(text));
        cell.value = value;
        sheet.set_cell(CellRef::parse(at).unwrap(), cell);
    }
    let bytes = gridcraft_xlsx::write_xlsx(&wb).unwrap();
    json!({"name": "synthetic-caches.xlsx", "base64": gridcraft_engine::io::base64_encode(&bytes)})
}

/// Read a result through the public workbook model.
fn value(s: &Session, at: &str) -> Value {
    s.doc().unwrap().wb.sheet(0).unwrap().value(CellRef::parse(at).unwrap())
}

/// Import preserves saved results, fills missing chains and survives save/reopen.
#[test]
fn cached_open_preserves_saved_results_and_fills_missing_chains() {
    let mut s = Session::new();
    let opened = s.execute("file.open", fixture()).unwrap();
    assert_eq!(value(&s, "B1"), Value::Number(1250.25));
    assert_eq!(value(&s, "C1"), Value::Number(2500.5));
    assert_eq!(value(&s, "D1"), Value::Number(30.0));
    assert_eq!(value(&s, "E1"), Value::Number(2530.5));
    assert_eq!(value(&s, "F1"), Value::Number(999.0));
    assert!(!s.doc().unwrap().is_dirty());
    assert!(opened["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("may be out of date")));
    s.execute("cell.set", json!({"cell": "A10", "input": "1"})).unwrap();
    assert_eq!(value(&s, "B1"), Value::Number(1250.25));
    assert_eq!(value(&s, "C1"), Value::Number(2500.5));
    let saved = s.execute("file.saveBytes", json!({"format": "xlsx"})).unwrap();
    let mut reopened = Session::new();
    reopened.execute("file.open", json!({"name": "roundtrip.xlsx", "base64": saved["base64"]})).unwrap();
    for at in ["B1", "C1", "D1", "E1", "F1"] {
        assert_eq!(value(&reopened, at), value(&s, at));
    }
}

/// A real input edit invalidates its dependent formulas instead of silently freezing them.
#[test]
fn cached_open_recalculates_edited_inputs_and_their_dependents() {
    let mut s = Session::new();
    s.execute("file.open", fixture()).unwrap();
    s.execute("cell.set", json!({"cell": "A1", "input": "20"})).unwrap();
    assert_eq!(value(&s, "B1"), Value::Error(CellError::Name));
    assert_eq!(value(&s, "C1"), Value::Error(CellError::Name));
    assert_eq!(value(&s, "D1"), Value::Number(60.0));
    assert_eq!(value(&s, "F1"), Value::Number(21.0));
}

/// Callers can request the previous full-recalculation policy explicitly.
#[test]
fn cached_open_can_explicitly_recalculate_all_on_open() {
    let mut s = Session::new();
    let mut params = fixture();
    params["recalculate"] = json!(true);
    let opened = s.execute("file.open", params).unwrap();
    assert_eq!(value(&s, "B1"), Value::Error(CellError::Name));
    assert_eq!(value(&s, "C1"), Value::Error(CellError::Name));
    assert_eq!(value(&s, "F1"), Value::Number(11.0));
    assert!(!opened["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("may be out of date")));
}

/// Workbooks without saved formula results do not show the cache notice.
#[test]
fn cached_open_without_saved_formula_results_has_no_cache_warning() {
    let mut s = Session::new();
    let wb = Workbook::new();
    let bytes = gridcraft_xlsx::write_xlsx(&wb).unwrap();
    let opened = s.execute("file.open", json!({"name": "empty.xlsx", "base64": gridcraft_engine::io::base64_encode(&bytes)})).unwrap();
    assert!(opened["warnings"].as_array().unwrap().is_empty());
}

/// Dynamic references and workbook-defined functions remain live after cache-preserving open.
#[test]
fn cached_open_keeps_supported_dynamic_references_live() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("cell.set", json!({"cell": "A1", "input": "10"})).unwrap();
    s.execute("cell.set", json!({"cell": "B1", "input": "=INDIRECT(\"A1\")"})).unwrap();
    s.execute("formulas.defineName", json!({"name": "READVALUE", "refersTo": "=LAMBDA(x,INDIRECT(x))"})).unwrap();
    s.execute("cell.set", json!({"cell": "C1", "input": "=READVALUE(\"A1\")"})).unwrap();
    let saved = s.execute("file.saveBytes", json!({"format": "xlsx"})).unwrap();
    let mut opened = Session::new();
    opened.execute("file.open", json!({"name": "dynamic.xlsx", "base64": saved["base64"]})).unwrap();
    assert_eq!(value(&opened, "B1"), Value::Number(10.0));
    assert_eq!(value(&opened, "C1"), Value::Number(10.0));
    opened.execute("cell.set", json!({"cell": "A1", "input": "20"})).unwrap();
    assert_eq!(value(&opened, "B1"), Value::Number(20.0));
    assert_eq!(value(&opened, "C1"), Value::Number(20.0));
}

/// The Calculate Now command remains an explicit full recalculation.
#[test]
fn cached_open_honors_explicit_calculate_now() {
    let mut s = Session::new();
    s.execute("file.open", fixture()).unwrap();
    s.execute("formulas.calculateNow", json!({})).unwrap();
    assert_eq!(value(&s, "B1"), Value::Error(CellError::Name));
    assert_eq!(value(&s, "F1"), Value::Number(11.0));
}

/// Saved values from registered but unimplemented functions survive an unrelated edit and save.
#[test]
fn cached_open_preserves_stub_results_through_unrelated_edits() {
    let mut wb = Workbook::new();
    for (at, formula, value) in
        [("A1", "FILTERXML(\"<x/>\",\"//x\")", Value::from("SYNTHETIC")), ("B1", "CUBEVALUE(\"synthetic-connection\")", Value::Number(75.5))]
    {
        let mut cell = Cell::formula(Formula::new(formula));
        cell.value = value;
        wb.sheet_mut(0).unwrap().set_cell(CellRef::parse(at).unwrap(), cell);
    }
    let bytes = gridcraft_xlsx::write_xlsx(&wb).unwrap();
    let mut s = Session::new();
    s.execute("file.open", json!({"name": "synthetic-stubs.xlsx", "base64": gridcraft_engine::io::base64_encode(&bytes)})).unwrap();
    s.execute("cell.set", json!({"cell": "Z20", "input": "1"})).unwrap();
    let saved = s.execute("file.saveBytes", json!({"format": "xlsx"})).unwrap();
    let mut reopened = Session::new();
    reopened.execute("file.open", json!({"name": "saved-stubs.xlsx", "base64": saved["base64"]})).unwrap();
    assert_eq!(value(&reopened, "A1"), Value::from("SYNTHETIC"));
    assert_eq!(value(&reopened, "B1"), Value::Number(75.5));
}

/// Manual calculation keeps saved dependents until their own formula is edited.
#[test]
fn cached_open_respects_manual_calculation_after_open() {
    let params = fixture();
    let bytes = gridcraft_engine::io::base64_decode(params["base64"].as_str().unwrap()).unwrap();
    let (mut wb, _) = gridcraft_xlsx::read_xlsx(&bytes).unwrap();
    wb.calc.mode = CalcMode::Manual;
    let bytes = gridcraft_xlsx::write_xlsx(&wb).unwrap();
    let mut s = Session::new();
    s.execute("file.open", json!({"name": "manual.xlsx", "base64": gridcraft_engine::io::base64_encode(&bytes)})).unwrap();
    s.execute("cell.set", json!({"cell": "A1", "input": "20"})).unwrap();
    assert_eq!(value(&s, "B1"), Value::Number(1250.25));
    assert_eq!(value(&s, "F1"), Value::Number(999.0));
    s.execute("cell.set", json!({"cell": "F1", "input": "=A1+1"})).unwrap();
    assert_eq!(value(&s, "F1"), Value::Number(21.0));
}

/// The stale-result notice stays visible when an XLSX also produces many import warnings.
#[test]
fn cached_open_shows_cache_notice_before_import_warning_overflow() {
    let params = fixture();
    let bytes = gridcraft_engine::io::base64_decode(params["base64"].as_str().unwrap()).unwrap();
    let mut source = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..source.len() {
        let mut part = source.by_index(index).unwrap();
        let mut data = Vec::new();
        part.read_to_end(&mut data).unwrap();
        if part.name() == "xl/worksheets/sheet1.xml" {
            let xml = String::from_utf8(data).unwrap();
            let rules = (0..6)
                .map(|n| format!(r#"<conditionalFormatting sqref="A1"><cfRule type="synthetic{n}" priority="{}"/></conditionalFormatting>"#, n + 1))
                .collect::<String>();
            data = xml.replace("</worksheet>", &format!("{rules}</worksheet>")).into_bytes();
        }
        output.start_file(part.name(), zip::write::SimpleFileOptions::default()).unwrap();
        output.write_all(&data).unwrap();
    }
    let bytes = output.finish().unwrap().into_inner();
    let mut s = Session::new();
    let opened = s.execute("file.open", json!({"name": "synthetic-warnings.xlsx", "base64": gridcraft_engine::io::base64_encode(&bytes)})).unwrap();
    let warnings = opened["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 7);
    for n in 0..6 {
        assert!(warnings.iter().any(|w| w.as_str().unwrap().contains(&format!("synthetic{n}"))));
    }
    let messages = s
        .take_ui_requests()
        .into_iter()
        .filter_map(|request| match request {
            gridcraft_engine::UiRequest::Message(message) => Some(message),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("Saved formula results were kept and may be out of date"));
    assert!(messages[0].contains("2 additional import warnings."));
}
