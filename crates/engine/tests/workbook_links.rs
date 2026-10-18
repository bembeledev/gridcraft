//! Data › Workbook Links and File › Info › Edit Links: updating linked workbooks' values from
//! their files, Check Status, Change Source, Break Link, the startup prompt, and the disabled
//! automatic update when a workbook with links is opened. Source files are written by the tests.
#![cfg(not(target_arch = "wasm32"))]

use std::io::Read as _;
use std::path::{Path, PathBuf};

use gridcraft_core::{CellError, CellRef, Value};
use gridcraft_engine::cmd::workbook_links::{LinkFiles, LinkStatus, NO_FILES};
use gridcraft_engine::{EngineError, Session, UiRequest};
use serde_json::{Value as Json, json};

/// A fresh folder for one test.
fn folder(name: &str) -> PathBuf {
    #[allow(clippy::disallowed_methods)] // native-only test: a unique temp dir name
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("gridcraft-wblinks-{name}-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(s: &mut Session, id: &str, params: Json) -> Json {
    s.execute(id, params.clone()).unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn set(s: &mut Session, cell: &str, input: &str) {
    run(s, "cell.set", json!({"cell": cell, "input": input}));
}

fn value(s: &Session, cell: &str) -> Value {
    s.doc().unwrap().wb.active().unwrap().value(CellRef::parse(cell).unwrap())
}

fn formula(s: &Session, cell: &str) -> Option<String> {
    s.doc().unwrap().wb.active().unwrap().cell(CellRef::parse(cell).unwrap()).and_then(|c| c.formula.as_ref()).map(|f| f.text.clone())
}

/// A source workbook at `path`: Sheet1!A1:A3 = `a`, 20, 30, B1 = 0.05 named `Rate`, A4 = SUM, and
/// a second sheet `Prices` with B2 = `price`.
fn write_source(path: &Path, a: f64, price: f64) {
    let mut s = Session::new();
    s.new_workbook();
    set(&mut s, "A1", &a.to_string());
    set(&mut s, "A2", "20");
    set(&mut s, "A3", "30");
    set(&mut s, "B1", "0.05");
    set(&mut s, "A4", "=SUM(A1:A3)");
    run(&mut s, "formulas.defineName", json!({"name": "Rate", "refersTo": "=Sheet1!$B$1"}));
    run(&mut s, "home.insertSheet", json!({"name": "Prices"}));
    run(&mut s, "cell.set", json!({"cell": "B2", "input": price.to_string(), "sheet": "Prices"}));
    run(&mut s, "file.saveAs", json!({"path": path}));
}

/// A workbook saved as `Dest.xlsx` in `dir` that links to `Source.xlsx` there.
fn dest(dir: &Path) -> Session {
    let mut s = Session::new();
    s.new_workbook();
    run(&mut s, "file.saveAs", json!({"path": dir.join("Dest.xlsx")}));
    set(&mut s, "A1", "=[Source.xlsx]Sheet1!A1");
    set(&mut s, "A2", "=SUM([Source.xlsx]Sheet1!A1:A3)+C1");
    set(&mut s, "A3", "=Source.xlsx!Rate");
    set(&mut s, "A4", "=[Source.xlsx]Sheet1!A4");
    set(&mut s, "A5", "=[Source.xlsx]Prices!B2*2");
    set(&mut s, "C1", "1");
    s
}

fn statuses(r: &Json) -> Vec<String> {
    r["links"].as_array().unwrap().iter().map(|l| l["status"].as_str().unwrap().to_string()).collect()
}

/// The saved package's parts by name.
fn parts(path: &Path) -> std::collections::HashMap<String, String> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(std::fs::read(path).unwrap())).unwrap();
    (0..z.len())
        .map(|i| {
            let mut f = z.by_index(i).unwrap();
            let mut t = String::new();
            f.read_to_string(&mut t).unwrap_or_default();
            (f.name().to_string(), t)
        })
        .collect()
}

