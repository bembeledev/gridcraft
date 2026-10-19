//! Text measurement a UI lends the engine.
//!
//! The engine has no fonts, so on its own it sizes rows of wrapped text from per-character width
//! estimates ([`crate::cmd::format::approx_text_width`]). Those can't match the font the grid draws
//! with, and in a narrow column a word that measures a little wider than estimated moves to its own
//! line and the row cuts it off. A UI with font metrics installs a measurer for its thread; row
//! heights then come from the text as it is drawn. Without one (CLI, MCP, tests) the estimate stays.

use std::cell::RefCell;
use std::rc::Rc;

/// The cell text a row height needs measured, in the font that draws it.
pub struct TextMeasure<'a> {
    pub text: &'a str,
    /// The cell's font name (e.g. `"Calibri"`).
    pub font: &'a str,
    /// Font size in points.
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    /// Wrap the text at this width (pixels at 100% zoom); `None` keeps each line whole.
    pub wrap_width: Option<f32>,
}

/// The laid-out text's height in pixels at 100% zoom and its number of lines, or `None` when it
/// can't be measured.
pub type Measurer = dyn Fn(&TextMeasure<'_>) -> Option<(f32, usize)>;

thread_local! {
    static MEASURER: RefCell<Option<Rc<Measurer>>> = const { RefCell::new(None) };
}

/// Install (or with `None`, remove) this thread's measurer.
pub fn set_text_measurer(m: Option<Rc<Measurer>>) {
    MEASURER.with(|cell| {
        if let Ok(mut slot) = cell.try_borrow_mut() {
            *slot = m;
        }
    });
}

/// Whether this thread has a measurer installed.
pub(crate) fn installed() -> bool {
    MEASURER.with(|cell| cell.try_borrow().is_ok_and(|slot| slot.is_some()))
}

/// The measured height and line count of `q`, if a measurer is installed and gives a usable size.
pub(crate) fn measure(q: &TextMeasure<'_>) -> Option<(f32, usize)> {
    // Clone the measurer out so no borrow is held while it runs.
    let m = MEASURER.with(|cell| cell.try_borrow().ok().and_then(|slot| slot.clone()))?;
    m(q).filter(|(h, _)| h.is_finite() && *h > 0.0)
}
