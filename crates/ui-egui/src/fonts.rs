//! The fonts installed on the machine, so the font list can offer them and cells can render with
//! them (#4).
//!
//! egui has no font discovery of its own and panics when asked for a family that was never bound
//! to font data. So we do both halves: scan the platform's font folders for family names (a read
//! of each font file's `name` table — damaged or unreadable files yield nothing, never a panic),
//! and bind the files a workbook actually uses into the families
//! [`theme::font_definitions`](crate::theme::font_definitions) assembles.
//!
//! Native only: on wasm there are no system fonts, every function here reports "nothing", and the
//! font list is the standard [`crate::ribbon::FONTS`] set.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::OnceLock;

/// A font file is skipped by the scan past this size: a catalog only needs each face's name, and a
/// damaged or enormous file must not turn startup into a long read.
#[cfg(not(target_arch = "wasm32"))]
const MAX_FONT_FILE: u64 = 64 << 20;

/// One face of one installed font file.
#[derive(Clone, Debug)]
pub struct Face {
    /// Typographic family name, e.g. `"Segoe UI"`.
    pub family: String,
    /// The file the face lives in.
    pub path: PathBuf,
    /// Face index within the file (collections hold several, e.g. a family's Regular and Bold).
    pub index: u32,
    /// Weight implied by the face's style name (400 = regular).
    pub weight: f32,
    /// Whether the style name reads as italic or oblique.
    pub italic: bool,
    /// Position in the browser queryLocalFonts() array, when discovered on web.
    pub js_index: u32,
    /// PostScript name reported by queryLocalFonts() on web, or empty.
    pub postscript_name: String,
}

impl Face {
    /// Creates a web-discovered face (bytes fetched separately on demand).
    pub fn new_web(family: String, js_index: u32, style: &str, postscript_name: String) -> Self {
        Self { family, path: PathBuf::new(), index: 0, weight: style_weight(style), italic: style_italic(style), js_index, postscript_name }
    }
}

/// The font folders of the platform (the system's and the user's). Empty on wasm.
pub fn system_font_dirs() -> Vec<PathBuf> {
    #[cfg(target_arch = "wasm32")]
    {
        Vec::new()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut dirs: Vec<PathBuf> = Vec::new();
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
        if cfg!(target_os = "macos") {
            dirs.extend(["/System/Library/Fonts", "/System/Library/Fonts/Supplemental", "/Library/Fonts"].map(Into::into));
            if let Some(h) = &home {
                dirs.push(h.join("Library/Fonts"));
            }
        } else if cfg!(windows) {
            let root = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Windows"));
            dirs.push(root.join("Fonts"));
            if let Some(l) = std::env::var_os("LOCALAPPDATA") {
                dirs.push(PathBuf::from(l).join("Microsoft\\Windows\\Fonts"));
            }
        } else {
            dirs.extend(["/usr/share/fonts", "/usr/local/share/fonts", "/usr/share/fonts/TTF"].map(Into::into));
            if let Some(h) = &home {
                dirs.push(h.join(".fonts"));
                dirs.push(h.join(".local/share/fonts"));
            }
        }
        dirs
    }
}

/// Weight implied by a style name (`"Bold"` → 700, `"Light"` → 300).
pub fn style_weight(style: &str) -> f32 {
    let s: String = style.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect();
    const TABLE: &[(&str, f32)] = &[
        ("hairline", 100.0),
        ("thin", 100.0),
        ("extralight", 200.0),
        ("ultralight", 200.0),
        ("light", 300.0),
        ("medium", 500.0),
        ("demibold", 600.0),
        ("semibold", 600.0),
        ("extrabold", 800.0),
        ("ultrabold", 800.0),
        ("bold", 700.0),
        ("black", 900.0),
        ("heavy", 900.0),
    ];
    TABLE.iter().find(|(k, _)| s.contains(k)).map(|(_, w)| *w).unwrap_or(400.0)
}

/// Whether a style name reads as italic or oblique.
pub fn style_italic(style: &str) -> bool {
    let s: String = style.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect();
    s.contains("italic") || s.contains("oblique")
}

