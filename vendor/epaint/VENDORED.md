# Vendored epaint 0.36.2 (with bidirectional text)

This is [epaint](https://github.com/emilk/egui/tree/main/crates/epaint) 0.36.2, the text and
shape layer of egui, as published on crates.io, patched into the workspace by
`[patch.crates-io]` in the root `Cargo.toml`. egui and epaint are © Emil Ernerfeldt and the egui
contributors, dual-licensed MIT OR Apache-2.0 (their upstream license files, unchanged from egui 0.36.2, are kept
here: `LICENSE-MIT`, `LICENSE-APACHE`).

It exists for one change: **right-to-left text** (Hebrew, Arabic). Upstream epaint shapes and lays
out text strictly left to right, so Hebrew showed with its words in the wrong order and numbers
inside it reversed (`120` → `021`).

## What changed

- `src/text/bidi.rs` (new): the Unicode Bidirectional Algorithm (UAX #9, via the `unicode-bidi`
  crate) applied per laid-out row. Glyphs keep their logical order in `Row::glyphs` (cursor
  indices, selection and copying are unchanged); their x positions move to their visual places,
  clusters (a letter and its points) move as one, and paired punctuation is mirrored inside
  right-to-left runs. The paragraph direction follows the first strong character.
- `src/text/text_layout.rs`: shaping always runs in logical (left-to-right) order —
  `guess_segment_properties` used to pick right-to-left for Hebrew runs, reversing letters inside
  each font run only; the new `apply_bidi` pass runs after line breaking; horizontal alignment
  measures bidi rows by their visual extent.
- `src/text/text_layout_types.rs`: `Glyph::rtl` (new field); `Row::x_offset` puts the caret on
  the leading edge of right-to-left characters, and `Row::char_at` hit-tests rows whose glyph
  positions are not monotonic.
- `Cargo.toml`: depends on `unicode-bidi`.

Rows without right-to-left text take exactly the old code path. When egui gains bidi support
upstream, drop this directory and the `[patch.crates-io]` entry.
