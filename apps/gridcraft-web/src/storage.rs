//! Local persistent browser storage for workbooks and session recovery.
//!
//! Stores workbooks in the Origin Private File System (OPFS) when available, falling back
//! to `localStorage` on insecure origins (e.g. plain HTTP on LAN) or older browsers.
//! Keeps continuous autosave recovery in `recovery.xlsx` + `recovery.json` (or `gridcraft.recovery.*`),
//! enabling recovery after unexpected page reloads or browser restarts.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use gridcraft_ui_egui::storage::{LocalStorageService, StoredWorkbook};
use wasm_bindgen::{JsCast as _, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{FileSystemDirectoryHandle, FileSystemFileHandle, FileSystemGetFileOptions, FileSystemWritableFileStream};

const RECOVERY_NAME: &str = "recovery.xlsx";
const RECOVERY_META_NAME: &str = "recovery.json";

const LOCAL_PREFIX: &str = "gridcraft.file.";
const LOCAL_RECOVERY_KEY: &str = "gridcraft.recovery.data";
const LOCAL_RECOVERY_META_KEY: &str = "gridcraft.recovery.meta";

fn err(e: JsValue) -> String {
    e.as_string().or_else(|| js_sys::Reflect::get(&e, &"message".into()).ok().and_then(|m| m.as_string())).unwrap_or_else(|| format!("{e:?}"))
}

fn local() -> Result<web_sys::Storage, String> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten()).ok_or_else(|| "localStorage is not available".to_string())
}

/// Text as `t<utf8>`, anything else as `x<hex>`.
fn encode(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(t) => format!("t{t}"),
        Err(_) => {
            let mut s = String::with_capacity(bytes.len() * 2 + 1);
            s.push('x');
            for b in bytes {
                s.push_str(&format!("{b:02x}"));
            }
            s
        }
    }
}

fn decode(s: &str) -> Vec<u8> {
    if let Some(t) = s.strip_prefix('t') {
        return t.as_bytes().to_vec();
    }
    let h = s.strip_prefix('x').unwrap_or("");
    (0..h.len() / 2).filter_map(|i| h.get(2 * i..2 * i + 2).and_then(|x| u8::from_str_radix(x, 16).ok())).collect()
}

fn storage_manager() -> Option<web_sys::StorageManager> {
    let nav = web_sys::window()?.navigator();
    let sm = js_sys::Reflect::get(&nav, &"storage".into()).ok().filter(|v| v.is_object())?;
    Some(sm.unchecked_into())
}

fn has_method(sm: &web_sys::StorageManager, method: &str) -> bool {
    js_sys::Reflect::get(sm, &method.into()).is_ok_and(|f| f.is_function())
}

async fn opfs_root() -> Result<FileSystemDirectoryHandle, String> {
    let sm = storage_manager()
        .filter(|sm| has_method(sm, "getDirectory"))
        .ok_or_else(|| "OPFS getDirectory is not supported (needs https or localhost)".to_string())?;
    let dir = JsFuture::from(sm.get_directory()).await.map_err(err)?;
    dir.dyn_into::<FileSystemDirectoryHandle>().map_err(|_| "not a FileSystemDirectoryHandle".to_string())
}

async fn read_file_handle(h: &FileSystemFileHandle) -> Result<(Vec<u8>, f64), String> {
    let file: web_sys::File = JsFuture::from(h.get_file()).await.map_err(err)?.dyn_into().map_err(|_| "not a File".to_string())?;
    let buf = JsFuture::from(file.array_buffer()).await.map_err(err)?;
    let uint8 = js_sys::Uint8Array::new(&buf);
    Ok((uint8.to_vec(), file.last_modified()))
}

async fn write_opfs(name: &str, bytes: &[u8]) -> Result<(), String> {
    let dir = opfs_root().await?;
    let opts = FileSystemGetFileOptions::new();
    opts.set_create(true);
    let h_val = JsFuture::from(dir.get_file_handle_with_options(name, &opts)).await.map_err(err)?;
    let h: FileSystemFileHandle = h_val.dyn_into().map_err(|_| "not a FileSystemFileHandle".to_string())?;
    let stream_val = JsFuture::from(h.create_writable()).await.map_err(err)?;
    let stream: FileSystemWritableFileStream = stream_val.dyn_into().map_err(|_| "not a FileSystemWritableFileStream".to_string())?;
    let _ = JsFuture::from(stream.write_with_u8_array(bytes).map_err(err)?).await.map_err(err)?;
    let _ = JsFuture::from(stream.close()).await.map_err(err)?;
    Ok(())
}

