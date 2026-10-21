//! Stacked vertical text (`textRotation="255"`): the OOXML sentinel must survive import, export
//! and a full round trip untouched, and ordinary angles must keep their own encoding.

use gridcraft_core::{CellRef, Value};
use gridcraft_model::*;

use crate::package::Package;
use crate::{read_xlsx, write_xlsx};

fn at(a: &str) -> CellRef {
    CellRef::parse(a).unwrap()
}

/// The `textRotation` attributes of every `<xf>` in the written styles part, in order.
fn written_rotations(bytes: &[u8]) -> Vec<Option<String>> {
    let mut pkg = Package::open(bytes).unwrap();
    let styles = pkg.read_xml("xl/styles.xml").unwrap().unwrap();
    styles.child("cellXfs").unwrap().kids("xf").map(|xf| xf.child("alignment").and_then(|a| a.attr("textRotation")).map(str::to_string)).collect()
}

/// A workbook with one cell whose only non-default style property is `rotation`.
fn workbook_with_rotation(rotation: i16) -> Workbook {
    let mut wb = Workbook::new();
    let style = wb.styles.derive(StyleId::DEFAULT, |s| s.align.rotation = rotation);
    let sheet = wb.sheet_mut(0).unwrap();
    sheet.cells.set(at("A1"), Cell { value: Value::text("VERTICAL MODE TEST"), formula: None, style });
    wb
}

fn rotation_of(wb: &Workbook, address: &str) -> i16 {
    let sh = wb.sheet(0).unwrap();
    wb.styles.get(sh.cell(at(address)).unwrap().style).align.rotation
}

/// A1: `textRotation="255"` reads back as the internal sentinel, not as an angle.
#[test]
fn stacked_vertical_text_rotation_reads_as_the_sentinel() {
    let styles = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
      <fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>
      <fills count="1"><fill><patternFill patternType="none"/></fill></fills>
      <borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
      <cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/>
        <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0" applyAlignment="1"><alignment textRotation="255"/></xf></cellXfs>
      </styleSheet>"#;
    let rels = r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#;
    let body = r#"<sheetData><row r="1"><c r="A1" t="inlineStr" s="1"><is><t>VERTICAL MODE TEST</t></is></c></row></sheetData>"#;
    let bytes = super::minimal(body, &[("xl/styles.xml", styles)], rels, "");

    let (wb, report) = read_xlsx(&bytes).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert_eq!(rotation_of(&wb, "A1"), 255, "the sentinel is kept verbatim, never turned into degrees");
    assert_eq!(wb.styles.get(wb.sheet(0).unwrap().cell(at("A1")).unwrap().style).align, Alignment { rotation: 255, ..Alignment::default() });
}

/// A2: the sentinel is written back out as `textRotation="255"`.
#[test]
fn stacked_vertical_text_rotation_writes_the_ooxml_sentinel() {
    let bytes = write_xlsx(&workbook_with_rotation(255)).unwrap();
    let rotations = written_rotations(&bytes);
    assert!(rotations.iter().any(|r| r.as_deref() == Some("255")), "styles.xml must carry textRotation=\"255\": {rotations:?}");
}

/// A3: write → read → write keeps 255 every time.
#[test]
fn stacked_vertical_text_survives_a_round_trip() {
    let first = write_xlsx(&workbook_with_rotation(255)).unwrap();
    let (back, report) = read_xlsx(&first).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert_eq!(rotation_of(&back, "A1"), 255);
    assert_eq!(back.sheet(0).unwrap().value(at("A1")), Value::text("VERTICAL MODE TEST"));

    let second = write_xlsx(&back).unwrap();
    assert!(written_rotations(&second).iter().any(|r| r.as_deref() == Some("255")), "a saved vertical cell stays vertical");
    let (again, _) = read_xlsx(&second).unwrap();
    assert_eq!(rotation_of(&again, "A1"), 255);
}

/// A4: ordinary angles keep their own model value and OOXML encoding (negatives are stored as
/// `90 - angle`, so -45° is `textRotation="135"`).
#[test]
fn ordinary_angles_keep_their_rotation_and_encoding() {
    for (rotation, encoded) in [(0, None), (45, Some("45")), (90, Some("90")), (-45, Some("135")), (-90, Some("180"))] {
        let bytes = write_xlsx(&workbook_with_rotation(rotation)).unwrap();
        let written = written_rotations(&bytes);
        assert!(written.contains(&encoded.map(str::to_string)), "rotation {rotation} should write {encoded:?}, wrote {written:?}");

        let (back, _) = read_xlsx(&bytes).unwrap();
        assert_eq!(rotation_of(&back, "A1"), rotation, "{rotation}° must come back as itself");
    }
}
