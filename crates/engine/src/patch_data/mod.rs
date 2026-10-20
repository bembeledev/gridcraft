//! Adapter for the patcher's external `ck-validator` JSONL protocol.
//! No CK schema, type, reference, ID, or version-validation rules live in GridCraft.
mod snapshot;
#[cfg(not(target_arch = "wasm32"))]
mod worker;
pub use snapshot::{Overlay, overlays};

use gridcraft_core::CellRef;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

pub const PROTOCOL_VERSION: u32 = 1;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub executable: String,
    pub id_map_directory: String,
    pub tc_id_prefix: String,
    pub build_name: Option<String>,
    pub version_filtering: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            executable: "ck-validator".into(),
            id_map_directory: String::new(),
            tc_id_prefix: "TC_".into(),
            build_name: None,
            version_filtering: false,
        }
    }
}
impl Config {
    pub fn from_environment() -> Self {
        #[cfg(target_arch = "wasm32")]
        {
            Self::default()
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut config = Self::default();
            for (key, field) in [
                ("GRIDCRAFT_CK_VALIDATOR", &mut config.executable),
                ("GRIDCRAFT_CK_ID_MAP", &mut config.id_map_directory),
                ("GRIDCRAFT_CK_TC_ID_PREFIX", &mut config.tc_id_prefix),
            ] {
                if let Ok(value) = std::env::var(key) {
                    *field = value;
                }
            }
            config
        }
    }
    pub fn check(&self) -> Result<(), String> {
        if self.executable.trim().is_empty() {
            return Err("ck-validator 실행 파일을 지정하세요.".into());
        }
        if self.id_map_directory.trim().is_empty() {
            return Err("PatchDataIdMap.csv가 있는 ID 맵 폴더를 설정하세요.".into());
        }
        if !Path::new(&self.id_map_directory).is_absolute() {
            return Err("ID 맵 폴더는 절대 경로여야 합니다.".into());
        }
        if self.tc_id_prefix.trim().is_empty() {
            return Err("패처와 같은 TC ID 접두사를 설정하세요.".into());
        }
        Ok(())
    }
}
fn normalized_directory(path: &Path) -> PathBuf {
    #[cfg(not(target_arch = "wasm32"))]
    {
        path.canonicalize().or_else(|_| std::path::absolute(path)).unwrap_or_else(|_| path.to_path_buf())
    }
    #[cfg(target_arch = "wasm32")]
    {
        path.to_path_buf()
    }
}
pub fn project_root(root: &str) -> PathBuf {
    let path = Path::new(root);
    normalized_directory(if path.file_name().is_some_and(|n| n == "protobuf") { path.parent().unwrap_or(path) } else { path })
}
/// Transport filename restriction, not a validation rule. The worker decides which files/sheets to validate.
pub fn workbook_file(root: &str, path: &str) -> Option<String> {
    let path = Path::new(path);
    let name = path.file_name()?.to_str()?;
    (normalized_directory(path.parent()?) == project_root(root).join("protobuf") && name.ends_with("Data.xlsx") && !name.starts_with(['_', '~']))
        .then(|| name.to_string())
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Location {
    pub file: Option<String>,
    pub sheet: Option<String>,
    pub table: Option<String>,
    pub row: Option<u32>,
    pub column: Option<u32>,
}
impl Location {
    pub fn cell(&self) -> Option<CellRef> {
        let (row, col) = (self.row?, self.column?);
        (row < gridcraft_core::MAX_ROWS && col < gridcraft_core::MAX_COLS).then_some(CellRef::new(row, col))
    }
    pub fn path(&self, root: &str) -> Option<String> {
        let file = self.file.as_deref()?;
        if file.is_empty() || file.contains(['/', '\\', ':']) || file == "." || file == ".." {
            return None;
        }
        Some(project_root(root).join("protobuf").join(file).to_string_lossy().into_owned())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: String,
    pub message: String,
    pub location: Location,
    pub related: Vec<Location>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StageResult {
    pub stage: String,
    pub status: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub valid: bool,
    pub stages: Vec<StageResult>,
    pub diagnostics: Vec<Diagnostic>,
    pub total_diagnostics: usize,
    pub truncated: bool,
    pub simulated_new_ids: usize,
    pub filtered_rows: usize,
    pub effective_build_name: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Version {
    pub protocol_version: u32,
    pub package_version: String,
    pub source_revision: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Checked {
    pub validator: Version,
    pub report: Report,
}
#[derive(Serialize)]
pub struct Request {
    pub protocol_version: u32,
    pub request_id: String,
    pub revision: String,
    pub root: PathBuf,
    pub id_map_directory: PathBuf,
    pub options: Options,
    pub overlays: Vec<Overlay>,
}
#[derive(Serialize)]
pub struct Options {
    pub project_code: String,
    pub tc_id_prefix: String,
    pub build_name: Option<String>,
    pub version_filtering: bool,
}
impl Request {
    pub fn new(root: &str, config: &Config, revision: String, overlays: Vec<Overlay>) -> Result<Self, String> {
        config.check()?;
        let root = project_root(root);
        if !root.is_absolute() {
            return Err("데이터 폴더는 절대 경로여야 합니다.".into());
        }
        Ok(Self {
            protocol_version: PROTOCOL_VERSION,
            request_id: revision.clone(),
            revision,
            root,
            id_map_directory: config.id_map_directory.clone().into(),
            options: Options {
                project_code: "ck".into(),
                tc_id_prefix: config.tc_id_prefix.clone(),
                build_name: config.build_name.clone(),
                version_filtering: config.version_filtering,
            },
            overlays,
        })
    }
}
pub fn check(config: &Config, request: &Request, cancel: &AtomicBool, progress: impl FnMut(&str)) -> Result<Checked, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        worker::check(config, request, cancel, progress)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (config, request, cancel, progress);
        Err("ck-validator 검사는 데스크톱에서만 사용할 수 있습니다.".into())
    }
}
