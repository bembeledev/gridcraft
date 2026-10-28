//! Conditional formatting evaluation: what extra style, data bar, colour scale or icon a cell
//! gets.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};

use gridcraft_core::{CellRef, RangeRef, Value, compare};
use gridcraft_model::{CfOperator, CfRule, CfValueKind, Color, CondFormat, PatternType, Sheet, Style, Underline, Workbook, default_icon_thresholds};

/// The conditional look of one cell.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CfLook {
    /// Differential style (only non-default parts apply).
    pub style: Option<Style>,
    /// Fill from a colour scale (RGB).
    pub scale_fill: Option<[u8; 3]>,
    /// Data bar: fraction 0..=1 of the cell width, colour, gradient.
    pub bar: Option<(f32, [u8; 3], bool)>,
    /// Icon: set name and index (0 = lowest).
    pub icon: Option<(String, usize)>,
    pub hide_value: bool,
}

/// Bounded, document-owned paint results. Weak identity preserves workbook copy-on-write.
#[derive(Clone, Debug, Default)]
pub struct RenderCache(Arc<Mutex<PaintResults>>);
#[derive(Debug, Default)]
struct PaintResults {
    workbook: Weak<Workbook>,
    revision: u64,
    start_ms: Option<f64>,
    values: HashMap<(usize, CellRef), Option<CfLook>>,
}
const MAX_PAINT_RESULTS: usize = 4096;
const PAINT_CACHE_MS: f64 = 1000.0;

/// Precomputed statistics per rule (min/max/percentiles/top-N thresholds/duplicates).
pub struct CfCache {
    stats: HashMap<usize, RuleStats>,
    frame_meter: crate::workload::Meter,
    results: Option<RenderCache>,
}

impl Default for CfCache {
    fn default() -> Self {
        Self::with_budget(crate::workload::AnalysisBudget::rendering())
    }
}

#[derive(Default, Clone)]
struct RuleStats {
    sorted: Vec<f64>,
    avg: f64,
    std: f64,
    counts: HashMap<String, usize>,
}

/// Which statistics a rule reads. Scanning the rule's whole range for them costs O(range), so
/// rules that read none (cell value, formula, text, blanks, errors, dates) never scan.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StatsNeed {
    /// Sorted numbers, their mean and deviation (top/bottom N, above average, scales, bars, icons).
    Numbers,
    /// How often each displayed text occurs (duplicate / unique values).
    Counts,
}

fn stats_need(rule: &CfRule) -> Option<StatsNeed> {
    match rule {
        CfRule::Duplicate { .. } | CfRule::Unique { .. } => Some(StatsNeed::Counts),
        CfRule::Top10 { .. } | CfRule::AboveAverage { .. } | CfRule::ColorScale { .. } | CfRule::DataBar { .. } | CfRule::IconSet { .. } => {
            Some(StatsNeed::Numbers)
        }
        _ => None,
    }
}

impl RuleStats {
    fn build(sh: &Sheet, ranges: &[RangeRef], need: StatsNeed) -> RuleStats {
        let (sorted, counts) = numbers_in(sh, ranges, need);
        let n = sorted.len().max(1) as f64;
        let avg = sorted.iter().sum::<f64>() / n;
        let std = (sorted.iter().map(|x| (x - avg).powi(2)).sum::<f64>() / n).sqrt();
        RuleStats { sorted, avg, std, counts }
    }
}

#[cfg(test)]
thread_local! {
    /// Cells `numbers_in` visited on this thread (tests count the work a lookup does).
    static SCANNED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn numbers_in(sh: &Sheet, ranges: &[RangeRef], need: StatsNeed) -> (Vec<f64>, HashMap<String, usize>) {
    let mut nums = Vec::new();
    let mut counts = HashMap::new();
    for r in ranges {
        let r = match sh.used_range().and_then(|u| r.intersection(&u)) {
            Some(x) => x,
            None => continue,
        };
        for c in r.iter().take(2_000_000) {
            #[cfg(test)]
            SCANNED.with(|n| n.set(n.get() + 1));
            let v = sh.value(c);
            match need {
                StatsNeed::Numbers => {
                    if let Value::Number(n) = v {
                        nums.push(n);
                    }
                }
                StatsNeed::Counts => {
                    if !v.is_empty() {
                        *counts.entry(v.display().to_lowercase()).or_insert(0) += 1;
                    }
                }
            }
        }
    }
    nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    (nums, counts)
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (p / 100.0).clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    let a = sorted.get(lo).copied().unwrap_or(0.0);
    let b = sorted.get(hi).copied().unwrap_or(a);
    a + (b - a) * (rank - lo as f64)
}

fn threshold(wb: &Workbook, sheet: usize, at: CellRef, st: &RuleStats, k: &CfValueKind) -> f64 {
    let min = st.sorted.first().copied().unwrap_or(0.0);
    let max = st.sorted.last().copied().unwrap_or(0.0);
    match k {
        CfValueKind::Min => min,
        CfValueKind::Max => max,
        CfValueKind::Number(n) => *n,
        CfValueKind::Percent(p) => min + (max - min) * p / 100.0,
        CfValueKind::Percentile(p) => percentile(&st.sorted, *p),
        CfValueKind::Formula(f) => gridcraft_calc::evaluate(wb, sheet, at, f).to_number().unwrap_or(0.0),
    }
}

fn lerp(a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        (a[0] as f64 + (b[0] as f64 - a[0] as f64) * t).round() as u8,
        (a[1] as f64 + (b[1] as f64 - a[1] as f64) * t).round() as u8,
        (a[2] as f64 + (b[2] as f64 - a[2] as f64) * t).round() as u8,
    ]
}

