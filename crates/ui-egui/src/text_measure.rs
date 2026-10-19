//! Lends the engine real text measurement, so automatic row heights fit wrapped text as the grid
//! draws it ([`gridcraft_engine::measure`]).
//!
//! The measurer keeps its own [`Fonts`] built from the same definitions the context uses, so it
//! never takes the context's lock: commands can run while egui holds it.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use egui::epaint::text::{Fonts, TextOptions};
use egui::{Color32, FontDefinitions, FontFamily, FontId};
use gridcraft_engine::measure::{TextMeasure, set_text_measurer};

use crate::theme;

/// Measures cell text with `defs` on this thread from now on. Call it wherever the context's fonts
/// are set, with the same definitions.
pub fn install(defs: &FontDefinitions) {
    let families: HashSet<FontFamily> = defs.families.keys().cloned().collect();
    let fonts = RefCell::new(Fonts::new(TextOptions::default(), defs.clone()));
    set_text_measurer(Some(Rc::new(move |q: &TextMeasure<'_>| {
        // A family the definitions don't hold would make epaint panic: fall back to the cell font.
        let family = Some(theme::cell_family(q.font, q.bold, q.italic))
            .filter(|f| families.contains(f))
            .or_else(|| Some(FontFamily::Name(theme::CELL.into())).filter(|f| families.contains(f)))?;
        let size = q.size * 96.0 / 72.0;
        if !(size.is_finite() && size > 0.0) {
            return None;
        }
        let wrap = q.wrap_width.filter(|w| w.is_finite() && *w > 0.0).unwrap_or(f32::INFINITY);
        let job = egui::text::LayoutJob::simple(q.text.to_owned(), FontId::new(size, family), Color32::PLACEHOLDER, wrap);
        let mut fonts = fonts.try_borrow_mut().ok()?;
        // Starts a pass so a full atlas is cleared and old galleys are dropped from the cache.
        fonts.begin_pass(TextOptions::default());
        let galley = fonts.with_pixels_per_point(1.0).layout_job(job);
        Some((galley.size().y, galley.rows.len()))
    })));
}
