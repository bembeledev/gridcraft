//! References into linked workbooks, as typed and as shown.
//!
//! Files and the model only hold `[n]` (`[1]Sheet1!A1`, `[1]!Rate`), the position of the link in
//! `Workbook::external_books`. People type and read the linked file instead: its name
//! (`[Source.xlsx]Sheet1!A1`, `Source.xlsx!Rate`) or its path
//! (`'C:\links\[Source.xlsx]Sheet1'!A1`, `'C:\links\Source.xlsx'!Rate`).
//!
//! Data › Workbook Links works on the links themselves: what each one reads from its source
//! ([`Workbook::link_needs`], [`book_from_source`]), Break Links ([`Workbook::break_links`]) and
//! Change Source ([`Workbook::merge_link`]).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use gridcraft_core::{CellRef, RangeRef, Value};
use gridcraft_formula::{Expr, SheetSel};

use crate::cell::{Cell, Formula};
use crate::workbook::{ExternalBook, ExternalName, ExternalSheet, Workbook};

impl Workbook {
    /// The link `[n]` (1-based) to the workbook at `path`: the link to that path or, for a bare
    /// file name, to a file of that name; otherwise a new link, which holds no values until the
    /// file is read (references into it are `#REF!`, as in Excel when the file is missing).
    pub fn link_book(&mut self, path: &str) -> usize {
        let bare = !path.contains(['\\', '/']);
        // `C:/links/Source.xlsx` is the same file as `C:\links\Source.xlsx`.
        let same = |a: &str, b: &str| a.replace('/', "\\").eq_ignore_ascii_case(&b.replace('/', "\\"));
        let found = self
            .external_books
            .iter()
            .position(|b| !b.path.is_empty() && if bare { b.file_name().eq_ignore_ascii_case(path) } else { same(&b.path, path) });
        let i = found.unwrap_or_else(|| {
            let books = std::sync::Arc::make_mut(&mut self.external_books);
            books.push(ExternalBook { path: path.to_string(), ..Default::default() });
            books.len() - 1
        });
        i + 1
    }

    /// References to another workbook by file name or path, as typed, made references through
    /// its link (`'C:\links\[Source.xlsx]Sheet1'!A1` → `[1]Sheet1!A1`, `Source.xlsx!Rate` →
    /// `[1]!Rate`), adding the link when there is none to that file.
    pub fn link_typed_books(&mut self, e: Expr) -> Expr {
        e.map(&mut |e| match e {
            Expr::Ref(mut r) => {
                r.sheet = match r.sheet {
                    SheetSel::Named(s) => SheetSel::Named(self.linked_sheet(&s, None).unwrap_or(s)),
                    SheetSel::Span(a, b) => SheetSel::Span(self.linked_sheet(&a, Some(&b)).unwrap_or(a), b),
                    other => other,
                };
                Expr::Ref(r)
            }
            Expr::Name(n) => Expr::Name(self.linked_name(&n).unwrap_or(n)),
            other => other,
        })
    }

    /// `[n]` written out as the linked file, as the formula bar and the Name Manager show it
    /// (`[1]Sheet1!A1` → `'C:\links\[Source.xlsx]Sheet1'!A1`).
    pub fn show_linked_books(&self, e: Expr) -> Expr {
        e.map(&mut |e| match e {
            Expr::Ref(mut r) => {
                r.sheet = match r.sheet {
                    SheetSel::Named(s) => SheetSel::Named(self.shown_sheet(&s).unwrap_or(s)),
                    SheetSel::Span(a, b) => SheetSel::Span(self.shown_sheet(&a).unwrap_or(a), b),
                    other => other,
                };
                Expr::Ref(r)
            }
            Expr::Name(n) => Expr::Name(self.shown_name(&n).unwrap_or(n)),
            other => other,
        })
    }

    /// [`Workbook::link_typed_books`] on formula text (without `=`); text that doesn't parse is
    /// returned as it is.
    pub fn link_typed_text(&mut self, text: &str) -> String {
        match gridcraft_formula::parse(text) {
            Ok(e) => gridcraft_formula::print(&self.link_typed_books(e)),
            Err(_) => text.to_string(),
        }
    }

    /// [`Workbook::show_linked_books`] on formula text (without `=`).
    pub fn shown_formula(&self, text: &str) -> String {
        if self.external_books.iter().all(|b| b.path.is_empty()) {
            return text.to_string();
        }
        match gridcraft_formula::parse(text) {
            Ok(e) => gridcraft_formula::print(&self.show_linked_books(e)),
            Err(_) => text.to_string(),
        }
    }

