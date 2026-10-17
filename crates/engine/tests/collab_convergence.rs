//! Co-editing convergence (protocol 2): two and three sessions run random command sequences
//! concurrently on one shared workbook, their updates delivered in random order and sometimes
//! twice, and must end with identical workbooks — the same shared content (schema 1) and the
//! same calculated values — equal to what a newcomer joining afterwards builds. Undo and Redo
//! are per-user and part of the mix, and so is the document chat (sends and clears).
//!
//! The sessions are connected in memory ([`collab::IN_MEMORY`]): the test is the transport.
//!
//! Knobs: `GRIDCRAFT_COLLAB_SEEDS` (default 6), `GRIDCRAFT_COLLAB_STEPS` (default 60),
//! `GRIDCRAFT_COLLAB_SEED` (replay one).

use gridcraft_core::{CellRef, RangeRef};
use gridcraft_engine::collab::{self, IN_MEMORY};
use gridcraft_engine::plugin::push::{Incoming, Pushed};
use gridcraft_engine::{DocState, Session, sample};
use serde_json::{Value as Json, json};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

fn env_num(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

fn letter(c: usize) -> String {
    gridcraft_core::col_to_letters(c as u32)
}

fn cell(rng: &mut Rng) -> String {
    format!("{}{}", letter(rng.below(8)), rng.below(18) + 1)
}

fn range(rng: &mut Rng) -> String {
    let (c, r) = (rng.below(7), rng.below(16));
    RangeRef::new(CellRef::new(r as u32, c as u32), CellRef::new((r + rng.below(3)) as u32, (c + rng.below(2)) as u32)).a1()
}

fn input(rng: &mut Rng, tag: &str) -> String {
    match rng.below(9) {
        0 => format!("={}{}*2", letter(rng.below(5)), rng.below(12) + 1),
        1 => format!("=SUM({}1:{}9)", letter(rng.below(4)), letter(rng.below(4))),
        2 => "007".into(),
        3 => "TRUE".into(),
        4 => String::new(),
        5 => format!("{tag}{}", rng.below(50)),
        _ => format!("{}", rng.below(1000) as f64 / 4.0),
    }
}

fn sheet_of(rng: &mut Rng, s: &Session) -> Json {
    let n = s.doc().map(|d| d.wb.sheets.len()).unwrap_or(1);
    json!(rng.below(n))
}

/// A random edit, as a person would make it.
fn generate(rng: &mut Rng, s: &Session, tag: &str) -> (String, Json) {
    let (id, p) = match rng.below(33) {
        0..=9 => ("cell.set", json!({"cell": cell(rng), "input": input(rng, tag)})),
        10 => ("home.bold", json!({"range": range(rng)})),
        11 => ("home.fillColor", json!({"range": range(rng), "color": rng.pick(&["#FF0000", "#00B050", "none"])})),
        12 => ("home.numberFormat", json!({"range": range(rng), "format": rng.pick(&["Currency", "Percentage", "General"])})),
        13 => ("edit.clearContents", json!({"range": range(rng)})),
        14 | 15 => ("home.insertRows", json!({"rows": format!("{0}:{0}", rng.below(16) + 1)})),
        16 | 17 => ("home.deleteRows", json!({"rows": format!("{0}:{0}", rng.below(16) + 1)})),
        18 => ("home.insertColumns", json!({"cols": format!("{0}:{0}", letter(rng.below(6)))})),
        19 => ("home.deleteColumns", json!({"cols": format!("{0}:{0}", letter(rng.below(6)))})),
        20 => ("home.mergeCells", json!({"range": range(rng)})),
        21 => ("home.unmergeCells", json!({"range": range(rng)})),
        22 => ("home.insertSheet", json!({"name": format!("{tag}{}", rng.below(20))})),
        23 => ("home.deleteSheet", json!({"sheet": sheet_of(rng, s)})),
        24 => ("sheet.rename", json!({"sheet": sheet_of(rng, s), "name": format!("{tag}r{}", rng.below(20))})),
        25 => ("sheet.activate", json!({"sheet": sheet_of(rng, s)})),
        26 => ("home.rowHeight", json!({"rows": format!("{0}:{0}", rng.below(16) + 1), "height": 10 + rng.below(30)})),
        27 => ("edit.undo", json!({})),
        28 => ("edit.redo", json!({})),
        // Document chat: concurrent messages land in one order everywhere; a clear removes the
        // messages its sender had, not the ones sent meanwhile.
        29 | 30 => ("chat.send", json!({"text": format!("{tag} says {}", rng.below(100))})),
        31 => ("chat.clear", json!({})),
        _ => ("selection.set", json!({"range": range(rng)})),
    };
    (id.to_string(), p)
}

struct Peer {
    s: Session,
    now: u64,
}

impl Peer {
    fn deliver(&mut self, msg: Json) {
        if trace() {
            eprintln!("  deliver {}", msg.to_string().chars().take(200).collect::<String>());
        }
        self.now += 7;
        self.s.collab_poll_pushed(self.now, vec![Pushed { plugin: IN_MEMORY.into(), incoming: Incoming::Message(msg) }]);
    }
}

/// App → transport messages become transport → app messages for every other peer.
fn route(peers: &mut [Peer], from: usize, queue: &mut Vec<(usize, Json)>, rng: &mut Rng) {
    for msg in peers[from].s.collab_take_outgoing() {
        let method = msg["method"].as_str().unwrap_or("");
        if method == "collab/joined" || method == "collab/leave" {
            continue;
        }
        for to in 0..peers.len() {
            if to != from {
                queue.push((to, msg.clone()));
                if rng.chance(8) {
                    queue.push((to, msg.clone()));
                }
            }
        }
    }
}

fn settle(peers: &mut [Peer], queue: &mut Vec<(usize, Json)>, rng: &mut Rng) {
    for _ in 0..10_000 {
        if queue.is_empty() {
            break;
        }
        let k = rng.below(queue.len());
        let (to, msg) = queue.remove(k);
        peers[to].deliver(msg);
        route(peers, to, queue, rng);
    }
}

/// The calculated values too: every peer recalculates the same inputs alike.
fn values(d: &DocState) -> String {
    let mut out = String::new();
    for sh in &d.wb.sheets {
        out.push_str(&format!("{}\n", sh.name));
        for (pos, _) in sh.cells.iter() {
            out.push_str(&format!("  {} {:?}\n", pos.a1(), sh.value(pos)));
        }
    }
    out
}

fn run(seed: u64, n: usize, steps: usize) -> Result<(), String> {
    let mut rng = Rng(seed.wrapping_mul(0xA24B_AED4_963E_E407) ^ 0xC011AB);
    let sample = *rng.pick(&["sales", "budget", "grades"]);
    let mut peers: Vec<Peer> = (0..n).map(|_| Peer { s: Session::new(), now: 1_000 }).collect();
    let wb = sample::build(sample).ok_or("no sample")?;
    peers[0].s.add_document(DocState::new(wb, None, sample::title(sample)));
    peers[0].s.collab_join_in_memory(json!({"room": "fuzz", "mode": "share"})).map_err(|e| e.to_string())?;
    for p in peers.iter_mut().skip(1) {
        p.s.collab_join_in_memory(json!({"room": "fuzz", "mode": "join"})).map_err(|e| e.to_string())?;
    }
    let mut queue = Vec::new();
    for i in 0..n {
        route(&mut peers, i, &mut queue, &mut rng);
    }
    settle(&mut peers, &mut queue, &mut rng);
    for (i, p) in peers.iter().enumerate() {
        if collab::read_only(&p.s).is_some() {
            return Err(format!("peer {i} is still joining after the first sync"));
        }
    }
    let mut log = Vec::new();
    for step in 0..steps {
        let i = rng.below(n);
        if rng.chance(55) || queue.is_empty() {
            let tag = format!("p{i}s{step}-");
            let (id, p) = generate(&mut rng, &peers[i].s, &tag);
            if trace() {
                eprintln!("{step}: peer {i} {id} {p}");
            }
            let r = peers[i].s.execute(&id, p.clone());
            log.push(format!("{step}: peer {i} {id} {p} → {}", if r.is_ok() { "ok".to_string() } else { format!("{:?}", r.err()) }));
            route(&mut peers, i, &mut queue, &mut rng);
        } else {
            let k = rng.below(queue.len());
            let (to, msg) = queue.remove(k);
            if trace() {
                eprintln!("{step}: peer {to} gets {}", msg["method"]);
            }
            peers[to].deliver(msg);
            route(&mut peers, to, &mut queue, &mut rng);
        }
    }
    settle(&mut peers, &mut queue, &mut rng);
    // A sync round, as when a transport comes back online.
    for i in 0..n {
        peers[i].deliver(json!({"jsonrpc": "2.0", "method": "collab/status", "params": {"room": "fuzz", "state": "offline"}}));
        peers[i].deliver(json!({"jsonrpc": "2.0", "method": "collab/status", "params": {"room": "fuzz", "state": "online"}}));
        route(&mut peers, i, &mut queue, &mut rng);
    }
    settle(&mut peers, &mut queue, &mut rng);
    let fp = |p: &Peer| collab::fingerprint(&p.s.doc().map(|d| d.wb.clone()).unwrap_or_default());
    let first = fp(&peers[0]);
    let first_values = peers[0].s.doc().map(values).unwrap_or_default();
    for (i, p) in peers.iter().enumerate().skip(1) {
        if fp(p) != first {
            return Err(format!("peer {i} differs from peer 0\n{}\n--- steps\n{}", diff(&first, &fp(p)), log.join("\n")));
        }
        let v = p.s.doc().map(values).unwrap_or_default();
        if v != first_values {
            return Err(format!("peer {i}'s values differ from peer 0's\n{}\n--- steps\n{}", diff(&first_values, &v), log.join("\n")));
        }
    }
    // A newcomer gets the same workbook.
    let mut late = Peer { s: Session::new(), now: 1_000 };
    late.s.collab_join_in_memory(json!({"room": "fuzz", "mode": "join"})).map_err(|e| e.to_string())?;
    peers.push(late);
    let last = peers.len() - 1;
    route(&mut peers, last, &mut queue, &mut rng);
    settle(&mut peers, &mut queue, &mut rng);
    if fp(&peers[last]) != first {
        return Err(format!("a newcomer differs\n{}", diff(&first, &fp(&peers[last]))));
    }
    Ok(())
}

fn trace() -> bool {
    std::env::var_os("GRIDCRAFT_COLLAB_TRACE").is_some()
}

fn diff(a: &str, b: &str) -> String {
    let la: Vec<&str> = a.lines().collect();
    let lb: Vec<&str> = b.lines().collect();
    let mut out = String::new();
    for l in la.iter().filter(|l| !lb.contains(l)).take(20) {
        out.push_str(&format!("- {}\n", l.chars().take(160).collect::<String>()));
    }
    for l in lb.iter().filter(|l| !la.contains(l)).take(20) {
        out.push_str(&format!("+ {}\n", l.chars().take(160).collect::<String>()));
    }
    out
}

fn seeds() -> Vec<u64> {
    match std::env::var("GRIDCRAFT_COLLAB_SEED").ok().and_then(|v| v.trim().parse().ok()) {
        Some(one) => vec![one],
        None => (0..env_num("GRIDCRAFT_COLLAB_SEEDS", 6)).collect(),
    }
}

#[test]
fn two_sessions_converge() {
    let steps = env_num("GRIDCRAFT_COLLAB_STEPS", 60) as usize;
    for seed in seeds() {
        if let Err(e) = run(seed, 2, steps) {
            panic!("seed {seed} (GRIDCRAFT_COLLAB_SEED={seed}): {e}");
        }
    }
}

#[test]
fn three_sessions_converge() {
    let steps = env_num("GRIDCRAFT_COLLAB_STEPS", 60) as usize;
    for seed in seeds() {
        if let Err(e) = run(seed + 1_000, 3, steps) {
            panic!("seed {} (GRIDCRAFT_COLLAB_SEED={}): {e}", seed + 1_000, seed + 1_000);
        }
    }
}
