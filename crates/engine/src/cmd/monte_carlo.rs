//! Seeded Monte Carlo over arbitrary workbook formulas. Trials never modify source cells.
//! Tornado sensitivity is computed from the same samples (conditional decile medians), not
//! from one-at-a-time substitutions. Sampling supports replicated LHS and Gaussian dependence.

use std::collections::HashSet;
use std::sync::Arc;

use gridcraft_core::{CellRef, Value};
use gridcraft_model::{Anchor, Cell, Chart, ChartKind, LegendPos, Series, Sheet, StyleId};
use serde_json::{Value as Json, json};

use super::whatif::Trial;
use super::*;

mod sampling;
mod statistics;
use sampling::Plan;
use statistics::Diagnostics;

const CMD: &str = "data.monteCarlo";
const MAX_INPUTS: usize = 32;
const MAX_OUTPUTS: usize = 64;
const MAX_ITERATIONS: u64 = 100_000;
const MAX_SAMPLE_CELLS: usize = 1_000_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "data.monteCarlo",
        "Monte Carlo…",
        ["Data", "Analysis"],
        None,
        "{inputs: [{cell: \"Inputs!B2\", distribution: uniform|triangular|normal, min?, mode?, max?, mean?, stdDev?}], outputs: [\"Model!F30\", ..], iterations?: 1000 (2–100000, ≤1000000 sampled cells), seed?: 0, samples?: true, newSheet?: name, method?: random|latinHypercube, replicates?: 1 (LHS default 8, at least 2), correlation?: [[1,0.5],[0.5,1]] (Gaussian latent correlation, input order)}. Includes Monte Carlo error estimates and convergence; formula outputs are recalculated on isolated workbook copies. Creates a static report with P10/P50/P90, probability > 0, samples and a conditional Monte Carlo tornado for the first output. Source inputs remain unchanged. Seed controls input sampling, not volatile formulas.",
        has_doc,
        run
    )]
}

#[derive(Clone, Debug)]
enum Distribution {
    Uniform(f64, f64),
    Triangular(f64, f64, f64),
    Normal(f64, f64),
}

/// SplitMix64, as used by the evolutionary solver; no platform RNG or new dependency.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn uniform(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn below(&mut self, n: usize) -> Result<usize> {
        let bound = n as u64;
        if bound == 0 {
            return Err(bad(CMD, "empty shuffle"));
        }
        let threshold = bound.wrapping_neg() % bound;
        for _ in 0..16 {
            let x = self.next();
            if x >= threshold {
                return Ok((x % bound) as usize);
            }
        }
        Err(bad(CMD, "random shuffle failed"))
    }
}
impl Distribution {
    fn parse(p: &Json) -> Result<Self> {
        let num = |k: &str| p.get(k).and_then(Json::as_f64).filter(|n| n.is_finite()).ok_or_else(|| bad(CMD, format!("missing or invalid `{k}`")));
        match str_param(p, "distribution").unwrap_or("uniform") {
            "uniform" | "triangular" => {
                let (a, b) = (num("min")?, num("max")?);
                if a > b || !(b - a).is_finite() {
                    return Err(bad(CMD, "min must not exceed max, with a finite width"));
                }
                if str_param(p, "distribution") == Some("triangular") {
                    let m = num("mode")?;
                    if m < a || m > b {
                        return Err(bad(CMD, "mode must be between min and max"));
                    }
                    Ok(Self::Triangular(a, m, b))
                } else {
                    Ok(Self::Uniform(a, b))
                }
            }
            "normal" => {
                let (m, sd) = (num("mean")?, num("stdDev")?);
                if sd < 0.0 {
                    return Err(bad(CMD, "stdDev must not be negative"));
                }
                Ok(Self::Normal(m, sd))
            }
            _ => Err(bad(CMD, "distribution must be uniform, triangular or normal")),
        }
    }
    fn inverse_cdf(&self, u: f64) -> Result<f64> {
        let x = match *self {
            Self::Uniform(a, b) => a + (b - a) * u,
            Self::Triangular(a, _, b) if a == b => a,
            Self::Triangular(a, m, b) => {
                let w = b - a;
                let cut = (m - a) / w;
                if u < cut { a + w * (u * cut).sqrt() } else { b - w * ((1.0 - u) * (1.0 - cut)).sqrt() }
            }
            Self::Normal(m, sd) => m + sd * sampling::normal_inverse(u)?,
        };
        if x.is_finite() { Ok(x) } else { Err(bad(CMD, "distribution produced a non-finite value")) }
    }
    fn sample(&self, rng: &mut Rng) -> f64 {
        let u = rng.uniform();
        match *self {
            Self::Uniform(a, b) => a + (b - a) * u,
            Self::Triangular(a, _, b) if a == b => a,
            Self::Triangular(a, m, b) => {
                let width = b - a;
                let mode = (m - a) / width;
                // Normalize before the square root to avoid overflowing width².
                if u < mode { a + width * (u * mode).sqrt() } else { b - width * ((1.0 - u) * (1.0 - mode)).sqrt() }
            }
            Self::Normal(m, sd) => m + sd * (-2.0 * (1.0 - u).ln()).sqrt() * (std::f64::consts::TAU * rng.uniform()).cos(),
        }
    }
}

