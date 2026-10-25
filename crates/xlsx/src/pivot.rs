//! PivotTables: pivot cache definitions and records (ECMA-376 Part 1 §18.10.1) and PivotTable
//! definitions (§18.10.1.73), written from the model's [`PivotTable`]s and read back into them.
//!
//! Writing: one cache per PivotTable, with a records snapshot of the source list (omitted, with
//! `saveData="0"`, for very large sources) and `refreshOnLoad="1"` so a reader recomputes the
//! report from the source on open. Fields used on an axis get shared items (or, for date
//! grouping, `fieldGroup` group items) so the table's `<items>` can refer to them. The report
//! cells the engine wrote stay on the sheet as plain cells.
//!
//! Reading: the cache's source and field names plus the table's areas, functions, show-values-as,
//! layout flags, grand totals and style become a model [`PivotTable`]. Anything we don't model
//! (OLAP sources, calculated fields, numeric grouping…) becomes a warning.

use std::collections::HashMap;
use std::fmt::Write as _;

use gridcraft_core::date::datetime_from_serial;
use gridcraft_core::{CellRef, DateSystem, RangeRef, Value, number_to_text};
use gridcraft_formula::quote_sheet;
use gridcraft_model::{
    PivotCalcField, PivotCalcItem, PivotDateGroup, PivotField, PivotFilter, PivotFunc, PivotLayout, PivotShowAs, PivotSort, PivotTable, PivotValue,
    PivotValueFilter, Workbook,
};

use crate::IoError;
use crate::read::Ctx;
use crate::write::{NS_MAIN, NS_REL, Out, Rels, XML_DECL, abs_range};
use crate::xml::{El, esc_attr};

pub const CT_PIVOT_TABLE: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotTable+xml";
pub const CT_PIVOT_CACHE_DEF: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml";
pub const CT_PIVOT_CACHE_REC: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheRecords+xml";

/// Largest source block scanned for a cache (cells); bigger sources are not written.
const MAX_SOURCE_CELLS: u64 = 10_000_000;
/// Largest records snapshot written (cells); bigger caches are written without records.
const MAX_RECORD_CELLS: u64 = 2_000_000;
/// Most shared items kept per field (Excel's limit is 1,048,576 unique items).
const MAX_ITEMS: usize = 1_048_576;
/// Most cache fields read.
const MAX_FIELDS: usize = 16_384;
/// The `x14:dataField` extension (Excel 2010 show-values-as additions such as rank).
const EXT_X14_DATA_FIELD: &str = "{E15A36E0-9728-4e99-A89B-3F7291B0FE68}";
const EXT_X14_PIVOT_TABLE: &str = "{962EF5D1-5CA2-4c93-8EF4-DBF5C05439D2}";
const NS_X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
/// GridCraft's own `<pivotTableDefinition>` extension: the report settings OOXML has no attribute
/// for. Excel ignores an `ext` it does not know, so this round-trips through our own app without
/// risking a repair prompt in Excel (unlike inventing attributes on the standard elements).
const EXT_GRIDCRAFT: &str = "{A1B2C3D4-5E6F-4A7B-8C9D-0E1F2A3B4C5D}";
const NS_GRIDCRAFT: &str = "https://gridcraft.dev/xlsx/pivot";

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

// ================================================================ shared helpers

/// The pivot item label of a value (blank shows as "(blank)").
fn label(v: &Value) -> String {
    match v {
        Value::Empty => "(blank)".into(),
        Value::Number(n) => number_to_text(*n),
        Value::Text(t) => t.to_string(),
        Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
        Value::Error(e) => e.as_str().into(),
        Value::Array(_) => label(&v.scalar()),
    }
}

fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

fn func_name(f: PivotFunc) -> &'static str {
    match f {
        PivotFunc::Sum => "sum",
        PivotFunc::Count => "count",
        PivotFunc::Average => "average",
        PivotFunc::Max => "max",
        PivotFunc::Min => "min",
        PivotFunc::Product => "product",
        PivotFunc::CountNumbers => "countNums",
        PivotFunc::StdDev => "stdDev",
        PivotFunc::StdDevP => "stdDevp",
        PivotFunc::Var => "var",
        PivotFunc::VarP => "varp",
    }
}

fn func_from(s: &str) -> Option<PivotFunc> {
    Some(match s {
        "sum" => PivotFunc::Sum,
        "count" => PivotFunc::Count,
        "average" => PivotFunc::Average,
        "max" => PivotFunc::Max,
        "min" => PivotFunc::Min,
        "product" => PivotFunc::Product,
        "countNums" => PivotFunc::CountNumbers,
        "stdDev" => PivotFunc::StdDev,
        "stdDevp" => PivotFunc::StdDevP,
        "var" => PivotFunc::Var,
        "varp" => PivotFunc::VarP,
        _ => return None,
    })
}

fn func_caption(f: PivotFunc) -> &'static str {
    match f {
        PivotFunc::Sum => "Sum",
        PivotFunc::Count => "Count",
        PivotFunc::Average => "Average",
        PivotFunc::Max => "Max",
        PivotFunc::Min => "Min",
        PivotFunc::Product => "Product",
        PivotFunc::CountNumbers => "Count",
        PivotFunc::StdDev => "StdDev",
        PivotFunc::StdDevP => "StdDevp",
        PivotFunc::Var => "Var",
        PivotFunc::VarP => "Varp",
    }
}

fn group_by_name(g: PivotDateGroup) -> &'static str {
    match g {
        PivotDateGroup::Years => "years",
        PivotDateGroup::Quarters => "quarters",
        PivotDateGroup::Months => "months",
        PivotDateGroup::Days => "days",
        PivotDateGroup::Hours => "hours",
        PivotDateGroup::Minutes => "minutes",
        PivotDateGroup::Seconds => "seconds",
        PivotDateGroup::None => "range",
    }
}

fn group_caption(g: PivotDateGroup) -> &'static str {
    match g {
        PivotDateGroup::Years => "Years",
        PivotDateGroup::Quarters => "Quarters",
        PivotDateGroup::Months => "Months",
        PivotDateGroup::Days => "Days",
        PivotDateGroup::Hours => "Hours",
        PivotDateGroup::Minutes => "Minutes",
        PivotDateGroup::Seconds => "Seconds",
        PivotDateGroup::None => "Group",
    }
}

