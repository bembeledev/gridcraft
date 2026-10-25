//! Data ▸ Get Data ▸ From Web: fetch a page through the host's [`crate::webquery::WebFetch`],
//! preview its tables and load one into a new sheet, with a saved connection `data.webRefresh`
//! re-fetches (Excel's Navigator, Load and Refresh).

use serde_json::{Value as Json, json};

use super::*;
use crate::webquery::{DEFAULT_MAX_BYTES, DEFAULT_MAX_ROWS, MAX_COLS, WebRequest, WebTable};

/// Preview rows and columns per table (the Navigator's preview pane).
const PREVIEW_ROWS: usize = 10;
const PREVIEW_COLS: usize = 8;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "data.fromWeb",
            "From Web",
            ["Data", "Get & Transform Data"],
            None,
            "{url: https page, table?: page table name (default: the first), sheet?: new sheet name, maxRows?: 1..1048575} → imports the table into a new sheet as a table and saves the connection",
            has_doc,
            from_web
        ),
        cmd!(
            query "data.webPreview",
            "Web Tables Preview",
            [],
            None,
            "{url} → {tables: [{name, rows, cols, preview: first rows}]} (the Navigator's table detection)",
            has_doc,
            web_preview
        ),
        cmd!(
            "data.webRefresh",
            "Refresh Web Table",
            ["Data", "Queries & Connections"],
            None,
            "{url? | name?: the saved connection (default: all)} → re-fetches and rewrites the imported table(s) in place",
            has_doc,
            web_refresh
        ),
    ]
}

/// Wall-clock ms since the epoch; `0` on wasm, where reading the clock traps.
#[allow(clippy::disallowed_methods)]
fn now_ms() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
}

/// A page request from the command params, with the caps clamped.
fn request(p: &Json) -> Result<WebRequest> {
    const ID: &str = "data.fromWeb";
    let url = str_param(p, "url").map(str::trim).filter(|u| u.starts_with("http")).ok_or_else(|| bad(ID, "missing `url`"))?;
    let max_rows = p
        .get("maxRows")
        .and_then(Json::as_u64)
        .map_or(DEFAULT_MAX_ROWS, |n| usize::try_from(n).unwrap_or(DEFAULT_MAX_ROWS).clamp(1, DEFAULT_MAX_ROWS));
    Ok(WebRequest { url: url.to_string(), max_rows, max_cols: MAX_COLS, max_bytes: DEFAULT_MAX_BYTES })
}

fn from_web(s: &mut Session, p: &Json) -> Result<Json> {
    if str_param(p, "url").map(|u| u.trim().is_empty()).unwrap_or(true) {
        s.ui_requests.push(crate::UiRequest::Dialog("fromWeb".into(), json!({})));
        return Ok(Json::Null);
    }
    let req = request(p)?;
    let Some(source) = s.web_source.clone() else {
        return Err(EngineError::Other("Web page import is not available in this build.".into()));
    };
    let tables = source(&req).map_err(EngineError::Other)?;
    let want = str_param(p, "table").map(str::trim).filter(|t| !t.is_empty());
    let table = pick_table(&tables, want).ok_or_else(|| EngineError::Other(no_table_message(&tables)))?;
    let name = str_param(p, "sheet").map(|n| n.chars().take(31).collect::<String>()).unwrap_or_else(|| default_sheet_name(&req.url));
    load_table(s, &req, table, &name, true)
}

/// The site's own name (`https://www.example.com/x` → `example`), capped at 31 characters.
/// Country suffixes that register under a second level (`.co.uk`, `.com.au`) are skipped, so
/// `https://sub.example.co.uk` is `example` too.
fn default_sheet_name(url: &str) -> String {
    const SECOND_LEVEL_CCTLDS: [&str; 12] = ["uk", "au", "jp", "br", "cn", "in", "kr", "nz", "za", "sg", "hk", "tw"];
    const SECOND_LEVEL_PREFIXES: [&str; 9] = ["co", "com", "org", "net", "gov", "ac", "edu", "or", "ne"];
    let authority = url.trim_start_matches("https://").split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or_default().split(':').next().unwrap_or_default();
    let labels: Vec<&str> = host.split('.').filter(|p| !p.is_empty() && *p != "www").collect();
    let base = match labels.len() {
        0 => "Web",
        1 => labels[0],
        n => {
            let last = labels[n - 1];
            let second = labels[n - 2];
            if SECOND_LEVEL_CCTLDS.contains(&last) && SECOND_LEVEL_PREFIXES.contains(&second) && n > 2 { labels[n - 3] } else { second }
        }
    };
    base.chars().take(31).collect()
}

