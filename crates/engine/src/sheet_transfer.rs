//! Checked, two-phase transfer of a self-contained worksheet between workbooks.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use gridcraft_core::MAX_COLS;
use gridcraft_formula::{Expr, SheetSel};
use gridcraft_model::{CfRule, CfValueKind, DefinedName, Sheet, StyleId, TotalsFn, Visibility, Workbook};

use crate::{EngineError, Result};

pub(crate) fn run(s: &mut crate::Session, target: &serde_json::Value, sheet: usize, before: usize, copy: bool) -> Result<serde_json::Value> {
    let source_index = s.active_index();
    let source = s.doc()?.wb.clone();
    let new = target.as_str() == Some("new");
    let target_index = if new {
        None
    } else {
        let uid = target.as_u64().ok_or_else(|| refuse("choose an open workbook or 'new'."))?;
        Some(s.documents().iter().position(|d| d.uid == uid).ok_or_else(|| refuse("the destination workbook is no longer open."))?)
    };
    let mut blank = Workbook::new();
    blank.date_system = source.date_system;
    blank.theme = source.theme.clone();
    blank.styles = gridcraft_model::StyleTable::with_default(source.styles.get(StyleId::DEFAULT).clone());
    let mut destination = if let Some(index) = target_index {
        s.documents().get(index).ok_or(crate::EngineError::NoDocument)?.wb.as_ref().clone()
    } else {
        let mut empty = blank.clone();
        empty.sheets.clear();
        empty
    };
    if new && source.sheet(sheet).is_some_and(|sh| sh.visibility != Visibility::Visible) {
        return Err(refuse("a new workbook needs a visible worksheet."));
    }
    // New workbooks have no insertion list; stale positions are not reused.
    let before = if new { 0 } else { before };
    let source_doc = s.doc()?;
    let source_path = source_doc.path.as_deref();
    let source_base = crate::cmd::workbook_links::base_dir(source_doc);
    let destination_path = target_index.and_then(|i| s.documents().get(i)).and_then(|d| d.path.as_deref());
    let paths = TransferPaths { source: source_path, destination: destination_path, base: source_base.as_deref() };
    let (mut remaining, output, at) = prepare(&source, &destination, sheet, before, copy, paths)?;
    destination = output;
    let mut destination_calc = gridcraft_calc::Calc::new();
    destination_calc.recalc_all(&mut destination);
    let remaining_calc = if copy {
        None
    } else {
        let mut calc = gridcraft_calc::Calc::new();
        calc.recalc_all(&mut remaining);
        Some(calc)
    };
    // All validation and recalculation has succeeded. Publish both results together.
    let index = if let Some(index) = target_index {
        index
    } else {
        let n = (1..).find(|n| !s.documents().iter().any(|d| d.title == format!("Book{n}") && d.path.is_none())).unwrap_or(1);
        s.add_document(crate::DocState::new(blank, None, format!("Book{n}")))
    };
    if let Some(calc) = remaining_calc
        && let Some(d) = s.docs.get_mut(source_index)
    {
        d.wb = Arc::new(remaining);
        d.calc = calc;
        d.selection = crate::Selection::at(d.wb.active().map(|sh| sh.view_active).unwrap_or_default());
        if sheet < d.sheet_selections.len() {
            d.sheet_selections.remove(sheet);
        }
    }
    let d = s.docs.get_mut(index).ok_or(crate::EngineError::NoDocument)?;
    d.wb = Arc::new(destination);
    d.calc = destination_calc;
    d.sheet_selections.clear();
    d.selection = crate::Selection::at(d.wb.active().map(|sh| sh.view_active).unwrap_or_default());
    let uid = d.uid;
    s.set_active(index);
    if !copy {
        s.clipboard = None;
    }
    Ok(serde_json::json!({"workbook": uid, "sheet": at}))
}

fn refuse(reason: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("Cannot transfer this worksheet: {reason}"))
}