    /// `C:\links\[Source.xlsx]Sheet1` → `[1]Sheet1`, adding the sheet (and `last`, the end of a
    /// sheet range) to the link when it doesn't know it.
    fn linked_sheet(&mut self, typed: &str, last: Option<&str>) -> Option<String> {
        let (path, sheet) = typed_book(typed)?;
        let n = self.link_book(&path);
        if std::iter::once(sheet).chain(last).all(|s| self.external_books.get(n - 1).is_some_and(|b| b.sheet_index(s).is_some())) {
            return Some(format!("[{n}]{sheet}"));
        }
        let book = std::sync::Arc::make_mut(&mut self.external_books).get_mut(n - 1)?;
        // The link part is written from the model from now on, so the new sheet is in it.
        book.kept = None;
        for s in std::iter::once(sheet).chain(last) {
            if book.sheet_index(s).is_none() {
                book.sheets.push(ExternalSheet { name: s.to_string(), refresh_error: true, ..Default::default() });
            }
        }
        Some(format!("[{n}]{sheet}"))
    }

    /// `Source.xlsx!Rate` → `[1]!Rate`, `C:\links\[Source.xlsx]Sheet1!Rate` → `[1]Sheet1!Rate`,
    /// adding the name to the link when it doesn't know it.
    fn linked_name(&mut self, typed: &str) -> Option<String> {
        let (qualifier, name) = typed.rsplit_once('!')?;
        let (path, sheet) = match typed_book(qualifier) {
            Some((path, sheet)) => (path, sheet),
            None if self.sheet_index(qualifier).is_none() && looks_like_file(qualifier) => (qualifier.to_string(), ""),
            None => return None,
        };
        let n = self.link_book(&path);
        let known = self.external_books.get(n - 1).is_some_and(|b| {
            let scope = if sheet.is_empty() { Some(None) } else { b.sheet_index(sheet).map(Some) };
            scope.is_some_and(|scope| b.names.iter().any(|x| x.sheet == scope && x.name.eq_ignore_ascii_case(name)))
        });
        if known {
            return Some(format!("[{n}]{sheet}!{name}"));
        }
        let book = std::sync::Arc::make_mut(&mut self.external_books).get_mut(n - 1)?;
        book.kept = None;
        let scope = if sheet.is_empty() {
            None
        } else {
            Some(book.sheet_index(sheet).unwrap_or_else(|| {
                book.sheets.push(ExternalSheet { name: sheet.to_string(), refresh_error: true, ..Default::default() });
                book.sheets.len() - 1
            }))
        };
        if !book.names.iter().any(|x| x.sheet == scope && x.name.eq_ignore_ascii_case(name)) {
            book.names.push(ExternalName { name: name.to_string(), sheet: scope, refers_to: String::new() });
        }
        Some(format!("[{n}]{sheet}!{name}"))
    }

    /// The linked book `[n]` that starts `s`, with the rest of `s`.
    fn book_of<'a>(&self, s: &'a str) -> Option<(&ExternalBook, &'a str)> {
        let (n, rest) = s.strip_prefix('[')?.split_once(']')?;
        let n: usize = n.parse().ok()?;
        let book = self.external_books.get(n.checked_sub(1)?)?;
        (!book.path.is_empty()).then_some((book, rest))
    }

    /// `[1]Sheet1` → `C:\links\[Source.xlsx]Sheet1`.
    fn shown_sheet(&self, s: &str) -> Option<String> {
        let (book, sheet) = self.book_of(s)?;
        Some(format!("{}[{}]{sheet}", book.folder(), book.file_name()))
    }

    /// `[1]!Rate` → `'C:\links\Source.xlsx'!Rate`, `[1]Sheet1!Rate` →
    /// `'C:\links\[Source.xlsx]Sheet1'!Rate`.
    fn shown_name(&self, s: &str) -> Option<String> {
        let (book, rest) = self.book_of(s)?;
        let (sheet, name) = rest.split_once('!')?;
        let qualifier = if sheet.is_empty() { book.path.clone() } else { format!("{}[{}]{sheet}", book.folder(), book.file_name()) };
        // Names hold their qualifier bare; the printer quotes it (`'C:\links\Source.xlsx'!Rate`).
        Some(format!("{qualifier}!{name}"))
    }
}

/// `C:\links\[Source.xlsx]Sheet1` → (`C:\links\Source.xlsx`, `Sheet1`). `None` for `[1]Sheet1`,
/// already a link's number, and for anything without a file name in brackets (sheet names
/// can't hold brackets). The bracketed part must look like a file (a folder before it, or an
/// extension: `[Source.xlsx]`), so `[-1]Sheet1` and `[ 1]Sheet1` stay malformed indexes.
pub fn typed_book(s: &str) -> Option<(String, &str)> {
    let (folder, rest) = s.split_once('[')?;
    let (file, sheet) = rest.split_once(']')?;
    if file.is_empty() || (folder.is_empty() && file.bytes().all(|b| b.is_ascii_digit())) || sheet.contains(['[', ']']) {
        return None;
    }
    let path = format!("{folder}{file}");
    looks_like_file(&path).then_some((path, sheet))
}