struct Input {
    sheet: usize,
    cell: CellRef,
    label: String,
    distribution: Distribution,
}
struct Output {
    sheet: usize,
    cell: CellRef,
    label: String,
}

fn address(wb: &Workbook, text: &str) -> Result<(usize, CellRef, String)> {
    let (name, body) = split_sheet(text);
    let si = match name {
        Some(name) => wb.sheet_index(&name).ok_or_else(|| bad(CMD, format!("no sheet named `{name}`")))?,
        None => wb.active_sheet(),
    };
    let cell = CellRef::parse(body.trim()).ok_or_else(|| bad(CMD, format!("not a cell: `{text}`")))?;
    let sh = wb.sheet(si).ok_or(EngineError::NoDocument)?;
    Ok((si, cell, format!("{}!{}", gridcraft_formula::quote_sheet(&sh.name), cell.a1())))
}

fn integer(p: &Json, key: &str, default: u64, max: u64) -> Result<u64> {
    let n = match p.get(key) {
        None => default,
        Some(v) => v.as_u64().ok_or_else(|| bad(CMD, format!("`{key}` must be a non-negative integer")))?,
    };
    if n > max {
        return Err(bad(CMD, format!("`{key}` must not exceed {max}")));
    }
    Ok(n)
}

/// Inclusive linear interpolation, matching PERCENTILE.INC.
fn quantile(sorted: &[f64], p: f64) -> f64 {
    let k = p * sorted.len().saturating_sub(1) as f64;
    let lo = k.floor() as usize;
    let a = sorted.get(lo).copied().unwrap_or(0.0);
    let b = sorted.get(lo.saturating_add(1)).copied().unwrap_or(a);
    // Weighted average avoids overflowing b-a for opposite extreme finite values.
    a * (1.0 - k.fract()) + b * k.fract()
}
fn sorted(values: &[f64]) -> Vec<f64> {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    v
}
fn ranks(values: &[f64]) -> Vec<f64> {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|a, b| values.get(*a).unwrap_or(&0.0).total_cmp(values.get(*b).unwrap_or(&0.0)));
    let mut out = vec![0.0; values.len()];
    let mut i = 0;
    while i < order.len() {
        let mut j = i + 1;
        while j < order.len() && order.get(i).and_then(|k| values.get(*k)) == order.get(j).and_then(|k| values.get(*k)) {
            j += 1;
        }
        let rank = (i + j - 1) as f64 / 2.0;
        for k in order.get(i..j).unwrap_or_default() {
            if let Some(v) = out.get_mut(*k) {
                *v = rank;
            }
        }
        i = j;
    }
    out
}
fn correlation(a: &[f64], b: &[f64]) -> Option<f64> {
    statistics::correl(a, b)
}