/// Neither input is mutated. Recalculation and publication happen only after validation.
fn prepare(
    source: &Workbook,
    destination: &Workbook,
    sheet: usize,
    before: usize,
    copy: bool,
    paths: TransferPaths<'_>,
) -> Result<(Workbook, Workbook, usize)> {
    let original = source.sheet(sheet).ok_or_else(|| refuse("the source sheet no longer exists."))?;
    if source.protected_structure || destination.protected_structure {
        return Err(refuse("a workbook's structure is protected."));
    }
    if !copy && original.visibility == Visibility::Visible && source.sheets.iter().filter(|s| s.visibility == Visibility::Visible).count() <= 1 {
        return Err(refuse("the source workbook must keep at least one visible worksheet. Use Create a copy instead."));
    }
    if source.date_system != destination.date_system {
        return Err(refuse("the workbooks use different date systems."));
    }
    if source.theme.colors != destination.theme.colors
        || source.theme.major_font != destination.theme.major_font
        || source.theme.minor_font != destination.theme.minor_font
    {
        return Err(refuse("the workbooks use different themes; theme conversion is not supported yet."));
    }
    if source.vba_project.is_some() || original.preserved.as_ref().is_some_and(|p| !p.is_empty()) {
        return Err(refuse("VBA or preserved package content cannot yet be transferred safely."));
    }
    if !original.pivots.is_empty() || !original.slicers.is_empty() || original.charts.iter().any(|c| c.pivot.is_some()) {
        return Err(refuse("PivotTables, PivotCharts and slicers cannot yet be transferred safely."));
    }
    if before > destination.sheets.len() {
        return Err(refuse("the destination sheet position no longer exists."));
    }
    let name = unique_name(destination, &original.name)?;
    let mut output = destination.clone();
    let mut moved = original.clone();
    moved.name = name.clone();
    let table_names: HashSet<_> = original.tables.iter().map(|t| t.name.to_ascii_lowercase()).collect();
    for table in &moved.tables {
        if table.connection_id.is_some() || table.table_type.is_some() || table.columns.iter().any(|c| c.xml.is_some()) {
            return Err(refuse("a table is bound to workbook-level data that cannot be transferred yet."));
        }
        if output.sheets.iter().flat_map(|s| &s.tables).any(|t| t.name.eq_ignore_ascii_case(&table.name))
            || output.names.iter().any(|n| n.name.eq_ignore_ascii_case(&table.name))
        {
            return Err(refuse(format!("the destination already has a table or name called '{}'.", table.name)));
        }
    }
    let mut needed: BTreeSet<usize> = source.names.iter().enumerate().filter(|(_, n)| n.scope == Some(sheet)).map(|(i, _)| i).collect();
    let mut transfer =
        FormulaTransfer { source, sheet, name: &name, tables: &table_names, paths, imported: HashMap::new(), snapshot: None, allow_links: true };
    map_cell_formulas(&mut moved, &mut |text| transfer.rewrite(text, &mut output, &mut needed))?;
    transfer.allow_links = false;
    map_noncell_formulas(&mut moved, &mut |text| transfer.rewrite(text, &mut output, &mut needed))?;
    transfer.allow_links = true;
    // Names can refer to other names. A set bounds traversal even for cyclic definitions.
    let mut done = BTreeSet::new();
    let mut imported = Vec::new();
    while let Some(index) = needed.iter().find(|i| !done.contains(*i)).copied() {
        done.insert(index);
        let Some(old) = source.names.get(index) else { return Err(refuse("a defined name is unavailable.")) };
        let formula = transfer.rewrite(&old.formula, &mut output, &mut needed)?;
        if (old.scope.is_none() && output.names.iter().any(|n| n.scope.is_none() && n.name.eq_ignore_ascii_case(&old.name)))
            || output.sheets.iter().flat_map(|s| &s.tables).any(|t| t.name.eq_ignore_ascii_case(&old.name))
        {
            return Err(refuse(format!("the destination already defines '{}'; resolve the name conflict first.", old.name)));
        }
        imported.push(DefinedName { formula, scope: old.scope.map(|_| before), ..old.clone() });
    }
    remap_styles(source, &mut output, &mut moved);
    // Tables have workbook-wide ids, whereas drawing ids are local to their sheet.
    let mut id = output.sheets.iter().flat_map(|s| &s.tables).map(|t| t.id).max().unwrap_or(0);
    for table in &mut moved.tables {
        id = id.checked_add(1).ok_or_else(|| refuse("there are too many table ids."))?;
        table.id = id;
    }
    for n in &mut output.names {
        if let Some(scope) = &mut n.scope
            && *scope >= before
        {
            *scope += 1;
        }
    }
    output.names.extend(imported);
    let visible = moved.visibility == Visibility::Visible;
    output.sheets.insert(before, Arc::new(moved));
    if let Some(n) = transfer.snapshot {
        cache_snapshot(&mut output, n, source)?;
    }
    output.active_sheet = if visible { before } else { destination.active_sheet + usize::from(destination.active_sheet >= before) };
    check_object_names(&output, before)?;
    let mut remaining = source.clone();
    if !copy {
        rewrite_inbound(source, &mut remaining, &output, sheet, &name, &table_names, paths)?;
        remaining.sheets.remove(sheet);
        remaining.names.retain(|n| n.scope != Some(sheet));
        for n in &mut remaining.names {
            if let Some(scope) = &mut n.scope
                && *scope > sheet
            {
                *scope -= 1;
            }
        }
        for i in 0..remaining.sheets.len() {
            check_object_names(&remaining, i)?;
        }
        remaining.active_sheet =
            remaining.active_sheet.saturating_sub(usize::from(remaining.active_sheet >= sheet)).min(remaining.sheets.len().saturating_sub(1));
    }
    Ok((remaining, output, before))
}

