//! Reading the HTML flavour of the system clipboard on paste.
//!
//! Spreadsheets put an HTML table on the clipboard next to the tab-separated text. The text holds
//! what each cell *shows* ("Rp10,000.00"), which en-US input parsing can't always turn back into a
//! number. The HTML of the common sources also carries the underlying number as a cell attribute:
//! `x:num="10000"` (Excel, Excel for the web) and `sdval="10000"` (LibreOffice Calc). Paste uses
//! it to keep such cells numeric (#257). The markup is read by `gridcraft_xlsx::table_rows`; only
//! the table structure, the cell text and these two attributes are used here.

/// Largest clipboard HTML we look at; past this the paste uses the plain text alone.
const MAX_HTML: usize = 16 * 1024 * 1024;
/// Widest `colspan` honoured (the sheet's column count).
const MAX_SPAN: usize = gridcraft_core::MAX_COLS as usize;

/// One table cell: its visible text and the number the source recorded for it, if any.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct HtmlCell {
    pub text: String,
    pub num: Option<f64>,
}

/// The cells of the first table in `html`, row by row; a cell spanning columns is followed by
/// empty cells so positions line up with the tab-separated text. Empty when there is no table.
pub(crate) fn table_cells(html: &str) -> Vec<Vec<HtmlCell>> {
    if html.len() > MAX_HTML {
        return Vec::new();
    }
    gridcraft_xlsx::table_rows(html, true)
        .into_iter()
        .map(|row| {
            let mut cells = Vec::with_capacity(row.cells.len());
            for c in row.cells {
                let span = c.colspan as usize;
                cells.push(HtmlCell { text: c.text, num: c.num });
                let room = MAX_SPAN.saturating_sub(cells.len());
                cells.extend(std::iter::repeat_n(HtmlCell::default(), span.saturating_sub(1).min(room)));
            }
            cells
        })
        .collect()
}

/// Same-looking text: non-breaking spaces as spaces, whitespace runs collapsed, trimmed.
pub(crate) fn normalized(s: &str) -> String {
    s.replace('\u{a0}', " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A number format that shows `n` exactly as `shown` ("Rp10,000.00" for 10000 →
/// `"Rp"#,##0.00`), inferred from the text around the digits. `None` unless the format renders
/// `shown` back character for character.
pub(crate) fn format_for(shown: &str, n: f64) -> Option<String> {
    let shown = normalized(shown);
    let start = shown.find(|c: char| c.is_ascii_digit())?;
    let end = shown.rfind(|c: char| c.is_ascii_digit()).map(|i| i + 1)?;
    let core = shown.get(start..end)?;
    let mut prefix = shown.get(..start)?.to_string();
    let suffix = shown.get(end..)?;
    if core.chars().any(|c| !(c.is_ascii_digit() || c == ',' || c == '.')) {
        return None;
    }
    if n < 0.0 {
        // A one-section format shows the minus sign itself, in front of everything.
        prefix = prefix.strip_prefix('-')?.to_string();
    }
    let decimals = core.split_once('.').map_or(0, |(_, f)| f.len());
    let mut code = String::new();
    let quote = |s: &str| if s.is_empty() { String::new() } else { format!("\"{}\"", s.replace('"', "")) };
    code.push_str(&quote(&prefix));
    code.push_str(if core.contains(',') { "#,##0" } else { "0" });
    if decimals > 0 {
        code.push('.');
        code.push_str(&"0".repeat(decimals.min(30)));
    }
    code.push_str(&quote(suffix));
    let rendered = gridcraft_numfmt::text_function(&gridcraft_core::Value::number(n), &code, gridcraft_core::date::DateSystem::D1900).ok()?;
    (normalized(&rendered) == shown).then_some(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_excel_and_libreoffice_cell_values() {
        let excel = r#"<html><head><style>td{mso-number-format:General}</style></head><body><table>
            <!--StartFragment--><tr height=20><td class=xl65 x:num="10000">Rp10,000.00</td><td x:str>Name</td></tr>
            <tr><td colspan=2 x:num="2.5">2.50 &amp; up</td><td>x</td></tr><!--EndFragment--></table></body></html>"#;
        let rows = table_cells(excel);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0], HtmlCell { text: "Rp10,000.00".into(), num: Some(10000.0) });
        assert_eq!(rows[0][1], HtmlCell { text: "Name".into(), num: None });
        assert_eq!(rows[1].len(), 3, "colspan keeps the next cell in its column");
        assert_eq!(rows[1][0].text, "2.50 & up");
        assert_eq!(rows[1][2].text, "x");

        let calc = r#"<table cellspacing="0" border="0"><colgroup width="85"></colgroup><tr>
            <td height="17" align="right" sdval="10000" sdnum="1033;0;[$Rp-421]#,##0.00"><font face="Liberation Sans">Rp10,000.00</font></td></tr></table>"#;
        assert_eq!(table_cells(calc), vec![vec![HtmlCell { text: "Rp10,000.00".into(), num: Some(10000.0) }]]);

        let quoted_gt = r#"<table><tr><td style='mso-number-format:"\[>=1000\]0"' x:num="1500">1500</td></tr></table>"#;
        assert_eq!(table_cells(quoted_gt), vec![vec![HtmlCell { text: "1500".into(), num: Some(1500.0) }]]);
    }

    #[test]
    fn hostile_markup_does_not_panic() {
        for html in [
            "",
            "<",
            "<td",
            "<table><td x:num=>",
            "<table><tr><td colspan=999999999>a",
            "&#xFFFFFFFF;&",
            "<table><td x:num='1e999'>1</td>",
            "<!--",
            "<style>",
            "<table><table><td>",
        ] {
            let _ = table_cells(html);
        }
        assert_eq!(table_cells("<table><td x:num='1e999'>1</td></table>")[0][0].num, None);
        assert!(table_cells("<table><tr><td colspan=999999999>a</td></tr></table>")[0].len() <= MAX_SPAN);
    }

    #[test]
    fn infers_a_format_that_shows_the_same_text() {
        assert_eq!(format_for("Rp10,000.00", 10000.0).as_deref(), Some("\"Rp\"#,##0.00"));
        assert_eq!(format_for("1 234 kr", 1234.0), None, "space grouping is not reproduced");
        assert_eq!(format_for("10.000,00 €", 10000.0), None, "comma decimals don't render back in en-US");
        assert_eq!(format_for("-Rp5", -5.0).as_deref(), Some("\"Rp\"0"));
        assert_eq!(format_for("12 EUR", 12.0).as_deref(), Some("0\" EUR\""));
    }
}
