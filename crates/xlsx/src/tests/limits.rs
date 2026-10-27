//! Excel's limits as the bound of legitimate input, and the schema as the list of what a streamed
//! element may hold. A worksheet row of exactly the grid's width, with or without its extension
//! list, is read whole with no warning; one cell more is dropped with a warning that says so and
//! never replaces what was already read. Cell text and shared-string runs past Excel's length are
//! kept whole, since cutting them would lose the user's data at the next save, and warned about.
//! Elements the schema does not know are tolerated a few times and then dropped, and never crowd
//! out an element the schema allows.

use gridcraft_core::{CellRef, MAX_CELL_CHARS, MAX_COLS, Value, col_to_letters};

use super::minimal;
use crate::read_xlsx;
use crate::xml::MAX_UNLISTED_CHILD_NAMES;

const SST_RELS: &str = r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>"#;
const EXT_LST: &str = r#"<extLst><ext uri="{00000000-0000-0000-0000-000000000000}"/></extLst>"#;
const A1_IS_SHARED_STRING_0: &str = r#"<sheetData><row r="1"><c r="A1" t="s"><v>0</v></c></row></sheetData>"#;

fn row_warning(n: u64) -> String {
    format!("sheet \"Data\": dropped {n} element(s) a row may not hold (more than {MAX_COLS} cells, a second extension list, or unknown elements)")
}
fn string_warning(n: u64) -> String {
    format!("shared strings: dropped {n} element(s) a string may not hold (a second text or phonetic properties, or unknown elements)")
}
const LONG_TEXT: &str = "cell texts longer than Excel's 32767 characters were kept whole; Excel may refuse or repair the saved file";
const LONG_SHARED: &str =
    "shared strings: 1 string(s) longer than Excel's 32767 characters were kept whole; Excel may refuse or repair the saved file";

/// One row: `unknown` invented elements first, then `cells` cells from A1 on, then `ext_lsts`
/// extension lists, then `extra` cells that re-address A1 with the value 999. Such a cell is inside
/// the grid, so the reader's own grid check cannot catch it: only the row's cell limit can keep it
/// out.
fn row_of(unknown: usize, cells: u32, ext_lsts: usize, extra: usize) -> String {
    let mut body = String::from("<sheetData><row r=\"1\">");
    for i in 0..unknown {
        body.push_str(&format!("<unknown{i}/>"));
    }
    for col in 0..cells {
        body.push_str(&format!("<c r=\"{}1\"><v>{}</v></c>", col_to_letters(col), col + 1));
    }
    body.push_str(&EXT_LST.repeat(ext_lsts));
    body.push_str(&"<c r=\"A1\"><v>999</v></c>".repeat(extra));
    body.push_str("</row></sheetData>");
    body
}

fn a1(wb: &gridcraft_model::Workbook) -> Value {
    wb.sheet(0).unwrap().value(CellRef::parse("A1").unwrap())
}

#[test]
fn a_row_as_wide_as_the_grid_is_read_whole_and_one_cell_more_is_dropped_with_a_warning() {
    // The full width, with and without the extLst.
    for ext_lsts in [0, 1] {
        let (wb, rep) = read_xlsx(&minimal(&row_of(0, MAX_COLS, ext_lsts, 0), &[], "", "")).unwrap();
        let s = wb.sheet(0).unwrap();
        assert_eq!(a1(&wb), Value::Number(1.0));
        assert_eq!(s.value(CellRef::parse("XFD1").unwrap()), Value::Number(MAX_COLS as f64));
        assert_eq!(s.cells.len(), MAX_COLS as usize);
        assert!(!rep.warnings.iter().any(|w| w.contains("dropped")), "extLst {ext_lsts}: {:?}", rep.warnings);
    }
    // One cell more than the width, with and without the extLst: dropped, A1 untouched, one
    // warning.
    for ext_lsts in [0, 1] {
        let (wb, rep) = read_xlsx(&minimal(&row_of(0, MAX_COLS, ext_lsts, 1), &[], "", "")).unwrap();
        let s = wb.sheet(0).unwrap();
        assert_eq!(a1(&wb), Value::Number(1.0), "extLst {ext_lsts}: the cell past the row's width overwrote A1");
        assert_eq!(s.value(CellRef::parse("XFD1").unwrap()), Value::Number(MAX_COLS as f64));
        assert_eq!(s.cells.len(), MAX_COLS as usize);
        assert!(rep.warnings.contains(&row_warning(1)), "extLst {ext_lsts}: {:?}", rep.warnings);
    }
    // A second extLst is one more than the schema allows: dropped with the same warning.
    let (wb, rep) = read_xlsx(&minimal(&row_of(0, 3, 2, 0), &[], "", "")).unwrap();
    assert_eq!(wb.sheet(0).unwrap().cells.len(), 3);
    assert!(rep.warnings.contains(&row_warning(1)), "{:?}", rep.warnings);
}