fn unique_name(destination: &Workbook, base: &str) -> Result<String> {
    if destination.sheet_index(base).is_none() {
        return Ok(base.to_string());
    }
    for n in 2..=destination.sheets.len().saturating_add(2) {
        let suffix = format!(" ({n})");
        let keep = 31usize.saturating_sub(suffix.chars().count());
        let name = format!("{}{suffix}", base.chars().take(keep).collect::<String>().trim_end());
        if destination.sheet_index(&name).is_none() {
            return Ok(name);
        }
    }
    Err(refuse("no available sheet name."))
}

#[derive(Clone, Copy)]
struct TransferPaths<'a> {
    source: Option<&'a str>,
    destination: Option<&'a str>,
    base: Option<&'a std::path::Path>,
}

struct FormulaTransfer<'a> {
    source: &'a Workbook,
    sheet: usize,
    name: &'a str,
    tables: &'a HashSet<String>,
    paths: TransferPaths<'a>,
    imported: HashMap<usize, usize>,
    snapshot: Option<usize>,
    allow_links: bool,
}

fn saved_path(path: Option<&str>) -> Result<String> {
    let path = path
        .filter(|p| std::path::Path::new(p).is_absolute())
        .ok_or_else(|| refuse("save the linked workbook to an absolute file path before transferring this worksheet."))?;
    if !path.to_ascii_lowercase().ends_with(".xlsx") {
        return Err(refuse("linked worksheet transfers currently require an XLSX source or destination."));
    }
    Ok(path.to_string())
}

fn distinct_path(path: Option<&str>, other: Option<&str>) -> Result<String> {
    let path = saved_path(path)?;
    if other.is_some_and(|other| other.replace('\\', "/").eq_ignore_ascii_case(&path.replace('\\', "/"))) {
        return Err(refuse("both open workbooks have the same file path; save them to separate files first."));
    }
    Ok(path)
}

