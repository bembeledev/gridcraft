# Roadmap detail

> **Last reviewed:** 2026-10-13 · **Last updated:** 2026-10-13 · **Change:** minor (audit against main: current focus, alpha-gate evidence, beta-gate status and milestones brought up to date) · **Target:** Microsoft Excel (Microsoft 365)

Forward-looking plan. The one-page summary is [`ROADMAP.md`](../ROADMAP.md); the ranked work
list is [`gaps.md`](gaps.md); numbers and method are in
[`target-app-parity.md`](target-app-parity.md). Estimates are Opus 5.5 agent-hours.

## Current focus

1. **Review and land the open PR queue** (225 open PRs on 2026-10-12). The 10-10 queue items
   are resolved: recalc and spill fixes (#148, #155, #156, #157), whole-sheet and large-range fixes
   (#151, #152, #153, #154), chart kinds (#158, #161), macOS document open (#164) and 15-digit
   comparison (#187) are merged; the German UI, numbers and formulas PR (#163) and #167 were closed
   without merging (German UI landed separately as #271). Language work extends the `i18n` catalog
   past the ribbon rather than adding scattered literals. Hours not re-estimated (was 8–15 h).
2. **XLSX on real files** (gaps #1, #3): corpus run plus Excel opening our output.
3. **Large workbooks and multi-threaded recalc** (gap #2).

## Alpha gate

The core workflows a typical Excel user runs every day, checked end to end on the main platform
(macOS desktop), including saving and reopening the work. Any "no", or a "partial" that blocks the
workflow, would make GridCraft pre-alpha.

| Core workflow | Works end to end? | Evidence | Hours to pass |
|---|---|---|---|
| Build a model: enter data and formulas, recalc, save XLSX, reopen | yes | 504 working functions, dynamic arrays, LET/LAMBDA, names, tables; XLSX round-trip tests (`crates/xlsx/src/tests/roundtrip.rs`). The listed edge-case bugs (#57 15-digit equality, stale spills #133/#139) were fixed in #187 and #148; more edge cases may remain (no Excel-computed oracle corpus is checked in) | 0 |
| Open a colleague's XLSX, edit, save, hand it back | partial, not blocking | Typical workbooks open and save with cells, styles, CF, validation, tables, names and pivots; some unmodelled parts are still dropped (slicers, timelines, data model; gap #3), others are kept unchanged on save (pass-through, #333); `.xls` and password-encrypted files open since #332 and #436; Excel-side repair prompts unverified (gap #1); ~100 MB files fail (#175) | 0 for alpha (gaps #1–#3 are beta gates) |
| Format and present: number formats, styles, conditional formatting, print or PDF | yes | Full number-format language, 47 cell styles, all CF kinds, Page Break Preview, PDF export and printing. PDF can't carry non-Latin text (gap #5) | 0 |
| Analyze: sort, filter, tables, PivotTable | yes | Multi-level sort, AutoFilter, tables with totals, PivotTables with field list, layouts, grouping, refresh and XLSX round-trip. Calculated fields (#306), PivotCharts (#313) and table slicers (#193) are done; PivotTable slicers and timelines are missing (gap #8) | 0 |
| Chart the data | partial, not blocking | Column, bar, line, pie, area, scatter and combo charts are created, edited and survive save and reopen; histogram, box, waterfall, funnel, treemap, sunburst, stock and combo keep their kind in XLSX since #158 and #161, and trendlines are read, written and drawn (#424); no axis options or error bars (gap #4) | 0 |
| Import and export CSV | yes | Encoding and delimiter detection on read, CSV/TSV write the displayed values (#422) and keep the separator the file was opened with (#427) | 0 |

**Passes:** every core workflow works end to end on macOS with save and reopen; the two partial
rows lose fidelity only on less common content (very large files, unmodelled parts, chart options), which are beta gates, not alpha blockers. Ready for real work ~50% is inside the
40–75% alpha band.

## Beta gates

Beta means: ~75% ready for real work, opens and saves XLSX reliably, remaining gaps are known bugs
and edge features. Each gate links its gap.

| Gate | Gap | Hours | Status on main (2026-10-12) |
|---|---|---|---|
| XLSX proven on a real-world corpus; Excel opens our output without repair | #1 | 15–25 | open; `gridcraft-cli check` (#416) measures calculation and round-trip on any file set, no corpus run is recorded |
| Unmodelled XLSX parts preserved on save | #3 | 10–15 | partly: pass-through (#333) keeps custom XML, external links, connections, controls, OLE and more; slicers, timelines, data model, table query tables and unmodelled `extLst` entries still dropped |
| 100 MB workbooks open; 1M rows usable; multi-threaded recalc; 100k-edit recalc < 100 ms | #2 | 25–40 | open; #175 open (a part is capped at 512 MiB decompressed), no crate depends on `rayon`; range-formula recalc is linear (#66) |
| Charts: kinds and options survive round trip; axes, trendlines, error bars | #4 | 25–35 | partly: kinds (#158, #161) and trendlines (#424) done; axes, error bars and per-point formatting open |
| Text in every script renders and exports | #5 | 12–20 | open: CJK/Thai/Hebrew fallback (#38) only; no shaping; PDF uses the 14 standard fonts |
| Calculation correctness bugs closed; iterative calculation | #6 | 10–15 | the listed bugs are fixed (#187, #148, #155, #156, #157, #178, #183) and iterative calculation landed (#421); no oracle corpus yet |
| `.xls` import, password-protected workbooks | #7 | 28–42 | partly: `.xls` (BIFF8) values, formulas and number formats open (#332) and encrypted workbooks open (#436); `.xls` styles and objects, saving with a password open |
| PivotTables: slicers, calculated fields, PivotCharts, verified in Excel | #8 | 20–30 | partly: calculated fields (#306), PivotCharts (#313) and table slicers (#193) done; PivotTable slicers, timelines, calculated items, GETPIVOTDATA and Excel verification open |
| Keyboard: key tips, shortcut audit, context menus | #9 | 12–20 | key tips (#411, #412) and the shortcut audit (#319) done; context menus open |
| Field crashes and freezes closed; autosave/recovery soak | #10 | 8–12 | fixed: #120 (#315), #117 (#316), #130 (#179), #69 (#164); #166 and #76 still open |
| **Total** | | **~165–255 (≈160–240 after overlap with the PR queue)** | hours not re-estimated |

With 4–5 agents and an integrator, about 40–60 wall-clock hours. Needs the owner: running Excel
to check our files, and Windows machines with the reported GPU drivers.

## Milestones

| # | Milestone | Status |
|---|---|---|
| M0 | Foundation: core, number formats, formula language, functions, model, calc, XLSX/CSV, engine | done |
| M1 | Excel look: chrome, ribbon, formula bar, grid, tabs, status bar, control channel | done (polish continues) |
| M2 | Formatting completeness | done |
| M3 | Editing power (AutoComplete, spelling, drag-move/copy) | done |
| M4 | Data (advanced filter, consolidate, what-if) | most; Solver (#327), Analysis ToolPak (#320) and Forecast Sheet (#325) landed; Power Query remains |
| M5 | Formulas tab (trace arrows, watch window, evaluate, error checking) | done |
| M6 | Conditional formatting manager parity | done |
| M7 | Charts parity | partial; Format pane, kind round-trip (#158, #161) and trendlines (#424) in; axes and error bars remain |
| M8 | Insert & Review | partial; ink, icons, comments pane in |
| M9 | Page layout & print | most; Page Layout view landed (#334); margin dragging on the ruler landed (#794); header/footer pictures remain |
| M10 | PivotTables, Solver, Analysis ToolPak | PivotTables, PivotCharts, calculated fields, table slicers, Solver and Analysis ToolPak in; PivotTable slicers and timelines remain |
| M11 | Automation (record actions, scripts) | done |
| M12 | Performance, accessibility, i18n | partial; ten interface languages besides English with Spanish formulas; multi-threaded recalc and an accessible grid tree remain |
| M13 | Release & polish | releases shipping (tags v0.1.0, v0.3.0, v0.4.0, v0.5.0, v0.6.1; signed) |
| M14 | **Beta gates** (table above) | not started as a milestone; parts landed (see the status column) |
| M15 | Localization infrastructure and the twelve languages | started: catalogs for ja, zh, ko, pt-BR, ru, de, cs, es, fr, sr (#31, #104, #82, #271, #335, #242, #413, #453); Spanish formulas (#98); more languages in open PRs |
| M16 | Get & Transform, data model | not started |

## After beta (ranked)

| Work | Gap | Hours |
|---|---|---|
| Localization: catalog past the ribbon, then twelve languages | #11 | 100–165 |
| Get & Transform and the data model | #12 | 60–100 |
| Solver, Analysis ToolPak, Forecast Sheet (all landed: #327, #320, #325; Forecast Start landed in #786) | #13 | 22–35 (not re-estimated) |
| Remaining functions, oracle suite | #15 | 18–30 |
| Collaboration and review | #16 | 35–55 |
| Insert objects | #17 | 25–40 |
| Accessibility | #18 | 12–18 |
| Page Layout view (landed, #334), print polish | #14 | 12–20 (not re-estimated) |
| Mobile layout | #23 | 30–60 |
| AI features with local models | #22 | 30–50 |
| Plugin / custom-function API | #21 | 25–40 |

## Measuring

- `cargo xtask parity` regenerates [`parity-checklist.md`](parity-checklist.md) (ribbon
  catalog vs registered commands); `cargo xtask ci` checks it is current.
- `cargo test --workspace`: engine, formula, function, XLSX and UI behaviour.
- `cargo run --release -p gridcraft-ui-egui --example snapshot -- --sample sales out.png`: look at
  the UI offscreen.
- Function coverage: the script method in [function-parity.md](function-parity.md). A
  `cargo xtask functions` that diffs the registry against a checked-in Excel name list would make
  it a CI number (1–2 h).
- No `cargo xtask scorecard` or `bench` yet.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-13 | minor | Re-verified after the 2026-10-12/13 merges: M9 ruler margin dragging (#794) and the Forecast Start option (#786) are landed; ten interface languages |
| 2026-10-12 | minor | Audit against main: current focus, alpha-gate evidence, beta-gate status column, milestones M4, M7, M9, M10, M12–M15 and two after-beta rows updated; hours not re-estimated |
| 2026-10-11 | minor | PivotTable calculated fields landed |
| 2026-10-10 | minor | Current focus and M15 updated for the 10-10 merges |
| 2026-10-10 | minor | Added the alpha gate table (six core workflows; passes, stays alpha) |
| 2026-10-10 | major | Created: current focus, beta gates (~160–240 h), milestones moved from ROADMAP.md, post-beta ranking |
