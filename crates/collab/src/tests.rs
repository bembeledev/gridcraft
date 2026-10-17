//! Replica tests: sharing and joining, convergence of random concurrent edits on two and three
//! peers with updates delivered in random order and duplicated, per-user undo, and refusals.

use std::sync::Arc;

use gridcraft_core::{CellRef, RangeRef, Value};
use gridcraft_model::{Cell, Formula, LineInfo, Sheet, StyleId, Workbook};

use crate::{Axis, CollabError, Replica, StructOp, fingerprint};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
}

struct Peer {
    rep: Replica,
    wb: Arc<Workbook>,
    inbox: Vec<Vec<u8>>,
}

impl Peer {
    fn edit(&mut self, f: impl FnOnce(&mut Workbook) -> Vec<StructOp>) -> Option<Vec<u8>> {
        let mut wb = (*self.wb).clone();
        let ops = f(&mut wb);
        self.wb = Arc::new(wb);
        self.rep.commit(&self.wb, &ops).expect("commit")
    }
    fn receive(&mut self, update: &[u8]) {
        self.rep.apply_update(update).expect("apply");
        self.refresh();
    }
    fn refresh(&mut self) {
        if let Some(r) = self.rep.refresh(&self.wb).expect("refresh") {
            self.wb = Arc::new(r.wb);
        }
    }
}

fn sample() -> Workbook {
    let mut wb = Workbook::new();
    let sh = Arc::make_mut(&mut wb.sheets[0]);
    sh.set_value(CellRef::new(0, 0), Value::Text("Item".into()));
    sh.set_value(CellRef::new(0, 1), Value::Text("Price".into()));
    for r in 1..6 {
        sh.set_value(CellRef::new(r, 0), Value::Text(format!("thing {r}").into()));
        sh.set_value(CellRef::new(r, 1), Value::Number(r as f64 * 1.5));
    }
    sh.cells.set(CellRef::new(6, 1), Cell::formula(Formula::new("=SUM(B2:B6)")));
    let bold = wb.styles.derive(StyleId::DEFAULT, |s| s.font.bold = true);
    Arc::make_mut(&mut wb.sheets[0]).set_style(CellRef::new(0, 0), bold);
    Arc::make_mut(&mut wb.sheets[0]).rows.insert(0, LineInfo { size: Some(30.0), custom: true, ..Default::default() });
    Arc::make_mut(&mut wb.sheets[0]).merges.push(RangeRef::new(CellRef::new(8, 0), CellRef::new(8, 2)));
    wb.sheets.push(Arc::new(Sheet::new("Notes")));
    wb
}

/// Peer 0 shares `sample()`; the others join and get the whole state.
fn room(n: usize, seed: u64) -> Vec<Peer> {
    let mut first = Replica::new(seed.wrapping_mul(7) + 1);
    let wb = Arc::new(sample());
    first.seed(wb.clone(), "Budget").expect("seed");
    let state = first.full_state();
    let mut peers = vec![Peer { rep: first, wb, inbox: vec![] }];
    for i in 1..n {
        let mut p = Peer { rep: Replica::new(seed.wrapping_mul(7) + 1 + i as u64), wb: Arc::new(Workbook::new()), inbox: vec![] };
        assert!(!p.rep.is_ready());
        p.receive(&state);
        assert!(p.rep.is_ready());
        peers.push(p);
    }
    peers
}

fn insert_lines(sh: &mut Sheet, axis: Axis, at: u32, count: u32) {
    match axis {
        Axis::Rows => {
            sh.cells.shift_rows(at, count as i64);
            sh.rows = std::mem::take(&mut sh.rows).into_iter().map(|(p, i)| (if p >= at { p + count } else { p }, i)).collect();
            for m in sh.merges.iter_mut() {
                if m.start.row >= at {
                    m.start.row += count;
                    m.end.row += count;
                }
            }
        }
        Axis::Cols => {
            sh.cells.shift_cols(at, count as i64);
            sh.cols = std::mem::take(&mut sh.cols).into_iter().map(|(p, i)| (if p >= at { p + count } else { p }, i)).collect();
            for m in sh.merges.iter_mut() {
                if m.start.col >= at {
                    m.start.col += count;
                    m.end.col += count;
                }
            }
        }
    }
}