async fn remove_opfs(name: &str) -> Result<(), String> {
    let dir = opfs_root().await?;
    let _ = JsFuture::from(dir.remove_entry(name)).await.map_err(err)?;
    Ok(())
}

enum RecoveryOp {
    Save { bytes: Vec<u8>, meta: String },
    Clear,
}

#[derive(Default)]
struct RecoveryQueue {
    queue: VecDeque<RecoveryOp>,
    active: bool,
}

pub struct WebStorage {
    entries: Arc<Mutex<Vec<StoredWorkbook>>>,
    cache: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    recovery: Arc<Mutex<Option<(String, Vec<u8>)>>>,
    pending_error: Arc<Mutex<Option<String>>>,
    fallback: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    recovery_queue: Arc<Mutex<RecoveryQueue>>,
}

impl WebStorage {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: Arc::new(Mutex::new(Vec::new())),
            cache: Arc::new(Mutex::new(HashMap::new())),
            recovery: Arc::new(Mutex::new(None)),
            pending_error: Arc::new(Mutex::new(None)),
            fallback: Arc::new(AtomicBool::new(false)),
            ready: Arc::new(AtomicBool::new(false)),
            recovery_queue: Arc::new(Mutex::new(RecoveryQueue::default())),
        })
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    fn enqueue_recovery(&self, op: RecoveryOp) {
        let mut state = self.recovery_queue.lock().unwrap_or_else(PoisonError::into_inner);
        state.queue.push_back(op);
        if !state.active {
            state.active = true;
            let queue = self.recovery_queue.clone();
            let fallback_flag = self.fallback.clone();
            let pending_error = self.pending_error.clone();
            spawn_local(async move {
                loop {
                    let next_op = {
                        let mut state = queue.lock().unwrap_or_else(PoisonError::into_inner);
                        match state.queue.pop_front() {
                            Some(op) => op,
                            None => {
                                state.active = false;
                                break;
                            }
                        }
                    };
                    let fallback = fallback_flag.load(Ordering::Relaxed);
                    match next_op {
                        RecoveryOp::Save { bytes, meta } => {
                            if fallback {
                                match local() {
                                    Ok(st) => {
                                        if let Err(e) = st.set_item(LOCAL_RECOVERY_KEY, &encode(&bytes)) {
                                            *pending_error.lock().unwrap_or_else(PoisonError::into_inner) =
                                                Some(format!("Autosave failed: {}", err(e)));
                                        } else if let Err(e) = st.set_item(LOCAL_RECOVERY_META_KEY, &meta) {
                                            *pending_error.lock().unwrap_or_else(PoisonError::into_inner) =
                                                Some(format!("Autosave failed: {}", err(e)));
                                        }
                                    }
                                    Err(e) => {
                                        *pending_error.lock().unwrap_or_else(PoisonError::into_inner) = Some(format!("Autosave failed: {e}"));
                                    }
                                }
                            } else if let Err(e) = write_opfs(RECOVERY_NAME, &bytes).await {
                                *pending_error.lock().unwrap_or_else(PoisonError::into_inner) = Some(format!("Autosave failed: {e}"));
                            } else if let Err(e) = write_opfs(RECOVERY_META_NAME, meta.as_bytes()).await {
                                *pending_error.lock().unwrap_or_else(PoisonError::into_inner) = Some(format!("Autosave failed: {e}"));
                            }
                        }
                        RecoveryOp::Clear => {
                            if fallback {
                                if let Ok(st) = local() {
                                    let _ = st.remove_item(LOCAL_RECOVERY_KEY);
                                    let _ = st.remove_item(LOCAL_RECOVERY_META_KEY);
                                }
                            } else {
                                let _ = remove_opfs(RECOVERY_NAME).await;
                                let _ = remove_opfs(RECOVERY_META_NAME).await;
                            }
                        }
                    }
                }
            });
        }
    }
}

impl LocalStorageService for WebStorage {
    fn list(&self) -> Vec<StoredWorkbook> {
        let mut list = self.entries.lock().unwrap_or_else(PoisonError::into_inner).clone();
        list.sort_by_key(|a| a.name.to_lowercase());
        list
    }

