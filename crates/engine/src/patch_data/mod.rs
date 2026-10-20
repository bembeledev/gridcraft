//! Read-only CK patch-data assistance. These checks explain authoring rules; publication still
//! belongs to the patcher, including ID-map allocation, protobuf generation and remote checks.
pub mod schema;
pub mod validate;

use gridcraft_core::{CellRef, Value};
use gridcraft_model::{Sheet, Workbook};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_CELLS: usize = 1_000_000;
pub const MAX_COLUMNS: u32 = 2048;
pub const MAX_ROWS: u32 = 200_000;
pub const MAX_ISSUES: usize = 2000;

/// The patcher accepts XLSX packages, not CSV content renamed with an .xlsx suffix.
pub fn read_xlsx(bytes: &[u8]) -> Result<(Workbook, Vec<String>), String> {
    gridcraft_xlsx::read_xlsx_values(bytes, |name| !name.starts_with('_')).map(|(wb, report)| (wb, report.warnings)).map_err(|e| e.to_string())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Column {
    pub index: u32,
    pub flags: String,
    pub typ: String,
    pub name: String,
}
impl Column {
    pub fn has(&self, name: &str) -> bool {
        schema::has(&schema::flags(&self.flags), name)
    }
    pub fn argument(&self, name: &str) -> Option<String> {
        schema::argument(&schema::flags(&self.flags), name).map(str::to_string)
    }
    pub fn included(&self) -> bool {
        !self.flags.trim().is_empty()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DataSheet {
    pub file: String,
    pub name: String,
    pub columns: Vec<Column>,
    pub rows: BTreeMap<u32, Vec<Value>>,
    pub transposed: bool,
}
impl DataSheet {
    pub fn table(&self) -> &str {
        self.name.split('_').next().unwrap_or_default()
    }
    pub fn ignored(&self) -> bool {
        self.name.starts_with('_')
    }
    pub fn cell(&self, row: u32, column: u32) -> CellRef {
        if self.transposed { CellRef::new(column, row) } else { CellRef::new(row, column) }
    }
    pub fn column(&self, physical: CellRef) -> Option<&Column> {
        let index = if self.transposed { physical.row } else { physical.col };
        self.columns.iter().find(|c| c.index == index)
    }
    pub fn field(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name && c.included())
    }
    pub fn row_text(&self, values: &[Value], name: &str) -> String {
        self.field(name).and_then(|i| values.get(i)).map(Value::display).unwrap_or_default()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Context {
    /// Canonical workspace root. No profile is active unless the UI or caller explicitly sets it.
    pub root: Option<String>,
    pub files: BTreeMap<String, Vec<DataSheet>>,
    pub build_name: String,
    pub complete: bool,
    pub notes: Vec<String>,
}

pub fn extract(file: &str, wb: &Workbook) -> Result<Vec<DataSheet>, String> {
    let mut budget = MAX_CELLS;
    wb.sheets.iter().map(|sheet| extract_sheet(file, sheet, &mut budget)).collect()
}

fn extract_sheet(file: &str, sh: &Sheet, budget: &mut usize) -> Result<DataSheet, String> {
    let transposed = sh.name.ends_with("_Transpose");
    let mut max_row = 2;
    let mut max_col = 0;
    for (cell, value) in sh.cells.iter() {
        if value.value.is_empty() && value.formula.is_none() {
            continue;
        }
        let (row, col) = if transposed { (cell.col, cell.row) } else { (cell.row, cell.col) };
        max_row = max_row.max(row);
        max_col = max_col.max(col);
    }
    if max_row >= MAX_ROWS || max_col >= MAX_COLUMNS {
        return Err(format!("{file}/{}: 규칙 검사 범위 한도({MAX_ROWS}행, {MAX_COLUMNS}열)를 넘었습니다.", sh.name));
    }
    let cells = (max_row as usize).saturating_add(1).saturating_mul((max_col as usize).saturating_add(1));
    if cells > *budget {
        return Err(format!("{file}: 규칙 인덱스의 셀 수 한도({MAX_CELLS})를 넘었습니다."));
    }
    *budget = budget.saturating_sub(cells);
    let cell = |r, c| if transposed { CellRef::new(c, r) } else { CellRef::new(r, c) };
    let columns = (0..=max_col)
        .map(|index| Column {
            index,
            flags: sh.value(cell(0, index)).display(),
            typ: sh.value(cell(1, index)).display(),
            name: sh.value(cell(2, index)).display(),
        })
        .collect::<Vec<_>>();
    let rows = (3..=max_row).map(|r| (r, (0..=max_col).map(|c| sh.value(cell(r, c))).collect())).collect();
    Ok(DataSheet { file: file.into(), name: sh.name.clone(), columns, rows, transposed })
}

/// Visit the complete logical table in small chunks, retaining physical addresses. Unlike the
/// synchronous workbook query, the background project index has no cumulative cell budget.
/// Ignored sheets and columns never expand into dense rectangles. Empty intermediate rows
/// remain present, since Key/NotBlank and group constraints apply to them too.
pub fn visit_chunks(file: &str, wb: &Workbook, mut visit: impl FnMut(DataSheet) -> Result<(), String>) -> Result<(), String> {
    for sh in &wb.sheets {
        if sh.name.starts_with('_') {
            continue;
        }
        let transposed = sh.name.ends_with("_Transpose");
        let cell = |r, c| if transposed { CellRef::new(c, r) } else { CellRef::new(r, c) };
        let mut max_row = 2;
        let mut fields = std::collections::BTreeSet::new();
        for (pos, value) in sh.cells.iter() {
            if value.value.is_empty() && value.formula.is_none() {
                continue;
            }
            let (row, col) = if transposed { (pos.col, pos.row) } else { (pos.row, pos.col) };
            max_row = max_row.max(row);
            if row == 0 && !value.value.display().trim().is_empty() {
                fields.insert(col);
            }
        }
        let columns = fields
            .into_iter()
            .map(|index| Column {
                index,
                flags: sh.value(cell(0, index)).display(),
                typ: sh.value(cell(1, index)).display(),
                name: sh.value(cell(2, index)).display(),
            })
            .collect::<Vec<_>>();
        let mut chunk = DataSheet { file: file.into(), name: sh.name.clone(), columns, rows: BTreeMap::new(), transposed };
        // A wide transposed table can contain more than 32K fields; keep just one row then.
        let rows_per_chunk = (32768 / chunk.columns.len().max(1)).clamp(1, 256);
        // Send headers even when there are no data rows, so schema checks still run.
        visit(chunk.clone())?;
        for row in 3..=max_row {
            let values = chunk.columns.iter().map(|col| sh.value(cell(row, col.index))).collect();
            chunk.rows.insert(row, values);
            if chunk.rows.len() >= rows_per_chunk {
                visit(chunk.clone())?;
                chunk.rows.clear();
            }
        }
        if !chunk.rows.is_empty() {
            visit(chunk)?;
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Issue {
    pub severity: String,
    pub code: String,
    pub file: String,
    pub sheet: String,
    pub cell: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Report {
    pub issues: Vec<Issue>,
    pub total_issues: usize,
    pub included_rows: usize,
    pub excluded_rows: usize,
    pub truncated: bool,
    pub complete_context: bool,
}
impl Report {
    pub fn push(&mut self, sh: &DataSheet, row: u32, col: u32, severity: &str, code: &str, message: impl Into<String>) {
        self.total_issues = self.total_issues.saturating_add(1);
        if self.issues.len() >= MAX_ISSUES {
            self.truncated = true;
            return;
        }
        self.issues.push(Issue {
            severity: severity.into(),
            code: code.into(),
            file: sh.file.clone(),
            sheet: sh.name.clone(),
            cell: sh.cell(row, col).a1(),
            message: message.into(),
        });
    }
}