pub use gridcraft_model::icon_count;

impl CfCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Uses a document-owned bank so constructing a fresh paint cache cannot reset allowance.
    pub fn with_budget(budget: crate::workload::AnalysisBudget) -> Self {
        Self { stats: HashMap::new(), frame_meter: crate::workload::Meter::new(budget), results: None }
    }

    /// Reuses results across frames for one revision, with a one-second TTL for volatile formulas.
    pub fn with_cache(budget: crate::workload::AnalysisBudget, results: RenderCache, wb: &Arc<Workbook>, revision: u64) -> Self {
        Self::with_cache_at(budget, results, wb, revision, gridcraft_core::date::unix_now_ms())
    }

    fn with_cache_at(budget: crate::workload::AnalysisBudget, results: RenderCache, wb: &Arc<Workbook>, revision: u64, now: Option<f64>) -> Self {
        let now = now.filter(|time| time.is_finite());
        let usable = if let Ok(mut stored) = results.0.lock() {
            let fresh = match (stored.start_ms, now) {
                (Some(start), Some(now)) => now >= start && now - start < PAINT_CACHE_MS,
                _ => false,
            };
            if !Weak::ptr_eq(&stored.workbook, &Arc::downgrade(wb)) || stored.revision != revision || !fresh {
                stored.values.clear();
                stored.workbook = Arc::downgrade(wb);
                stored.revision = revision;
                stored.start_ms = now;
            }
            now.is_some()
        } else {
            false
        };
        Self { stats: HashMap::new(), frame_meter: crate::workload::Meter::new(budget), results: usable.then_some(results) }
    }

    /// Conditional look of `c`, or `None` when no rule applies.
    pub fn look(&mut self, wb: &Workbook, sheet: usize, c: CellRef) -> Option<CfLook> {
        if !gridcraft_calc::WorkHook::allowed() {
            return None;
        }
        if let Some(results) = &self.results
            && let Ok(stored) = results.0.lock()
            && std::ptr::eq(stored.workbook.as_ptr(), wb)
            && let Some(value) = stored.values.get(&(sheet, c))
        {
            return value.clone();
        }
        let value = self.look_uncached(wb, sheet, c);
        if let Some(results) = &self.results
            && let Ok(mut stored) = results.0.lock()
            && std::ptr::eq(stored.workbook.as_ptr(), wb)
        {
            if stored.values.len() >= MAX_PAINT_RESULTS {
                stored.values.clear();
            }
            stored.values.insert((sheet, c), value.clone());
        }
        value
    }

    fn look_uncached(&mut self, wb: &Workbook, sheet: usize, c: CellRef) -> Option<CfLook> {
        let sh = wb.sheet(sheet)?;
        if sh.cond_formats.is_empty() {
            return None;
        }
        // One frame shares prepaid credit between misses instead of discarding a reservation
        // per cell. The independent per-cell meter rejects oversized native work before it
        // can reserve the document bank. A fresh hook keeps a local refusal cell-scoped.
        let local = crate::workload::Meter::ordinary();
        local.set_limit(100_000).ok()?;
        let frame = self.frame_meter.clone();
        let parent = gridcraft_calc::WorkHook::current();
        let combined = gridcraft_calc::WorkHook::new(move |units| {
            local.charge(units).is_ok() && parent.as_ref().is_none_or(|policy| policy.charge(units)) && frame.charge(units).is_ok()
        });
        let _scope = combined.enter()?;
        if !gridcraft_calc::WorkHook::allowed() {
            return None;
        }
        let mut out = CfLook::default();
        let mut any = false;
        let mut rules: Vec<(usize, &CondFormat)> =
            sh.cond_formats.iter().enumerate().filter(|(_, cf)| cf.ranges.iter().any(|r| r.contains(c))).collect();
        rules.sort_by_key(|(_, cf)| cf.priority);
        let v = sh.value(c);
        let no_stats = RuleStats::default();
        for (idx, cf) in rules {
            let first = cf.ranges.first().map(|r| r.start).unwrap_or(c);
            let st: &RuleStats = match stats_need(&cf.rule) {
                Some(need) => &*self.stats.entry(idx).or_insert_with(|| RuleStats::build(sh, &cf.ranges, need)),
                None => &no_stats,
            };
            let text = v.display();
            let matched_style: Option<&Style> = match &cf.rule {
                CfRule::CellIs { op, a, b, style } => {
                    let eval = |f: &str| gridcraft_calc::evaluate(wb, sheet, c, &relative(f, first, c));
                    let av = eval(a);
                    let ok = match op {
                        CfOperator::Between | CfOperator::NotBetween => {
                            let bv = b.as_deref().map(eval).unwrap_or_default();
                            let (lo, hi) = if compare(&av, &bv) == std::cmp::Ordering::Greater { (bv, av) } else { (av, bv) };
                            let inside = compare(&v, &lo) != std::cmp::Ordering::Less && compare(&v, &hi) != std::cmp::Ordering::Greater;
                            if *op == CfOperator::Between { inside } else { !inside }
                        }
                        CfOperator::Equal => compare(&v, &av) == std::cmp::Ordering::Equal,
                        CfOperator::NotEqual => compare(&v, &av) != std::cmp::Ordering::Equal,
                        CfOperator::Greater => compare(&v, &av) == std::cmp::Ordering::Greater,
                        CfOperator::Less => compare(&v, &av) == std::cmp::Ordering::Less,
                        CfOperator::GreaterOrEqual => compare(&v, &av) != std::cmp::Ordering::Less,
                        CfOperator::LessOrEqual => compare(&v, &av) != std::cmp::Ordering::Greater,
                    };
                    // As in Excel, a blank cell compares as 0 (or ""), and an error never matches.
                    (ok && !v.is_error()).then_some(style.as_ref())
                }
                CfRule::Expression { formula, style } => {
                    gridcraft_calc::evaluate(wb, sheet, c, &relative(formula, first, c)).to_bool().unwrap_or(false).then_some(style.as_ref())
                }
                // As in Excel, an error value never contains, begins or ends with anything (its rule
                // formulas SEARCH/LEFT/RIGHT fail on it), so it always "does not contain" (#1162).
                CfRule::ContainsText { text: t, style } => {
                    (!v.is_error() && text.to_lowercase().contains(&t.to_lowercase())).then_some(style.as_ref())
                }
                CfRule::NotContainsText { text: t, style } => {
                    (v.is_error() || !text.to_lowercase().contains(&t.to_lowercase())).then_some(style.as_ref())
                }
                CfRule::BeginsWith { text: t, style } => {
                    (!v.is_error() && text.to_lowercase().starts_with(&t.to_lowercase())).then_some(style.as_ref())
                }
                CfRule::EndsWith { text: t, style } => (!v.is_error() && text.to_lowercase().ends_with(&t.to_lowercase())).then_some(style.as_ref()),
                CfRule::Blanks { style } => (v.is_empty() || text.trim().is_empty()).then_some(style.as_ref()),
                CfRule::NoBlanks { style } => (!v.is_empty() && !text.trim().is_empty()).then_some(style.as_ref()),
                CfRule::Errors { style } => v.is_error().then_some(style.as_ref()),
                CfRule::NoErrors { style } => (!v.is_error()).then_some(style.as_ref()),
                CfRule::Duplicate { style } => {
                    (!v.is_empty() && st.counts.get(&text.to_lowercase()).copied().unwrap_or(0) > 1).then_some(style.as_ref())
                }
                CfRule::Unique { style } => {
                    (!v.is_empty() && st.counts.get(&text.to_lowercase()).copied().unwrap_or(0) == 1).then_some(style.as_ref())
                }
                CfRule::Top10 { bottom, percent, rank, style } => match v.as_f64() {
                    Some(n) if !st.sorted.is_empty() => {
                        let k = if *percent { ((st.sorted.len() as f64) * (*rank as f64) / 100.0).floor().max(1.0) as usize } else { *rank as usize };
                        let k = k.clamp(1, st.sorted.len());
                        let ok = if *bottom {
                            n <= st.sorted.get(k - 1).copied().unwrap_or(n)
                        } else {
                            n >= st.sorted.get(st.sorted.len() - k).copied().unwrap_or(n)
                        };
                        ok.then_some(style.as_ref())
                    }
                    _ => None,
                },
                CfRule::AboveAverage { below, equal, std_dev, style } => match v.as_f64() {
                    Some(n) => {
                        let thr = st.avg + if *below { -1.0 } else { 1.0 } * st.std * *std_dev as f64;
                        let ok = if *below { n < thr || (*equal && n == thr) } else { n > thr || (*equal && n == thr) };
                        ok.then_some(style.as_ref())
                    }
                    None => None,
                },
                CfRule::TimePeriod { period, style } => match v.as_f64() {
                    Some(n) => {
                        let today = gridcraft_core::date::serial_in_system(wb.date_system, gridcraft_calc::now_serial()).floor();
                        let d = n.floor();
                        let wd = gridcraft_core::date::datetime_from_serial(wb.date_system, today).map(|x| x.weekday as f64).unwrap_or(0.0);
                        let week_start = today - wd;
                        let (ty, tm) =
                            gridcraft_core::date::datetime_from_serial(wb.date_system, today).map(|x| (x.year, x.month)).unwrap_or((2026, 1));
                        let ym = gridcraft_core::date::datetime_from_serial(wb.date_system, d).map(|x| (x.year, x.month));
                        let month_off = |k: i32| {
                            let m = tm as i32 + k;
                            let y = ty + (m - 1).div_euclid(12);
                            let m = (m - 1).rem_euclid(12) + 1;
                            ym == Some((y, m as u32))
                        };
                        let ok = match period.as_str() {
                            "yesterday" => d == today - 1.0,
                            "today" => d == today,
                            "tomorrow" => d == today + 1.0,
                            "last7Days" => d > today - 7.0 && d <= today,
                            "lastWeek" => d >= week_start - 7.0 && d < week_start,
                            "thisWeek" => d >= week_start && d < week_start + 7.0,
                            "nextWeek" => d >= week_start + 7.0 && d < week_start + 14.0,
                            "lastMonth" => month_off(-1),
                            "thisMonth" => month_off(0),
                            "nextMonth" => month_off(1),
                            _ => false,
                        };
                        ok.then_some(style.as_ref())
                    }
                    None => None,
                },
                CfRule::ColorScale { stops } => {
                    if let Some(n) = v.as_f64() {
                        let pts: Vec<(f64, [u8; 3])> = stops
                            .iter()
                            .map(|(k, col)| (threshold(wb, sheet, c, st, k), col.resolve(&wb.theme).unwrap_or([255, 255, 255])))
                            .collect();
                        let rgb = match pts.as_slice() {
                            [(a, ca), (b, cb)] => lerp(*ca, *cb, if b > a { (n - a) / (b - a) } else { 0.0 }),
                            [(a, ca), (m, cm), (b, cb)] => {
                                if n <= *m {
                                    lerp(*ca, *cm, if m > a { (n - a) / (m - a) } else { 0.0 })
                                } else {
                                    lerp(*cm, *cb, if b > m { (n - m) / (b - m) } else { 1.0 })
                                }
                            }
                            _ => [255, 255, 255],
                        };
                        if out.scale_fill.is_none() {
                            out.scale_fill = Some(rgb);
                            any = true;
                        }
                    }
                    None
                }
                CfRule::DataBar { min, max, color, gradient, show_value } => {
                    if let Some(n) = v.as_f64() {
                        let lo = threshold(wb, sheet, c, st, min);
                        let hi = threshold(wb, sheet, c, st, max);
                        let lo = if lo > 0.0 { 0.0 } else { lo };
                        let frac = if hi > lo { ((n - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 1.0 };
                        if out.bar.is_none() {
                            out.bar = Some((
                                (frac as f32) * 0.9 + 0.1 * (frac > 0.0) as u8 as f32,
                                color.resolve(&wb.theme).unwrap_or([0x63, 0x8E, 0xC6]),
                                *gradient,
                            ));
                            out.hide_value |= !show_value;
                            any = true;
                        }
                    }
                    None
                }
                CfRule::IconSet { set, thresholds, reverse, show_value } => {
                    if let Some(n) = v.as_f64() {
                        let k = icon_count(set);
                        // One threshold per icon, the first being the lower bound (as in the file);
                        // also accept the lower bound left out, else Excel's defaults.
                        let defaults;
                        let lower = match thresholds.len() {
                            n if n == k => thresholds.get(1..).unwrap_or_default(),
                            n if n + 1 == k => thresholds.as_slice(),
                            _ => {
                                defaults = default_icon_thresholds(set);
                                defaults.get(1..).unwrap_or_default()
                            }
                        };
                        let cuts: Vec<f64> = lower.iter().map(|t| threshold(wb, sheet, c, st, t)).collect();
                        let mut idx = cuts.iter().filter(|cut| n >= **cut).count();
                        if *reverse {
                            idx = k - 1 - idx.min(k - 1);
                        }
                        if out.icon.is_none() {
                            out.icon = Some((set.clone(), idx.min(k - 1)));
                            out.hide_value |= !show_value;
                            any = true;
                        }
                    }
                    None
                }
            };
            if !gridcraft_calc::WorkHook::allowed() {
                return None;
            }
            if let Some(st) = matched_style {
                any = true;
                // Every matching rule applies; where two set the same part, the rule with the
                // higher priority (seen first) wins.
                match &mut out.style {
                    None => out.style = Some(st.clone()),
                    Some(prev) => layer_under(prev, st),
                }
                if cf.stop_if_true {
                    break;
                }
            }
        }
        any.then_some(out)
    }
}

/// Adds the parts `lower` sets that `style` leaves unset (differential styles: default parts
/// are unset).
fn layer_under(style: &mut Style, lower: &Style) {
    if style.fill.pattern == PatternType::None {
        style.fill = lower.fill;
    }
    if style.font.color == Color::Auto {
        style.font.color = lower.font.color;
    }
    style.font.bold |= lower.font.bold;
    style.font.italic |= lower.font.italic;
    style.font.strike |= lower.font.strike;
    if style.font.underline == Underline::None {
        style.font.underline = lower.font.underline;
    }
    let b = &mut style.borders;
    for (dst, src) in [
        (&mut b.left, lower.borders.left),
        (&mut b.right, lower.borders.right),
        (&mut b.top, lower.borders.top),
        (&mut b.bottom, lower.borders.bottom),
        (&mut b.diag_down, lower.borders.diag_down),
        (&mut b.diag_up, lower.borders.diag_up),
    ] {
        if dst.is_none() {
            *dst = src;
        }
    }
    if style.num_fmt.as_str() == "General" {
        style.num_fmt = lower.num_fmt.clone();
    }
}

/// Shifts a rule formula written for the top-left cell of the range to `c`.
fn relative(f: &str, first: CellRef, c: CellRef) -> String {
    if first == c {
        return f.to_string();
    }
    match gridcraft_formula::parse(f.trim_start_matches('=')) {
        Ok(e) => {
            gridcraft_formula::print(&gridcraft_formula::adjust::shift_relative(e, c.row as i64 - first.row as i64, c.col as i64 - first.col as i64))
        }
        Err(_) => f.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A frame shares prepaid credit across misses instead of reserving 4096 units per cell.
    #[test]
    fn one_frame_can_style_a_thousand_uncached_cells_with_a_small_bank() {
        let mut s = crate::Session::new();
        s.new_workbook();
        s.execute("home.conditionalFormat", json!({"range":"A1:T50","rule":{"type":"expression","formula":"=1+1=2","preset":"lightRedFill"}}))
            .unwrap();
        let d = s.doc().unwrap();
        assert_eq!(d.render_budget.remaining().unwrap(), 1_000_000);
        let mut cache = CfCache::with_budget(d.render_budget.clone());
        for row in 0..50 {
            for col in 0..20 {
                assert!(cache.look(&d.wb, 0, CellRef::new(row, col)).is_some());
            }
        }
        assert!(d.render_budget.remaining().unwrap() > 990_000);
        assert!(cache.frame_meter.spent().unwrap() < 10_000);
    }

    /// Rejecting an oversized kernel is local to its cell and precedes its bank reservation.
    #[test]
    fn local_native_refusal_preserves_frame_credit_for_other_cells() {
        let mut s = crate::Session::new();
        s.new_workbook();
        let values = (1..=2048).map(|n| json!([n])).collect::<Vec<_>>();
        s.execute("range.setValues", json!({"range":"A1:A2048","values":values})).unwrap();
        s.execute("home.conditionalFormat", json!({"range":"B1","rule":{"type":"expression","formula":"=IFERROR(INDEX(ACT.GPD.FIT.MLE($A$1:$A$2048),1,1),TRUE)","preset":"lightRedFill"}}))
            .unwrap();
        s.execute("home.conditionalFormat", json!({"range":"B2","rule":{"type":"expression","formula":"=TRUE","preset":"lightRedFill"}})).unwrap();
        let d = s.doc().unwrap();
        let mut cache = CfCache::with_budget(d.render_budget.clone());
        assert!(cache.look(&d.wb, 0, CellRef::parse("B1").unwrap()).is_none());
        assert!(d.render_budget.remaining().unwrap() > 990_000);
        assert!(cache.frame_meter.check().is_ok(), "a local refusal must not poison the frame meter");
        assert!(cache.look(&d.wb, 0, CellRef::parse("B2").unwrap()).is_some());
    }

    /// Hits, including cached fallback, do not reserve credit; misses retain terminal bank failure.
    #[test]
    fn cached_hits_and_fallback_do_not_reserve_frame_credit() {
        let mut s = crate::Session::new();
        s.new_workbook();
        s.execute("home.conditionalFormat", json!({"range":"A1:A3","rule":{"type":"expression","formula":"=TRUE","preset":"lightRedFill"}})).unwrap();
        let d = s.doc().unwrap();
        let budget = d.render_budget.clone();
        let results = d.render_cache.clone();
        let mut first = CfCache::with_cache_at(budget.clone(), results.clone(), &d.wb, d.revision, Some(1000.));
        assert!(first.look(&d.wb, 0, CellRef::default()).is_some());
        crate::workload::Meter::new(budget.clone()).charge(budget.remaining().unwrap()).unwrap();
        let mut next = CfCache::with_cache_at(budget.clone(), results.clone(), &d.wb, d.revision, Some(1000.));
        assert!(next.look(&d.wb, 0, CellRef::default()).is_some());
        assert_eq!(next.frame_meter.spent().unwrap(), 0);
        let uncached = CellRef::new(1, 0);
        assert!(next.look(&d.wb, 0, uncached).is_none());
        assert!(next.frame_meter.check().is_err());
        let spent = next.frame_meter.spent().unwrap();
        assert!(next.look(&d.wb, 0, uncached).is_none());
        assert_eq!(next.frame_meter.spent().unwrap(), spent);
        assert!(results.0.lock().unwrap().values.contains_key(&(0, uncached)));
        let parent = gridcraft_calc::WorkHook::new(|_| false);
        let _scope = parent.enter().unwrap();
        assert!(next.look(&d.wb, 0, CellRef::default()).is_none());
    }

    /// Unchanged frames reuse bounded results; identity/revision/TTL changes invalidate results only.
    #[test]
    fn repeated_paints_reuse_results_and_invalidate_without_refilling_work() {
        let mut s = crate::Session::new();
        s.new_workbook();
        s.execute("home.conditionalFormat", json!({"range":"A1:T50","rule":{"type":"expression","formula":"=1+1=2","preset":"lightRedFill"}}))
            .unwrap();
        let d = s.doc().unwrap();
        let budget = d.render_budget.clone();
        let results = d.render_cache.clone();
        for frame in 0..500 {
            let mut cache = CfCache::with_cache_at(budget.clone(), results.clone(), &d.wb, d.revision, Some(1000.));
            for row in 0..50 {
                for col in 0..20 {
                    assert!(cache.look(&d.wb, 0, CellRef::new(row, col)).is_some(), "frame {frame}");
                }
            }
        }
        let remaining = budget.remaining().unwrap();
        assert!(remaining > 990_000);
        assert_eq!(results.0.lock().unwrap().values.len(), 1000);
        let c = CellRef::default();
        let mut expired = CfCache::with_cache_at(budget.clone(), results.clone(), &d.wb, d.revision, Some(2000.));
        assert!(expired.look(&d.wb, 0, c).is_some());
        assert!(budget.remaining().unwrap() < remaining);
        let before_revision = budget.remaining().unwrap();
        let mut revised = CfCache::with_cache_at(budget.clone(), results.clone(), &d.wb, d.revision + 1, Some(2000.));
        assert!(revised.look(&d.wb, 0, c).is_some());
        assert!(budget.remaining().unwrap() < before_revision);
        let cloned = Arc::new((*d.wb).clone());
        let before_identity = budget.remaining().unwrap();
        let mut changed = CfCache::with_cache_at(budget.clone(), results.clone(), &cloned, d.revision + 1, Some(2000.));
        assert!(changed.look(&cloned, 0, c).is_some());
        assert!(budget.remaining().unwrap() < before_identity);
        // Unknown and backward clocks never serve cached volatile results.
        for now in [None, Some(1000.)] {
            let before_clock = budget.remaining().unwrap();
            let mut cache = CfCache::with_cache_at(budget.clone(), results.clone(), &cloned, d.revision + 1, now);
            assert!(cache.look(&cloned, 0, c).is_some());
            assert!(budget.remaining().unwrap() < before_clock);
        }
        assert_eq!(Arc::strong_count(&cloned), 1, "paint identity must remain weak");
    }

    /// Workbook paint expressions are bounded without a command and cannot style a refused value.
    #[test]
    fn rendering_gpd_refusal_has_safe_fallback_even_with_iferror() {
        let mut s = crate::Session::new();
        s.new_workbook();
        let values = (1..=2048).map(|n| json!([n])).collect::<Vec<_>>();
        s.execute("range.setValues", json!({"range":"A1:A2048","values":values})).unwrap();
        for rule in [
            json!({"type":"expression","formula":"=IFERROR(INDEX(ACT.GPD.FIT.MLE($A$1:$A$2048),1,1),TRUE)","preset":"lightRedFill"}),
            json!({"type":"cellIs","operator":"notEqual","value":"=ACT.GPD.FIT.MLE($A$1:$A$2048)","preset":"lightRedFill"}),
        ] {
            s.execute("home.conditionalFormat", json!({"range":"B1:B2","rule":rule})).unwrap();
            let d = s.doc().unwrap();
            let before = d.analysis_budget.remaining().unwrap();
            for _ in 0..3 {
                let mut paint = CfCache::with_budget(d.render_budget.clone());
                assert!(paint.look(&d.wb, 0, CellRef::parse("B1").unwrap()).is_none());
                assert!(paint.look(&d.wb, 0, CellRef::parse("B2").unwrap()).is_none());
            }
            assert_eq!(d.analysis_budget.remaining().unwrap(), before);
        }
    }

    /// New paint caches and formula IFERROR cannot reset the document's consumed window.
    #[test]
    fn repaint_caches_share_one_window_and_preserve_ambient_refusal() {
        let mut s = crate::Session::new();
        s.new_workbook();
        s.execute("home.conditionalFormat", json!({"range":"A1","rule":{"type":"expression","formula":"=TRUE","preset":"lightRedFill"}})).unwrap();
        let d = s.doc().unwrap();
        let budget = d.render_budget.clone();
        assert!(CfCache::with_budget(budget.clone()).look(&d.wb, 0, CellRef::default()).is_some());
        crate::workload::Meter::new(budget.clone()).charge(budget.remaining().unwrap()).unwrap();
        for _ in 0..3 {
            assert!(CfCache::with_budget(budget.clone()).look(&d.wb, 0, CellRef::default()).is_none());
        }
        let parent = gridcraft_calc::WorkHook::new(|_| false);
        let _scope = parent.enter().unwrap();
        assert!(CfCache::new().look(&d.wb, 0, CellRef::default()).is_none());
        assert!(!gridcraft_calc::WorkHook::allowed());
    }

    #[test]
    fn every_matching_rule_applies() {
        let mut s = crate::Session::new();
        s.new_workbook();
        s.execute("range.setValues", json!({"range": "A1", "values": [[200], [400], [510]]})).unwrap();
        let rule = |op: &str, value: &str, preset: &str| json!({"range": "A1:A3", "rule": {"type": "cellIs", "operator": op, "value": value, "preset": preset}});
        s.execute("home.conditionalFormat", rule("greaterThan", "300", "lightRedFill")).unwrap();
        s.execute("home.conditionalFormat", rule("greaterThan", "500", "redBorder")).unwrap();
        let d = s.doc().unwrap();
        let mut cache = CfCache::new();
        let mut look = |a: &str| cache.look(&d.wb, 0, CellRef::parse(a).unwrap()).and_then(|l| l.style);
        assert!(look("A1").is_none());
        let only_fill = look("A2").unwrap();
        assert_eq!(only_fill.fill.pattern, PatternType::Solid);
        assert!(only_fill.borders.top.is_none());
        let both = look("A3").unwrap();
        assert_eq!(both.fill, only_fill.fill);
        assert!(!both.borders.top.is_none());
    }

    #[test]
    fn cell_value_rules_on_blanks_and_errors() {
        let mut s = crate::Session::new();
        s.new_workbook();
        s.execute("range.setValues", json!({"range": "A1", "values": [[3, "=NA()"], [null, 7], ["=NA()", null], [" ", null], [7, null]]})).unwrap();
        let rule = |range: &str, r: serde_json::Value| json!({"range": range, "rule": r});
        s.execute("home.conditionalFormat", rule("A1:A5", json!({"type": "cellIs", "operator": "less", "value": "5", "preset": "lightRedFill"})))
            .unwrap();
        s.execute("home.conditionalFormat", rule("A1:A5", json!({"type": "noBlanks", "preset": "redBorder"}))).unwrap();
        s.execute("home.conditionalFormat", rule("B1:B2", json!({"type": "cellIs", "operator": "greater", "value": "5", "preset": "lightRedFill"})))
            .unwrap();
        let d = s.doc().unwrap();
        let mut cache = CfCache::new();
        let mut look = |a: &str| cache.look(&d.wb, 0, CellRef::parse(a).unwrap()).and_then(|l| l.style);
        let fill = |st: Option<Style>| st.is_some_and(|st| st.fill.pattern == PatternType::Solid);
        let border = |st: Option<Style>| st.is_some_and(|st| !st.borders.top.is_none());
        // Less than 5: 3 and the blank cell (as 0), not #N/A.
        assert_eq!([look("A1"), look("A2"), look("A3"), look("A5")].map(fill), [true, true, false, false]);
        // Greater than 5: 7, not #N/A.
        assert_eq!([look("B1"), look("B2")].map(fill), [false, true]);
        // No blanks: neither the empty cell nor the one holding a space.
        assert_eq!([look("A1"), look("A2"), look("A4")].map(border), [true, false, false]);
    }

    /// The statistics as every rule used to get them: one pass collecting both parts.
    fn all_stats_before(sh: &Sheet, ranges: &[RangeRef]) -> RuleStats {
        let mut nums = Vec::new();
        let mut counts = HashMap::new();
        for r in ranges {
            let Some(r) = sh.used_range().and_then(|u| r.intersection(&u)) else { continue };
            for c in r.iter() {
                let v = sh.value(c);
                if let Value::Number(n) = v {
                    nums.push(n);
                }
                if !v.is_empty() {
                    *counts.entry(v.display().to_lowercase()).or_insert(0) += 1;
                }
            }
        }
        nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = nums.len().max(1) as f64;
        let avg = nums.iter().sum::<f64>() / n;
        let std = (nums.iter().map(|x| (x - avg).powi(2)).sum::<f64>() / n).sqrt();
        RuleStats { sorted: nums, avg, std, counts }
    }

    #[test]
    fn needed_statistics_equal_the_full_ones() {
        let mut s = crate::Session::new();
        s.new_workbook();
        s.execute("range.setValues", json!({"range": "A1", "values": [[3, "x"], [7, "X"], [3, null], ["=NA()", 2.5], [" ", 9], [-1, "y"]]})).unwrap();
        let d = s.doc().unwrap();
        let sh = d.wb.sheet(0).unwrap();
        for ranges in ["A1:B6", "A1:A3", "B2:B9", "D1:E4", "A1:A2"].map(|r| vec![RangeRef::parse(r).unwrap()]) {
            let full = all_stats_before(sh, &ranges);
            let numbers = RuleStats::build(sh, &ranges, StatsNeed::Numbers);
            assert_eq!((numbers.sorted, numbers.avg.to_bits(), numbers.std.to_bits()), (full.sorted, full.avg.to_bits(), full.std.to_bits()));
            assert_eq!(RuleStats::build(sh, &ranges, StatsNeed::Counts).counts, full.counts);
        }
    }

    #[test]
    fn rules_that_read_statistics_still_match() {
        let mut s = crate::Session::new();
        s.new_workbook();
        s.execute("range.setValues", json!({"range": "A1", "values": [[1, "a"], [2, "A"], [3, "b"], [10, "c"], [2, null]]})).unwrap();
        let rule = |range: &str, r: serde_json::Value| json!({"range": range, "rule": r});
        s.execute("home.conditionalFormat", rule("B1:B5", json!({"type": "duplicate", "preset": "lightRedFill"}))).unwrap();
        s.execute("home.conditionalFormat", rule("A1:A5", json!({"type": "top10", "rank": 1, "preset": "redBorder"}))).unwrap();
        s.execute("home.conditionalFormat", rule("A1:A5", json!({"type": "colorScale"}))).unwrap();
        let d = s.doc().unwrap();
        let mut cache = CfCache::new();
        let mut look = |a: &str| cache.look(&d.wb, 0, CellRef::parse(a).unwrap());
        let fill = |l: Option<CfLook>| l.and_then(|l| l.style).is_some_and(|st| st.fill.pattern == PatternType::Solid);
        // Duplicates are case-insensitive texts: "a" and "A".
        assert_eq!(["B1", "B2", "B3", "B4", "B5"].map(|a| fill(look(a))), [true, true, false, false, false]);
        // The top value gets the border, the scale runs from the minimum to the maximum.
        assert!(look("A4").and_then(|l| l.style).is_some_and(|st| !st.borders.top.is_none()));
        assert!(look("A1").and_then(|l| l.style).is_none_or(|st| st.borders.top.is_none()));
        assert_eq!(look("A1").and_then(|l| l.scale_fill), Some([0xF8, 0x69, 0x6B]));
        assert_eq!(look("A4").and_then(|l| l.scale_fill), Some([0x63, 0xBE, 0x7B]));
    }

    /// Looking cells up does work in proportion to the cells asked for, not the rule's range: the
    /// statistics are only gathered for rules that read them, and once per rule.
    #[test]
    fn lookups_do_not_scan_the_range_for_rules_without_statistics() {
        let mut s = crate::Session::new();
        s.new_workbook();
        let rows: Vec<serde_json::Value> = (1..=10_000).map(|i| json!([i, i % 7])).collect();
        s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
        s.execute(
            "home.conditionalFormat",
            json!({"range": "A1:B10000", "rule": {"type": "expression", "formula": "=MOD(ROW(),2)=0", "preset": "lightRedFill"}}),
        )
        .unwrap();
        s.execute(
            "home.conditionalFormat",
            json!({"range": "A1:B10000", "rule": {"type": "cellIs", "operator": "greater", "value": "5", "preset": "redBorder"}}),
        )
        .unwrap();
        s.execute("home.conditionalFormat", json!({"range": "A1:B10000", "rule": {"type": "noBlanks", "preset": "redBorder"}})).unwrap();
        let d = s.doc().unwrap();
        let mut cache = CfCache::new();
        SCANNED.with(|n| n.set(0));
        for r in 1..=50 {
            for col in ["A", "B"] {
                cache.look(&d.wb, 0, CellRef::parse(&format!("{col}{r}")).unwrap());
            }
        }
        assert_eq!(SCANNED.with(|n| n.get()), 0);
        // A rule that does read them scans its range once, however many cells are looked up.
        s.execute("home.conditionalFormat", json!({"range": "A1:A10000", "rule": {"type": "top10", "preset": "redBorder"}})).unwrap();
        let d = s.doc().unwrap();
        let mut cache = CfCache::new();
        SCANNED.with(|n| n.set(0));
        for r in 1..=50 {
            cache.look(&d.wb, 0, CellRef::parse(&format!("A{r}")).unwrap());
        }
        assert_eq!(SCANNED.with(|n| n.get()), 10_000);
    }

    #[test]
    fn cell_value_operator_names() {
        let mut s = crate::Session::new();
        s.new_workbook();
        s.execute("range.setValues", json!({"range": "A1", "values": [[3], [7]]})).unwrap();
        let rule = |op: &str| json!({"range": "A1:A2", "rule": {"type": "cellIs", "operator": op, "value": "5", "preset": "lightRedFill"}});
        s.execute("home.conditionalFormat", rule("lessThan")).unwrap();
        assert!(s.execute("home.conditionalFormat", rule("smaller")).is_err());
        let d = s.doc().unwrap();
        let mut cache = CfCache::new();
        let mut look = |a: &str| cache.look(&d.wb, 0, CellRef::parse(a).unwrap()).and_then(|l| l.style).is_some();
        assert_eq!([look("A1"), look("A2")], [true, false]);
    }

    #[test]
    fn text_rules_skip_error_values() {
        let mut s = crate::Session::new();
        s.new_workbook();
        let values = json!([["=NA()", "=NA()", "=1/0", "=NA()"], ["abc", "#x", "x!", "abc"], ["xyz", "y", "y", "xyz"]]);
        s.execute("range.setValues", json!({"range": "A1", "values": values})).unwrap();
        let rule = |range: &str, kind: &str, text: &str| json!({"range": range, "rule": {"type": kind, "text": text, "preset": "lightRedFill"}});
        s.execute("home.conditionalFormat", rule("A1:A3", "containsText", "a")).unwrap();
        s.execute("home.conditionalFormat", rule("B1:B3", "beginsWith", "#")).unwrap();
        s.execute("home.conditionalFormat", rule("C1:C3", "endsWith", "!")).unwrap();
        s.execute("home.conditionalFormat", rule("D1:D3", "notContainsText", "a")).unwrap();
        let d = s.doc().unwrap();
        let mut cache = CfCache::new();
        let mut look = |a: &str| cache.look(&d.wb, 0, CellRef::parse(a).unwrap()).and_then(|l| l.style).is_some();
        // Contains "a": abc, never #N/A although its text holds an "a".
        assert_eq!([look("A1"), look("A2"), look("A3")], [false, true, false]);
        // Begins with "#": #x, never #N/A.
        assert_eq!([look("B1"), look("B2"), look("B3")], [false, true, false]);
        // Ends with "!": x!, never #DIV/0!.
        assert_eq!([look("C1"), look("C2"), look("C3")], [false, true, false]);
        // Does not contain "a": #N/A and xyz, not abc.
        assert_eq!([look("D1"), look("D2"), look("D3")], [true, false, true]);
    }
}