/// Every face in `data` as `(index, family, style)`. Damaged data yields nothing, never a panic.
#[cfg(not(target_arch = "wasm32"))]
fn file_faces(data: &[u8]) -> Vec<(u32, String, String)> {
    use skrifa::raw::FileRef;
    use skrifa::{MetadataProvider, string::StringId};
    let count = match FileRef::new(data) {
        Ok(FileRef::Font(_)) => 1,
        Ok(FileRef::Collection(c)) => c.len(),
        Err(_) => return Vec::new(),
    };
    let name = |f: &skrifa::FontRef<'_>, ids: &[StringId]| -> Option<String> {
        ids.iter().find_map(|id| f.localized_strings(*id).english_or_first().map(|s| s.to_string()).filter(|s| !s.is_empty()))
    };
    (0..count)
        .filter_map(|i| {
            let f = skrifa::FontRef::from_index(data, i).ok()?;
            let family = name(&f, &[StringId::TYPOGRAPHIC_FAMILY_NAME, StringId::FAMILY_NAME])?;
            let style = name(&f, &[StringId::TYPOGRAPHIC_SUBFAMILY_NAME, StringId::SUBFAMILY_NAME]).unwrap_or_else(|| "Regular".into());
            Some((i, family, style))
        })
        .collect()
}

/// Catalog every face under `dirs`, recursing into subfolders. A folder reached twice (through a
/// symlink) is read once, so a link back to a parent ends the walk.
#[cfg(not(target_arch = "wasm32"))]
pub fn scan_dirs(dirs: &[PathBuf]) -> Vec<Face> {
    let mut out: Vec<Face> = Vec::new();
    let mut stack: Vec<PathBuf> = dirs.to_vec();
    let mut seen_dirs: HashSet<PathBuf> = HashSet::new();
    while let Some(dir) = stack.pop() {
        let key = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
        if !seen_dirs.insert(key) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let is_font =
                path.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc" | "otc"));
            if !is_font || std::fs::metadata(&path).map(|m| m.len() > MAX_FONT_FILE).unwrap_or(true) {
                continue;
            }
            let Ok(data) = std::fs::read(&path) else { continue };
            for (index, family, style) in file_faces(&data) {
                out.push(Face {
                    family,
                    path: path.clone(),
                    index,
                    weight: style_weight(&style),
                    italic: style_italic(&style),
                    js_index: 0,
                    postscript_name: String::new(),
                });
            }
        }
    }
    out
}

/// On WebAssembly, returns an empty list because the local file system is not accessible.
#[cfg(target_arch = "wasm32")]
pub fn scan_dirs(_dirs: &[PathBuf]) -> Vec<Face> {
    Vec::new()
}

static CATALOG: OnceLock<Vec<Face>> = OnceLock::new();

/// The catalog of installed faces, scanned on first use. Blocks until the scan finishes.
pub fn catalog() -> &'static [Face] {
    let cat = CATALOG.get_or_init(|| scan_dirs(&system_font_dirs()));
    ensure_catalog_families(cat);
    cat
}

/// The catalog if the scan has finished, else `None` — never blocks, for use while painting.
fn catalog_now() -> Option<&'static [Face]> {
    CATALOG.get().map(Vec::as_slice)
}

/// Start cataloging the installed fonts on a background thread, so the first frame doesn't wait for
/// it. A no-op on wasm and after the scan has started.
pub fn start_scan() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        #[cfg(not(target_arch = "wasm32"))]
        if CATALOG.get().is_none() {
            // A failed spawn leaves the scan to the first blocking lookup.
            let _ = std::thread::Builder::new().name("font-scan".into()).spawn(|| {
                catalog();
            });
        }
    });
}

/// Whether the background scan has finished.
pub fn scan_done() -> bool {
    CATALOG.get().is_some()
}

/// The faces of one installed family, regular-weight first (the order roles pick from).
pub fn faces_of(family: &str) -> Vec<Face> {
    let Some(cat) = catalog_now() else { return Vec::new() };
    let mut v: Vec<Face> = cat.iter().filter(|f| f.family.eq_ignore_ascii_case(family)).cloned().collect();
    v.sort_by(|a, b| a.weight.total_cmp(&b.weight).then(a.italic.cmp(&b.italic)).then(a.index.cmp(&b.index)));
    v
}

/// The subset of `names` that is installed, sorted and deduped. These are the families whose files
/// get bound for a workbook.
pub fn installed_among(names: &[String]) -> Vec<String> {
    let Some(cat) = catalog_now() else { return Vec::new() };
    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for wanted in names {
        if !seen.insert(wanted.to_lowercase()) {
            continue;
        }
        if let Some(f) = cat.iter().find(|f| f.family.eq_ignore_ascii_case(wanted)) {
            out.push(f.family.clone());
        }
    }
    out.sort_by_key(|a| a.to_lowercase());
    out
}