#[test]
fn unknown_elements_in_a_row_are_tolerated_a_few_times_and_never_crowd_out_cells_or_the_extension_list() {
    // Exactly the tolerated number of unknown elements: nothing dropped, and the full row of cells
    // plus its extLst behind them are read whole.
    let (wb, rep) = read_xlsx(&minimal(&row_of(MAX_UNLISTED_CHILD_NAMES, MAX_COLS, 1, 0), &[], "", "")).unwrap();
    assert_eq!(wb.sheet(0).unwrap().cells.len(), MAX_COLS as usize);
    assert!(!rep.warnings.iter().any(|w| w.contains("dropped")), "{:?}", rep.warnings);
    // Many more unknown elements: the extra ones are dropped and counted, the cells and the extLst
    // behind them are still read whole, and A1 is still protected from a cell past the width.
    let (wb, rep) = read_xlsx(&minimal(&row_of(1000, MAX_COLS, 1, 1), &[], "", "")).unwrap();
    let s = wb.sheet(0).unwrap();
    assert_eq!(s.cells.len(), MAX_COLS as usize);
    assert_eq!(a1(&wb), Value::Number(1.0));
    assert_eq!(s.value(CellRef::parse("XFD1").unwrap()), Value::Number(MAX_COLS as f64));
    assert!(rep.warnings.contains(&row_warning(1000 - MAX_UNLISTED_CHILD_NAMES as u64 + 1)), "{:?}", rep.warnings);
}