/// The table `want` names (page name or 1-based index), else the first one.
fn pick_table<'a>(tables: &'a [WebTable], want: Option<&str>) -> Option<&'a WebTable> {
    match want {
        Some(w) => {
            tables.iter().find(|t| t.name.eq_ignore_ascii_case(w)).or_else(|| w.parse::<usize>().ok().and_then(|n| tables.get(n.saturating_sub(1))))
        }
        None => tables.first(),
    }
}

fn no_table_message(tables: &[WebTable]) -> String {
    if tables.is_empty() {
        "No tables were found on the page.".into()
    } else {
        format!("The page has no table by that name. It holds: {}.", tables.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", "))
    }
}

/// Writes `table` to a new sheet as a table and (when `save`) records the connection.
pub(crate) fn load_table(s: &mut Session, req: &WebRequest, table: &WebTable, sheet_name: &str, save: bool) -> Result<Json> {
    s.grouped("From Web", |s| load_table_steps(s, req, table, sheet_name, save))
}

/// The import as one undo step: a failure after the sheet was inserted rolls the whole
/// workbook back, so no empty sheet or half-written table is left behind.
fn load_table_steps(s: &mut Session, req: &WebRequest, table: &WebTable, sheet_name: &str, save: bool) -> Result<Json> {
    let rows = json_rows(table);
    if rows.is_empty() {
        return Err(EngineError::Other("The table is empty.".into()));
    }
    s.execute("home.insertSheet", json!({"name": sheet_name})).or_else(|_| s.execute("home.insertSheet", json!({})))?;
    let r = s.doc()?;
    let sheet = r.wb.active_sheet;
    let sheet_name = r.wb.sheet(sheet).map(|sh| sh.name.clone()).unwrap_or_default();
    s.execute("range.setValues", json!({"range": "A1", "sheet": sheet, "values": rows}))?;
    let w = table.rows.iter().map(Vec::len).max().unwrap_or(1);
    let range = RangeRef::new(CellRef::new(0, 0), CellRef::new(table.rows.len() as u32 - 1, w as u32 - 1));
    // The table takes the page table's name (spaces and other characters a table name cannot
    // hold become `_`); a taken name gets a generated one, and the connection saves the name
    // that was actually created.
    let sanitize = |n: &str| n.chars().take(255).map(|c| if c.is_alphanumeric() || c == '_' || c == '.' { c } else { '_' }).collect::<String>();
    let mut tname = sanitize(&table.name);
    if !tname.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_' || c == '\\') {
        tname = format!("Web_{tname}");
    }
    s.execute("insert.table", json!({"range": range.a1(), "header": true, "name": tname})).or_else(|e| {
        // The name is taken or still not valid: a generated one keeps the import working.
        let taken = e.to_string().contains("already exists");
        let generated = s
            .doc()
            .map(|d| (1..).map(|n| format!("Web_Table{n}")).find(|n| d.wb.table(n).is_none()).unwrap_or_else(|| "Web_Table".into()))
            .unwrap_or_else(|_| "Web_Table".into());
        if !taken && !e.to_string().contains("not valid") {
            return Err(e);
        }
        s.execute("insert.table", json!({"range": range.a1(), "header": true, "name": generated}))
    })?;
    let _ = s.execute("home.autofitColumnWidth", json!({"cols": RangeRef::cols(0, w as u32 - 1).a1()}));
    // The table that was actually created: its name may be a generated one when the page
    // table's name was taken or not valid. The connection and the result both report it.
    let created_name = {
        let d = s.doc()?;
        d.wb.sheet(sheet).and_then(|sh| sh.tables.iter().find(|t| t.range == range).map(|t| t.name.clone())).unwrap_or_else(|| tname.clone())
    };
    if save {
        let synced = now_ms();
        let name = created_name.clone();
        let q =
            gridcraft_model::WebQuery { name, url: req.url.clone(), table: table.name.clone(), sheet: sheet_name.clone(), last_synced: Some(synced) };
        edit(s, |cx| {
            cx.wb.web_queries.retain(|x| !(x.sheet.eq_ignore_ascii_case(&q.sheet) && x.name.eq_ignore_ascii_case(&q.name)));
            cx.wb.web_queries.push(q.clone());
            cx.structural = true;
            Ok(())
        })?;
    }
    Ok(json!({"sheet": sheet_name, "table": created_name, "rows": table.rows.len(), "cols": w, "tables_found": table_count(s)}))
}

