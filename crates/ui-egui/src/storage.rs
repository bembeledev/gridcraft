//! Local persistent browser storage for workbooks and session recovery.
//!
//! On the web platform, workbooks can be persisted in the browser's Origin Private File
//! System (OPFS) with a fallback to LocalStorage. An autosave of unsaved work is maintained
//! continuously and restored after a crash, reload, or unexpected tab closure.
//! Desktop builds do not use this service.

use std::sync::Arc;

/// Metadata of a workbook stored in browser storage.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredWorkbook {
    /// File name (e.g. "budget.xlsx").
    pub name: String,
    /// Size in bytes.
    pub size: u64,
    /// Last modified timestamp (milliseconds since Unix epoch, 0.0 if unavailable).
    pub modified: f64,
}

/// Interface for persistent local storage provided by the web shell.
pub trait LocalStorageService: Send + Sync {
    /// List workbooks saved in browser storage.
    fn list(&self) -> Vec<StoredWorkbook>;
    /// Read the bytes of a stored workbook by name.
    fn read(&self, name: &str) -> Result<Vec<u8>, String>;
    /// Save a workbook into browser storage.
    fn save(&self, name: &str, bytes: &[u8]) -> Result<(), String>;
    /// Delete a workbook from browser storage.
    fn delete(&self, name: &str) -> Result<(), String>;
    /// Rename a stored workbook.
    fn rename(&self, from: &str, to: &str) -> Result<(), String>;
    /// Persist the autosave recovery copy of the active session.
    fn autosave(&self, name: &str, bytes: &[u8]);
    /// Clear the autosave recovery copy (when cleanly saved or closed).
    fn clear_recovery(&self);
    /// Take the recovered session if available on startup: returns (name, bytes).
    fn take_recovery(&self) -> Option<(String, Vec<u8>)>;
    /// Returns true if storage has finished initializing.
    fn is_ready(&self) -> bool {
        true
    }
    /// Returns true if OPFS is not available and storage is constrained to localStorage.
    fn is_local_storage_fallback(&self) -> bool {
        false
    }
    /// Take any pending storage error that occurred asynchronously.
    fn take_pending_error(&self) -> Option<String> {
        None
    }
}

pub type StorageHandle = Arc<dyn LocalStorageService>;
