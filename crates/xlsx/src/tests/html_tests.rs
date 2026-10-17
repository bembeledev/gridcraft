use gridcraft_core::{CellRef, RangeRef, Value};
use gridcraft_model::Workbook;

use crate::{Format, is_html, read_html, sniff, table_rows};

fn v(wb: &Workbook, r: u32, c: u32) -> Value {
    wb.sheet(0).unwrap().value(CellRef::new(r, c))
}

fn fmt(wb: &Workbook, r: u32, c: u32) -> String {
    let sh = wb.sheet(0).unwrap();
    wb.styles.get(sh.style_id(CellRef::new(r, c))).num_fmt.as_str().to_string()
}

fn bold(wb: &Workbook, r: u32, c: u32) -> bool {
    let sh = wb.sheet(0).unwrap();
    wb.styles.get(sh.style_id(CellRef::new(r, c))).font.bold
}

/// The shape of a bank's "Excel" export: an XHTML page in Latin-1 named `.xls`, a style sheet of
/// `#id` rules, self-closing spacer cells, `<font>` wrapped around cells, continental numbers.
const STATEMENT: &str = "\t<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Strict//EN\" \"http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd\">
<html xmlns=\"http://www.w3.org/1999/xhtml\" lang=\"es\"><head><STYLE type=\"text/css\">
  #Spacer {width:10pt}
  #Head {text-align:center;border-bottom:.5pt solid windowtext}
</STYLE><meta http-equiv=\"Content-Type\" content=\"text/html; charset=iso-8859-1\" /></head>
<body><table>
<tr style=\"height:31.5pt\"><td id=\"Spacer\" /><td><b>Lista de Movimientos</b></td></tr>
<tr><td id=\"Spacer\" /><font><td>Saldo:</td><td><b>1.500,25 EUR</b></td></font></tr>
<tr><td id=\"Spacer\" /><td id=\"Head\" width=\"95\"><b>Fecha Operaci\u{F3}n</b></td><td id=\"Head\" width=\"172\"><b>Importe</b></td><td><b>Saldo</b></td></tr>
<tr style=\"height:30pt\"><td id=\"Spacer\" /><td align=\"center\"><font>16/10/2026</font></td><td><font>-17,35</font></td><td><font>1.500,25</font></td></tr>
<tr><td id=\"Spacer\" /><td><font>05/10/2026</font></td><td><font>-260,00</font></td><td><font>1.517,60</font></td></tr>
<tr><td id=\"Spacer\" /><td>Compra &amp; pago, 12,5%</td><td>1.234</td><td x:str>007</td></tr>
</table></body></html>";

fn latin1(s: &str) -> Vec<u8> {
    s.chars().map(|c| u8::try_from(u32::from(c)).unwrap()).collect()
}

#[test]
fn bank_statement_saved_as_xls_opens_with_numbers_and_dates() {
    let bytes = latin1(STATEMENT);
    assert!(is_html(&bytes));
    assert_eq!(sniff(&bytes), Format::Csv, "HTML is text to the sniffer; the caller asks is_html");
    let (wb, report) = read_html(&bytes).unwrap();
    assert!(report.warnings.is_empty());
    assert_eq!(v(&wb, 0, 1), Value::text("Lista de Movimientos"));
    assert!(bold(&wb, 0, 1));
    assert_eq!(v(&wb, 1, 1), Value::text("Saldo:"));
    assert!(!bold(&wb, 1, 1));
    assert_eq!(v(&wb, 1, 2), Value::text("1.500,25 EUR"), "a number with a unit stays text, as written");
    assert_eq!(v(&wb, 2, 1), Value::text("Fecha Operación"), "Latin-1 decoded");

    // Day-first dates, also when the day is 12 or less.
    let Value::Number(d1) = v(&wb, 3, 1) else { panic!("date not read: {:?}", v(&wb, 3, 1)) };
    assert_eq!(d1, 46311.0); // 2026-10-16
    assert_eq!(fmt(&wb, 3, 1), "dd/mm/yyyy");
    assert_eq!(v(&wb, 4, 1), Value::Number(46300.0)); // 2026-10-05, not May 10

    // Comma decimals, dot grouping.
    assert_eq!(v(&wb, 3, 2), Value::Number(-17.35));
    assert_eq!(v(&wb, 3, 3), Value::Number(1500.25));
    assert_eq!(fmt(&wb, 3, 3), "#,##0.00");
    assert_eq!(v(&wb, 4, 2), Value::Number(-260.0));
    assert_eq!(v(&wb, 5, 2), Value::Number(1234.0), "a dot before three digits groups");
    assert_eq!(v(&wb, 5, 1), Value::text("Compra & pago, 12,5%"));
    assert_eq!(v(&wb, 5, 3), Value::text("007"), "x:str keeps text");

    let sh = wb.sheet(0).unwrap();
    let spacer = sh.cols.get(&0).and_then(|c| c.size).unwrap();
    assert!((spacer - 40.0 / 3.0).abs() < 1e-3, "width from the #id rule (10pt): {spacer}");
    assert_eq!(sh.cols.get(&1).and_then(|c| c.size), Some(95.0), "width=95 px");
    assert_eq!(sh.rows.get(&0).and_then(|r| r.size), Some(42.0));
    assert_eq!(sh.rows.get(&3).and_then(|r| r.size), Some(40.0));
}

