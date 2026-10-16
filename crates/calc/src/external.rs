//! External (add-in) functions: formulas call them by name when no built-in function or
//! LAMBDA name matches. Plugins register them here (`crates/plugin`); the registry is
//! process-wide, like Excel's add-ins, so every evaluation path (cells, conditional formats,
//! validation, charts, the CLI's `eval`) sees them.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use gridcraft_core::Value;

/// A function provided from outside the built-in library.
pub trait ExternalFunction: Send + Sync {
    /// Calls the function with its evaluated arguments (ranges arrive as arrays). Never panics:
    /// a failure is an error value.
    fn call(&self, args: &[Value]) -> Value;
    /// Recalculated on every recalculation, like `RAND`.
    fn volatile(&self) -> bool {
        false
    }
    fn min_args(&self) -> usize {
        0
    }
    /// Excel allows at most 255 arguments.
    fn max_args(&self) -> usize {
        255
    }
}

type Registry = RwLock<HashMap<String, Arc<dyn ExternalFunction>>>;

fn registry() -> &'static Registry {
    static REG: OnceLock<Registry> = OnceLock::new();
    REG.get_or_init(|| RwLock::new(HashMap::new()))
}

fn key(name: &str) -> String {
    name.trim().to_ascii_uppercase()
}

/// Registers (or replaces) the function called `name` (case-insensitive).
pub fn register(name: &str, f: Arc<dyn ExternalFunction>) {
    let mut reg = match registry().write() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    reg.insert(key(name), f);
}

/// Removes the function called `name`; true when it was registered.
pub fn unregister(name: &str) -> bool {
    let mut reg = match registry().write() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    reg.remove(&key(name)).is_some()
}

/// The function called `name`, if one is registered.
pub fn get(name: &str) -> Option<Arc<dyn ExternalFunction>> {
    let reg = match registry().read() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    reg.get(&key(name)).cloned()
}

/// Names of the registered functions, sorted.
pub fn names() -> Vec<String> {
    let reg = match registry().read() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    let mut v: Vec<String> = reg.keys().cloned().collect();
    v.sort();
    v
}

// ---------------------------------------------------------------- the calling workbook

/// The workbook whose formulas are being calculated, as add-in functions are told
/// (`functions/call`'s `document`, plugin protocol 2.2).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct CallingDocument {
    /// The local file it was opened from or last saved to (absolute); `None` for a workbook that
    /// has no file.
    pub path: Option<String>,
    /// Its name as the title bar shows it.
    pub name: String,
    /// An id that stays the same for the workbook while the app runs, saved or not.
    pub id: String,
}

thread_local! {
    static DOCUMENT: std::cell::RefCell<Option<Arc<CallingDocument>>> = const { std::cell::RefCell::new(None) };
    /// An add-in function returned explicit blanks during the formula being evaluated.
    static BLANKS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Puts the previous calling document back, also when the calculation unwinds.
struct Restore(Option<Arc<CallingDocument>>);

impl Drop for Restore {
    fn drop(&mut self) {
        let prev = self.0.take();
        DOCUMENT.with(|d| *d.borrow_mut() = prev);
    }
}

/// Runs `f` with `doc` as the calling workbook of the add-in functions it calculates (on this
/// thread), then puts the previous one back. Nests.
pub fn with_document<R>(doc: Option<CallingDocument>, f: impl FnOnce() -> R) -> R {
    let prev = DOCUMENT.with(|d| std::mem::replace(&mut *d.borrow_mut(), doc.map(Arc::new)));
    let _restore = Restore(prev);
    f()
}

/// The workbook being calculated on this thread, when the app said which ([`with_document`]).
pub fn document() -> Option<Arc<CallingDocument>> {
    DOCUMENT.with(|d| d.borrow().clone())
}

/// Called by an add-in function whose result holds explicit blanks: a formula that is the call
/// itself shows them blank instead of 0.
pub fn note_blanks() {
    BLANKS.with(|b| b.set(true));
}

/// Sets the explicit-blanks flag, returning what it was (the recalculation keeps one per formula).
pub(crate) fn swap_blanks(v: bool) -> bool {
    BLANKS.with(|b| b.replace(v))
}
