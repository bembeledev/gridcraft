//! Explicit opt-in, read-only authoring diagnostics for the active workbook.
use super::*;
use serde_json::json;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(query "patch.validate", "Validate CK Patch Data", ["Data", "Patch Data"], None,
        "{project: ck, version?: name} → local workbook diagnostics; cross-file references are provisional", has_doc, validate)]
}

fn validate(s: &mut Session, p: &Json) -> Result<Json> {
    if p.get("project").and_then(Json::as_str) != Some("ck") {
        return Err(bad("patch.validate", "explicit project: ck is required"));
    }
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let sheets = crate::patch_data::extract(d.path.as_deref().unwrap_or("Unsaved"), &d.wb).map_err(EngineError::Other)?;
    let (report, catalog) = crate::patch_data::validate::analyze(&sheets.iter().collect::<Vec<_>>(), p.get("version").and_then(Json::as_str), false);
    Ok(json!({"report": report, "catalog": catalog, "scope": "active workbook; not a publication check"}))
}
