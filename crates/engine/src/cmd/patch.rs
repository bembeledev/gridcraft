//! Synchronous CLI/MCP access to the same external validator used by the desktop.
use super::*;
use crate::patch_data::{self as data, Config, Request};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(query "patch.validate", "Validate CK Patch Data", ["Data", "Patch Data"], None,
        "{project: ck, root, config: {executable?, id_map_directory, tc_id_prefix?, build_name?, version_filtering?}} → ck-validator full-project report with unsaved overlays; desktop only", always, validate)]
}
fn validate(s: &mut Session, p: &Json) -> Result<Json> {
    if p.get("project").and_then(Json::as_str) != Some("ck") {
        return Err(bad("patch.validate", "explicit project: ck is required"));
    }
    let root = p.get("root").and_then(Json::as_str).ok_or_else(|| bad("patch.validate", "root is required"))?;
    let config: Config = if let Some(config) = p.get("config") {
        serde_json::from_value(config.clone()).map_err(|e| bad("patch.validate", e.to_string()))?
    } else {
        Config::from_environment()
    };
    let books: BTreeMap<_, _> =
        s.documents().iter().filter(|d| d.is_dirty()).filter_map(|d| d.path.as_ref().map(|p| (p.clone(), d.wb.clone()))).collect();
    let cancel = AtomicBool::new(false);
    let run = || -> std::result::Result<Json, String> {
        let overlays = data::overlays(root, &books, &cancel)?;
        let request = Request::new(root, &config, "engine-query".into(), overlays)?;
        let checked = data::check(&config, &request, &cancel, |_| {})?;
        serde_json::to_value(checked).map_err(|e| e.to_string())
    };
    run().map_err(EngineError::Other)
}
