# Actuarial validation evidence

Tests: new regression for monetary/statistical calculations.

Final verification completed 2026-10-28: **PASS** for the documented implemented scope.

Integration base: `02e4d50088e532f81a96f80eab539cabf77da7e4`, local branch
`feat/actuarial-pack`. The supported estimands/domains are defined in
[the pack contracts](actuarial.md) and [scientific-method contracts](actuarial-scientific.md).
Validation applies to those implemented domains, not every actuarial model or a particular
real portfolio. All fixtures are original synthetic data or independently calculated values.

## Reproducible artifacts

- [Foundational workbook command script](examples/actuarial.jsonl): life, losses, reserves,
  Markov and pricing. Recalculation, XLSX round-trip, undo and MC source isolation are tested.
- [Scientific workbook command script](examples/actuarial-scientific.jsonl): censoring,
  Bayesian prediction, Cox, EVT, dual sensitivities, reinsurance, ALM, validated surrogate,
  semi-Markov and externally supplied bootstrap replications. Also round-tripped through XLSX.
- [Independent numerical reference cases](../crates/engine/tests/fixtures/actuarial-reference.json):
  54 engine-boundary fixtures with closed-form/Decimal oracles, including intentionally invalid
  models. Expected values are not copied from the product's observed outputs.
- Mathematical unit tests in each `functions::actuarial` module cover model identities,
  invalid/hostile input, finite-work limits and numerical edge cases.

## Independent scientific review

A separate, explicitly authorized reviewer uses the actual CLI/MCP executable, not mocks.
Checks include closed-form identities, negative controls, GPD parameter recovery, independent
Cox score roots, a 200-dataset illustrative Beta interval experiment, uniform copula margins
and 2-increasing rectangles, a direct-PDF Gamma integration for bootstrap process draws and
hand-calculated Mack cross-origin covariance. Empirical coverage from a finite synthetic
experiment is not a universal coverage guarantee or a real-data calibration claim.

Independent review identified and drove corrections for cancellation in compound TVaR,
small-shape negative-binomial recurrence, tiny joint copula tails and Frank's independence
limit, extreme Gamma credible endpoints, undefined KM confidence bounds at survival 0/1,
and IPCW event/censor tie ordering. The pre-fix executable was preserved outside the repo as
a RED control, and the same oracle cases are run against the corrected executable.

## Gates and completion checklist

- [x] Implement all 95 worksheet functions with explicit contracts and no duplicate RNG.
- [x] Ship two synthetic replay scripts and source-level analytic/regression tests.
- [x] Archive independent expected numerical values at the engine boundary.
- [x] Pass final complete `cargo xtask ci`: all 10 gates, 2,637 workspace tests across
  24 test binaries, including 84 actuarial unit tests and four workbook/engine tests.
- [x] Independent review: 52/52 closed-form probes; 6/6 bootstrap/Mack cases;
  6/6 recovery/interval checks; 9/9 copula property configurations (324 CDF queries).
  Real MCP stdio initializes successfully and discovers all 95 functions.
- [x] RED control: the same 52-case independent battery passes 42 cases on the old
  executable and all 52 on the corrected executable, preserving ten expected failures.
- [x] Rustdoc: all 274 production functions/methods in the extension have attached `///`;
  non-rustdoc source content was verified unchanged by the documentation pass.

The full CI passed i18n-report, formatting, Clippy, workspace tests, doctests, assets,
dependency layers, WASM, command parity and generated keytips. The independent executable
probes additionally verified the CLI/MCP path and deliberately invalid models. The archived
54 engine-boundary cases preserve the closed-form/Decimal expectations in normal repository CI.
`cargo doc -p gridcraft-functions --no-deps --document-private-items` succeeded with zero
diagnostics in the actuarial modules; preexisting financial/private-link warnings remain
outside the extension. No mathematical defect reproduced by the independent reviewer remains open.

The full gate uses optional official `craft-fonts` inputs and local sockets for existing
control-server tests. Low-disk native builds can disable incremental/debug information:

```sh
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 \
  CRAFT_FONTS_DIR=/path/to/craft-fonts cargo xtask ci
```

An initial full debug build exhausted disk; its build artifacts were cleaned. The first
compact sandboxed run hit local socket permissions. Neither attempt is counted as a pass.
Final execution evidence is recorded only after the latest source passes the complete gate.
