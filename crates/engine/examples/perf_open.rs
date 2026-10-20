//! Read-only open benchmark: cargo run -p gridcraft-engine --example perf_open -- book.xlsx
//! Reports decoding/row fitting separately from full recalculation; never saves the workbook.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::disallowed_methods)] // Desktop benchmark clock and file access.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::hash::{Hash, Hasher};
    use std::time::Instant;

    let path = std::env::args().nth(1).ok_or("workbook path required")?;
    let start = Instant::now();
    let bytes = gridcraft_engine::io::read_file(&path)?;
    let read = start.elapsed().as_secs_f64();
    let decode = Instant::now();
    let (wb, warnings) = gridcraft_engine::io::open_bytes(&path, &bytes)?;
    let decode = decode.elapsed().as_secs_f64();
    eprintln!("decoded in {decode:.3}s; recalculating");
    let calc = Instant::now();
    let doc = gridcraft_engine::DocState::new(wb, Some(path), "benchmark".into());
    let calc = calc.elapsed().as_secs_f64();
    let opened = start.elapsed().as_secs_f64();
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    let mut formulas = 0usize;
    let mut errors = 0usize;
    for sh in &doc.wb.sheets {
        sh.name.hash(&mut hash);
        for (c, cell) in sh.cells.iter() {
            c.hash(&mut hash);
            serde_json::to_string(cell)?.hash(&mut hash);
            formulas += usize::from(cell.formula.is_some());
            errors += usize::from(cell.value.is_error());
        }
    }
    println!(
        "{}",
        serde_json::json!({
            "readSeconds":read, "decodeSeconds":decode, "recalcSeconds":calc, "openSeconds":opened,
            "sheets":doc.wb.sheets.len(), "formulas":formulas, "errorCells":errors,
            "warnings":warnings.len(), "cellFingerprint":format!("{:016x}", hash.finish())
        })
    );
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}
