# Lithuanian interface

> **Last reviewed:** 2026-10-18 · **Last updated:** 2026-10-18 · **Change:** minor (Lithuanian interface draft) · **Target:** GridCraft 0.8.0 source

Choose **View › Interface language › Lietuvių**, or choose **Lietuvių** in File › Options.
The `lt` preference is saved with the other interface settings. On first run, Lithuanian system
locale tags such as `lt-LT` and `lt_LT.UTF-8` select it through the existing locale matching.
Agents can use `app.language.set` with `{"language":"lt"}` (or the `code` alias).

The catalog has **972 entries**: every English key from the existing language catalogs and the
literal interface labels already looked up through `Language::tr` at the time of this change.
It covers ribbon controls, File pages, shared dialog labels, Format Cells, analysis dialogs,
printing, chart controls and panes where those call sites already use the translation system.
Strings that are not looked up through that system still appear in English, as they do for the
other interface languages. This is not a claim of complete translation of every visible string.

## Scope

This contribution changes the interface only. Lithuanian uses the existing English formula locale:
function names, argument separators and decimal syntax in formula entry keep their current values.
Workbook cells, sheet names, custom names, formats, files, command ids and APIs are not translated.
Lithuanian spelling dictionaries, proofing and locale-specific number/date formatting are not added.

The draft was prepared for @ulrichas with Codex from the meaning of the repository's English labels.
No Microsoft Excel or other product's localisation, string table or terminology database was copied.
Native-speaker terminology review and review of the running application are pending.

## Terminology and conventions

| English | Lithuanian |
|---|---|
| Workbook / Sheet / Cell / Range | Darbaknygė / Lakštas / Langelis / Diapazonas |
| Home / Insert / Page Layout | Pradžia / Įterpti / Puslapio maketas |
| Formulas / Data / Review / View | Formulės / Duomenys / Recenzavimas / Rodinys |
| Save / Undo / Redo | Įrašyti / Anuliuoti / Grąžinti |
| PivotTable / Sparkline | Suvestinė lentelė / Miniatiūrinė diagrama |
| Conditional Formatting / Freeze Panes | Sąlyginis formatavimas / Fiksuoti sritis |
| Header / Footer | Puslapinė antraštė / Poraštė |

- Commands use infinitives, and labels use sentence case.
- Context keys distinguish text orientation (`Orientation` → `Teksto kryptis`) from page orientation
  (`Page Layout|Orientation` → `Puslapio padėtis`), and the General number format from General options.
- Counts use neutral labels (`Rasta langelių: {count}`) or unit abbreviations (`Prieš {n} d.`) to
  avoid incorrect Lithuanian endings for counts such as 1, 2, 10, 11, 21 and 111.
- Ribbon line breaks, placeholders, shortcuts, file extensions and dialog ellipses are preserved.
- Names such as GridCraft, Excel, CSV, PDF, VBA and MCP keep their spelling. Numeric examples in
  input fields retain the syntax that those fields currently accept.

## Validation

Local validation checked the 972 catalog keys against all existing language catalogs and literal
translation call sites, plus duplicates, placeholders, whitespace, Unicode normalization, line
breaks and trailing ellipses. The two changed Rust files pass `rustfmt --check`.

Three focused Rust catalog tests were compiled for `wasm32-wasip1` using the official portable Rust
compiler and executed under Node's WASI runtime: context and workbook syntax; neutral counts; and
ribbon line breaks/placeholders. All three passed. This does not replace the application's tests.

The existing shared catalog test now requires Lithuanian coverage for every Japanese reference key.
A UI regression checks locale aliases, saved preferences, the language command, English formula
syntax and unchanged workbook contents after switching the interface. Full `cargo xtask ci`, that
UI regression and a running-app screenshot were not run locally because Cargo and the native build
dependencies are not installed. Those checks and native-speaker review remain required before merge.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-18 | minor | Lithuanian interface draft with 972 entries, locale registration, focused catalog tests and UI regression; formulas and proofing unchanged |