fn table_count(s: &Session) -> usize {
    s.doc().map(|d| d.wb.web_queries.len()).unwrap_or(0)
}

/// The table as `range.setValues` rows: cell text parsed like typed input, so numbers, dates and
/// booleans keep their types (Excel's column detection). A cell that would become a formula
/// (`=`, or `+`/`-`/`@` starting one) is escaped with the leading apostrophe, which parses as
/// the text itself: page content never becomes live formulas (CSV injection).
fn json_rows(table: &WebTable) -> Vec<Vec<Json>> {
    table
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|c| {
                    if c.is_empty() {
                        Json::Null
                    } else if would_be_formula(c) {
                        json!(format!("'{c}"))
                    } else {
                        json!(c)
                    }
                })
                .collect()
        })
        .collect()
}

/// True when typed-input parsing would make `c` a formula: `=` always, and `+`/`-`/`@` only
/// in the forms that start one (a plain `-5` stays the number it is). Mirrors the check in
/// `input_to_cell` (cmd/edit.rs).
fn would_be_formula(c: &str) -> bool {
    c.starts_with('=')
        || (c.len() > 1
            && c.starts_with(['+', '-', '@'])
            && gridcraft_core::parse::parse_number_text(c).is_none()
            && c.chars().nth(1).is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '('))
}

fn web_preview(s: &mut Session, p: &Json) -> Result<Json> {
    let req = request(p)?;
    let Some(source) = s.web_source.clone() else {
        return Err(EngineError::Other("Web page import is not available in this build.".into()));
    };
    let tables = source(&req).map_err(EngineError::Other)?;
    let preview = |t: &WebTable| {
        json!({
            "name": t.name,
            "rows": t.rows.len(),
            "cols": t.rows.iter().map(Vec::len).max().unwrap_or(0),
            "preview": t.rows.iter().take(PREVIEW_ROWS).map(|r| r.iter().take(PREVIEW_COLS).map(|c| json!(c)).collect::<Vec<_>>()).collect::<Vec<_>>(),
        })
    };
    Ok(json!({"url": req.url, "tables": tables.iter().map(preview).collect::<Vec<_>>()}))
}

fn web_refresh(s: &mut Session, p: &Json) -> Result<Json> {
    const ID: &str = "data.webRefresh";
    let url = str_param(p, "url").map(str::trim).filter(|u| !u.is_empty());
    let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty());
    let Some(source) = s.web_source.clone() else {
        return Err(EngineError::Other("Web page import is not available in this build.".into()));
    };
    let queries: Vec<gridcraft_model::WebQuery> = {
        let d = s.doc()?;
        d.wb.web_queries
            .iter()
            .filter(|q| url.is_none_or(|u| q.url.eq_ignore_ascii_case(u)) && name.is_none_or(|n| q.name.eq_ignore_ascii_case(n)))
            .cloned()
            .collect()
    };
    if queries.is_empty() {
        return Err(EngineError::Other("No saved web table matches.".into()));
    }
    let mut refreshed = 0;
    let mut errors: Vec<Json> = Vec::new();
    for q in queries {
        let req = WebRequest { url: q.url.clone(), max_rows: DEFAULT_MAX_ROWS, max_cols: MAX_COLS, max_bytes: DEFAULT_MAX_BYTES };
        match source(&req).map_err(EngineError::Other).and_then(|tables| {
            let table = pick_table(&tables, Some(&q.table)).ok_or_else(|| EngineError::Other(no_table_message(&tables)))?;
            rewrite_table(s, &q, table)
        }) {
            Ok((rows, cols)) => {
                refreshed += 1;
                log::debug!("webRefresh {}: {} rows", q.name, rows);
                let _ = cols;
            }
            Err(e) => errors.push(json!({"table": q.name, "error": e.to_string()})),
        }
    }
    let _ = ID;
    Ok(json!({"refreshed": refreshed, "errors": errors}))
}

