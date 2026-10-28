# Monte Carlo simulation

> **Last reviewed:** 2026-10-28 · **Last updated:** 2026-10-28 · **Change:** document numerical contracts and LHS-only replica controls

**Data → Monte Carlo** samples numeric input cells, recalculates the workbook's own formulas,
and creates a static report. The same command is available through CLI, the control channel,
and MCP's `execute_command`. It is built into the Rust engine; no Python, proprietary add-in,
or financial-model template is required.

## Configure a simulation

1. Add each uncertain input cell. Use a sheet-qualified reference for another sheet, such as
   `'Sales assumptions'!B2`. The input must be a numeric constant, not a formula.
2. Choose a distribution: **uniform** (minimum/maximum), **triangular**
   (minimum/mode/maximum), or **normal** (mean/standard deviation).
3. Enter output cells, one per line. They may be on different sheets. Outputs must evaluate to
   finite numbers in every trial.
4. Choose random Monte Carlo or Latin Hypercube sampling (LHS), total iterations, and a
   non-negative seed. The dialog additionally shows independent replicas for LHS: the default
   is eight and the minimum is two. LHS iterations must be divisible by replica count, with at
   least two samples per replica. The random dialog uses IID trials without a replica-count
   restriction. Optionally omit the raw sample table.
5. Optionally enter a correlation matrix: rows on separate lines, numbers separated by spaces.
   The matrix follows input order and specifies **latent Gaussian correlation**, not the
   Pearson correlation of non-normal sampled variables.
6. Run. The source inputs remain unchanged; adding the report is one undoable action.

The report contains mean, sample standard deviation, minimum, P10/P50/P90, maximum, probability
above zero, raw samples when requested, and a conditional tornado for the **first output**.
It also reports Monte Carlo standard errors, approximate 95% intervals, convergence by completed
blocks/replicas, and requested versus observed input dependence. A second native chart shows
convergence of the first output mean and its sampling-error interval.

It also stores the canonical JSON configuration, with sheet-qualified references, so the model
can be run again after reopening the workbook. This is not a live recalculating report: rerun
after changing the model or the distributions.

## Programmatic example

Create a synthetic model, then run the command:

```sh
gridcraft-cli run \
  --cmd 'range.setValues={"range":"A1","values":[[100,20]]}' \
  --cmd 'cell.set={"cell":"C1","input":"=A1*B1-1500"}' \
  --cmd 'data.monteCarlo={"inputs":[{"cell":"Sheet1!A1","distribution":"triangular","min":50,"mode":100,"max":150},{"cell":"Sheet1!B1","distribution":"normal","mean":20,"stdDev":2}],"outputs":["Sheet1!C1"],"iterations":10000,"seed":42}' \
  --out risk.xlsx
```

In MCP, call `execute_command` with `command: "data.monteCarlo"` and the same JSON as `params`.
Omitting `newSheet` creates a unique `Monte Carlo` report name. An explicit existing or invalid
sheet name is rejected. Reuse the report's configuration, or record the action as an automation.

## What the tornado means

For each input, the command finds its sampled P10 and P90, then takes the median of the first
output within the low-input group (≤P10) and high-input group (≥P90). Bars show the adverse and
favorable changes from the overall median; inputs are ordered by the spread between conditional
medians. The report retains both medians, group sizes, and the Spearman rank correlation.

This is sensitivity derived from the Monte Carlo samples, **not a one-at-a-time causal effect**.
A constant input has undefined rank correlation (blank in the report). Ties can make a group
larger than 10%. Small effects may be sampling noise; increase iterations and check stability
before interpreting their order. Percentiles describe the chosen hypothetical distributions;
they are not confidence intervals or calibrated business probabilities.

## Dependence, precision and convergence

`correlation` is a square numeric matrix in input order, symmetric with diagonal one, entries in
[-1,1], and positive semidefinite. Singular matrices (including perfect ±1 relationships) are
supported. Invalid matrices are rejected, not replaced by a nearest correlation matrix.
Cholesky validation uses a 1e-10 tolerance for floating-point residuals near zero.