static DEFAULT_FAMILIES: OnceLock<Vec<String>> = OnceLock::new();
static DEFAULT_COMBO_ITEMS: OnceLock<Vec<(&'static str, String)>> = OnceLock::new();
static CATALOG_FAMILIES: OnceLock<Vec<String>> = OnceLock::new();
static CATALOG_COMBO_ITEMS: OnceLock<Vec<(&'static str, String)>> = OnceLock::new();

fn default_families() -> &'static [String] {
    DEFAULT_FAMILIES.get_or_init(|| crate::ribbon::FONTS.iter().map(|s| s.to_string()).collect())
}

fn default_combo_items() -> &'static [(&'static str, String)] {
    DEFAULT_COMBO_ITEMS.get_or_init(|| default_families().iter().map(|f| (f.as_str(), f.clone())).collect())
}

fn ensure_catalog_families(cat: &[Face]) {
    CATALOG_FAMILIES.get_or_init(|| {
        let mut out: Vec<String> = crate::ribbon::FONTS.iter().map(|s| s.to_string()).collect();
        let mut seen: HashSet<String> = out.iter().map(|s| s.to_lowercase()).collect();
        let mut installed: Vec<String> = cat.iter().map(|f| f.family.clone()).collect();
        installed.sort_by_key(|a| a.to_lowercase());
        installed.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        for f in installed {
            if seen.insert(f.to_lowercase()) {
                out.push(f);
            }
        }
        out
    });
    CATALOG_COMBO_ITEMS.get_or_init(|| {
        let fams = CATALOG_FAMILIES.get().map(|v| v.as_slice()).unwrap_or(&[]);
        fams.iter().map(|f| (f.as_str(), f.clone())).collect()
    });
}

/// The font list a combo box shows: the standard [`crate::ribbon::FONTS`] names first (what Excel
/// offers, and what we substitute), then every other installed family A–Z. Never blocks: until the
/// background scan finishes it returns just the standard names.
pub fn families() -> &'static [String] {
    if let Some(cat) = catalog_now() {
        ensure_catalog_families(cat);
        CATALOG_FAMILIES.get().map(|v| v.as_slice()).unwrap_or_else(default_families)
    } else {
        default_families()
    }
}

/// The pre-built combo items `(&str, String)` corresponding to [`families`], cached for UI dropdowns.
pub fn family_combo_items() -> &'static [(&'static str, String)] {
    if let Some(cat) = catalog_now() {
        ensure_catalog_families(cat);
        CATALOG_COMBO_ITEMS.get().map(|v| v.as_slice()).unwrap_or_else(default_combo_items)
    } else {
        default_combo_items()
    }
}

/// The egui family name for one role of one family: `fnt:{family}:{b|n}{i|n}`.
pub fn role_key(family: &str, bold: bool, italic: bool) -> String {
    format!("fnt:{family}:{}{}", if bold { 'b' } else { 'n' }, if italic { 'i' } else { 'n' })
}

/// The face to use for a role, or `None` when the family has no faces. Missing bold/italic faces
/// fall back to the family's regular face, so every bound role has data behind it.
fn pick_face(faces: &[Face], bold: bool, italic: bool) -> Option<&Face> {
    let regular = || faces.iter().find(|f| !f.italic && (f.weight - 400.0).abs() < 1.0);
    let exact = |wanted_bold: bool, wanted_italic: bool| {
        faces.iter().find(|f| f.italic == wanted_italic && if wanted_bold { f.weight >= 600.0 } else { f.weight < 600.0 })
    };
    (match (bold, italic) {
        (true, true) => exact(true, true).or_else(|| exact(true, false)).or_else(|| exact(false, true)),
        (true, false) => exact(true, false),
        (false, true) => exact(false, true),
        (false, false) => None,
    })
    .or_else(regular)
    .or_else(|| faces.first())
}

/// A stable key for a face's font data, unique per file/index within the process.
fn face_key(_family: &str, face: &Face) -> String {
    let fam = face.family.to_ascii_lowercase();
    if face.path.as_os_str().is_empty() {
        format!("fnt-src:{fam}:web:{}", face.js_index)
    } else {
        let file = face.path.file_name().and_then(|s| s.to_str()).unwrap_or("font");
        format!("fnt-src:{fam}:{file}:{}", face.index)
    }
}

static BYTES_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Current generation of stored in-memory face bytes. Increments every time new font bytes are stored.
pub fn bytes_generation() -> u64 {
    BYTES_GENERATION.load(std::sync::atomic::Ordering::Acquire)
}