    fn read(&self, name: &str) -> Result<Vec<u8>, String> {
        let cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(bytes) = cache.get(name) {
            return Ok(bytes.clone());
        }
        if self.fallback.load(Ordering::Relaxed) {
            let st = local()?;
            let val = st.get_item(&format!("{LOCAL_PREFIX}{name}")).map_err(err)?.ok_or("file not found")?;
            return Ok(decode(&val));
        }
        Err("file not found in cache".into())
    }

    fn save(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let name = name.trim().to_string();
        if name.is_empty() || name.contains('/') || name.contains('\\') {
            return Err("invalid file name: slashes not allowed".into());
        }
        if name.eq_ignore_ascii_case(RECOVERY_NAME) || name.eq_ignore_ascii_case(RECOVERY_META_NAME) {
            return Err("file name is reserved for recovery".into());
        }
        let now = js_sys::Date::now();
        {
            let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            cache.insert(name.clone(), bytes.to_vec());
        }
        {
            let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
            entries.retain(|e| e.name != name);
            entries.push(StoredWorkbook { name: name.clone(), size: bytes.len() as u64, modified: now });
        }
        if self.fallback.load(Ordering::Relaxed) {
            let st = local()?;
            if let Err(e) = st.set_item(&format!("{LOCAL_PREFIX}{name}"), &encode(bytes)) {
                let err_msg = err(e);
                self.cache.lock().unwrap_or_else(PoisonError::into_inner).remove(&name);
                self.entries.lock().unwrap_or_else(PoisonError::into_inner).retain(|e| e.name != name);
                return Err(format!("failed saving to localStorage: {err_msg}"));
            }
            return Ok(());
        }
        let bytes_vec = bytes.to_vec();
        let name_clone = name.clone();
        let cache_clone = self.cache.clone();
        let entries_clone = self.entries.clone();
        let error_clone = self.pending_error.clone();
        spawn_local(async move {
            if let Err(e) = write_opfs(&name_clone, &bytes_vec).await {
                log::error!("failed writing OPFS file {name_clone}: {e}");
                cache_clone.lock().unwrap_or_else(PoisonError::into_inner).remove(&name_clone);
                entries_clone.lock().unwrap_or_else(PoisonError::into_inner).retain(|entry| entry.name != name_clone);
                *error_clone.lock().unwrap_or_else(PoisonError::into_inner) = Some(format!("Failed to save {name_clone}: {e}"));
            }
        });
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        let name = name.to_string();
        {
            let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            cache.remove(&name);
        }
        {
            let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
            entries.retain(|e| e.name != name);
        }
        let fallback = self.fallback.load(Ordering::Relaxed);
        let name_clone = name.clone();
        spawn_local(async move {
            if fallback {
                if let Ok(st) = local() {
                    let _ = st.remove_item(&format!("{LOCAL_PREFIX}{name_clone}"));
                }
            } else if let Err(e) = remove_opfs(&name_clone).await {
                log::warn!("failed removing OPFS file {name_clone}: {e}");
            }
        });
        Ok(())
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        let bytes = self.read(from)?;
        self.save(to, &bytes)?;
        self.delete(from)?;
        Ok(())
    }

    fn autosave(&self, name: &str, bytes: &[u8]) {
        let name = name.to_string();
        let bytes_vec = bytes.to_vec();
        *self.recovery.lock().unwrap_or_else(PoisonError::into_inner) = Some((name.clone(), bytes_vec.clone()));
        let meta = serde_json::json!({
            "name": name,
            "modified": js_sys::Date::now(),
        })
        .to_string();
        self.enqueue_recovery(RecoveryOp::Save { bytes: bytes_vec, meta });
    }

    fn clear_recovery(&self) {
        *self.recovery.lock().unwrap_or_else(PoisonError::into_inner) = None;
        self.enqueue_recovery(RecoveryOp::Clear);
    }

