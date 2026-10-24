//! A sheet's saved view: the active cell of the active pane, the scrolling pane's first cell on the
//! axes that are split, and the tab strip's first tab (`firstSheet`).

use std::io::Read;

use gridcraft_core::CellRef;
use gridcraft_model::{Sheet, Workbook};

use super::minimal;
use crate::{read_xlsx, write_xlsx};

fn sheet(view: &str) -> Sheet {
    let body = format!("<sheetViews>{view}</sheetViews><sheetData/>");
    read_xlsx(&minimal(&body, &[], "", "")).unwrap().0.sheet(0).unwrap().clone()
}

fn c(a: &str) -> CellRef {
    CellRef::parse(a).unwrap()
}

#[test]
fn active_pane_without_a_selection_starts_at_its_first_cell() {
    // Rows 1 and columns A:D frozen, the bottom-right pane active, only a top-left selection.
    let sh = sheet(
        r#"<sheetView workbookViewId="0"><pane xSplit="4" ySplit="1" topLeftCell="E2" activePane="bottomRight" state="frozen"/><selection activeCell="A1" sqref="A1"/></sheetView>"#,
    );
    assert_eq!(sh.freeze, Some((1, 4)));
    assert_eq!(sh.view_top_left, c("E2"));
    assert_eq!(sh.view_active, c("E2"));
}

#[test]
fn each_active_pane_starts_at_its_own_first_cell() {
    let pane = |active: &str| {
        sheet(&format!(
            r#"<sheetView workbookViewId="0"><pane xSplit="2" ySplit="3" topLeftCell="F10" activePane="{active}" state="frozen"/><selection activeCell="A1" sqref="A1"/></sheetView>"#
        ))
        .view_active
    };
    assert_eq!(pane("bottomRight"), c("F10"));
    assert_eq!(pane("topRight"), c("F1"));
    assert_eq!(pane("bottomLeft"), c("A10"));
    assert_eq!(pane("topLeft"), c("A1"));
}

#[test]
fn a_split_pane_without_a_selection_starts_at_its_first_cell() {
    // Split (not frozen) after column B and row 3, the sheet view at A1, the second panes at F20.
    let pane = |active: &str| {
        sheet(&format!(
            r#"<sheetView workbookViewId="0"><pane xSplit="2400" ySplit="900" topLeftCell="F20" activePane="{active}"/><selection activeCell="A1" sqref="A1"/></sheetView>"#
        ))
        .view_active
    };
    assert_eq!(pane("bottomRight"), c("F20"));
    assert_eq!(pane("topRight"), c("F1"));
    assert_eq!(pane("bottomLeft"), c("A20"));
    assert_eq!(pane("topLeft"), c("A1"));
}

#[test]
fn a_pane_without_a_selection_keeps_the_axis_that_isnt_frozen() {
    // Rows 1:7 frozen, the sheet scrolled to column D: the panes below and above the split both
    // start at column D.
    let rows_only = |active: &str| {
        sheet(&format!(
            r#"<sheetView topLeftCell="D1" workbookViewId="0"><pane ySplit="7" topLeftCell="A40" activePane="{active}" state="frozen"/></sheetView>"#
        ))
        .view_active
    };
    assert_eq!(rows_only("bottomLeft"), c("D40"));
    assert_eq!(rows_only("topLeft"), c("D1"));
    // Columns A:B frozen, the sheet scrolled to row 30: the panes left and right of the split both
    // start at row 30.
    let cols_only = |active: &str| {
        sheet(&format!(
            r#"<sheetView topLeftCell="A30" workbookViewId="0"><pane xSplit="2" topLeftCell="F1" activePane="{active}" state="frozen"/></sheetView>"#
        ))
        .view_active
    };
    assert_eq!(cols_only("topRight"), c("F30"));
    assert_eq!(cols_only("topLeft"), c("A30"));
}

#[test]
fn active_pane_selection_is_the_active_cell() {
    let sh = sheet(
        r#"<sheetView workbookViewId="0"><pane xSplit="2" ySplit="1" topLeftCell="M2" activePane="bottomRight" state="frozen"/><selection pane="topRight" activeCell="C1" sqref="C1"/><selection pane="bottomLeft" activeCell="A2" sqref="A2"/><selection pane="bottomRight" activeCell="AH80" sqref="AH80"/></sheetView>"#,
    );
    assert_eq!(sh.view_top_left, c("M2"));
    assert_eq!(sh.view_active, c("AH80"));
}

