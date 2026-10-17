//! The workbook ↔ Yjs mapping (GridCraft schema 1) and the per-user undo.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use gridcraft_core::{CellRef, MAX_COLS, MAX_ROWS, RangeRef};
use gridcraft_model::{Cell, Formula, LineInfo, Sheet, Style, StyleId, StyleTable, Workbook};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value as Json;
use yrs::types::{Event, PathSegment};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{
    Any, Array, ArrayPrelim, ArrayRef, DeepObservable, Doc, In, Map, MapPrelim, MapRef, Observable, Options, Origin, Out, ReadTxn, StateVector,
    Transact, TransactionMut, UndoManager, Update, WriteTxn,
};

use crate::input::{self, Input};
use crate::{APP, CollabError, Rng, SCHEMA};

/// Transaction origins.
const LOCAL: &str = "gridcraft/local";
const REMOTE: &str = "gridcraft/remote";
const SEED: &str = "seed";

/// Row and column ids are added in blocks of this many when an edit goes past the last one.
const BLOCK: u32 = 64;
/// The sheet a document shows when the room has none (all deleted concurrently): its id is
/// fixed, so peers that recreate it independently create the same sheet.
pub(crate) const FALLBACK_SHEET: &str = "00000000";
/// Updates kept while they wait for earlier ones.
const MAX_HELD: usize = 1_000;
/// Longest sheet name (Excel's limit).
const MAX_SHEET_NAME: usize = 31;

/// Keys of a sheet map whose change means "read the whole sheet again".
const STRUCTURE_KEYS: &[&str] = &["rows", "cols", "cells", "rowProps", "colProps", "merges", "props"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Rows,
    Cols,
}

/// A row or column insert or delete a command made, so the ids move with the lines (instead of
/// every cell below being rewritten). Recorded by the engine, in order, per committed command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructOp {
    /// The sheet's name when the command ran.
    pub sheet: String,
    pub axis: Axis,
    pub at: u32,
    pub count: u32,
    pub insert: bool,
}

impl StructOp {
    /// Where line `p` (before the op) is after it; `None` when deleted.
    fn map(&self, p: u32) -> Option<u32> {
        if self.insert {
            if p >= self.at { p.checked_add(self.count) } else { Some(p) }
        } else if p < self.at {
            Some(p)
        } else if p < self.at.saturating_add(self.count) {
            None
        } else {
            Some(p - self.count)
        }
    }
}

/// The model after remote changes (or an undo): what changed, for recalculation and the
/// highlight of others' edits.
#[derive(Debug, PartialEq)]
pub struct Refreshed {
    pub wb: Workbook,
    /// Cells whose content changed (sheet index in `wb`, cell).
    pub changed: Vec<(usize, CellRef)>,
    /// Sheets, rows or columns changed: recalculate everything.
    pub structural: bool,
    /// The document chat changed (others' messages, a clear).
    pub chat: bool,
}

#[derive(Default, Debug)]
struct SheetChanges {
    full: bool,
    cells: HashSet<Arc<str>>,
    props: bool,
}

/// What remote transactions (and undos) changed since the last [`Replica::refresh`].
#[derive(Default, Debug)]
struct Changes {
    list: bool,
    /// New entries in `styles` (they can arrive after the cells that use them).
    styles: bool,
    book: bool,
    meta: bool,
    /// The document chat (`chat`, `chatMeta`).
    chat: bool,
    sheets: HashMap<String, SheetChanges>,
}

/// One axis's ids, in order, without duplicates.
#[derive(Clone, Debug, Default)]
struct Line {
    ids: Vec<Arc<str>>,
    ix: HashMap<Arc<str>, u32>,
    /// The block number the line grows by next (schema 1, "Ids"): the larger of the sheet's
    /// `rowBlocks`/`colBlocks` and one past the highest block present.
    next: u64,
}

impl Line {
    fn from_ids(all: impl IntoIterator<Item = Arc<str>>, limit: u32) -> Line {
        let mut l = Line::default();
        for id in all {
            if l.ids.len() as u64 >= u64::from(limit) {
                break;
            }
            if !l.ix.contains_key(&id) {
                l.ix.insert(id.clone(), l.ids.len() as u32);
                l.ids.push(id);
            }
        }
        l.next = l.highest_next();
        l
    }
    /// One past the highest block number present (0 without block ids).
    fn highest_next(&self) -> u64 {
        self.ids.iter().filter_map(|id| block_of(id)).max().map(|b| b.saturating_add(1)).unwrap_or(0)
    }
    /// The id of position `p`, or past the end, the deterministic block id it gets when the
    /// line grows to it (`~<next + k / 64>.<k % 64>` for the `k`-th position past the end).
    /// Awareness names positions with these without writing to the document.
    fn id_or_block(&self, p: u32) -> String {
        match self.id(p) {
            Some(id) => id.to_string(),
            None => {
                let k = u64::from(p.saturating_sub(self.len()));
                format!("~{}.{}", self.next.saturating_add(k / u64::from(BLOCK)), k % u64::from(BLOCK))
            }
        }
    }
    /// The position of `id`, or, for a block id the line hasn't grown to yet, the position it
    /// will have ([`Line::id_or_block`] reversed). `None` for unknown ids, positions at or past
    /// `limit` and blocks numbered below `next` that aren't there (deleted).
    fn pos_or_block(&self, id: &str, limit: u32) -> Option<u32> {
        if let Some(p) = self.pos(id) {
            return Some(p);
        }
        let (b, i) = block_parts(id)?;
        if i >= u64::from(BLOCK) || b < self.next {
            return None;
        }
        let k = (b - self.next).checked_mul(u64::from(BLOCK))?.checked_add(i)?;
        let p = u64::from(self.len()).checked_add(k)?;
        u32::try_from(p).ok().filter(|p| *p < limit)
    }
    fn len(&self) -> u32 {
        self.ids.len() as u32
    }
    fn id(&self, p: u32) -> Option<&Arc<str>> {
        self.ids.get(p as usize)
    }
    fn pos(&self, id: &str) -> Option<u32> {
        self.ix.get(id).copied()
    }
    fn reindex(&mut self) {
        self.ix = self.ids.iter().enumerate().map(|(i, id)| (id.clone(), i as u32)).collect();
        self.next = self.next.max(self.highest_next());
    }
}

#[derive(Clone, Debug)]
struct SheetIds {
    id: String,
    rows: Line,
    cols: Line,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn origin_is(txn: &TransactionMut, name: &str) -> bool {
    txn.origin().is_some_and(|o| *o == Origin::from(name))
}

/// Remote updates and undos change the model; our own commits and the seed already did.
fn recorded(txn: &TransactionMut) -> bool {
    !origin_is(txn, LOCAL) && !origin_is(txn, SEED)
}

fn root_name(id: &str) -> String {
    format!("sheet:{id}")
}

fn out_str(o: Option<Out>) -> Option<Arc<str>> {
    match o {
        Some(Out::Any(Any::String(s))) => Some(s),
        _ => None,
    }
}

fn get_json<T: DeserializeOwned, R: ReadTxn>(m: &MapRef, txn: &R, key: &str) -> Option<T> {
    out_str(m.get(txn, key)).and_then(|s| serde_json::from_str(&s).ok())
}

fn put_json<T: Serialize>(m: &MapRef, txn: &mut TransactionMut, key: &str, v: &T) {
    match serde_json::to_string(v) {
        Ok(s) => {
            m.insert(txn, key, s);
        }
        Err(e) => log::warn!("collab: can't encode {key}: {e}"),
    }
}

fn sub_map<R: ReadTxn>(root: &MapRef, txn: &R, key: &str) -> Option<MapRef> {
    match root.get(txn, key) {
        Some(Out::YMap(m)) => Some(m),
        _ => None,
    }
}

fn sub_array<R: ReadTxn>(root: &MapRef, txn: &R, key: &str) -> Option<ArrayRef> {
    match root.get(txn, key) {
        Some(Out::YArray(a)) => Some(a),
        _ => None,
    }
}

fn ensure_map(root: &MapRef, txn: &mut TransactionMut, key: &str) -> MapRef {
    match sub_map(root, txn, key) {
        Some(m) => m,
        None => root.insert(txn, key, MapPrelim::default()),
    }
}

fn ensure_array(root: &MapRef, txn: &mut TransactionMut, key: &str) -> ArrayRef {
    match sub_array(root, txn, key) {
        Some(a) => a,
        None => root.insert(txn, key, ArrayPrelim::default()),
    }
}

fn read_ids<R: ReadTxn>(a: Option<&ArrayRef>, txn: &R) -> Vec<Arc<str>> {
    let Some(a) = a else { return vec![] };
    a.iter(txn)
        .filter_map(|v| match v {
            Out::Any(Any::String(s)) => Some(s),
            _ => None,
        })
        .collect()
}

/// The Y index of the `p`-th distinct id of `a` (its length when `p` is past the end).
fn y_index<R: ReadTxn>(a: &ArrayRef, txn: &R, p: u32) -> u32 {
    let mut seen = HashSet::new();
    for (i, v) in a.iter(txn).enumerate() {
        if let Out::Any(Any::String(s)) = v
            && seen.insert(s)
            && seen.len() as u32 > p
        {
            return i as u32;
        }
    }
    a.len(txn)
}

/// Refuses an update that writes another app's or schema's `meta` (it's dropped, not applied:
/// the room goes on syncing). Only updates that mention `meta` are looked at, in a scratch
/// document.
fn foreign_meta(update: &[u8]) -> Result<(), CollabError> {
    if !update.windows(4).any(|w| w == b"meta") {
        return Ok(());
    }
    let Ok(u) = Update::decode_v1(update) else { return Ok(()) };
    let scratch = Doc::new();
    let applied = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut txn = scratch.transact_mut();
        txn.apply_update(u).is_ok()
    }));
    if !matches!(applied, Ok(true)) {
        return Ok(());
    }
    let txn = scratch.transact();
    let Some(meta) = txn.get_map("meta") else { return Ok(()) };
    let app = out_str(meta.get(&txn, "app"));
    let schema = match meta.get(&txn, "schema") {
        Some(Out::Any(Any::Number(n))) => n.as_f64().map(|f| f as u64),
        _ => None,
    };
    match (app, schema) {
        (Some(a), s) if &*a != APP || s.is_some_and(|s| s != u64::from(SCHEMA)) => {
            Err(CollabError::Mismatch { app: a.to_string(), schema: s.unwrap_or(0) })
        }
        (None, Some(s)) if s != u64::from(SCHEMA) => Err(CollabError::Mismatch { app: APP.into(), schema: s }),
        _ => Ok(()),
    }
}

