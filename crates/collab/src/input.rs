//! A cell's `input`: what a person typed, as text that converts back exactly.
//!
//! - empty: no value (a cell that only has a style);
//! - `=…`: a formula (its canonical text after `=`);
//! - `'…`: text, when the text alone would read as something else (`'007`, `'=A1`, `'TRUE`);
//! - `TRUE` / `FALSE`: a logical value;
//! - `#N/A`, `#DIV/0!`… (exactly as Excel spells them): an error value;
//! - a number in Rust's shortest round-trip notation (`0.1`, `-3`, `1e-7`);
//! - anything else: text.
//!
//! Unlike typing in a cell, nothing depends on the locale or the cell's number format: `1/2`
//! is text here, never a date. [`decode`] accepts any finite number Rust's `f64` parser reads.

use gridcraft_core::{CellError, Value};
use gridcraft_model::Cell;

/// What an input holds.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Value(Value),
    /// A formula's text without `=`.
    Formula(String),
}

/// The input of a cell's content (its formula, or its constant value).
pub fn encode(cell: &Cell) -> String {
    if let Some(f) = &cell.formula {
        return format!("={}", f.text);
    }
    encode_value(&cell.value)
}

/// The input of a constant value.
pub fn encode_value(v: &Value) -> String {
    match v {
        Value::Empty => String::new(),
        Value::Number(n) => number(*n),
        Value::Text(t) => {
            if decode(t) == Input::Value(Value::Text(t.clone())) {
                t.to_string()
            } else {
                format!("'{t}")
            }
        }
        Value::Bool(b) => (if *b { "TRUE" } else { "FALSE" }).to_string(),
        Value::Error(e) => e.as_str().to_string(),
        // Arrays are formula results, never typed content.
        Value::Array(_) => v.display(),
    }
}

fn number(n: f64) -> String {
    if !n.is_finite() {
        return CellError::Num.as_str().to_string();
    }
    let n = if n == 0.0 { 0.0 } else { n };
    format!("{n}")
}

/// Reads an input back.
pub fn decode(s: &str) -> Input {
    if s.is_empty() {
        return Input::Value(Value::Empty);
    }
    if let Some(f) = s.strip_prefix('=') {
        return Input::Formula(f.to_string());
    }
    if let Some(t) = s.strip_prefix('\'') {
        return Input::Value(Value::Text(t.into()));
    }
    match s {
        "TRUE" => return Input::Value(Value::Bool(true)),
        "FALSE" => return Input::Value(Value::Bool(false)),
        _ => {}
    }
    if let Some(e) = CellError::parse(s).filter(|e| e.as_str() == s) {
        return Input::Value(Value::Error(e));
    }
    // Only plain decimal notation: Rust also reads `inf`, `NaN` and `infinity`, which are text.
    if s.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'+' | b'e' | b'E'))
        && let Ok(n) = s.parse::<f64>()
        && n.is_finite()
    {
        return Input::Value(Value::number(n));
    }
    Input::Value(Value::Text(s.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridcraft_model::Formula;

    fn round_trip(v: Value) {
        let cell = Cell::value(v.clone());
        assert_eq!(decode(&encode(&cell)), Input::Value(v.clone()), "{v:?} as {:?}", encode(&cell));
    }

    #[test]
    fn values_round_trip() {
        for v in [
            Value::Empty,
            Value::Number(0.1),
            Value::Number(-3.0),
            Value::Number(1e-7),
            Value::Number(123456789012345680000.0),
            Value::Bool(true),
            Value::Bool(false),
            Value::Error(CellError::NA),
            Value::Error(CellError::Div0),
            Value::Text("hello".into()),
            Value::Text("007".into()),
            Value::Text("=A1".into()),
            Value::Text("'quoted".into()),
            Value::Text("TRUE".into()),
            Value::Text("true".into()),
            Value::Text("#N/A".into()),
            Value::Text("1e5".into()),
            Value::Text("inf".into()),
            Value::Text("".into()),
            Value::Text(" 5".into()),
        ] {
            round_trip(v);
        }
    }

    #[test]
    fn formulas_keep_their_text() {
        let cell = Cell::formula(Formula::new("=SUM(A1:A3)"));
        assert_eq!(encode(&cell), "=SUM(A1:A3)");
        assert_eq!(decode("=SUM(A1:A3)"), Input::Formula("SUM(A1:A3)".into()));
    }

    #[test]
    fn nothing_depends_on_the_locale() {
        assert_eq!(decode("1/2"), Input::Value(Value::Text("1/2".into())));
        assert_eq!(decode("1,5"), Input::Value(Value::Text("1,5".into())));
        assert_eq!(decode("NaN"), Input::Value(Value::Text("NaN".into())));
        assert_eq!(decode("1e999"), Input::Value(Value::Text("1e999".into())));
    }
}