fn cache_snapshot(wb: &mut Workbook, n: usize, source: &Workbook) -> Result<()> {
    let old = wb.external_books.get(n.saturating_sub(1)).ok_or_else(|| refuse("a workbook link is unavailable."))?;
    let (book, missing) =
        gridcraft_model::links::book_from_source(source, &wb.link_needs(n), old, gridcraft_model::links::MAX_LINK_CELLS).map_err(refuse)?;
    if !missing.is_empty() {
        return Err(refuse("a linked sheet is unavailable."));
    }
    for name in &book.names {
        let expr = gridcraft_formula::parse(&name.refers_to).map_err(|_| refuse("a linked name could not be checked."))?;
        let mut unsupported = !gridcraft_model::links::links_in(&expr).is_empty();
        expr.walk(&mut |e| match e {
            Expr::Struct(_) => unsupported = true,
            Expr::Ref(r) if matches!(r.sheet, SheetSel::Span(..)) => unsupported = true,
            Expr::Call(n, _) if n.eq_ignore_ascii_case("INDIRECT") => unsupported = true,
            _ => {}
        });
        if unsupported {
            return Err(refuse("a linked name depends on another workbook, a table or an unsupported reference."));
        }
    }
    if let Some(slot) = Arc::make_mut(&mut wb.external_books).get_mut(n.saturating_sub(1)) {
        *slot = book;
    }
    Ok(())
}

impl FormulaTransfer<'_> {
    fn linked_sheet(&mut self, selector: &str, destination: &mut Workbook) -> Result<String> {
        if !self.allow_links {
            return Err(refuse("external references in worksheet objects cannot yet be transferred safely."));
        }
        if let Some(n) = gridcraft_model::links::link_number(selector) {
            let tail = selector.split_once(']').map_or("", |(_, s)| s);
            let target = if let Some(target) = self.imported.get(&n) {
                *target
            } else {
                let book = self
                    .source
                    .external_books
                    .get(n.saturating_sub(1))
                    .filter(|_| n > 0)
                    .ok_or_else(|| refuse("an external workbook link is unavailable."))?;
                let cells = book.sheets.iter().fold(0usize, |count, sheet| count.saturating_add(sheet.cells.len()));
                if cells > gridcraft_model::links::MAX_LINK_CELLS {
                    return Err(refuse("the external link cache is too large to transfer safely."));
                }
                let mut book = book.clone();
                let path = crate::cmd::workbook_links::resolve(&book.path, self.paths.base);
                if book.path.is_empty() || !path.is_absolute() {
                    return Err(refuse("an external link has no resolvable file path."));
                }
                book.path = path.to_string_lossy().into_owned();
                book.kept = None;
                let books = Arc::make_mut(&mut destination.external_books);
                books.push(book);
                let target = books.len();
                self.imported.insert(n, target);
                target
            };
            return Ok(format!("[{target}]{tail}"));
        }
        if self.source.sheet_index(selector).is_none() {
            return Err(refuse("a referenced source sheet is unavailable."));
        }
        let n = if let Some(n) = self.snapshot {
            n
        } else {
            let path = distinct_path(self.paths.source, self.paths.destination)?;
            let books = Arc::make_mut(&mut destination.external_books);
            books.push(gridcraft_model::ExternalBook { path, ..Default::default() });
            let n = books.len();
            self.snapshot = Some(n);
            n
        };
        Ok(format!("[{n}]{selector}"))
    }

    fn rewrite(&mut self, text: &str, destination: &mut Workbook, needed: &mut BTreeSet<usize>) -> Result<String> {
        let source = self.source;
        let sheet = self.sheet;
        let name = self.name;
        let tables = self.tables;
        if text.is_empty() {
            return Ok(String::new());
        }
        let expr = gridcraft_formula::parse(text).map_err(|_| refuse("a formula could not be checked."))?;
        let old = source.sheet(sheet).map(|s| s.name.as_str()).unwrap_or("");
        let mut error = None;
        let mapped = expr.map(&mut |mut e| {
            match &mut e {
                Expr::Ref(r) => match &mut r.sheet {
                    SheetSel::Current => {}
                    SheetSel::Named(s) if s.eq_ignore_ascii_case(old) => *s = name.to_string(),
                    SheetSel::Span(a, b) if a.eq_ignore_ascii_case(old) && b.eq_ignore_ascii_case(old) => {
                        *a = name.to_string();
                        *b = name.to_string();
                    }
                    SheetSel::Named(s) => match self.linked_sheet(s, destination) {
                        Ok(linked) => *s = linked,
                        Err(e) => error = Some(e.to_string()),
                    },
                    _ => error = Some("a multi-sheet reference cannot yet be transferred safely.".to_string()),
                },
                Expr::Struct(r) if !r.table.is_empty() && !tables.contains(&r.table.to_ascii_lowercase()) => {
                    error = Some("a formula refers to a table on another sheet.".into());
                }
                Expr::Name(n) => {
                    if let Some((qualifier, field)) = n.rsplit_once('!')
                        && gridcraft_model::links::link_number(qualifier).is_some()
                    {
                        match self.linked_sheet(qualifier, destination) {
                            Ok(linked) => *n = format!("{linked}!{field}"),
                            Err(e) => error = Some(e.to_string()),
                        }
                        return e;
                    }
                    if source.sheets.iter().flat_map(|s| &s.tables).any(|t| t.name.eq_ignore_ascii_case(n))
                        && !tables.contains(&n.to_ascii_lowercase())
                    {
                        error = Some("a formula refers to a table on another sheet.".into());
                    }
                    let found = source
                        .names
                        .iter()
                        .position(|d| d.scope == Some(sheet) && d.name.eq_ignore_ascii_case(n))
                        .or_else(|| source.names.iter().position(|d| d.scope.is_none() && d.name.eq_ignore_ascii_case(n)));
                    if let Some(index) = found {
                        needed.insert(index);
                    } else if n.contains(['!', '[', ']'])
                        || destination.names.iter().any(|d| d.name.eq_ignore_ascii_case(n))
                        || destination.sheets.iter().flat_map(|s| &s.tables).any(|t| t.name.eq_ignore_ascii_case(n))
                    {
                        error = Some(format!("the name '{n}' cannot be resolved safely in the destination."));
                    }
                }
                Expr::Call(n, _) if n.eq_ignore_ascii_case("INDIRECT") => error = Some("INDIRECT references cannot be checked safely.".into()),
                _ => {}
            }
            e
        });
        if let Some(error) = error {
            return Err(refuse(error));
        }
        Ok(gridcraft_formula::print(&mapped))
    }
}