fn delete_lines(sh: &mut Sheet, axis: Axis, at: u32, count: u32) {
    let end = at + count;
    match axis {
        Axis::Rows => {
            sh.take_cells(RangeRef::rows(at, end - 1));
            sh.cells.shift_rows(end, -(count as i64));
            sh.rows = std::mem::take(&mut sh.rows)
                .into_iter()
                .filter(|(p, _)| *p < at || *p >= end)
                .map(|(p, i)| (if p >= end { p - count } else { p }, i))
                .collect();
            sh.merges.retain(|m| m.end.row < at || m.start.row >= end);
            for m in sh.merges.iter_mut() {
                if m.start.row >= end {
                    m.start.row -= count;
                    m.end.row -= count;
                }
            }
        }
        Axis::Cols => {
            sh.take_cells(RangeRef::cols(at, end - 1));
            sh.cells.shift_cols(end, -(count as i64));
            sh.cols = std::mem::take(&mut sh.cols)
                .into_iter()
                .filter(|(p, _)| *p < at || *p >= end)
                .map(|(p, i)| (if p >= end { p - count } else { p }, i))
                .collect();
            sh.merges.retain(|m| m.end.col < at || m.start.col >= end);
            for m in sh.merges.iter_mut() {
                if m.start.col >= end {
                    m.start.col -= count;
                    m.end.col -= count;
                }
            }
        }
    }
}

/// One random edit, as a command would make it.
fn random_edit(rng: &mut Rng, wb: &mut Workbook, tag: &str) -> Vec<StructOp> {
    let si = rng.below(wb.sheets.len() as u64) as usize;
    let r = rng.below(14) as u32;
    let c = rng.below(6) as u32;
    let name = wb.sheets[si].name.clone();
    match rng.below(16) {
        0..=4 => {
            let v = match rng.below(4) {
                0 => Value::Number(rng.below(1000) as f64 / 4.0),
                1 => Value::Text(format!("{tag}{}", rng.below(100)).into()),
                2 => Value::Text("007".into()),
                _ => Value::Bool(true),
            };
            Arc::make_mut(&mut wb.sheets[si]).set_value(CellRef::new(r, c), v);
        }
        5 => {
            Arc::make_mut(&mut wb.sheets[si]).cells.set(CellRef::new(r, c), Cell::formula(Formula::new("=A1+1")));
        }
        6 => {
            Arc::make_mut(&mut wb.sheets[si]).cells.remove(CellRef::new(r, c));
        }
        7 => {
            let size = 8.0 + rng.below(20) as f32;
            let st = wb.styles.derive(StyleId::DEFAULT, |s| s.font.size = size);
            Arc::make_mut(&mut wb.sheets[si]).set_style(CellRef::new(r, c), st);
        }
        8 | 9 => {
            let axis = if rng.below(2) == 0 { Axis::Rows } else { Axis::Cols };
            let count = 1 + rng.below(2) as u32;
            insert_lines(Arc::make_mut(&mut wb.sheets[si]), axis, if axis == Axis::Rows { r } else { c }, count);
            return vec![StructOp { sheet: name, axis, at: if axis == Axis::Rows { r } else { c }, count, insert: true }];
        }
        10 | 11 => {
            let axis = if rng.below(2) == 0 { Axis::Rows } else { Axis::Cols };
            let at = if axis == Axis::Rows { r } else { c };
            delete_lines(Arc::make_mut(&mut wb.sheets[si]), axis, at, 1);
            return vec![StructOp { sheet: name, axis, at, count: 1, insert: false }];
        }
        12 => {
            let n = format!("{tag}{}", rng.below(50));
            if wb.sheet_index(&n).is_none() {
                let at = rng.below(wb.sheets.len() as u64 + 1) as usize;
                wb.sheets.insert(at, Arc::new(Sheet::new(n)));
            }
        }
        13 => {
            let n = format!("{tag}r{}", rng.below(50));
            if wb.sheet_index(&n).is_none() {
                Arc::make_mut(&mut wb.sheets[si]).name = n;
            }
        }
        14 => {
            if wb.sheets.len() > 1 {
                wb.sheets.remove(si);
                wb.active_sheet = 0;
            }
        }
        _ => {
            let sh = Arc::make_mut(&mut wb.sheets[si]);
            let m = RangeRef::new(CellRef::new(r, c), CellRef::new(r + 1, c + 1));
            if sh.merges.iter().any(|x| x.intersects(&m)) {
                sh.merges.retain(|x| !x.intersects(&m));
            } else {
                sh.merges.push(m);
            }
            sh.rows.insert(r, LineInfo { size: Some(12.0 + rng.below(9) as f32), custom: true, ..Default::default() });
        }
    }
    vec![]
}