/// Whether the qualifier of `Source.xlsx!Rate` names a file: it has a folder or an extension
/// (and isn't a linked sheet, `[1]Data.v2`).
fn looks_like_file(q: &str) -> bool {
    if q.contains(['[', ']']) {
        return false;
    }
    let file = q.rsplit(['\\', '/']).next().unwrap_or(q);
    q.len() != file.len()
        || file.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()))
}

// ---------------------------------------------------------------- managing links

/// Most cells a link may hold after its values are read from the source. A link holds the cells
/// this workbook refers to; whole-column references are trimmed to the source's used range.
pub const MAX_LINK_CELLS: usize = 2_000_000;
/// Most names followed inside a linked workbook's names when its values are read.
const MAX_LINK_NAMES: usize = 1_000;

/// The link a sheet or name qualifier names by its number: `[2]Prices` → 2, `[1]` → 1. `None` for
/// this workbook (`Prices`, `[0]Prices`) and anything else.
pub fn link_number(s: &str) -> Option<usize> {
    let (n, _) = s.strip_prefix('[')?.split_once(']')?;
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    n.parse::<usize>().ok().filter(|n| *n > 0)
}

/// The links (1-based) `e` names directly: `[n]` before a sheet, a sheet range or a name.
pub fn links_in(e: &Expr) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    e.walk(&mut |e| match e {
        Expr::Ref(r) => match &r.sheet {
            SheetSel::Named(s) | SheetSel::Span(s, _) => out.extend(link_number(s)),
            SheetSel::Current => {}
        },
        Expr::Name(n) => {
            if let Some((q, _)) = n.rsplit_once('!') {
                out.extend(link_number(q));
            }
        }
        _ => {}
    });
    out
}

/// `e` with its links renumbered by `map` (`[3]Prices!A1` → `[2]Prices!A1`).
fn renumber_expr(e: Expr, map: &dyn Fn(usize) -> usize) -> Expr {
    let fix = |s: String| match link_number(&s) {
        Some(k) if map(k) != k => format!("[{}]{}", map(k), s.split_once(']').map_or("", |(_, rest)| rest)),
        _ => s,
    };
    e.map(&mut |e| match e {
        Expr::Ref(mut r) => {
            r.sheet = match r.sheet {
                SheetSel::Named(s) => SheetSel::Named(fix(s)),
                SheetSel::Span(a, b) => SheetSel::Span(fix(a), b),
                other => other,
            };
            Expr::Ref(r)
        }
        Expr::Name(n) => match n.rsplit_once('!') {
            Some((q, name)) if link_number(q).is_some() => Expr::Name(format!("{}!{name}", fix(q.to_string()))),
            _ => Expr::Name(n),
        },
        other => other,
    })
}

/// Formula text with its links renumbered by `map`; `None` when nothing changes (or it doesn't
/// parse).
fn renumber_text(text: &str, map: &dyn Fn(usize) -> usize) -> Option<String> {
    let e = gridcraft_formula::parse(text.strip_prefix('=').unwrap_or(text)).ok()?;
    if links_in(&e).iter().all(|k| map(*k) == *k) {
        return None;
    }
    Some(gridcraft_formula::print(&renumber_expr(e, map)))
}

/// What a workbook reads from one of its links, as its formulas, names and charts say: cells of
/// the linked workbook's sheets, sheet ranges and names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LinkNeeds {
    /// (sheet name, cells); whole rows and columns are trimmed to the source's used range.
    pub ranges: Vec<(String, RangeRef)>,
    /// `[1]Jan:Dec!B2`: (first sheet, last sheet, cells), every sheet between them in the source.
    pub spans: Vec<(String, String, RangeRef)>,
    /// (name, sheet it is scoped to): `[1]!Rate` is (`Rate`, `None`), `[1]Prices!Rate` is
    /// (`Rate`, `Prices`).
    pub names: Vec<(String, Option<String>)>,
}

impl LinkNeeds {
    fn add(&mut self, n: usize, e: &Expr) {
        e.walk(&mut |e| match e {
            Expr::Ref(r) => match &r.sheet {
                SheetSel::Named(s) if link_number(s) == Some(n) => {
                    let sheet = s.split_once(']').map_or("", |(_, rest)| rest);
                    self.ranges.push((sheet.to_string(), r.range()));
                }
                SheetSel::Span(a, b) if link_number(a) == Some(n) => {
                    let first = a.split_once(']').map_or("", |(_, rest)| rest);
                    self.spans.push((first.to_string(), b.clone(), r.range()));
                }
                _ => {}
            },
            Expr::Name(name) => {
                if let Some((q, name)) = name.rsplit_once('!')
                    && link_number(q) == Some(n)
                {
                    let sheet = q.split_once(']').map_or("", |(_, rest)| rest);
                    self.names.push((name.to_string(), (!sheet.is_empty()).then(|| sheet.to_string())));
                }
            }
            _ => {}
        });
    }
}

