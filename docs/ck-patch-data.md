# CK patch-data authoring

> **Last reviewed:** 2026-10-20 · **Last updated:** 2026-10-20 · **Change:** new authoring extension; not an Excel parity claim

Open a `ck-patch-data*` or `patch-data-ck*` folder (or its `protobuf` folder). The explorer opens the CK data panel automatically. For a differently named checkout, choose **CK 규칙 → 이 폴더에 CK 규칙 적용**. The profile can be disabled without changing any workbook. Generic folders stay generic.

The right panel explains the selected column, edits its flags/type/name and literal cell value, and offers Bool, Enum and FKey/StringIdReference candidates. Lists accept multiple candidates. Alias definitions are available as a reference. Header changes use one undoable `range.setValues` command; values use `cell.set` with literal text to preserve long integers, IDs and leading equals signs. To author a formula, use the normal formula bar. A document/revision check prevents applying an old draft to a changed document.

**다시 검사** reads eligible workbooks in the background and overlays unsaved edits and explicitly opened decrypted/imported documents. Diagnostics are hidden by default; **진단 열기 / 진단 닫기** in the right panel shows or hides the paginated results table. The bottom panel also has a close button. Visibility is a saved UI preference and refreshing does not reopen a dismissed panel. Clicking a diagnostic location activates its file and sheet, selects the exact cell, and scrolls the grid to reveal it, including after switching from another document. Opening an unchanged saved file keeps the index current. Edits make the previous result visibly stale; disk changes require an explicit refresh. The version selector previews filtering, including an optional `BuildName` target. It does not delete rows or change a publishing configuration.

## Format evidence and implemented checks

Reviewed the adjacent CK patcher source at revision `b3e03786`, specifically `crates/patcher-data/src/patch_data/{schema_layout.rs,io.rs,entities/game_data/column_types.rs,entities/game_data/column_flags.rs,logic/*}`, and the Excel add-in's schema/editor behavior. Implementation is independent; there is no runtime/build dependency on the patcher or a particular data checkout. The current backend takes precedence over the older format guide and add-in type menu.

| Contract | GridCraft behavior |
|---|---|
| Top-level `protobuf/*Data.xlsx`; case sensitive; leading `_`/`~` files excluded | Same file eligibility; strict XLSX reader, no CSV-by-extension fallback; nested files are not indexed |
| Flags / type / field name in rows 1 / 2 / 3 | Selected-column editor and diagnostics; data starts at row 4 |
| `_` sheet exclusion; `_Transpose` suffix | Ignored-sheet explanation; transposed headers and diagnostic cell coordinates |
| Same table prefix before `_` | Merged header comparison and shared key/reference candidates |
| Blank flags; Design | Blank flags excluded; Design permits arbitrary type annotations and skips value conversion |
| Target flags and ID-role conflicts | Checks for exactly one target (SplitBinaryName exception), Version-only, unknown flags, incompatible ID flags |
| CK direct `TString` restriction | Error only for direct `TString`; does not incorrectly reject nullable/list TString variants |
| Type grammar and value hints | Scalars, nullable, vectors/grids, Enum, TString families, lists and inner list aliases; nested `List<List<T>>` rejected |
| Basic values | Integer range, numeric separators, Bool, JSON, scaled numbers, vector arity; quoted list delimiters and empty fields |
| Key / Unique / CompoundKey / GroupKey | Cardinality, blank restrictions, duplicate values/combinations and contiguous groups |
| SplitBinary flags / ZipWith | Paired/cardinality constraints and element-count checks |
| ResourceVersion | Version target and row preview; FKey candidates/checks use the filtered rows |
| FKey | Cross-workbook scalar Key candidates, list element checks, nullable/list target-key rejection; GroupKey is not a target |
| Alias / string IDs | Single-pass substitution, undefined Alias/ID diagnostics, whitespace-normalized IDs, Raw/Design exclusions |

## Deliberate limits

This is **local authoring assistance, not the patcher's publish validator**. DateTime/Duration conversion, ZoneOffset, complete typed equality for temporal/vector/JSON/translated values, ID-map assignment/obfuscation and banned/deprecated IDs, translation-key collisions, resource-version groups, enum/alias metadata constraints, protobuf generation, remote revision checks and publishing remain the patcher's responsibility. Displayed integer/string reference candidates preserve symbolic IDs; no ID numbers are invented or assigned.

The project worker visits every eligible file, with no file-count, cumulative-byte or cumulative-cell budget. It reads cached XLSX values (including spill followers), retaining the normal package decompression and input safety limits. Presentation parts do not affect data-index completeness. Data errors, missing cached formula values and unreadable files are explicitly reported as incomplete; manually opened documents supply their current values, including unsaved edits.