/// The lines only one side has.
fn line_diff(a: &str, b: &str) -> String {
    let la: Vec<&str> = a.lines().collect();
    let lb: Vec<&str> = b.lines().collect();
    let mut out = String::new();
    for l in &la {
        if !lb.contains(l) {
            out.push_str(&format!("- {}\n", l.chars().take(200).collect::<String>()));
        }
    }
    for l in &lb {
        if !la.contains(l) {
            out.push_str(&format!("+ {}\n", l.chars().take(200).collect::<String>()));
        }
    }
    out
}

/// Runs a random concurrent session and checks that every peer ends with the same workbook,
/// the same one a newcomer joining afterwards builds.
fn converge(n: usize, seed: u64, steps: usize) {
    let mut rng = Rng(seed);
    let mut peers = room(n, seed);
    for step in 0..steps {
        let i = rng.below(n as u64) as usize;
        match rng.below(3) {
            0 | 1 => {
                let tag = format!("p{i}s{step}-");
                let mut r2 = Rng(rng.next());
                if let Some(u) = peers[i].edit(|wb| random_edit(&mut r2, wb, &tag)) {
                    for (j, p) in peers.iter_mut().enumerate() {
                        if j != i {
                            p.inbox.push(u.clone());
                            if r2.below(10) == 0 {
                                p.inbox.push(u.clone());
                            }
                        }
                    }
                }
            }
            _ => {
                // Deliver one waiting update, picked at random (out of order).
                let p = &mut peers[i];
                if !p.inbox.is_empty() {
                    let k = rng.below(p.inbox.len() as u64) as usize;
                    let u = p.inbox.remove(k);
                    p.receive(&u);
                }
            }
        }
    }
    // Everything delivered (reversed: worst order), then a sync round for good measure.
    for p in peers.iter_mut() {
        let inbox = std::mem::take(&mut p.inbox);
        for u in inbox.iter().rev() {
            p.receive(u);
        }
    }
    for i in 0..n {
        for j in 0..n {
            if i != j {
                let sv = peers[j].rep.state_vector();
                let d = peers[i].rep.diff_for(&sv).expect("diff");
                peers[j].receive(&d);
            }
        }
    }
    // And each peer's model is what its own document says (no drift between model and room).
    for (i, p) in peers.iter_mut().enumerate() {
        let mut check = Peer { rep: Replica::new(5_000 + i as u64), wb: Arc::new(Workbook::new()), inbox: vec![] };
        check.receive(&p.rep.full_state());
        let (a, b) = (fingerprint(&p.wb), fingerprint(&check.wb));
        let wb = p.wb.clone();
        let own = fingerprint(&p.rep.debug_build(&wb));
        assert!(a == own, "seed {seed}: peer {i}'s model drifted from its own document:\n{}", line_diff(&a, &own));
        assert!(a == b, "seed {seed}: peer {i}'s document differs from its encoding:\n{}", line_diff(&a, &b));
    }
    let f0 = fingerprint(&peers[0].wb);
    for (i, p) in peers.iter().enumerate().skip(1) {
        let f = fingerprint(&p.wb);
        assert!(f0 == f, "seed {seed}: peer {i} differs from peer 0:\n{}", line_diff(&f0, &f));
    }
    // A newcomer builds the same workbook.
    let mut late = Peer { rep: Replica::new(999), wb: Arc::new(Workbook::new()), inbox: vec![] };
    late.receive(&peers[0].rep.full_state());
    assert!(f0 == fingerprint(&late.wb), "seed {seed}: a newcomer differs:\n{}", line_diff(&f0, &fingerprint(&late.wb)));
}

