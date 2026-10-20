use gridcraft_engine::{
    Session,
    core::{CellRef, Value},
    patch_data::{
        self as data,
        schema::{self, Type},
        validate::{self, Catalog},
    },
};
use serde_json::json;
fn sheet(name: &str, rows: serde_json::Value) -> data::DataSheet {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("sheet.rename", json!({"name": name})).unwrap();
    s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
    data::extract("TestData.xlsx", &s.active().unwrap().wb).unwrap().remove(0)
}
#[test]
fn ck_headers_and_design_exceptions() {
    let sh = sheet(
        "Item",
        json!([
            ["All", "All", "Design", "Version;Key"],
            ["TString<Item.[Id]>", "NullableTString<Item.[Id]>", "designer notes", "String"],
            ["Id", "Label", "Note", "Version"]
        ]),
    );
    let (report, _) = validate::analyze(&[&sh], None, true);
    assert_eq!(report.issues.iter().filter(|i| i.code == "ck.tstring").count(), 1);
    assert!(!report.issues.iter().any(|i| i.code == "schema.type"));
    assert!(report.issues.iter().any(|i| i.code == "schema.flags" && i.cell == "D1"));
    assert!(Type::parse("List<List<Int>>").is_err());
    assert!(Type::parse("IntList").is_err());
    assert!(Type::parse("List<IntList>").unwrap().matrix);
    assert!(schema::flag_errors("All;StringIdDefinition;Raw").len() == 1);
    assert!(schema::flag_errors("All;StringIdDefinition;FKey.Item").len() == 1);
}
#[test]
fn list_grammar_quotes_empty_fields_and_records() {
    assert_eq!(schema::split_values("1, \n2,\"a,b\"", false), ["1", "2", "\"a,b\""]);
    assert_eq!(schema::split_values(" , ", false), ["", ""]);
    assert!(schema::split_values("  ", false).is_empty());
    assert_eq!(schema::split_values("1,2\n3,4", true), ["1,2", "3,4"]);
    assert_eq!(schema::split_values("x,\"odd,y", false), ["x,\"odd", "y"]);
    assert!(validate::value_error(&Type::parse("List<IntList>").unwrap(), &Value::from("1,2\n\n3,4"), &Catalog::default()).is_ok());
}
#[test]
fn version_filters_references_after_merging_and_physical_columns_are_kept() {
    let version =
        sheet("ResourceVersion", json!([["All", "All"], ["String", "Int"], ["ResourceVersionName", "ResourceVersionNumber"], ["v1", 1], ["v2", 2]]));
    let target = sheet("Item_A", json!([["All;Key", "Version"], ["Int", "String"], ["Id", "Version"], [1, "v1"], [2, "v2"]]));
    let from =
        sheet("Shop", json!([["", "All;FKey.Item", "All;FKey.Item"], ["", "Int", "List<Int>"], ["", "Item", "Items"], ["unused", 2, "1_0, 2"]]));
    let (report, _) = validate::analyze(&[&version, &target, &from], Some("v1"), true);
    assert_eq!(report.excluded_rows, 1);
    assert!(report.issues.iter().any(|i| i.cell == "B4" && i.code == "fkey.missing"));
    let (report, _) = validate::analyze(&[&version, &target, &from], Some("v2"), true);
    assert!(!report.issues.iter().any(|i| i.cell == "B4" && i.code == "fkey.missing"));
    assert_eq!(report.issues.iter().filter(|i| i.cell == "C4" && i.code == "fkey.missing").count(), 1);
}
#[test]
fn transposed_headers_and_diagnostics_have_physical_addresses() {
    let sh = sheet("Item_Transpose", json!([["All;Key", "Int", "Id", 1, 1], ["All", "String", "Name", "A", "B"]]));
    assert_eq!(sh.columns[0].name, "Id");
    assert_eq!(sh.cell(4, 0), CellRef::new(0, 4));
    let (report, _) = validate::analyze(&[&sh], None, true);
    assert!(report.issues.iter().any(|i| i.cell == "E1" && i.code == "key.duplicate"));
}
#[test]
fn aliases_are_single_pass_and_raw_ids_are_not_definitions() {
    let mut c = Catalog::default();
    c.aliases.insert("##A##".into(), "##B##".into());
    c.aliases.insert("##B##".into(), "2".into());
    assert_eq!(validate::resolve(&Value::from("##A## + ##B##"), &c).display(), "##B## + 2");
    let sh = sheet("Item", json!([["All;Key;Raw", "All;StringIdReference"], ["String", "String"], ["Id", "Other"], ["[ abc ]", "[abc]"]]));
    let (report, _) = validate::analyze(&[&sh], None, false);
    assert!(report.issues.iter().any(|i| i.code == "id.undefined" && i.severity == "warning"));
    let raw = sheet("RawItem", json!([["All;Raw"], ["List<Long>"], ["Ids"], ["[literal]"]]));
    let (report, _) = validate::analyze(&[&raw], None, true);
    assert!(report.issues.iter().any(|i| i.code == "value.type" && i.cell == "A4"));
}
#[test]
fn value_precision_nullable_boolean_and_scaled_numbers() {
    let c = Catalog::default();
    let check = |typ: &str, v: Value| validate::value_error(&Type::parse(typ).unwrap(), &v, &c);
    assert!(check("Long", Value::from("9223372036854775807")).is_ok());
    assert!(check("Long", Value::from("9223372036854775808")).is_err());
    assert!(check("Int", Value::Number(1.00000001)).is_ok());
    assert!(check("Int", Value::Number(1.5)).is_err());
    assert!(check("NullableLong", Value::from("null")).is_ok());
    assert!(check("Bool", Value::Number(2.0)).is_ok());
    assert!(check("Bool", Value::from("2")).is_err());
    assert!(check("Percentile", Value::from("1.234")).is_err());
    assert!(check("Json", Value::from("{\"x\":2}")).is_ok());
    assert!(check("Vector3", Value::from("1,2")).is_err());
}
#[test]
fn duplicate_compound_group_zip_and_schema_merge_are_reported() {
    let sh = sheet(
        "Item_A",
        json!([
            ["All;CompoundKey;GroupKey", "All;CompoundKey", "All;ZipWith.B", "All"],
            ["String", "String", "List<Int>", "List<String>"],
            ["Id", "Name", "A", "B"],
            ["x", "a", "1,2", "a"],
            ["y", "b", "", ""],
            ["x", "a", "", ""]
        ]),
    );
    let other = sheet("Item_B", json!([["All"], ["String"], ["Different"]]));
    let (r, _) = validate::analyze(&[&sh, &other], None, true);
    for code in ["key.compound", "key.group", "zip.length", "sheet.merge"] {
        assert!(r.issues.iter().any(|i| i.code == code), "{code}");
    }
}
#[test]
fn explicit_command_and_input_bounds() {
    let mut s = Session::new();
    s.new_workbook();
    assert!(s.execute("patch.validate", json!({})).is_err());
    assert!(s.execute("patch.validate", json!({"project": "ck"})).is_ok());
    s.execute("cell.set", json!({"cell": "XFD1048576", "input": "1"})).unwrap();
    assert!(s.execute("patch.validate", json!({"project": "ck"})).is_err());
    assert!(schema::eligible_file("ItemData.xlsx"));
    for name in ["ItemData.XLSX", "_ItemData.xlsx", "~ItemData.xlsx", "Data.xlsm"] {
        assert!(!schema::eligible_file(name));
    }
}

