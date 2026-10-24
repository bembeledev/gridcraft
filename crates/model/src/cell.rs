//! Cells.

use std::sync::Arc;

use gridcraft_core::{RangeRef, Value};
use gridcraft_formula::Expr;
use serde::{Deserialize, Serialize};

use crate::style::{Font, StyleId};

/// A parsed formula. `text` is the canonical formula text without the leading `=`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Formula {
    pub text: String,
    #[serde(skip)]
    pub expr: Option<Arc<Expr>>,
    /// Legacy array formula (Ctrl+Shift+Enter) covering this range; the formula lives in the
    /// top-left cell.
    pub array: Option<RangeRef>,
    /// Read from a file that marks this formula as a dynamic-array formula (cell metadata `cm`).
    /// The writer keeps the mark, so a formula Excel evaluates as an array is never saved as a
    /// legacy formula (which Excel evaluates with implicit intersection). Formulas entered or
    /// rewritten in GridCraft start without it, and the writer decides from their shape.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dynamic: bool,
}

impl Formula {
    /// Parses `text` (with or without `=`). Unparseable formulas keep their text and evaluate to
    /// `#NAME?`.
    pub fn new(text: &str) -> Formula {
        let body = text.strip_prefix('=').unwrap_or(text);
        match gridcraft_formula::parse(body) {
            Ok(e) => Formula { text: gridcraft_formula::print(&e), expr: Some(Arc::new(e)), array: None, dynamic: false },
            Err(_) => Formula { text: body.to_string(), expr: None, array: None, dynamic: false },
        }
    }
    pub fn from_expr(e: Expr) -> Formula {
        Formula { text: gridcraft_formula::print(&e), expr: Some(Arc::new(e)), array: None, dynamic: false }
    }
    /// The parsed expression, re-parsing after deserialization.
    pub fn expr(&self) -> Option<Expr> {
        self.expr.as_deref().cloned().or_else(|| gridcraft_formula::parse(&self.text).ok())
    }
    /// The parsed expression without copying it (re-parses after deserialization).
    pub fn expr_arc(&self) -> Option<Arc<Expr>> {
        self.expr.clone().or_else(|| gridcraft_formula::parse(&self.text).ok().map(Arc::new))
    }
}

/// A static picture stored as cell content, independent of floating drawing anchors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellPicture {
    /// Encoded PNG/JPEG bytes, base64 in JSON. Sheet entries share the payload through Arc.
    #[serde(with = "crate::b64")]
    pub data: Vec<u8>,
    pub mime: String,
    pub alt: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    /// Constant value, or the cached result of the formula.
    pub value: Value,
    pub formula: Option<Arc<Formula>>,
    pub style: StyleId,
    /// Formatted runs of a text constant (rich text). Kept with the cell, so they move with it;
    /// replacing the content drops them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rich: Option<Arc<RichText>>,
}

/// The formatted runs of a text cell: each run's length in characters and its font (`None`: the
/// cell's own font). They describe the text only while their lengths add up to it (see
/// [`RichText::spans`]); otherwise the cell shows in its own font.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RichText {
    pub runs: Vec<TextRun>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextRun {
    /// Characters (Unicode scalar values) in the run.
    pub len: u32,
    pub font: Option<Font>,
}

impl RichText {
    /// Most runs kept for one cell (more are merged into the last).
    pub const MAX_RUNS: usize = 1024;

    /// `text` cut into the runs with their fonts, when the runs describe exactly that text and at
    /// least one has a font of its own.
    pub fn spans<'a>(&self, text: &'a str) -> Option<Vec<(&'a str, Option<&Font>)>> {
        if self.runs.is_empty() || self.runs.len() > Self::MAX_RUNS || self.runs.iter().all(|r| r.font.is_none()) {
            return None;
        }
        let mut out = Vec::with_capacity(self.runs.len());
        let mut rest = text;
        for r in &self.runs {
            let n = r.len as usize;
            let cut = if n == 0 { 0 } else { rest.char_indices().nth(n).map_or(rest.len(), |(i, _)| i) };
            if n > 0 && rest[..cut].chars().count() != n {
                return None;
            }
            let (head, tail) = rest.split_at(cut);
            out.push((head, r.font.as_ref()));
            rest = tail;
        }
        rest.is_empty().then_some(out)
    }
}

impl Cell {
    pub fn value(v: Value) -> Cell {
        Cell { value: v, formula: None, style: StyleId::DEFAULT, rich: None }
    }
    pub fn formula(f: Formula) -> Cell {
        Cell { value: Value::Empty, formula: Some(Arc::new(f)), style: StyleId::DEFAULT, rich: None }
    }
    /// No value, no formula and the default style: storing it is pointless.
    pub fn is_blank(&self) -> bool {
        self.value.is_empty() && self.formula.is_none() && self.style == StyleId::DEFAULT
    }
    /// What the formula bar shows.
    pub fn input_text(&self) -> String {
        if let Some(f) = &self.formula {
            return format!("={}", f.text);
        }
        match &self.value {
            Value::Text(t) => {
                // Text that would parse as something else is shown with its apostrophe by Excel
                // only when entered that way; we show it plainly.
                t.to_string()
            }
            v => v.display(),
        }
    }
    /// What the formula bar shows when not editing: [`input_text`](Self::input_text), with a
    /// legacy array formula in braces (`{=A1:A2*2}`).
    pub fn bar_text(&self) -> String {
        match &self.formula {
            Some(f) if f.array.is_some() => format!("{{={}}}", f.text),
            _ => self.input_text(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red() -> Font {
        Font { bold: true, color: crate::Color::Rgb(0xFF0000), ..Font::default() }
    }

    #[test]
    fn rich_text_spans() {
        let r = RichText { runs: vec![TextRun { len: 4, font: None }, TextRun { len: 7, font: Some(red()) }] };
        // Lengths count characters, not bytes ("ö" is two bytes).
        let spans = r.spans("Note gelöst").expect("the runs describe the text");
        assert_eq!(spans.iter().map(|(t, f)| (*t, f.is_some())).collect::<Vec<_>>(), vec![("Note", false), (" gelöst", true)]);
        // Runs that don't add up to the text, or have no font of their own, describe nothing.
        assert!(r.spans("Notes").is_none());
        assert!(r.spans("Note gelöst!!").is_none());
        assert!(RichText { runs: vec![TextRun { len: 3, font: None }] }.spans("abc").is_none());
        assert!(RichText::default().spans("").is_none());
        // Empty runs are harmless.
        let r = RichText { runs: vec![TextRun { len: 0, font: Some(red()) }, TextRun { len: 3, font: Some(red()) }] };
        assert_eq!(r.spans("abc").map(|s| s.len()), Some(2));
        // A hostile length doesn't overflow.
        let r = RichText { runs: vec![TextRun { len: u32::MAX, font: Some(red()) }] };
        assert!(r.spans("abc").is_none());
    }
}