/// What [`Workbook::break_links`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BrokenLinks {
    /// Cell formulas replaced by their values.
    pub formulas: usize,
    /// Links removed (1-based, numbered as before the break).
    pub removed: Vec<usize>,
    /// Links still named by defined names or charts, which keep them.
    pub kept: Vec<usize>,
}

impl Workbook {
    /// The position of the defined name `name` scoped to `scope` (`None` = the workbook).
    fn name_position(&self, name: &str, scope: Option<usize>) -> Option<usize> {
        self.names.iter().position(|d| d.scope == scope && d.name.eq_ignore_ascii_case(name))
    }

    /// The defined names (positions in `names`) `e` uses as seen from `sheet`: the sheet's own
    /// name first, then the workbook's. Names in linked workbooks (`[1]!Rate`) aren't these.
    fn names_used(&self, e: &Expr, sheet: Option<usize>) -> Vec<usize> {
        let mut out = vec![];
        e.walk(&mut |e| {
            if let Expr::Name(n) = e {
                let found = match n.rsplit_once('!') {
                    Some((q, name)) => self.sheet_index(q).and_then(|si| self.name_position(name, Some(si))),
                    None => sheet.and_then(|si| self.name_position(n, Some(si))).or_else(|| self.name_position(n, None)),
                };
                out.extend(found);
            }
        });
        out
    }

    /// For each defined name (by position), the links it reads, also through the names it uses.
    fn name_links(&self) -> Vec<BTreeSet<usize>> {
        let exprs: Vec<Option<Expr>> = self.names.iter().map(|n| gridcraft_formula::parse(&n.formula).ok()).collect();
        let mut links: Vec<BTreeSet<usize>> = exprs.iter().map(|e| e.as_ref().map(links_in).unwrap_or_default()).collect();
        let uses: Vec<Vec<usize>> =
            exprs.iter().zip(&self.names).map(|(e, n)| e.as_ref().map(|e| self.names_used(e, n.scope)).unwrap_or_default()).collect();
        // A name can use a name that uses a link: spread until nothing changes (no chain is
        // longer than the number of names).
        for _ in 0..self.names.len() {
            let mut changed = false;
            for (i, used) in uses.iter().enumerate() {
                let add: BTreeSet<usize> = used.iter().filter_map(|j| links.get(*j)).flatten().copied().collect();
                if let Some(l) = links.get_mut(i) {
                    let before = l.len();
                    l.extend(add);
                    changed |= l.len() != before;
                }
            }
            if !changed {
                break;
            }
        }
        links
    }

    /// The links a formula on sheet `sheet` reads, directly or through defined names.
    fn formula_links(&self, sheet: usize, e: &Expr, names: &[BTreeSet<usize>]) -> BTreeSet<usize> {
        let mut out = links_in(e);
        for i in self.names_used(e, Some(sheet)) {
            out.extend(names.get(i).into_iter().flatten().copied());
        }
        out
    }

    /// How many cell formulas read link `n` (1-based), directly or through a defined name.
    pub fn link_formula_count(&self, n: usize) -> usize {
        let names = self.name_links();
        let mut count = 0usize;
        for (si, sh) in self.sheets.iter().enumerate() {
            for (_, c) in sh.cells.iter() {
                if c.formula.as_ref().and_then(|f| f.expr_arc()).is_some_and(|e| self.formula_links(si, &e, &names).contains(&n)) {
                    count += 1;
                }
            }
        }
        count
    }

    /// Every chart series formula's text.
    fn chart_formulas(&self) -> impl Iterator<Item = &str> {
        self.sheets
            .iter()
            .flat_map(|sh| sh.charts.iter())
            .flat_map(|c| c.series.iter())
            .flat_map(|s| [s.name.as_deref(), s.categories.as_deref(), Some(s.values.as_str()), s.bubble_sizes.as_deref()].into_iter().flatten())
    }

    /// Calls `f` with every formula that can name a link: cell formulas, defined names and
    /// chart series.
    fn each_link_formula(&self, f: &mut dyn FnMut(&Expr)) {
        for sh in &self.sheets {
            for (_, c) in sh.cells.iter() {
                if let Some(e) = c.formula.as_ref().filter(|x| x.text.contains('[')).and_then(|x| x.expr_arc()) {
                    f(&e);
                }
            }
        }
        let texts = self.names.iter().map(|n| n.formula.as_str()).chain(self.chart_formulas());
        for t in texts.filter(|t| t.contains('[')) {
            if let Ok(e) = gridcraft_formula::parse(t.strip_prefix('=').unwrap_or(t)) {
                f(&e);
            }
        }
    }

