//! Cell values on the wire.
//!
//! | Cell value | JSON |
//! |---|---|
//! | empty | `null` |
//! | number | number |
//! | text | string |
//! | logical | `true` / `false` |
//! | error | `{"error": "#N/A"}` |
//! | array (a range) | rows of values: `[[1, 2], [3, 4]]` |
//!
//! A plugin's result may also be a flat list, read as one row. Ragged rows are padded with
//! `#N/A`, as Excel pads arrays.
//!
//! In a result, `null` is an empty value, which a cell shows as 0 (as Excel shows a VBA function
//! that returns `Empty`); `{"empty": true}` (protocol 2.2) is a blank the cell shows blank, when
//! the formula is the call itself ([`from_json_result`]).

use std::sync::Arc;

use gridcraft_core::{Array, CellError, Value};
use serde_json::{Value as Json, json};

/// Most cells sent to or accepted from a plugin in one call.
pub const MAX_CELLS: usize = 1_000_000;
/// Excel's cell text limit.
const MAX_TEXT: usize = 32_767;

pub fn to_json(v: &Value) -> Json {
    match v {
        Value::Empty => Json::Null,
        Value::Number(n) => serde_json::Number::from_f64(*n).map_or(Json::Null, Json::Number),
        Value::Text(t) => Json::String(t.to_string()),
        Value::Bool(b) => Json::Bool(*b),
        Value::Error(e) => json!({"error": e.as_str()}),
        Value::Array(a) => {
            let cols = a.cols.max(1);
            Json::Array(a.data.chunks(cols).map(|row| Json::Array(row.iter().map(to_json).collect())).collect())
        }
    }
}

/// A plugin's result as a cell value; anything malformed is `#VALUE!`.
pub fn from_json(j: &Json) -> Value {
    match j {
        Json::Array(rows) => array(rows),
        j => scalar(j),
    }
}

/// A function's result, and whether it holds explicit blanks (`{"empty": true}`, protocol 2.2).
/// Explicit blanks are empty values; with them, the result's `null`s become 0, as the cell would
/// have shown them, so the host can keep every empty value of this result blank.
pub fn from_json_result(j: &Json) -> (Value, bool) {
    let blanks = match j {
        Json::Array(rows) => rows.iter().any(|r| match r {
            Json::Array(cells) => cells.iter().any(is_blank),
            other => is_blank(other),
        }),
        other => is_blank(other),
    };
    if !blanks {
        return (from_json(j), false);
    }
    let v = match from_json(j) {
        Value::Array(a) if a.data.iter().any(|x| matches!(x, Value::Empty)) => {
            // Which empty values were nulls: those turn into 0.
            let flat: Vec<&Json> = match j {
                Json::Array(rows) if rows.iter().any(Json::is_array) => {
                    let width = a.cols;
                    let mut out = Vec::with_capacity(a.data.len());
                    for r in rows {
                        let row: Vec<&Json> = match r {
                            Json::Array(cells) => cells.iter().collect(),
                            single => vec![single],
                        };
                        let n = row.len();
                        out.extend(row);
                        out.extend(std::iter::repeat_n(&Json::Bool(false), width.saturating_sub(n)));
                    }
                    out
                }
                Json::Array(rows) => rows.iter().collect(),
                _ => Vec::new(),
            };
            let data = a
                .data
                .iter()
                .enumerate()
                .map(|(i, x)| match (x, flat.get(i)) {
                    (Value::Empty, Some(Json::Null)) => Value::Number(0.0),
                    (x, _) => x.clone(),
                })
                .collect();
            Array::new(a.rows, a.cols, data).map_or(Value::Array(a), Value::from)
        }
        v => v,
    };
    (v, true)
}

/// `{"empty": true}`.
fn is_blank(j: &Json) -> bool {
    j.as_object().is_some_and(|o| o.get("empty") == Some(&Json::Bool(true)) && !o.contains_key("error"))
}

fn scalar(j: &Json) -> Value {
    match j {
        Json::Null => Value::Empty,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => n.as_f64().map_or(Value::Error(CellError::Num), Value::number),
        Json::String(s) => Value::Text(Arc::from(truncate(s))),
        Json::Object(o) => match o.get("error").and_then(Json::as_str) {
            Some(e) => Value::Error(CellError::parse(e).unwrap_or(CellError::Value)),
            None if is_blank(j) => Value::Empty,
            None => Value::Error(CellError::Value),
        },
        Json::Array(_) => Value::Error(CellError::Value),
    }
}

fn truncate(s: &str) -> &str {
    match s.char_indices().nth(MAX_TEXT) {
        Some((i, _)) => s.get(..i).unwrap_or(s),
        None => s,
    }
}

fn array(rows: &[Json]) -> Value {
    if rows.is_empty() {
        return Value::Error(CellError::Calc);
    }
    // A flat list is one row.
    if !rows.iter().any(Json::is_array) {
        if rows.len() > MAX_CELLS {
            return Value::Error(CellError::Num);
        }
        return Value::from(Array::row(rows.iter().map(scalar).collect()));
    }
    let width = rows.iter().map(|r| r.as_array().map_or(1, Vec::len)).max().unwrap_or(1);
    if width == 0 {
        return Value::Error(CellError::Calc);
    }
    if rows.len().checked_mul(width).is_none_or(|n| n > MAX_CELLS) {
        return Value::Error(CellError::Num);
    }
    let mut data = Vec::with_capacity(rows.len() * width);
    for r in rows {
        let row: &[Json] = match r {
            Json::Array(cells) => cells,
            single => std::slice::from_ref(single),
        };
        data.extend(row.iter().map(scalar));
        data.extend(std::iter::repeat_n(Value::Error(CellError::NA), width - row.len()));
    }
    match Array::new(rows.len(), width, data) {
        Some(a) => Value::from(a),
        None => Value::Error(CellError::Value),
    }
}

/// Cells in a list of arguments (an array counts each of its cells).
pub fn cell_count(args: &[Value]) -> usize {
    args.iter().map(|a| if let Value::Array(x) = a { x.data.len() } else { 1 }).fold(0usize, usize::saturating_add)
}