#[test]
fn share_then_join_gives_the_same_workbook() {
    let peers = room(2, 1);
    assert_eq!(fingerprint(&peers[0].wb), fingerprint(&peers[1].wb));
    let sh = &peers[1].wb.sheets[0];
    assert_eq!(sh.cell(CellRef::new(6, 1)).and_then(|c| c.formula.as_ref()).map(|f| f.text.clone()), Some("SUM(B2:B6)".into()));
    assert!(peers[1].wb.styles.get(sh.style_id(CellRef::new(0, 0))).font.bold);
    assert_eq!(sh.merges, vec![RangeRef::new(CellRef::new(8, 0), CellRef::new(8, 2))]);
    assert_eq!(peers[1].wb.sheets[1].name, "Notes");
    assert_eq!(peers[1].rep.title().as_deref(), Some("Budget"));
}

#[test]
fn edits_and_row_inserts_travel() {
    let mut peers = room(2, 2);
    let u = peers[0]
        .edit(|wb| {
            insert_lines(Arc::make_mut(&mut wb.sheets[0]), Axis::Rows, 1, 1);
            Arc::make_mut(&mut wb.sheets[0]).set_value(CellRef::new(1, 0), Value::Text("new".into()));
            vec![StructOp { sheet: "Sheet1".into(), axis: Axis::Rows, at: 1, count: 1, insert: true }]
        })
        .expect("an update");
    // Meanwhile peer 1 typed into what was row 3 (now row 4 for peer 0).
    let u1 = peers[1].edit(|wb| {
        Arc::make_mut(&mut wb.sheets[0]).set_value(CellRef::new(3, 2), Value::Number(42.0));
        vec![]
    });
    peers[1].receive(&u);
    if let Some(u1) = u1 {
        peers[0].receive(&u1);
    }
    for p in &peers {
        let sh = &p.wb.sheets[0];
        assert_eq!(sh.value(CellRef::new(1, 0)), Value::Text("new".into()));
        // The typing followed its row.
        assert_eq!(sh.value(CellRef::new(4, 2)), Value::Number(42.0));
        assert_eq!(sh.value(CellRef::new(4, 0)), Value::Text("thing 3".into()));
    }
    assert_eq!(fingerprint(&peers[0].wb), fingerprint(&peers[1].wb));
}