    /// The links still named anywhere (cell formulas, defined names, chart series).
    pub fn links_in_use(&self) -> BTreeSet<usize> {
        let mut used = BTreeSet::new();
        self.each_link_formula(&mut |e| used.extend(links_in(e)));
        used
    }

    /// What this workbook reads from link `n` (1-based), from its cell formulas, defined names
    /// and chart series.
    pub fn link_needs(&self, n: usize) -> LinkNeeds {
        let mut needs = LinkNeeds::default();
        self.each_link_formula(&mut |e| needs.add(n, e));
        needs
    }

    /// Renumbers links everywhere formulas name them (cells, defined names, chart series):
    /// `[k]` becomes `[map(k)]`.
    pub fn renumber_links(&mut self, map: &dyn Fn(usize) -> usize) {
        for si in 0..self.sheets.len() {
            let Some(sh) = self.sheets.get(si) else { continue };
            let changes: Vec<(CellRef, Cell)> = sh
                .cells
                .iter()
                .filter_map(|(at, c)| {
                    let f = c.formula.as_ref().filter(|f| f.text.contains('['))?;
                    let e = f.expr()?;
                    if links_in(&e).iter().all(|k| map(*k) == *k) {
                        return None;
                    }
                    let mut nf = Formula::from_expr(renumber_expr(e, map));
                    nf.array = f.array;
                    Some((at, Cell { value: c.value.clone(), formula: Some(Arc::new(nf)), style: c.style }))
                })
                .collect();
            let charts = sh.charts.iter().flat_map(|c| c.series.iter()).any(|s| {
                [s.name.as_deref(), s.categories.as_deref(), Some(s.values.as_str()), s.bubble_sizes.as_deref()]
                    .into_iter()
                    .flatten()
                    .any(|t| renumber_text(t, map).is_some())
            });
            if changes.is_empty() && !charts {
                continue;
            }
            let Some(sh) = self.sheet_mut(si) else { continue };
            for (at, cell) in changes {
                sh.cells.set(at, cell);
            }
            for s in sh.charts.iter_mut().flat_map(|c| c.series.iter_mut()) {
                for field in [&mut s.name, &mut s.categories, &mut s.bubble_sizes].into_iter().flatten() {
                    if let Some(t) = renumber_text(field, map) {
                        *field = t;
                    }
                }
                if let Some(t) = renumber_text(&s.values, map) {
                    s.values = t;
                }
            }
        }
        for name in &mut self.names {
            if let Some(t) = renumber_text(&name.formula, map) {
                name.formula = t;
            }
        }
    }

    /// Removes link `n` (1-based) and renumbers the links after it (`[3]` becomes `[2]`).
    /// References to `n` itself are left alone: callers remove only links nothing names.
    fn remove_link(&mut self, n: usize) {
        if n == 0 || n > self.external_books.len() {
            return;
        }
        Arc::make_mut(&mut self.external_books).remove(n - 1);
        self.renumber_links(&|k| if k > n { k - 1 } else { k });
    }

    /// Data › Workbook Links › Break Links: every cell formula that reads one of `links`
    /// (1-based), directly or through a defined name, becomes the value it shows, and a dynamic
    /// or legacy array the values of its whole range. Defined names stay as they are, as in
    /// Excel. A link nothing names any more is removed, and the links after it move up.
    pub fn break_links(&mut self, links: &BTreeSet<usize>) -> BrokenLinks {
        let names = self.name_links();
        let mut report = BrokenLinks::default();
        for si in 0..self.sheets.len() {
            let Some(sh) = self.sheets.get(si) else { continue };
            let mut values: Vec<(CellRef, Value)> = vec![];
            for (at, c) in sh.cells.iter() {
                let Some(f) = c.formula.as_ref() else { continue };
                let Some(e) = f.expr_arc() else { continue };
                if self.formula_links(si, &e, &names).is_disjoint(links) {
                    continue;
                }
                report.formulas += 1;
                match sh.spill_ranges.get(&at).copied().or(f.array) {
                    Some(range) => values.extend(range.iter().map(|p| (p, sh.value(p)))),
                    None => values.push((at, c.value.clone())),
                }
            }
            if values.is_empty() {
                continue;
            }
            let Some(sh) = self.sheet_mut(si) else { continue };
            for (at, v) in values {
                let style = sh.style_id(at);
                if v.is_empty() && style == crate::StyleId::default() {
                    sh.remove_cell(at);
                } else {
                    sh.set_cell(at, Cell { value: v, formula: None, style });
                }
            }
        }
        let used = self.links_in_use();
        for &n in links.iter().rev() {
            if n == 0 || n > self.external_books.len() {
                continue;
            }
            if used.contains(&n) {
                report.kept.push(n);
            } else {
                self.remove_link(n);
                report.removed.push(n);
            }
        }
        report.removed.reverse();
        report.kept.reverse();
        report
    }