fn remap_styles(source: &Workbook, destination: &mut Workbook, sheet: &mut Sheet) {
    let mut ids = HashMap::new();
    let mut map = |id: StyleId| *ids.entry(id).or_insert_with(|| destination.styles.intern(source.styles.get(id).clone()));
    let default = map(StyleId::DEFAULT);
    let cells: Vec<_> = sheet.cells.iter().map(|(c, cell)| (c, cell.style)).collect();
    for (c, old) in cells {
        let new = map(old);
        if new != old
            && let Some(cell) = sheet.cells.get_mut(c)
        {
            cell.style = new;
        }
    }
    for line in sheet.rows.values_mut().chain(sheet.cols.values_mut()) {
        if let Some(old) = line.style {
            line.style = Some(map(old));
        }
    }
    // A worksheet has no separate Normal style. Preserve implicit formatting through
    // column defaults rather than materialising its millions of empty cells.
    if default != StyleId::DEFAULT {
        for col in 0..MAX_COLS {
            sheet.cols.entry(col).or_default().style.get_or_insert(default);
        }
    }
}

fn rewrite_inbound(
    source: &Workbook,
    remaining: &mut Workbook,
    output: &Workbook,
    moving: usize,
    name: &str,
    tables: &HashSet<String>,
    paths: TransferPaths<'_>,
) -> Result<()> {
    let old = source.sheet(moving).map(|s| s.name.as_str()).unwrap_or("");
    let mut snapshot = None;
    let mut rewrite = |text: &str, allow: bool| -> Result<String> {
        if text.is_empty() {
            return Ok(String::new());
        }
        let e = gridcraft_formula::parse(text).map_err(|_| refuse("a remaining formula could not be checked."))?;
        let mut unsupported = false;
        let mut changed = false;
        let mut error = None;
        let mut target = || -> Result<String> {
            if !allow {
                return Err(refuse("a worksheet object depends on the sheet being moved."));
            }
            let n = if let Some(n) = snapshot {
                n
            } else {
                let path = distinct_path(paths.destination, paths.source)?;
                let books = Arc::make_mut(&mut remaining.external_books);
                books.push(gridcraft_model::ExternalBook { path, ..Default::default() });
                let n = books.len();
                snapshot = Some(n);
                n
            };
            Ok(format!("[{n}]{name}"))
        };
        let mapped = e.map(&mut |mut e| {
            match &mut e {
                Expr::Ref(r) => match &mut r.sheet {
                    SheetSel::Named(s) if s.eq_ignore_ascii_case(old) => match target() {
                        Ok(linked) => {
                            *s = linked;
                            changed = true;
                        }
                        Err(e) => error = Some(e),
                    },
                    SheetSel::Span(a, b) => {
                        let a = source.sheet_index(a);
                        let b = source.sheet_index(b);
                        unsupported |= a.zip(b).is_some_and(|(a, b)| (a.min(b)..=a.max(b)).contains(&moving));
                    }
                    _ => {}
                },
                Expr::Struct(r) => unsupported |= tables.contains(&r.table.to_ascii_lowercase()),
                Expr::Name(n) => {
                    unsupported |= tables.contains(&n.to_ascii_lowercase());
                    if let Some((sh, field)) = n.rsplit_once('!')
                        && sh.trim_matches('\'').replace("''", "'").eq_ignore_ascii_case(old)
                    {
                        match target() {
                            Ok(linked) => {
                                *n = format!("{linked}!{field}");
                                changed = true;
                            }
                            Err(e) => error = Some(e),
                        }
                    }
                }
                Expr::Call(n, _) if n.eq_ignore_ascii_case("INDIRECT") => unsupported = true,
                _ => {}
            }
            e
        });
        if let Some(e) = error {
            return Err(e);
        }
        if unsupported {
            return Err(refuse("a remaining table, multi-sheet or INDIRECT reference cannot be preserved safely. Use Create a copy instead."));
        }
        Ok(if changed { gridcraft_formula::print(&mapped) } else { text.to_string() })
    };
    for (index, sh) in source.sheets.iter().enumerate() {
        if index != moving {
            let mut updated = (**sh).clone();
            map_cell_formulas(&mut updated, &mut |text| rewrite(text, true))?;
            map_noncell_formulas(&mut updated, &mut |text| rewrite(text, false))?;
            for pivot in &sh.pivots {
                rewrite(&pivot.source, false)?;
            }
            if sh.slicers.iter().any(|s| tables.contains(&s.table.to_ascii_lowercase())) {
                return Err(refuse("a slicer depends on a table on the sheet being moved."));
            }
            if let Some(slot) = remaining.sheets.get_mut(index) {
                *slot = Arc::new(updated);
            }
        }
    }
    for (i, n) in source.names.iter().enumerate() {
        if n.scope != Some(moving) {
            let formula = rewrite(&n.formula, true)?;
            if let Some(n) = remaining.names.get_mut(i) {
                n.formula = formula;
            }
        }
    }
    if let Some(n) = snapshot {
        cache_snapshot(remaining, n, output)?;
    }
    Ok(())
}