    fn take_recovery(&self) -> Option<(String, Vec<u8>)> {
        self.recovery.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    fn take_pending_error(&self) -> Option<String> {
        self.pending_error.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    fn is_local_storage_fallback(&self) -> bool {
        self.fallback.load(Ordering::Relaxed)
    }
}

/// Ask browser for persistent storage quota and preload all stored files into memory.
pub fn init(storage: Arc<WebStorage>, ctx: egui::Context) {
    if let Some(sm) = storage_manager().filter(|sm| has_method(sm, "persist"))
        && let Ok(persist_prom) = sm.persist()
    {
        spawn_local(async move {
            let _ = JsFuture::from(persist_prom).await;
        });
    }

    let storage_clone = storage.clone();
    spawn_local(async move {
        match opfs_root().await {
            Ok(dir) => {
                let mut entries = Vec::new();
                let mut cache = HashMap::new();
                let mut recovery_bytes: Option<Vec<u8>> = None;
                let mut recovery_title = "Untitled.xlsx".to_string();

                let iter_val: JsValue = dir.values().into();
                if let Ok(next_fn) =
                    js_sys::Reflect::get(&iter_val, &"next".into()).map_err(err).and_then(|f| f.dyn_into::<js_sys::Function>().map_err(err))
                {
                    for _ in 0..10_000 {
                        let step_val = match next_fn.call0(&iter_val).map_err(err) {
                            Ok(val) => val,
                            Err(_) => break,
                        };
                        let step = match JsFuture::from(js_sys::Promise::resolve(&step_val)).await {
                            Ok(val) => val,
                            Err(_) => break,
                        };
                        if js_sys::Reflect::get(&step, &"done".into()).ok().and_then(|d| d.as_bool()).unwrap_or(true) {
                            break;
                        }
                        let Ok(h) =
                            js_sys::Reflect::get(&step, &"value".into()).map_err(err).and_then(|v| v.dyn_into::<FileSystemFileHandle>().map_err(err))
                        else {
                            continue;
                        };
                        let name = h.name();
                        if let Ok((bytes, modified)) = read_file_handle(&h).await {
                            if name == RECOVERY_NAME {
                                recovery_bytes = Some(bytes);
                            } else if name == RECOVERY_META_NAME {
                                if let Ok(meta) = serde_json::from_slice::<serde_json::Value>(&bytes)
                                    && let Some(n) = meta.get("name").and_then(|v| v.as_str())
                                {
                                    recovery_title = n.to_string();
                                }
                            } else {
                                entries.push(StoredWorkbook { name: name.clone(), size: bytes.len() as u64, modified });
                                cache.insert(name, bytes);
                            }
                        }
                    }
                }

                *storage_clone.entries.lock().unwrap_or_else(PoisonError::into_inner) = entries;
                *storage_clone.cache.lock().unwrap_or_else(PoisonError::into_inner) = cache;
                if let Some(rec) = recovery_bytes {
                    *storage_clone.recovery.lock().unwrap_or_else(PoisonError::into_inner) = Some((recovery_title, rec));
                }
            }
            Err(e) => {
                log::warn!("OPFS unavailable ({e}); falling back to localStorage");
                storage_clone.fallback.store(true, Ordering::Relaxed);
                if let Ok(st) = local() {
                    let mut entries = Vec::new();
                    let mut cache = HashMap::new();
                    let len = st.length().unwrap_or(0).min(10_000);
                    for i in 0..len {
                        if let Ok(Some(k)) = st.key(i)
                            && let Some(name) = k.strip_prefix(LOCAL_PREFIX)
                            && let Ok(Some(val)) = st.get_item(&k)
                        {
                            let bytes = decode(&val);
                            entries.push(StoredWorkbook { name: name.to_string(), size: bytes.len() as u64, modified: 0.0 });
                            cache.insert(name.to_string(), bytes);
                        }
                    }
                    if let Ok(Some(rec_val)) = st.get_item(LOCAL_RECOVERY_KEY) {
                        let mut name = "Untitled.xlsx".to_string();
                        if let Ok(Some(meta_val)) = st.get_item(LOCAL_RECOVERY_META_KEY)
                            && let Ok(meta) = serde_json::from_str::<serde_json::Value>(&meta_val)
                            && let Some(n) = meta.get("name").and_then(|v| v.as_str())
                        {
                            name = n.to_string();
                        }
                        *storage_clone.recovery.lock().unwrap_or_else(PoisonError::into_inner) = Some((name, decode(&rec_val)));
                    }
                    *storage_clone.entries.lock().unwrap_or_else(PoisonError::into_inner) = entries;
                    *storage_clone.cache.lock().unwrap_or_else(PoisonError::into_inner) = cache;
                }
            }
        }
        storage_clone.ready.store(true, Ordering::Relaxed);
        ctx.request_repaint();
    });
}