#[test]
fn rows_frozen_only_keep_the_sheet_views_first_column() {
    let sh = sheet(
        r#"<sheetView topLeftCell="D1" workbookViewId="0"><pane ySplit="7" topLeftCell="A56" activePane="bottomLeft" state="frozen"/><selection activeCell="J1" sqref="J1"/><selection pane="bottomLeft" activeCell="D80" sqref="A80:XFD80"/></sheetView>"#,
    );
    assert_eq!(sh.freeze, Some((7, 0)));
    assert_eq!(sh.view_top_left, c("D56"));
    assert_eq!(sh.view_active, c("D80"));
    // Columns frozen only: the rows come from the sheet view.
    let sh = sheet(
        r#"<sheetView topLeftCell="A30" workbookViewId="0"><pane xSplit="2" topLeftCell="F1" activePane="topRight" state="frozen"/><selection pane="topRight" activeCell="G31" sqref="G31"/></sheetView>"#,
    );
    assert_eq!(sh.view_top_left, c("F30"));
    assert_eq!(sh.view_active, c("G31"));
}

#[test]
fn view_without_panes() {
    let sh = sheet(r#"<sheetView tabSelected="1" topLeftCell="A71" workbookViewId="0"><selection activeCell="D10" sqref="D10"/></sheetView>"#);
    assert_eq!((sh.view_top_left, sh.view_active, sh.freeze), (c("A71"), c("D10"), None));
    // A selection naming a pane that isn't there still gives the active cell.
    let sh = sheet(r#"<sheetView workbookViewId="0"><selection pane="bottomRight" activeCell="C3" sqref="C3"/></sheetView>"#);
    assert_eq!(sh.view_active, c("C3"));
}

fn workbook_xml(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name("xl/workbook.xml").unwrap().read_to_string(&mut s).unwrap();
    s
}

#[test]
fn first_tab_round_trips() {
    let mut wb = Workbook::new();
    wb.sheets = ["One", "Two", "Three", "Four"].iter().map(|n| std::sync::Arc::new(Sheet::new(*n))).collect();
    wb.active_sheet = 3;
    wb.first_tab = 2;
    let bytes = write_xlsx(&wb).unwrap();
    assert!(workbook_xml(&bytes).contains(r#"firstSheet="2" activeTab="3""#), "{}", workbook_xml(&bytes));
    let (back, _) = read_xlsx(&bytes).unwrap();
    assert_eq!((back.first_tab, back.active_sheet), (2, 3));
    // Never past the active tab; the first tab isn't written.
    wb.first_tab = 9;
    assert!(workbook_xml(&write_xlsx(&wb).unwrap()).contains(r#"firstSheet="3""#));
    wb.first_tab = 0;
    assert!(!workbook_xml(&write_xlsx(&wb).unwrap()).contains("firstSheet"));
}

#[test]
fn first_tab_on_a_chart_sheet_moves_to_the_next_worksheet() {
    use super::{CT, ROOT_RELS, make_zip};
    let ns = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
    let wb = format!(
        r#"<workbook {ns}><bookViews><workbookView firstSheet="1" activeTab="2"/></bookViews><sheets><sheet name="One" sheetId="1" r:id="rId1"/><sheet name="Chart" sheetId="2" r:id="rId2"/><sheet name="Two" sheetId="3" r:id="rId3"/></sheets></workbook>"#
    );
    let rel = |id: &str, kind: &str, target: &str| {
        format!(r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{target}"/>"#)
    };
    let rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{}{}{}</Relationships>"#,
        rel("rId1", "worksheet", "worksheets/sheet1.xml"),
        rel("rId2", "chartsheet", "chartsheets/sheet1.xml"),
        rel("rId3", "worksheet", "worksheets/sheet2.xml")
    );
    let sheet = format!("<worksheet {ns}><sheetData/></worksheet>");
    let chart = format!("<chartsheet {ns}><sheetViews><sheetView workbookViewId=\"0\"/></sheetViews></chartsheet>");
    let bytes = make_zip(&[
        ("[Content_Types].xml", CT.as_bytes()),
        ("_rels/.rels", ROOT_RELS.as_bytes()),
        ("xl/workbook.xml", wb.as_bytes()),
        ("xl/_rels/workbook.xml.rels", rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
        ("xl/chartsheets/sheet1.xml", chart.as_bytes()),
        ("xl/worksheets/sheet2.xml", sheet.as_bytes()),
    ]);
    let (wb, _) = read_xlsx(&bytes).unwrap();
    assert_eq!(wb.sheets.len(), 2);
    assert_eq!((wb.first_tab, wb.active_sheet), (1, 1));
}
