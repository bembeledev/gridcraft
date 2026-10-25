//! PivotTables: write → read round trips, package structure, hostile pivot parts.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::Arc;

use gridcraft_core::date::serial_from_ymd;
use gridcraft_core::{CellRef, RangeRef, Value};
use gridcraft_model::*;

use super::minimal;
use crate::xml;
use crate::{read_xlsx, write_xlsx};

fn at(a: &str) -> CellRef {
    CellRef::parse(a).unwrap()
}
fn rr(a: &str) -> RangeRef {
    RangeRef::parse(a).unwrap()
}

fn date(y: i64, m: i64, d: i64) -> f64 {
    serial_from_ymd(gridcraft_core::DateSystem::D1900, y, m, d).unwrap()
}

fn field(name: &str) -> PivotField {
    PivotField { source_col: name.into(), ..PivotField::default() }
}

/// Source data on "Data" (A1:F9, also table "SalesTbl" and defined name "SalesData"), three
/// PivotTables on "Report" with a few hand-written report cells.
fn workbook() -> Workbook {
    let mut wb = Workbook::new();
    let date_style = wb.styles.derive(StyleId::DEFAULT, |s| s.num_fmt = NumFmt::new("m/d/yyyy"));
    let data = wb.sheet_mut(0).unwrap();
    data.name = "Data".into();
    let headers = ["Region", "Product", "Date", "Units", "Sales", "Channel"];
    for (c, h) in headers.iter().enumerate() {
        data.cells.set(CellRef::new(0, c as u32), Cell::value(Value::text(*h)));
    }
    let rows: [(&str, &str, f64, f64, f64, &str); 8] = [
        ("East", "A", date(2024, 1, 5), 3.0, 30.5, "Online"),
        ("West", "B", date(2024, 2, 9), 5.0, 50.0, "Store"),
        ("East", "B", date(2024, 3, 1), 2.0, 20.0, "Online"),
        ("North", "C", date(2025, 4, 12), 7.0, 70.25, "Store"),
        ("west", "A", date(2025, 5, 30), 1.0, 10.0, "Online"),
        ("East", "C", date(2025, 6, 2), 4.0, 40.0, "Phone"),
        ("North", "A", date(2024, 7, 7), 6.0, 60.0, "Online"),
        ("West", "C", date(2025, 12, 31), 8.0, 80.0, "Store"),
    ];
    for (r, (a, b, d, u, s, ch)) in rows.iter().enumerate() {
        let r = r as u32 + 1;
        data.cells.set(CellRef::new(r, 0), Cell::value(Value::text(*a)));
        data.cells.set(CellRef::new(r, 1), Cell::value(Value::text(*b)));
        data.cells.set(CellRef::new(r, 2), Cell { value: Value::number(*d), formula: None, style: date_style });
        data.cells.set(CellRef::new(r, 3), Cell::value(Value::number(*u)));
        data.cells.set(CellRef::new(r, 4), Cell::value(Value::number(*s)));
        data.cells.set(CellRef::new(r, 5), Cell::value(Value::text(*ch)));
    }
    data.tables.push(Table {
        id: 1,
        name: "SalesTbl".into(),
        range: rr("A1:F9"),
        header_row: true,
        totals_row: false,
        columns: headers
            .iter()
            .map(|h| TableColumn { name: (*h).into(), totals: TotalsFn::None, totals_label: None, formula: None, unique_name: None, xml: None })
            .collect(),
        style: "TableStyleMedium2".into(),
        banded_rows: true,
        banded_cols: false,
        first_col: false,
        last_col: false,
        filter_button: true,
        filter: vec![],
        table_type: None,
        connection_id: None,
    });
    wb.names.push(DefinedName { name: "SalesData".into(), scope: None, formula: "Data!$A$1:$F$9".into(), comment: String::new(), hidden: false });

    let mut report = Sheet::new("Report");
    // Pivot 1: tabular, page filter, custom and built-in value formats, collapsed item, Enable
    // show details off.
    let p1 = PivotTable {
        id: 1,
        name: "Sales by Region".into(),
        source: "Data!$A$1:$F$9".into(),
        anchor: at("A4"),
        rows: vec![PivotField { source_col: "Region".into(), sort: PivotSort::Asc, collapsed_items: vec!["East".into()], ..PivotField::default() }],
        columns: vec![PivotField { sort: PivotSort::None, ..field("Product") }],
        values: vec![
            PivotValue {
                source_col: "Sales".into(),
                func: PivotFunc::Sum,
                name: "Sum of Sales".into(),
                number_format: Some("#,##0.00".into()),
                ..PivotValue::default()
            },
            PivotValue {
                source_col: "Units".into(),
                func: PivotFunc::Count,
                name: "Count of Units".into(),
                number_format: Some("0.0\" u\"".into()),
                ..PivotValue::default()
            },
        ],
        filters: vec![PivotFilter { source_col: "Channel".into(), selected: Some(vec!["Online".into()]), value_filter: None }],
        layout: PivotLayout::Tabular,
        enable_show_details: false,
        grand_totals_rows: false,
        grand_totals_cols: true,
        subtotals: false,
        subtotals_top: false,
        style: "PivotStyleMedium9".into(),
        show_headers: false,
        last_range: Some(rr("A2:H12")),
        calculated_fields: vec![],
        calculated_items: vec![],
        repeat_item_labels: false,
        blank_lines: false,
        show_items_with_no_data: false,
        shared_cache: None,
        values_on_rows: false,
        values_position: None,
    };
    // Pivot 2: table source, date grouping on two levels, show-values-as, multi-select page
    // filter and a hidden-items filter on a column field.
    let p2 = PivotTable {
        id: 2,
        name: "Trend".into(),
        source: "SalesTbl".into(),
        anchor: at("K4"),
        rows: vec![
            PivotField {
                source_col: "Date".into(),
                date_group: PivotDateGroup::Years,
                sort: PivotSort::None,
                group_bins: None,
                grouped_items: vec![],
                sort_by: None,
                custom_order: vec![],
                collapsed_items: vec!["2024".into()],
            },
            PivotField {
                source_col: "Date".into(),
                date_group: PivotDateGroup::Months,
                sort: PivotSort::Asc,
                group_bins: None,
                grouped_items: vec![],
                sort_by: None,
                custom_order: vec![],
                collapsed_items: vec![],
            },
        ],
        columns: vec![PivotField { sort: PivotSort::Desc, ..field("Product") }],
        values: vec![
            PivotValue {
                source_col: "Sales".into(),
                func: PivotFunc::Sum,
                name: "Share".into(),
                show_as: PivotShowAs::PercentOfGrandTotal,
                base_field: None,
                base_item: None,
                error_display: None,
                empty_display: None,
                number_format: None,
            },
            PivotValue {
                source_col: "Sales".into(),
                func: PivotFunc::Max,
                name: "Rank".into(),
                show_as: PivotShowAs::Rank,
                base_field: None,
                base_item: None,
                error_display: None,
                empty_display: None,
                number_format: None,
            },
            PivotValue {
                source_col: "Sales".into(),
                func: PivotFunc::Average,
                name: "Running".into(),
                show_as: PivotShowAs::RunningTotal,
                base_field: None,
                base_item: None,
                error_display: None,
                empty_display: None,
                number_format: None,
            },
            PivotValue {
                source_col: "Units".into(),
                func: PivotFunc::StdDevP,
                name: "Spread".into(),
                show_as: PivotShowAs::PercentOfColumnTotal,
                base_field: None,
                base_item: None,
                error_display: None,
                empty_display: None,
                number_format: None,
            },
            PivotValue {
                source_col: "Units".into(),
                func: PivotFunc::VarP,
                name: "RowPct".into(),
                show_as: PivotShowAs::PercentOfRowTotal,
                base_field: None,
                base_item: None,
                error_display: None,
                empty_display: None,
                number_format: None,
            },
        ],
        filters: vec![
            PivotFilter { source_col: "Region".into(), selected: Some(vec!["East".into(), "North".into()]), value_filter: None },
            PivotFilter { source_col: "Product".into(), selected: Some(vec!["A".to_string(), "B".to_string()]), value_filter: None },
        ],
        layout: PivotLayout::Outline,
        enable_show_details: true,
        grand_totals_rows: true,
        grand_totals_cols: false,
        subtotals: true,
        subtotals_top: true,
        style: "PivotStyleDark3".into(),
        show_headers: true,
        last_range: Some(rr("K2:Z30")),
        calculated_fields: vec![],
        calculated_items: vec![],
        repeat_item_labels: false,
        blank_lines: false,
        show_items_with_no_data: false,
        shared_cache: None,
        values_on_rows: false,
        values_position: None,
    };
    // Pivot 3: defaults, defined-name source, compact.
    let p3 = PivotTable {
        id: 3,
        name: "PivotTable3".into(),
        source: "SalesData".into(),
        anchor: at("AB1"),
        rows: vec![field("Product")],
        values: vec![PivotValue { source_col: "Units".into(), func: PivotFunc::Product, name: "Product of Units".into(), ..PivotValue::default() }],
        filters: vec![PivotFilter { source_col: "Channel".into(), selected: None, value_filter: None }],
        last_range: Some(rr("AB1:AC6")),
        ..PivotTable::default()
    };
    report.pivots = vec![p1, p2, p3];
    // Hand-written report cells (the engine's output stays plain cells).
    report.cells.set(at("A2"), Cell::value(Value::text("Channel")));
    report.cells.set(at("B2"), Cell::value(Value::text("Online")));
    report.cells.set(at("A5"), Cell::value(Value::text("East")));
    report.cells.set(at("B5"), Cell::value(Value::number(50.5)));
    wb.sheets.push(Arc::new(report));
    wb
}