/// The block number of a block id (`~12.5` → 12).
fn block_of(id: &str) -> Option<u64> {
    block_parts(id).map(|(b, _)| b)
}

/// A block id's block number and index (`~12.5` → (12, 5)).
fn block_parts(id: &str) -> Option<(u64, u64)> {
    let (b, i) = id.strip_prefix('~')?.split_once('.')?;
    Some((b.parse().ok()?, i.parse().ok()?))
}

/// A sheet's `rowBlocks`/`colBlocks` counter (0 when missing or out of range).
fn block_counter<R: ReadTxn>(root: &MapRef, txn: &R, key: &str) -> u64 {
    match root.get(txn, key) {
        Some(Out::Any(Any::Number(n))) => n.as_f64().filter(|x| *x >= 0.0 && *x < 1e9).map(|x| x as u64).unwrap_or(0),
        _ => 0,
    }
}

/// Removes every occurrence of the ids in `gone` from `a`.
fn remove_ids(a: &ArrayRef, txn: &mut TransactionMut, gone: &HashSet<Arc<str>>) {
    if gone.is_empty() {
        return;
    }
    let all: Vec<Option<Arc<str>>> = a
        .iter(txn)
        .map(|v| match v {
            Out::Any(Any::String(s)) => Some(s),
            _ => None,
        })
        .collect();
    for (i, v) in all.iter().enumerate().rev() {
        if v.as_ref().is_some_and(|s| gone.contains(s)) {
            a.remove(txn, i as u32);
        }
    }
}

/// A cell as schema 1 stores it.
#[derive(Clone, Debug, PartialEq)]
struct Enc {
    input: String,
    style: Option<String>,
    /// A legacy array formula's bottom-right cell, `<rowId>:<colId>`.
    array: Option<String>,
}

fn key_of(r: &str, c: &str) -> String {
    format!("{r}:{c}")
}

/// Content hash key of a style: `s` + 16 hex digits of SHA-256 of its JSON.
fn style_key(json: &str) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(json.as_bytes());
    let mut s = String::with_capacity(17);
    s.push('s');
    for b in d.iter().take(8) {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// The style of a line (row or column) with its style as a key.
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize, Default)]
struct LineProps {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    size: Option<f32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    hidden: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    outline: u8,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    collapsed: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    custom: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    style: Option<String>,
}

fn is_zero(v: &u8) -> bool {
    *v == 0
}

/// Sheet features stored as JSON text, last writer wins. `R`: a key of the sheet map itself
/// (the protocol's outline names them); `P`: a key of the sheet's `props` map.
macro_rules! sheet_fields {
    ($m:ident) => {
        $m!(R, "tables", tables);
        $m!(R, "charts", charts);
        $m!(R, "cf", cond_formats);
        $m!(R, "dv", validations);
        $m!(P, "defaultColWidth", default_col_width);
        $m!(P, "defaultRowHeight", default_row_height);
        $m!(P, "freeze", freeze);
        $m!(P, "tabColor", tab_color);
        $m!(P, "visibility", visibility);
        $m!(P, "showGridlines", show_gridlines);
        $m!(P, "showHeadings", show_headings);
        $m!(P, "showZeros", show_zeros);
        $m!(P, "rightToLeft", right_to_left);
        $m!(P, "autofilter", autofilter);
        $m!(P, "images", images);
        $m!(P, "shapes", shapes);
        $m!(P, "sparklines", sparklines);
        $m!(P, "slicers", slicers);
        $m!(P, "pivots", pivots);
        $m!(P, "protection", protection);
        $m!(P, "editRanges", edit_ranges);
        $m!(P, "print", print);
    };
}

/// Workbook settings in the `book` map, JSON text, last writer wins.
macro_rules! book_fields {
    ($m:ident) => {
        $m!("cellStyles", cell_styles);
        $m!("customLists", custom_lists);
        $m!("theme", theme);
        $m!("dateSystem", date_system);
        $m!("calc", calc);
        $m!("props", props);
        $m!("protectedStructure", protected_structure);
        $m!("structurePassword", structure_password);
    };
}

fn a1_pairs<V: Clone>(m: &BTreeMap<CellRef, V>) -> Vec<(String, V)> {
    m.iter().map(|(k, v)| (k.a1(), v.clone())).collect()
}

fn from_a1_pairs<V>(v: Option<Vec<(String, V)>>) -> BTreeMap<CellRef, V> {
    v.unwrap_or_default().into_iter().filter_map(|(k, v)| CellRef::parse(&k).map(|c| (c, v))).collect()
}

fn write_sheet_fields(txn: &mut TransactionMut, root: &MapRef, props: &MapRef, old: Option<&Sheet>, s: &Sheet) {
    macro_rules! w {
        (R, $k:literal, $f:ident) => {
            if old.is_none_or(|o| o.$f != s.$f) {
                put_json(root, txn, $k, &s.$f);
            }
        };
        (P, $k:literal, $f:ident) => {
            if old.is_none_or(|o| o.$f != s.$f) {
                put_json(props, txn, $k, &s.$f);
            }
        };
    }
    sheet_fields!(w);
    if old.is_none_or(|o| o.comments != s.comments) {
        put_json(props, txn, "comments", &a1_pairs(&s.comments));
    }
    if old.is_none_or(|o| o.hyperlinks != s.hyperlinks) {
        put_json(props, txn, "hyperlinks", &a1_pairs(&s.hyperlinks));
    }
    if old.is_none_or(|o| o.cell_pictures != s.cell_pictures) {
        put_json(props, txn, "cellPictures", &a1_pairs(&s.cell_pictures));
    }
}

fn read_sheet_fields<R: ReadTxn>(txn: &R, root: Option<&MapRef>, props: Option<&MapRef>, s: &mut Sheet) {
    let fresh = Sheet::new("");
    macro_rules! r {
        (R, $k:literal, $f:ident) => {
            s.$f = root.and_then(|r| get_json(r, txn, $k)).unwrap_or_else(|| fresh.$f.clone());
        };
        (P, $k:literal, $f:ident) => {
            s.$f = props.and_then(|p| get_json(p, txn, $k)).unwrap_or_else(|| fresh.$f.clone());
        };
    }
    sheet_fields!(r);
    s.comments = from_a1_pairs(props.and_then(|p| get_json(p, txn, "comments")));
    s.hyperlinks = from_a1_pairs(props.and_then(|p| get_json(p, txn, "hyperlinks")));
    s.cell_pictures = from_a1_pairs(props.and_then(|p| get_json(p, txn, "cellPictures")));
}

/// A valid, unique sheet name for `want` among `taken` (lower-cased).
fn unique_name(want: &str, taken: &HashSet<String>) -> String {
    let mut base: String =
        want.chars().filter(|c| !matches!(c, ':' | '\\' | '/' | '?' | '*' | '[' | ']') && !c.is_control()).take(MAX_SHEET_NAME).collect();
    base = base.trim_matches('\'').to_string();
    if base.trim().is_empty() {
        base = "Sheet".into();
    }
    if !taken.contains(&base.to_lowercase()) {
        return base;
    }
    for n in 2..10_000 {
        let suffix = format!(" ({n})");
        let keep = MAX_SHEET_NAME.saturating_sub(suffix.chars().count());
        let name: String = base.chars().take(keep).collect::<String>() + &suffix;
        if !taken.contains(&name.to_lowercase()) {
            return name;
        }
    }
    base
}

/// A shared workbook: the Yjs document, kept in step with the engine's model.
pub struct Replica {
    doc: Doc,
    client_id: u64,
    undo: UndoManager,
    changes: Arc<Mutex<Changes>>,
    outbox: Arc<Mutex<Vec<Vec<u8>>>>,
    /// Sheet roots observed and in the undo scope.
    observed: HashSet<String>,
    /// The model as last synced; `None` until the room's document is there (joining).
    base: Option<Arc<Workbook>>,
    /// Ids of `base`'s sheets, rows and columns, in its order.
    proj: Vec<SheetIds>,
    rng: Rng,
    /// Style keys known to be in `styles`.
    styles_written: HashSet<String>,
    /// Style keys read, parsed.
    styles_read: HashMap<String, Style>,
    /// Cached `(style id in base's table) → key`.
    keys_by_id: HashMap<StyleId, Option<String>>,
    /// Sheets with a cell or line whose style hasn't arrived yet (read again when it does).
    unresolved: HashSet<String>,
    /// The sheet being read (for `unresolved`).
    reading: String,
    /// Updates waiting for earlier ones ([`Replica::apply_update`]).
    held: Vec<Vec<u8>>,
}

