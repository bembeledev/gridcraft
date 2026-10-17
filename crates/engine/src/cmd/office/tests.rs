use serde_json::{Value as Json, json};

use crate::Session;

/// Office add-ins are process-wide: these tests take turns.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// A minimal add-in manifest (written for these tests from the public schema) with `permissions`.
fn add_addin(guid: &str, permissions: &str) -> String {
    let xml = format!(
        r#"<OfficeApp xmlns="http://schemas.microsoft.com/office/appforoffice/1.1" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="TaskPaneApp">
  <Id>{guid}</Id><Version>1.0</Version><ProviderName>Tests</ProviderName><DefaultLocale>en-US</DefaultLocale>
  <DisplayName DefaultValue="Sync Test"/><Hosts><Host Name="Workbook"/></Hosts>
  <DefaultSettings><SourceLocation DefaultValue="https://tests.example/pane.html"/></DefaultSettings>
  <Permissions>{permissions}</Permissions></OfficeApp>"#
    );
    let m = gridcraft_plugin::office::parse_manifest(&xml, "gridcraft").unwrap();
    let id = m.id.clone();
    gridcraft_plugin::install(m, std::env::temp_dir()).unwrap();
    id
}

fn session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s
}

fn sync(s: &mut Session, addin: &str, actions: Json) -> Json {
    s.execute("office.sync", json!({"addin": addin, "actions": actions})).unwrap()
}

fn range(sheet: &str, addr: &str) -> Json {
    json!([{"p": "workbook"}, {"p": "worksheets"}, {"m": "getItem", "a": [sheet]}, {"m": "getRange", "a": [addr]}])
}

#[test]
fn a_sync_writes_and_reads_ranges_as_one_undo_step() {
    let _g = serial();
    let id = add_addin("aaaaaaaa-0000-4000-8000-000000000001", "ReadWriteDocument");
    let mut s = session();
    s.execute("cell.set", json!({"cell": "A1", "input": "keep"})).unwrap();
    let undo = s.active().unwrap().undo.len();
    let out = sync(
        &mut s,
        &id,
        json!([
            {"op": "set", "path": range("Sheet1", "B2:C3"), "prop": "values", "value": [[1, 2], [3, "=B2+C2"]]},
            {"op": "set", "path": [{"p": "workbook"}, {"p": "worksheets"}, {"m": "getItem", "a": ["sheet1"]}, {"m": "getRange", "a": ["B2:C3"]}, {"p": "format"}, {"p": "font"}], "prop": "bold", "value": true},
            {"op": "set", "path": range("Sheet1", "D1"), "prop": "values", "value": "filled"},
            {"op": "set", "path": [{"p": "workbook"}, {"p": "worksheets"}, {"m": "getItem", "a": ["Sheet1"]}, {"m": "getRange", "a": ["D1"]}, {"p": "format"}, {"p": "fill"}], "prop": "color", "value": "#FFFF00"},
            {"op": "set", "path": range("Sheet1", "B2"), "prop": "numberFormat", "value": "0.00"},
            {"op": "load", "path": range("Sheet1", "B2:C3"), "props": ["values", "formulas", "text", "address", "rowCount"]},
            {"op": "load", "path": [{"p": "workbook"}, {"p": "worksheets"}, {"m": "getItem", "a": ["Sheet1"]}, {"m": "getRange", "a": ["B2"]}, {"p": "format"}, {"p": "font"}], "props": ["bold", "name"]},
            {"op": "load", "path": [{"p": "workbook"}, {"p": "worksheets"}], "props": ["items"]},
            {"op": "load", "path": [{"p": "workbook"}, {"p": "worksheets"}, {"m": "getItem", "a": ["Sheet1"]}, {"m": "getRange", "a": ["D1"]}, {"p": "format"}, {"p": "fill"}], "props": ["color"]},
        ]),
    );
    let r = &out["results"];
    assert_eq!(r[5]["values"], json!([[1.0, 2.0], [3.0, 3.0]]), "{out}");
    assert_eq!(r[5]["formulas"][1][1], "=B2+C2");
    assert_eq!(r[5]["text"][0][0], "1.00");
    assert_eq!(r[5]["address"], "Sheet1!B2:C3");
    assert_eq!(r[5]["rowCount"], 2);
    assert_eq!(r[6]["bold"], true);
    assert_eq!(r[7]["items"][0]["name"], "Sheet1");
    assert_eq!(r[8]["color"], "#FFFF00", "a fill colour reads back as #RRGGBB");
    assert_eq!(s.active().unwrap().undo.len(), undo + 1, "the whole sync is one undo step");
    s.execute("edit.undo", json!({})).unwrap();
    let back = sync(&mut s, &id, json!([{"op": "load", "path": range("Sheet1", "A1:B2"), "props": ["values"]}]));
    assert_eq!(back["results"][0]["values"], json!([["keep", null], [null, null]]));
    assert!(gridcraft_plugin::unload(&id));
}