fn unzip(bytes: &[u8]) -> HashMap<String, Vec<u8>> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut m = HashMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let mut b = Vec::new();
        f.read_to_end(&mut b).unwrap();
        m.insert(f.name().to_string(), b);
    }
    m
}

#[test]
fn pivots_round_trip() {
    let wb = workbook();
    let bytes = write_xlsx(&wb).unwrap();
    let (back, report) = read_xlsx(&bytes).unwrap();
    assert!(report.warnings.iter().all(|w| !w.to_lowercase().contains("pivot")), "{:?}", report.warnings);
    let orig = &wb.sheet(1).unwrap().pivots;
    let read = &back.sheet(1).unwrap().pivots;
    assert_eq!(read.len(), 3);
    for (a, b) in orig.iter().zip(read.iter()) {
        assert_eq!(a, b, "pivot {} differs", a.name);
    }
    // Report cells stay as written.
    let s = back.sheet(1).unwrap();
    assert_eq!(s.value(at("B5")), Value::number(50.5));
    assert_eq!(s.value(at("A5")), Value::text("East"));
    // A second trip is stable.
    let (again, _) = read_xlsx(&write_xlsx(&back).unwrap()).unwrap();
    assert_eq!(&again.sheet(1).unwrap().pivots, read);
}

#[test]
fn pivot_package_structure() {
    let bytes = write_xlsx(&workbook()).unwrap();
    let parts = unzip(&bytes);
    let ct = xml::parse(parts.get("[Content_Types].xml").unwrap()).unwrap();
    let overrides: HashMap<String, String> = ct
        .kids("Override")
        .map(|o| (o.attr("PartName").unwrap().trim_start_matches('/').to_string(), o.attr("ContentType").unwrap().to_string()))
        .collect();
    let defaults: HashSet<String> = ct.kids("Default").map(|d| d.attr("Extension").unwrap().to_string()).collect();
    // Every override names an existing part; every part has a content type.
    for p in overrides.keys() {
        assert!(parts.contains_key(p), "override for missing part {p}");
    }
    for p in parts.keys() {
        if p == "[Content_Types].xml" {
            continue;
        }
        let ext = p.rsplit('.').next().unwrap();
        assert!(overrides.contains_key(p) || defaults.contains(ext), "no content type for {p}");
    }
    let count = |ct: &str| overrides.values().filter(|v| v.as_str() == ct).count();
    assert_eq!(count(crate::pivot::CT_PIVOT_TABLE), 3);
    assert_eq!(count(crate::pivot::CT_PIVOT_CACHE_DEF), 3);
    assert_eq!(count(crate::pivot::CT_PIVOT_CACHE_REC), 3);
    // Every relationship target exists.
    let mut kinds: Vec<String> = vec![];
    for (name, data) in &parts {
        if !name.ends_with(".rels") {
            continue;
        }
        let dir = name.trim_end_matches(".rels").rsplit_once("/_rels/").map(|(d, _)| d).unwrap_or("");
        let rels = xml::parse(data).unwrap();
        for r in rels.kids("Relationship") {
            if r.attr("TargetMode") == Some("External") {
                continue;
            }
            let target = crate::package::resolve(dir, r.attr("Target").unwrap());
            assert!(parts.contains_key(&target), "{name} points at missing {target}");
            kinds.push(r.attr("Type").unwrap().rsplit('/').next().unwrap().to_string());
        }
    }
    for k in ["pivotTable", "pivotCacheDefinition", "pivotCacheRecords"] {
        assert!(kinds.iter().any(|x| x == k), "no {k} relationship");
    }
    // The workbook lists the caches by id with relationships of the right type.
    let wbx = xml::parse(parts.get("xl/workbook.xml").unwrap()).unwrap();
    let wb_rels = xml::parse(parts.get("xl/_rels/workbook.xml.rels").unwrap()).unwrap();
    let caches: Vec<&xml::El> = wbx.child("pivotCaches").unwrap().kids("pivotCache").collect();
    assert_eq!(caches.len(), 3);
    let mut ids = HashSet::new();
    for c in &caches {
        assert!(ids.insert(c.attr("cacheId").unwrap().to_string()));
        let rid = c.attr("id").unwrap();
        let rel = wb_rels.kids("Relationship").find(|r| r.attr("Id") == Some(rid)).unwrap();
        assert!(rel.attr("Type").unwrap().ends_with("/pivotCacheDefinition"));
    }
    // Each table's cacheId is one of the workbook's caches and its field counts agree.
    for t in 1..=3 {
        let pt = xml::parse(parts.get(&format!("xl/pivotTables/pivotTable{t}.xml")).unwrap()).unwrap();
        assert!(ids.contains(pt.attr("cacheId").unwrap()));
        let trels = xml::parse(parts.get(&format!("xl/pivotTables/_rels/pivotTable{t}.xml.rels")).unwrap()).unwrap();
        let target = crate::package::resolve("xl/pivotTables", trels.kids("Relationship").next().unwrap().attr("Target").unwrap());
        let cache = xml::parse(parts.get(&target).unwrap()).unwrap();
        let nf = cache.child("cacheFields").unwrap().kids("cacheField").count();
        assert_eq!(pt.child("pivotFields").unwrap().kids("pivotField").count(), nf);
        assert_eq!(cache.child("cacheFields").unwrap().attr_u32("count"), Some(nf as u32));
        // Item indices stay inside the shared/group items.
        for (i, pf) in pt.child("pivotFields").unwrap().kids("pivotField").enumerate() {
            let cf = cache.child("cacheFields").unwrap().kids("cacheField").nth(i).unwrap();
            let n = cf
                .path(&["fieldGroup", "groupItems"])
                .map(|g| g.children.len())
                .unwrap_or_else(|| cf.child("sharedItems").map(|s| s.children.len()).unwrap_or(0));
            if let Some(items) = pf.child("items") {
                assert_eq!(items.attr_u32("count"), Some(items.children.len() as u32));
                for it in items.kids("item") {
                    if let Some(x) = it.attr_u32("x") {
                        assert!((x as usize) < n, "item x={x} out of {n} in table {t} field {i}");
                    }
                }
            }
        }
        // Records match the database fields.
        if let Some(rid) = cache.attr("id") {
            let crels_name = target.replace("pivotCache/", "pivotCache/_rels/") + ".rels";
            let crels = xml::parse(parts.get(&crels_name).unwrap()).unwrap();
            let rel = crels.kids("Relationship").find(|r| r.attr("Id") == Some(rid)).unwrap();
            let rec = xml::parse(parts.get(&crate::package::resolve("xl/pivotCache", rel.attr("Target").unwrap())).unwrap()).unwrap();
            let db = cache.child("cacheFields").unwrap().kids("cacheField").filter(|f| f.attr("databaseField") != Some("0")).count();
            assert_eq!(rec.attr_u32("count"), cache.attr_u32("recordCount"));
            assert_eq!(rec.kids("r").count(), 8);
            for r in rec.kids("r") {
                assert_eq!(r.children.len(), db);
            }
        }
    }
    // Sheet 2 (Report) relates to all three tables.
    let srels = xml::parse(parts.get("xl/worksheets/_rels/sheet2.xml.rels").unwrap()).unwrap();
    assert_eq!(srels.kids("Relationship").filter(|r| r.attr("Type").unwrap().ends_with("/pivotTable")).count(), 3);
}