/// Bumps the in-memory face bytes generation counter.
pub fn bump_bytes_generation() {
    BYTES_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Release);
}

/// Validate whether font file bytes parse successfully with skrifa at the given face index.
pub fn is_valid_font_bytes(bytes: &[u8], index: u32) -> bool {
    skrifa::FontRef::from_index(bytes, index).is_ok()
}

static IN_FLIGHT_FACES: OnceLock<std::sync::RwLock<HashSet<String>>> = OnceLock::new();

/// Global registry tracking faces currently being fetched over network or IPC.
fn in_flight_faces() -> &'static std::sync::RwLock<HashSet<String>> {
    IN_FLIGHT_FACES.get_or_init(|| std::sync::RwLock::new(HashSet::new()))
}

/// Marks a face as being fetched. Returns false if it was already in flight.
pub fn mark_face_in_flight(family: &str, face: &Face) -> bool {
    let key = face_key(family, face);
    in_flight_faces().write().unwrap_or_else(|e| e.into_inner()).insert(key)
}

/// Clears in-flight status for a face.
pub fn clear_face_in_flight(family: &str, face: &Face) {
    let key = face_key(family, face);
    in_flight_faces().write().unwrap_or_else(|e| e.into_inner()).remove(&key);
}

/// Whether a face is currently being fetched.
pub fn is_face_in_flight(family: &str, face: &Face) -> bool {
    let key = face_key(family, face);
    in_flight_faces().read().unwrap_or_else(|e| e.into_inner()).contains(&key)
}

/// Cached in-memory font file bytes and resolved collection face index.
#[derive(Clone, Copy)]
struct InMemoryFace {
    buffer: &'static [u8],
    index: u32,
}

static IN_MEMORY_FACES: OnceLock<std::sync::RwLock<std::collections::HashMap<String, InMemoryFace>>> = OnceLock::new();

/// Global registry caching in-memory font file bytes.
fn in_memory_faces() -> &'static std::sync::RwLock<std::collections::HashMap<String, InMemoryFace>> {
    IN_MEMORY_FACES.get_or_init(|| std::sync::RwLock::new(std::collections::HashMap::new()))
}

/// Stores an existing process-lifetime font buffer for an in-memory face after verifying it parses.
pub fn store_face_buffer(family: &str, face: &Face, buffer: &'static [u8]) -> bool {
    if !is_valid_font_bytes(buffer, face.index) {
        log::warn!("dropping unparseable font bytes for family {family} (face index {})", face.index);
        return false;
    }
    let key = face_key(family, face);
    in_memory_faces().write().unwrap_or_else(|e| e.into_inner()).insert(key, InMemoryFace { buffer, index: face.index });
    bump_bytes_generation();
    true
}

/// Stores font file bytes for an in-memory or web-loaded face after verifying they parse.
/// Reuses any existing process-lifetime buffer with identical contents to avoid duplicate allocations.
pub fn store_face_bytes(family: &str, face: &Face, bytes: Vec<u8>) -> bool {
    if !is_valid_font_bytes(&bytes, face.index) {
        log::warn!("dropping unparseable font bytes for family {family} (face index {})", face.index);
        return false;
    }
    let key = face_key(family, face);
    let mut map = in_memory_faces().write().unwrap_or_else(|e| e.into_inner());
    let buffer = map
        .values()
        .find_map(|im| if im.buffer == bytes.as_slice() { Some(im.buffer) } else { None })
        .unwrap_or_else(|| Box::leak(bytes.into_boxed_slice()));
    map.insert(key, InMemoryFace { buffer, index: face.index });
    drop(map);
    bump_bytes_generation();
    true
}

/// Returns the process-lifetime byte buffer for a cached face, if present.
pub fn get_face_buffer(family: &str, face: &Face) -> Option<&'static [u8]> {
    let key = face_key(family, face);
    in_memory_faces().read().unwrap_or_else(|e| e.into_inner()).get(&key).map(|im| im.buffer)
}

/// Whether bytes have already been fetched and cached for this face.
pub fn has_face_bytes(family: &str, face: &Face) -> bool {
    let key = face_key(family, face);
    in_memory_faces().read().unwrap_or_else(|e| e.into_inner()).contains_key(&key)
}