impl Replica {
    /// An empty replica with Yjs client id `client_id` (below 2^53).
    pub fn new(client_id: u64) -> Replica {
        let client_id = client_id & ((1u64 << 53) - 1);
        // Garbage collection off: with it, yrs 0.28 can free items the undo manager and later
        // encodings still point at (a crash seen in the convergence fuzz).
        let mut options = Options::with_client_id(yrs::block::ClientID::new(client_id));
        options.skip_gc = true;
        let doc = Doc::with_options(options);
        let changes: Arc<Mutex<Changes>> = Default::default();
        let outbox: Arc<Mutex<Vec<Vec<u8>>>> = Default::default();
        let ob = outbox.clone();
        if let Err(e) = doc.observe_update_v1("gridcraft/out", move |txn, e| {
            if !origin_is(txn, REMOTE) {
                lock(&ob).push(e.update.clone());
            }
        }) {
            log::warn!("collab: can't observe updates: {e}");
        }
        let sheets = doc.get_or_insert_array("sheets");
        let book = doc.get_or_insert_map("book");
        let meta = doc.get_or_insert_map("meta");
        let styles = doc.get_or_insert_map("styles");
        let ch = changes.clone();
        styles.observe("gridcraft/styles", move |txn, _| {
            if !origin_is(txn, LOCAL) {
                lock(&ch).styles = true;
            }
        });
        let ch = changes.clone();
        sheets.observe("gridcraft/sheets", move |txn, _| {
            if recorded(txn) {
                lock(&ch).list = true;
            }
        });
        let names = doc.get_or_insert_map("names");
        let ch = changes.clone();
        names.observe_deep("gridcraft/names", move |txn, _| {
            if recorded(txn) {
                lock(&ch).book = true;
            }
        });
        let ch = changes.clone();
        book.observe_deep("gridcraft/book", move |txn, _| {
            if recorded(txn) {
                lock(&ch).book = true;
            }
        });
        let ch = changes.clone();
        meta.observe("gridcraft/meta", move |txn, _| {
            if recorded(txn) {
                lock(&ch).meta = true;
            }
        });
        // The chat isn't in the undo scope: Undo never touches it.
        let ch = changes.clone();
        doc.get_or_insert_array(crate::chat::ROOT).observe_deep("gridcraft/chat", move |txn, _| {
            if recorded(txn) {
                lock(&ch).chat = true;
            }
        });
        let ch = changes.clone();
        doc.get_or_insert_map(crate::chat::META).observe("gridcraft/chatMeta", move |txn, _| {
            if recorded(txn) {
                lock(&ch).chat = true;
            }
        });
        let options = yrs::undo::Options {
            capture_timeout_millis: 0,
            tracked_origins: [Origin::from(LOCAL)].into_iter().collect(),
            capture_transaction: None,
            timestamp: Arc::new(|| 0u64),
            init_undo_stack: vec![],
            init_redo_stack: vec![],
        };
        let mut undo = UndoManager::with_options(options);
        undo.expand_scope(&doc, &sheets);
        undo.expand_scope(&doc, &book);
        undo.expand_scope(&doc, &names);
        Replica {
            doc,
            client_id,
            undo,
            changes,
            outbox,
            observed: HashSet::new(),
            base: None,
            proj: vec![],
            rng: Rng::new(client_id),
            styles_written: HashSet::new(),
            styles_read: HashMap::new(),
            keys_by_id: HashMap::new(),
            unresolved: HashSet::new(),
            reading: String::new(),
            held: Vec::new(),
        }
    }

    pub fn client_id(&self) -> u64 {
        self.client_id
    }

    /// The room's document is here (always after [`Replica::seed`]; after joining, once a peer's
    /// state arrived).
    pub fn is_ready(&self) -> bool {
        self.base.is_some()
    }

    /// The workbook as last synced.
    pub fn base(&self) -> Option<&Arc<Workbook>> {
        self.base.as_ref()
    }

    /// Takes `wb` as the synced workbook: the same shared content as the current one, after
    /// local-only work (recalculation, view state).
    pub fn rebase(&mut self, wb: Arc<Workbook>) {
        if self.base.is_some() {
            self.base = Some(wb);
        }
    }

    /// Checks `meta`: `Ok(true)` ours, `Ok(false)` not there yet, an error for another app or
    /// schema.
    pub fn check_meta(&self) -> Result<bool, CollabError> {
        let txn = self.doc.transact();
        let Some(meta) = txn.get_map("meta") else { return Ok(false) };
        let app = out_str(meta.get(&txn, "app"));
        let schema = match meta.get(&txn, "schema") {
            Some(Out::Any(Any::Number(n))) => n.as_f64().map(|f| f as u64),
            _ => None,
        };
        match (app, schema) {
            (None, None) => Ok(false),
            (Some(a), Some(s)) if &*a == APP && s == u64::from(SCHEMA) => Ok(true),
            (a, s) => Err(CollabError::Mismatch { app: a.map(|a| a.to_string()).unwrap_or_else(|| "another app".into()), schema: s.unwrap_or(0) }),
        }
    }

    /// The room's title (`meta.title`).
    pub fn title(&self) -> Option<String> {
        let txn = self.doc.transact();
        txn.get_map("meta").and_then(|m| out_str(m.get(&txn, "title"))).map(|s| s.to_string())
    }

    fn take_outbox(&self) -> Result<Option<Vec<u8>>, CollabError> {
        let ups: Vec<Vec<u8>> = std::mem::take(&mut *lock(&self.outbox));
        match ups.len() {
            0 => Ok(None),
            1 => Ok(ups.into_iter().next()),
            _ => yrs::merge_updates_v1(ups.iter().map(|u| u.as_slice()).collect::<Vec<_>>())
                .map(Some)
                .map_err(|e| CollabError::Other(format!("merging updates: {e}"))),
        }
    }

    /// Observes a sheet's root (remote changes) and adds it to the undo scope.
    fn observe_sheet(&mut self, id: &str) {
        if self.observed.contains(id) {
            return;
        }
        self.observed.insert(id.to_string());
        let root = self.doc.get_or_insert_map(root_name(id));
        let ch = self.changes.clone();
        let sid = id.to_string();
        root.observe_deep("gridcraft/sheet", move |txn, events| {
            if !recorded(txn) {
                return;
            }
            let mut ch = lock(&ch);
            let sc = ch.sheets.entry(sid.clone()).or_default();
            for e in events.iter() {
                let path: Vec<PathSegment> = e.path().into_iter().collect();
                let first = match path.first() {
                    Some(PathSegment::Key(k)) => Some(k.clone()),
                    _ => None,
                };
                match (first.as_deref(), path.len()) {
                    (None, _) => {
                        if let Event::Map(m) = e {
                            for k in m.keys(txn).keys() {
                                if STRUCTURE_KEYS.contains(&k.as_ref()) {
                                    sc.full = true;
                                } else {
                                    sc.props = true;
                                }
                            }
                        } else {
                            sc.full = true;
                        }
                    }
                    (Some("cells"), 1) => {
                        if let Event::Map(m) = e {
                            sc.cells.extend(m.keys(txn).keys().cloned());
                        } else {
                            sc.full = true;
                        }
                    }
                    (Some("cells"), _) => match path.get(1) {
                        Some(PathSegment::Key(k)) => {
                            sc.cells.insert(k.clone());
                        }
                        _ => sc.full = true,
                    },
                    (Some("props"), _) => sc.props = true,
                    _ => sc.full = true,
                }
            }
        });
        self.undo.expand_scope(&self.doc, &root);
    }

    // ------------------------------------------------------------ sharing and joining

    /// Shares `wb`: builds the shared document from it (one transaction, origin `seed`). Returns
    /// the update (the whole document).
    pub fn seed(&mut self, wb: Arc<Workbook>, title: &str) -> Result<Vec<u8>, CollabError> {
        let doc = self.doc.clone();
        let mut proj = Vec::new();
        {
            let mut txn = doc.transact_mut_with(SEED);
            let meta = txn.get_or_insert_map("meta");
            meta.insert(&mut txn, "app", APP);
            meta.insert(&mut txn, "schema", i64::from(SCHEMA));
            meta.insert(&mut txn, "title", title);
            let book = txn.get_or_insert_map("book");
            self.write_book(&mut txn, &book, None, &wb);
            let sheets = txn.get_or_insert_array("sheets");
            for sh in &wb.sheets {
                let ids = self.write_new_sheet(&mut txn, &wb, sh)?;
                sheets.push_back(&mut txn, ids.id.as_str());
                proj.push(ids);
            }
            crate::chat::seed(&mut txn, &wb.chat);
        }
        for ids in &proj {
            self.observe_sheet(&ids.id.clone());
        }
        self.proj = proj;
        self.base = Some(wb);
        lock(&self.changes).sheets.clear();
        Ok(self.take_outbox()?.unwrap_or_default())
    }

    fn style_key_of(&mut self, txn: &mut TransactionMut, wb: &Workbook, id: StyleId) -> Option<String> {
        if id == StyleId::DEFAULT {
            return None;
        }
        let style = wb.styles.get(id);
        if *style == *wb.styles.get(StyleId::DEFAULT) {
            return None;
        }
        let json = serde_json::to_string(style).ok()?;
        let key = style_key(&json);
        if !self.styles_written.contains(&key) {
            let styles = txn.get_or_insert_map("styles");
            if !styles.contains_key(txn, &key) {
                styles.insert(txn, key.as_str(), json);
            }
            self.styles_written.insert(key.clone());
        }
        Some(key)
    }

    /// Ensures line ids up to position `p` exist: whole blocks of 64 deterministic ids
    /// (`~<block>.<i>`) appended at the end, numbered from the larger of the sheet's
    /// `rowBlocks`/`colBlocks` counter and one past the highest block present, then the counter
    /// is set. Two people growing a sheet at once append the same ids (schema 1, "Ids").
    fn ensure_line(&mut self, txn: &mut TransactionMut, root: &MapRef, arr: &ArrayRef, line: &mut Line, p: u32, limit: u32) -> bool {
        if p < line.len() {
            return true;
        }
        if p >= limit {
            return false;
        }
        let counter_key = if limit == MAX_ROWS { "rowBlocks" } else { "colBlocks" };
        let counter = block_counter(root, txn, counter_key);
        let highest = line.ids.iter().filter_map(|id| block_of(id)).max().map(|b| b + 1).unwrap_or(0);
        let mut next = counter.max(highest);
        let mut new = Vec::new();
        while line.len() <= p {
            for i in 0..BLOCK {
                let id: Arc<str> = Arc::from(format!("~{next}.{i}"));
                if !line.ix.contains_key(&id) {
                    line.ix.insert(id.clone(), line.ids.len() as u32);
                    line.ids.push(id.clone());
                }
                new.push(Any::String(id));
            }
            next += 1;
        }
        let at = arr.len(txn);
        arr.insert_range(txn, at, new);
        root.insert(txn, counter_key, next as i64);
        line.next = next;
        true
    }