#[test]
fn pivot_xml_details() {
    let parts = unzip(&write_xlsx(&workbook()).unwrap());
    let t1 = String::from_utf8(parts.get("xl/pivotTables/pivotTable1.xml").unwrap().clone()).unwrap();
    assert!(
        t1.contains("<location ref=\"A4:H12\" firstHeaderRow=\"1\" firstDataRow=\"3\" firstDataCol=\"1\" rowPageCount=\"1\" colPageCount=\"1\"/>"),
        "{t1}"
    );
    assert!(t1.contains("<field x=\"-2\"/>"));
    assert!(t1.contains("subtotal=\"count\""));
    assert!(t1.contains("numFmtId=\"4\""));
    assert!(t1.contains("axis=\"axisPage\""));
    assert!(t1.contains("<pivotTableStyleInfo name=\"PivotStyleMedium9\""));
    let c1 = String::from_utf8(parts.get("xl/pivotCache/pivotCacheDefinition1.xml").unwrap().clone()).unwrap();
    assert!(c1.contains("<worksheetSource ref=\"A1:F9\" sheet=\"Data\"/>"), "{c1}");
    assert!(c1.contains("refreshOnLoad=\"1\""));
    let c2 = String::from_utf8(parts.get("xl/pivotCache/pivotCacheDefinition2.xml").unwrap().clone()).unwrap();
    assert!(c2.contains("<worksheetSource name=\"SalesTbl\"/>"));
    assert!(c2.contains("groupBy=\"years\"") && c2.contains("groupBy=\"months\""));
    let t2 = String::from_utf8(parts.get("xl/pivotTables/pivotTable2.xml").unwrap().clone()).unwrap();
    assert!(t2.contains("pivotShowAs=\"rankDescending\""));
    assert!(t2.contains("showDataAs=\"runTotal\""));
    // Pivot 1 has no Grand Total column (`colGrandTotals`), pivot 2 no Grand Total row
    // (`rowGrandTotals`).
    assert!(t1.contains(" colGrandTotals=\"0\"") && !t1.contains("rowGrandTotals"), "{t1}");
    assert!(t2.contains(" rowGrandTotals=\"0\"") && !t2.contains("colGrandTotals"), "{t2}");
    // The custom value format is registered in the styles part.
    let styles = String::from_utf8(parts.get("xl/styles.xml").unwrap().clone()).unwrap();
    assert!(styles.contains("formatCode=\"0.0&quot; u&quot;\""));
}

#[test]
fn unresolvable_pivot_source_is_skipped() {
    let mut wb = workbook();
    let s = wb.sheet_mut(1).unwrap();
    s.pivots[0].source = "Nowhere!A1:B2".into();
    s.pivots.truncate(1);
    let (back, _) = read_xlsx(&write_xlsx(&wb).unwrap()).unwrap();
    assert!(back.sheet(1).unwrap().pivots.is_empty());
}

#[test]
fn large_cache_is_written_without_records() {
    let mut wb = Workbook::new();
    let s = wb.sheet_mut(0).unwrap();
    s.cells.set(at("A1"), Cell::value(Value::text("K")));
    s.cells.set(at("B1"), Cell::value(Value::text("V")));
    s.cells.set(at("C1"), Cell::value(Value::text("W")));
    // 700,000 records x 3 fields is over the records limit.
    for r in 1..=700_000u32 {
        s.cells.set(CellRef::new(r, 1), Cell::value(Value::number(r as f64)));
        s.cells.set(CellRef::new(r, 2), Cell::value(Value::number(0.5)));
        s.cells.set(CellRef::new(r, 0), Cell::value(Value::number((r % 3) as f64)));
    }
    s.pivots.push(PivotTable {
        id: 1,
        source: "Sheet1!$A$1:$C$700001".into(),
        anchor: at("E1"),
        rows: vec![field("K")],
        values: vec![PivotValue { source_col: "V".into(), name: "Sum of V".into(), ..PivotValue::default() }],
        last_range: Some(rr("E1:F5")),
        ..PivotTable::default()
    });
    let bytes = write_xlsx(&wb).unwrap();
    let parts = unzip(&bytes);
    assert!(!parts.keys().any(|k| k.contains("pivotCacheRecords")));
    let c = String::from_utf8(parts.get("xl/pivotCache/pivotCacheDefinition1.xml").unwrap().clone()).unwrap();
    assert!(c.contains("saveData=\"0\""));
    assert!(c.contains("<sharedItems containsSemiMixedTypes=\"0\" containsString=\"0\" containsNumber=\"1\" containsInteger=\"1\" minValue=\"0\" maxValue=\"2\" count=\"3\">"), "{}", &c[..c.len().min(2000)]);
    let (back, _) = read_xlsx(&bytes).unwrap();
    assert_eq!(back.sheet(0).unwrap().pivots.len(), 1);
}