// Excel does not permit external references in data validation and conditional
// formatting. Follow names too, so an imported global name cannot bypass that guard.
fn check_object_names(wb: &Workbook, sheet: usize) -> Result<()> {
    let Some(sh) = wb.sheet(sheet) else { return Ok(()) };
    map_noncell_formulas(&mut sh.clone(), &mut |text| {
        let mut queue = vec![text.to_string()];
        let mut seen = BTreeSet::new();
        while let Some(text) = queue.pop() {
            if text.is_empty() {
                continue;
            }
            let expr = gridcraft_formula::parse(&text).map_err(|_| refuse("a worksheet object formula could not be checked."))?;
            let mut external = !gridcraft_model::links::links_in(&expr).is_empty();
            expr.walk(&mut |e| {
                if let Expr::Name(n) = e {
                    let (scope, name) = match n.rsplit_once('!') {
                        Some((q, name)) => (wb.sheet_index(&q.trim_matches('\'').replace("''", "'")), name),
                        None => (Some(sheet), n.as_str()),
                    };
                    let found = wb
                        .names
                        .iter()
                        .enumerate()
                        .find(|(_, d)| scope.is_some() && d.scope == scope && d.name.eq_ignore_ascii_case(name))
                        .or_else(|| wb.names.iter().enumerate().find(|(_, d)| d.scope.is_none() && d.name.eq_ignore_ascii_case(name)));
                    if let Some((i, d)) = found
                        && seen.insert(i)
                    {
                        queue.push(d.formula.clone());
                    }
                    external |= n.contains('[');
                }
            });
            if external {
                return Err(refuse("a worksheet object uses a defined name linked to another workbook."));
            }
        }
        Ok(text.to_string())
    })
}