/// Re-fetch result over the sheet range the connection's table holds: clear the old range,
/// write the new rows, and resize the table. Formulas and formatting outside the range are kept.
fn rewrite_table(s: &mut Session, q: &gridcraft_model::WebQuery, table: &WebTable) -> Result<(usize, usize)> {
    let d = s.doc()?;
    let Some((sheet, tname, old)) =
        d.wb.sheets
            .iter()
            .enumerate()
            .find_map(|(si, sh)| sh.tables.iter().find(|t| t.name.eq_ignore_ascii_case(&q.name)).map(|t| (si, t.name.clone(), t.range)))
            .or_else(|| {
                d.wb.sheets.iter().enumerate().find_map(|(si, sh)| {
                    sh.name.eq_ignore_ascii_case(&q.sheet).then(|| sh.tables.first().map(|t| (si, t.name.clone(), t.range))).flatten()
                })
            })
    else {
        return Err(EngineError::Other(format!("The sheet or table of the connection `{}` is gone.", q.name)));
    };
    let rows = json_rows(table);
    if rows.is_empty() {
        return Err(EngineError::Other("The table is empty.".into()));
    }
    let w = table.rows.iter().map(Vec::len).max().unwrap_or(1);
    // The rewrite is one undo step over the table's own place: a failure mid-way rolls the
    // old table and its data back instead of leaving the sheet empty.
    s.grouped("Refresh Web Table", |s| {
        // Clear the old range, then drop the table so a new one can take the new shape.
        s.execute("edit.clearContents", json!({"range": old.a1(), "sheet": sheet}))?;
        let remove = |s: &mut Session| {
            edit(s, |cx| {
                if let Some(sh) = cx.wb.sheet_mut(sheet) {
                    sh.tables.retain(|t| !t.name.eq_ignore_ascii_case(&tname));
                }
                cx.structural = true;
                Ok(())
            })
        };
        remove(s)?;
        // Write at the table's own origin, not A1: a moved table refreshes where it is.
        s.execute("range.setValues", json!({"range": old.start.a1(), "sheet": sheet, "values": rows}))?;
        let range = RangeRef::new(old.start, CellRef::new(old.start.row + table.rows.len() as u32 - 1, old.start.col + w as u32 - 1));
        s.execute("insert.table", json!({"range": range.a1(), "header": true, "name": tname}))?;
        let synced = now_ms();
        edit(s, |cx| {
            if let Some(qq) = cx.wb.web_queries.iter_mut().find(|x| x.sheet.eq_ignore_ascii_case(&q.sheet) && x.name.eq_ignore_ascii_case(&q.name)) {
                qq.last_synced = Some(synced);
            }
            cx.structural = true;
            Ok(())
        })?;
        Ok(())
    })?;
    Ok((table.rows.len(), w))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use gridcraft_core::Value;

    fn page() -> &'static str {
        "<table id=\"prices\"><tr><th>Sym</th><th>Price</th></tr><tr><td>ABCD</td><td>12.50</td></tr></table>\
         <table><tr><td>x</td><td>1</td></tr></table>"
    }

    fn session_with_source() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        s.web_source = Some(Arc::new(|req: &WebRequest| {
            assert!(req.url.starts_with("https://"));
            Ok(crate::webquery::extract_tables(page(), req.max_rows, req.max_cols))
        }));
        s
    }

    fn a1(s: &Session, at: &str) -> String {
        let d = s.active().unwrap();
        let sh = d.wb.active().unwrap();
        crate::display::cell_text(&d.wb, sh, CellRef::parse(at).unwrap())
    }

    #[test]
    fn imports_the_first_table_into_a_new_sheet_and_saves_the_connection() {
        let mut s = session_with_source();
        let r = s.execute("data.fromWeb", json!({"url": "https://example.com/markets", "sheet": "Markets"})).unwrap();
        assert_eq!(r["sheet"], "Markets");
        assert_eq!(r["rows"], 2);
        assert_eq!(a1(&s, "A1"), "Sym");
        assert_eq!(a1(&s, "B2"), "12.5");
        let d = s.active().unwrap();
        assert_eq!(d.wb.web_queries.len(), 1);
        assert_eq!(d.wb.web_queries[0].url, "https://example.com/markets");
        assert!(d.wb.web_queries[0].last_synced.is_some());
        assert!(d.wb.active().unwrap().tables.iter().any(|t| t.name == "prices"));
    }

    #[test]
    fn imports_a_named_table_and_keeps_text_and_numbers() {
        let mut s = session_with_source();
        s.execute("data.fromWeb", json!({"url": "https://example.com", "table": "Table 2"})).unwrap();
        assert_eq!(a1(&s, "A1"), "x");
        let d = s.active().unwrap();
        let sh = d.wb.active().unwrap();
        // The page name "Table 2" cannot be a table name (the space); the import still makes
        // the table, with a name the page table's was sanitized into.
        assert!(sh.tables.iter().any(|t| t.name == "Table_2"));
        // A one-row import is the table's header row, which is text.
        assert_eq!(sh.value(CellRef::parse("B1").unwrap()), Value::text("1"));
    }

    #[test]
    fn preview_lists_the_page_tables() {
        let mut s = session_with_source();
        let r = s.execute("data.webPreview", json!({"url": "https://example.com"})).unwrap();
        let tables = r["tables"].as_array().unwrap();
        assert_eq!(tables.len(), 2);
        assert_eq!(tables[0]["name"], "prices");
        assert_eq!(tables[0]["rows"], 2);
        assert_eq!(tables[1]["name"], "Table 2");
    }

    #[test]
    fn refresh_rewrites_the_table_in_place() {
        let mut s = session_with_source();
        s.execute("data.fromWeb", json!({"url": "https://example.com", "sheet": "Web"})).unwrap();
        // The source now serves a bigger table.
        s.web_source = Some(Arc::new(|req: &WebRequest| {
            Ok(crate::webquery::extract_tables(
                "<table id=\"prices\"><tr><th>Sym</th><th>Price</th></tr><tr><td>A</td><td>1</td></tr><tr><td>B</td><td>2</td></tr></table>",
                req.max_rows,
                req.max_cols,
            ))
        }));
        let r = s.execute("data.webRefresh", json!({})).unwrap();
        assert_eq!(r["refreshed"], 1);
        assert!(r["errors"].as_array().unwrap().is_empty());
        assert_eq!(a1(&s, "A3"), "B");
        assert_eq!(a1(&s, "B3"), "2");
        let d = s.active().unwrap();
        let sh = d.wb.active().unwrap();
        assert_eq!(sh.tables.len(), 1);
        assert_eq!(sh.tables[0].range.height(), 3);
        // The old range's leftovers are gone: nothing beyond the new table.
        assert!(sh.value(CellRef::parse("C1").unwrap()).is_empty());
    }

    #[test]
    fn default_sheet_name_is_the_site_not_the_tld() {
        let mut s = session_with_source();
        s.execute("data.fromWeb", json!({"url": "https://www.example.com/markets"})).unwrap();
        let d = s.active().unwrap();
        assert_eq!(d.wb.active().unwrap().name, "example");
        let mut s = session_with_source();
        s.execute("data.fromWeb", json!({"url": "https://sub.example.co.uk/x?a=1"})).unwrap();
        let d = s.active().unwrap();
        assert_eq!(d.wb.active().unwrap().name, "example");
    }

    #[test]
    fn a_second_import_keeps_the_first_connection() {
        let mut s = session_with_source();
        s.execute("data.fromWeb", json!({"url": "https://example.com", "sheet": "One"})).unwrap();
        s.execute("data.fromWeb", json!({"url": "https://example.com", "sheet": "Two"})).unwrap();
        let d = s.active().unwrap();
        assert_eq!(d.wb.web_queries.len(), 2);
        assert_eq!(d.wb.web_queries[0].sheet, "One");
        assert_eq!(d.wb.web_queries[1].sheet, "Two");
    }

    #[test]
    fn refresh_writes_at_the_moved_table_s_own_place() {
        let mut s = session_with_source();
        s.execute("data.fromWeb", json!({"url": "https://example.com", "sheet": "Web"})).unwrap();
        // Move the sheet's contents down two rows (values plus the table range).
        s.execute("sheet.activate", json!({"sheet": "Web"})).unwrap();
        s.execute("home.insertRows", json!({"rows": "1:2"})).unwrap();
        s.web_source = Some(Arc::new(|req: &WebRequest| {
            Ok(crate::webquery::extract_tables(
                "<table id=\"prices\"><tr><th>Sym</th><th>Price</th></tr><tr><td>A</td><td>1</td></tr></table>",
                req.max_rows,
                req.max_cols,
            ))
        }));
        let r = s.execute("data.webRefresh", json!({})).unwrap();
        assert_eq!(r["refreshed"], 1);
        // The header sits where the moved table is, and A1 stayed empty.
        assert_eq!(a1(&s, "A3"), "Sym");
        assert_eq!(a1(&s, "A1"), "");
    }

    #[test]
    fn the_result_and_connection_name_the_created_table() {
        let mut s = session_with_source();
        // "prices" is a valid name, so the first import takes it; the second, from another
        // sheet, must fall back to a generated name and report it.
        s.execute("data.fromWeb", json!({"url": "https://example.com", "sheet": "One"})).unwrap();
        let r = s.execute("data.fromWeb", json!({"url": "https://example.com", "sheet": "Two"})).unwrap();
        let made = r["table"].as_str().unwrap();
        assert_ne!(made, "prices");
        assert!(made.starts_with("Web_Table"));
        let d = s.active().unwrap();
        assert!(d.wb.web_queries.iter().any(|q| q.name == made));
    }

    #[test]
    fn page_content_never_becomes_a_formula() {
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        s.web_source = Some(Arc::new(|req: &WebRequest| {
            Ok(crate::webquery::extract_tables(
                "<table id=\"prices\"><tr><th>A</th><th>B</th></tr>\
                 <tr><td>=HYPERLINK(\"http://evil\")</td><td>-5</td></tr></table>",
                req.max_rows,
                req.max_cols,
            ))
        }));
        s.execute("data.fromWeb", json!({"url": "https://example.com", "sheet": "Web"})).unwrap();
        let d = s.active().unwrap();
        let sh = d.wb.active().unwrap();
        // The `=` cell is the text it came as, not a live formula.
        let c = sh.cell(CellRef::parse("A2").unwrap()).unwrap();
        assert!(c.formula.is_none());
        assert_eq!(c.value, gridcraft_core::Value::text("=HYPERLINK(\"http://evil\")"));
        // A plain negative number is still a number.
        assert_eq!(sh.value(CellRef::parse("B2").unwrap()), Value::number(-5.0));
    }

    #[test]
    fn errors_without_source_bad_url_or_missing_table() {
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        let err = s.execute("data.fromWeb", json!({"url": "https://x.com"})).unwrap_err();
        assert!(err.to_string().contains("not available"));
        assert!(s.execute("data.fromWeb", json!({"url": "ftp://x"})).is_err());
        assert!(s.execute("data.fromWeb", json!({})).is_ok()); // opens the dialog
        let mut s = session_with_source();
        let err = s.execute("data.fromWeb", json!({"url": "https://example.com", "table": "nope"})).unwrap_err();
        assert!(err.to_string().contains("no table by that name") || err.to_string().to_lowercase().contains("no table"));
        assert!(s.execute("data.webRefresh", json!({"name": "nothing"})).is_err());
    }

    #[test]
    fn headless_empty_params_fail_instead_of_opening_a_dialog() {
        let mut s = session_with_source();
        assert!(s.execute_headless("data.fromWeb", json!({})).is_err());
    }

    #[test]
    fn every_command_survives_empty_params() {
        let mut s = session_with_source();
        for id in ["data.fromWeb", "data.webPreview", "data.webRefresh"] {
            let _ = s.execute(id, json!({}));
        }
    }
}