#[test]
fn two_peers_converge() {
    let n = std::env::var("GRIDCRAFT_COLLAB_SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(12u64);
    for seed in 0..n {
        converge(2, seed, 120);
    }
}

#[test]
fn three_peers_converge() {
    let n = std::env::var("GRIDCRAFT_COLLAB_SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(8u64);
    for seed in 100..100 + n {
        converge(3, seed, 150);
    }
}

#[test]
fn undo_reverts_only_your_own_changes() {
    let mut peers = room(2, 3);
    let mine = peers[0]
        .edit(|wb| {
            Arc::make_mut(&mut wb.sheets[0]).set_value(CellRef::new(1, 1), Value::Number(100.0));
            vec![]
        })
        .expect("update");
    let theirs = peers[1]
        .edit(|wb| {
            Arc::make_mut(&mut wb.sheets[0]).set_value(CellRef::new(2, 1), Value::Number(200.0));
            vec![]
        })
        .expect("update");
    peers[1].receive(&mine);
    peers[0].receive(&theirs);
    // Peer 0 undoes: its own edit goes, peer 1's stays.
    let undo = peers[0].rep.undo().expect("undo").expect("an undo update");
    peers[0].refresh();
    peers[1].receive(&undo);
    for p in &peers {
        assert_eq!(p.wb.sheets[0].value(CellRef::new(1, 1)), Value::Number(1.5));
        assert_eq!(p.wb.sheets[0].value(CellRef::new(2, 1)), Value::Number(200.0));
    }
    // Remote changes never enter the undo stack: peer 1 has one step, peer 0 none left.
    assert!(!peers[0].rep.can_undo());
    assert!(peers[0].rep.can_redo());
    let redo = peers[0].rep.redo().expect("redo").expect("a redo update");
    peers[0].refresh();
    peers[1].receive(&redo);
    for p in &peers {
        assert_eq!(p.wb.sheets[0].value(CellRef::new(1, 1)), Value::Number(100.0));
    }
    assert!(peers[1].rep.can_undo());
    // Undoing an added sheet removes it everywhere.
    let add = peers[1]
        .edit(|wb| {
            wb.sheets.push(Arc::new(Sheet::new("Extra")));
            vec![]
        })
        .expect("update");
    peers[0].receive(&add);
    assert_eq!(peers[0].wb.sheets.len(), 3);
    let undo = peers[1].rep.undo().expect("undo").expect("update");
    peers[1].refresh();
    peers[0].receive(&undo);
    assert_eq!(peers[0].wb.sheets.len(), 2);
    assert_eq!(fingerprint(&peers[0].wb), fingerprint(&peers[1].wb));
}

#[test]
fn another_apps_room_is_refused() {
    let doc = yrs::Doc::new();
    {
        use yrs::{Map, Transact};
        let meta = doc.get_or_insert_map("meta");
        let mut txn = doc.transact_mut();
        meta.insert(&mut txn, "app", "wordcraft");
        meta.insert(&mut txn, "schema", 1.0);
    }
    let state = {
        use yrs::{ReadTxn, StateVector, Transact};
        doc.transact().encode_state_as_update_v1(&StateVector::default())
    };
    // Dropped, not applied: the room goes on syncing.
    let mut r = Replica::new(4);
    let err = r.apply_update(&state).expect_err("refused");
    assert_eq!(err, CollabError::Mismatch { app: "wordcraft".into(), schema: 1 });
    assert_eq!(err.to_string(), "This room is for wordcraft 1");
    assert_eq!(r.refresh(&Arc::new(Workbook::new())), Ok(None));
    assert!(!r.is_ready());
    // Into a shared GridCraft room too: refused, and the room's document stays.
    let mut peers = room(1, 11);
    let before = fingerprint(&peers[0].wb);
    assert!(peers[0].rep.apply_update(&state).is_err());
    peers[0].refresh();
    assert_eq!(fingerprint(&peers[0].wb), before);
    // Another schema of ours too.
    let mut r = Replica::new(5);
    let doc = yrs::Doc::new();
    {
        use yrs::{Map, Transact};
        let meta = doc.get_or_insert_map("meta");
        let mut txn = doc.transact_mut();
        meta.insert(&mut txn, "app", "gridcraft");
        meta.insert(&mut txn, "schema", 2.0);
    }
    let state = {
        use yrs::{ReadTxn, StateVector, Transact};
        doc.transact().encode_state_as_update_v1(&StateVector::default())
    };
    assert!(matches!(r.apply_update(&state), Err(CollabError::Mismatch { schema: 2, .. })));
}

#[test]
fn garbage_is_refused_not_a_crash() {
    let mut r = Replica::new(6);
    for bad in [&b""[..], &[0xff; 64], &[1, 2, 3], &[0, 0], &[5, 200, 1, 0, 0, 0]] {
        let _ = r.apply_update(bad);
        let _ = r.refresh(&Arc::new(Workbook::new()));
    }
    assert!(r.diff_for(&[0xff, 0xff]).is_err());
    assert!(matches!(r.apply_update(&vec![0; crate::MAX_UPDATE + 1]), Err(CollabError::TooLarge(_))));
}

#[test]
fn selections_travel_as_ids() {
    let mut peers = room(2, 9);
    let sel = peers[0].rep.selection_to_ids(0, &[RangeRef::new(CellRef::new(2, 1), CellRef::new(4, 2))]).expect("ids");
    assert_eq!(peers[1].rep.selection_from_ids(&sel), Some((0, vec![RangeRef::new(CellRef::new(2, 1), CellRef::new(4, 2))])));
    // After peer 1 inserts a row above, the same ids are one row lower there.
    let _ = peers[1].edit(|wb| {
        insert_lines(Arc::make_mut(&mut wb.sheets[0]), Axis::Rows, 0, 1);
        vec![StructOp { sheet: "Sheet1".into(), axis: Axis::Rows, at: 0, count: 1, insert: true }]
    });
    assert_eq!(peers[1].rep.selection_from_ids(&sel), Some((0, vec![RangeRef::new(CellRef::new(3, 1), CellRef::new(5, 2))])));
}

#[test]
fn selections_on_an_empty_sheet_and_past_the_end_travel() {
    let mut peers = room(2, 11);
    let before = peers[0].rep.full_state();
    // "Notes" (sheet 1) has no row or column ids yet: the selection is named by block ids.
    let range = RangeRef::new(CellRef::new(1, 2), CellRef::new(70, 3));
    let sel = peers[0].rep.selection_to_ids(1, &[range]).expect("ids");
    assert_eq!(sel["ranges"][0], serde_json::json!(["~0.1", "~0.2", "~1.6", "~0.3"]));
    assert_eq!(peers[1].rep.selection_from_ids(&sel), Some((1, vec![range])));
    // Naming them wrote nothing.
    assert_eq!(peers[0].rep.full_state(), before);
    // Past the materialized rows of a non-empty sheet (64 row ids): rows 100 to 200 and a whole
    // column.
    let far = RangeRef::new(CellRef::new(100, 1), CellRef::new(200, 1));
    let whole = RangeRef::new(CellRef::new(0, 4), CellRef::new(gridcraft_core::MAX_ROWS - 1, 4));
    let sel = peers[0].rep.selection_to_ids(0, &[far, whole]).expect("ids");
    assert_eq!(peers[1].rep.selection_from_ids(&sel), Some((0, vec![far, whole])));
    // Once the sheet grows there (peer 1 types in row 150 of Notes), the same ids resolve to the
    // same cells on both sides.
    let sel = peers[0].rep.selection_to_ids(1, &[RangeRef::new(CellRef::new(149, 0), CellRef::new(160, 0))]).expect("ids");
    let up = peers[1]
        .edit(|wb| {
            Arc::make_mut(&mut wb.sheets[1]).set_value(CellRef::new(149, 0), Value::Text("here".into()));
            vec![]
        })
        .expect("update");
    let want = Some((1, vec![RangeRef::new(CellRef::new(149, 0), CellRef::new(160, 0))]));
    assert_eq!(peers[1].rep.selection_from_ids(&sel), want);
    peers[0].receive(&up);
    assert_eq!(peers[0].rep.selection_from_ids(&sel), want);
    // Garbage and deleted blocks resolve to nothing, not a crash.
    let bad = serde_json::json!({"sheet": sel["sheet"], "ranges": [["~0.64", "~0.0", "~0.0", "~0.0"], ["~99999999999.1", "~0.0", "~0.0", "~0.0"], ["~x.1", "~0.0", "~0.0", "~0.0"]]});
    assert_eq!(peers[0].rep.selection_from_ids(&bad), Some((1, vec![])));
}

// ---------------------------------------------------------------- document chat

fn say(p: &mut Peer, id: &str, who: &str, text: &str) -> Option<Vec<u8>> {
    let author = gridcraft_model::ChatAuthor { id: format!("u-{who}"), name: who.into(), color: "#3B7DD8".into() };
    p.edit(|wb| {
        let mut c = (*wb.chat).clone();
        c.send(gridcraft_model::ChatMessage { id: id.into(), author, at: 1_000, text: Some(text.into()), ..Default::default() }).expect("send");
        wb.chat = Arc::new(c);
        vec![]
    })
}

fn chat_texts(p: &Peer) -> Vec<String> {
    p.wb.chat.messages.iter().map(|m| m.text.clone().unwrap_or_default()).collect()
}

#[test]
fn chat_seeds_joins_and_converges_on_three_peers() {
    let mut wb = sample();
    let mut c = gridcraft_model::Chat::default();
    let ann = gridcraft_model::ChatAuthor { id: "u-ann".into(), name: "Ann".into(), color: "#3B7DD8".into() };
    c.send(gridcraft_model::ChatMessage { id: "note0000".into(), author: ann, at: 5, text: Some("from the file".into()), ..Default::default() })
        .expect("send");
    wb.chat = Arc::new(c);
    let mut first = Replica::new(41);
    let wb = Arc::new(wb);
    first.seed(wb.clone(), "Budget").expect("seed");
    let state = first.full_state();
    let mut peers = vec![Peer { rep: first, wb, inbox: vec![] }];
    for i in 1..3 {
        let mut p = Peer { rep: Replica::new(41 + i), wb: Arc::new(Workbook::new()), inbox: vec![] };
        p.receive(&state);
        assert_eq!(chat_texts(&p), ["from the file"], "joining brings the room's chat in");
        peers.push(p);
    }
    // Everyone sends at once; updates arrive in different orders.
    let mut ups = vec![];
    for (i, p) in peers.iter_mut().enumerate() {
        let id = format!("msg{i}0000");
        ups.push((i, say(p, &id, &format!("p{i}"), &format!("hello from {i}")).expect("an update")));
    }
    for (to, p) in peers.iter_mut().enumerate() {
        let mut mine: Vec<&(usize, Vec<u8>)> = ups.iter().filter(|(from, _)| *from != to).collect();
        if to == 1 {
            mine.reverse();
        }
        for (_, u) in mine {
            p.receive(u);
        }
    }
    let order = chat_texts(&peers[0]);
    assert_eq!(order.len(), 4);
    assert_eq!(chat_texts(&peers[1]), order);
    assert_eq!(chat_texts(&peers[2]), order);
    assert_eq!(fingerprint(&peers[1].wb), fingerprint(&peers[2].wb));
    // The model reads what the document holds.
    assert_eq!(*peers[2].wb.chat, peers[2].rep.chat());
}

#[test]
fn chat_clear_and_undo() {
    let mut peers = room(2, 3);
    let u = say(&mut peers[0], "aaaaaaaa", "ann", "one").expect("update");
    peers[1].receive(&u);
    // An edit, then a message: undo reverts the edit only.
    let u = peers[0].edit(|wb| {
        Arc::make_mut(&mut wb.sheets[0]).set_value(CellRef::new(20, 0), Value::Number(1.0));
        vec![]
    });
    peers[1].receive(&u.expect("update"));
    let u = say(&mut peers[0], "bbbbbbbb", "ann", "two").expect("update");
    peers[1].receive(&u);
    let undo = peers[0].rep.undo().expect("undo").expect("an undo step");
    peers[0].refresh();
    peers[1].receive(&undo);
    assert_eq!(chat_texts(&peers[0]), ["one", "two"], "undo never touches the chat");
    assert_eq!(chat_texts(&peers[1]), ["one", "two"]);
    assert!(peers[1].wb.sheets[0].cells.get(CellRef::new(20, 0)).is_none());
    // Bob clears while Ann sends: Ann's new message stays everywhere.
    let clear = peers[1].edit(|wb| {
        let mut c = (*wb.chat).clone();
        c.clear(9_000);
        wb.chat = Arc::new(c);
        vec![]
    });
    let sent = say(&mut peers[0], "cccccccc", "ann", "meanwhile").expect("update");
    peers[0].receive(&clear.expect("update"));
    peers[1].receive(&sent);
    assert_eq!(chat_texts(&peers[0]), ["meanwhile"]);
    assert_eq!(chat_texts(&peers[1]), ["meanwhile"]);
    assert_eq!(peers[0].wb.chat.cleared_at, Some(9_000));
}