#[derive(Clone)]
struct Summary {
    label: String,
    mean: f64,
    sd: f64,
    min: f64,
    p10: f64,
    median: f64,
    p90: f64,
    max: f64,
    above: f64,
}
impl Summary {
    fn json(&self) -> Json {
        json!({"cell":self.label,"mean":self.mean,"stdDev":self.sd,"min":self.min,"p10":self.p10,"p50":self.median,"p90":self.p90,"max":self.max,"probabilityAboveZero":self.above})
    }
}
#[derive(Clone)]
struct Sensitivity {
    input: String,
    low: f64,
    high: f64,
    low_n: usize,
    high_n: usize,
    spearman: Option<f64>,
    adverse: f64,
    favorable: f64,
    span: f64,
}
impl Sensitivity {
    fn json(&self) -> Json {
        json!({"input":self.input,"lowMedian":self.low,"highMedian":self.high,"lowCount":self.low_n,"highCount":self.high_n,"spearman":self.spearman,"adverse":self.adverse,"favorable":self.favorable,"span":self.span})
    }
}

fn run(s: &mut Session, p: &Json) -> Result<Json> {
    let d = s.doc()?;
    if d.wb.protected_structure {
        return Err(bad(CMD, "the workbook structure is protected"));
    }
    let raw_inputs =
        p.get("inputs").and_then(Json::as_array).filter(|a| !a.is_empty() && a.len() <= MAX_INPUTS).ok_or_else(|| bad(CMD, "give 1–32 inputs"))?;
    let raw_outputs = p
        .get("outputs")
        .and_then(Json::as_array)
        .filter(|a| !a.is_empty() && a.len() <= MAX_OUTPUTS)
        .ok_or_else(|| bad(CMD, "give 1–64 output cells"))?;
    let iterations = integer(p, "iterations", 1000, MAX_ITERATIONS)? as usize;
    if iterations < 2 {
        return Err(bad(CMD, "iterations must be at least 2"));
    }
    if iterations.saturating_mul(raw_inputs.len() + raw_outputs.len()) > MAX_SAMPLE_CELLS {
        return Err(bad(CMD, "too many sampled cells (at most 1000000)"));
    }
    let seed = integer(p, "seed", 0, u64::MAX)?;
    let keep_samples = match p.get("samples") {
        None => true,
        Some(v) => v.as_bool().ok_or_else(|| bad(CMD, "samples must be a boolean"))?,
    };
    let name = match p.get("newSheet") {
        Some(Json::String(n)) if !n.trim().is_empty() => n.trim().to_string(),
        None | Some(Json::String(_)) => (1..=100_000)
            .map(|i| if i == 1 { "Monte Carlo".to_string() } else { format!("Monte Carlo {i}") })
            .find(|n| d.wb.sheet_index(n).is_none())
            .ok_or_else(|| bad(CMD, "no report sheet name available"))?,
        _ => return Err(bad(CMD, "newSheet must be a string")),
    };
    d.wb.check_sheet_name(&name, None).map_err(EngineError::Other)?;
    let mut used = HashSet::new();
    let mut inputs = Vec::with_capacity(raw_inputs.len());
    for item in raw_inputs {
        let text = str_param(item, "cell").ok_or_else(|| bad(CMD, "each input needs a cell"))?;
        let (sheet, cell, label) = address(&d.wb, text)?;
        if !used.insert((sheet, cell)) {
            return Err(bad(CMD, "input cells must be unique"));
        }
        let sh = d.wb.sheet(sheet).ok_or(EngineError::NoDocument)?;
        if sh.cell(cell).is_some_and(|c| c.formula.is_some()) || !matches!(sh.value(cell),Value::Number(n) if n.is_finite()) {
            return Err(bad(CMD, format!("input `{label}` must be a numeric constant")));
        }
        inputs.push(Input { sheet, cell, label, distribution: Distribution::parse(item)? });
    }
    let mut outputs = Vec::with_capacity(raw_outputs.len());
    let mut seen_outputs = HashSet::new();
    for item in raw_outputs {
        let text = item.as_str().ok_or_else(|| bad(CMD, "outputs must be cell references"))?;
        let (sheet, cell, label) = address(&d.wb, text)?;
        if !seen_outputs.insert((sheet, cell)) {
            return Err(bad(CMD, "output cells must be unique"));
        }
        outputs.push(Output { sheet, cell, label });
    }
    // Canonical cross-sheet refs let the report retain a reusable model even when active sheet changes.
    let mut config = p.clone();
    if let Some(c) = config.as_object_mut() {
        c.remove("newSheet");
        c.insert("seed".into(), json!(seed));
        c.insert("iterations".into(), json!(iterations));
        let canonical = raw_inputs
            .iter()
            .zip(&inputs)
            .map(|(p, i)| {
                let mut p = p.clone();
                if let Some(o) = p.as_object_mut() {
                    o.insert("cell".into(), json!(i.label));
                }
                p
            })
            .collect::<Vec<_>>();
        c.insert("inputs".into(), json!(canonical));
        c.insert("outputs".into(), json!(outputs.iter().map(|o| &o.label).collect::<Vec<_>>()));
    }
    let plan = Plan::parse(p, inputs.len(), iterations)?;
    if let Some(c) = config.as_object_mut() {
        c.insert("method".into(), json!(plan.method_name()));
        c.insert("replicates".into(), json!(plan.replicates));
    }
    let xs = plan.sample(&inputs, seed)?;
    let reads = outputs.iter().map(|o| (o.sheet, o.cell)).collect::<Vec<_>>();
    let mut trial = Trial::new(d);
    let mut ys = (0..outputs.len()).map(|_| Vec::with_capacity(iterations)).collect::<Vec<_>>();
    for k in 0..iterations {
        let sets =
            inputs.iter().zip(&xs).map(|(i, values)| (i.sheet, i.cell, Value::Number(values.get(k).copied().unwrap_or(0.0)))).collect::<Vec<_>>();
        let values = trial.run(&sets, &reads);
        for ((output, column), value) in outputs.iter().zip(&mut ys).zip(values) {
            match value {
                Value::Number(n) if n.is_finite() => column.push(n),
                _ => return Err(bad(CMD, format!("output `{}` is not a finite number at iteration {}", output.label, k + 1))),
            }
        }
    }
    let mut summaries = Vec::with_capacity(outputs.len());
    for (output, values) in outputs.iter().zip(&ys) {
        let ordered = sorted(values);
        let n = iterations as f64;
        let (mean, sd) = statistics::moments(values)?;
        let summary = Summary {
            label: output.label.clone(),
            mean,
            sd,
            min: ordered.first().copied().unwrap_or(0.0),
            p10: quantile(&ordered, 0.1),
            median: quantile(&ordered, 0.5),
            p90: quantile(&ordered, 0.9),
            max: ordered.last().copied().unwrap_or(0.0),
            above: values.iter().filter(|x| **x > 0.0).count() as f64 / n,
        };
        if !mean.is_finite() || !sd.is_finite() {
            return Err(bad(CMD, "summary statistics overflowed; rescale the model"));
        }
        summaries.push(summary);
    }
    let first = ys.first().ok_or_else(|| bad(CMD, "missing output samples"))?;
    let center = quantile(&sorted(first), 0.5);
    let output_ranks = ranks(first);
    let mut sensitivity = Vec::with_capacity(inputs.len());
    for (input, values) in inputs.iter().zip(&xs) {
        let ordered = sorted(values);
        let (p10, p90) = (quantile(&ordered, 0.1), quantile(&ordered, 0.9));
        let low = values.iter().zip(first).filter_map(|(x, y)| (*x <= p10).then_some(*y)).collect::<Vec<_>>();
        let high = values.iter().zip(first).filter_map(|(x, y)| (*x >= p90).then_some(*y)).collect::<Vec<_>>();
        let (lm, hm) = (quantile(&sorted(&low), 0.5), quantile(&sorted(&high), 0.5));
        let (a, b) = (lm - center, hm - center);
        if !a.is_finite() || !b.is_finite() || !(hm - lm).is_finite() {
            return Err(bad(CMD, "tornado statistics overflowed; rescale the model"));
        }
        sensitivity.push(Sensitivity {
            input: input.label.clone(),
            low: lm,
            high: hm,
            low_n: low.len(),
            high_n: high.len(),
            spearman: correlation(&ranks(values), &output_ranks),
            adverse: a.min(b).min(0.0),
            favorable: a.max(b).max(0.0),
            span: (hm - lm).abs(),
        });
    }
    sensitivity.sort_by(|a, b| b.span.total_cmp(&a.span));
    let diagnostics = Diagnostics::new(&plan, &inputs, &outputs, &xs, &ys)?;
    write_report(s, &name, &config, seed, iterations, keep_samples, &inputs, &outputs, &xs, &ys, &summaries, &sensitivity, &diagnostics)?;
    Ok(
        json!({"sheet":name,"iterations":iterations,"seed":seed,"static":true,"independentInputs":plan.independent(),"sampling":plan.method_name(),"replicates":plan.replicates,"accuracy":diagnostics.accuracy_json(),"convergence":diagnostics.first_trace_json(),"config":config,"outputs":summaries.iter().map(Summary::json).collect::<Vec<_>>(),"tornadoOutput":outputs.first().map(|o|&o.label),"tornado":sensitivity.iter().map(Sensitivity::json).collect::<Vec<_>>()}),
    )
}

