//! Convert evaluated editor values to the worker's transport. Never interpret CK column types.
use super::workbook_file;
use gridcraft_core::{CellRef, Value};
use gridcraft_model::Workbook;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
const MAX_CELLS: usize = 2_000_000;
#[derive(Debug, Serialize)]
pub struct Overlay {
    pub file: String,
    pub sheets: Vec<Sheet>,
}
#[derive(Debug, Serialize)]
pub struct Sheet {
    pub name: String,
    pub rows: Vec<Vec<Cell>>,
}
#[derive(Debug, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Cell {
    Empty,
    String(Arc<str>),
    Float(f64),
    Bool(bool),
    DateTime(f64),
}

pub fn overlays(root: &str, books: &BTreeMap<String, Arc<Workbook>>, cancel: &AtomicBool) -> Result<Vec<Overlay>, String> {
    let mut count = 0usize;
    let mut result = Vec::new();
    for (path, wb) in books {
        let Some(file) = workbook_file(root, path) else { continue };
        let mut sheets = Vec::new();
        for sheet in &wb.sheets {
            // Ignored worksheets are still named in a whole-workbook replacement, but need no values.
            if sheet.name.starts_with('_') {
                sheets.push(Sheet { name: sheet.name.clone(), rows: Vec::new() });
                continue;
            }
            let mut end = None::<CellRef>;
            for pos in
                sheet.cells.iter().filter(|(_, c)| !c.value.is_empty() || c.formula.is_some()).map(|(c, _)| c).chain(sheet.spill.keys().copied())
            {
                end = Some(end.map_or(pos, |e| CellRef::new(e.row.max(pos.row), e.col.max(pos.col))));
            }
            let mut rows = Vec::new();
            if let Some(end) = end {
                let cells = (end.row as usize)
                    .checked_add(1)
                    .and_then(|h| (end.col as usize).checked_add(1).and_then(|w| h.checked_mul(w)))
                    .ok_or("미저장 시트 크기가 너무 큽니다.")?;
                count = count.checked_add(cells).ok_or("미저장 시트 크기가 너무 큽니다.")?;
                if count > MAX_CELLS {
                    return Err("패처의 미저장 전송 한도(2,000,000셀)를 넘었습니다. 저장 후 다시 검사하세요.".into());
                }
                for row in 0..=end.row {
                    if cancel.load(Ordering::Relaxed) {
                        return Err("검사가 취소되었습니다.".into());
                    }
                    let mut values = Vec::new();
                    for col in 0..=end.col {
                        let pos = CellRef::new(row, col);
                        let value = sheet.value(pos).scalar();
                        let cell = match value {
                            Value::Empty if sheet.cell(pos).is_some_and(|c| c.formula.is_some()) => {
                                return Err(format!("{file}/{}!{}: 수식 결과가 없어 검사할 수 없습니다.", sheet.name, pos.a1()));
                            }
                            Value::Empty => Cell::Empty,
                            Value::Text(s) => Cell::String(s),
                            Value::Bool(b) => Cell::Bool(b),
                            Value::Number(n) if n.is_finite() => {
                                let format = &wb.styles.get(sheet.style_id(pos)).num_fmt;
                                if crate::display::number_format(format.as_str()).is_date() {
                                    // Preserve the raw Excel serial, just as the patcher reads calamine DateTime::as_f64.
                                    Cell::DateTime(n)
                                } else {
                                    Cell::Float(n)
                                }
                            }
                            _ => return Err(format!("{file}/{}!{}: 계산 오류 또는 지원하지 않는 값이 있습니다.", sheet.name, pos.a1())),
                        };
                        values.push(cell);
                    }
                    rows.push(values);
                }
            }
            sheets.push(Sheet { name: sheet.name.clone(), rows });
        }
        result.push(Overlay { file, sheets });
    }
    Ok(result)
}