    fn encode_cell(&mut self, txn: &mut TransactionMut, wb: &Workbook, sheet: &mut SheetW, pos: CellRef, cell: &Cell) -> Enc {
        let style = self.cached_style_key(txn, wb, cell.style);
        let array = cell.formula.as_ref().and_then(|f| f.array).filter(|a| a.start == pos && a.end != a.start).and_then(|a| {
            let ok = self.ensure_line(txn, &sheet.root, &sheet.rows_arr, &mut sheet.ids.rows, a.end.row, MAX_ROWS)
                && self.ensure_line(txn, &sheet.root, &sheet.cols_arr, &mut sheet.ids.cols, a.end.col, MAX_COLS);
            ok.then(|| Some(key_of(sheet.ids.rows.id(a.end.row)?, sheet.ids.cols.id(a.end.col)?))).flatten()
        });
        Enc { input: input::encode(cell), style, array }
    }

    fn cached_style_key(&mut self, txn: &mut TransactionMut, wb: &Workbook, id: StyleId) -> Option<String> {
        if let Some(k) = self.keys_by_id.get(&id) {
            return k.clone();
        }
        let k = self.style_key_of(txn, wb, id);
        self.keys_by_id.insert(id, k.clone());
        k
    }

    /// The encoding of a cell without writing anything (comparison only): input and style.
    fn compare_key<'a>(wb: &'a Workbook, cell: &Cell) -> (String, &'a Style, Option<RangeRef>) {
        (input::encode(cell), wb.styles.get(cell.style), cell.formula.as_ref().and_then(|f| f.array))
    }

    fn write_cell(txn: &mut TransactionMut, cells: &MapRef, key: &str, enc: &Enc) {
        if let Some(Out::YMap(m)) = cells.get(txn, key) {
            if out_str(m.get(txn, "input")).as_deref() != Some(enc.input.as_str()) {
                m.insert(txn, "input", enc.input.as_str());
            }
            for (k, v) in [("style", &enc.style), ("array", &enc.array)] {
                let have = out_str(m.get(txn, k));
                match v {
                    Some(v) if have.as_deref() != Some(v.as_str()) => {
                        m.insert(txn, k, v.as_str());
                    }
                    None if m.contains_key(txn, k) => {
                        m.remove(txn, k);
                    }
                    _ => {}
                }
            }
            return;
        }
        let mut fields: Vec<(&str, In)> = vec![("input", In::Any(Any::from(enc.input.as_str())))];
        if let Some(s) = &enc.style {
            fields.push(("style", In::Any(Any::from(s.as_str()))));
        }
        if let Some(a) = &enc.array {
            fields.push(("array", In::Any(Any::from(a.as_str()))));
        }
        cells.insert(txn, key, MapPrelim::from_iter(fields));
    }

    fn line_props(&mut self, txn: &mut TransactionMut, wb: &Workbook, info: &LineInfo) -> LineProps {
        LineProps {
            size: info.size,
            hidden: info.hidden,
            outline: info.outline,
            collapsed: info.collapsed,
            custom: info.custom,
            style: info.style.and_then(|s| self.cached_style_key(txn, wb, s)),
        }
    }

    /// Writes a sheet that isn't in the room yet; returns its ids.
    fn write_new_sheet(&mut self, txn: &mut TransactionMut, wb: &Workbook, sh: &Sheet) -> Result<SheetIds, CollabError> {
        let mut id = self.rng.id();
        while txn.get_map(root_name(&id).as_str()).is_some_and(|m| m.len(txn) > 0) {
            id = self.rng.id();
        }
        self.write_sheet_as(txn, wb, sh, id)
    }

    fn write_sheet_as(&mut self, txn: &mut TransactionMut, wb: &Workbook, sh: &Sheet, id: String) -> Result<SheetIds, CollabError> {
        let root = txn.get_or_insert_map(root_name(&id).as_str());
        root.insert(txn, "name", sh.name.as_str());
        let rows_arr: ArrayRef = root.insert(txn, "rows", ArrayPrelim::default());
        let cols_arr: ArrayRef = root.insert(txn, "cols", ArrayPrelim::default());
        let cells: MapRef = root.insert(txn, "cells", MapPrelim::default());
        let row_props: MapRef = root.insert(txn, "rowProps", MapPrelim::default());
        let col_props: MapRef = root.insert(txn, "colProps", MapPrelim::default());
        let merges: MapRef = root.insert(txn, "merges", MapPrelim::default());
        let props: MapRef = root.insert(txn, "props", MapPrelim::default());
        // Extent: every row and column with something in it, and a margin to type into.
        // Extent: every row and column with something in it, in whole blocks.
        let used = sh.cells.used_range();
        let mut last_row = used.map(|u| u.end.row);
        let mut last_col = used.map(|u| u.end.col);
        let max = |a: Option<u32>, b: Option<u32>| match (a, b) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        last_row = max(last_row, sh.rows.keys().next_back().copied());
        last_col = max(last_col, sh.cols.keys().next_back().copied());
        for m in &sh.merges {
            last_row = max(last_row, Some(m.end.row));
            last_col = max(last_col, Some(m.end.col));
        }
        let mut w = SheetW { root: root.clone(), ids: SheetIds { id: id.clone(), rows: Line::default(), cols: Line::default() }, rows_arr, cols_arr };
        if let Some(r) = last_row {
            self.ensure_line(txn, &w.root.clone(), &w.rows_arr.clone(), &mut w.ids.rows, r, MAX_ROWS);
        }
        if let Some(c) = last_col {
            self.ensure_line(txn, &w.root.clone(), &w.cols_arr.clone(), &mut w.ids.cols, c, MAX_COLS);
        }
        for (pos, cell) in sh.cells.iter() {
            if cell.is_blank() {
                continue;
            }
            let enc = self.encode_cell(txn, wb, &mut w, pos, cell);
            if let (Some(r), Some(c)) = (w.ids.rows.id(pos.row), w.ids.cols.id(pos.col)) {
                Self::write_cell(txn, &cells, &key_of(r, c), &enc);
            }
        }
        for (axis_rows, map, target) in [(true, &sh.rows, &row_props), (false, &sh.cols, &col_props)] {
            for (p, info) in map {
                let lp = self.line_props(txn, wb, info);
                let line = if axis_rows { &w.ids.rows } else { &w.ids.cols };
                if let Some(id) = line.id(*p) {
                    put_json(target, txn, &id.clone(), &lp);
                }
            }
        }
        for m in &sh.merges {
            if let Some((k, v)) = merge_entry(&w.ids, m) {
                merges.insert(txn, k.as_str(), v);
            }
        }
        write_sheet_fields(txn, &root, &props, None, sh);
        Ok(w.ids)
    }

    fn write_book(&mut self, txn: &mut TransactionMut, book: &MapRef, old: Option<&Workbook>, wb: &Workbook) {
        macro_rules! w {
            ($k:literal, $f:ident) => {
                if old.is_none_or(|o| o.$f != wb.$f) {
                    put_json(book, txn, $k, &wb.$f);
                }
            };
        }
        book_fields!(w);
        let normal = wb.styles.get(StyleId::DEFAULT);
        if old.is_none_or(|o| o.styles.get(StyleId::DEFAULT) != normal) {
            put_json(book, txn, "normal", normal);
        }
        if old.is_none_or(|o| o.names != wb.names) {
            let names = txn.get_or_insert_map("names");
            let want: HashMap<String, String> = wb.names.iter().filter_map(|n| Some((name_key(n), serde_json::to_string(n).ok()?))).collect();
            let have: Vec<(String, Option<Arc<str>>)> = names.iter(txn).map(|(k, v)| (k.to_string(), out_str(Some(v)))).collect();
            for (k, v) in &have {
                match want.get(k) {
                    Some(w) if v.as_deref() == Some(w.as_str()) => {}
                    Some(_) => {}
                    None => {
                        names.remove(txn, k);
                    }
                }
            }
            for (k, v) in want {
                if have.iter().any(|(h, hv)| *h == k && hv.as_deref() == Some(v.as_str())) {
                    continue;
                }
                names.insert(txn, k.as_str(), v);
            }
        }
    }

    // ------------------------------------------------------------ local edits

    /// Turns the change from the last synced workbook to `cur` into one transaction with the
    /// local origin (one undo step). `ops` are the row and column inserts and deletes the
    /// command made, in order. Returns the update to send, `None` when nothing shared changed.
    pub fn commit(&mut self, cur: &Arc<Workbook>, ops: &[StructOp]) -> Result<Option<Vec<u8>>, CollabError> {
        let Some(base) = self.base.clone() else { return Err(CollabError::NotReady) };
        if Arc::ptr_eq(&base, cur) {
            return Ok(None);
        }
        // Style ids index the workbook's own table: a fresh lookup per commit.
        self.keys_by_id.clear();
        let doc = self.doc.clone();
        let mut added_ids = Vec::new();
        {
            let mut txn = doc.transact_mut_with(LOCAL);
            let matches = match_sheets(&base, cur);
            let mut new_proj: Vec<SheetIds> = Vec::with_capacity(cur.sheets.len());
            let mut kept: HashSet<usize> = HashSet::new();
            for (i, m) in matches.iter().enumerate() {
                match m.and_then(|j| self.proj.get(j).map(|p| (j, p.clone()))) {
                    Some((j, ids)) => {
                        kept.insert(j);
                        new_proj.push(ids);
                    }
                    None => {
                        let Some(sh) = cur.sheets.get(i) else { continue };
                        let ids = self.write_new_sheet(&mut txn, cur, sh)?;
                        added_ids.push(ids.id.clone());
                        new_proj.push(ids);
                    }
                }
            }
            // The sheet list: removed ids out, then the order of `cur`.
            let sheets = txn.get_or_insert_array("sheets");
            let gone: HashSet<Arc<str>> =
                self.proj.iter().enumerate().filter(|(j, _)| !kept.contains(j)).map(|(_, p)| Arc::from(p.id.as_str())).collect();
            remove_ids(&sheets, &mut txn, &gone);
            let desired: Vec<Arc<str>> = new_proj.iter().map(|p| Arc::from(p.id.as_str())).collect();
            reorder(&sheets, &mut txn, &desired);
            // Each kept sheet: its own changes.
            let normal_changed = base.styles.get(StyleId::DEFAULT) != cur.styles.get(StyleId::DEFAULT);
            for (i, m) in matches.iter().enumerate() {
                let (Some(j), Some(after)) = (*m, cur.sheets.get(i)) else { continue };
                let Some(before) = base.sheets.get(j) else { continue };
                if Arc::ptr_eq(before, after) && !normal_changed {
                    continue;
                }
                let sheet_ops: Vec<&StructOp> = ops.iter().filter(|o| o.sheet == after.name || o.sheet == before.name).collect();
                let Some(ids) = new_proj.get_mut(i) else { continue };
                self.diff_sheet(&mut txn, &base, cur, before, after, ids, &sheet_ops);
            }
            let book = txn.get_or_insert_map("book");
            self.write_book(&mut txn, &book, Some(&base), cur);
            if !Arc::ptr_eq(&base.chat, &cur.chat) {
                crate::chat::write_diff(&mut txn, &base.chat, &cur.chat);
            }
            self.proj = new_proj;
        }
        for id in added_ids {
            self.observe_sheet(&id);
        }
        self.base = Some(cur.clone());
        self.undo.reset();
        self.take_outbox()
    }

    /// Shares only the chat's change (a viewer's messages: their other changes never leave).
    /// The synced workbook takes `chat`. Returns the update to send.
    pub fn commit_chat(&mut self, chat: &Arc<gridcraft_model::Chat>) -> Result<Option<Vec<u8>>, CollabError> {
        let Some(base) = self.base.clone() else { return Err(CollabError::NotReady) };
        if Arc::ptr_eq(&base.chat, chat) || *base.chat == **chat {
            return Ok(None);
        }
        {
            let doc = self.doc.clone();
            let mut txn = doc.transact_mut_with(LOCAL);
            crate::chat::write_diff(&mut txn, &base.chat, chat);
        }
        let mut wb = (*base).clone();
        wb.chat = chat.clone();
        self.base = Some(Arc::new(wb));
        self.take_outbox()
    }

    /// The chat in the shared document.
    pub fn chat(&self) -> gridcraft_model::Chat {
        crate::chat::read(&self.doc.transact())
    }

    #[allow(clippy::too_many_arguments)]
    fn diff_sheet(
        &mut self,
        txn: &mut TransactionMut,
        bwb: &Workbook,
        cwb: &Workbook,
        before: &Sheet,
        after: &Sheet,
        ids: &mut SheetIds,
        ops: &[&StructOp],
    ) {
        let root = txn.get_or_insert_map(root_name(&ids.id).as_str());
        if before.name != after.name {
            root.insert(txn, "name", after.name.as_str());
        }
        let rows_arr = ensure_array(&root, txn, "rows");
        let cols_arr = ensure_array(&root, txn, "cols");
        let cells = ensure_map(&root, txn, "cells");
        let props = ensure_map(&root, txn, "props");
        write_sheet_fields(txn, &root, &props, Some(before), after);

        // Rows and columns inserted and deleted: the ids move with the lines.
        let old_ids = ids.clone();
        let mut deleted_rows: HashSet<Arc<str>> = HashSet::new();
        let mut deleted_cols: HashSet<Arc<str>> = HashSet::new();
        for op in ops {
            let (line, arr, deleted, limit) = match op.axis {
                Axis::Rows => (&mut ids.rows, &rows_arr, &mut deleted_rows, MAX_ROWS),
                Axis::Cols => (&mut ids.cols, &cols_arr, &mut deleted_cols, MAX_COLS),
            };
            if op.insert {
                if op.at < line.len() && op.count > 0 {
                    let yi = y_index(arr, txn, op.at);
                    let mut new = Vec::new();
                    let mut fresh: Vec<Arc<str>> = Vec::new();
                    while fresh.len() < op.count.min(limit) as usize {
                        let id: Arc<str> = Arc::from(self.rng.id());
                        if line.ix.contains_key(&id) || fresh.contains(&id) {
                            continue;
                        }
                        fresh.push(id.clone());
                        new.push(Any::String(id));
                    }
                    arr.insert_range(txn, yi, new);
                    let at = op.at as usize;
                    line.ids.splice(at..at, fresh);
                    line.ids.truncate(limit as usize);
                    line.reindex();
                }
            } else {
                let end = op.at.saturating_add(op.count).min(line.len());
                if op.at < end {
                    let gone: HashSet<Arc<str>> = line.ids.drain(op.at as usize..end as usize).collect();
                    remove_ids(arr, txn, &gone);
                    deleted.extend(gone);
                    line.reindex();
                }
            }
        }
        let structural = !ops.is_empty();
        let map_row = |p: u32| ops.iter().filter(|o| o.axis == Axis::Rows).try_fold(p, |p, o| o.map(p));
        let map_col = |p: u32| ops.iter().filter(|o| o.axis == Axis::Cols).try_fold(p, |p, o| o.map(p));

        let mut w = SheetW { root: root.clone(), ids: ids.clone(), rows_arr: rows_arr.clone(), cols_arr: cols_arr.clone() };
        // Cells.
        if structural {
            // Where each of `before`'s cells is now.
            let mut moved: HashMap<CellRef, &Cell> = HashMap::new();
            for (pos, cell) in before.cells.iter() {
                match (map_row(pos.row), map_col(pos.col)) {
                    (Some(r), Some(c)) => {
                        moved.insert(CellRef::new(r, c), cell);
                    }
                    _ => {
                        // In a deleted line: its key goes.
                        if let (Some(r), Some(c)) = (old_ids.rows.id(pos.row), old_ids.cols.id(pos.col)) {
                            cells.remove(txn, &key_of(r, c));
                        }
                    }
                }
            }
            for (pos, cell) in after.cells.iter() {
                let old = moved.remove(&pos);
                if old.is_some_and(|o| Self::compare_key(bwb, o) == Self::compare_key(cwb, cell)) {
                    continue;
                }
                self.put_cell(txn, cwb, &mut w, &cells, pos, Some(cell));
            }
            for pos in moved.into_keys() {
                self.put_cell(txn, cwb, &mut w, &cells, pos, None);
            }
        } else {
            for pos in before.cells.diff(&after.cells) {
                let (a, b) = (before.cells.get(pos), after.cells.get(pos));
                let same = match (a, b) {
                    (Some(a), Some(b)) => Self::compare_key(bwb, a) == Self::compare_key(cwb, b),
                    (None, None) => true,
                    _ => false,
                };
                if !same {
                    self.put_cell(txn, cwb, &mut w, &cells, pos, b);
                }
            }
            // A changed style table (the Normal style, say) changes cells that kept their ids.
            if bwb.styles.get(StyleId::DEFAULT) != cwb.styles.get(StyleId::DEFAULT) {
                for (pos, cell) in after.cells.iter() {
                    if before.cells.get(pos).is_some_and(|b| Self::compare_key(bwb, b) != Self::compare_key(cwb, cell)) {
                        self.put_cell(txn, cwb, &mut w, &cells, pos, Some(cell));
                    }
                }
            }
        }
        // Row and column properties.
        for rows in [true, false] {
            let (bm, am) = if rows { (&before.rows, &after.rows) } else { (&before.cols, &after.cols) };
            if !structural && bm == am && bwb.styles.get(StyleId::DEFAULT) == cwb.styles.get(StyleId::DEFAULT) {
                continue;
            }
            let target = ensure_map(&root, txn, if rows { "rowProps" } else { "colProps" });
            let mut moved: HashMap<u32, (LineInfo, Option<&Style>)> = HashMap::new();
            for (p, info) in bm {
                let np = if rows { map_row(*p) } else { map_col(*p) };
                if let Some(np) = np {
                    moved.insert(np, line_cmp(bwb, info));
                } else {
                    let line = if rows { &old_ids.rows } else { &old_ids.cols };
                    if let Some(id) = line.id(*p) {
                        target.remove(txn, id);
                    }
                }
            }
            for (p, info) in am {
                if moved.remove(p) == Some(line_cmp(cwb, info)) {
                    continue;
                }
                let lp = self.line_props(txn, cwb, info);
                let ok = if rows {
                    self.ensure_line(txn, &root, &rows_arr, &mut w.ids.rows, *p, MAX_ROWS)
                } else {
                    self.ensure_line(txn, &root, &cols_arr, &mut w.ids.cols, *p, MAX_COLS)
                };
                let line = if rows { &w.ids.rows } else { &w.ids.cols };
                if let (true, Some(id)) = (ok, line.id(*p)) {
                    put_json(&target, txn, &id.clone(), &lp);
                }
            }
            for p in moved.into_keys() {
                let line = if rows { &w.ids.rows } else { &w.ids.cols };
                if let Some(id) = line.id(p) {
                    target.remove(txn, id);
                }
            }
        }
        // Merges: compared with the room's own entries.
        if structural || before.merges != after.merges {
            let merges = ensure_map(&root, txn, "merges");
            let mut want: HashMap<String, String> = HashMap::new();
            for m in &after.merges {
                let ok = self.ensure_line(txn, &root, &rows_arr, &mut w.ids.rows, m.end.row, MAX_ROWS)
                    && self.ensure_line(txn, &root, &cols_arr, &mut w.ids.cols, m.end.col, MAX_COLS);
                if ok && let Some((k, v)) = merge_entry(&w.ids, m) {
                    want.insert(k, v);
                }
            }
            let have: Vec<(String, Option<Arc<str>>)> = merges.iter(txn).map(|(k, v)| (k.to_string(), out_str(Some(v)))).collect();
            for (k, v) in &have {
                match want.remove(k) {
                    Some(w) if v.as_deref() == Some(w.as_str()) => {}
                    Some(w) => {
                        merges.insert(txn, k.as_str(), w);
                    }
                    None => {
                        merges.remove(txn, k);
                    }
                }
            }
            for (k, v) in want {
                merges.insert(txn, k.as_str(), v);
            }
        }
        *ids = w.ids;
    }

    /// Writes (or removes, `cell` `None` or blank) the cell at `pos`.
    fn put_cell(&mut self, txn: &mut TransactionMut, wb: &Workbook, w: &mut SheetW, cells: &MapRef, pos: CellRef, cell: Option<&Cell>) {
        match cell.filter(|c| !c.is_blank()) {
            Some(cell) => {
                let ok = self.ensure_line(txn, &w.root.clone(), &w.rows_arr.clone(), &mut w.ids.rows, pos.row, MAX_ROWS)
                    && self.ensure_line(txn, &w.root.clone(), &w.cols_arr.clone(), &mut w.ids.cols, pos.col, MAX_COLS);
                if !ok {
                    return;
                }
                let enc = self.encode_cell(txn, wb, w, pos, cell);
                if let (Some(r), Some(c)) = (w.ids.rows.id(pos.row), w.ids.cols.id(pos.col)) {
                    Self::write_cell(txn, cells, &key_of(r, c), &enc);
                }
            }
            None => {
                if let (Some(r), Some(c)) = (w.ids.rows.id(pos.row), w.ids.cols.id(pos.col)) {
                    cells.remove(txn, &key_of(r, c));
                }
            }
        }
    }

    // ------------------------------------------------------------ remote changes

    /// Applies a remote update (sync step 2 or an incremental update) to the shared document.
    /// The model follows at the next [`Replica::refresh`]. An update that skips some of a peer's
    /// changes (they were lost, or arrive later) waits until the gap is filled, by a later update
    /// or a sync step 2 (see [`Replica::has_gaps`]): applied early, its changes inside existing
    /// shared types wouldn't be reported.
    pub fn apply_update(&mut self, update: &[u8]) -> Result<(), CollabError> {
        if update.len() > crate::MAX_UPDATE {
            return Err(CollabError::TooLarge(update.len()));
        }
        let u = Update::decode_v1(update).map_err(|e| CollabError::Corrupt(e.to_string()))?;
        foreign_meta(update)?;
        if self.gap(&u) {
            if self.held.len() >= MAX_HELD {
                self.held.remove(0);
            }
            self.held.push(update.to_vec());
            return Ok(());
        }
        self.integrate(u)?;
        // Updates that waited for this one.
        loop {
            let ready = self.held.iter().position(|h| Update::decode_v1(h).is_ok_and(|u| !self.gap(&u)));
            let Some(i) = ready else { break };
            let h = self.held.remove(i);
            if let Ok(u) = Update::decode_v1(&h)
                && let Err(e) = self.integrate(u)
            {
                log::warn!("collab: a held update failed: {e}");
            }
        }
        Ok(())
    }

    /// Whether `u` starts past what we have of one of its clients.
    fn gap(&self, u: &Update) -> bool {
        let have = self.doc.transact().state_vector();
        u.state_vector_lower().iter().any(|(c, clock)| *clock > have.get(c))
    }

    fn integrate(&mut self, u: Update) -> Result<(), CollabError> {
        let doc = self.doc.clone();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut txn = doc.transact_mut_with(REMOTE);
            txn.apply_update(u).map_err(|e| CollabError::Corrupt(e.to_string()))
        }));
        match r {
            Ok(r) => r,
            Err(_) => Err(CollabError::Corrupt("the update couldn't be applied".into())),
        }
    }

    /// Some updates wait for others that haven't arrived: ask the peers for them (sync step 1).
    pub fn has_gaps(&self) -> bool {
        !self.held.is_empty()
    }

    /// Whether remote changes (or an undo) are waiting for [`Replica::refresh`].
    pub fn has_changes(&self) -> bool {
        let ch = lock(&self.changes);
        ch.list || ch.book || ch.meta || ch.styles || ch.chat || !ch.sheets.is_empty() || (self.base.is_none())
    }

    /// Brings remote changes into the model. `cur` is the engine's workbook, already committed
    /// (so it equals the last synced one but for view state). While joining, `cur` is the
    /// starting point whose unshared parts (view state, kept file content) are kept, and this
    /// builds the first workbook once the room's document arrived. `None`: nothing to do.
    pub fn refresh(&mut self, cur: &Arc<Workbook>) -> Result<Option<Refreshed>, CollabError> {
        if self.base.is_none() {
            if !self.check_meta()? {
                return Ok(None);
            }
            *lock(&self.changes) = Changes::default();
            let wb = self.build_all(cur);
            self.base = Some(Arc::new(wb.clone()));
            return Ok(Some(Refreshed { wb, changed: vec![], structural: true, chat: true }));
        }
        let mut ch = std::mem::take(&mut *lock(&self.changes));
        if ch.meta {
            self.check_meta()?;
        }
        if ch.styles {
            for id in std::mem::take(&mut self.unresolved) {
                ch.sheets.entry(id).or_default().full = true;
            }
        }
        if !ch.list && !ch.book && !ch.chat && ch.sheets.is_empty() {
            return Ok(None);
        }
        let doc = self.doc.clone();
        let txn = doc.transact();
        let normal_changed = ch.book && {
            let book = txn.get_map("book");
            let normal: Option<Style> = book.and_then(|b| get_json(&b, &txn, "normal"));
            normal.unwrap_or_default() != *cur.styles.get(StyleId::DEFAULT)
        };
        drop(txn);
        if normal_changed {
            let wb = self.build_all(cur);
            self.base = Some(Arc::new(wb.clone()));
            return Ok(Some(Refreshed { wb, changed: vec![], structural: true, chat: ch.chat }));
        }
        let txn = doc.transact();
        let mut wb: Workbook = (**cur).clone();
        let list = self.sheet_list(&txn);
        let old_index: HashMap<String, usize> = self.proj.iter().enumerate().map(|(i, p)| (p.id.clone(), i)).collect();
        let mut sheets = Vec::with_capacity(list.len());
        let mut proj = Vec::with_capacity(list.len());
        let mut changed = Vec::new();
        let mut structural = ch.list;
        for (ni, id) in list.iter().enumerate() {
            let old = old_index.get(id).copied();
            let sc = ch.sheets.get(id.as_str());
            match (old, sc) {
                (Some(oi), None) => {
                    // Untouched.
                    if let (Some(sh), Some(p)) = (cur.sheets.get(oi), self.proj.get(oi)) {
                        sheets.push(sh.clone());
                        proj.push(p.clone());
                        continue;
                    }
                }
                (Some(oi), Some(sc)) if !sc.full => {
                    if let (Some(sh), Some(p)) = (cur.sheets.get(oi), self.proj.get(oi).cloned()) {
                        let p = &p;
                        let mut sh = sh.clone();
                        self.reading = id.clone();
                        let root = txn.get_map(root_name(id).as_str());
                        let s = Arc::make_mut(&mut sh);
                        if let Some(root) = &root {
                            if let Some(n) = out_str(root.get(&txn, "name")) {
                                s.name = n.to_string();
                            }
                            if sc.props {
                                let props = sub_map(root, &txn, "props");
                                read_sheet_fields(&txn, Some(root), props.as_ref(), s);
                            }
                            let cells = sub_map(root, &txn, "cells");
                            for key in &sc.cells {
                                let Some((r, c)) = key.split_once(':') else { continue };
                                let (Some(r), Some(c)) = (p.rows.pos(r), p.cols.pos(c)) else { continue };
                                let pos = CellRef::new(r, c);
                                let cell = match cells.as_ref().and_then(|m| m.get(&txn, key)) {
                                    Some(Out::YMap(m)) => self.read_cell(&txn, &m, p, pos, &mut wb.styles),
                                    _ => None,
                                };
                                match cell {
                                    Some(cell) => s.cells.set(pos, cell),
                                    None => {
                                        s.cells.remove(pos);
                                    }
                                }
                                changed.push((ni, pos));
                            }
                        }
                        sheets.push(sh);
                        proj.push(p.clone());
                        continue;
                    }
                }
                _ => {}
            }
            // New, or changed throughout: read it whole.
            structural = true;
            let template = old.and_then(|oi| cur.sheets.get(oi)).map(|s| (**s).clone());
            let (sh, ids) = self.build_sheet(&txn, id, template, &mut wb.styles);
            sheets.push(Arc::new(sh));
            proj.push(ids);
        }
        if ch.book {
            match txn.get_map("book") {
                Some(book) => read_book(&txn, &book, &mut wb),
                None => read_names(&txn, &mut wb),
            }
        }
        if ch.chat {
            wb.chat = Arc::new(crate::chat::read(&txn));
        }
        drop(txn);
        // Unique names (concurrent adds can give two sheets one name): later ones get a suffix.
        let mut taken = HashSet::new();
        for sh in sheets.iter_mut() {
            let name = unique_name(&sh.name, &taken);
            if name != sh.name {
                Arc::make_mut(sh).name = name.clone();
            }
            taken.insert(name.to_lowercase());
        }
        let active_id = self.proj.get(cur.active_sheet).map(|p| p.id.clone());
        wb.sheets = sheets;
        wb.active_sheet = active_id.and_then(|a| proj.iter().position(|p| p.id == a)).unwrap_or(0).min(wb.sheets.len().saturating_sub(1));
        for id in list {
            self.observe_sheet(&id);
        }
        self.proj = proj;
        self.base = Some(Arc::new(wb.clone()));
        Ok(Some(Refreshed { wb, changed, structural, chat: ch.chat }))
    }

    /// The room's sheet ids in order, without duplicates (never empty).
    fn sheet_list<R: ReadTxn>(&self, txn: &R) -> Vec<String> {
        let arr = txn.get_array("sheets");
        let mut seen = HashSet::new();
        let mut out: Vec<String> = read_ids(arr.as_ref(), txn)
            .into_iter()
            .filter(|s| seen.insert(s.clone()))
            .filter(|s| txn.get_map(root_name(s).as_str()).is_some_and(|m| m.len(txn) > 0))
            .map(|s| s.to_string())
            .collect();
        out.truncate(4_096);
        if out.is_empty() {
            out.push(FALLBACK_SHEET.to_string());
        }
        out
    }

    /// The whole workbook from the room, keeping `cur`'s unshared parts (view state, file
    /// content GridCraft doesn't model).
    fn build_all(&mut self, cur: &Arc<Workbook>) -> Workbook {
        let doc = self.doc.clone();
        let txn = doc.transact();
        let mut wb: Workbook = (**cur).clone();
        let normal: Style = txn.get_map("book").and_then(|b| get_json(&b, &txn, "normal")).unwrap_or_default();
        wb.styles = StyleTable::with_default(normal);
        self.styles_read.clear();
        match txn.get_map("book") {
            Some(book) => read_book(&txn, &book, &mut wb),
            None => read_names(&txn, &mut wb),
        }
        // The room's chat replaces the workbook's.
        wb.chat = Arc::new(crate::chat::read(&txn));
        let list = self.sheet_list(&txn);
        // Sheets of `cur` keep their unshared parts when they're in the room (same id), or, on
        // joining with a file, by position.
        let old_index: HashMap<String, usize> = self.proj.iter().enumerate().map(|(i, p)| (p.id.clone(), i)).collect();
        let mut sheets = Vec::new();
        let mut proj = Vec::new();
        for (i, id) in list.iter().enumerate() {
            let template = match old_index.get(id) {
                Some(oi) => cur.sheets.get(*oi),
                None if self.proj.is_empty() => cur.sheets.get(i),
                None => None,
            }
            .map(|s| (**s).clone());
            let (sh, ids) = self.build_sheet(&txn, id, template, &mut wb.styles);
            sheets.push(Arc::new(sh));
            proj.push(ids);
        }
        drop(txn);
        let mut taken = HashSet::new();
        for sh in sheets.iter_mut() {
            let name = unique_name(&sh.name, &taken);
            if name != sh.name {
                Arc::make_mut(sh).name = name.clone();
            }
            taken.insert(name.to_lowercase());
        }
        let active_id = self.proj.get(cur.active_sheet).map(|p| p.id.clone());
        wb.sheets = sheets;
        wb.active_sheet = active_id.and_then(|a| proj.iter().position(|p| p.id == a)).unwrap_or(0).min(wb.sheets.len().saturating_sub(1));
        for id in &list {
            self.observe_sheet(id);
        }
        self.proj = proj;
        wb
    }

    fn style_of<R: ReadTxn>(&mut self, txn: &R, key: &str, table: &mut StyleTable) -> StyleId {
        if let Some(s) = self.styles_read.get(key) {
            return table.intern(s.clone());
        }
        let style: Option<Style> = txn.get_map("styles").and_then(|m| get_json(&m, txn, key));
        match style {
            Some(s) => {
                self.styles_read.insert(key.to_string(), s.clone());
                table.intern(s)
            }
            None => {
                self.unresolved.insert(self.reading.clone());
                StyleId::DEFAULT
            }
        }
    }

    fn read_cell<R: ReadTxn>(&mut self, txn: &R, m: &MapRef, ids: &SheetIds, pos: CellRef, table: &mut StyleTable) -> Option<Cell> {
        let input = out_str(m.get(txn, "input")).unwrap_or_else(|| Arc::from(""));
        let style = match out_str(m.get(txn, "style")) {
            Some(k) => self.style_of(txn, &k, table),
            None => StyleId::DEFAULT,
        };
        let mut cell = match input::decode(&input) {
            Input::Value(v) => Cell { value: v, formula: None, style },
            Input::Formula(f) => Cell { value: gridcraft_core::Value::Empty, formula: Some(Arc::new(Formula::new(&f))), style },
        };
        if let Some(a) = out_str(m.get(txn, "array"))
            && let Some(f) = cell.formula.as_mut()
            && let Some((r, c)) = a.split_once(':')
            && let (Some(r), Some(c)) = (ids.rows.pos(r), ids.cols.pos(c))
            && r >= pos.row
            && c >= pos.col
        {
            Arc::make_mut(f).array = Some(RangeRef::new(pos, CellRef::new(r, c)));
        }
        (!cell.is_blank()).then_some(cell)
    }

    fn build_sheet<R: ReadTxn>(&mut self, txn: &R, id: &str, template: Option<Sheet>, table: &mut StyleTable) -> (Sheet, SheetIds) {
        self.reading = id.to_string();
        self.unresolved.remove(id);
        let root = txn.get_map(root_name(id).as_str());
        let name = root.as_ref().and_then(|r| out_str(r.get(txn, "name"))).map(|s| s.to_string()).unwrap_or_else(|| "Sheet1".into());
        let mut sh = template.unwrap_or_else(|| Sheet::new(name.clone()));
        sh.name = name;
        sh.cells = Default::default();
        sh.spill.clear();
        sh.spill_ranges.clear();
        sh.rows.clear();
        sh.cols.clear();
        sh.merges.clear();
        let mut rows = Line::from_ids(read_ids(root.as_ref().and_then(|r| sub_array(r, txn, "rows")).as_ref(), txn), MAX_ROWS);
        let mut cols = Line::from_ids(read_ids(root.as_ref().and_then(|r| sub_array(r, txn, "cols")).as_ref(), txn), MAX_COLS);
        if let Some(r) = root.as_ref() {
            rows.next = rows.next.max(block_counter(r, txn, "rowBlocks"));
            cols.next = cols.next.max(block_counter(r, txn, "colBlocks"));
        }
        let ids = SheetIds { id: id.to_string(), rows, cols };
        let Some(root) = root else {
            read_sheet_fields(txn, None, None, &mut sh);
            return (sh, ids);
        };
        let props = sub_map(&root, txn, "props");
        read_sheet_fields(txn, Some(&root), props.as_ref(), &mut sh);
        if let Some(cells) = sub_map(&root, txn, "cells") {
            let entries: Vec<(String, Out)> = cells.iter(txn).map(|(k, v)| (k.to_string(), v)).collect();
            for (key, v) in entries {
                let Out::YMap(m) = v else { continue };
                let Some((r, c)) = key.split_once(':') else { continue };
                let (Some(r), Some(c)) = (ids.rows.pos(r), ids.cols.pos(c)) else { continue };
                let pos = CellRef::new(r, c);
                if let Some(cell) = self.read_cell(txn, &m, &ids, pos, table) {
                    sh.cells.set(pos, cell);
                }
            }
        }
        for (rows, key) in [(true, "rowProps"), (false, "colProps")] {
            let Some(m) = sub_map(&root, txn, key) else { continue };
            let entries: Vec<(String, Option<Arc<str>>)> = m.iter(txn).map(|(k, v)| (k.to_string(), out_str(Some(v)))).collect();
            for (id, v) in entries {
                let Some(lp) = v.and_then(|v| serde_json::from_str::<LineProps>(&v).ok()) else { continue };
                let line = if rows { &ids.rows } else { &ids.cols };
                let Some(p) = line.pos(&id) else { continue };
                let style = lp.style.as_deref().map(|k| self.style_of(txn, k, table));
                let info = LineInfo { size: lp.size, hidden: lp.hidden, outline: lp.outline, collapsed: lp.collapsed, style, custom: lp.custom };
                if rows {
                    sh.rows.insert(p, info);
                } else {
                    sh.cols.insert(p, info);
                }
            }
        }
        if let Some(m) = sub_map(&root, txn, "merges") {
            let mut ranges: Vec<RangeRef> = m
                .iter(txn)
                .filter_map(|(k, v)| {
                    let v = out_str(Some(v))?;
                    let (r0, c0) = k.split_once(':')?;
                    let (r1, c1) = v.split_once(':')?;
                    let a = CellRef::new(ids.rows.pos(r0)?, ids.cols.pos(c0)?);
                    let b = CellRef::new(ids.rows.pos(r1)?, ids.cols.pos(c1)?);
                    let r = RangeRef::new(a, b);
                    (r.start != r.end).then_some(r)
                })
                .collect();
            ranges.sort_by_key(|r| (r.start.row, r.start.col, r.end.row, r.end.col));
            // Concurrent merges can overlap: the first (top-left) wins.
            let mut kept: Vec<RangeRef> = Vec::new();
            for r in ranges {
                if !kept.iter().any(|k| k.intersects(&r)) {
                    kept.push(r);
                }
            }
            sh.merges = kept;
        }
        (sh, ids)
    }

    // ------------------------------------------------------------ undo

    /// Undoes this person's last change (only theirs). Returns the update to send; the model
    /// follows at [`Replica::refresh`]. `None`: nothing to undo.
    pub fn undo(&mut self) -> Result<Option<Vec<u8>>, CollabError> {
        self.pop(true)
    }

    /// Redoes the last undone change.
    pub fn redo(&mut self) -> Result<Option<Vec<u8>>, CollabError> {
        self.pop(false)
    }

    fn pop(&mut self, undo: bool) -> Result<Option<Vec<u8>>, CollabError> {
        if self.base.is_none() {
            return Err(CollabError::NotReady);
        }
        let can = if undo { self.undo.can_undo() } else { self.undo.can_redo() };
        if !can {
            return Ok(None);
        }
        let um = &mut self.undo;
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| if undo { um.undo_blocking() } else { um.redo_blocking() }));
        if r.is_err() {
            return Err(CollabError::Other("undo failed".into()));
        }
        self.take_outbox()
    }

    pub fn can_undo(&self) -> bool {
        self.undo.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.undo.can_redo()
    }

    // ------------------------------------------------------------ sync

    /// Sync step 1: our state vector.
    pub fn state_vector(&self) -> Vec<u8> {
        self.doc.transact().state_vector().encode_v1()
    }

    /// Whether a peer's state vector shows changes this document lacks (then ask back with our
    /// own sync step 1).
    pub fn lacks(&self, sv: &[u8]) -> bool {
        let Ok(theirs) = StateVector::decode_v1(sv) else { return false };
        let ours = self.doc.transact().state_vector();
        theirs.iter().any(|(c, clock)| *clock > ours.get(c))
    }

    /// Sync step 2: what a peer with state vector `sv` lacks.
    pub fn diff_for(&self, sv: &[u8]) -> Result<Vec<u8>, CollabError> {
        let sv = StateVector::decode_v1(sv).map_err(|e| CollabError::Corrupt(e.to_string()))?;
        Ok(self.doc.transact().encode_state_as_update_v1(&sv))
    }

    /// The whole document as one update (snapshots, recovery).
    pub fn full_state(&self) -> Vec<u8> {
        self.doc.transact().encode_state_as_update_v1(&StateVector::default())
    }

    // ------------------------------------------------------------ selections

    /// A selection as awareness carries it: `{sheet, ranges: [[rowId, colId, rowId, colId]]}`.
    /// Rows and columns past the end of the room's ids are named by the block ids they get when
    /// the sheet grows to them (schema 1, "Awareness"), so a selection on an empty sheet, or
    /// below its last row, travels without writing to the document.
    pub fn selection_to_ids(&self, sheet: usize, ranges: &[RangeRef]) -> Option<Json> {
        let ids = self.proj.get(sheet)?;
        let list: Vec<Json> = ranges
            .iter()
            .take(64)
            .map(|r| {
                let row = |p: u32| ids.rows.id_or_block(p.min(MAX_ROWS - 1));
                let col = |p: u32| ids.cols.id_or_block(p.min(MAX_COLS - 1));
                Json::Array(vec![row(r.start.row).into(), col(r.start.col).into(), row(r.end.row).into(), col(r.end.col).into()])
            })
            .collect();
        Some(serde_json::json!({"sheet": ids.id, "ranges": list}))
    }

    /// A peer's selection in the model: `(sheet index, ranges)`; block ids past the end resolve
    /// to the positions they will have, and ranges whose ids are gone are left out.
    pub fn selection_from_ids(&self, sel: &Json) -> Option<(usize, Vec<RangeRef>)> {
        let sid = sel.get("sheet")?.as_str()?;
        let si = self.proj.iter().position(|p| p.id == sid)?;
        let ids = self.proj.get(si)?;
        let ranges = sel
            .get("ranges")?
            .as_array()?
            .iter()
            .take(64)
            .filter_map(|r| {
                let r = r.as_array()?;
                let s = |i: usize| r.get(i).and_then(Json::as_str);
                let row = |id: &str| ids.rows.pos_or_block(id, MAX_ROWS);
                let col = |id: &str| ids.cols.pos_or_block(id, MAX_COLS);
                let a = CellRef::new(row(s(0)?)?, col(s(1)?)?);
                let b = CellRef::new(row(s(2)?)?, col(s(3)?)?);
                Some(RangeRef::new(a, b))
            })
            .collect();
        Some((si, ranges))
    }

    /// The workbook this replica's own document holds (tests: model vs document).
    #[cfg(test)]
    pub(crate) fn debug_build(&mut self, cur: &Arc<Workbook>) -> Workbook {
        let proj = self.proj.clone();
        let unresolved = self.unresolved.clone();
        let wb = self.build_all(cur);
        self.proj = proj;
        self.unresolved = unresolved;
        wb
    }

    /// The id of the sheet at `index` in the synced model.
    pub fn sheet_id(&self, index: usize) -> Option<&str> {
        self.proj.get(index).map(|p| p.id.as_str())
    }
}

