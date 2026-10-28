//! Original synthetic file formulas must keep their namespace even if the function is newer.
use super::minimal;
use gridcraft_core::{CellRef, Value};
use std::io::Read;

/// Read the literal formula spelling directly from the saved package, without model normalization.
fn formulas(bytes: &[u8]) -> std::collections::BTreeMap<String, String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = Vec::new();
    zip.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_end(&mut xml).unwrap();
    let root = crate::xml::parse(&xml).unwrap();
    root.child("sheetData")
        .unwrap()
        .kids("row")
        .flat_map(|r| r.kids("c"))
        .filter_map(|c| Some((c.attr("r")?.to_string(), c.child("f")?.text.clone())))
        .collect()
}

/// Two full saves and an unrelated edit must retain the supplied spelling, not only its parsed call.
fn roundtrip_formula(text: &str) {
    let xml = format!(r#"<sheetData><row r="1"><c r="A1"><f>{text}</f><v>7</v></c></row></sheetData>"#);
    let mut bytes = minimal(&xml, &[], "", "");
    for _ in 0..2 {
        let (mut wb, _) = crate::read_xlsx(&bytes).unwrap();
        wb.sheet_mut(0).unwrap().set_value(CellRef::parse("Z20").unwrap(), Value::Number(1.0));
        bytes = crate::write_xlsx(&wb).unwrap();
        assert_eq!(formulas(&bytes)["A1"], text);
    }
}

/// Existing M365 functions are preserved without needing to implement their calculation.
#[test]
fn future_prefixes_known_m365_survive_read_and_save() {
    for formula in [r#"_xlfn.STOCKHISTORY("SYNTHETIC",1)"#, r#"_xlfn.TRANSLATE("test","ro","en")"#, r#"_xlfn.DETECTLANGUAGE("test")"#] {
        roundtrip_formula(formula);
    }
}

/// Unknown functions, nested calls and the worksheet namespace must remain distinguishable.
#[test]
fn future_prefixes_unknown_survive_read_and_save() {
    for formula in [
        r#"_xlfn.COPILOT("SYNTHETIC")"#,
        "_xlfn.SYNTHETICFUTURE(B1)",
        "SUM(_xlfn.SYNTHETICFUTURE(B1),1)",
        "_xlfn._xlws.SYNTHETICFILTER(B1)",
        "_xludf.SUM(1)",
    ] {
        roundtrip_formula(formula);
    }
}

/// Shared descendants get both shifted references and the master's unknown-function namespace.
#[test]
fn future_prefixes_shared_and_array_formulas_survive() {
    let xml = r#"<sheetData><row r="1"><c r="A1"><f t="shared" si="0" ref="A1:A2">_xlfn.SYNTHETICFUTURE(B1)</f><v>7</v></c></row><row r="2"><c r="A2"><f t="shared" si="0"/><v>8</v></c></row><row r="3"><c r="A3"><f t="array" ref="A3">_xlfn.SYNTHETICARRAY(B1)</f><v>9</v></c></row></sheetData>"#;
    let (wb, _) = crate::read_xlsx(&minimal(xml, &[], "", "")).unwrap();
    let bytes = crate::write_xlsx(&wb).unwrap();
    let f = formulas(&bytes);
    assert_eq!(f["A1"], "_xlfn.SYNTHETICFUTURE(B1)");
    assert_eq!(f["A2"], "_xlfn.SYNTHETICFUTURE(B2)");
    assert_eq!(f["A3"], "_xlfn.SYNTHETICARRAY(B1)");
}

/// Namespace metadata belongs to each occurrence: a bare call must not inherit another call's tag.
#[test]
fn mixed_namespaces_on_the_same_function_remain_distinct() {
    for formula in ["_xlfn.SYNTHETICFUTURE(A1)+SYNTHETICFUTURE(B1)", "_xlfn.SYNTHETICFUTURE(_xlfn._xlws.SYNTHETICFUTURE(A1))+SYNTHETICFUTURE(B1)"] {
        roundtrip_formula(formula);
    }
}