#[test]
fn dot_decimal_pages_read_the_us_way() {
    let html = "<html><body><table><tr><td>3/4/2026</td><td>1,234.50</td><td>2.5</td><td>1,234</td></tr></table></body></html>";
    let (wb, _) = read_html(html.as_bytes()).unwrap();
    assert_eq!(v(&wb, 0, 0), Value::Number(46085.0)); // March 4
    assert_eq!(v(&wb, 0, 1), Value::Number(1234.5));
    assert_eq!(v(&wb, 0, 2), Value::Number(2.5));
    assert_eq!(v(&wb, 0, 3), Value::Number(1234.0));
}

#[test]
fn excel_html_values_spans_and_several_tables() {
    let html = r#"<html xmlns:o="urn:schemas-microsoft-com:office:office"><head><style>.h {font-weight:700}</style></head><body>
        <table><tr><th rowspan=2>Region</th><td colspan=2 class=h>Sales</td></tr>
        <tr><td x:num="1000.5">1,000.50 €</td><td>2</td></tr>
        <tr><td>North</td><td>3</td><td>4</td></tr></table>
        <p>between</p>
        <table><tr><td>second</td></tr></table></body></html>"#;
    let (wb, _) = read_html(html.as_bytes()).unwrap();
    let sh = wb.sheet(0).unwrap();
    assert_eq!(v(&wb, 0, 0), Value::text("Region"));
    assert!(bold(&wb, 0, 0), "<th> is bold");
    assert!(bold(&wb, 0, 1), "class rule font-weight:700");
    assert_eq!(v(&wb, 1, 1), Value::Number(1000.5), "x:num wins; the row-spanned column is skipped");
    assert_eq!(v(&wb, 1, 2), Value::Number(2.0));
    assert_eq!(v(&wb, 2, 0), Value::text("North"));
    assert_eq!(v(&wb, 3, 0), Value::text("second"), "the next table follows");
    assert!(sh.merges.contains(&RangeRef::new(CellRef::new(0, 0), CellRef::new(1, 0))));
    assert!(sh.merges.contains(&RangeRef::new(CellRef::new(0, 1), CellRef::new(0, 2))));
}

#[test]
fn detection() {
    for yes in [
        "<!DOCTYPE html><html>",
        "\u{FEFF}  <html>",
        "<?xml version=\"1.0\"?>\n<!-- x --><html xmlns=\"http://www.w3.org/1999/xhtml\">",
        "<table><tr><td>1</td></tr></table>",
        "\n<HTML>",
    ] {
        assert!(is_html(yes.as_bytes()), "{yes}");
    }
    for no in ["a,b,c\n1,2,3", "<a>,<b>\n1,2", "<!DOCTYPE foo>", "<?xml version=\"1.0\"?><Workbook>", "", "<!--"] {
        assert!(!is_html(no.as_bytes()), "{no}");
    }
    assert!(read_html(b"<html><body><p>no grid</p></body></html>").is_err());
}

#[test]
fn hostile_markup_is_bounded() {
    for html in [
        "<table><tr><td rowspan=999999999 colspan=999999999>a</td><td>b</td></tr><tr><td>c</td></tr></table>",
        "<table><td width='1e999' style='width:-5pt;height:NaNpt'>1</td></table>",
        "<style>{}{{}}#{width:1pt} . {font-weight:9999999999999}</style><table><td>1",
        "<table><table><tr><td>x</td></table></table></table><td>y",
        "<table><td>&#xFFFFFFFF;&euro;&bogus;&",
        "<table></b></b></strong><td>z",
    ] {
        let _ = read_html(html.as_bytes());
        let _ = table_rows(html, true);
    }
    let (wb, _) = read_html(b"<table><tr><td rowspan=999999999 colspan=999999999>a</td><td>b</td></tr><tr><td>c</td></tr></table>").unwrap();
    assert_eq!(v(&wb, 0, 0), Value::text("a"));
    assert_eq!(wb.sheet(0).unwrap().cells.iter().count(), 1, "b and c fall off the grid");
}