/// A sheet being written: its map, ids and arrays.
struct SheetW {
    root: MapRef,
    ids: SheetIds,
    rows_arr: ArrayRef,
    cols_arr: ArrayRef,
}

/// A line's properties for comparison: the style by value (style ids are per workbook).
fn line_cmp<'a>(wb: &'a Workbook, info: &LineInfo) -> (LineInfo, Option<&'a Style>) {
    (LineInfo { style: None, ..*info }, info.style.map(|s| wb.styles.get(s)))
}

fn merge_entry(ids: &SheetIds, m: &RangeRef) -> Option<(String, String)> {
    Some((key_of(ids.rows.id(m.start.row)?, ids.cols.id(m.start.col)?), key_of(ids.rows.id(m.end.row)?, ids.cols.id(m.end.col)?)))
}

/// A defined name's key in `names`: the name (lower case), and `@<sheet index>` when it's
/// local to a sheet.
fn name_key(n: &gridcraft_model::DefinedName) -> String {
    match n.scope {
        Some(s) => format!("{}@{s}", n.name.to_lowercase()),
        None => n.name.to_lowercase(),
    }
}

fn read_names<R: ReadTxn>(txn: &R, wb: &mut Workbook) {
    let mut names: Vec<(String, gridcraft_model::DefinedName)> = match txn.get_map("names") {
        Some(m) => m.iter(txn).filter_map(|(k, v)| Some((k.to_string(), serde_json::from_str(&out_str(Some(v))?).ok()?))).collect(),
        None => vec![],
    };
    names.sort_by(|a, b| a.0.cmp(&b.0));
    wb.names = names.into_iter().map(|(_, n)| n).collect();
}