/// `2024-03-05T00:00:00` for a date serial.
fn iso(sys: DateSystem, serial: f64) -> Option<String> {
    let d = datetime_from_serial(sys, serial)?;
    Some(format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", d.year, d.month, d.day, d.hour, d.minute, d.second))
}

/// `3/5/2024` for a date serial (group boundary captions).
fn mdy(sys: DateSystem, serial: f64) -> String {
    match datetime_from_serial(sys, serial) {
        Some(d) => format!("{}/{}/{}", d.month, d.day, d.year),
        None => number_to_text(serial),
    }
}

// ================================================================ writing

/// Identity of a shared item (text items match case-insensitively, as in Excel).
#[derive(Clone, PartialEq, Eq, Hash)]
enum Key {
    Blank,
    Num(u64),
    Text(String),
    Bool(bool),
    Err(&'static str),
}

fn key(v: &Value) -> Key {
    match v {
        Value::Empty => Key::Blank,
        Value::Number(n) => Key::Num(if *n == 0.0 { 0 } else { n.to_bits() }),
        Value::Text(t) => Key::Text(t.to_lowercase()),
        Value::Bool(b) => Key::Bool(*b),
        Value::Error(e) => Key::Err(e.as_str()),
        Value::Array(_) => key(&v.scalar()),
    }
}

#[derive(Default, Clone)]
struct Stats {
    blank: bool,
    text: bool,
    number: bool,
    non_int: bool,
    boolean: bool,
    error: bool,
    long: bool,
    min: f64,
    max: f64,
}

impl Stats {
    fn add(&mut self, v: &Value) {
        match v {
            Value::Empty => self.blank = true,
            Value::Number(n) => {
                if !self.number {
                    self.min = *n;
                    self.max = *n;
                }
                self.number = true;
                self.min = self.min.min(*n);
                self.max = self.max.max(*n);
                if n.fract() != 0.0 {
                    self.non_int = true;
                }
            }
            Value::Text(t) => {
                self.text = true;
                if t.chars().count() > 255 {
                    self.long = true;
                }
            }
            Value::Bool(_) => self.boolean = true,
            Value::Error(_) => self.error = true,
            Value::Array(_) => self.add(&v.scalar()),
        }
    }
    /// Excel counts booleans and errors as strings in the cache field flags.
    fn strings(&self) -> bool {
        self.text || self.boolean || self.error
    }
    /// Kinds for `containsMixedTypes`: strings (text, booleans, errors) and numbers.
    fn kinds(&self) -> usize {
        usize::from(self.strings()) + usize::from(self.number)
    }
}

/// Date grouping of a cache field.
#[derive(Clone)]
struct Group {
    by: PivotDateGroup,
    /// The database field the grouping applies to.
    base: usize,
    /// The next (coarser) grouping field of the same base.
    par: Option<usize>,
    start: f64,
    end: f64,
    /// Group item captions, including the `<start` and `>end` items.
    items: Vec<String>,
}

#[derive(Clone)]
struct CField {
    name: String,
    /// Column offset in the source (database fields only).
    col: Option<u32>,
    stats: Stats,
    /// Shared items (written when `itemized`).
    items: Vec<Value>,
    index: HashMap<Key, u32>,
    itemized: bool,
    group: Option<Group>,
    /// A calculated field's formula (without `=`); such fields aren't database fields.
    formula: Option<String>,
}

impl CField {
    /// Item captions the PivotTable's `<item x>` indices refer to.
    fn item_labels(&self) -> Vec<String> {
        match &self.group {
            Some(g) => g.items.clone(),
            None => self.items.iter().map(label).collect(),
        }
    }
}

/// A cache built from a PivotTable's source.
#[derive(Clone)]
struct Cache {
    fields: Vec<CField>,
    /// Pivot field → cache field, by the pivot field's position in rows, then columns.
    axis_map: Vec<Option<usize>>,
    source_xml: String,
    records: Option<(u64, String)>,
}

/// PivotTables being written; the tables themselves are finished after the styles part (their
/// value number formats need style number-format ids).
#[derive(Default)]
pub struct PivotWriter {
    /// (cache id, workbook-relative part name).
    caches: Vec<(u32, String)>,
    pending: Vec<Pending>,
    tables: u32,
    /// Reused caches keyed by shared_cache name.
    shared: std::collections::HashMap<String, (u32, String, Cache)>,
}

struct Pending {
    part: String,
    sheet: usize,
    idx: usize,
    cache_id: u32,
    cache: Cache,
}

impl PivotWriter {
    /// Writes the caches of a sheet's PivotTables and adds the sheet's relationships to its
    /// PivotTable parts. Pivots whose source can't be resolved are not written.
    pub fn add_sheet(&mut self, wb: &Workbook, si: usize, out: &mut Out, rels: &mut Rels) {
        let Some(sheet) = wb.sheet(si) else { return };
        for (idx, pt) in sheet.pivots.iter().enumerate() {
            // Reuse an existing cache when the pivot shares one (shared_cache) *and* that cache was
            // built from the same source and can serve this table's axes and groupings; otherwise
            // fall through and write its own. A key alone proves nothing: it can arrive from JSON.
            let shared_key = pt.shared_cache.clone();
            let src = resolve_source(wb, &pt.source, si, 0).map(|(_, _, xml)| xml);
            if let Some((cid, cache_part, cache)) = shared_key.as_ref().and_then(|k| self.shared.get(k))
                && src.as_deref() == Some(cache.source_xml.as_str())
                && calc_fields_match(&cache.fields, pt)
                && let Some(axis_map) = shared_axis_map(&cache.fields, pt)
            {
                let mut reused_cache = cache.clone();
                reused_cache.axis_map = axis_map;
                self.tables += 1;
                let t = self.tables;
                rels.add("pivotTable", &format!("../pivotTables/pivotTable{t}.xml"));
                let mut tr = Rels::default();
                tr.add("pivotCacheDefinition", &format!("../pivotCache/{}", cache_part.trim_start_matches("pivotCache/")));
                out.part(&format!("xl/pivotTables/_rels/pivotTable{t}.xml.rels"), None, tr.xml().into_bytes());
                self.pending.push(Pending { part: format!("xl/pivotTables/pivotTable{t}.xml"), sheet: si, idx, cache_id: *cid, cache: reused_cache });
                continue;
            }
            let Some(cache) = build_cache(wb, pt, si) else { continue };
            let n = self.caches.len() as u32 + 1;
            let cache_part = format!("xl/pivotCache/pivotCacheDefinition{n}.xml");
            let cache_rel = format!("pivotCache/pivotCacheDefinition{n}.xml");
            let rec_rid = if let Some((count, xml)) = &cache.records {
                let mut r = Rels::default();
                let rid = r.add("pivotCacheRecords", &format!("pivotCacheRecords{n}.xml"));
                out.part(&format!("xl/pivotCache/_rels/pivotCacheDefinition{n}.xml.rels"), None, r.xml().into_bytes());
                let s = format!("{XML_DECL}<pivotCacheRecords xmlns=\"{NS_MAIN}\" xmlns:r=\"{NS_REL}\" count=\"{count}\">{xml}</pivotCacheRecords>");
                out.part(&format!("xl/pivotCache/pivotCacheRecords{n}.xml"), Some(CT_PIVOT_CACHE_REC), s.into_bytes());
                Some(rid)
            } else {
                None
            };
            out.part(&cache_part, Some(CT_PIVOT_CACHE_DEF), cache_xml(&cache, rec_rid.as_deref(), wb.date_system).into_bytes());
            self.caches.push((n, format!("pivotCache/pivotCacheDefinition{n}.xml")));
            self.tables += 1;
            let t = self.tables;
            rels.add("pivotTable", &format!("../pivotTables/pivotTable{t}.xml"));
            let mut tr = Rels::default();
            tr.add("pivotCacheDefinition", &format!("../pivotCache/pivotCacheDefinition{n}.xml"));
            out.part(&format!("xl/pivotTables/_rels/pivotTable{t}.xml.rels"), None, tr.xml().into_bytes());
            self.pending.push(Pending { part: format!("xl/pivotTables/pivotTable{t}.xml"), sheet: si, idx, cache_id: n, cache: cache.clone() });
            if let Some(k) = shared_key {
                self.shared.insert(k, (n, cache_rel.clone(), cache.clone()));
            }
        }
    }

    /// Adds the workbook relationships to the caches; returns the `<pivotCaches>` element (empty
    /// when there are none).
    pub fn workbook_xml(&self, wb_rels: &mut Rels) -> String {
        if self.caches.is_empty() {
            return String::new();
        }
        let mut s = String::from("<pivotCaches>");
        for (id, target) in &self.caches {
            let rid = wb_rels.add("pivotCacheDefinition", target);
            let _ = write!(s, "<pivotCache cacheId=\"{id}\" r:id=\"{rid}\"/>");
        }
        s.push_str("</pivotCaches>");
        s
    }

    /// Number formats used by value fields (to be registered in the styles part).
    pub fn number_formats(&self, wb: &Workbook) -> Vec<String> {
        let mut v = Vec::new();
        for p in &self.pending {
            if let Some(pt) = wb.sheet(p.sheet).and_then(|s| s.pivots.get(p.idx)) {
                for val in &pt.values {
                    if let Some(f) = &val.number_format
                        && !v.contains(f)
                    {
                        v.push(f.clone());
                    }
                }
            }
        }
        v
    }

    /// Writes the PivotTable parts. `fmt_id` maps a number format code to its styles id.
    pub fn finish(self, wb: &Workbook, out: &mut Out, fmt_id: &dyn Fn(&str) -> Option<u32>) {
        for p in self.pending {
            let Some(pt) = wb.sheet(p.sheet).and_then(|s| s.pivots.get(p.idx)) else { continue };
            let xml = table_xml(pt, p.idx, p.cache_id, &p.cache, fmt_id);
            out.part(&p.part, Some(CT_PIVOT_TABLE), xml.into_bytes());
        }
    }
}

/// Resolves a pivot source to (sheet, range including the header row, worksheetSource XML).
fn resolve_source(wb: &Workbook, text: &str, default_sheet: usize, depth: u32) -> Option<(usize, RangeRef, String)> {
    let t = text.trim().trim_start_matches('=').trim();
    if t.is_empty() || depth > 4 {
        return None;
    }
    if let Some((si, ti)) = wb.table(t)
        && let Some(sh) = wb.sheet(si)
        && let Some(tb) = sh.tables.get(ti)
    {
        if !tb.header_row {
            return None;
        }
        let end_row = tb.range.end.row.saturating_sub(tb.totals_row as u32).max(tb.range.start.row);
        let r = RangeRef::new(tb.range.start, CellRef::new(end_row, tb.range.end.col));
        // Table names are sanitized when written; refer by range when this one would change.
        let valid = !tb.name.is_empty()
            && tb.name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.')
            && tb.name.starts_with(|c: char| c.is_alphabetic() || c == '_')
            && CellRef::parse(&tb.name).is_none();
        let xml = if valid { format!("<worksheetSource name=\"{}\"/>", esc_attr(&tb.name)) } else { range_source_xml(&sh.name, r) };
        return Some((si, r, xml));
    }
    let (sheet, body) = gridcraft_core::split_sheet(t);
    if let Some(r) = RangeRef::parse(body) {
        let si = match &sheet {
            Some(n) => wb.sheet_index(n)?,
            None => default_sheet,
        };
        let sh = wb.sheet(si)?;
        let r = if r.is_full_cols() || r.is_full_rows() { r.intersection(&sh.used_range()?)? } else { r };
        return Some((si, r, range_source_xml(&sh.name, r)));
    }
    if sheet.is_none()
        && let Some(n) = wb.name(t, default_sheet)
    {
        let (si, r, _) = resolve_source(wb, &n.formula.clone(), default_sheet, depth + 1)?;
        let xml = if n.scope.is_none() {
            format!("<worksheetSource name=\"{}\"/>", esc_attr(&n.name))
        } else {
            wb.sheet(si).map(|s| range_source_xml(&s.name, r))?
        };
        return Some((si, r, xml));
    }
    None
}

fn range_source_xml(sheet: &str, r: RangeRef) -> String {
    let name: String = sheet.chars().take(31).collect();
    format!("<worksheetSource ref=\"{}\" sheet=\"{}\"/>", r.a1(), esc_attr(&name))
}

fn header_text(v: &Value) -> String {
    match v {
        Value::Empty => String::new(),
        v => label(v).trim().to_string(),
    }
}

/// Whether a cache already carries this PivotTable's calculated fields (name and formula). A value
/// field built on a calculated field is dropped by `table_xml` when the reused cache lacks it.
fn calc_fields_match(fields: &[CField], pt: &PivotTable) -> bool {
    pt.calculated_fields.iter().all(|cf| {
        let f = cf.formula.trim();
        let f = f.strip_prefix('=').unwrap_or(f).trim();
        fields.iter().any(|x| x.formula.as_deref().is_some_and(|xf| same(&x.name, &cf.name) && xf == f))
    })
}

/// Maps this PivotTable's axis fields onto an existing cache's fields, or `None` when the cache
/// can't serve it: a field that isn't a database field of that cache, a date grouping the cache
/// didn't build, or an axis/page field whose items the cache never itemized (so the table part
/// would write an empty `<items>` list). A shared cache is reused only when this succeeds.
fn shared_axis_map(fields: &[CField], pt: &PivotTable) -> Option<Vec<Option<usize>>> {
    let mut map = Vec::with_capacity(pt.rows.len() + pt.columns.len());
    for pf in pt.rows.iter().chain(pt.columns.iter()) {
        let b = fields.iter().position(|f| f.col.is_some() && same(&f.name, &pf.source_col))?;
        let at = if pf.date_group == PivotDateGroup::None {
            b
        } else {
            fields.iter().position(|f| f.group.as_ref().is_some_and(|g| g.base == b && g.by == pf.date_group))?
        };
        let usable = fields.get(at).is_some_and(|f| f.group.is_some() || f.itemized);
        if !usable {
            return None;
        }
        map.push(Some(at));
    }
    // Page fields need their items too (a one-item selection is written as an item index).
    for f in &pt.filters {
        if pt.rows.iter().chain(pt.columns.iter()).any(|a| same(&a.source_col, &f.source_col)) {
            continue;
        }
        let i = fields.iter().position(|x| x.col.is_some() && same(&x.name, &f.source_col))?;
        if !fields.get(i).is_some_and(|x| x.group.is_some() || x.itemized) {
            return None;
        }
    }
    Some(map)
}

fn build_cache(wb: &Workbook, pt: &PivotTable, pivot_sheet: usize) -> Option<Cache> {
    let (si, r, source_xml) = resolve_source(wb, &pt.source, pivot_sheet, 0)?;
    if r.count() > MAX_SOURCE_CELLS {
        return None;
    }
    let sh = wb.sheet(si)?;
    let mut fields: Vec<CField> = Vec::new();
    // Lower-case names taken, and the next suffix for each header (linear for repeated headers).
    let mut taken = std::collections::HashSet::new();
    let mut next_suffix: HashMap<String, usize> = HashMap::new();
    for (k, c) in (r.start.col..=r.end.col).enumerate() {
        let mut h = header_text(&sh.value(CellRef::new(r.start.row, c)).scalar());
        if h.is_empty() {
            h = format!("Column{}", k + 1);
        }
        let mut n = h.clone();
        let mut i = next_suffix.get(&h.to_lowercase()).copied().unwrap_or(2);
        while taken.contains(&n.to_lowercase()) {
            n = format!("{h}{i}");
            i += 1;
        }
        next_suffix.insert(h.to_lowercase(), i);
        taken.insert(n.to_lowercase());
        fields.push(CField {
            name: n,
            col: Some(k as u32),
            stats: Stats::default(),
            items: vec![],
            index: HashMap::new(),
            itemized: false,
            group: None,
            formula: None,
        });
    }
    let find = |fields: &[CField], name: &str| fields.iter().position(|f| f.col.is_some() && same(&f.name, name));

    // Records (rows that aren't entirely blank), read once.
    let mut rows: Vec<u32> = Vec::new();
    if r.end.row > r.start.row {
        for row in r.start.row + 1..=r.end.row {
            let blank = (r.start.col..=r.end.col).all(|c| sh.value(CellRef::new(row, c)).scalar().is_empty());
            if !blank {
                rows.push(row);
            }
        }
    }
    for &row in &rows {
        for f in fields.iter_mut() {
            let c = r.start.col + f.col.unwrap_or(0);
            f.stats.add(&sh.value(CellRef::new(row, c)).scalar());
        }
    }

    // Which fields are on an axis, and how dates are grouped: per base column, the base field
    // takes the finest grouping and coarser ones become extra (non-database) fields chained
    // through `par`, as Excel lays them out.
    let axis: Vec<&PivotField> = pt.rows.iter().chain(pt.columns.iter()).collect();
    let groupable = |f: &CField| {
        let s = &f.stats;
        s.number
            && !s.text
            && !s.boolean
            && !s.error
            && datetime_from_serial(wb.date_system, s.min).is_some()
            && datetime_from_serial(wb.date_system, s.max).is_some()
    };
    // (base column field, groupings fine → coarse)
    let mut groupings: Vec<(usize, Vec<PivotDateGroup>)> = Vec::new();
    for pf in &axis {
        let Some(b) = find(&fields, &pf.source_col) else { continue };
        if pf.date_group == PivotDateGroup::None || !fields.get(b).is_some_and(groupable) {
            continue;
        }
        match groupings.iter_mut().find(|(x, _)| *x == b) {
            Some((_, v)) => {
                if !v.contains(&pf.date_group) {
                    v.push(pf.date_group);
                }
            }
            None => groupings.push((b, vec![pf.date_group])),
        }
    }
    let fineness = |g: &PivotDateGroup| match g {
        PivotDateGroup::Seconds => 0,
        PivotDateGroup::Minutes => 1,
        PivotDateGroup::Hours => 2,
        PivotDateGroup::Days => 3,
        PivotDateGroup::Months => 4,
        PivotDateGroup::Quarters => 5,
        PivotDateGroup::Years => 6,
        PivotDateGroup::None => 7,
    };
    for (b, list) in groupings.iter_mut() {
        list.sort_by_key(fineness);
        let (min, max) = fields.get(*b).map(|f| (f.stats.min, f.stats.max)).unwrap_or((0.0, 0.0));
        let (start, end) = (min.floor(), max.floor() + 1.0);
        let make = |by: PivotDateGroup| Group { by, base: *b, par: None, start, end, items: group_items(wb.date_system, by, start, end) };
        let mut prev = *b;
        for (k, by) in list.iter().enumerate() {
            if k == 0 {
                if let Some(f) = fields.get_mut(*b) {
                    f.group = Some(make(*by));
                }
                continue;
            }
            let new = fields.len();
            let caption = group_caption(*by);
            let mut n = caption.to_string();
            let mut i = 2;
            while fields.iter().any(|f| same(&f.name, &n)) {
                n = format!("{caption}{i}");
                i += 1;
            }
            if let Some(g) = fields.get_mut(prev).and_then(|f| f.group.as_mut()) {
                g.par = Some(new);
            }
            fields.push(CField {
                name: n,
                col: None,
                stats: Stats::default(),
                items: vec![],
                index: HashMap::new(),
                itemized: false,
                group: Some(make(*by)),
                formula: None,
            });
            prev = new;
        }
    }
    let mut axis_map: Vec<Option<usize>> = Vec::with_capacity(axis.len());
    for pf in &axis {
        let Some(b) = find(&fields, &pf.source_col) else {
            axis_map.push(None);
            continue;
        };
        let grouped = fields.iter().position(|f| f.group.as_ref().is_some_and(|g| g.base == b && g.by == pf.date_group));
        match grouped {
            Some(i) if pf.date_group != PivotDateGroup::None => axis_map.push(Some(i)),
            _ => {
                if let Some(f) = fields.get_mut(b)
                    && f.group.is_none()
                {
                    f.itemized = true;
                }
                axis_map.push(Some(b));
            }
        }
    }
    // Page fields (filters not on an axis) need items too.
    for f in &pt.filters {
        let on_axis = axis.iter().any(|a| same(&a.source_col, &f.source_col));
        if !on_axis
            && let Some(i) = find(&fields, &f.source_col)
            && let Some(cf) = fields.get_mut(i)
        {
            cf.itemized = true;
        }
    }
    for f in fields.iter_mut() {
        if f.group.is_some() {
            f.itemized = false;
        } else if f.col.is_some() && (f.stats.text || f.stats.boolean || f.stats.error) {
            f.itemized = true;
        }
    }
    // Shared items, in order of first appearance.
    for &row in &rows {
        for f in fields.iter_mut().filter(|f| f.itemized) {
            let c = r.start.col + f.col.unwrap_or(0);
            let v = sh.value(CellRef::new(row, c)).scalar();
            let k = key(&v);
            if !f.index.contains_key(&k) && f.items.len() < MAX_ITEMS {
                f.index.insert(k, f.items.len() as u32);
                f.items.push(v);
            }
        }
    }
    // Records snapshot.
    let db_fields = fields.iter().filter(|f| f.col.is_some()).count() as u64;
    let records = if (rows.len() as u64).saturating_mul(db_fields) <= MAX_RECORD_CELLS {
        let mut s = String::new();
        for &row in &rows {
            s.push_str("<r>");
            for f in fields.iter().filter(|f| f.col.is_some()) {
                let c = r.start.col + f.col.unwrap_or(0);
                let v = sh.value(CellRef::new(row, c)).scalar();
                if f.itemized {
                    match f.index.get(&key(&v)) {
                        Some(i) => {
                            let _ = write!(s, "<x v=\"{i}\"/>");
                        }
                        None => s.push_str(&item_xml(&v, wb.date_system, false)),
                    }
                } else {
                    s.push_str(&item_xml(&v, wb.date_system, f.group.is_some()));
                }
            }
            s.push_str("</r>");
        }
        Some((rows.len() as u64, s))
    } else {
        None
    };
    // Calculated fields come after the database and grouping fields.
    for cf in &pt.calculated_fields {
        let f = cf.formula.trim();
        let f = f.strip_prefix('=').unwrap_or(f).trim();
        if cf.name.trim().is_empty() || f.is_empty() || fields.iter().any(|x| same(&x.name, &cf.name)) {
            continue;
        }
        fields.push(CField {
            name: cf.name.clone(),
            col: None,
            stats: Stats::default(),
            items: vec![],
            index: HashMap::new(),
            itemized: false,
            group: None,
            formula: Some(f.to_string()),
        });
    }
    Some(Cache { fields, axis_map, source_xml, records })
}

fn group_items(sys: DateSystem, by: PivotDateGroup, start: f64, end: f64) -> Vec<String> {
    let mut v = vec![format!("<{}", mdy(sys, start))];
    match by {
        PivotDateGroup::Years => {
            let y0 = datetime_from_serial(sys, start).map(|d| d.year).unwrap_or(1900);
            let y1 = datetime_from_serial(sys, end - 1.0).map(|d| d.year).unwrap_or(y0).max(y0);
            for y in y0..=y1.min(y0 + 10_000) {
                v.push(y.to_string());
            }
        }
        PivotDateGroup::Quarters => v.extend((1..=4).map(|q| format!("Qtr{q}"))),
        PivotDateGroup::Months => v.extend(MONTHS.iter().map(ToString::to_string)),
        PivotDateGroup::Days | PivotDateGroup::None => {
            const DAYS: [u32; 12] = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
            for (m, n) in MONTHS.iter().zip(DAYS) {
                for d in 1..=n {
                    v.push(format!("{d}-{m}"));
                }
            }
        }
        PivotDateGroup::Hours => v.extend((0..24).map(|h| format!("{:02}", h))),
        PivotDateGroup::Minutes => v.extend((0..60).map(|m| format!("{:02}", m))),
        PivotDateGroup::Seconds => v.extend((0..60).map(|s| format!("{:02}", s))),
    }
    v.push(format!(">{}", mdy(sys, end)));
    v
}

/// A cache value element (`<n>`, `<s>`, `<m>`…); `date` writes numbers as `<d>`.
fn item_xml(v: &Value, sys: DateSystem, date: bool) -> String {
    match v {
        Value::Empty => "<m/>".into(),
        Value::Number(n) => {
            if date && let Some(d) = iso(sys, *n) {
                return format!("<d v=\"{d}\"/>");
            }
            format!("<n v=\"{}\"/>", crate::xml::num(*n))
        }
        Value::Text(t) => format!("<s v=\"{}\"/>", esc_attr(t)),
        Value::Bool(b) => format!("<b v=\"{}\"/>", u8::from(*b)),
        Value::Error(e) => format!("<e v=\"{}\"/>", esc_attr(e.as_str())),
        Value::Array(_) => item_xml(&v.scalar(), sys, date),
    }
}

fn cache_xml(c: &Cache, records_rid: Option<&str>, sys: DateSystem) -> String {
    let mut s = format!("{XML_DECL}<pivotCacheDefinition xmlns=\"{NS_MAIN}\" xmlns:r=\"{NS_REL}\"");
    match (records_rid, &c.records) {
        (Some(rid), Some((count, _))) => {
            let _ = write!(s, " r:id=\"{rid}\" recordCount=\"{count}\"");
        }
        _ => s.push_str(" saveData=\"0\""),
    }
    s.push_str(" refreshOnLoad=\"1\" refreshedBy=\"GridCraft\" createdVersion=\"6\" refreshedVersion=\"6\" minRefreshableVersion=\"3\">");
    let _ = write!(s, "<cacheSource type=\"worksheet\">{}</cacheSource>", c.source_xml);
    let _ = write!(s, "<cacheFields count=\"{}\">", c.fields.len());
    for f in &c.fields {
        let fmt = if f.group.is_some() { 14 } else { 0 };
        let _ = write!(s, "<cacheField name=\"{}\" numFmtId=\"{fmt}\"", esc_attr(&f.name));
        if let Some(formula) = &f.formula {
            let _ = write!(s, " formula=\"{}\"", esc_attr(formula));
        }
        if f.col.is_none() {
            s.push_str(" databaseField=\"0\"");
        }
        s.push('>');
        let st = &f.stats;
        if f.col.is_some() {
            s.push_str("<sharedItems");
            if let Some(g) = &f.group {
                s.push_str(" containsSemiMixedTypes=\"0\" containsNonDate=\"0\" containsDate=\"1\" containsString=\"0\"");
                if st.blank {
                    s.push_str(" containsBlank=\"1\"");
                }
                if let (Some(a), Some(b)) = (iso(sys, st.min), iso(sys, st.max)) {
                    let _ = write!(s, " minDate=\"{a}\" maxDate=\"{b}\"");
                }
                let _ = g;
            } else {
                if !st.strings() && !st.blank {
                    s.push_str(" containsSemiMixedTypes=\"0\"");
                }
                if !st.strings() {
                    s.push_str(" containsString=\"0\"");
                }
                if st.blank {
                    s.push_str(" containsBlank=\"1\"");
                }
                if st.kinds() > 1 {
                    s.push_str(" containsMixedTypes=\"1\"");
                }
                if st.number {
                    s.push_str(" containsNumber=\"1\"");
                    if !st.non_int {
                        s.push_str(" containsInteger=\"1\"");
                    }
                    let _ = write!(s, " minValue=\"{}\" maxValue=\"{}\"", crate::xml::num(st.min), crate::xml::num(st.max));
                }
                if st.long {
                    s.push_str(" longText=\"1\"");
                }
            }
            if f.itemized {
                let _ = write!(s, " count=\"{}\">", f.items.len());
                for v in &f.items {
                    s.push_str(&item_xml(v, sys, false));
                }
                s.push_str("</sharedItems>");
            } else {
                s.push_str("/>");
            }
        }
        if let Some(g) = &f.group {
            s.push_str("<fieldGroup");
            if let Some(p) = g.par {
                let _ = write!(s, " par=\"{p}\"");
            }
            let _ = write!(s, " base=\"{}\">", g.base);
            let _ = write!(s, "<rangePr groupBy=\"{}\"", group_by_name(g.by));
            if let (Some(a), Some(b)) = (iso(sys, g.start), iso(sys, g.end)) {
                let _ = write!(s, " startDate=\"{a}\" endDate=\"{b}\"");
            }
            let _ = write!(s, "/><groupItems count=\"{}\">", g.items.len());
            for it in &g.items {
                let _ = write!(s, "<s v=\"{}\"/>", esc_attr(it));
            }
            s.push_str("</groupItems></fieldGroup>");
        }
        s.push_str("</cacheField>");
    }
    s.push_str("</cacheFields></pivotCacheDefinition>");
    s
}

fn table_xml(pt: &PivotTable, idx: usize, cache_id: u32, c: &Cache, fmt_id: &dyn Fn(&str) -> Option<u32>) -> String {
    let nf = c.fields.len();
    let find_db = |name: &str| c.fields.iter().position(|f| f.col.is_some() && same(&f.name, name));
    let row_idx: Vec<usize> = c.axis_map.iter().take(pt.rows.len()).filter_map(|x| *x).collect();
    let col_idx: Vec<usize> = c.axis_map.iter().skip(pt.rows.len()).filter_map(|x| *x).collect();
    // Filters: on an axis field → hidden items there; otherwise a page field.
    let mut hidden: HashMap<usize, &PivotFilter> = HashMap::new();
    let mut pages: Vec<(usize, &PivotFilter)> = Vec::new();
    for f in &pt.filters {
        let axis_pos = pt.rows.iter().chain(pt.columns.iter()).position(|a| same(&a.source_col, &f.source_col));
        match axis_pos {
            Some(p) => {
                if let Some(Some(i)) = c.axis_map.get(p) {
                    hidden.insert(*i, f);
                }
            }
            None => {
                if let Some(i) = find_db(&f.source_col)
                    && !pages.iter().any(|(j, _)| *j == i)
                {
                    pages.push((i, f));
                    hidden.insert(i, f);
                }
            }
        }
    }
    let find_calc = |name: &str| c.fields.iter().position(|f| f.formula.is_some() && same(&f.name, name));
    let vals: Vec<(usize, &PivotValue)> =
        pt.values.iter().filter_map(|v| find_db(&v.source_col).or_else(|| find_calc(&v.source_col)).map(|i| (i, v))).collect();
    let axis_field = |i: usize| -> Option<&PivotField> {
        let p = c.axis_map.iter().position(|x| *x == Some(i))?;
        pt.rows.iter().chain(pt.columns.iter()).nth(p)
    };

    let compact = pt.layout == PivotLayout::Compact;
    let name = if pt.name.trim().is_empty() { format!("PivotTable{}", idx + 1) } else { pt.name.clone() };
    let mut s = format!(
        "{XML_DECL}<pivotTableDefinition xmlns=\"{NS_MAIN}\" name=\"{}\" cacheId=\"{cache_id}\" applyNumberFormats=\"0\" applyBorderFormats=\"0\" applyFontFormats=\"0\" applyPatternFormats=\"0\" applyAlignmentFormats=\"0\" applyWidthHeightFormats=\"1\" dataCaption=\"Values\" updatedVersion=\"6\" minRefreshableVersion=\"3\" useAutoFormatting=\"1\" itemPrintTitles=\"1\" createdVersion=\"6\" indent=\"0\"",
        esc_attr(&name)
    );
    match pt.layout {
        PivotLayout::Compact => s.push_str(" outline=\"1\" outlineData=\"1\""),
        PivotLayout::Outline => s.push_str(" compact=\"0\" compactData=\"0\" outline=\"1\" outlineData=\"1\""),
        PivotLayout::Tabular => s.push_str(" compact=\"0\" compactData=\"0\""),
    }
    if pt.values_on_rows {
        s.push_str(" dataOnRows=\"1\"");
    }
    if !pt.show_headers {
        s.push_str(" showHeaders=\"0\"");
    }
    if !pt.enable_show_details {
        s.push_str(" enableDrill=\"0\"");
    }
    // `rowGrandTotals` is the grand total row below the report (Excel's "On for Columns"),
    // `colGrandTotals` the grand total column at its right ("On for Rows").
    if !pt.grand_totals_cols {
        s.push_str(" rowGrandTotals=\"0\"");
    }
    if !pt.grand_totals_rows {
        s.push_str(" colGrandTotals=\"0\"");
    }
    s.push_str(" multipleFieldFilters=\"0\">");

    // Location: the body (report filters sit above it).
    let a = pt.anchor;
    let end = pt.last_range.map(|r| r.end).unwrap_or(a);
    let loc = RangeRef::new(a, CellRef::new(end.row.max(a.row), end.col.max(a.col)));
    let k = vals.len();
    // Σ Values (two or more value fields): its index among the written fields of its axis.
    let (row_map, _) = c.axis_map.split_at(pt.rows.len().min(c.axis_map.len()));
    let values_at = |map: &[Option<usize>], n: usize| pt.values_position.map_or(n, |p| map.iter().take(p).filter(|x| x.is_some()).count());
    let row_values = (k > 1 && pt.values_on_rows).then(|| values_at(row_map, row_idx.len()));
    // In Columns Σ Values comes after the last field: the column layout doesn't place it elsewhere.
    let col_values = (k > 1 && !pt.values_on_rows).then_some(col_idx.len());
    let first_data_row = if col_idx.is_empty() { 1 } else { 1 + col_idx.len() + usize::from(col_values.is_some()) };
    let row_levels = row_idx.len() + usize::from(row_values.is_some());
    let first_data_col = if row_levels == 0 && col_idx.is_empty() {
        0
    } else if compact {
        1
    } else {
        row_levels.max(1)
    };
    // Two or more value fields in Columns alone: Excel 365 would add a `Values` caption row.
    let hide_values_row = col_values.is_some() && col_idx.is_empty();
    let first_header_row = usize::from(!hide_values_row);
    let _ = write!(
        s,
        "<location ref=\"{}\" firstHeaderRow=\"{first_header_row}\" firstDataRow=\"{first_data_row}\" firstDataCol=\"{first_data_col}\"",
        loc.a1()
    );
    if !pages.is_empty() {
        let _ = write!(s, " rowPageCount=\"{}\" colPageCount=\"1\"", pages.len());
    }
    s.push_str("/>");

    // Fields.
    let _ = write!(s, "<pivotFields count=\"{nf}\">");
    for (i, f) in c.fields.iter().enumerate() {
        s.push_str("<pivotField");
        let axis = if row_idx.contains(&i) {
            Some("axisRow")
        } else if col_idx.contains(&i) {
            Some("axisCol")
        } else if pages.iter().any(|(j, _)| *j == i) {
            Some("axisPage")
        } else {
            None
        };
        if let Some(ax) = axis {
            let _ = write!(s, " axis=\"{ax}\"");
        }
        if vals.iter().any(|(j, _)| *j == i) {
            s.push_str(" dataField=\"1\"");
        }
        if f.formula.is_some() {
            s.push_str(" dragToRow=\"0\" dragToCol=\"0\" dragToPage=\"0\"");
        }
        match pt.layout {
            PivotLayout::Compact => {}
            PivotLayout::Outline => s.push_str(" compact=\"0\""),
            PivotLayout::Tabular => s.push_str(" compact=\"0\" outline=\"0\""),
        }
        if f.group.is_some() {
            s.push_str(" numFmtId=\"14\"");
        }
        if pt.repeat_item_labels && axis.is_some() {
            s.push_str(" repeatItemLabels=\"1\"");
        }
        let filter = hidden.get(&i).copied();
        let labels = f.item_labels();
        let is_page = axis == Some("axisPage");
        // Hidden items; never hide every item (a field needs one visible item).
        let mut hide: Vec<bool> = match filter.and_then(|f| f.selected.as_ref()) {
            Some(sel) if !(is_page && sel.len() == 1) => labels.iter().map(|l| !sel.iter().any(|x| same(x, l))).collect(),
            _ => vec![false; labels.len()],
        };
        if !hide.is_empty() && hide.iter().all(|h| *h) {
            hide.iter_mut().for_each(|h| *h = false);
        }
        if is_page && hide.iter().any(|h| *h) {
            s.push_str(" multipleItemSelectionAllowed=\"1\"");
        }
        // `showAll` is the OOXML way to keep items with no records, per axis field.
        s.push_str(if pt.show_items_with_no_data { " showAll=\"1\"" } else { " showAll=\"0\"" });
        if let Some(af) = axis_field(i) {
            match af.sort {
                PivotSort::Asc => s.push_str(" sortType=\"ascending\""),
                PivotSort::Desc => s.push_str(" sortType=\"descending\""),
                PivotSort::None => {}
            }
        }
        if !pt.subtotals {
            s.push_str(" defaultSubtotal=\"0\"");
        }
        if !pt.subtotals_top {
            s.push_str(" subtotalTop=\"0\"");
        }
        if axis.is_some() {
            let collapsed: &[String] = axis_field(i).map(|f| f.collapsed_items.as_slice()).unwrap_or(&[]);
            let count = labels.len() + usize::from(pt.subtotals);
            let _ = write!(s, "><items count=\"{count}\">");
            for (x, l) in labels.iter().enumerate() {
                let _ = write!(s, "<item x=\"{x}\"");
                if hide.get(x).copied().unwrap_or(false) {
                    s.push_str(" h=\"1\"");
                }
                if collapsed.iter().any(|cl| same(cl, l)) {
                    s.push_str(" sd=\"0\"");
                }
                s.push_str("/>");
            }
            if pt.subtotals {
                s.push_str("<item t=\"default\"/>");
            }
            s.push_str("</items></pivotField>");
        } else {
            s.push_str("/>");
        }
    }
    s.push_str("</pivotFields>");

    axis_fields_xml(&mut s, "rowFields", &row_idx, row_values);
    axis_fields_xml(&mut s, "colFields", &col_idx, col_values);
    if !pages.is_empty() {
        let _ = write!(s, "<pageFields count=\"{}\">", pages.len());
        for (i, f) in &pages {
            let _ = write!(s, "<pageField fld=\"{i}\"");
            if let Some(sel) = &f.selected
                && sel.len() == 1
                && let Some(one) = sel.first()
                && let Some(x) = c.fields.get(*i).and_then(|cf| cf.item_labels().iter().position(|l| same(l, one)))
            {
                let _ = write!(s, " item=\"{x}\"");
            }
            s.push_str(" hier=\"-1\"/>");
        }
        s.push_str("</pageFields>");
    }
    if !vals.is_empty() {
        let _ = write!(s, "<dataFields count=\"{k}\">");
        let mut used: Vec<String> = Vec::new();
        for (i, v) in &vals {
            let base = if v.name.trim().is_empty() {
                format!("{} of {}", func_caption(v.func), c.fields.get(*i).map(|f| f.name.as_str()).unwrap_or(""))
            } else {
                v.name.clone()
            };
            let mut n = base.clone();
            let mut j = 2;
            while used.iter().any(|u| same(u, &n)) {
                n = format!("{base}{j}");
                j += 1;
            }
            used.push(n.clone());
            let _ = write!(s, "<dataField name=\"{}\" fld=\"{i}\"", esc_attr(&n));
            if v.func != PivotFunc::Sum {
                let _ = write!(s, " subtotal=\"{}\"", func_name(v.func));
            }
            let base_field = row_idx.first().or(col_idx.first()).copied().unwrap_or(0);
            match v.show_as {
                PivotShowAs::Normal => s.push_str(" baseField=\"0\" baseItem=\"0\""),
                PivotShowAs::PercentOfGrandTotal => s.push_str(" showDataAs=\"percentOfTotal\" baseField=\"0\" baseItem=\"0\""),
                PivotShowAs::PercentOfColumnTotal => s.push_str(" showDataAs=\"percentOfCol\" baseField=\"0\" baseItem=\"0\""),
                PivotShowAs::PercentOfRowTotal => s.push_str(" showDataAs=\"percentOfRow\" baseField=\"0\" baseItem=\"0\""),
                PivotShowAs::PercentOf => {
                    let _ = write!(s, " showDataAs=\"percent\" baseField=\"{base_field}\" baseItem=\"0\"");
                }
                PivotShowAs::DifferenceFrom => {
                    let _ = write!(s, " showDataAs=\"difference\" baseField=\"{base_field}\" baseItem=\"0\"");
                }
                PivotShowAs::PercentDifferenceFrom => {
                    let _ = write!(s, " showDataAs=\"percentDiff\" baseField=\"{base_field}\" baseItem=\"0\"");
                }
                PivotShowAs::RunningTotal => {
                    let _ = write!(s, " showDataAs=\"runTotal\" baseField=\"{base_field}\" baseItem=\"0\"");
                }
                PivotShowAs::PercentRunningTotalIn => {
                    let _ = write!(s, " showDataAs=\"percentRunningTotal\" baseField=\"{base_field}\" baseItem=\"0\"");
                }
                PivotShowAs::Rank => {
                    let _ = write!(s, " baseField=\"{base_field}\" baseItem=\"0\"");
                }
                PivotShowAs::RankAscending => {
                    let _ = write!(s, " baseField=\"{base_field}\" baseItem=\"0\"");
                }
                PivotShowAs::Index => {
                    s.push_str(" showDataAs=\"index\" baseField=\"0\" baseItem=\"0\"");
                }
                PivotShowAs::PercentOfParentRowTotal => {
                    s.push_str(" showDataAs=\"percentOfParentRow\" baseField=\"0\" baseItem=\"0\"");
                }
            }
            if let Some(id) = v.number_format.as_deref().and_then(fmt_id) {
                let _ = write!(s, " numFmtId=\"{id}\"");
            }
            if v.show_as == PivotShowAs::Rank {
                let _ = write!(
                    s,
                    "><extLst><ext uri=\"{EXT_X14_DATA_FIELD}\" xmlns:x14=\"{NS_X14}\"><x14:dataField pivotShowAs=\"rankDescending\"/></ext></extLst></dataField>"
                );
            } else if v.show_as == PivotShowAs::RankAscending {
                let _ = write!(
                    s,
                    "><extLst><ext uri=\"{EXT_X14_DATA_FIELD}\" xmlns:x14=\"{NS_X14}\"><x14:dataField pivotShowAs=\"rankAscending\"/></ext></extLst></dataField>"
                );
            } else {
                s.push_str("/>");
            }
        }
        s.push_str("</dataFields>");
    }
    s.push_str("<pivotTableStyleInfo");
    if !pt.style.is_empty() {
        let _ = write!(s, " name=\"{}\"", esc_attr(&pt.style));
    }
    s.push_str(" showRowHeaders=\"1\" showColHeaders=\"1\" showRowStripes=\"0\" showColStripes=\"0\" showLastColumn=\"1\"/>");
    // Report settings OOXML cannot express: GridCraft's own extension (ignored by Excel).
    let settings = pt_settings_xml(pt);
    if hide_values_row || !settings.is_empty() {
        s.push_str("<extLst>");
        if hide_values_row {
            let _ = write!(s, "<ext uri=\"{EXT_X14_PIVOT_TABLE}\" xmlns:x14=\"{NS_X14}\"><x14:pivotTableDefinition hideValuesRow=\"1\"/></ext>");
        }
        if !settings.is_empty() {
            let _ = write!(s, "<ext uri=\"{EXT_GRIDCRAFT}\" xmlns:gc=\"{NS_GRIDCRAFT}\">{settings}</ext>");
        }
        s.push_str("</extLst>");
    }
    s.push_str("</pivotTableDefinition>");
    s
}

/// The report settings with no OOXML attribute, for the GridCraft extension element.
///
/// `<gc:pivotSettings>` carries the report flags; each `<gc:field>` carries one axis field's
/// grouping, sort-by and custom order; `<gc:item>`/`<gc:valueFilter>` carry calculated items and
/// value filters. `index` is the field's position in the pivot's `rows` then `columns`.
fn pt_settings_xml(pt: &PivotTable) -> String {
    let mut body = String::new();
    let mut flags = String::new();
    if pt.blank_lines {
        flags.push_str(" blankLines=\"1\"");
    }
    let axis: Vec<&PivotField> = pt.rows.iter().chain(pt.columns.iter()).collect();
    for (index, f) in axis.iter().enumerate() {
        if f.group_bins.is_none() && f.grouped_items.is_empty() && f.sort_by.is_none() && f.custom_order.is_empty() {
            continue;
        }
        let _ = write!(body, "<gc:field index=\"{index}\"");
        if let Some((start, size)) = f.group_bins.filter(|(a, b)| a.is_finite() && b.is_finite() && *b > 0.0) {
            let _ = write!(body, " binStart=\"{}\" binSize=\"{}\"", crate::xml::num(start), crate::xml::num(size));
        }
        if let Some(by) = &f.sort_by {
            let _ = write!(body, " sortBy=\"{}\"", esc_attr(by));
        }
        body.push('>');
        for group in &f.grouped_items {
            body.push_str("<gc:group>");
            for it in group {
                let _ = write!(body, "<gc:item v=\"{}\"/>", esc_attr(it));
            }
            body.push_str("</gc:group>");
        }
        for it in &f.custom_order {
            let _ = write!(body, "<gc:order v=\"{}\"/>", esc_attr(it));
        }
        body.push_str("</gc:field>");
    }
    for c in &pt.calculated_items {
        let _ =
            write!(body, "<gc:calculatedItem field=\"{}\" name=\"{}\" formula=\"{}\"/>", esc_attr(&c.field), esc_attr(&c.name), esc_attr(&c.formula));
    }
    // Label selections, including an explicit empty one (a timeline outside every date). OOXML
    // hides items per field and refuses to hide them all, so that state has nowhere else to go.
    for f in pt.filters.iter().filter(|f| f.selected.is_some()) {
        let Some(sel) = f.selected.as_ref() else { continue };
        let _ = write!(body, "<gc:selection field=\"{}\"", esc_attr(&f.source_col));
        body.push('>');
        for label in sel {
            let _ = write!(body, "<gc:item v=\"{}\"/>", esc_attr(label));
        }
        body.push_str("</gc:selection>");
    }
    // A value field's own settings that OOXML's `<dataField>` cannot express: its base for
    // Difference From / % Difference From (GridCraft's base is another *value* field, not an axis
    // field index) and its error/empty replacement text. The standard attributes on `<dataField>`
    // stay as they are for Excel's benefit.
    for (i, v) in pt.values.iter().enumerate() {
        if v.base_field.is_none() && v.base_item.is_none() && v.error_display.is_none() && v.empty_display.is_none() {
            continue;
        }
        let _ = write!(body, "<gc:valueBase index=\"{i}\"");
        if let Some(bf) = &v.base_field {
            let _ = write!(body, " baseField=\"{}\"", esc_attr(bf));
        }
        if let Some(bi) = &v.base_item {
            let _ = write!(body, " baseItem=\"{}\"", esc_attr(bi));
        }
        if let Some(e) = &v.error_display {
            let _ = write!(body, " errorDisplay=\"{}\"", esc_attr(e));
        }
        if let Some(e) = &v.empty_display {
            let _ = write!(body, " emptyDisplay=\"{}\"", esc_attr(e));
        }
        body.push_str("/>");
    }
    for f in pt.filters.iter().filter(|f| f.value_filter.is_some()) {
        let Some(vf) = f.value_filter.as_ref() else { continue };
        let _ = write!(body, "<gc:valueFilter field=\"{}\" n=\"{}\"", esc_attr(&f.source_col), vf.n);
        if vf.top {
            body.push_str(" top=\"1\"");
        }
        if vf.bottom {
            body.push_str(" bottom=\"1\"");
        }
        if vf.top_percent {
            body.push_str(" topPercent=\"1\"");
        }
        for (k, v) in [("above", vf.above), ("below", vf.below), ("betweenLo", vf.between_lo), ("betweenHi", vf.between_hi)] {
            if let Some(v) = v.filter(|v| v.is_finite()) {
                let _ = write!(body, " {k}=\"{}\"", crate::xml::num(v));
            }
        }
        if let Some(by) = &vf.by_field {
            let _ = write!(body, " byField=\"{}\"", esc_attr(by));
        }
        body.push_str("/>");
    }
    // Nothing to record: no element at all, so an untouched pivot's XML stays clean.
    if flags.is_empty() && body.is_empty() {
        return String::new();
    }
    format!("<gc:pivotSettings{flags}>{body}</gc:pivotSettings>")
}

/// `<rowFields>`/`<colFields>`: the axis's cache field indices, with Σ Values (`-2`) at `values_at`.
fn axis_fields_xml(s: &mut String, tag: &str, idx: &[usize], values_at: Option<usize>) {
    let n = idx.len() + usize::from(values_at.is_some());
    if n == 0 {
        return;
    }
    let _ = write!(s, "<{tag} count=\"{n}\">");
    for (j, i) in idx.iter().enumerate() {
        if values_at == Some(j) {
            s.push_str("<field x=\"-2\"/>");
        }
        let _ = write!(s, "<field x=\"{i}\"/>");
    }
    if values_at.is_some_and(|p| p >= idx.len()) {
        s.push_str("<field x=\"-2\"/>");
    }
    let _ = write!(s, "</{tag}>");
}

// ================================================================ reading

/// A cache field as read.
#[derive(Clone, Debug, Default)]
struct FieldIn {
    name: String,
    database: bool,
    /// Shared item captions.
    items: Vec<String>,
    /// (base field, grouping, group item captions); grouping `None` = unsupported grouping.
    group: Option<(Option<usize>, Option<PivotDateGroup>, Vec<String>)>,
    /// A calculated field's formula.
    formula: Option<String>,
}

impl FieldIn {
    fn labels(&self) -> &[String] {
        match &self.group {
            Some((_, _, items)) if !items.is_empty() => items,
            _ => &self.items,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct CacheIn {
    source: String,
    fields: Vec<FieldIn>,
}

/// Workbook-level pivot cache lookup, filled from `<pivotCaches>`; caches are parsed once.
#[derive(Default)]
pub struct CachesIn {
    by_id: HashMap<u32, String>,
    parsed: HashMap<String, Option<CacheIn>>,
    next_id: u32,
}

impl CachesIn {
    pub fn new(wb_xml: &El, wb_rels: &[crate::package::Rel]) -> CachesIn {
        let mut by_id = HashMap::new();
        if let Some(pc) = wb_xml.child("pivotCaches") {
            for c in pc.kids("pivotCache") {
                if let (Some(id), Some(rid)) = (c.attr_u32("cacheId"), c.attr("id"))
                    && let Some(r) = wb_rels.iter().find(|r| r.id == rid)
                {
                    by_id.insert(id, r.target.clone());
                }
            }
        }
        CachesIn { by_id, parsed: HashMap::new(), next_id: 1 }
    }
}

fn item_label_el(e: &El, sys: DateSystem) -> String {
    let v = e.attr("v").unwrap_or("");
    match e.name.as_str() {
        "m" => "(blank)".into(),
        "n" => v.trim().parse::<f64>().ok().filter(|n| n.is_finite()).map(number_to_text).unwrap_or_else(|| v.to_string()),
        "b" => if matches!(v.trim(), "1" | "true") { "TRUE" } else { "FALSE" }.into(),
        "d" => crate::sheet_read::parse_iso_datetime(v, sys).map(number_to_text).unwrap_or_else(|| v.to_string()),
        _ => v.to_string(),
    }
}

fn read_cache(cx: &mut Ctx<'_>, part: &str, sheet_name: &str) -> Option<CacheIn> {
    let x = cx.optional_xml(part).ok().flatten()?;
    if x.name != "pivotCacheDefinition" {
        cx.warn(format!("{part} is not a pivot cache definition; its PivotTable was kept as plain cells"));
        return None;
    }
    let src = x.child("cacheSource");
    let ty = src.and_then(|s| s.attr("type")).unwrap_or("worksheet");
    if ty != "worksheet" {
        cx.warn(format!("PivotTables with a {ty} data source are not supported; kept as plain cells"));
        return None;
    }
    let ws = src.and_then(|s| s.child("worksheetSource"));
    let source = match ws {
        Some(w) if w.attr("name").is_some_and(|n| !n.trim().is_empty()) => w.attr("name").unwrap_or("").trim().to_string(),
        Some(w) => {
            if w.attr("id").is_some() {
                cx.warn("a PivotTable's source is in another workbook; its source reference may not resolve");
            }
            let r = w.attr("ref").map(|r| r.replace('$', "")).and_then(|r| RangeRef::parse(&r));
            let Some(r) = r else {
                cx.warn("a PivotTable's source reference is missing or invalid; kept as plain cells");
                return None;
            };
            let sheet = w.attr("sheet").filter(|s| !s.is_empty()).unwrap_or(sheet_name);
            format!("{}!{}", quote_sheet(sheet), abs_range(&r))
        }
        None => {
            cx.warn("a PivotTable's cache has no worksheet source; kept as plain cells");
            return None;
        }
    };
    let sys = cx.date_system;
    let mut fields = Vec::new();
    if let Some(cf) = x.child("cacheFields") {
        for f in cf.kids("cacheField").take(MAX_FIELDS) {
            let items = f.child("sharedItems").map(|si| si.children.iter().map(|e| item_label_el(e, sys)).collect()).unwrap_or_default();
            let group = f.child("fieldGroup").map(|g| {
                let base = g.attr_u32("base").map(|b| b as usize);
                let by = match g.child("rangePr").map(|r| r.attr("groupBy").unwrap_or("range")) {
                    Some("years") => Some(PivotDateGroup::Years),
                    Some("quarters") => Some(PivotDateGroup::Quarters),
                    Some("months") => Some(PivotDateGroup::Months),
                    Some("days") => Some(PivotDateGroup::Days),
                    Some("hours") => Some(PivotDateGroup::Hours),
                    Some("minutes") => Some(PivotDateGroup::Minutes),
                    Some("seconds") => Some(PivotDateGroup::Seconds),
                    _ => None,
                };
                let gi = g.child("groupItems").map(|gi| gi.children.iter().map(|e| item_label_el(e, sys)).collect()).unwrap_or_default();
                (base, by, gi)
            });
            fields.push(FieldIn {
                name: f.attr("name").unwrap_or("").to_string(),
                database: f.flag("databaseField", true),
                items,
                group,
                formula: f.attr("formula").map(str::to_string),
            });
        }
    }
    if fields.is_empty() {
        cx.warn("a PivotTable's cache has no fields; kept as plain cells");
        return None;
    }
    Some(CacheIn { source, fields })
}

/// Reads the PivotTables related to a worksheet part.
pub fn read_pivots(cx: &mut Ctx<'_>, caches: &mut CachesIn, sheet_part: &str, sheet_name: &str) -> Result<Vec<PivotTable>, IoError> {
    let rels = cx.pkg.rels(sheet_part)?;
    let mut out = Vec::new();
    for rel in rels.iter().filter(|r| r.kind == "pivotTable" && !r.external) {
        let Some(x) = cx.optional_xml(&rel.target)? else { continue };
        if x.name != "pivotTableDefinition" {
            cx.warn(format!("{} is not a PivotTable definition; skipped", rel.target));
            continue;
        }
        // The cache: the table's own relationship, else the workbook's cache id.
        let trels = cx.pkg.rels(&rel.target)?;
        let cache_part = trels
            .iter()
            .find(|r| r.kind == "pivotCacheDefinition" && !r.external)
            .map(|r| r.target.clone())
            .or_else(|| x.attr_u32("cacheId").and_then(|id| caches.by_id.get(&id).cloned()));
        let Some(cache_part) = cache_part else {
            cx.warn(format!("PivotTable in {} has no cache; kept as plain cells", rel.target));
            continue;
        };
        let cache = match caches.parsed.get(&cache_part) {
            Some(c) => c.clone(),
            None => {
                let c = read_cache(cx, &cache_part, sheet_name);
                caches.parsed.insert(cache_part.clone(), c.clone());
                c
            }
        };
        let Some(cache) = cache else { continue };
        if let Some(pt) = read_table(cx, &x, &cache, caches.next_id) {
            caches.next_id = caches.next_id.saturating_add(1);
            out.push(pt);
        }
    }
    Ok(out)
}

/// A pivot field's data items (not subtotal or grand-total items).
fn items_of(e: &El) -> Vec<&El> {
    e.child("items").map(|it| it.kids("item").filter(|i| i.attr("t").is_none_or(|t| t == "data")).collect()).unwrap_or_default()
}

/// (source column name, date grouping) of a cache field used on an axis.
fn field_ref(cx: &mut Ctx<'_>, cache: &CacheIn, i: usize) -> Option<(String, PivotDateGroup)> {
    let f = cache.fields.get(i)?;
    if f.formula.is_some() {
        cx.warn("calculated PivotTable fields are not supported");
        return None;
    }
    match &f.group {
        Some((base, by, _)) => {
            let by = match by {
                Some(b) => *b,
                None => {
                    cx.warn(format!("PivotTable grouping of \"{}\" is not supported; the field is ungrouped", f.name));
                    PivotDateGroup::None
                }
            };
            let base_name = if f.database { f.name.clone() } else { base.and_then(|b| cache.fields.get(b))?.name.clone() };
            Some((base_name, by))
        }
        None if !f.database => {
            cx.warn(format!("PivotTable field \"{}\" (grouped items) is not supported", f.name));
            None
        }
        None => Some((f.name.clone(), PivotDateGroup::None)),
    }
}

/// Reads `<gc:pivotSettings>` back into the pivot (see [`pt_settings_xml`]).
fn read_settings(e: &El, pt: &mut PivotTable, filters: &mut Vec<PivotFilter>) {
    pt.blank_lines = e.flag("blankLines", false);
    for f in e.kids("field") {
        let Some(index) = f.attr_u32("index").map(|i| i as usize) else { continue };
        // `index` counts rows then columns, the same order the writer walked.
        let field = if index < pt.rows.len() { pt.rows.get_mut(index) } else { pt.columns.get_mut(index.saturating_sub(pt.rows.len())) };
        let Some(field) = field else { continue };
        if let (Some(start), Some(size)) = (f.attr_f64("binStart"), f.attr_f64("binSize"))
            && size > 0.0
            && size.is_finite()
        {
            field.group_bins = Some((start, size));
        }
        if let Some(by) = f.attr("sortBy").filter(|b| !b.trim().is_empty()) {
            field.sort_by = Some(by.to_string());
        }
        field.grouped_items = f
            .kids("group")
            .map(|g| g.kids("item").filter_map(|i| i.attr("v").map(str::to_string)).collect::<Vec<_>>())
            .filter(|g: &Vec<String>| !g.is_empty())
            .collect();
        field.custom_order = f.kids("order").filter_map(|o| o.attr("v").map(str::to_string)).collect();
    }
    for c in e.kids("calculatedItem") {
        let (Some(field), Some(name), Some(formula)) = (c.attr("field"), c.attr("name"), c.attr("formula")) else { continue };
        pt.calculated_items.push(PivotCalcItem { field: field.to_string(), name: name.to_string(), formula: formula.to_string() });
    }
    for v in e.kids("valueFilter") {
        let Some(field) = v.attr("field") else { continue };
        let vf = PivotValueFilter {
            n: v.attr_u32("n").unwrap_or(0),
            top: v.flag("top", false),
            bottom: v.flag("bottom", false),
            above: v.attr_f64("above"),
            below: v.attr_f64("below"),
            between_lo: v.attr_f64("betweenLo"),
            between_hi: v.attr_f64("betweenHi"),
            top_percent: v.flag("topPercent", false),
            by_field: v.attr("byField").map(str::to_string),
        };
        // A value filter shares the field's filter entry with any label selection.
        match filters.iter_mut().find(|f| same(&f.source_col, field)) {
            Some(f) => f.value_filter = Some(vf),
            None => filters.push(PivotFilter { source_col: field.to_string(), selected: None, value_filter: Some(vf) }),
        }
    }
    // Label selections (an empty one is a timeline that matched no date).
    for s in e.kids("selection") {
        let Some(field) = s.attr("field") else { continue };
        let labels: Vec<String> = s.kids("item").filter_map(|i| i.attr("v").map(str::to_string)).collect();
        let selected = Some(labels);
        match filters.iter_mut().find(|f| same(&f.source_col, field)) {
            Some(f) => f.selected = selected,
            None => filters.push(PivotFilter { source_col: field.to_string(), selected, value_filter: None }),
        }
    }
}

fn read_table(cx: &mut Ctx<'_>, x: &El, cache: &CacheIn, id: u32) -> Option<PivotTable> {
    let mut pt = PivotTable { id, ..PivotTable::default() };
    if let Some(n) = x.attr("name").filter(|n| !n.trim().is_empty()) {
        pt.name = n.to_string();
    } else {
        pt.name = format!("PivotTable{id}");
    }
    pt.source = cache.source.clone();
    let Some(loc) = x.child("location").and_then(|l| l.attr("ref")).and_then(|r| RangeRef::parse(&r.replace('$', ""))) else {
        cx.warn(format!("PivotTable \"{}\" has no valid location; kept as plain cells", pt.name));
        return None;
    };
    let pfs: Vec<&El> = x.child("pivotFields").map(|p| p.kids("pivotField").collect()).unwrap_or_default();
    let pf = |i: usize| pfs.get(i).copied();
    let idx_list = |name: &str, attr: &str| -> Vec<i64> {
        x.child(name).map(|e| e.children.iter().filter_map(|f| f.attr_i64(attr)).collect()).unwrap_or_default()
    };
    let row_x = idx_list("rowFields", "x");
    let col_x = idx_list("colFields", "x");
    let pages: Vec<&El> = x.child("pageFields").map(|p| p.kids("pageField").collect()).unwrap_or_default();

    // Layout from the first axis field (pivotField defaults: compact and outline on).
    let first_axis = row_x.iter().chain(col_x.iter()).find(|i| **i >= 0).and_then(|i| pf(*i as usize));
    let (compact, outline) = match first_axis {
        Some(f) => (f.flag("compact", true), f.flag("outline", true)),
        None => (x.flag("compact", true), x.flag("outline", false)),
    };
    pt.layout = if compact {
        PivotLayout::Compact
    } else if outline {
        PivotLayout::Outline
    } else {
        PivotLayout::Tabular
    };
    if let Some(f) = first_axis {
        pt.subtotals = f.flag("defaultSubtotal", true);
        pt.subtotals_top = f.flag("subtotalTop", true);
    }
    pt.grand_totals_rows = x.flag("colGrandTotals", true);
    pt.grand_totals_cols = x.flag("rowGrandTotals", true);
    pt.show_headers = x.flag("showHeaders", true);
    pt.enable_show_details = x.flag("enableDrill", true);
    pt.style = x.child("pivotTableStyleInfo").and_then(|s| s.attr("name")).unwrap_or("").to_string();
    if x.child("calculatedItems").is_some() || x.path(&["filters", "filter"]).is_some() {
        cx.warn(format!("PivotTable \"{}\": calculated items and label/value filters are not supported", pt.name));
    }

    let mut filters: Vec<PivotFilter> = Vec::new();

    // (fields, position of Σ Values (`-2`) among them)
    let mut axis = |cx: &mut Ctx<'_>, list: &[i64], filters: &mut Vec<PivotFilter>| -> (Vec<PivotField>, Option<usize>) {
        let mut v = Vec::new();
        let mut values_at = None;
        for &i in list {
            if i == -2 {
                values_at = Some(v.len());
                continue;
            }
            if i < 0 {
                continue;
            }
            let i = i as usize;
            let Some((name, group)) = field_ref(cx, cache, i) else { continue };
            let labels = cache.fields.get(i).map(|f| f.labels().to_vec()).unwrap_or_default();
            let mut f = PivotField {
                source_col: name.clone(),
                date_group: group,
                sort: PivotSort::None,
                group_bins: None,
                grouped_items: vec![],
                sort_by: None,
                custom_order: vec![],
                collapsed_items: vec![],
            };
            if let Some(e) = pf(i) {
                f.sort = match e.attr("sortType") {
                    Some("ascending") => PivotSort::Asc,
                    Some("descending") => PivotSort::Desc,
                    _ => PivotSort::None,
                };
                let items = items_of(e);
                let lab = |it: &El| it.attr_u32("x").and_then(|k| labels.get(k as usize)).cloned();
                f.collapsed_items = items.iter().filter(|it| !it.flag("sd", true)).filter_map(|it| lab(it)).collect();
                if e.flag("repeatItemLabels", false) {
                    pt.repeat_item_labels = true;
                }
                if e.flag("showAll", false) {
                    pt.show_items_with_no_data = true;
                }
                if items.iter().any(|it| it.flag("h", false)) {
                    let shown: Vec<String> = items.iter().filter(|it| !it.flag("h", false)).filter_map(|it| lab(it)).collect();
                    if !filters.iter().any(|ff| same(&ff.source_col, &name)) {
                        filters.push(PivotFilter { source_col: name.clone(), selected: Some(shown), value_filter: None });
                    }
                }
            }
            v.push(f);
        }
        (v, values_at)
    };
    let mut page_filters = Vec::new();
    for p in &pages {
        let Some(i) = p.attr_i64("fld").filter(|i| *i >= 0).map(|i| i as usize) else { continue };
        let Some((name, _)) = field_ref(cx, cache, i) else { continue };
        let labels = cache.fields.get(i).map(|f| f.labels().to_vec()).unwrap_or_default();
        let selected = if let Some(k) = p.attr_u32("item") {
            labels.get(k as usize).map(|l| vec![l.clone()])
        } else {
            pf(i).and_then(|e| {
                let items = items_of(e);
                items.iter().any(|it| it.flag("h", false)).then(|| {
                    items
                        .iter()
                        .filter(|it| !it.flag("h", false))
                        .filter_map(|it| it.attr_u32("x").and_then(|k| labels.get(k as usize)).cloned())
                        .collect()
                })
            })
        };
        page_filters.push(PivotFilter { source_col: name, selected, value_filter: None });
    }
    let mut axis_filters = Vec::new();
    let (rows, rows_values) = axis(cx, &row_x, &mut axis_filters);
    let (columns, cols_values) = axis(cx, &col_x, &mut axis_filters);
    pt.rows = rows;
    pt.columns = columns;
    // Σ Values: where its `-2` entry is, else `dataOnRows` and `dataPosition`; after the last
    // field by default. In Columns it always comes last, as the column layout puts it there.
    pt.values_on_rows = rows_values.is_some() || (cols_values.is_none() && x.flag("dataOnRows", false));
    pt.values_position =
        if pt.values_on_rows { rows_values.or_else(|| x.attr_u32("dataPosition").map(|p| p as usize)).filter(|p| *p < pt.rows.len()) } else { None };
    filters.extend(page_filters);
    filters.extend(axis_filters);
    // Our own extension: the settings OOXML has no attribute for. Read before the filters move
    // into `pt` so a value filter lands on the existing entry rather than duplicating it.
    if let Some(ext) = x
        .child("extLst")
        .into_iter()
        .flat_map(|l| l.kids("ext"))
        .find(|e| e.attr("uri").is_some_and(|u| same(u, EXT_GRIDCRAFT)))
        .and_then(|e| e.child("pivotSettings"))
    {
        read_settings(ext, &mut pt, &mut filters);
    }
    pt.filters = filters;

    // Values.
    if let Some(df) = x.child("dataFields") {
        for d in df.kids("dataField") {
            let Some(i) = d.attr_u32("fld").map(|i| i as usize) else { continue };
            let Some(f) = cache.fields.get(i) else { continue };
            let func = match d.attr("subtotal") {
                None => PivotFunc::Sum,
                Some(s) => func_from(s).unwrap_or_else(|| {
                    cx.warn(format!("PivotTable function {s} is not supported; using Sum"));
                    PivotFunc::Sum
                }),
            };
            let ext = d
                .child("extLst")
                .into_iter()
                .flat_map(|l| l.kids("ext"))
                .filter_map(|e| e.child("dataField"))
                .find_map(|e| e.attr("pivotShowAs").map(str::to_string));
            let show_as = match (ext.as_deref(), d.attr("showDataAs")) {
                (Some("rankDescending"), _) => PivotShowAs::Rank,
                (Some("rankAscending"), _) => PivotShowAs::RankAscending,
                (Some(other), _) => {
                    cx.warn(format!("PivotTable show-values-as {other} is not supported"));
                    PivotShowAs::Normal
                }
                (None, None | Some("normal")) => PivotShowAs::Normal,
                (None, Some("percentOfTotal")) => PivotShowAs::PercentOfGrandTotal,
                (None, Some("percentOfCol")) => PivotShowAs::PercentOfColumnTotal,
                (None, Some("percentOfRow")) => PivotShowAs::PercentOfRowTotal,
                (None, Some("percent")) => PivotShowAs::PercentOf,
                (None, Some("difference")) => PivotShowAs::DifferenceFrom,
                (None, Some("percentDiff")) => PivotShowAs::PercentDifferenceFrom,
                (None, Some("runTotal")) => PivotShowAs::RunningTotal,
                (None, Some("percentRunningTotal")) => PivotShowAs::PercentRunningTotalIn,
                (None, Some("index")) => PivotShowAs::Index,
                (None, Some("percentOfParentRow" | "normalized")) => PivotShowAs::PercentOfParentRowTotal,
                (None, Some(other)) => {
                    cx.warn(format!("PivotTable show-values-as {other} is not supported"));
                    PivotShowAs::Normal
                }
            };
            let number_format = d
                .attr_u32("numFmtId")
                .filter(|id| *id != 0)
                .and_then(|id| crate::tables::builtin_format(id).map(str::to_string).or_else(|| cx.num_fmts.get(&id).cloned()));
            let name = d.attr("name").map(str::to_string).unwrap_or_else(|| format!("{} of {}", func_caption(func), f.name));
            pt.values.push(PivotValue {
                source_col: f.name.clone(),
                func,
                name,
                show_as,
                base_field: None,
                base_item: None,
                error_display: None,
                empty_display: None,
                number_format,
            });
        }
    }

    // Calculated fields of the cache (each PivotTable on it can use them).
    for f in &cache.fields {
        if let Some(formula) = &f.formula
            && !f.name.trim().is_empty()
            && !pt.calculated_fields.iter().any(|c| same(&c.name, &f.name))
        {
            let formula = formula.trim();
            let formula = if formula.starts_with('=') { formula.to_string() } else { format!("={formula}") };
            pt.calculated_fields.push(PivotCalcField { name: f.name.clone(), formula });
        }
    }
    // Value-field bases from GridCraft's extension (see `pt_settings_xml`). Read here, after
    // `pt.values` exists, since the element indexes into it.
    if let Some(settings) = x
        .child("extLst")
        .into_iter()
        .flat_map(|l| l.kids("ext"))
        .find(|e| e.attr("uri").is_some_and(|u| same(u, EXT_GRIDCRAFT)))
        .and_then(|e| e.child("pivotSettings"))
    {
        for b in settings.kids("valueBase") {
            let Some(i) = b.attr_u32("index").map(|i| i as usize) else { continue };
            let Some(v) = pt.values.get_mut(i) else { continue };
            if let Some(bf) = b.attr("baseField") {
                v.base_field = Some(bf.to_string());
            }
            if let Some(bi) = b.attr("baseItem") {
                v.base_item = Some(bi.to_string());
            }
            if let Some(e) = b.attr("errorDisplay") {
                v.error_display = Some(e.to_string());
            }
            if let Some(e) = b.attr("emptyDisplay") {
                v.empty_display = Some(e.to_string());
            }
        }
    }
    pt.anchor = loc.start;
    let above = if pages.is_empty() { 0 } else { pages.len() as u32 + 1 };
    pt.last_range = Some(RangeRef::new(CellRef::new(loc.start.row.saturating_sub(above), loc.start.col), loc.end));
    Some(pt)
}