#[test]
fn update_values_reads_the_source_file_and_check_status_sees_it_change() {
    let dir = folder("update");
    write_source(&dir.join("Source.xlsx"), 10.0, 4.5);
    let mut s = dest(&dir);
    // A typed link has no values until its file is read.
    assert_eq!(value(&s, "A1"), Value::Error(CellError::Ref));
    assert_eq!(formula(&s, "A1").as_deref(), Some("[1]Sheet1!A1"));

    let r = run(&mut s, "data.refreshLinks", json!({}));
    assert_eq!(r["updated"], json!(1));
    assert_eq!(statuses(&r), ["ok"]);
    assert!(r["links"][0]["lastRefreshed"].is_number());
    assert_eq!(value(&s, "A1"), Value::Number(10.0));
    assert_eq!(value(&s, "A2"), Value::Number(61.0));
    assert_eq!(value(&s, "A3"), Value::Number(0.05));
    assert_eq!(value(&s, "A4"), Value::Number(60.0), "the source's formula value");
    assert_eq!(value(&s, "A5"), Value::Number(9.0));
    // The formulas still name the link; one undo step brings the old values back.
    assert_eq!(formula(&s, "A2").as_deref(), Some("SUM([1]Sheet1!A1:A3)+C1"));
    assert_eq!(s.doc().unwrap().undo.last().map(|u| u.label.as_str()), Some("Update Values"));

    // The source changes: Check Status sees it, Update Values reads it.
    write_source(&dir.join("Source.xlsx"), 11.0, 4.5);
    let r = run(&mut s, "data.checkLinkStatus", json!({"link": 1}));
    assert_eq!(statuses(&r), ["valuesNotUpdated"]);
    assert_eq!(r["links"][0]["statusText"], json!("Warning: Values not updated"));
    assert_eq!(value(&s, "A1"), Value::Number(10.0), "checking doesn't update");
    run(&mut s, "data.refreshLinks", json!({"link": "Source.xlsx"}));
    assert_eq!((value(&s, "A1"), value(&s, "A2")), (Value::Number(11.0), Value::Number(62.0)));
    assert_eq!(statuses(&run(&mut s, "data.checkLinkStatus", json!({}))), ["ok"]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(value(&s, "A1"), Value::Number(10.0));
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(value(&s, "A1"), Value::Number(11.0));

    // Saved and opened again: the values saved with the link, its update disabled.
    run(&mut s, "file.save", json!({}));
    let saved = parts(&dir.join("Dest.xlsx"));
    let link = &saved["xl/externalLinks/externalLink1.xml"];
    assert!(link.contains("<sheetNames><sheetName val=\"Sheet1\"/><sheetName val=\"Prices\"/></sheetNames>"), "{link}");
    assert!(link.contains("<cell r=\"A1\"><v>11</v></cell>"), "{link}");
    assert!(
        link.contains("<definedName name=\"Rate\" refersTo=\"=Sheet1!$B$1\" sheetId=")
            || link.contains("<definedName name=\"Rate\" refersTo=\"=Sheet1!$B$1\"/>"),
        "{link}"
    );
    assert!(!link.contains("refreshError"), "{link}");
    let mut t = Session::new();
    run(&mut t, "file.open", json!({"path": dir.join("Dest.xlsx")}));
    assert!(t.doc().unwrap().links.updates_disabled);
    assert_eq!((value(&t, "A1"), value(&t, "A3"), value(&t, "A5")), (Value::Number(11.0), Value::Number(0.05), Value::Number(9.0)));
    let r = run(&mut t, "data.editLinks", json!({}));
    assert_eq!(r["links"][0]["name"], json!("Source.xlsx"));
    assert_eq!(r["links"][0]["type"], json!("Worksheet"));
    assert_eq!(r["links"][0]["update"], json!("A"));
    assert_eq!(r["links"][0]["status"], json!("unknown"));
    assert_eq!(r["links"][0]["formulas"], json!(5));
    assert!(t.take_ui_requests().iter().any(|q| matches!(q, UiRequest::Dialog(n, _) if n == "editLinks")));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_missing_source_keeps_the_values_and_says_source_not_found() {
    let dir = folder("missing");
    write_source(&dir.join("Source.xlsx"), 10.0, 4.5);
    let mut s = dest(&dir);
    run(&mut s, "data.refreshLinks", json!({}));
    std::fs::remove_file(dir.join("Source.xlsx")).unwrap();
    s.take_ui_requests();
    let r = run(&mut s, "data.refreshLinks", json!({}));
    assert_eq!(r["updated"], json!(0));
    assert_eq!(statuses(&r), ["sourceNotFound"]);
    assert_eq!(r["errors"][0]["statusText"], json!("Error: Source not found"));
    assert_eq!(value(&s, "A1"), Value::Number(10.0), "the values saved with the link stay");
    assert!(s.take_ui_requests().iter().any(|q| matches!(q, UiRequest::Message(m) if m.contains("Source.xlsx: Error: Source not found"))));
    assert_eq!(statuses(&run(&mut s, "data.checkLinkStatus", json!({}))), ["sourceNotFound"]);
    // A link that never had values stays #REF!.
    set(&mut s, "B1", "=[Other.xlsx]Sheet1!A1");
    let r = run(&mut s, "data.refreshLinks", json!({"link": 2}));
    assert_eq!(statuses(&r), ["sourceNotFound", "sourceNotFound"]);
    assert_eq!(value(&s, "B1"), Value::Error(CellError::Ref));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_sheet_the_source_lacks_is_ref_and_worksheet_not_found() {
    let dir = folder("sheet");
    write_source(&dir.join("Source.xlsx"), 10.0, 4.5);
    let mut s = dest(&dir);
    set(&mut s, "B1", "=[Source.xlsx]Gone!A1");
    let r = run(&mut s, "data.refreshLinks", json!({}));
    assert_eq!(statuses(&r), ["worksheetNotFound"]);
    assert_eq!(r["links"][0]["detail"], json!("Gone"));
    assert_eq!(value(&s, "B1"), Value::Error(CellError::Ref));
    assert_eq!(value(&s, "A1"), Value::Number(10.0), "the sheets it has are read");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn change_source_points_the_link_at_another_file() {
    let dir = folder("change");
    write_source(&dir.join("Source.xlsx"), 10.0, 4.5);
    write_source(&dir.join("Budget.xlsx"), 100.0, 7.0);
    let mut s = dest(&dir);
    run(&mut s, "data.refreshLinks", json!({}));
    let budget = dir.join("Budget.xlsx");
    let r = run(&mut s, "data.changeLinkSource", json!({"link": 1, "path": budget}));
    assert_eq!(r["links"][0]["name"], json!("Budget.xlsx"));
    assert_eq!(statuses(&r), ["ok"]);
    assert_eq!((value(&s, "A1"), value(&s, "A5")), (Value::Number(100.0), Value::Number(14.0)));
    assert_eq!(formula(&s, "A1").as_deref(), Some("[1]Sheet1!A1"), "the formula keeps its link");
    let d = s.doc().unwrap();
    let bar = gridcraft_engine::display::edit_text(&d.wb, d.wb.active().unwrap(), CellRef::parse("A1").unwrap());
    assert!(bar.contains("[Budget.xlsx]Sheet1'!A1"), "{bar}");
    assert_eq!(s.doc().unwrap().undo.last().map(|u| u.label.as_str()), Some("Change Source"));
    // A file that isn't there leaves the link alone.
    let e = s.execute("data.changeLinkSource", json!({"link": 1, "path": dir.join("Nope.xlsx")})).unwrap_err().to_string();
    assert!(e.contains("Nope.xlsx"), "{e}");
    assert_eq!(value(&s, "A1"), Value::Number(100.0));
    // Without a path, the dialog is asked for and nothing changes.
    s.take_ui_requests();
    run(&mut s, "data.changeLinkSource", json!({"link": 1}));
    assert!(s.take_ui_requests().iter().any(|q| matches!(q, UiRequest::Dialog(n, _) if n == "changeLinkSource")));

    // Onto a file another link reads: the two links become one.
    let mut s = dest(&dir);
    set(&mut s, "B1", "=[Budget.xlsx]Sheet1!A1");
    run(&mut s, "data.refreshLinks", json!({}));
    assert_eq!(s.doc().unwrap().wb.external_books.len(), 2);
    run(&mut s, "data.changeLinkSource", json!({"link": "Source.xlsx", "path": budget}));
    assert_eq!(s.doc().unwrap().wb.external_books.len(), 1);
    assert_eq!((formula(&s, "A1").as_deref(), formula(&s, "B1").as_deref()), (Some("[1]Sheet1!A1"), Some("[1]Sheet1!A1")));
    assert_eq!((value(&s, "A1"), value(&s, "B1")), (Value::Number(100.0), Value::Number(100.0)));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn break_link_turns_the_formulas_that_use_it_into_values() {
    let dir = folder("break");
    write_source(&dir.join("Source.xlsx"), 10.0, 4.5);
    write_source(&dir.join("Budget.xlsx"), 100.0, 7.0);
    let mut s = dest(&dir);
    set(&mut s, "B1", "=[Budget.xlsx]Sheet1!A1");
    set(&mut s, "B2", "=C1*2");
    run(&mut s, "formulas.defineName", json!({"name": "Linked", "refersTo": "=[Source.xlsx]Sheet1!$A$1:$A$3"}));
    set(&mut s, "B3", "=SUM(Linked)");
    run(&mut s, "selection.set", json!({"range": "D1:D3"}));
    run(&mut s, "cell.set", json!({"cell": "D1", "input": "=[Source.xlsx]Sheet1!A1:A3", "array": true}));
    run(&mut s, "data.refreshLinks", json!({}));
    assert_eq!(value(&s, "B3"), Value::Number(60.0));

    let r = run(&mut s, "data.breakLink", json!({"link": "Source.xlsx"}));
    // As Excel: formulas reading the link (also through a name) become their values; names stay
    // and keep the link.
    for (cell, v) in [("A1", 10.0), ("A2", 61.0), ("A3", 0.05), ("A4", 60.0), ("A5", 9.0), ("B3", 60.0), ("D1", 10.0), ("D2", 20.0), ("D3", 30.0)] {
        assert_eq!(formula(&s, cell), None, "{cell}");
        assert_eq!(value(&s, cell), Value::Number(v), "{cell}");
    }
    assert_eq!(formula(&s, "B1").as_deref(), Some("[2]Sheet1!A1"), "another link's formula stays");
    assert_eq!(formula(&s, "B2").as_deref(), Some("C1*2"));
    assert_eq!(r["kept"], json!([1]));
    assert_eq!(s.doc().unwrap().wb.names.iter().find(|n| n.name == "Linked").map(|n| n.formula.as_str()), Some("[1]Sheet1!$A$1:$A$3"));
    assert_eq!(s.doc().unwrap().undo.last().map(|u| u.label.as_str()), Some("Break Link"));

    // Without the name the link goes, and the links after it move up: [2] becomes [1].
    run(&mut s, "formulas.deleteName", json!({"name": "Linked"}));
    let r = run(&mut s, "data.breakLink", json!({"link": 1}));
    assert_eq!(r["removed"], json!([1]));
    let wb = &s.doc().unwrap().wb;
    assert_eq!(wb.external_books.len(), 1);
    assert_eq!(wb.external_books[0].file_name(), "Budget.xlsx");
    assert_eq!(formula(&s, "B1").as_deref(), Some("[1]Sheet1!A1"));
    assert_eq!(value(&s, "B1"), Value::Number(100.0));
    // Saved, the file holds one link and B1 still reads it.
    run(&mut s, "file.save", json!({}));
    let saved = parts(&dir.join("Dest.xlsx"));
    assert!(saved["xl/workbook.xml"].matches("<externalReference ").count() == 1, "{}", saved["xl/workbook.xml"]);
    assert!(saved["xl/worksheets/sheet1.xml"].contains("<f>[1]Sheet1!A1</f>"));
    // Break All: every link.
    let r = run(&mut s, "data.breakLink", json!({}));
    assert_eq!(r["removed"], json!([1]));
    assert!(s.doc().unwrap().wb.external_books.is_empty());
    assert_eq!((formula(&s, "B1"), value(&s, "B1")), (None, Value::Number(100.0)));
    assert!(s.execute("data.workbookLinks", json!({})).is_err(), "no links: Workbook Links is off, as in Excel");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_startup_prompt_is_saved_with_the_workbook() {
    let dir = folder("prompt");
    write_source(&dir.join("Source.xlsx"), 10.0, 4.5);
    let mut s = dest(&dir);
    assert_eq!(run(&mut s, "data.linkStartupPrompt", json!({}))["mode"], json!("userSet"));
    assert!(s.take_ui_requests().iter().any(|q| matches!(q, UiRequest::Dialog(n, _) if n == "linkStartupPrompt")));
    run(&mut s, "file.save", json!({}));
    assert!(parts(&dir.join("Dest.xlsx"))["xl/workbook.xml"].contains("<workbookPr/>"));
    for (mode, attr) in [("never", "never"), ("always", "always"), ("Let users choose", "")] {
        run(&mut s, "data.linkStartupPrompt", json!({"mode": mode}));
        run(&mut s, "file.save", json!({}));
        let book = &parts(&dir.join("Dest.xlsx"))["xl/workbook.xml"];
        if attr.is_empty() {
            assert!(book.contains("<workbookPr/>"), "{book}");
        } else {
            assert!(book.contains(&format!("<workbookPr updateLinks=\"{attr}\"/>")), "{book}");
        }
        let mut t = Session::new();
        run(&mut t, "file.open", json!({"path": dir.join("Dest.xlsx")}));
        assert_eq!(t.doc().unwrap().wb.update_links.as_str(), if attr.is_empty() { "userSet" } else { attr });
        // "Don't display the alert and don't update": no security warning, nothing is read.
        assert_eq!(t.doc().unwrap().links.updates_disabled, attr != "never", "{mode}");
    }
    assert!(s.execute("data.linkStartupPrompt", json!({"mode": "sometimes"})).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn enable_content_updates_the_links_of_an_opened_workbook() {
    let dir = folder("enable");
    write_source(&dir.join("Source.xlsx"), 10.0, 4.5);
    let mut s = dest(&dir);
    run(&mut s, "data.refreshLinks", json!({}));
    run(&mut s, "file.save", json!({}));
    write_source(&dir.join("Source.xlsx"), 12.0, 4.5);
    let mut t = Session::new();
    run(&mut t, "file.open", json!({"path": dir.join("Dest.xlsx")}));
    assert!(t.doc().unwrap().links.updates_disabled);
    assert_eq!(value(&t, "A1"), Value::Number(10.0), "opening never reads the source");
    let r = run(&mut t, "data.enableLinkUpdates", json!({}));
    assert_eq!(r["updated"], json!(1));
    assert!(!t.doc().unwrap().links.updates_disabled);
    assert_eq!(value(&t, "A1"), Value::Number(12.0));
    // The pane checks each link's file when it opens.
    let r = run(&mut t, "data.workbookLinks", json!({}));
    assert_eq!(statuses(&r), ["ok"]);
    assert!(t.take_ui_requests().iter().any(|q| matches!(q, UiRequest::Dialog(n, _) if n == "workbookLinks")));
    // Open Source opens the linked workbook.
    let r = run(&mut t, "data.openLinkSource", json!({"link": 1}));
    assert!(r["title"].as_str().is_some_and(|t| t == "Source.xlsx"), "{r}");
    assert_eq!(t.documents().len(), 2);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn without_file_access_links_keep_their_saved_values() {
    let dir = folder("nofiles");
    write_source(&dir.join("Source.xlsx"), 10.0, 4.5);
    let mut s = dest(&dir);
    s.link_files = LinkFiles::none();
    for id in ["data.refreshLinks", "data.checkLinkStatus", "data.changeLinkSource", "data.openLinkSource", "data.enableLinkUpdates"] {
        match s.execute(id, json!({"link": 1, "path": "x.xlsx"})) {
            Err(EngineError::Disabled(_, why)) => assert_eq!(why, NO_FILES, "{id}"),
            other => panic!("{id}: {other:?}"),
        }
    }
    // Listing, Break Link and the startup prompt need no files.
    let r = run(&mut s, "data.workbookLinks", json!({}));
    assert_eq!(statuses(&r), ["unknown"]);
    assert_eq!(r["links"][0]["status"], json!(serde_json::to_value(LinkStatus::Unknown).unwrap()));
    run(&mut s, "data.linkStartupPrompt", json!({"mode": "never"}));
    run(&mut s, "data.breakLink", json!({}));
    assert!(s.doc().unwrap().wb.external_books.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn link_commands_survive_hostile_params() {
    let dir = folder("fuzz");
    write_source(&dir.join("Source.xlsx"), 10.0, 4.5);
    let mut s = dest(&dir);
    let ids = [
        "data.workbookLinks",
        "data.editLinks",
        "data.checkLinkStatus",
        "data.refreshLinks",
        "data.changeLinkSource",
        "data.openLinkSource",
        "data.breakLink",
        "data.linkStartupPrompt",
        "data.enableLinkUpdates",
    ];
    let params = [
        json!({}),
        json!([1, 2]),
        json!(null),
        json!({"link": 0}),
        json!({"link": -1}),
        json!({"link": 99}),
        json!({"link": 1.5}),
        json!({"link": u64::MAX}),
        json!({"link": "[9]"}),
        json!({"link": ""}),
        json!({"link": {"a": 1}}),
        json!({"link": [1]}),
        json!({"link": 1, "path": ""}),
        json!({"link": 1, "path": "\u{0}"}),
        json!({"link": 1, "path": dir.join("Dest.xlsx")}),
        json!({"link": 1, "path": "C:\\no\\such\\folder\\x.xlsx"}),
        json!({"mode": 5}),
        json!({"mode": ""}),
        json!({"check": "yes"}),
    ];
    for id in ids {
        for p in &params {
            let r = s.execute(id, p.clone());
            assert!(!matches!(r, Err(EngineError::Internal(..))), "{id} {p}: {r:?}");
            // Opening a source opens a second document; keep working on the destination.
            s.set_active(0);
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
