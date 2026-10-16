#![allow(clippy::unwrap_used)]

use super::*;
use gridcraft_core::Value;
use gridcraft_model::{Cell, Formula};
use serde_json::json;

#[test]
fn cached_import_preserves_values_and_edits_update_dependents() {
    let mut wb = Workbook::new();
    let sh = wb.sheet_mut(0).unwrap();
    sh.cells.set(cell(0, 0), Cell::value(Value::Number(2.0)));
    let mut formula = Cell::formula(Formula::new("A1*3"));
    formula.value = Value::Number(999.0);
    sh.cells.set(cell(0, 1), formula);
    let doc = DocState::with_calculation(wb, None, "cached.xlsx".into(), true);
    assert!(!doc.is_dirty());
    assert_eq!(doc.wb.sheet(0).unwrap().value(cell(0, 1)), Value::Number(999.0));
    let mut session = Session::new();
    session.add_document(doc);
    session.execute("cell.set", json!({"cell":"A1","input":"4"})).unwrap();
    assert_eq!(session.doc().unwrap().wb.sheet(0).unwrap().value(cell(0, 1)), Value::Number(12.0));
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(session.doc().unwrap().wb.sheet(0).unwrap().value(cell(0, 0)), Value::Number(2.0));
}

#[test]
fn binary_import_is_equivalent_and_invalid_import_keeps_existing_document() {
    let mut source = Session::new();
    source.new_workbook();
    source.execute("cell.set", json!({"cell":"A1","input":"=1+2"})).unwrap();
    let result = source.execute("file.saveBytes", json!({})).unwrap();
    let bytes = io::base64_decode(result["base64"].as_str().unwrap()).unwrap();
    assert!(gridcraft_xlsx::read_xlsx(&bytes).unwrap().1.cached_results_complete);
    let mut target = Session::new();
    target.new_workbook();
    target.open_bytes("sample.xlsx", &bytes).unwrap();
    assert_eq!(target.documents().len(), 1);
    assert_eq!(target.doc().unwrap().wb.sheet(0).unwrap().value(cell(0, 0)), Value::Number(3.0));
    let uid = target.doc().unwrap().uid;
    assert!(target.open_bytes("broken.xlsx", b"PK\x03\x04broken").is_err());
    assert_eq!(target.doc().unwrap().uid, uid);
    assert_eq!(target.doc().unwrap().wb.sheet(0).unwrap().value(cell(0, 0)), Value::Number(3.0));
}
use std::io::Write as _;

fn workbook_with_sheet(sheet: &str) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, xml) in [
        (
            "xl/workbook.xml",
            r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
        ),
        ("xl/worksheets/sheet1.xml", sheet),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn missing_caches_and_array_reconstruction_require_initial_calculation() {
    for (body, complete) in [
        (r#"<c r="A1"><f>1+2</f><v>3</v></c>"#, true),
        (r#"<c r="A1"><f>1+2</f></c>"#, false),
        (r#"<c r="A1"><f t="array" ref="A1:A2">ROW(A1:A2)</f><v>1</v></c>"#, false),
    ] {
        let sheet = format!(
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1">{body}</row></sheetData></worksheet>"#
        );
        let (_, report) = gridcraft_xlsx::read_xlsx(&workbook_with_sheet(&sheet)).unwrap();
        assert_eq!(report.cached_results_complete, complete);
    }
}

#[test]
fn large_sheet_defers_expression_retention_without_losing_formula_or_saved_value() {
    let padding = " ".repeat(20 * 1024 * 1024);
    let sheet = format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><!--{padding}--><sheetData><row r="1"><c r="A1"><f>1+2</f><v>123</v></c></row></sheetData></worksheet>"#
    );
    let (workbook, report) = gridcraft_xlsx::read_xlsx(&workbook_with_sheet(&sheet)).unwrap();
    assert!(report.cached_results_complete);
    let cell = workbook.sheet(0).unwrap().cell(cell(0, 0)).unwrap();
    assert_eq!(cell.value, Value::Number(123.0));
    let formula = cell.formula.as_ref().unwrap();
    assert_eq!(formula.text, "1+2");
    assert!(formula.expr.is_none());
    assert!(formula.expr_arc().is_some());
}