#[test]
fn a_failing_action_changes_nothing_and_names_itself() {
    let _g = serial();
    let id = add_addin("aaaaaaaa-0000-4000-8000-000000000002", "ReadWriteDocument");
    let mut s = session();
    let out = sync(
        &mut s,
        &id,
        json!([
            {"op": "set", "path": range("Sheet1", "A1"), "prop": "values", "value": [[5]]},
            {"op": "set", "path": range("Nowhere", "A1"), "prop": "values", "value": [[6]]},
        ]),
    );
    assert_eq!(out["error"]["code"], "ItemNotFound");
    assert_eq!(out["error"]["action"], 1);
    let a1 = sync(&mut s, &id, json!([{"op": "load", "path": range("Sheet1", "A1"), "props": ["values"]}]));
    assert_eq!(a1["results"][0]["values"], json!([[null]]), "the first action was taken back");
    // Sizes must match, as in Excel; a single value fills.
    let wrong = sync(&mut s, &id, json!([{"op": "set", "path": range("Sheet1", "A1:B2"), "prop": "values", "value": [[1]]}]));
    assert_eq!(wrong["error"]["code"], "InvalidArgument");
    let unknown = sync(&mut s, &id, json!([{"op": "call", "path": range("Sheet1", "A1"), "method": "explode"}]));
    assert_eq!(unknown["error"]["code"], "NotImplemented");
    assert!(gridcraft_plugin::unload(&id));
}

#[test]
fn permissions_bound_what_an_add_in_does() {
    let _g = serial();
    let reader = add_addin("aaaaaaaa-0000-4000-8000-000000000003", "ReadDocument");
    let restricted = add_addin("aaaaaaaa-0000-4000-8000-000000000004", "Restricted");
    let mut s = session();
    let w = sync(&mut s, &reader, json!([{"op": "set", "path": range("Sheet1", "A1"), "prop": "values", "value": 1}]));
    assert_eq!(w["error"]["code"], "AccessDenied");
    assert!(sync(&mut s, &reader, json!([{"op": "load", "path": range("Sheet1", "A1"), "props": ["values"]}]))["results"].is_array());
    let r = sync(&mut s, &restricted, json!([{"op": "load", "path": range("Sheet1", "A1"), "props": ["values"]}]));
    assert_eq!(r["error"]["code"], "AccessDenied");
    // Settings are always the add-in's own.
    let saved = sync(
        &mut s,
        &restricted,
        json!([{"op": "call", "path": [{"p": "settings"}], "method": "save", "args": [{"theme": "dark"}]}, {"op": "load", "path": [{"p": "settings"}]}]),
    );
    assert_eq!(saved["results"][1], json!({"theme": "dark"}));
    let other = sync(&mut s, &reader, json!([{"op": "load", "path": [{"p": "settings"}]}]));
    assert_eq!(other["results"][0], json!({}), "one add-in doesn't see another's settings");
    assert!(gridcraft_plugin::unload(&reader) && gridcraft_plugin::unload(&restricted));
}