fn map_cell_formulas(sheet: &mut Sheet, f: &mut impl FnMut(&str) -> Result<String>) -> Result<()> {
    let formulas: Vec<_> = sheet.cells.iter().filter_map(|(c, cell)| cell.formula.as_ref().map(|v| (c, v.clone()))).collect();
    for (c, old) in formulas {
        let text = f(&old.text)?;
        if text != old.text {
            let mut formula = gridcraft_model::Formula::new(&text);
            formula.array = old.array;
            if let Some(cell) = sheet.cells.get_mut(c) {
                cell.formula = Some(Arc::new(formula));
            }
        }
    }
    Ok(())
}

fn map_noncell_formulas(sheet: &mut Sheet, f: &mut impl FnMut(&str) -> Result<String>) -> Result<()> {
    let optional = |v: &mut Option<String>, f: &mut dyn FnMut(&str) -> Result<String>| -> Result<()> {
        if let Some(text) = v {
            *text = f(text)?;
        }
        Ok(())
    };
    for t in &mut sheet.tables {
        for c in &mut t.columns {
            optional(&mut c.formula, f)?;
            if c.totals == TotalsFn::Custom {
                optional(&mut c.totals_label, f)?;
            }
        }
    }
    for c in &mut sheet.charts {
        optional(&mut c.source, f)?;
        for s in &mut c.series {
            s.values = f(&s.values)?;
            optional(&mut s.categories, f)?;
            optional(&mut s.name, f)?;
        }
    }
    for s in &mut sheet.sparklines {
        s.source = f(&s.source)?;
    }
    for h in sheet.hyperlinks.values_mut() {
        if !h.external {
            let prefix = if h.target.starts_with('#') { "#" } else { "" };
            h.target = format!("{prefix}{}", f(h.target.trim_start_matches('#'))?);
        } else if !h.target.contains(":") && !h.target.starts_with('/') && !h.target.starts_with('\\') {
            return Err(refuse("relative file hyperlinks cannot yet be transferred safely."));
        }
    }
    for v in &mut sheet.validations {
        // Match list_items: references and names may be written without a leading =.
        let literal_list = v.kind == gridcraft_model::ValidationKind::List
            && (v.f1.trim().starts_with('"')
                || (!v.f1.contains('!') && gridcraft_formula::parse(&v.f1).map(|e| !matches!(e, Expr::Ref(_) | Expr::Name(_))).unwrap_or(true)));
        if !literal_list {
            v.f1 = f(&v.f1)?;
        }
        optional(&mut v.f2, f)?;
    }
    let threshold = |v: &mut CfValueKind, f: &mut dyn FnMut(&str) -> Result<String>| -> Result<()> {
        if let CfValueKind::Formula(text) = v {
            *text = f(text)?;
        }
        Ok(())
    };
    for cf in &mut sheet.cond_formats {
        match &mut cf.rule {
            CfRule::CellIs { a, b, .. } => {
                *a = f(a)?;
                optional(b, f)?;
            }
            CfRule::Expression { formula, .. } => *formula = f(formula)?,
            CfRule::ColorScale { stops } => {
                for (v, _) in stops {
                    threshold(v, f)?;
                }
            }
            CfRule::DataBar { min, max, .. } => {
                threshold(min, f)?;
                threshold(max, f)?;
            }
            CfRule::IconSet { thresholds, .. } => {
                for v in thresholds {
                    threshold(v, f)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}
