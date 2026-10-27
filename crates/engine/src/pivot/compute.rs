use super::items::{Acc, ByValue, FieldItems, build_tree, collect_siblings, date_flags, is_collapsed, item_label};
use super::output::{AggMap, CellKind, ColKind, ColSpec, Ctx, Grid, Line, LineKind, Output, RowKind, ValSrc, data_cols_exceed_sheet, flatten_cols};
use super::*;

/// Why Show Details turns a cell down when it isn't a value cell of a PivotTable (the front end
/// edits that cell instead).
pub const NOT_A_VALUE_CELL: &str = "select a value cell in a PivotTable";

/// Show Details: the source list and the indices of its records behind the value cell at
/// (`row`, `col`) of the report grid (relative to its top-left), in source order. `None` when
/// that cell holds no value (a label, a header, a blank). An error, for any cell, when the report
/// on the sheet is no longer what the source computes: the cell's records would be another cell's.
pub fn detail_records(wb: &Workbook, pt: &PivotTable, pivot_sheet: usize, row: u32, col: u32) -> Result<Option<(Source, Vec<usize>)>, String> {
    let src = read_source(wb, &pt.source, pivot_sheet)?;
    let mut probe = Probe { at: Some((row as usize, col as usize)), hit: None, paths: Some(Vec::new()), cells: None };
    let out = compute_from(wb, pt, &src, Some(&mut probe))?;
    // Checked first: in a stale report even a cell that computes as a label may show a value.
    check_current(wb, pt, pivot_sheet, &out)?;
    let Some((rp, cp)) = probe.hit else { return Ok(None) };
    let records =
        probe.paths.unwrap_or_default().into_iter().filter(|(_, r, c)| r.starts_with(&rp) && c.starts_with(&cp)).map(|(i, _, _)| i).collect();
    Ok(Some((src, records)))
}

/// The cells of the report grid (relative to its top-left) where [`detail_records`] finds
/// records: its value, subtotal and grand total cells, from one computation of the report. The
/// same errors as [`detail_records`].
pub fn detail_cells(wb: &Workbook, pt: &PivotTable, pivot_sheet: usize) -> Result<HashSet<(u32, u32)>, String> {
    let src = read_source(wb, &pt.source, pivot_sheet)?;
    let mut probe = Probe { at: None, hit: None, paths: None, cells: Some(HashSet::new()) };
    let out = compute_from(wb, pt, &src, Some(&mut probe))?;
    check_current(wb, pt, pivot_sheet, &out)?;
    Ok(probe.cells.unwrap_or_default().into_iter().map(|(r, c)| (r as u32, c as u32)).collect())
}

/// An error when the report on the sheet is not what `out` computes. Excel reads its PivotTable
/// cache; without one, the report written by the last refresh must still be what the source gives.
fn check_current(wb: &Workbook, pt: &PivotTable, pivot_sheet: usize, out: &Output) -> Result<(), String> {
    let sh = wb.sheet(pivot_sheet).ok_or("There's no such sheet.")?;
    let current = pt.last_range.is_some_and(|r| {
        r.height() == out.height().max(1)
            && r.width() == out.width.max(1)
            && out.grid.iter().enumerate().all(|(ri, line)| {
                line.iter().enumerate().all(|(ci, oc)| {
                    sh.value_ref(CellRef::new(r.start.row + ri as u32, r.start.col + ci as u32)).unwrap_or(&Value::Empty) == &oc.value
                })
            })
    });
    if !current {
        return Err("The PivotTable report is out of date. Refresh it first.".into());
    }
    Ok(())
}

/// What [`detail_records`] and [`detail_cells`] ask of a report computation: the row and column
/// item paths of the value cell at `at`, the paths of every record the filters let through (when
/// `paths` is set), and every value cell (when `cells` is set).
pub(super) struct Probe {
    at: Option<(usize, usize)>,
    hit: Option<(Vec<u32>, Vec<u32>)>,
    paths: Option<Vec<(usize, Vec<u32>, Vec<u32>)>>,
    cells: Option<HashSet<(usize, usize)>>,
}