#[test]
fn the_selection_sheets_and_common_api() {
    let _g = serial();
    let id = add_addin("aaaaaaaa-0000-4000-8000-000000000005", "ReadWriteDocument");
    let mut s = session();
    s.execute("range.setValues", json!({"range": "A1", "values": [["a", 1], ["b", 2]]})).unwrap();
    s.execute("selection.set", json!({"range": "A1:B2"})).unwrap();
    let out = sync(
        &mut s,
        &id,
        json!([
            {"op": "load", "path": [{"p": "workbook"}, {"m": "getSelectedRange"}], "props": ["address"]},
            {"op": "load", "path": [{"p": "workbook"}, {"m": "getSelectedRange"}, {"m": "getCell", "a": [1, 1]}], "props": ["values", "address"]},
            {"op": "call", "path": [{"p": "document"}], "method": "getSelectedData", "args": ["text"]},
            {"op": "call", "path": [{"p": "document"}], "method": "getSelectedData", "args": ["matrix"]},
            {"op": "call", "path": [{"p": "workbook"}, {"p": "worksheets"}], "method": "add", "args": ["Prices"]},
            {"op": "set", "path": range("Prices", "A1"), "prop": "values", "value": "here"},
            {"op": "call", "path": [{"p": "workbook"}, {"p": "worksheets"}, {"m": "getItem", "a": ["Prices"]}], "method": "activate"},
            {"op": "load", "path": [{"p": "workbook"}, {"p": "worksheets"}, {"m": "getActiveWorksheet"}], "props": ["name", "position"]},
        ]),
    );
    let r = &out["results"];
    assert_eq!(r[0]["address"], "Sheet1!A1:B2", "{out}");
    assert_eq!(r[1]["address"], "Sheet1!B2");
    assert_eq!(r[1]["values"], json!([[2.0]]));
    assert_eq!(r[2], "a\t1\nb\t2");
    assert_eq!(r[3], json!([["a", 1.0], ["b", 2.0]]));
    assert_eq!(r[4]["name"], "Prices");
    assert_eq!(r[7], json!({"name": "Prices", "position": 1}));
    assert!(gridcraft_plugin::unload(&id));
}

#[test]
fn office_commands_open_their_pane() {
    let _g = serial();
    let id = add_addin("aaaaaaaa-0000-4000-8000-000000000006", "ReadDocument");
    let mut s = session();
    let out = s.execute("plugins.run", json!({"plugin": id, "command": "taskpane"})).unwrap();
    assert_eq!(out["office"]["kind"], "showTaskpane");
    assert!(
        s.take_ui_requests().iter().any(
            |q| matches!(q, crate::UiRequest::OfficePane { url, title, .. } if url == "https://tests.example/pane.html" && title == "Sync Test")
        )
    );
    assert!(s.execute("office.sync", json!({"addin": "nope", "actions": []})).is_err());
    assert!(gridcraft_plugin::unload(&id));
}