/// #1447 item 1: Excel counts booleans and errors as strings in `<sharedItems>`; writing
/// `containsString="0"` for them makes a file Excel can't open.
#[test]
fn boolean_and_error_cache_flags_match_excel() {
    let mut wb = Workbook::new();
    let s = wb.sheet_mut(0).unwrap();
    let na = || Value::Error(gridcraft_core::CellError::NA);
    let cols: [(&str, [Value; 3]); 7] = [
        ("K", [Value::text("a"), Value::text("b"), Value::text("a")]),
        ("BoolOnly", [Value::Bool(true), Value::Bool(false), Value::Bool(true)]),
        ("NumBool", [Value::number(1.0), Value::Bool(true), Value::number(2.5)]),
        ("ErrOnly", [na(), na(), na()]),
        ("NumErr", [Value::number(1.0), na(), Value::number(3.0)]),
        ("TextBool", [Value::text("x"), Value::Bool(true), Value::text("y")]),
        ("BoolBlank", [Value::Bool(true), Value::Empty, Value::Bool(false)]),
    ];
    for (c, (h, vals)) in cols.iter().enumerate() {
        s.cells.set(CellRef::new(0, c as u32), Cell::value(Value::text(*h)));
        for (r, v) in vals.iter().enumerate() {
            if !matches!(v, Value::Empty) {
                s.cells.set(CellRef::new(r as u32 + 1, c as u32), Cell::value(v.clone()));
            }
        }
    }
    s.pivots.push(PivotTable {
        id: 1,
        source: "Sheet1!$A$1:$G$4".into(),
        anchor: at("J1"),
        rows: vec![field("K")],
        values: vec![PivotValue { source_col: "NumBool".into(), name: "Sum of NumBool".into(), ..PivotValue::default() }],
        last_range: Some(rr("J1:K4")),
        ..PivotTable::default()
    });
    let bytes = write_xlsx(&wb).unwrap();
    let parts = unzip(&bytes);
    let c = String::from_utf8(parts.get("xl/pivotCache/pivotCacheDefinition1.xml").unwrap().clone()).unwrap();
    let start = |name: &str| -> String {
        let i = c.find(&format!("<cacheField name=\"{name}\"")).unwrap();
        let j = i + c[i..].find("<sharedItems").unwrap();
        let k = j + c[j..].find('>').unwrap();
        c[j..=k].trim_end_matches('>').trim_end_matches('/').to_string()
    };
    let flags = |name: &str| -> String {
        let t = start(name);
        // Drop the trailing count attribute, which depends on itemization.
        match t.find(" count=") {
            Some(i) => t[..i].to_string(),
            None => t,
        }
    };
    assert_eq!(flags("BoolOnly"), "<sharedItems");
    assert_eq!(flags("NumBool"), "<sharedItems containsMixedTypes=\"1\" containsNumber=\"1\" minValue=\"1\" maxValue=\"2.5\"");
    assert_eq!(flags("ErrOnly"), "<sharedItems");
    assert_eq!(flags("NumErr"), "<sharedItems containsMixedTypes=\"1\" containsNumber=\"1\" containsInteger=\"1\" minValue=\"1\" maxValue=\"3\"");
    assert_eq!(flags("TextBool"), "<sharedItems");
    assert_eq!(flags("BoolBlank"), "<sharedItems containsBlank=\"1\"");
    let (back, report) = read_xlsx(&bytes).unwrap();
    assert_eq!(back.sheet(0).unwrap().pivots.len(), 1);
    assert!(!report.warnings.iter().any(|w| w.to_lowercase().contains("pivot")), "{:?}", report.warnings);
}

const PT_RELS: &str = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotTable" Target="../pivotTables/pivotTable1.xml"/></Relationships>"#;
const PT_CACHE_RELS: &str = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotCacheDefinition" Target="../pivotCache/pivotCacheDefinition1.xml"/></Relationships>"#;

fn with_pivot(table: &str, cache: Option<&str>) -> Vec<u8> {
    let mut extra: Vec<(&str, &str)> = vec![
        ("xl/worksheets/_rels/sheet1.xml.rels", PT_RELS),
        ("xl/pivotTables/pivotTable1.xml", table),
        ("xl/pivotTables/_rels/pivotTable1.xml.rels", PT_CACHE_RELS),
    ];
    if let Some(c) = cache {
        extra.push(("xl/pivotCache/pivotCacheDefinition1.xml", c));
    }
    minimal("<sheetData/>", &extra, "", "")
}

const GOOD_CACHE: &str = r#"<pivotCacheDefinition xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><cacheSource type="worksheet"><worksheetSource ref="A1:B3"/></cacheSource><cacheFields count="2"><cacheField name="K"><sharedItems><s v="x"/><n v="2"/><m/><b v="1"/><d v="2024-01-01T00:00:00"/></sharedItems></cacheField><cacheField name="V"><sharedItems/><fieldGroup base="0"><rangePr startNum="0" endNum="10" groupInterval="5"/><groupItems><s v="0-4"/></groupItems></fieldGroup></cacheField></cacheFields></pivotCacheDefinition>"#;