#[test]
fn string_keys_preserve_spaces_and_multiple_fkeys_are_checked() {
    let target = sheet("Item", json!([["All;Key"], ["String"], ["Id"], ["a"], [" a "]]));
    let from = sheet("Shop", json!([["All;FKey.Item;FKey.Other"], ["String"], ["Item"], [" a "]]));
    let (r, _) = validate::analyze(&[&target, &from], None, true);
    assert!(!r.issues.iter().any(|i| i.code == "key.duplicate" || i.code == "fkey.missing"));
    assert!(r.issues.iter().any(|i| i.code == "fkey.table" && i.message.contains("Other")));
}

#[test]
fn chunked_validation_matches_whole_tables_across_boundaries() {
    use std::sync::Arc;
    let mut source = sheet("Item", json!([["All;Key;GroupKey", "All;FKey.Other"], ["Int", "Int"], ["Id", "Other"]]));
    for row in 3..603 {
        source.rows.insert(row, vec![Value::Number((row % 299) as f64), Value::Number(7.0)]);
    }
    let other = sheet("Other", json!([["All;Key"], ["Int"], ["Id"], [7]]));
    let (expected, catalog) = validate::analyze(&[&source, &other], None, true);
    let mut streaming = validate::Validator::new(Arc::new(catalog), None, true);
    for rows in source.rows.iter().collect::<Vec<_>>().chunks(17) {
        let mut chunk = source.clone();
        chunk.rows = rows.iter().map(|(k, v)| (**k, (*v).clone())).collect();
        streaming.push(&chunk);
    }
    streaming.push(&other);
    assert_eq!(serde_json::to_value(expected).unwrap(), serde_json::to_value(streaming.report).unwrap());
}

#[test]
fn chunked_extraction_visits_full_sparse_extents_and_preserves_addresses() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("sheet.rename", json!({"name":"Item"})).unwrap();
    s.execute("range.setValues", json!({"range":"C1","values":[["All;Key"],["Int"],["Id"]]})).unwrap();
    s.execute("cell.set", json!({"cell":"C3004","input":"1"})).unwrap();
    s.execute("cell.set", json!({"cell":"XFD3004","input":"unused"})).unwrap();
    let mut rows = 0;
    let mut last = 0;
    data::visit_chunks("LargeData.xlsx", &s.active().unwrap().wb, |chunk| {
        assert!(chunk.rows.len() <= 256);
        assert_eq!(chunk.columns.len(), 1);
        assert_eq!(chunk.columns[0].index, 2);
        rows += chunk.rows.len();
        last = chunk.rows.keys().last().copied().unwrap_or(last);
        Ok(())
    })
    .unwrap();
    assert_eq!((rows, last), (3001, 3003));
    let mut visited = 0;
    assert!(
        data::visit_chunks("LargeData.xlsx", &s.active().unwrap().wb, |_| {
            visited += 1;
            Err("cancel".into())
        })
        .is_err()
    );
    assert_eq!(visited, 1);
}