fn read_book<R: ReadTxn>(txn: &R, book: &MapRef, wb: &mut Workbook) {
    read_names(txn, wb);
    let fresh = Workbook::new();
    macro_rules! r {
        ($k:literal, $f:ident) => {
            wb.$f = get_json(book, txn, $k).unwrap_or_else(|| fresh.$f.clone());
        };
    }
    book_fields!(r);
}

/// Pairs `cur`'s sheets with `base`'s: the same name, else, when as many sheets went as came,
/// in order (renames).
fn match_sheets(base: &Workbook, cur: &Workbook) -> Vec<Option<usize>> {
    let mut out: Vec<Option<usize>> = vec![None; cur.sheets.len()];
    let mut used = vec![false; base.sheets.len()];
    for (i, s) in cur.sheets.iter().enumerate() {
        // Same Arc: the very same sheet.
        if let Some(j) = base.sheets.iter().position(|b| Arc::ptr_eq(b, s))
            && !used.get(j).copied().unwrap_or(true)
        {
            out[i] = Some(j);
            used[j] = true;
        }
    }
    for (i, s) in cur.sheets.iter().enumerate() {
        if out.get(i).copied().flatten().is_some() {
            continue;
        }
        if let Some(j) = base.sheets.iter().enumerate().position(|(j, b)| !used[j] && b.name.eq_ignore_ascii_case(&s.name)) {
            out[i] = Some(j);
            used[j] = true;
        }
    }
    let left_cur: Vec<usize> = (0..cur.sheets.len()).filter(|i| out[*i].is_none()).collect();
    let left_base: Vec<usize> = (0..base.sheets.len()).filter(|j| !used[*j]).collect();
    if left_cur.len() == left_base.len() {
        for (i, j) in left_cur.into_iter().zip(left_base) {
            out[i] = Some(j);
        }
    }
    out
}