/// Initializes the installed face catalog (used on wasm after Font Access API query).
pub fn init_catalog(faces: Vec<Face>) -> bool {
    let ok = CATALOG.set(faces).is_ok();
    if let Some(cat) = catalog_now().filter(|_| ok) {
        ensure_catalog_families(cat);
    }
    ok
}

/// If `bytes` is a font collection (TTC/OTC) and `postscript_name` is non-empty, finds the face
/// index matching OpenType Name ID 6 (`POSTSCRIPT_NAME`). Returns `0` if not a collection or not found.
pub fn resolve_collection_face_index(bytes: &[u8], postscript_name: &str) -> u32 {
    use skrifa::raw::FileRef;
    use skrifa::{MetadataProvider, string::StringId};

    if postscript_name.is_empty() {
        return 0;
    }
    match FileRef::new(bytes) {
        Ok(FileRef::Collection(collection)) => (0..collection.len())
            .find(|&i| {
                skrifa::FontRef::from_index(bytes, i)
                    .ok()
                    .and_then(|f| f.localized_strings(StringId::POSTSCRIPT_NAME).english_or_first())
                    .is_some_and(|name| name.to_string().eq_ignore_ascii_case(postscript_name))
            })
            .unwrap_or(0),
        _ => 0,
    }
}

/// Whether `bytes` represents an OpenType or TrueType font collection (TTC/OTC).
pub fn is_collection_font_bytes(bytes: &[u8]) -> bool {
    matches!(skrifa::raw::FileRef::new(bytes), Ok(skrifa::raw::FileRef::Collection(_)))
}

/// Stores collection bytes for any sibling faces in `faces` whose `postscript_name` matches
/// a face in the collection `buffer`. All matching sibling faces share `buffer` without cloning.
pub fn store_collection_sibling_faces(faces: &[Face], buffer: &'static [u8]) {
    use skrifa::raw::FileRef;
    use skrifa::{MetadataProvider, string::StringId};

    let Ok(FileRef::Collection(collection)) = FileRef::new(buffer) else {
        return;
    };

    let collection_names: Vec<(u32, String)> = (0..collection.len())
        .filter_map(|i| {
            let f = skrifa::FontRef::from_index(buffer, i).ok()?;
            let ps = f.localized_strings(StringId::POSTSCRIPT_NAME).english_or_first()?.to_string();
            Some((i, ps))
        })
        .collect();

    for face in faces {
        if face.postscript_name.is_empty() || has_face_bytes(&face.family, face) {
            continue;
        }
        if let Some((idx, _)) = collection_names.iter().find(|(_, ps)| ps.eq_ignore_ascii_case(&face.postscript_name)) {
            let mut resolved = face.clone();
            resolved.index = *idx;
            if is_valid_font_bytes(buffer, resolved.index) {
                store_face_buffer(&face.family, &resolved, buffer);
            }
            clear_face_in_flight(&face.family, face);
        }
    }
}

/// Returns the deduplicated list of faces for `family` that [`bind_faces`] selects across its four roles.
pub fn needed_faces(family: &str) -> Vec<Face> {
    let faces = faces_of(family);
    if faces.is_empty() {
        return Vec::new();
    }
    let mut selected: Vec<Face> = Vec::new();
    for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
        if let Some(primary) = pick_face(&faces, bold, italic).filter(|primary| {
            !selected.iter().any(|f| f.family.eq_ignore_ascii_case(&primary.family) && f.index == primary.index && f.js_index == primary.js_index)
        }) {
            selected.push(primary.clone());
        }
        if let Some(regular) = pick_face(&faces, false, false).filter(|regular| {
            !selected.iter().any(|f| f.family.eq_ignore_ascii_case(&regular.family) && f.index == regular.index && f.js_index == regular.js_index)
        }) {
            selected.push(regular.clone());
        }
    }
    selected
}