#[test]
fn colours_may_be_names_and_read_back_as_hex() {
    let _g = serial();
    let id = add_addin("aaaaaaaa-0000-4000-8000-000000000007", "ReadWriteDocument");
    let mut s = session();
    let fill = |a: &str| json!([{"p": "workbook"}, {"p": "worksheets"}, {"m": "getItem", "a": ["Sheet1"]}, {"m": "getRange", "a": [a]}, {"p": "format"}, {"p": "fill"}]);
    let font = |a: &str| json!([{"p": "workbook"}, {"p": "worksheets"}, {"m": "getItem", "a": ["Sheet1"]}, {"m": "getRange", "a": [a]}, {"p": "format"}, {"p": "font"}]);
    let out = sync(
        &mut s,
        &id,
        json!([
            {"op": "set", "path": fill("A1"), "prop": "color", "value": "yellow"},
            {"op": "set", "path": font("A1"), "prop": "color", "value": "LightBlue"},
            {"op": "set", "path": fill("B1"), "prop": "color", "value": "#0f8"},
            {"op": "set", "path": font("B1"), "prop": "color", "value": "#a52a2a"},
            {"op": "load", "path": fill("A1"), "props": ["color"]},
            {"op": "load", "path": font("A1"), "props": ["color"]},
            {"op": "load", "path": fill("B1"), "props": ["color"]},
            {"op": "load", "path": font("B1"), "props": ["color"]},
        ]),
    );
    let r = &out["results"];
    assert_eq!(r[4]["color"], "#FFFF00", "{out}");
    assert_eq!(r[5]["color"], "#ADD8E6");
    assert_eq!(r[6]["color"], "#00FF88");
    assert_eq!(r[7]["color"], "#A52A2A");
    // Not a colour: the whole batch fails with InvalidArgument and names the action.
    let out = sync(
        &mut s,
        &id,
        json!([{"op": "set", "path": range("Sheet1", "C1"), "prop": "values", "value": 1}, {"op": "set", "path": fill("C1"), "prop": "color", "value": "bluish"}]),
    );
    assert_eq!(out["error"]["code"], "InvalidArgument", "{out}");
    assert_eq!(out["error"]["action"], 1);
    assert_eq!(s.execute("sheet.read", json!({"range": "C1"})).unwrap()["values"], json!([[null]]));
    assert!(gridcraft_plugin::unload(&id));
}

#[test]
fn worksheets_add_without_a_name_is_the_next_sheet_at_the_end() {
    let _g = serial();
    let id = add_addin("aaaaaaaa-0000-4000-8000-000000000008", "ReadWriteDocument");
    let mut s = session();
    let sheets = json!([{"p": "workbook"}, {"p": "worksheets"}]);
    let added = json!([{"p": "workbook"}, {"p": "worksheets"}, {"m": "getAdded", "a": ["t1"]}]);
    let out = sync(
        &mut s,
        &id,
        json!([
            {"op": "call", "path": sheets, "method": "add", "args": [null, {"token": "t1"}]},
            {"op": "set", "path": [{"p": "workbook"}, {"p": "worksheets"}, {"m": "getAdded", "a": ["t1"]}, {"m": "getRange", "a": ["A1"]}], "prop": "values", "value": "new"},
            {"op": "load", "path": added, "props": ["name", "position"]},
            {"op": "load", "path": [{"p": "workbook"}, {"p": "worksheets"}, {"m": "getActiveWorksheet"}], "props": ["name"]},
            {"op": "call", "path": sheets, "method": "add", "args": ["Named"]},
            {"op": "set", "path": added, "prop": "name", "value": "Renamed"},
            {"op": "load", "path": added, "props": ["name"]},
            {"op": "load", "path": sheets, "props": ["items"]},
        ]),
    );
    let r = &out["results"];
    assert_eq!(r[0]["name"], "Sheet2", "{out}");
    assert_eq!(r[2], json!({"name": "Sheet2", "position": 1}));
    assert_eq!(r[3]["name"], "Sheet1", "adding leaves the active worksheet as it was");
    assert_eq!(r[6]["name"], "Renamed", "the proxy follows a rename");
    let names: Vec<&str> = r[7]["items"].as_array().unwrap().iter().filter_map(|i| i["name"].as_str()).collect();
    assert_eq!(names, ["Sheet1", "Renamed", "Named"], "added at the end");
    assert_eq!(s.execute("sheet.read", json!({"range": "Renamed!A1"})).unwrap()["values"], json!([["new"]]));
    let gone = sync(
        &mut s,
        &id,
        json!([{"op": "load", "path": [{"p": "workbook"}, {"p": "worksheets"}, {"m": "getAdded", "a": ["nope"]}], "props": ["name"]}]),
    );
    assert_eq!(gone["error"]["code"], "ItemNotFound");
    assert!(gridcraft_plugin::unload(&id));
}