#[test]
fn hostile_pivot_parts_warn_not_panic() {
    let tables = [
        "garbage <<<",
        r#"<notAPivot/>"#,
        r#"<pivotTableDefinition name="P" cacheId="1"/>"#,
        r#"<pivotTableDefinition name="P" cacheId="1"><location ref="nonsense"/></pivotTableDefinition>"#,
        r#"<pivotTableDefinition name="" cacheId="9"><location ref="$D$3:$F$9"/><pivotFields count="1"><pivotField axis="axisRow" sortType="descending"><items><item x="99" h="1" sd="0"/><item x="-4"/><item t="default"/></items></pivotField></pivotFields><rowFields><field x="0"/><field x="4294967295"/><field x="-2"/><field x="77"/></rowFields><colFields><field x="1"/></colFields><pageFields><pageField fld="0" item="9999"/><pageField fld="-1"/><pageField fld="55"/></pageFields><dataFields><dataField fld="1" subtotal="bogus" showDataAs="index" numFmtId="99999"/><dataField fld="12"/><dataField name="E" fld="0"><extLst><ext><dataField pivotShowAs="percentOfParent"/></ext></extLst></dataField></dataFields></pivotTableDefinition>"#,
    ];
    let caches = [
        None,
        Some("garbage"),
        Some(r#"<pivotCacheDefinition><cacheSource type="external"/></pivotCacheDefinition>"#),
        Some(r#"<pivotCacheDefinition><cacheSource type="worksheet"><worksheetSource ref="zz"/></cacheSource></pivotCacheDefinition>"#),
        Some(r#"<pivotCacheDefinition><cacheSource type="worksheet"><worksheetSource ref="A1:B2"/></cacheSource></pivotCacheDefinition>"#),
        Some(r#"<somethingElse/>"#),
        Some(GOOD_CACHE),
    ];
    for t in &tables {
        for c in &caches {
            let bytes = with_pivot(t, *c);
            let (wb, report) = read_xlsx(&bytes).unwrap();
            let pivots = &wb.sheet(0).unwrap().pivots;
            if pivots.is_empty() {
                assert!(!report.warnings.is_empty(), "no warning for table {t} / cache {c:?}");
            }
        }
    }
    // The fully hostile table against a good cache still yields a pivot, with warnings.
    let (wb, report) = read_xlsx(&with_pivot(tables[4], Some(GOOD_CACHE))).unwrap();
    let pt = &wb.sheet(0).unwrap().pivots[0];
    assert_eq!(pt.anchor, at("D3"));
    assert_eq!(pt.source, "Data!$A$1:$B$3");
    assert_eq!(pt.rows.len(), 1);
    assert_eq!(pt.rows[0].sort, PivotSort::Desc);
    assert!(pt.values.iter().any(|v| v.func == PivotFunc::Sum && v.source_col == "V"));
    assert!(report.warnings.iter().any(|w| w.contains("bogus")), "{:?}", report.warnings);
    assert!(report.warnings.iter().any(|w| w.contains("grouping")), "{:?}", report.warnings);
    assert!(report.warnings.iter().any(|w| w.contains("percentOfParent")), "{:?}", report.warnings);
}

#[test]
fn cache_found_by_workbook_cache_id() {
    let rels_extra = r#"<Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotCacheDefinition" Target="pivotCache/pivotCacheDefinition1.xml"/>"#;
    let table = r#"<pivotTableDefinition name="ById" cacheId="7"><location ref="C1:D4"/><pivotFields><pivotField axis="axisRow"><items><item x="0"/><item x="1"/><item t="default"/></items></pivotField><pivotField dataField="1"/></pivotFields><rowFields><field x="0"/></rowFields><dataFields><dataField name="Total" fld="1" subtotal="average"/></dataFields></pivotTableDefinition>"#;
    let bytes = minimal(
        "<sheetData/>",
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", PT_RELS),
            ("xl/pivotTables/pivotTable1.xml", table),
            ("xl/pivotCache/pivotCacheDefinition1.xml", GOOD_CACHE),
        ],
        rels_extra,
        "",
    );
    // `minimal` puts wb_extra before <sheets>; pivotCaches belongs after, but readers locate it by name.
    let bytes2 = {
        let mut parts = unzip(&bytes);
        let wbx = String::from_utf8(parts.remove("xl/workbook.xml").unwrap()).unwrap();
        let wbx = wbx.replace("</workbook>", r#"<pivotCaches><pivotCache cacheId="7" r:id="rId9"/></pivotCaches></workbook>"#);
        let mut list: Vec<(String, Vec<u8>)> = parts.into_iter().collect();
        list.push(("xl/workbook.xml".into(), wbx.into_bytes()));
        let refs: Vec<(&str, &[u8])> = list.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
        super::make_zip(&refs)
    };
    let (wb, report) = read_xlsx(&bytes2).unwrap();
    let pts = &wb.sheet(0).unwrap().pivots;
    assert_eq!(pts.len(), 1, "{:?}", report.warnings);
    assert_eq!(pts[0].name, "ById");
    assert_eq!(pts[0].values[0].func, PivotFunc::Average);
    assert_eq!(pts[0].rows[0].source_col, "K");
}

#[test]
fn calculated_fields_round_trip() {
    let mut wb = workbook();
    {
        let p = wb.sheet_mut(1).unwrap().pivots.get_mut(0).unwrap();
        p.calculated_fields.push(PivotCalcField { name: "Price".into(), formula: "=Sales/Units".into() });
        p.calculated_fields.push(PivotCalcField { name: "Unit Bonus".into(), formula: "='Price'*0.1".into() });
        p.values.push(PivotValue { source_col: "Price".into(), name: "Sum of Price".into(), ..PivotValue::default() });
    }
    let bytes = write_xlsx(&wb).unwrap();
    let parts = unzip(&bytes);
    let c1 = String::from_utf8(parts.get("xl/pivotCache/pivotCacheDefinition1.xml").unwrap().clone()).unwrap();
    assert!(c1.contains("<cacheField name=\"Price\" numFmtId=\"0\" formula=\"Sales/Units\" databaseField=\"0\">"), "{c1}");
    assert!(c1.contains("formula=\"&apos;Price&apos;*0.1\"") || c1.contains("formula=\"'Price'*0.1\""), "{c1}");
    let t1 = String::from_utf8(parts.get("xl/pivotTables/pivotTable1.xml").unwrap().clone()).unwrap();
    assert!(t1.contains("<dataField name=\"Sum of Price\" fld=\"6\""), "{t1}");
    assert!(t1.contains("dataField=\"1\" dragToRow=\"0\""), "{t1}");
    let (back, report) = read_xlsx(&bytes).unwrap();
    assert!(report.warnings.iter().all(|w| !w.to_lowercase().contains("pivot")), "{:?}", report.warnings);
    let orig = &wb.sheet(1).unwrap().pivots;
    let read = &back.sheet(1).unwrap().pivots;
    for (a, b) in orig.iter().zip(read.iter()) {
        assert_eq!(a, b, "pivot {} differs", a.name);
    }
}

/// The `x` attributes of an axis list (`rowFields`/`colFields`) in a pivotTable part.
fn axis_xs(table: &str, tag: &str) -> Vec<i64> {
    let Some(start) = table.find(&format!("<{tag} ")) else { return vec![] };
    let end = table[start..].find(&format!("</{tag}>")).map_or(table.len(), |e| start + e);
    table[start..end].split("<field x=\"").skip(1).filter_map(|s| s.split('"').next()?.parse().ok()).collect()
}

#[test]
fn values_on_rows_round_trip() {
    let mut wb = workbook();
    {
        let p = wb.sheet_mut(1).unwrap().pivots.get_mut(1).unwrap();
        p.values_on_rows = true;
        p.values_position = Some(1);
    }
    let bytes = write_xlsx(&wb).unwrap();
    let parts = unzip(&bytes);
    let t2 = String::from_utf8(parts.get("xl/pivotTables/pivotTable2.xml").unwrap().clone()).unwrap();
    assert!(t2.contains(" dataOnRows=\"1\""), "{t2}");
    // Σ Values (-2) between the two row fields; Columns keep only Product.
    let rows = axis_xs(&t2, "rowFields");
    assert_eq!(rows.len(), 3, "{t2}");
    assert_eq!(rows[1], -2, "{t2}");
    assert!(!axis_xs(&t2, "colFields").contains(&-2), "{t2}");
    assert!(t2.contains("firstDataRow=\"2\" firstDataCol=\"3\""), "{t2}");
    let (back, report) = read_xlsx(&bytes).unwrap();
    assert!(report.warnings.iter().all(|w| !w.to_lowercase().contains("pivot")), "{:?}", report.warnings);
    assert_eq!(back.sheet(1).unwrap().pivots, wb.sheet(1).unwrap().pivots);

    // After the last row field.
    wb.sheet_mut(1).unwrap().pivots[1].values_position = None;
    let bytes = write_xlsx(&wb).unwrap();
    let t2 = String::from_utf8(unzip(&bytes).get("xl/pivotTables/pivotTable2.xml").unwrap().clone()).unwrap();
    assert_eq!(axis_xs(&t2, "rowFields").last(), Some(&-2), "{t2}");
    let (back, _) = read_xlsx(&bytes).unwrap();
    assert_eq!(back.sheet(1).unwrap().pivots, wb.sheet(1).unwrap().pivots);

    // Σ Values in Columns is written after the last column field, its only place in the layout.
    {
        let p = wb.sheet_mut(1).unwrap().pivots.get_mut(1).unwrap();
        p.values_on_rows = false;
        p.values_position = Some(0);
    }
    let bytes = write_xlsx(&wb).unwrap();
    let t2 = String::from_utf8(unzip(&bytes).get("xl/pivotTables/pivotTable2.xml").unwrap().clone()).unwrap();
    assert!(!t2.contains("dataOnRows"), "{t2}");
    assert_eq!(axis_xs(&t2, "colFields").last(), Some(&-2), "{t2}");
    let (back, _) = read_xlsx(&bytes).unwrap();
    assert_eq!(back.sheet(1).unwrap().pivots[1].values_position, None);
}

#[test]
fn values_ahead_of_column_fields_read_last() {
    let table = r#"<pivotTableDefinition name="ValuesFirst" cacheId="1"><location ref="C1:F4"/><pivotFields><pivotField axis="axisCol"><items><item x="0"/><item x="1"/><item t="default"/></items></pivotField><pivotField dataField="1"/></pivotFields><colFields count="2"><field x="-2"/><field x="0"/></colFields><dataFields><dataField name="Sum of V" fld="1"/><dataField name="Count of V" fld="1" subtotal="count"/></dataFields></pivotTableDefinition>"#;
    let (wb, _) = read_xlsx(&with_pivot(table, Some(GOOD_CACHE))).unwrap();
    let pt = &wb.sheet(0).unwrap().pivots[0];
    assert_eq!((pt.columns.len(), pt.values.len()), (1, 2));
    assert!(!pt.values_on_rows);
    assert_eq!(pt.values_position, None);
}

// As Excel saves them: `rowGrandTotals="0"` drops the Grand Total row under the report (leaving
// Excel's "On for Rows Only"), `colGrandTotals="0"` the Grand Total column at its right ("On for
// Columns Only"). LibreOffice's Pivot2_Row_Subtotals.xlsx (`rowGrandTotals="0"`) shows no Grand
// Total row; ClosedXML's TemplateWithTableSourcePivotTables.xlsx pivotTable5 (`colGrandTotals="0"`,
// row and column fields) shows a Grand Total row and no Grand Total column.
#[test]
fn grand_totals_read_in_excel_direction() {
    let table = |attr: &str| {
        format!(
            r#"<pivotTableDefinition name="GT" cacheId="1" {attr}><location ref="C1:F4"/><pivotFields><pivotField axis="axisRow"><items><item x="0"/><item x="1"/><item t="default"/></items></pivotField><pivotField dataField="1"/></pivotFields><rowFields><field x="0"/></rowFields><dataFields><dataField name="Sum of V" fld="1"/></dataFields></pivotTableDefinition>"#
        )
    };
    for (attr, rows, cols) in [("rowGrandTotals=\"0\"", true, false), ("colGrandTotals=\"0\"", false, true), ("", true, true)] {
        let (wb, _) = read_xlsx(&with_pivot(&table(attr), Some(GOOD_CACHE))).unwrap();
        let pt = &wb.sheet(0).unwrap().pivots[0];
        assert_eq!((pt.grand_totals_rows, pt.grand_totals_cols), (rows, cols), "{attr}");
    }
}

// `enableDrill="0"` is PivotTable Options › Data › Enable show details turned off; without the
// attribute the option is on.
#[test]
fn enable_show_details_reads_and_writes_enable_drill() {
    let table = |attr: &str| {
        format!(
            r#"<pivotTableDefinition name="Drill" cacheId="1" {attr}><location ref="C1:F4"/><pivotFields><pivotField axis="axisRow"><items><item x="0"/><item x="1"/><item t="default"/></items></pivotField><pivotField dataField="1"/></pivotFields><rowFields><field x="0"/></rowFields><dataFields><dataField name="Sum of V" fld="1"/></dataFields></pivotTableDefinition>"#
        )
    };
    for (attr, on) in [("enableDrill=\"0\"", false), ("enableDrill=\"1\"", true), ("", true)] {
        let (wb, _) = read_xlsx(&with_pivot(&table(attr), Some(GOOD_CACHE))).unwrap();
        assert_eq!(wb.sheet(0).unwrap().pivots[0].enable_show_details, on, "{attr}");
        let parts = unzip(&write_xlsx(&wb).unwrap());
        let written = String::from_utf8(parts.get("xl/pivotTables/pivotTable1.xml").unwrap().clone()).unwrap();
        assert_eq!(written.contains(" enableDrill=\"0\""), !on, "{attr}: {written}");
    }
}

#[test]
fn data_on_rows_with_one_value_field_reads_without_warning() {
    let table = r#"<pivotTableDefinition name="OneValue" cacheId="1" dataOnRows="1"><location ref="C1:D4"/><pivotFields><pivotField axis="axisRow"><items><item x="0"/><item x="1"/><item t="default"/></items></pivotField><pivotField dataField="1"/></pivotFields><rowFields><field x="0"/></rowFields><dataFields><dataField name="Total" fld="1"/></dataFields></pivotTableDefinition>"#;
    let (wb, report) = read_xlsx(&with_pivot(table, Some(GOOD_CACHE))).unwrap();
    assert!(report.warnings.iter().all(|w| !w.contains("values on rows")), "{:?}", report.warnings);
    let pt = &wb.sheet(0).unwrap().pivots[0];
    assert!(pt.values_on_rows);
    assert_eq!(pt.values_position, None);
    let (back, _) = read_xlsx(&write_xlsx(&wb).unwrap()).unwrap();
    assert!(back.sheet(0).unwrap().pivots[0].values_on_rows);
}

/// A one-sheet-of-data workbook with a pivot on "Report": `columns` are the source columns
/// (header, values), `pt` the pivot built on `Data!A1:<last>`.
fn small_pivot(columns: &[(&str, Vec<Value>)], pt: impl FnOnce(&mut PivotTable)) -> HashMap<String, String> {
    let mut wb = Workbook::new();
    let data = wb.sheet_mut(0).unwrap();
    data.name = "Data".into();
    let mut rows = 0;
    for (c, (h, vals)) in columns.iter().enumerate() {
        data.cells.set(CellRef::new(0, c as u32), Cell::value(Value::text(*h)));
        for (r, v) in vals.iter().enumerate() {
            if *v != Value::Empty {
                data.cells.set(CellRef::new(r as u32 + 1, c as u32), Cell::value(v.clone()));
            }
        }
        rows = rows.max(vals.len());
    }
    let last = format!("{}{}", gridcraft_core::col_to_letters(columns.len() as u32 - 1), rows + 1);
    let mut table = PivotTable { id: 1, name: "PT".into(), source: format!("Data!$A$1:{last}"), anchor: at("A3"), ..PivotTable::default() };
    pt(&mut table);
    let mut report = Sheet::new("Report");
    report.pivots = vec![table];
    wb.sheets.push(Arc::new(report));
    unzip(&write_xlsx(&wb).unwrap()).into_iter().map(|(k, v)| (k, String::from_utf8_lossy(&v).into_owned())).collect()
}

/// #1447: two value fields and no column field: Excel 365 adds a `Values` row unless told not to.
#[test]
fn two_value_fields_without_columns_hide_the_values_row() {
    let cols = [
        ("Region", vec![Value::text("East"), Value::text("West"), Value::text("East")]),
        ("Sales", vec![Value::number(10.0), Value::number(20.0), Value::number(5.0)]),
        ("Qty", vec![Value::number(1.0), Value::number(2.0), Value::number(3.0)]),
    ];
    let value = |c: &str| PivotValue { source_col: c.into(), name: format!("Sum of {c}"), ..PivotValue::default() };
    let two = small_pivot(&cols, |p| {
        p.rows = vec![field("Region")];
        p.values = vec![value("Sales"), value("Qty")];
    });
    let t = &two["xl/pivotTables/pivotTable1.xml"];
    assert!(t.contains("firstHeaderRow=\"0\" firstDataRow=\"1\""), "{t}");
    assert!(t.contains("<x14:pivotTableDefinition hideValuesRow=\"1\"/>"), "{t}");
    let one = small_pivot(&cols, |p| {
        p.rows = vec![field("Region")];
        p.values = vec![value("Sales")];
    });
    let t = &one["xl/pivotTables/pivotTable1.xml"];
    assert!(t.contains("firstHeaderRow=\"1\""), "{t}");
    assert!(!t.contains("hideValuesRow"), "{t}");
}

#[test]
fn pivot_settings_survive_a_round_trip() {
    // These were all silently dropped: the grouping, the sort-by, the report toggles and the
    // filters. Losing them means saving and reopening a grouped or filtered pivot loses the work.
    let mut wb = workbook();
    let pt = &mut wb.sheet_mut(1).unwrap().pivots[2];
    pt.rows[0].group_bins = Some((0.0, 10.0));
    pt.rows[0].grouped_items = vec![vec!["A".to_string(), "B".to_string()]];
    pt.rows[0].sort_by = Some("Sales".to_string());
    pt.rows[0].custom_order = vec!["B".to_string(), "A".to_string()];
    pt.repeat_item_labels = true;
    pt.blank_lines = true;
    pt.show_items_with_no_data = true;
    pt.calculated_items = vec![PivotCalcItem { name: "East+West".into(), field: "Region".into(), formula: "=East+West".into() }];
    pt.filters[0].value_filter = Some(PivotValueFilter { n: 2, top: true, by_field: Some("Sales".into()), ..PivotValueFilter::default() });

    let bytes = write_xlsx(&wb).unwrap();
    let (back, _) = read_xlsx(&bytes).unwrap();
    let got = &back.sheet(1).unwrap().pivots[2];
    assert_eq!(got.rows[0].group_bins, Some((0.0, 10.0)), "number bins");
    assert_eq!(got.rows[0].grouped_items, vec![vec!["A".to_string(), "B".to_string()]], "manual text groups");
    assert_eq!(got.rows[0].sort_by.as_deref(), Some("Sales"), "sort by a value field");
    assert_eq!(got.rows[0].custom_order, vec!["B".to_string(), "A".to_string()], "custom order");
    assert!(got.repeat_item_labels, "repeat item labels");
    assert!(got.blank_lines, "blank lines between groups");
    assert!(got.show_items_with_no_data, "show items with no data");
    assert_eq!(got.calculated_items.len(), 1, "calculated items");
    assert_eq!(got.calculated_items[0].formula, "=East+West");
    let vf = got.filters.iter().find_map(|f| f.value_filter.as_ref()).expect("the value filter survives");
    assert_eq!((vf.n, vf.top), (2, true), "Top 2 by Sales");
    assert_eq!(vf.by_field.as_deref(), Some("Sales"));
}

#[test]
fn the_gridcraft_extension_is_well_formed_and_ignored_by_the_reader_when_absent() {
    // The settings live in an `ext` Excel ignores, so it must be a well-formed element nested
    // inside the standard extLst, and a file without it must still read.
    let mut wb = workbook();
    let pt = &mut wb.sheet_mut(1).unwrap().pivots[2];
    pt.blank_lines = true;
    pt.rows[0].group_bins = Some((2.5, 7.0));
    pt.rows[0].sort_by = Some("Sales & \"more\"".to_string());
    let parts = unzip(&write_xlsx(&wb).unwrap());
    let xml = String::from_utf8(parts.get("xl/pivotTables/pivotTable3.xml").cloned().unwrap_or_default()).unwrap();
    assert!(xml.contains("<extLst>"), "the extension list is written");
    assert!(xml.contains("<gc:pivotSettings"), "our settings are inside it");
    assert!(xml.contains("binStart=\"2.5\"") && xml.contains("binSize=\"7\""), "the bin is escaped and written");
    assert!(xml.contains("Sales &amp; &quot;more&quot;"), "attributes are XML-escaped");
    // Every `ext` sits in the one extLst, which ends once, right before the closing element.
    assert_eq!(xml.matches("<extLst>").count(), 1);
    assert_eq!(xml.matches("</extLst>").count(), 1);
    assert!(xml.ends_with("</extLst></pivotTableDefinition>"));
    // A pivot with no such settings writes no extension at all.
    let plain = unzip(&write_xlsx(&workbook()).unwrap());
    let plain_xml = String::from_utf8(plain.get("xl/pivotTables/pivotTable3.xml").cloned().unwrap_or_default()).unwrap();
    assert!(!plain_xml.contains("gc:pivotSettings"));
}

#[test]
fn a_shared_cache_is_not_reused_for_a_pivot_it_cannot_serve() {
    // `shared_cache` can arrive from JSON (imported or hand-built workbooks), not only from our own
    // writer: two tables may share a key while disagreeing. A date grouping is the clearest case —
    // the shared cache has no month field, so the second table must get a cache of its own rather
    // than borrow the first one's axis mapping.
    let mut wb = workbook();
    {
        let sh = wb.sheet_mut(1).unwrap();
        sh.pivots[1].rows = vec![PivotField { source_col: "Region".into(), ..PivotField::default() }];
        sh.pivots[2].rows = vec![PivotField { source_col: "Date".into(), date_group: PivotDateGroup::Months, ..PivotField::default() }];
        sh.pivots[1].shared_cache = Some("Cache1".into());
        sh.pivots[2].shared_cache = Some("Cache1".into());
    }
    let bytes = write_xlsx(&wb).unwrap();
    let parts = unzip(&bytes);
    let caches = parts.keys().filter(|k| k.starts_with("xl/pivotCache/pivotCacheDefinition")).count();
    assert!(caches >= 3, "an incompatible share gets its own cache (pivot 0's, the shared one, theirs), got {caches}");
    // And the month grouping is written, so the file reads back with the axis the table asked for.
    let (back, report) = read_xlsx(&bytes).unwrap();
    assert!(report.warnings.iter().all(|w| !w.to_lowercase().contains("pivot")), "{:?}", report.warnings);
    assert_eq!(back.sheet(1).unwrap().pivots[2].rows[0].date_group, PivotDateGroup::Months);
}

#[test]
fn a_compatible_shared_cache_is_still_reused() {
    let mut wb = workbook();
    {
        let sh = wb.sheet_mut(1).unwrap();
        sh.pivots[1].shared_cache = Some("Sales".into());
        sh.pivots[2] = sh.pivots[1].clone();
        sh.pivots[2].id = 99;
        sh.pivots[2].name = "Copy".into();
        sh.pivots[2].anchor = at("A40");
        sh.pivots[2].shared_cache = Some("Sales".into());
    }
    let parts = unzip(&write_xlsx(&wb).unwrap());
    let caches = parts.keys().filter(|k| k.starts_with("xl/pivotCache/pivotCacheDefinition")).count();
    // Pivot 0 has its own cache; the two identical tables after it share one.
    assert_eq!(caches, 2, "one shared cache for the two identical tables, plus pivot 0's");
}

#[test]
fn a_value_fields_base_survives_the_round_trip() {
    // Difference From / % Difference From are computed from `base_field` / `base_item`. The writer
    // used to emit a first-axis field and item 0, which the reader restored as None, so reopening
    // a saved report compared against a different field or sibling.
    let mut wb = workbook();
    {
        let pt = &mut wb.sheet_mut(1).unwrap().pivots[2];
        let v = pt.values.first_mut().unwrap();
        v.show_as = PivotShowAs::DifferenceFrom;
        v.base_field = Some("Units".into());
        v.base_item = Some("B".into());
        // The error/empty replacement text has no standard attribute either.
        v.error_display = Some("#N/A".into());
        v.empty_display = Some("-".into());
    }
    let (back, _) = read_xlsx(&write_xlsx(&wb).unwrap()).unwrap();
    let v = &back.sheet(1).unwrap().pivots[2].values[0];
    assert_eq!(v.show_as, PivotShowAs::DifferenceFrom, "the mode survives");
    assert_eq!(v.base_field.as_deref(), Some("Units"), "the base value field survives");
    assert_eq!(v.base_item.as_deref(), Some("B"), "the base item survives");
    assert_eq!(v.error_display.as_deref(), Some("#N/A"), "the error text survives");
    assert_eq!(v.empty_display.as_deref(), Some("-"), "the empty text survives");
}

#[test]
fn a_shared_cache_key_does_not_cross_sources() {
    // `shared_cache` is plain JSON: two tables can carry the same key while reading different
    // ranges. Reusing the first one's cache would save the second pointing at the wrong data.
    let mut wb = workbook();
    {
        let sh = wb.sheet_mut(1).unwrap();
        sh.pivots[1].shared_cache = Some("Cache1".into());
        sh.pivots[2].shared_cache = Some("Cache1".into());
        // Same field names, different range.
        sh.pivots[2].source = "Data!$A$1:$F$5".into();
    }
    let bytes = write_xlsx(&wb).unwrap();
    let parts = unzip(&bytes);
    let caches = parts.keys().filter(|k| k.starts_with("xl/pivotCache/pivotCacheDefinition")).count();
    assert!(caches >= 3, "a different source gets its own cache (pivot 0's, the shared one, theirs), got {caches}");
    // Each written cache names its own range, so neither table reads the other's rows.
    let ranges: Vec<String> = parts
        .iter()
        .filter(|(k, _)| k.starts_with("xl/pivotCache/pivotCacheDefinition"))
        .map(|(_, v)| String::from_utf8_lossy(v).to_string())
        .filter_map(|x| {
            let i = x.find("<worksheetSource ref=\"")? + "<worksheetSource ref=\"".len();
            x[i..].split('"').next().map(str::to_string)
        })
        .collect();
    assert!(ranges.iter().any(|r| r.ends_with("F5")), "the second table's own range is written, got {ranges:?}");
}

#[test]
fn a_shared_cache_key_does_not_hide_calculated_fields() {
    // Same source and key, but the second table has a calculated field the first's cache lacks.
    // Reusing that cache made `table_xml` drop the value field built on it.
    let mut wb = workbook();
    {
        let sh = wb.sheet_mut(1).unwrap();
        sh.pivots[1].shared_cache = Some("Shared".into());
        sh.pivots[1].calculated_fields = vec![];
        sh.pivots[2].shared_cache = Some("Shared".into());
        sh.pivots[2].source = sh.pivots[1].source.clone();
        sh.pivots[2].calculated_fields = vec![PivotCalcField { name: "Margin".into(), formula: "=Sales-Units".into() }];
        sh.pivots[2].values =
            vec![PivotValue { source_col: "Margin".into(), func: PivotFunc::Sum, name: "Sum of Margin".into(), ..PivotValue::default() }];
    }
    let parts = unzip(&write_xlsx(&wb).unwrap());
    let caches = parts.keys().filter(|k| k.starts_with("xl/pivotCache/pivotCacheDefinition")).count();
    assert!(caches >= 3, "the table with its own calculated field gets its own cache, got {caches}");
    // And its calculated field is written, so the value field is not dropped.
    let has_margin =
        parts.iter().filter(|(k, _)| k.starts_with("xl/pivotCache/pivotCacheDefinition")).any(|(_, v)| String::from_utf8_lossy(v).contains("Margin"));
    assert!(has_margin, "a cache carries the calculated field");
}

#[test]
fn an_empty_timeline_selection_survives_the_round_trip() {
    // `pivot.timeline` stores "no date in range" as Some(empty). OOXML hides items per field and
    // refuses to hide them all, so the writer used to reset that to "all visible" and reopening
    // lost the filter.
    let mut wb = workbook();
    {
        let pt = &mut wb.sheet_mut(1).unwrap().pivots[1];
        pt.filters = vec![PivotFilter { source_col: "Product".into(), selected: Some(Vec::new()), value_filter: None }];
    }
    let (back, _) = read_xlsx(&write_xlsx(&wb).unwrap()).unwrap();
    let f = back.sheet(1).unwrap().pivots[1].filters.iter().find(|f| f.source_col == "Product").expect("the filter survives");
    assert_eq!(f.selected.as_deref(), Some(&[][..]), "an explicit empty selection stays empty, not None");
}

#[test]
fn a_label_selection_survives_the_round_trip() {
    let mut wb = workbook();
    {
        let pt = &mut wb.sheet_mut(1).unwrap().pivots[1];
        pt.filters = vec![PivotFilter { source_col: "Product".into(), selected: Some(vec!["A".into(), "C".into()]), value_filter: None }];
    }
    let (back, _) = read_xlsx(&write_xlsx(&wb).unwrap()).unwrap();
    let f = back.sheet(1).unwrap().pivots[1].filters.iter().find(|f| f.source_col == "Product").expect("the filter survives");
    assert_eq!(f.selected.as_deref(), Some(&["A".to_string(), "C".to_string()][..]));
}