/// Bind one installed family's faces into `fonts`, one egui family per cell role, and return the
/// role keys that are now bound. A role with no exact face chains to the family's regular face, so
/// no role is ever left pointing at missing data (egui panics on that). A family with no faces
/// binds nothing.
///
/// Each role's chain ends with egui's default proportional fonts, so glyphs the installed font
/// lacks (emoji, a stray script) still render.
pub fn bind_faces(fonts: &mut egui::FontDefinitions, family: &str, faces: &[Face]) -> Vec<String> {
    if faces.is_empty() {
        return Vec::new();
    }
    let base: Vec<String> = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
    let mut bound: Vec<String> = Vec::new();
    for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
        let Some(primary) = pick_face(faces, bold, italic) else { continue };
        let mut chain: Vec<String> = Vec::new();
        for face in [Some(primary), pick_face(faces, false, false)] {
            let Some(face) = face else { continue };
            let key = face_key(family, face);
            if chain.contains(&key) {
                continue;
            }
            if !fonts.font_data.contains_key(&key) {
                let entry: Option<egui::FontData> = match std::fs::read(&face.path) {
                    Ok(bytes) => {
                        let mut fd = egui::FontData::from_owned(bytes);
                        fd.index = face.index;
                        Some(fd)
                    }
                    Err(_) => in_memory_faces().read().unwrap_or_else(|e| e.into_inner()).get(&key).map(|im| {
                        let mut fd = egui::FontData::from_static(im.buffer);
                        fd.index = im.index;
                        fd
                    }),
                };
                let Some(fd) = entry else { continue };
                fonts.font_data.insert(key.clone(), std::sync::Arc::new(fd));
            }
            chain.push(key);
        }
        if chain.is_empty() {
            continue;
        }
        chain.extend(base.iter().cloned());
        let role = role_key(family, bold, italic);
        fonts.families.insert(egui::FontFamily::Name(role.clone().into()), chain);
        bound.push(role);
    }
    bound
}

/// Bind every installed face of `family` (see [`bind_faces`]).
pub fn bind_family(fonts: &mut egui::FontDefinitions, family: &str) -> Vec<String> {
    bind_faces(fonts, family, &faces_of(family))
}

/// The role families currently bound to font data. `cell_family` consults this, so it never names a
/// family egui hasn't been given.
static BOUND: OnceLock<std::sync::RwLock<HashSet<String>>> = OnceLock::new();

fn bound() -> &'static std::sync::RwLock<HashSet<String>> {
    BOUND.get_or_init(|| std::sync::RwLock::new(HashSet::new()))
}

/// Record the role families a freshly built [`egui::FontDefinitions`] bound.
pub fn set_bound(keys: impl IntoIterator<Item = String>) {
    *bound().write().unwrap_or_else(|e| e.into_inner()) = keys.into_iter().collect();
}

/// Is this exact role family bound to font data?
pub fn is_bound(role: &str) -> bool {
    bound().read().unwrap_or_else(|e| e.into_inner()).contains(role)
}