The Gaussian copula transforms independent normal coordinates with this matrix, then maps to
the requested marginals. Correlation of non-normal final inputs generally differs from the
latent target. Both observed Pearson and Spearman correlations are reported. A Gaussian copula
does not model every kind of dependence or tail dependence.

For LHS, each **independent driving uniform** is stratified within each replica. Subsequent
copula transformations need not preserve Latin strata in the final input coordinates. Each
replica is randomized independently, so sampling error is estimated from replica means rather
than incorrectly treating dependent LHS rows as IID. LHS can reduce variance, particularly for
smooth/additive responses, but improvement is not guaranteed for every workbook.

- **Random Monte Carlo:** mean standard error is sample SD / sqrt(N). Mean intervals use the
  Student t critical value; probability-above-zero intervals use Wilson score bounds, including
  non-degenerate bounds when no event or every event was observed.
- **Replicated LHS:** errors come from independent replica means/proportions; Student t
  intervals use replica count - 1 degrees of freedom. When replica proportions have no observed
  variation, their confidence interval is left blank with an explicit warning: zero observed
  crossings do not prove zero tail risk.
- Mean/t intervals are approximate for non-normal data or few independent units. Percentiles
  are not confidence intervals. Confidence intervals quantify finite-simulation error under
  the chosen distributions, not uncertainty in the distributions or business assumptions.
- A convergence trace uses complete random prefixes or complete independent LHS replicas.
  Intervals need not shrink monotonically. It is a diagnostic, not an automatic stop rule or
  a guarantee that rare events have been adequately sampled.
- Means use compensated summation, centering same-sign samples around an origin and preserving
  small cancellation residuals for mixed signs. Variance calculations scale deviations to
  avoid overflow and preserve small variation around a large offset. Numeric overflow still
  aborts the report atomically instead of publishing a misleading value.

References (mathematical descriptions, no implementation code copied):
[NIST mean intervals](https://www.itl.nist.gov/div898/handbook/eda/section3/eda352.htm),
[NIST proportion intervals](https://www.itl.nist.gov/div898/handbook/prc/section2/prc241.htm),
[Sandia Dakota sampling](https://snl-dakota.github.io/docs/latest_release/users/usingdakota/studytypes/uq.html).

## Limits and reproducibility

- Discrete distributions, non-Gaussian copulas, background execution/cancellation and percentile
  fan charts are future work. The report supports up to 64 formula outputs for monthly forecasts.
- At most 32 inputs, 64 outputs, 2–100,000 iterations, and 1,000,000 input/output sample cells.
  Limits apply even when the sample table is omitted. Large formula models may take time.
- The seed controls the input RNG. Workbook formulas such as `RAND`/`NOW`, external data and
  floating-point platform differences are outside that reproducibility contract. Use
  deterministic formulas when reproducibility matters.
- What-if trials always recalculate, including in manual calculation mode, without changing the
  workbook's calculation setting. Trial failures and non-finite sampled/results values abort
  without modifying the workbook. Report creation is refused when structure is protected.
- Results and charts round-trip as ordinary XLSX data. Simulation runs in GridCraft's Rust engine.

## Implementation

`crates/engine/src/cmd/monte_carlo.rs` reuses `whatif::Trial`: copy-on-write workbooks and
incremental recalculation. `crates/ui-egui/src/dialogs/monte_carlo.rs` is a thin parameter editor.
Distribution moments, analytic covariance, singular/invalid matrices, LHS strata, NIST interval
reference values, high-offset numerical stability, repeatability, cross-sheet/manual recalculation, error atomicity, undo/redo and
XLSX round-trip tests use synthetic workbooks built in Rust.

## Follow-up hardening

These are planned shared analysis improvements; the current command remains synchronous with
the per-run limits above.

- [ ] Define a shared workload budget for expensive analysis commands, including formula
  evaluation cost and repeated automation requests.
- [ ] Add cooperative cancellation and progress at a shared evaluator boundary. Cancellation
  before report publication must preserve source cells and leave no partial report.
- [ ] Exercise cumulative report/history growth and end-to-end interruption through the
  control/MCP paths, including the behavior of volatile or externally backed formulas.