/// A shared-string table with one string: `unknown` invented elements, then the plain text if
/// asked, `runs` one-character rich-text runs, `phonetic` one-character phonetic runs, and `props`
/// phonetic properties elements.
fn sst_with(unknown: usize, plain: Option<&str>, runs: usize, phonetic: usize, props: usize) -> String {
    let mut si = String::from("<si>");
    for i in 0..unknown {
        si.push_str(&format!("<unknown{i}/>"));
    }
    if let Some(t) = plain {
        si.push_str(&format!("<t>{t}</t>"));
    }
    si.push_str(&"<r><t>a</t></r>".repeat(runs));
    si.push_str(&"<rPh sb=\"0\" eb=\"1\"><t>b</t></rPh>".repeat(phonetic));
    si.push_str(&"<phoneticPr fontId=\"0\"/>".repeat(props));
    si.push_str("</si>");
    format!(r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="1" uniqueCount="1">{si}</sst>"#)
}

fn read_sst(sst: String) -> (gridcraft_model::Workbook, crate::ReadReport) {
    read_xlsx(&minimal(A1_IS_SHARED_STRING_0, &[("xl/sharedStrings.xml", &sst)], SST_RELS, "")).unwrap()
}

#[test]
fn a_shared_string_keeps_its_runs_whole_past_excels_length_and_drops_only_what_the_schema_forbids() {
    let most = MAX_CELL_CHARS as usize;
    // Everything the schema allows, with as many runs as a cell has characters: read whole, no
    // warning.
    let (wb, rep) = read_sst(sst_with(0, Some(""), most, most, 1));
    assert_eq!(a1(&wb), Value::Text("a".repeat(most).into()));
    assert!(!rep.warnings.iter().any(|w| w.contains("dropped") || w.contains("kept whole")), "{:?}", rep.warnings);
    // One run more than Excel's length: kept whole (32,768 characters), with the long-text warning
    // and no drop.
    let (wb, rep) = read_sst(sst_with(0, None, most + 1, 0, 0));
    assert_eq!(a1(&wb), Value::Text("a".repeat(most + 1).into()));
    assert!(rep.warnings.contains(&LONG_SHARED.to_string()), "{:?}", rep.warnings);
    assert!(!rep.warnings.iter().any(|w| w.contains("dropped")), "{:?}", rep.warnings);
    // A second text or a second phonetic properties element is one more than the schema allows:
    // dropped with the warning; the first text stays.
    let two_of_each = r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>first</t><t>second</t><phoneticPr fontId="0"/><phoneticPr fontId="1"/></si></sst>"#;
    let (wb, rep) = read_sst(two_of_each.to_string());
    assert_eq!(a1(&wb), Value::Text("first".into()));
    assert!(rep.warnings.contains(&string_warning(2)), "{:?}", rep.warnings);
    // Unknown elements before the text are tolerated a few times and never crowd the text out.
    let (wb, rep) = read_sst(sst_with(MAX_UNLISTED_CHILD_NAMES, Some("hello"), 0, 0, 1));
    assert_eq!(a1(&wb), Value::Text("hello".into()));
    assert!(!rep.warnings.iter().any(|w| w.contains("dropped")), "{:?}", rep.warnings);
    let (wb, rep) = read_sst(sst_with(1000, Some("hello"), 2, 1, 1));
    assert_eq!(a1(&wb), Value::Text("helloaa".into()));
    assert!(rep.warnings.contains(&string_warning(1000 - MAX_UNLISTED_CHILD_NAMES as u64)), "{:?}", rep.warnings);
}

#[test]
fn cell_text_longer_than_excel_allows_is_kept_whole_with_a_warning() {
    let most = MAX_CELL_CHARS as usize;
    let plain =
        |n: usize| format!(r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>{}</t></si></sst>"#, "a".repeat(n));
    // A shared string of exactly the limit: no warning. One character more: kept whole, warned.
    let (wb, rep) = read_sst(plain(most));
    assert_eq!(a1(&wb), Value::Text("a".repeat(most).into()));
    assert!(!rep.warnings.iter().any(|w| w.contains("kept whole")), "{:?}", rep.warnings);
    let (wb, rep) = read_sst(plain(most + 1));
    assert_eq!(a1(&wb), Value::Text("a".repeat(most + 1).into()));
    assert!(rep.warnings.contains(&LONG_SHARED.to_string()), "{:?}", rep.warnings);
    // Inline strings and formula string results in the sheet itself: the same, as one warning.
    for body in [
        format!(r#"<sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>{}</t></is></c></row></sheetData>"#, "a".repeat(most + 1)),
        format!(r#"<sheetData><row r="1"><c r="A1" t="str"><f>REPT("a",40000)</f><v>{}</v></c></row></sheetData>"#, "a".repeat(most + 1)),
    ] {
        let (wb, rep) = read_xlsx(&minimal(&body, &[], "", "")).unwrap();
        assert_eq!(a1(&wb), Value::Text("a".repeat(most + 1).into()));
        assert!(rep.warnings.contains(&LONG_TEXT.to_string()), "{:?}", rep.warnings);
    }
    // The limit is counted as Excel counts, in UTF-16 units: 16,384 characters outside the Basic
    // Multilingual Plane are 32,768 units, one over, and 16,383 are within it.
    let astral =
        |n: usize| format!(r#"<sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>{}</t></is></c></row></sheetData>"#, "\u{1F600}".repeat(n));
    let (wb, rep) = read_xlsx(&minimal(&astral(most / 2 + 1), &[], "", "")).unwrap();
    assert_eq!(a1(&wb), Value::Text("\u{1F600}".repeat(most / 2 + 1).into()));
    assert!(rep.warnings.contains(&LONG_TEXT.to_string()), "{:?}", rep.warnings);
    let (_, rep) = read_xlsx(&minimal(&astral(most / 2), &[], "", "")).unwrap();
    assert!(!rep.warnings.iter().any(|w| w.contains("kept whole")), "{:?}", rep.warnings);
    // At the limit exactly, nothing is said.
    let body = format!(r#"<sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>{}</t></is></c></row></sheetData>"#, "a".repeat(most));
    let (wb, rep) = read_xlsx(&minimal(&body, &[], "", "")).unwrap();
    assert_eq!(a1(&wb), Value::Text("a".repeat(most).into()));
    assert!(!rep.warnings.iter().any(|w| w.contains("kept whole")), "{:?}", rep.warnings);
}