    /// Change Source onto a file another link already reads: references to link `from` become
    /// references to link `into` (both 1-based), and `from` goes.
    pub fn merge_link(&mut self, from: usize, into: usize) {
        let n = self.external_books.len();
        if from == into || from == 0 || into == 0 || from > n || into > n {
            return;
        }
        self.renumber_links(&|k| if k == from { into } else { k });
        self.remove_link(from);
    }
}

/// A link's new contents read from `src`, the linked workbook: every sheet of `src` (by name, in
/// its order) with the cells `needs` asks for and the cells `old` already held, and the names
/// `needs` asks for (and `old` held) with what they refer to in `src`, following the names used
/// inside them. Also returns the sheets asked for that `src` doesn't have: they stay in the link
/// as `refreshError` sheets without cells, so references to them are `#REF!`. Errs when the link
/// would hold more than `max_cells` cells.
pub fn book_from_source(src: &Workbook, needs: &LinkNeeds, old: &ExternalBook, max_cells: usize) -> Result<(ExternalBook, Vec<String>), String> {
    let src_sheet = |name: &str| src.sheets.iter().position(|s| s.name.eq_ignore_ascii_case(name));
    let mut ranges: Vec<(String, RangeRef)> = needs.ranges.clone();
    let mut spans = needs.spans.clone();
    let mut missing: Vec<String> = vec![];
    let note_missing = |name: &str, missing: &mut Vec<String>| {
        if src_sheet(name).is_none() && !missing.iter().any(|m| m.eq_ignore_ascii_case(name)) {
            missing.push(name.to_string());
        }
    };

    // Names, and the cells and names they reach.
    let mut queue: Vec<(String, Option<String>)> = needs.names.clone();
    queue.extend(old.names.iter().map(|n| (n.name.clone(), n.sheet.and_then(|i| old.sheets.get(i)).map(|s| s.name.clone()))));
    let mut found: Vec<(String, Option<String>, String)> = vec![];
    let mut seen: BTreeSet<(String, Option<String>)> = BTreeSet::new();
    while let Some((name, scope)) = queue.pop() {
        if !seen.insert((name.to_ascii_lowercase(), scope.as_ref().map(|s| s.to_ascii_lowercase()))) {
            continue;
        }
        if seen.len() > MAX_LINK_NAMES {
            break;
        }
        let scope_index = scope.as_deref().and_then(src_sheet);
        let def = scope_index
            .and_then(|si| src.names.iter().find(|d| d.scope == Some(si) && d.name.eq_ignore_ascii_case(&name)))
            .or_else(|| src.names.iter().find(|d| d.scope.is_none() && d.name.eq_ignore_ascii_case(&name)));
        let Some(def) = def else {
            if let Some(s) = &scope {
                note_missing(s, &mut missing);
            }
            found.push((name, scope, "#REF!".into()));
            continue;
        };
        let def_scope = def.scope.and_then(|i| src.sheets.get(i)).map(|s| s.name.clone());
        if let Ok(e) = gridcraft_formula::parse(&def.formula) {
            e.walk(&mut |e| match e {
                Expr::Ref(r) => match &r.sheet {
                    SheetSel::Named(s) => ranges.push((s.clone(), r.range())),
                    SheetSel::Span(a, b) => spans.push((a.clone(), b.clone(), r.range())),
                    // A sheetless reference reads the name's own sheet; in a workbook-level name it
                    // reads nothing (`#REF!` when evaluated).
                    SheetSel::Current => {
                        if let Some(s) = &def_scope {
                            ranges.push((s.clone(), r.range()));
                        }
                    }
                },
                Expr::Name(m) => match m.rsplit_once('!') {
                    Some((q, m)) if link_number(q).is_none() => queue.push((m.to_string(), Some(q.to_string()))),
                    Some(_) => {}
                    None => queue.push((m.clone(), def_scope.clone())),
                },
                _ => {}
            });
        }
        found.push((def.name.clone(), def_scope, def.formula.clone()));
    }

    // Sheets: the source's, in its order, then the ones asked for that it lacks.
    for (sheet, _) in &needs.ranges {
        note_missing(sheet, &mut missing);
    }
    for (a, b, _) in &needs.spans {
        note_missing(a, &mut missing);
        note_missing(b, &mut missing);
    }
    let mut sheets: Vec<ExternalSheet> = vec![];
    let mut total = 0usize;
    for (si, sh) in src.sheets.iter().enumerate() {
        let mut cells: BTreeMap<CellRef, Value> = BTreeMap::new();
        let mut wanted: Vec<RangeRef> = ranges.iter().filter(|(s, _)| s.eq_ignore_ascii_case(&sh.name)).map(|(_, r)| *r).collect();
        for (a, b, r) in &spans {
            if let (Some(i), Some(j)) = (src_sheet(a), src_sheet(b))
                && (i.min(j)..=i.max(j)).contains(&si)
            {
                wanted.push(*r);
            }
        }
        let used = sh.used_range();
        let put = |at: CellRef, v: Value, cells: &mut BTreeMap<CellRef, Value>| {
            if !v.is_empty() && !matches!(v, Value::Array(_)) {
                cells.insert(at, v);
            }
        };
        for r in wanted {
            let Some(r) = used.and_then(|u| u.intersection(&r)) else { continue };
            for (at, _) in sh.cells.iter_range(r) {
                put(at, sh.value(at), &mut cells);
            }
            for (at, v) in sh.spill.range(r.start..=r.end) {
                if r.contains(*at) && !cells.contains_key(at) {
                    put(*at, v.clone(), &mut cells);
                }
            }
            if total.saturating_add(cells.len()) > max_cells {
                return Err(format!("the link would hold more than {max_cells} cells"));
            }
        }
        // Cells the link already held stay in it, with their new values.
        if let Some(prev) = old.sheet(&sh.name) {
            for (at, _) in &prev.cells {
                put(*at, sh.value(*at), &mut cells);
            }
        }
        total = total.saturating_add(cells.len());
        if total > max_cells {
            return Err(format!("the link would hold more than {max_cells} cells"));
        }
        sheets.push(ExternalSheet { name: sh.name.clone(), cells: cells.into_iter().collect(), refresh_error: false });
    }
    for m in &missing {
        sheets.push(ExternalSheet { name: m.clone(), cells: vec![], refresh_error: true });
    }
    let sheet_of = |name: &Option<String>| name.as_ref().and_then(|n| sheets.iter().position(|s| s.name.eq_ignore_ascii_case(n)));
    let mut names: Vec<ExternalName> = found
        .iter()
        .map(|(name, scope, refers_to)| ExternalName { name: name.clone(), sheet: sheet_of(scope), refers_to: refers_to.clone() })
        .collect();
    names.sort_by_key(|a| (a.sheet, a.name.to_ascii_lowercase()));
    names.dedup_by(|a, b| a.sheet == b.sheet && a.name.eq_ignore_ascii_case(&b.name));
    Ok((ExternalBook { path: old.path.clone(), kept: None, sheets, names }, missing))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DefinedName;

    fn name(name: &str, scope: Option<usize>, formula: &str) -> DefinedName {
        DefinedName { name: name.into(), scope, formula: formula.into(), comment: String::new(), hidden: false }
    }

    fn put(wb: &mut Workbook, sheet: usize, cell: &str, v: Value) {
        let at = CellRef::parse(cell).unwrap_or_default();
        if let Some(sh) = wb.sheet_mut(sheet) {
            sh.set_value(at, v);
        }
    }

    /// A linked workbook: Sheet1!A1:A3 = 1, 2, 3, Jan/Feb!B2 = 10/20, the names `Rate`
    /// (Sheet1!$A$1), `Twice` (=Rate*2, through another name) and Feb's own `Local` (B2).
    fn source() -> Workbook {
        let mut src = Workbook::new();
        for (i, v) in [1.0, 2.0, 3.0].into_iter().enumerate() {
            put(&mut src, 0, &format!("A{}", i + 1), Value::Number(v));
        }
        put(&mut src, 0, "Z99", Value::Number(9.0));
        for (name, v) in [("Jan", 10.0), ("Feb", 20.0)] {
            src.sheets.push(Arc::new(crate::Sheet::new(name)));
            let i = src.sheets.len() - 1;
            put(&mut src, i, "B2", Value::Number(v));
        }
        src.names.push(name("Rate", None, "Sheet1!$A$1"));
        src.names.push(name("Twice", None, "Rate*2+Sheet1!$A$3"));
        src.names.push(name("Local", Some(2), "$B$2"));
        src
    }

    #[test]
    fn a_link_reads_what_the_workbook_asks_of_it() {
        let mut wb = Workbook::new();
        Arc::make_mut(&mut wb.external_books).push(ExternalBook { path: "Source.xlsx".into(), ..Default::default() });
        for (cell, f) in
            [("A1", "SUM([1]Sheet1!A1:A2)"), ("A2", "SUM([1]Jan:Feb!B2)"), ("A3", "[1]!Twice"), ("A4", "[1]Feb!Local"), ("A5", "[1]Gone!A1")]
        {
            if let Some(sh) = wb.sheet_mut(0) {
                sh.set_cell(CellRef::parse(cell).unwrap_or_default(), Cell::formula(Formula::new(f)));
            }
        }
        let needs = wb.link_needs(1);
        assert_eq!(needs.spans.len(), 1);
        assert_eq!(needs.names, [("Twice".to_string(), None), ("Local".to_string(), Some("Feb".to_string()))]);
        let (book, missing) = book_from_source(&source(), &needs, &wb.external_books[0], MAX_LINK_CELLS).unwrap_or_default();
        assert_eq!(missing, ["Gone"]);
        let names: Vec<&str> = book.sheets.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Sheet1", "Jan", "Feb", "Gone"]);
        // A1:A2, and A1 and A3 through the names; not Z99, which nothing reads.
        let s1: Vec<String> = book.sheets[0].cells.iter().map(|(c, _)| c.a1()).collect();
        assert_eq!(s1, ["A1", "A2", "A3"]);
        assert_eq!(book.sheets[1].value(CellRef::new(1, 1)), Value::Number(10.0));
        assert!(book.sheets[3].refresh_error && book.sheets[3].cells.is_empty());
        let found: Vec<(&str, Option<usize>, &str)> = book.names.iter().map(|n| (n.name.as_str(), n.sheet, n.refers_to.as_str())).collect();
        assert_eq!(found, [("Rate", None, "Sheet1!$A$1"), ("Twice", None, "Rate*2+Sheet1!$A$3"), ("Local", Some(2), "$B$2")]);
        // Too many cells is an error, not a giant link.
        assert!(book_from_source(&source(), &needs, &wb.external_books[0], 2).is_err());
    }

    #[test]
    fn breaking_a_link_renumbers_the_ones_after_it() {
        let mut wb = Workbook::new();
        let books = Arc::make_mut(&mut wb.external_books);
        for path in ["A.xlsx", "B.xlsx", "C.xlsx"] {
            books.push(ExternalBook { path: path.into(), ..Default::default() });
        }
        let cells = [("A1", "[1]S!A1"), ("A2", "[2]S!A1+[3]S!A1"), ("A3", "[3]!Rate"), ("A4", "SUM('[3]Jan:Dec'!A1)"), ("A5", "UsesB*2")];
        for (cell, f) in cells {
            if let Some(sh) = wb.sheet_mut(0) {
                let mut c = Cell::formula(Formula::new(f));
                c.value = Value::Number(7.0);
                sh.set_cell(CellRef::parse(cell).unwrap_or_default(), c);
            }
        }
        wb.names.push(name("UsesB", None, "[2]S!$A$1"));
        wb.names.push(name("KeepsA", None, "[1]S!$A$1"));
        let r = wb.break_links(&BTreeSet::from([1, 2]));
        // A1 reads [1]; A2 reads [2]; A5 reads [2] through UsesB.
        assert_eq!(r.formulas, 3);
        // [1] is still named by KeepsA; [2] only by UsesB, a name: both stay, as in Excel.
        assert_eq!((r.removed.clone(), r.kept.clone()), (vec![], vec![1, 2]));
        wb.names.clear();
        let r = wb.break_links(&BTreeSet::from([1, 2]));
        assert_eq!((r.formulas, r.removed), (0, vec![1, 2]));
        let text = |cell: &str| {
            wb.sheet(0).and_then(|s| s.cell(CellRef::parse(cell).unwrap_or_default())).and_then(|c| c.formula.as_ref()).map(|f| f.text.clone())
        };
        assert_eq!(text("A3").as_deref(), Some("[1]!Rate"));
        assert_eq!(text("A4").as_deref(), Some("SUM([1]Jan:Dec!A1)"));
        assert_eq!(text("A1"), None);
        assert_eq!(wb.external_books.iter().map(|b| b.path.as_str()).collect::<Vec<_>>(), ["C.xlsx"]);
    }

    #[test]
    fn change_source_onto_another_link_merges_them() {
        let mut wb = Workbook::new();
        let books = Arc::make_mut(&mut wb.external_books);
        for path in ["A.xlsx", "B.xlsx", "C.xlsx"] {
            books.push(ExternalBook { path: path.into(), ..Default::default() });
        }
        wb.names.push(name("N", None, "[1]S!A1+[2]S!A1+[3]S!A1"));
        wb.merge_link(1, 3);
        assert_eq!(wb.names[0].formula, "[2]S!A1+[1]S!A1+[2]S!A1");
        assert_eq!(wb.external_books.iter().map(|b| b.path.as_str()).collect::<Vec<_>>(), ["B.xlsx", "C.xlsx"]);
    }
}
