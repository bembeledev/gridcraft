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
