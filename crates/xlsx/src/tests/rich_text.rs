//! Rich text: a text cell's formatted runs (`<r><rPr>…</rPr><t>…</t></r>`) in shared and inline
//! strings are read onto the cell and written back.

use gridcraft_core::{CellRef, Value};
use gridcraft_model::{Color, Workbook};

use super::minimal;
use crate::{read_xlsx, write_xlsx};

const SST: &str = r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="3" uniqueCount="3">
<si><r><t xml:space="preserve">Matrikel </t></r><r><rPr><b/><sz val="14"/><color rgb="FFFF0000"/><rFont val="Calibri"/><family val="2"/></rPr><t>Rep</t></r></si>
<si><t>plain</t></si>
<si><r><t>Matrikel </t></r><r><t>Rep</t></r></si>
</sst>"#;

const RELS: &str = r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>"#;

fn book() -> Workbook {
    let body = r#"<sheetData><row r="1">
<c r="A1" t="s"><v>0</v></c>
<c r="B1" t="s"><v>1</v></c>
<c r="C1" t="s"><v>2</v></c>
<c r="D1" t="inlineStr"><is><r><t>x</t></r><r><rPr><i/><u/><vertAlign val="superscript"/><sz val="8"/></rPr><t>2</t></r></is></c>
<c r="E1" t="str"><f>A1&amp;""</f><v>Matrikel Rep</v></c>
</row></sheetData>"#;
    read_xlsx(&minimal(body, &[("xl/sharedStrings.xml", SST)], RELS, "")).unwrap().0
}

fn check(wb: &Workbook) {
    let sh = wb.sheet(0).unwrap();
    let cell = |a: &str| sh.cell(CellRef::parse(a).unwrap()).unwrap();
    let a1 = cell("A1");
    assert_eq!(a1.value, Value::from("Matrikel Rep"));
    let rich = a1.rich.as_ref().expect("A1 is rich");
    let spans = rich.spans("Matrikel Rep").expect("the runs describe the text");
    assert_eq!(spans.len(), 2);
    assert_eq!(spans[0], ("Matrikel ", None));
    let (t, f) = spans[1];
    let f = f.expect("the second run has a font");
    assert_eq!(t, "Rep");
    assert!(f.bold && f.size == 14.0 && f.color == Color::Rgb(0xFF0000) && f.name == "Calibri");
    // Plain strings, runs without formatting and formula results carry no runs.
    assert!(cell("B1").rich.is_none());
    assert!(cell("C1").rich.is_none());
    assert_eq!(cell("C1").value, Value::from("Matrikel Rep"));
    assert!(cell("E1").rich.is_none());
    // Inline rich strings too: italic, underlined, superscript.
    let d1 = cell("D1").rich.as_ref().and_then(|r| r.spans("x2")).expect("D1 is rich");
    let f = d1[1].1.unwrap();
    assert!(f.italic && f.underline != gridcraft_model::Underline::None && f.vert == gridcraft_model::VertAlign::Superscript && f.size == 8.0);
}

#[test]
fn rich_strings_are_read() {
    check(&book());
}

#[test]
fn rich_strings_survive_saving() {
    let bytes = write_xlsx(&book()).unwrap();
    let (back, _) = read_xlsx(&bytes).unwrap();
    check(&back);
    // A second save writes the same shared strings: the plain and the rich "Matrikel Rep" stay two items.
    let sst = |b: &[u8]| {
        use std::io::Read;
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(b.to_vec())).unwrap();
        let mut s = String::new();
        z.by_name("xl/sharedStrings.xml").unwrap().read_to_string(&mut s).unwrap();
        s
    };
    let first = sst(&bytes);
    assert_eq!(first, sst(&write_xlsx(&back).unwrap()));
    assert_eq!(first.matches("<si>").count(), 4, "{first}");
}

#[test]
fn runs_that_no_longer_fit_the_text_are_not_written() {
    let mut wb = book();
    let sh = std::sync::Arc::make_mut(&mut wb.sheets[0]);
    let a1 = CellRef::parse("A1").unwrap();
    let mut cell = sh.cell(a1).unwrap().clone();
    cell.value = Value::from("changed");
    sh.cells.set(a1, cell);
    let (back, _) = read_xlsx(&write_xlsx(&wb).unwrap()).unwrap();
    let a1 = back.sheet(0).unwrap().cell(a1).unwrap();
    assert_eq!(a1.value, Value::from("changed"));
    assert!(a1.rich.is_none());
}

#[test]
fn runs_are_not_written_onto_a_replacement_text() {
    use gridcraft_model::{Cell, Font, RichText, Table, TableColumn, TextRun, TotalsFn};
    use std::sync::Arc;
    // A table header cell is saved with its column's name; a rich "abcd" there must not lend its
    // runs to a header named "wxyz" (same length).
    let mut wb = Workbook::new();
    let sh = wb.sheet_mut(0).unwrap();
    let red = Font { color: Color::Rgb(0xFF0000), ..Font::default() };
    let rich = RichText { runs: vec![TextRun { len: 2, font: None }, TextRun { len: 2, font: Some(red) }] };
    sh.cells.set(CellRef::parse("A1").unwrap(), Cell { rich: Some(Arc::new(rich)), ..Cell::value(Value::from("abcd")) });
    sh.cells.set(CellRef::parse("A2").unwrap(), Cell::value(Value::Number(1.0)));
    let column = TableColumn { name: "wxyz".into(), totals: TotalsFn::None, totals_label: None, formula: None, unique_name: None, xml: None };
    sh.tables.push(Table {
        id: 1,
        name: "T".into(),
        range: gridcraft_core::RangeRef::parse("A1:A2").unwrap(),
        header_row: true,
        totals_row: false,
        columns: vec![column],
        style: "TableStyleLight1".into(),
        banded_rows: true,
        banded_cols: false,
        first_col: false,
        last_col: false,
        filter_button: true,
        filter: vec![],
        table_type: None,
        connection_id: None,
    });
    let (back, _) = read_xlsx(&write_xlsx(&wb).unwrap()).unwrap();
    let a1 = back.sheet(0).unwrap().cell(CellRef::parse("A1").unwrap()).unwrap();
    assert_eq!(a1.value, Value::from("wxyz"));
    assert!(a1.rich.is_none());
}