fn write_report(
    s: &mut Session,
    name: &str,
    config: &Json,
    seed: u64,
    iterations: usize,
    keep_samples: bool,
    inputs: &[Input],
    outputs: &[Output],
    xs: &[Vec<f64>],
    ys: &[Vec<f64>],
    summaries: &[Summary],
    sensitivity: &[Sensitivity],
    diagnostics: &Diagnostics,
) -> Result<()> {
    let chart_id = s.doc()?.wb.next_object_id();
    let si = edit(s, |cx| {
        let mut sheet = Sheet::new(name.to_string());
        let bold = cx.wb.styles.derive(StyleId::DEFAULT, |st| st.font.bold = true);
        let mut put = |r: u32, c: u32, v: Value, style: StyleId| {
            sheet.set_cell(CellRef::new(r, c), Cell { value: v, formula: None, style });
        };
        put(0, 0, Value::text("Monte Carlo simulation"), bold);
        put(
            1,
            0,
            Value::text(
                "Static snapshot. Percentiles describe outcomes; confidence intervals describe sampling error. Conditional tornado is not a causal effect.",
            ),
            StyleId::DEFAULT,
        );
        put(2, 0, Value::text(format!("Iterations: {iterations}; seed: {seed}. Seed controls inputs, not volatile formulas.")), StyleId::DEFAULT);
        put(3, 0, Value::text("Reusable configuration (JSON)"), bold);
        put(3, 1, Value::text(config.to_string()), StyleId::DEFAULT);
        for (c, h) in ["Output", "Mean", "Sample standard deviation", "Minimum", "P10", "P50", "P90", "Maximum", "Probability > 0"].iter().enumerate()
        {
            put(5, c as u32, Value::text(*h), bold);
        }
        for (r, a) in summaries.iter().enumerate() {
            let r = r as u32 + 6;
            put(r, 0, Value::text(a.label.as_str()), StyleId::DEFAULT);
            for (c, n) in [a.mean, a.sd, a.min, a.p10, a.median, a.p90, a.max, a.above].into_iter().enumerate() {
                put(r, c as u32 + 1, Value::Number(n), StyleId::DEFAULT);
            }
        }
        let t = outputs.len() as u32 + 9;
        put(t - 1, 0, Value::text("Conditional tornado: first output, same Monte Carlo samples; tied deciles may contain more than 10%."), bold);
        for (c, h) in [
            "Input",
            "Spearman",
            "Low-group median",
            "High-group median",
            "Adverse vs median",
            "Favorable vs median",
            "Span",
            "Low group size",
            "High group size",
        ]
        .iter()
        .enumerate()
        {
            put(t, c as u32, Value::text(*h), bold);
        }
        for (r, a) in sensitivity.iter().enumerate() {
            let r = t + 1 + r as u32;
            put(r, 0, Value::text(a.input.as_str()), StyleId::DEFAULT);
            put(r, 1, a.spearman.map(Value::Number).unwrap_or_default(), StyleId::DEFAULT);
            for (c, n) in [a.low, a.high, a.adverse, a.favorable, a.span, a.low_n as f64, a.high_n as f64].into_iter().enumerate() {
                put(r, c as u32 + 2, Value::Number(n), StyleId::DEFAULT);
            }
        }
        let (diagnostics_end, trace_start, trace_end) = diagnostics.write(&mut put, t + sensitivity.len() as u32 + 3, bold);
        // Ascending helper rows put the most influential input at the top of a native bar chart.
        let helper = 28;
        for (c, h) in ["Input", "Adverse", "Favorable"].iter().enumerate() {
            put(helper, 12 + c as u32, Value::text(*h), bold);
        }
        for (r, a) in sensitivity.iter().rev().enumerate() {
            put(helper + 1 + r as u32, 12, Value::text(a.input.as_str()), StyleId::DEFAULT);
            put(helper + 1 + r as u32, 13, Value::Number(a.adverse), StyleId::DEFAULT);
            put(helper + 1 + r as u32, 14, Value::Number(a.favorable), StyleId::DEFAULT);
        }
        if keep_samples {
            let start = diagnostics_end + 2;
            put(start, 0, Value::text("Iteration"), bold);
            for (c, a) in inputs.iter().map(|i| &i.label).chain(outputs.iter().map(|o| &o.label)).enumerate() {
                put(start, c as u32 + 1, Value::text(a.as_str()), bold);
            }
            for r in 0..iterations {
                put(start + 1 + r as u32, 0, Value::Number((r + 1) as f64), StyleId::DEFAULT);
                for (c, values) in xs.iter().chain(ys).enumerate() {
                    if let Some(v) = values.get(r) {
                        put(start + 1 + r as u32, c as u32 + 1, Value::Number(*v), StyleId::DEFAULT);
                    }
                }
            }
        }
        let q = gridcraft_formula::quote_sheet(name);
        let end = helper + sensitivity.len() as u32 + 1;
        sheet.charts.push(Chart {
            id: chart_id,
            kind: ChartKind::BarStacked,
            anchor: Anchor { cell: CellRef::new(0, 12), width: 640.0, height: 420.0, ..Default::default() },
            title: Some("Monte Carlo conditional tornado".into()),
            series: (13..=14)
                .map(|col| Series {
                    name: Some(format!("{q}!${}${}", if col == 13 { "N" } else { "O" }, helper + 1)),
                    categories: Some(format!("{q}!$M${}:$M${end}", helper + 2)),
                    values: format!("{q}!${0}${1}:${0}${end}", if col == 13 { "N" } else { "O" }, helper + 2),
                    ..Default::default()
                })
                .collect(),
            legend: LegendPos::Bottom,
            data_labels: false,
            gridlines: true,
            style: 1,
            x_title: Some("Change from overall median".into()),
            y_title: None,
            source: None,
            by_rows: false,
            pivot: None,
        });
        let trace_series = [(2, "Mean"), (4, "Mean CI95 lower"), (5, "Mean CI95 upper")]
            .into_iter()
            .map(|(col, label)| {
                let letter = gridcraft_core::col_to_letters(col);
                Series {
                    name: Some(label.into()),
                    categories: Some(format!("{q}!$B${}:$B${}", trace_start + 1, trace_end + 1)),
                    values: format!("{q}!${letter}${}:${letter}${}", trace_start + 1, trace_end + 1),
                    ..Default::default()
                }
            })
            .collect();
        sheet.charts.push(Chart {
            id: chart_id.saturating_add(1),
            kind: ChartKind::LineMarkers,
            anchor: Anchor { cell: CellRef::new(helper + sensitivity.len() as u32 + 5, 12), width: 640.0, height: 360.0, ..Default::default() },
            title: Some("Mean convergence and 95% sampling-error interval".into()),
            series: trace_series,
            legend: LegendPos::Bottom,
            data_labels: false,
            gridlines: true,
            style: 1,
            x_title: Some("Completed samples".into()),
            y_title: None,
            source: None,
            by_rows: false,
            pivot: None,
        });
        for col in 0..15 {
            sheet.cols.insert(col, gridcraft_model::LineInfo { size: Some(if col == 0 || col == 12 { 180.0 } else { 140.0 }), ..Default::default() });
        }
        let si = cx.wb.sheets.len();
        cx.wb.sheets.push(Arc::new(sheet));
        cx.wb.set_active_sheet(si);
        *cx.sel = crate::Selection::default();
        cx.structural = true;
        Ok(si)
    })?;
    if let Ok(d) = s.doc_mut() {
        d.sheet_selections.insert(si.min(d.sheet_selections.len()), crate::Selection::default());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
