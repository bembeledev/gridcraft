//! `audit`: report hidden information in a `.xlsx` package before sharing.
//!
//! Reads the raw OPC bytes with `gridcraft-audit`, prints the JSON report to
//! stdout, and exits 0 when clean, 1 when findings are present (CI-gate
//! usable), or 2 when the audit could not run.

use std::fs;
use std::process::ExitCode;

use super::Args;

/// Run `gridcraft-cli audit <file>`.
pub fn main(args: &[String]) -> ExitCode {
    let parsed = match Args::parse(args, &[], &[]) {
        Ok(a) => a,
        Err(e) => return fail(&format!("audit: {e}")),
    };
    let path = match parsed.pos(0, "file") {
        Ok(p) => p,
        Err(e) => return fail(&format!("audit: {e}")),
    };
    if let Err(e) = parsed.no_extra(1) {
        return fail(&format!("audit: {e}"));
    }
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => return fail(&format!("audit: cannot read {path}: {e}")),
    };
    let findings = match gridcraft_audit::audit_bytes(&bytes) {
        Ok(f) => f,
        Err(e) => return fail(&format!("audit: {e}")),
    };
    let report = serde_json::json!({ "file": path, "findings": findings, "count": findings.len() });
    match serde_json::to_string_pretty(&report) {
        Ok(text) => outln!("{text}"),
        Err(e) => return fail(&format!("audit: cannot render report: {e}")),
    }
    if findings.is_empty() { ExitCode::SUCCESS } else { ExitCode::from(1) }
}

fn fail(message: &str) -> ExitCode {
    eprintln!("gridcraft-cli: {message}");
    ExitCode::from(2)
}