/// Serializes tests that mutate the process-wide bound-family registry.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_names_map_to_weight_and_slant() {
        assert_eq!(style_weight("Bold"), 700.0);
        assert_eq!(style_weight("SemiBold Italic"), 600.0);
        assert_eq!(style_weight("Regular"), 400.0);
        assert_eq!(style_weight("Light Oblique"), 300.0);
        assert!(style_italic("Bold Italic") && style_italic("Oblique"));
        assert!(!style_italic("Bold"));
        // The weight table is checked in order, so "extralight" wins over "light".
        assert_eq!(style_weight("ExtraLight"), 200.0);
    }

    #[test]
    fn role_keys_are_stable_and_distinct() {
        assert_eq!(role_key("Segoe UI", false, false), "fnt:Segoe UI:nn");
        assert_eq!(role_key("Segoe UI", true, false), "fnt:Segoe UI:bn");
        assert_eq!(role_key("Segoe UI", false, true), "fnt:Segoe UI:ni");
        assert_eq!(role_key("Segoe UI", true, true), "fnt:Segoe UI:bi");
    }

    #[test]
    fn nothing_is_bound_until_font_definitions_says_so() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // A role key is only safe to name after font_definitions bound it.
        let k = role_key("Some Installed Font", true, false);
        assert!(!is_bound(&k));
        set_bound([k.clone()]);
        assert!(is_bound(&k));
        set_bound([]);
        assert!(!is_bound(&k));
    }

    #[test]
    fn damaged_or_non_font_files_contribute_nothing() {
        assert!(file_faces(b"ttcf\x00\x01\x00\x00\xff\xff\xff\xff").is_empty());
        assert!(file_faces(b"not a font at all").is_empty());
        assert!(file_faces(&[]).is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_scan_catalogs_fonts_and_reads_each_folder_once() {
        let dir = std::env::temp_dir().join(format!("gridcraft-fonts-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Sub")).expect("temp dir");
        std::fs::write(dir.join("Ubuntu-Light.ttf"), epaint_default_fonts::UBUNTU_LIGHT).expect("write");
        std::fs::write(dir.join("damaged.ttf"), b"ttcf\x00\x01\x00\x00\xff\xff\xff\xff").expect("write");
        std::fs::write(dir.join("readme.txt"), b"not a font").expect("write");
        // The same folder listed twice (once through `Sub/..`) is read once.
        let faces = scan_dirs(&[dir.clone(), dir.join("Sub/..")]);
        let families: Vec<&str> = faces.iter().map(|f| f.family.as_str()).collect();
        assert_eq!(families, ["Ubuntu"], "one face, family read from the name table");
        assert_eq!(faces[0].weight, 300.0, "“Light” reads as weight 300");
        assert!(!faces[0].italic);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_back_to_a_parent_folder_ends_the_scan() {
        let dir = std::env::temp_dir().join(format!("gridcraft-fonts-loop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Sub")).expect("temp dir");
        std::fs::write(dir.join("Sub/Ubuntu-Light.ttf"), epaint_default_fonts::UBUNTU_LIGHT).expect("write");
        std::os::unix::fs::symlink(&dir, dir.join("Sub/loop")).expect("symlink");
        assert_eq!(scan_dirs(std::slice::from_ref(&dir)).len(), 1, "the loop doesn't multiply faces");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Tests that binding an installed family binds all four cell roles and skips missing faces.
    #[test]
    fn binding_a_family_binds_all_four_roles_and_never_stores_missing_data() {
        let dir = std::env::temp_dir().join(format!("gridcraft-fonts-bind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("Ubuntu-Light.ttf");
        std::fs::write(&path, epaint_default_fonts::UBUNTU_LIGHT).expect("write");
        let face = Face { family: "Ubuntu".into(), path, index: 0, weight: 300.0, italic: false, js_index: 0, postscript_name: String::new() };

        let mut fonts = egui::FontDefinitions::default();
        let bound = bind_faces(&mut fonts, "Ubuntu", std::slice::from_ref(&face));
        assert_eq!(bound.len(), 4, "regular, bold, italic and bold-italic roles");
        for key in &bound {
            let chain = fonts.families.get(&egui::FontFamily::Name(key.as_str().into())).expect("role family");
            assert!(chain.iter().all(|k| fonts.font_data.contains_key(k)), "every chained key has data");
        }
        // A family with no faces binds nothing and never touches the map.
        assert!(bind_faces(&mut fonts, "No Such Font", &[]).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Tests that binding an in-memory face binds all cell roles into egui definitions.
    #[test]
    fn binding_in_memory_face_binds_roles() {
        let face = Face::new_web("TestWebFont".into(), 0, "Regular", "TestWebFont-Regular".into());
        assert!(store_face_bytes("TestWebFont", &face, epaint_default_fonts::UBUNTU_LIGHT.to_vec()));
        assert!(has_face_bytes("TestWebFont", &face));

        let mut fonts = egui::FontDefinitions::default();
        let bound = bind_faces(&mut fonts, "TestWebFont", std::slice::from_ref(&face));
        assert_eq!(bound.len(), 4);
        for key in &bound {
            let chain = fonts.families.get(&egui::FontFamily::Name(key.as_str().into())).expect("role family");
            assert!(chain.iter().all(|k| fonts.font_data.contains_key(k)), "every chained key has data");
        }
    }

    /// Tests that invalid font bytes fail validation and are never stored into memory.
    #[test]
    fn invalid_font_bytes_are_rejected_and_never_stored() {
        let face = Face::new_web("BadFont".into(), 42, "Regular", "BadFont-Regular".into());
        let bad_bytes = vec![0xDE, 0xAD, 0xBE, 0xEF];
        assert!(!is_valid_font_bytes(&bad_bytes, face.index));
        let stored = store_face_bytes("BadFont", &face, bad_bytes);
        assert!(!stored);
        assert!(!has_face_bytes("BadFont", &face));

        let mut fonts = egui::FontDefinitions::default();
        let bound = bind_faces(&mut fonts, "BadFont", std::slice::from_ref(&face));
        assert!(bound.is_empty(), "invalid face binds nothing");
    }

    /// Tests in-flight tracking flags and byte generation counter increments.
    #[test]
    fn in_flight_tracking_and_generation_counter() {
        let face = Face::new_web("AsyncFont".into(), 1, "Bold", "AsyncFont-Bold".into());
        assert!(!is_face_in_flight("AsyncFont", &face));

        assert!(mark_face_in_flight("AsyncFont", &face));
        assert!(is_face_in_flight("AsyncFont", &face));
        assert!(!mark_face_in_flight("AsyncFont", &face));

        clear_face_in_flight("AsyncFont", &face);
        assert!(!is_face_in_flight("AsyncFont", &face));

        let gen_before = bytes_generation();
        assert!(store_face_bytes("AsyncFont", &face, epaint_default_fonts::UBUNTU_LIGHT.to_vec()));
        let gen_after = bytes_generation();
        assert!(gen_after > gen_before, "storing valid bytes bumps generation");
    }

    /// Tests collection face detection, index resolution, and needed_faces selection.
    #[test]
    fn collection_detection_and_needed_faces() {
        let bytes = epaint_default_fonts::UBUNTU_LIGHT;
        assert!(!is_collection_font_bytes(bytes), "single ttf is not a collection");
        assert_eq!(resolve_collection_face_index(bytes, "Ubuntu-Light"), 0);

        let faces = vec![
            Face::new_web("MultiFace".into(), 0, "Regular", "MultiFace-Regular".into()),
            Face::new_web("MultiFace".into(), 1, "Bold", "MultiFace-Bold".into()),
            Face::new_web("MultiFace".into(), 2, "Italic", "MultiFace-Italic".into()),
            Face::new_web("MultiFace".into(), 3, "Light", "MultiFace-Light".into()),
        ];
        let regular = pick_face(&faces, false, false).unwrap();
        assert_eq!(regular.js_index, 0);
        let bold = pick_face(&faces, true, false).unwrap();
        assert_eq!(bold.js_index, 1);
    }

    /// Tests that a web face stored with a resolved collection index matches catalog lookups and binds with the collection index.
    #[test]
    fn collection_face_storage_matches_catalog_lookup() {
        // In the catalog, web faces start with index: 0, js_index: 3:
        let catalog_face = Face::new_web("CollectionFamily".into(), 3, "Bold", "CollectionFamily-Bold".into());
        assert_eq!(catalog_face.index, 0);

        // When bytes arrive, resolved face retains js_index: 3:
        let mut resolved_face = catalog_face.clone();
        resolved_face.index = 0;
        assert!(store_face_bytes("CollectionFamily", &resolved_face, epaint_default_fonts::UBUNTU_LIGHT.to_vec()));

        // Catalog face (with index 0) must return true for has_face_bytes:
        assert!(has_face_bytes("CollectionFamily", &catalog_face));

        // Binding the catalog face must succeed:
        let mut fonts = egui::FontDefinitions::default();
        let bound = bind_faces(&mut fonts, "CollectionFamily", std::slice::from_ref(&catalog_face));
        assert_eq!(bound.len(), 4);
    }

    /// Tests that storing identical font bytes reuses the cached process-lifetime buffer pointer without leaking.
    #[test]
    fn storing_identical_bytes_reuses_buffer_pointer() {
        let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let face1 = Face::new_web("BufferReuseFamily".into(), 0, "Regular", "BufferReuse-Regular".into());
        let face2 = Face::new_web("BufferReuseFamily".into(), 1, "Bold", "BufferReuse-Bold".into());
        assert!(store_face_bytes("BufferReuseFamily", &face1, epaint_default_fonts::UBUNTU_LIGHT.to_vec()));
        let buf1 = get_face_buffer("BufferReuseFamily", &face1).expect("buffer 1");

        // Storing for another face with same bytes reuses existing buffer:
        assert!(store_face_bytes("BufferReuseFamily", &face2, epaint_default_fonts::UBUNTU_LIGHT.to_vec()));
        let buf2 = get_face_buffer("BufferReuseFamily", &face2).expect("buffer 2");
        assert!(std::ptr::eq(buf1.as_ptr(), buf2.as_ptr()), "buffer pointers must match");

        // Overwriting an existing face with same bytes also preserves the same buffer:
        assert!(store_face_bytes("BufferReuseFamily", &face1, epaint_default_fonts::UBUNTU_LIGHT.to_vec()));
        let buf1_again = get_face_buffer("BufferReuseFamily", &face1).expect("buffer 1 again");
        assert!(std::ptr::eq(buf1.as_ptr(), buf1_again.as_ptr()), "overwritten buffer pointer must match");
    }

    /// Tests that family_combo_items returns cached items matching families().
    #[test]
    fn family_combo_items_matches_families() {
        let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fams = families();
        let items = family_combo_items();
        assert_eq!(fams.len(), items.len());
        for (f, (val, disp)) in fams.iter().zip(items.iter()) {
            assert_eq!(f.as_str(), *val);
            assert_eq!(f.as_str(), disp.as_str());
        }
    }
}