/// Makes the distinct ids of `a` read `desired`, moving and inserting as needed.
fn reorder(a: &ArrayRef, txn: &mut TransactionMut, desired: &[Arc<str>]) {
    let distinct = |a: &ArrayRef, txn: &TransactionMut| -> Vec<Arc<str>> {
        let mut seen = HashSet::new();
        read_ids(Some(a), txn).into_iter().filter(|s| seen.insert(s.clone())).collect()
    };
    let mut have = distinct(a, txn);
    if have == desired {
        return;
    }
    for (i, d) in desired.iter().enumerate() {
        if have.get(i) == Some(d) {
            continue;
        }
        if have.contains(d) {
            remove_ids(a, txn, &[d.clone()].into_iter().collect());
        }
        let yi = y_index(a, txn, i as u32);
        a.insert(txn, yi, Any::String(d.clone()));
        have = distinct(a, txn);
    }
    if have.len() > desired.len() {
        let extra: HashSet<Arc<str>> = have.into_iter().skip(desired.len()).collect();
        remove_ids(a, txn, &extra);
    }
}

/// What schema 1 shares of a workbook, as canonical text: equal on two peers exactly when
/// they show the same shared content (tests and diagnostics compare peers with it). Style ids
/// are replaced by the styles themselves; view state and calculated values are left out.
pub fn fingerprint(wb: &Workbook) -> String {
    let mut out = String::new();
    let style = |id: StyleId| serde_json::to_string(wb.styles.get(id)).unwrap_or_default();
    out.push_str(&format!("normal {}\n", style(StyleId::DEFAULT)));
    macro_rules! b {
        ($k:literal, $f:ident) => {
            out.push_str(&format!("{} {}\n", $k, serde_json::to_string(&wb.$f).unwrap_or_default()));
        };
    }
    book_fields!(b);
    let mut names: Vec<String> = wb.names.iter().map(|n| format!("{} {}", name_key(n), serde_json::to_string(n).unwrap_or_default())).collect();
    names.sort();
    out.push_str(&format!("names {names:?}\n"));
    out.push_str(&format!("chat {}\n", wb.chat.to_json()));
    for sh in &wb.sheets {
        out.push_str(&format!("sheet {:?}\n", sh.name));
        for (pos, cell) in sh.cells.iter() {
            if cell.is_blank() {
                continue;
            }
            let array = cell.formula.as_ref().and_then(|f| f.array).map(|a| a.a1()).unwrap_or_default();
            out.push_str(&format!("  {} {:?} {} {}\n", pos.a1(), input::encode(cell), style(cell.style), array));
        }
        for (what, map) in [("row", &sh.rows), ("col", &sh.cols)] {
            for (p, info) in map {
                out.push_str(&format!("  {what} {p} {:?} {}\n", LineInfo { style: None, ..*info }, info.style.map(style).unwrap_or_default()));
            }
        }
        let mut merges = sh.merges.clone();
        merges.sort_by_key(|r| (r.start.row, r.start.col));
        out.push_str(&format!("  merges {:?}\n", merges.iter().map(|m| m.a1()).collect::<Vec<_>>()));
        macro_rules! s {
            ($w:ident, $k:literal, $f:ident) => {
                out.push_str(&format!("  {} {}\n", $k, serde_json::to_string(&sh.$f).unwrap_or_default()));
            };
        }
        sheet_fields!(s);
        out.push_str(&format!("  comments {:?}\n  links {:?}\n", a1_pairs(&sh.comments), a1_pairs(&sh.hyperlinks)));
    }
    out
}