One workbook at a time is converted to small logical row chunks (at most 256 rows, normally 32K values) in an automatically removed temporary file. Three passes collect metadata, build version-filtered reference sets, then check every row. Only headers and reference/constraint sets stay resident across files. Folder/profile changes cancel obsolete work between chunks/files. `patch.inspect.progress` reports discovered/processed/failed files, stage and checked rows. The UI continues repainting during indexing.

Every diagnostic is counted in `report.total_issues`; the first 2,000 are retained for the 25-row paginated preview (`truncated` indicates additional findings). This display limit never stops row validation. The synchronous single-workbook `patch.validate` query retains its bounded input limit; full-project checking uses the streaming worker. Indexing never writes source workbooks, generated data or ID maps.

Each explorer file has a disclosure arrow for its worksheet children. XLSX expansion reads only the workbook manifest on a background worker; clicking a worksheet opens its file and activates that sheet. Open documents supply live names, including unsaved renames. Folder refresh reloads closed-file previews.

Explorer clicks decode and recalculate the requested workbook on a separate worker, with a loading label. Rapid selections coalesce; incomplete edits and dialogs are not interrupted. Open documents retain unsaved values and undo history. Conditional-format range statistics persist across frames, invalidate on workbook/sheet changes and are borrowed rather than copied per visible cell. Cell-local formatting rules never scan their full ranges. Reference candidates are enumerated only inside an open dropdown.


Automatic activation is a folder-name convention, not proof of repository identity. A renamed clone needs explicit profile selection. Folder indexes are desktop-only; the pure engine schema checks and explicit `patch.validate` command also compile to WASM. Panel copy currently targets the Korean CK authoring workflow.

## Commands and verification

- `patch.validate {project: "ck", version?: name}`: synchronous read-only active-workbook query, explicitly incomplete for cross-file references; available through engine/CLI/MCP.
- `patch.profile {mode: "auto"|"ck"|"off"}`, `view.patchData {on?: bool}`: desktop profile/panel.
- `view.patchDiagnostics {on?: bool}`: independently show/hide the diagnostics table without discarding results.
- `patch.refresh {}`, `patch.version {name: version|"@build"|""}`: async workspace checks.
- `patch.inspect {}`: loading/error/stale state, progress, indexed file names, report and skipped-input notes.
- `patch.jump {file,sheet,cell}`: navigate within the current indexed files.
- `patch.apply {uid,revision,sheet,cell,value}` or `{uid,revision,sheet,cell,kind:"schema",flags,type,name}`: guarded literal editing through existing engine commands.

Regression coverage: `crates/engine/tests/patch_data.rs`, `crates/ui-egui/tests/patch_data.rs`. Fixtures are constructed with GridCraft in temporary directories; no patch-data source files or vendor-created assets are committed. Offscreen UI verification uses the snapshot example with `--folder`, `--in` and `--cmd`.

## Measured verification (2026-10-20)

Read-only run against the local `ck-patch-data-4/protobuf` checkout: all **170 eligible files**, **1,253 included worksheets**, and **555,747 logical data rows** were checked. An independent ZIP/XML traversal matched the file set and row count exactly. No read failures or incomplete-context warnings remained. Existing data produced 11,190 findings; only the first 2,000 are displayed, as described above.

On this development machine, the 1920×1000 offscreen benchmark with `CakeTycoonData.xlsx` open completed project validation in **54.3 s**. Frame time: **10.20 ms median / 11.06 ms p95 / 24.48 ms maximum**; queuing the background file open took **0.03 ms**. The separate scrolling benchmark improved from **672.70 ms median / 697.99 ms p95** to **13.28 ms median / 14.44 ms p95** after the conditional-format changes. These measure CPU-side egui frames, not display refresh or GPU presentation.

Reproduce with `cargo run -p gridcraft-ui-egui --example perf_workspace -- <protobuf-folder> <CakeTycoonData.xlsx>` and `cargo run -p gridcraft-ui-egui --example perf_workbook -- <CakeTycoonData.xlsx> 40`. Benchmarks only read source workbooks and keep results outside the repository.

## Revision history

| Date | Change |
|---|---|
| 2026-10-20 | Diagnostic location clicks now scroll the selected cell into view |
| 2026-10-20 | Hidden-by-default, reopenable diagnostics and lazy per-file worksheet navigation |
| 2026-10-20 | Complete disk-backed project indexing, cached-value reader, progress/cancellation, background file opening and conditional-format cache |
| 2026-10-19 | CK profile, background workspace index, selected-column editor, version preview and navigable diagnostics |
