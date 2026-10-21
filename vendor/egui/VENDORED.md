# Vendored egui 0.36.2 (with right-to-left grids)

This is [egui](https://github.com/emilk/egui/tree/main/crates/egui) 0.36.2 as published on
crates.io, patched into the workspace by `[patch.crates-io]` in the root `Cargo.toml`, next to the
vendored epaint (see `vendor/epaint/VENDORED.md`). egui is (c) Emil Ernerfeldt and the egui
contributors, dual-licensed MIT OR Apache-2.0 (their upstream license files, unchanged from egui 0.36.2, are kept
here: `LICENSE-MIT`, `LICENSE-APACHE`).

It exists for one change: **grids that run right to left** (Hebrew, Arabic). Upstream
`egui::Grid` always puts its first column on the left and aligns every cell's contents left, so in
a mirrored dialog a form's labels sat left of their fields and a table read backwards.

## What changed

- `src/grid.rs`: `Grid::right_to_left(bool)` (new, default `false`). With it the grid's columns run
  from the right edge of the available width leftwards, the first column rightmost, and each
  cell's contents align to its right. The column cursor still counts left to right, so column
  widths, row heights, striping and the first-frame sizing pass are measured exactly as before;
  only the rects the grid hands to the placer (`available_rect`, `next_cell`, the row background)
  are mirrored across the grid's available width, so widgets are both drawn and hit-tested there.
  The grid's own `Ui` is then a right-aligned column instead of a left-to-right row; the grid
  places the cells itself, so this only changes what cell contents inherit: rows inside a cell
  (`ui.horizontal`) run right to left too.

Without `right_to_left(true)` nothing changes. When egui gains right-to-left layouts upstream, drop
this directory and its `[patch.crates-io]` entry.
