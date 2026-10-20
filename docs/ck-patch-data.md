# CK patch-data authoring and validation

> **Last reviewed:** 2026-10-20 · **Last updated:** 2026-10-20 · **Change:** external patcher validation replaces all GridCraft CK rules

Open a `ck-patch-data*` or `patch-data-ck*` folder (or its `protobuf` folder). The CK panel opens automatically. Other folders can opt in with **이 폴더에 CK 규칙 적용**. Folder and worksheet exploration remain independent of validation.

## One validation implementation

GridCraft has **no CK validation rules**. Its former schema/type parser, key/reference/version/alias checks and local diagnostic generator have been deleted. All diagnostics now come from the patcher's separate **`ck-validator`** executable:

```text
GridCraft → ck-validator (JSONL v1) → patcher-validation → patcher-data
```

The worker uses the patcher's existing data-loading, schema, version and typed validation passes. GridCraft transports evaluated unsaved values and displays the returned report without rewriting its messages or inventing cell locations. A missing worker, missing ID map, malformed response, changed input or failed process is an error; there is no local fallback. The private patcher crate and its rules are neither linked nor copied into this public repository.

The patcher stops on its first failed pass. The UI shows **통과 / 실패 / 미실행** for each of raw, parse, versions and validation. A failed early stage does not imply that downstream checks ran. A local pass covers the worker's data-validation pipeline, not protobuf encoding, remote revision/assets checks or publication.

## Setup

Build `ck-validator` in the patcher repository using that repository's pinned Rust toolchain:

```sh
cargo build --release --locked -p ck-validator
./target/release/ck-validator --version
```

In GridCraft's **패처 검사 설정**, set the executable's absolute path (or `ck-validator` on PATH), the directory containing the **real `PatchDataIdMap.csv`**, and the same TC ID prefix and version options as the intended patcher operation. **설정 적용** stores settings per project in local UI preferences. No settings file is written into the data checkout. An empty target version uses the project's `BuildName` file. Version filtering is explicit.

Optional launch defaults:

```sh
GRIDCRAFT_CK_VALIDATOR=/absolute/path/to/ck-validator \
GRIDCRAFT_CK_ID_MAP=/absolute/path/to/ck-id-map \
GRIDCRAFT_CK_TC_ID_PREFIX=TC_ \
cargo run -p gridcraft -- /absolute/path/to/ck-patch-data
```

Saved per-project settings take precedence. Pin the worker and ID-map revision to those used by your patcher deployment. The panel and `patch.inspect` expose protocol, package and source revision. Compatibility was verified against worker revision `0cdf70cccc80bf0b164d4fc545016e926bf50799`, protocol 1. GridCraft does not infer server revision equivalence from the package version.

## Editing, progress and navigation

**다시 검사** starts a background worker. It reads the full project's saved workbooks, `ZoneOffset`, `BuildName`, `PatcherConfig.toml` and the selected ID map itself. It receives whole-workbook overlays for unsaved edits, including renamed/deleted sheets, evaluated formulas and spill followers. Physical blank rows/columns and text integers are preserved. Invalid/unavailable evaluated values block transport with their file/sheet/cell; they never become empty cells. The protocol's 2,000,000 rectangular-cell and 64 MiB overlay limits fail explicitly and do not truncate validation of saved files. Save large edits before retrying.

There is one process per requested check. **취소**, folder/profile changes and applying new settings terminate and reap it. A 30-minute deadline bounds stalled processes; requests and responses have byte limits. Temporary transport files are removed automatically. The worker never receives authority to save the source workbooks or ID map; its API performs read-only validation with in-memory ID allocation simulation.

Changing a document during validation discards that result. Editing after completion marks the displayed result stale. Disk changes during the check are detected by the worker; refresh explicitly after later external edits. Validation is not run on every keystroke.

Diagnostics remain hidden by default and can be opened/closed independently of the side panel. The paginated table shows 25 entries per page and the total/truncation count supplied by the worker. A location click opens its workbook and sheet; when both physical coordinates exist, it selects and scrolls to that exact cell. Table-only errors remain table-only. Related locations are separately navigable. No coordinates are extracted from error-message text.

The header and literal-value editor remains undoable and guarded by document UID/revision. Its former independent type/flag warnings and locally calculated reference candidate menus were removed with the local validator. Type validity and cross-file relationships are determined by the patcher. Long literal integers, symbolic IDs and leading equals signs remain text; author formulas in the regular formula bar.

## Commands

| Command | Parameters / behavior |
|---|---|
| `patch.profile` | `{mode: "auto"|"ck"|"off"}` |
| `patch.configure` | `{executable?, id_map_directory, tc_id_prefix?, build_name?, version_filtering?}`; persists per folder and cancels old checks |
| `patch.refresh` | `{}`; async full-project check with unsaved overlays |
| `patch.cancel` | `{}`; cancel the active check |
| `patch.inspect` | `{}`; backend/config/progress/error/stale, exact report and validator provenance |
| `patch.version` | `{name: ""|"@build"|target}`; disable filtering, use BuildName, or choose a target, then check |
| `patch.jump` | `{index, related?: index}`; navigate to a worker-returned diagnostic location |
| `patch.apply` | `{uid, revision, sheet, cell, value}` or `{..., kind:"schema", flags, type, name}` |
| `view.patchData` / `view.patchDiagnostics` | `{on?: bool}`; independently control panel visibility |
| `patch.validate` | `{project:"ck", root, config:{...}}`; synchronous engine/CLI query using the same worker, with unsaved file-backed documents |

External processes are desktop-only. WASM returns an explicit unsupported error; it contains no substitute CK validator.

## Verification

Synthetic fixtures are authored with GridCraft; no real workbooks, ID maps or worker binaries are committed. Standard engine tests cover transport identity/version checks, abnormal exit, missing configuration, cancellation/reaping, input limits, formula errors, physical addresses and exact report forwarding. UI tests cover literal editing and per-project configuration without requiring the private dependency.

The real-worker integration tests are explicit (ignored by the public default suite):

```sh
GRIDCRAFT_TEST_CK_VALIDATOR=/absolute/path/to/ck-validator \
cargo test -p gridcraft-ui-egui --test patch_data -- --ignored
```

They exercise saved-file diagnostics, unsaved repairs, engine/UI report equivalence, unchanged source/ID-map bytes, missing/corrupt inputs, failed/skipped version stages, stale in-flight edits, cancellation, diagnostic panel toggling and actual cell-click navigation.

A read-only run on the local CK checkout and the patcher's managed ID-map checkout produced the same seven diagnostics and stage outcomes in the native GridCraft window and direct `ck-validator check`. Raw, parse and versions passed; validation failed on existing data. SHA-256 comparisons of 173 input files (workbooks, metadata and ID map) found no changes. This verifies report forwarding, not that the data is valid.

## Revision history

| Date | Change |
|---|---|
| 2026-10-20 | Removed all local CK checks; use only external patcher reports with unsaved overlays, cancellable background execution and per-project configuration |
| 2026-10-20 | Diagnostic location navigation, hidden-by-default results, worksheet exploration and responsive opening |
| 2026-10-19 | Initial folder workspace and CK authoring panel |