pub(super) fn compute_from(wb: &Workbook, pt: &PivotTable, src: &Source, mut probe: Option<&mut Probe>) -> Result<Output, String> {
    let fmt_of = |c: usize| src.formats.get(c).cloned().unwrap_or_else(|| "General".into());
    let rows: Vec<(usize, &PivotField)> = pt.rows.iter().filter_map(|f| src.col(&f.source_col).map(|c| (c, f))).collect();
    let cols: Vec<(usize, &PivotField)> = pt.columns.iter().filter_map(|f| src.col(&f.source_col).map(|c| (c, f))).collect();
    // Value fields: a source column, or a calculated field evaluated on sums (see `pivot_calc`).
    let calc = CalcSet::new(pt, &src.headers);
    let vals: Vec<(ValSrc, &PivotValue)> = pt
        .values
        .iter()
        .filter_map(|v| match src.col(&v.source_col) {
            Some(c) => Some((ValSrc::Col(c), v)),
            None => calc.index(&v.source_col).map(|i| (ValSrc::Calc(i), v)),
        })
        .collect();
    // Source columns summed for the calculated fields, accumulated after the value fields.
    let mut extra: Vec<usize> = Vec::new();
    for (vs, _) in &vals {
        if let ValSrc::Calc(i) = vs {
            for name in calc.source_refs(*i) {
                if let Some(c) = src.col(&name)
                    && !extra.contains(&c)
                {
                    extra.push(c);
                }
            }
        }
    }
    if rows.len() + cols.len() > MAX_AXIS_FIELDS {
        return Err(format!("A PivotTable can have at most {MAX_AXIS_FIELDS} row and column fields."));
    }
    if vals.len() > MAX_VALUE_FIELDS {
        return Err(format!("A PivotTable can have at most {MAX_VALUE_FIELDS} value fields."));
    }
    let axis_group = |name: &str| -> Option<PivotDateGroup> {
        pt.rows.iter().chain(pt.columns.iter()).find(|f| f.source_col.eq_ignore_ascii_case(name)).map(|f| f.date_group)
    };
    // (column, filter, grouping used for labels, shown as a report filter)
    let filters: Vec<(usize, &PivotFilter, PivotDateGroup, bool)> = pt
        .filters
        .iter()
        .filter_map(|f| {
            let c = src.col(&f.source_col)?;
            let g = axis_group(&f.source_col);
            Some((c, f, g.unwrap_or_default(), g.is_none()))
        })
        .collect();
    let shown_filters: Vec<&(usize, &PivotFilter, PivotDateGroup, bool)> = filters.iter().filter(|f| f.3).collect();

    let k = vals.len();
    let empty = rows.is_empty() && cols.is_empty() && vals.is_empty();
    // Σ Values in Rows (it exists with two or more value fields): its depth among the row fields
    // found in the source.
    let vpos: Option<usize> = (pt.values_on_rows && k > 1).then(|| {
        let p = pt.values_position.unwrap_or(pt.rows.len());
        pt.rows.iter().take(p).filter(|f| src.col(&f.source_col).is_some()).count()
    });
    let on_rows = vpos.is_some();
    let row_levels = rows.len() + usize::from(on_rows);

    // ---- aggregate
    let mut row_items: Vec<FieldItems> = rows.iter().map(|_| FieldItems::default()).collect();
    let mut col_items: Vec<FieldItems> = cols.iter().map(|_| FieldItems::default()).collect();
    let mut aggs: AggMap = HashMap::new();
    let mut row_set: HashSet<Vec<u32>> = HashSet::new();
    let mut col_set: HashSet<Vec<u32>> = HashSet::new();
    let mut col_leaves: HashSet<Vec<u32>> = HashSet::new();
    let filter_fmts: Vec<String> = filters.iter().map(|f| fmt_of(f.0)).collect();
    // Item labels use the field's number format, else the source column's.
    let field_fmt = |c: usize, f: &PivotField| f.number_format.clone().unwrap_or_else(|| fmt_of(c));
    let row_fmts: Vec<String> = rows.iter().map(|(c, f)| field_fmt(*c, f)).collect();
    let col_fmts: Vec<String> = cols.iter().map(|(c, f)| field_fmt(*c, f)).collect();
    let row_dates: Vec<Vec<bool>> = rows.iter().map(|(c, f)| date_flags(wb, src, *c, f.date_group)).collect();
    let col_dates: Vec<Vec<bool>> = cols.iter().map(|(c, f)| date_flags(wb, src, *c, f.date_group)).collect();
    let is_date = |dates: &[bool], ri: usize| dates.get(ri).copied().unwrap_or(false);
    'rec: for (ri, rec) in src.rows.iter().enumerate() {
        for ((c, f, g, _), fmt) in filters.iter().zip(filter_fmts.iter()) {
            if let Some(sel) = &f.selected {
                let label = item_label(wb, rec.get(*c).unwrap_or(&Value::Empty), *g, fmt);
                if !sel.iter().any(|s| s.eq_ignore_ascii_case(&label)) {
                    continue 'rec;
                }
            }
        }
        if empty {
            continue;
        }
        let mut rp = Vec::with_capacity(rows.len());
        for ((((c, f), fi), fmt), dates) in rows.iter().zip(row_items.iter_mut()).zip(row_fmts.iter()).zip(row_dates.iter()) {
            rp.push(fi.intern(wb, rec.get(*c).unwrap_or(&Value::Empty), f.date_group, fmt, is_date(dates, ri), ri));
        }
        let mut cp = Vec::with_capacity(cols.len());
        for ((((c, f), fi), fmt), dates) in cols.iter().zip(col_items.iter_mut()).zip(col_fmts.iter()).zip(col_dates.iter()) {
            cp.push(fi.intern(wb, rec.get(*c).unwrap_or(&Value::Empty), f.date_group, fmt, is_date(dates, ri), ri));
        }
        for a in 0..=rp.len() {
            for b in 0..=cp.len() {
                let e = aggs.entry((rp[..a].to_vec(), cp[..b].to_vec())).or_insert_with(|| vec![Acc::default(); k + extra.len()]);
                for (acc, (vs, _)) in e.iter_mut().zip(vals.iter()) {
                    if let ValSrc::Col(c) = vs {
                        acc.add(rec.get(*c).unwrap_or(&Value::Empty));
                    }
                }
                for (acc, c) in e.iter_mut().skip(k).zip(extra.iter()) {
                    acc.add(rec.get(*c).unwrap_or(&Value::Empty));
                }
            }
        }
        // The column leaf this record lands in: its path cut after the first collapsed item (as in `flatten_cols`).
        let shown = cols
            .iter()
            .zip(col_items.iter())
            .zip(cp.iter())
            .position(|(((_, f), fi), &it)| !f.collapsed_items.is_empty() && is_collapsed(f, &fi.label(it)))
            .map_or(cp.len(), |d| d + 1);
        if !col_leaves.contains(&cp[..shown]) {
            col_leaves.insert(cp[..shown].to_vec());
        }
        if let Some(paths) = probe.as_deref_mut().and_then(|pr| pr.paths.as_mut()) {
            paths.push((ri, rp.clone(), cp.clone()));
        }
        row_set.insert(rp);
        col_set.insert(cp);
        // Refuse a report that cannot fit before the accumulators for more column groups are allocated.
        if !cols.is_empty() && data_cols_exceed_sheet(col_leaves.len(), pt.grand_totals_rows, k) {
            return Err("The PivotTable report will not fit on the sheet.".into());
        }
        if aggs.len() > 5_000_000 {
            return Err("The PivotTable report is too large.".into());
        }
    }

    for (fi, (_, f)) in row_items.iter_mut().zip(rows.iter()) {
        fi.apply(f);
    }
    for (fi, (_, f)) in col_items.iter_mut().zip(cols.iter()) {
        fi.apply(f);
    }

    let get = |rp: &[u32], cp: &[u32], vi: usize| -> Option<Value> {
        let (vs, v) = vals.get(vi)?;
        let accs = aggs.get(&(rp.to_vec(), cp.to_vec()))?;
        match vs {
            ValSrc::Col(_) => accs.get(vi).map(|a| a.result(v.func)),
            ValSrc::Calc(i) => {
                let sum = |name: &str| -> Value {
                    src.col(name)
                        .and_then(|c| extra.iter().position(|x| *x == c))
                        .and_then(|j| accs.get(k + j))
                        .map(|a| a.result(PivotFunc::Sum))
                        .unwrap_or(Value::Number(0.0))
                };
                Some(calc.value(wb, *i, &sum))
            }
        }
    };

    // ---- axes
    let row_fields: Vec<&PivotField> = rows.iter().map(|(_, f)| *f).collect();
    let col_fields: Vec<&PivotField> = cols.iter().map(|(_, f)| *f).collect();
    let row_paths: Vec<&[u32]> = row_set.iter().map(Vec::as_slice).collect();
    let col_paths: Vec<&[u32]> = col_set.iter().map(Vec::as_slice).collect();
    // The other-axis path a sort scope names: its items of the other axis's fields, outer first,
    // up to the first field it doesn't name. A scope naming an item that isn't there sorts by the
    // grand total, as the writer then drops it.
    let scope_path = |f: &PivotField, others: &[(usize, &PivotField)], other_items: &[FieldItems]| -> Vec<u32> {
        let mut path = Vec::new();
        for ((_, o), oi) in others.iter().zip(other_items) {
            let Some(e) = f.sort_scope.iter().find(|e| e.is_of(o)) else { break };
            match oi.find(&e.item) {
                Some(i) => path.push(i),
                None => return Vec::new(),
            }
        }
        path
    };
    let sorting = |f: &PivotField, others: &[(usize, &PivotField)], other_items: &[FieldItems]| {
        let vi = f.sort_by.as_deref().and_then(|n| vals.iter().position(|(_, v)| v.name.eq_ignore_ascii_case(n)))?;
        Some((vi, scope_path(f, others, other_items)))
    };
    let row_value = |p: &[u32], other: &[u32], vi: usize| get(p, other, vi).as_ref().and_then(Value::as_f64);
    let col_value = |p: &[u32], other: &[u32], vi: usize| get(other, p, vi).as_ref().and_then(Value::as_f64);
    let row_by = ByValue { fields: row_fields.iter().map(|f| sorting(f, &cols, &col_items)).collect(), value: &row_value };
    let col_by = ByValue { fields: col_fields.iter().map(|f| sorting(f, &rows, &row_items)).collect(), value: &col_value };
    let row_tree = build_tree(&row_paths, 0, &row_fields, &row_items, &row_by);
    let col_tree = build_tree(&col_paths, 0, &col_fields, &col_items, &col_by);
    let mut siblings: HashMap<Vec<u32>, Vec<u32>> = HashMap::new();
    collect_siblings(&row_tree, &mut Vec::new(), &mut siblings);
    let level0: Vec<u32> = row_tree.iter().map(|n| n.item).collect();

    let val_names: Vec<String> = vals.iter().map(|(_, v)| v.name.clone()).collect();
    let grand_caption = pt.grand_total_caption.as_deref().unwrap_or("Grand Total");
    let ctx = Ctx { pt, rows: row_fields.clone(), row_items, row_fmts: row_fmts.clone(), vpos, val_names: val_names.clone() };
    let mut lines: Vec<Line> = Vec::new();
    if row_levels == 0 {
        lines.push(Line { kind: LineKind::Item(false), leaf: true, path: vec![], values: true, labels: vec![], vi: None });
    } else {
        if pt.layout == PivotLayout::Tabular {
            ctx.flatten_tabular(&row_tree, 0, None, &mut Vec::new(), &[], &mut Vec::new(), &mut lines);
        } else {
            ctx.flatten(&row_tree, 0, None, &mut Vec::new(), &mut Vec::new(), &mut lines);
        }
        // With only Σ Values in Rows its items are the totals already.
        if pt.grand_totals_cols && !rows.is_empty() {
            let grand = |text: String, vi: Option<usize>| Line {
                kind: LineKind::Grand,
                leaf: false,
                path: vec![],
                values: true,
                labels: vec![(0, Value::text(text), 0, None)],
                vi,
            };
            if on_rows {
                lines.extend(val_names.iter().enumerate().map(|(vi, name)| grand(format!("Total {name}"), Some(vi))));
            } else {
                lines.push(grand(grand_caption.into(), None));
            }
        }
    }
    let mut specs: Vec<ColSpec> = Vec::new();
    if cols.is_empty() {
        specs.push(ColSpec { kind: ColKind::Leaf, path: vec![] });
    } else {
        flatten_cols(&col_tree, 0, &col_fields, &col_items, pt, &mut Vec::new(), &mut specs);
        if pt.grand_totals_rows {
            specs.push(ColSpec { kind: ColKind::Grand, path: vec![] });
        }
    }
    let mut data_cols: Vec<(usize, Option<usize>)> = Vec::new();
    for (si, _) in specs.iter().enumerate() {
        if on_rows {
            // Each line names its value field; one data column per column item.
            data_cols.push((si, None));
        } else if k == 0 {
            if !cols.is_empty() {
                data_cols.push((si, None));
            }
        } else {
            for vi in 0..k {
                data_cols.push((si, Some(vi)));
            }
        }
        if data_cols.len() > MAX_COLS as usize {
            return Err("The PivotTable report will not fit on the sheet.".into());
        }
    }

    let compact = pt.layout == PivotLayout::Compact;
    let l = if row_levels == 0 && cols.is_empty() {
        0
    } else if compact {
        1
    } else {
        row_levels.max(1)
    };
    let mut width = l + data_cols.len();
    if !shown_filters.is_empty() {
        width = width.max(2);
    }
    if empty {
        width = width.max(1);
    }
    let width = width.max(1);
    let mut g = Grid { rows: vec![], kinds: vec![], width };

    // ---- report filters
    for (c, f, _, _) in &shown_filters {
        let r = g.push_row(RowKind::Filter)?;
        let name = src.headers.get(*c).cloned().unwrap_or_default();
        let shown = match &f.selected {
            None => "(All)".to_string(),
            Some(v) if v.len() == 1 => v.first().cloned().unwrap_or_default(),
            Some(_) => "(Multiple Items)".to_string(),
        };
        g.set(r, 0, Value::text(name), CellKind::Filter, 0, None);
        g.set(r, 1, Value::text(shown), CellKind::Filter, 0, None);
    }
    if !shown_filters.is_empty() {
        g.push_row(RowKind::Blank)?;
    }
    let body_offset = g.rows.len() as u32;

    if empty {
        let r = g.push_row(RowKind::Placeholder)?;
        g.set(r, 0, Value::text(format!("{} — choose fields", pt.name)), CellKind::Placeholder, 0, None);
        return Ok(Output {
            width: g.width as u32,
            grid: g.rows,
            row_kinds: g.kinds,
            body_offset,
            chart_rows: vec![],
            chart_cols: vec![],
            label_col: None,
        });
    }

    let val_name = |vi: usize| vals.get(vi).map(|(_, v)| v.name.clone()).unwrap_or_default();
    let headers_on = pt.show_headers;
    let row_header = |g: &mut Grid, r: usize| {
        if row_levels == 0 || !headers_on {
            return;
        }
        if compact {
            g.set(r, 0, Value::text(pt.row_header_caption.as_deref().unwrap_or("Row Labels")), CellKind::Header, 0, None);
        } else {
            // One column per row field, Σ Values (its caption, "Values" by default) at its place.
            let mut names: Vec<&str> = row_fields.iter().map(|f| f.header()).collect();
            if let Some(p) = vpos {
                names.insert(p.min(names.len()), pt.data_caption.as_deref().unwrap_or("Values"));
            }
            for (i, name) in names.into_iter().enumerate() {
                g.set(r, i, Value::text(name), CellKind::Header, 0, None);
            }
        }
    };

    // ---- header rows
    if cols.is_empty() {
        // Classic layout with one value field: its caption sits above the row headers and the
        // value column is headed "Total" (an empty column area).
        let classic = pt.classic_layout && !compact && k == 1 && headers_on;
        if classic {
            let r = g.push_row(RowKind::Header)?;
            g.set(r, 0, Value::text(val_name(0)), CellKind::Header, 0, None);
        }
        if !compact && k > 1 && !on_rows && pt.show_values_row && headers_on {
            let r = g.push_row(RowKind::Header)?;
            g.set(r, l, Value::text(pt.data_caption.as_deref().unwrap_or("Values")), CellKind::Header, 0, None);
        }
        let r = g.push_row(RowKind::Header)?;
        row_header(&mut g, r);
        for (j, (_, vi)) in data_cols.iter().enumerate() {
            match vi {
                Some(vi) => {
                    let caption = if classic { "Total".to_string() } else { val_name(*vi) };
                    g.set(r, l + j, Value::text(caption), CellKind::Header, 0, None);
                }
                // Values on rows, no column field: the one data column is the grand total.
                None if on_rows => g.set(r, l + j, Value::text("Total"), CellKind::Header, 0, None),
                None => {}
            }
        }
    } else {
        let cap = g.push_row(RowKind::Header)?;
        if l > 0 && k == 1 {
            g.set(cap, 0, Value::text(val_name(0)), CellKind::Header, 0, None);
        }
        if headers_on {
            let caption = if compact {
                pt.col_header_caption.as_deref().unwrap_or("Column Labels").to_string()
            } else {
                col_fields.first().map(|f| f.header().to_string()).unwrap_or_default()
            };
            g.set(cap, l, Value::text(caption), CellKind::Header, 0, None);
        }
        let levels = cols.len() + usize::from(k > 1 && !on_rows);
        let first_level = g.rows.len();
        for _ in 0..levels {
            g.push_row(RowKind::Header)?;
        }
        let mut prev: Option<&ColSpec> = None;
        for (j, (si, vi)) in data_cols.iter().enumerate() {
            let Some(spec) = specs.get(*si) else { continue };
            let c = l + j;
            match spec.kind {
                ColKind::Leaf => {
                    for (lv, &it) in spec.path.iter().enumerate() {
                        let same = prev.is_some_and(|p| p.kind != ColKind::Grand && p.path.get(..=lv) == spec.path.get(..=lv));
                        if !same {
                            let Some(item) = col_items.get(lv).and_then(|fi| fi.get(it)) else { continue };
                            let fmt = if item.formatted && item.caption.is_none() { col_fmts.get(lv).cloned() } else { None };
                            g.set(first_level + lv, c, item.shown_value(), CellKind::ColLabel, 0, fmt);
                        }
                    }
                    if k > 1
                        && let Some(vi) = vi
                    {
                        g.set(first_level + cols.len(), c, Value::text(val_name(*vi)), CellKind::ColLabel, 0, None);
                    }
                }
                ColKind::Subtotal => {
                    let lv = spec.path.len().saturating_sub(1);
                    let label = spec.path.last().and_then(|it| col_items.get(lv).map(|fi| fi.shown_label(*it))).unwrap_or_default();
                    let text = match vi {
                        Some(vi) if k > 1 => format!("{label} {}", val_name(*vi)),
                        _ => format!("{label} Total"),
                    };
                    g.set(first_level + lv, c, Value::text(text), CellKind::ColLabel, 0, None);
                }
                ColKind::Grand => {
                    let text = match vi {
                        Some(vi) if k > 1 => format!("Total {}", val_name(*vi)),
                        _ => grand_caption.to_string(),
                    };
                    g.set(first_level, c, Value::text(text), CellKind::ColLabel, 0, None);
                }
            }
            prev = Some(spec);
        }
        let last = g.rows.len() - 1;
        row_header(&mut g, last);
    }

    // ---- body
    let num = |v: &Option<Value>| v.as_ref().and_then(Value::as_f64);
    let ratio = |a: Value, b: Option<Value>| -> Value {
        match (a, b) {
            (Value::Error(e), _) => Value::Error(e),
            (_, Some(Value::Error(e))) => Value::Error(e),
            (Value::Number(x), Some(Value::Number(y))) => {
                if y == 0.0 {
                    Value::Error(CellError::Div0)
                } else {
                    Value::number(x / y)
                }
            }
            (v, _) => v,
        }
    };
    let data_fmt: Vec<Option<String>> = vals
        .iter()
        .map(|(vs, v)| match v.show_as {
            PivotShowAs::PercentOfGrandTotal | PivotShowAs::PercentOfColumnTotal | PivotShowAs::PercentOfRowTotal => {
                Some(v.number_format.clone().unwrap_or_else(|| "0.00%".into()))
            }
            PivotShowAs::Rank => v.number_format.clone(),
            _ => v.number_format.clone().or_else(|| match v.func {
                PivotFunc::Count | PivotFunc::CountNumbers => None,
                _ => {
                    let f = match vs {
                        ValSrc::Col(c) => fmt_of(*c),
                        ValSrc::Calc(_) => "General".to_string(),
                    };
                    (f != "General").then_some(f)
                }
            }),
        })
        .collect();
    let mut chart_rows: Vec<u32> = Vec::new();
    let chart_cols: Vec<(u32, String)> = data_cols
        .iter()
        .enumerate()
        .filter_map(|(j, (si, vi))| {
            let spec = specs.get(*si)?;
            if spec.kind != ColKind::Leaf {
                return None;
            }
            let mut parts: Vec<String> =
                spec.path.iter().enumerate().map(|(lv, it)| col_items.get(lv).map(|fi| fi.shown_label(*it)).unwrap_or_default()).collect();
            if let Some(vi) = vi
                && (k > 1 || parts.is_empty())
            {
                parts.push(val_name(*vi));
            } else if parts.is_empty() {
                parts.push("Total".into());
            }
            Some(((l + j) as u32, parts.join(" - ")))
        })
        .filter(|_| k > 0)
        .collect();
    let label_col = if row_levels == 0 {
        None
    } else if compact {
        Some(0)
    } else {
        Some(row_levels.saturating_sub(1) as u32)
    };
    for line in &lines {
        let rk = match line.kind {
            LineKind::Item(b) => RowKind::Item(b),
            LineKind::Subtotal => RowKind::Subtotal,
            LineKind::Grand => RowKind::Grand,
        };
        let ck = match line.kind {
            LineKind::Item(_) => (CellKind::RowLabel, CellKind::Data),
            LineKind::Subtotal => (CellKind::Subtotal, CellKind::Subtotal),
            LineKind::Grand => (CellKind::GrandTotal, CellKind::GrandTotal),
        };
        let r = g.push_row(rk)?;
        if line.leaf && k > 0 {
            chart_rows.push(r as u32);
        }
        for (c, v, indent, fmt) in &line.labels {
            g.set(r, *c, v.clone(), ck.0, *indent, fmt.clone());
        }
        if !line.values {
            continue;
        }
        let rp = line.path.as_slice();
        for (j, (si, col_vi)) in data_cols.iter().enumerate() {
            let line_vi = col_vi.or(line.vi);
            let (Some(spec), Some(vi)) = (specs.get(*si), &line_vi) else { continue };
            let Some((_, vdef)) = vals.get(*vi) else { continue };
            let cp = spec.path.as_slice();
            let Some(raw) = get(rp, cp, *vi) else {
                // No records: blank, or the "For empty cells show" caption (a number when it
                // reads as one, as Excel stores `missingCaption="0"`).
                if let Some(c) = &pt.empty_caption {
                    let v = c.trim().parse::<f64>().ok().filter(|x| x.is_finite()).map(Value::number).unwrap_or_else(|| Value::text(c.as_str()));
                    let fmt = if matches!(v, Value::Number(_)) { data_fmt.get(*vi).cloned().flatten() } else { None };
                    g.set(r, l + j, v, ck.1, 0, fmt);
                }
                continue;
            };
            let shown = match vdef.show_as {
                PivotShowAs::Normal => raw,
                PivotShowAs::PercentOfGrandTotal => ratio(raw, get(&[], &[], *vi)),
                PivotShowAs::PercentOfColumnTotal => ratio(raw, get(&[], cp, *vi)),
                PivotShowAs::PercentOfRowTotal => ratio(raw, get(rp, &[], *vi)),
                PivotShowAs::RunningTotal => match (rp.first(), &raw) {
                    (Some(first), Value::Number(_)) => {
                        let mut acc = 0.0;
                        for it in &level0 {
                            let mut p = rp.to_vec();
                            if let Some(x) = p.first_mut() {
                                *x = *it;
                            }
                            acc += num(&get(&p, cp, *vi)).unwrap_or(0.0);
                            if it == first {
                                break;
                            }
                        }
                        Value::number(acc)
                    }
                    _ => raw,
                },
                PivotShowAs::Rank => match (rp.split_last(), raw.as_f64()) {
                    (Some((_, parent)), Some(me)) => {
                        let sibs = siblings.get(parent).cloned().unwrap_or_default();
                        let mut rank = 1u64;
                        for sib in sibs {
                            let mut p = parent.to_vec();
                            p.push(sib);
                            if num(&get(&p, cp, *vi)).is_some_and(|x| x > me) {
                                rank += 1;
                            }
                        }
                        Value::number(rank as f64)
                    }
                    (None, _) => Value::Empty,
                    _ => raw,
                },
            };
            if let Some(pr) = probe.as_deref_mut() {
                if pr.at == Some((r, l + j)) {
                    pr.hit = Some((rp.to_vec(), cp.to_vec()));
                }
                if let Some(cells) = pr.cells.as_mut() {
                    cells.insert((r, l + j));
                }
            }
            g.set(r, l + j, shown, ck.1, 0, data_fmt.get(*vi).cloned().flatten());
        }
    }
    Ok(Output { width: g.width as u32, grid: g.rows, row_kinds: g.kinds, body_offset, chart_rows, chart_cols, label_col })
}
