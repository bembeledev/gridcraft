//! Persian and Arabic (right-to-left) text must render with real glyphs, not tofu boxes, in both
//! the interface and the cell fonts, and a workbook must keep it through an XLSX round-trip.
//!
//! A separate test binary: it runs the process-wide installed-font scan, whose faces bind from the
//! next frame and would leak into the other integration tests (see `it/main.rs`).

use egui::{Color32, FontFamily, FontId};
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_ui_egui::{fonts, theme};
use serde_json::json;
use std::sync::{Mutex, MutexGuard};

/// `font_definitions*` records the bound role families process-wide. Tests in this binary build
/// different font sets, so serialize them; the app has one font set and never hits this race.
static FONT_LOCK: Mutex<()> = Mutex::new(());

fn lock_fonts() -> MutexGuard<'static, ()> {
    FONT_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// One sample per goal: a Persian phrase, mixed Persian/English, Arabic with punctuation, and a
/// Persian sentence holding a Latin URL and Latin digits (all common in real sheets).
const PERSIAN: &str = "سلام دنیا";
const MIXED: &str = "GridCraft فارسی 123";
const ARABIC_PUNCT: &str = "مرحبا، كيف حالك؟ (١٢٣)";
const PERSIAN_URL: &str = "وب‌سایت https://example.com؟ 42";

/// A context with the same font set the app installs, built after the scan so dynamically-found
/// faces (not just the hardcoded paths) are bound.
fn context() -> egui::Context {
    let _ = fonts::catalog(); // finish the scan before building the font set
    let ctx = egui::Context::default();
    ctx.set_fonts(theme::font_definitions());
    let mut out = ctx.run_ui(egui::RawInput::default(), |_ui| {});
    out.textures_delta.clear(); // nothing consumes the atlas in a headless test
    ctx
}

/// Every drawn glyph of `text` in `family`.
fn drawn(ctx: &egui::Context, family: &FontFamily, text: &str) -> Vec<egui::epaint::text::Glyph> {
    let font = FontId::new(14.0, family.clone());
    let galley = ctx.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font, Color32::BLACK));
    galley.rows.iter().flat_map(|r| r.glyphs.iter().copied()).collect()
}

/// Whether the font set has an Arabic-script face for the fallback chain: a system face or the
/// permitted craft-fonts Arabic face. Without one there is nothing to shape with, and the tests
/// skip instead of failing a minimal install (the CI gate builds craft fonts).
fn arabic_face_loaded() -> bool {
    let fonts = theme::font_definitions();
    fonts.font_data.contains_key("sys-arabic") || fonts.families.contains_key(&FontFamily::Name(theme::CELL_ARABIC.into()))
}

/// The characters of `text` that have no glyph in `family`. Fonts may draw a missing character
/// as a zero-width component (egui 0.37's replacement glyph), so ask the font chain directly.
fn tofu_chars(ctx: &egui::Context, family: &FontFamily, text: &str) -> String {
    let font = FontId::new(14.0, family.clone());
    ctx.fonts_mut(|f| text.chars().filter(|ch| *ch != ' ' && !f.has_glyph(&font, *ch)).collect())
}

#[test]
fn persian_renders_real_glyphs_in_the_interface_and_cell_fonts() {
    let _g = lock_fonts();
    let ctx = context();
    if !arabic_face_loaded() {
        eprintln!("skipped: no Arabic-script font installed");
        return;
    }
    if !fonts::faces_of("Vazirmatn").is_empty() || !fonts::faces_of("Noto Sans Arabic").is_empty() {
        // The scan found an Arabic-script face; the fallback must have loaded one of them.
        assert!(
            theme::font_definitions().font_data.contains_key("sys-arabic"),
            "an Arabic font is installed but no Arabic-script fallback face was bound"
        );
    }
    for family in [FontFamily::Proportional, FontFamily::Name(theme::CELL.into())] {
        for sample in [PERSIAN, MIXED, ARABIC_PUNCT, PERSIAN_URL] {
            let missing = tofu_chars(&ctx, &family, sample);
            assert!(missing.is_empty(), "{sample:?} renders as boxes in {family:?}: {missing:?}");
        }
    }
}

#[test]
fn bold_and_italic_cell_roles_still_have_arabic() {
    let _g = lock_fonts();
    if !arabic_face_loaded() {
        eprintln!("skipped: no Arabic-script font installed");
        return;
    }
    // Bold cells use a different role chain; it must carry `sys-arabic` too, or a bold Persian cell
    // would fall to tofu even though the regular one renders.
    let ctx = context();
    for role in [theme::CELL_BOLD, theme::CELL_ITALIC, theme::CELL_BOLD_ITALIC] {
        let family = FontFamily::Name(role.into());
        let missing = tofu_chars(&ctx, &family, PERSIAN);
        assert!(missing.is_empty(), "bold/italic {role} renders Persian as boxes: {missing:?}");
    }
}

#[test]
fn mixed_latin_and_persian_keeps_the_persian_shaped_and_the_latin_readable() {
    let _g = lock_fonts();
    let ctx = context();
    if !arabic_face_loaded() {
        eprintln!("skipped: no Arabic-script font installed");
        return;
    }
    let family = FontFamily::Name(theme::CELL.into());
    // Every character (Latin, Persian, digits, space) has a real glyph.
    assert!(tofu_chars(&ctx, &family, MIXED).is_empty());
    // The galley is wider than its Latin part alone, so the Persian is actually laid out.
    let width = |text: &str| ctx.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), FontId::new(14.0, family.clone()), Color32::BLACK)).size().x;
    let (full, latin) = (width(MIXED), width("GridCraft 123"));
    assert!(full > latin, "mixed text ({full}) must be wider than its Latin part ({latin})");
}

/// Within one shaping run, HarfRust lays a right-to-left script's glyphs out in visual order: the
/// last logical character comes first, drawn at the left. That is what makes the letters join and
/// read right to left. epaint has no bidi reordering *between* runs (`TODO(emilk): heed bidi
/// characters`): a space can be owned by a different face, splitting a sentence into one run per
/// word, and those runs are concatenated in logical order. So this checks a single word only; the
/// limitation is recorded in `docs/localization-parity.md`.
#[test]
fn a_right_to_left_word_is_shaped_in_visual_order() {
    let _g = lock_fonts();
    let ctx = context();
    if !arabic_face_loaded() {
        eprintln!("skipped: no Arabic-script font installed");
        return;
    }
    let family = FontFamily::Name(theme::CELL.into());
    let glyphs = drawn(&ctx, &family, "اب");
    let at = |ch: char| glyphs.iter().find(|g| g.chr == ch && g.advance_width > 0.0).map(|g| g.pos.x);
    let (Some(alef), Some(beh)) = (at('ا'), at('ب')) else {
        panic!("both letters should be shaped: {glyphs:?}");
    };
    assert!(beh < alef, "the last logical letter (ب) must be left of the first (ا), got ا={alef}, ب={beh}");
}

/// A Persian sentence is two right-to-left runs if a space belongs to a Latin face. The Arabic cell
/// family leads with the script face instead, so the whole sentence is one run and its words are in
/// reading order. (Mixed Latin/Arabic still has no bidi; see `docs/localization-parity.md`.)
#[test]
fn a_persian_sentence_keeps_its_word_order_in_the_arabic_cell_family() {
    let _g = lock_fonts();
    let ctx = context();
    if !arabic_face_loaded() || !fonts::is_bound(theme::CELL_ARABIC) {
        eprintln!("skipped: no Arabic-script font installed");
        return;
    }
    assert_eq!(
        theme::cell_family_for_text("No Such Font", false, false, PERSIAN),
        FontFamily::Name(theme::CELL_ARABIC.into()),
        "a font without Arabic must use the script-leading family"
    );
    let glyphs = drawn(&ctx, &FontFamily::Name(theme::CELL_ARABIC.into()), PERSIAN);
    let at = |ch: char| glyphs.iter().find(|g| g.chr == ch && g.advance_width > 0.0).map(|g| g.pos.x);
    let (first_word, second_word) = (at('س'), at('د'));
    let (Some(salam), Some(donya)) = (first_word, second_word) else {
        panic!("both words should be shaped: {glyphs:?}");
    };
    assert!(donya < salam, "دنیا must be left of سلام in a right-to-left sentence");
}

#[test]
fn an_installed_arabic_font_keeps_its_own_family_for_arabic_cells() {
    let _g = lock_fonts();
    let _ = fonts::catalog();
    let installed = ["Vazirmatn", "Noto Naskh Arabic", "Noto Sans Arabic"].into_iter().find(|name| !fonts::faces_of(name).is_empty());
    let Some(name) = installed else {
        eprintln!("skipped: no named Arabic-script font installed");
        return;
    };
    let ctx = egui::Context::default();
    ctx.set_fonts(theme::font_definitions_with(gridcraft_ui_egui::i18n::Language::En, &[name.to_string()]));
    let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
    out.textures_delta.clear();
    let family = theme::cell_family_for_text(name, false, false, PERSIAN);
    let FontFamily::Name(role) = &family else { panic!("an installed font must name its own family, got {family:?}") };
    assert!(fonts::is_bound(role), "the chosen Arabic font's role must be bound");
    assert_ne!(&**role, theme::CELL_ARABIC, "a font with Arabic coverage keeps its own typeface");
    assert!(tofu_chars(&ctx, &family, PERSIAN).is_empty(), "the chosen font {name} must draw Persian");
}

#[test]
fn a_workbook_keeps_persian_and_its_font_through_an_xlsx_round_trip() {
    let _g = lock_fonts();
    let _ = fonts::catalog();
    if !arabic_face_loaded() {
        eprintln!("skipped: no Arabic-script font installed");
        return;
    }
    let mut session = Session::new();
    session.new_workbook();
    session.execute("range.setValues", json!({"range": "A1:A2", "values": [[PERSIAN], [MIXED]]})).expect("set values");
    // A Persian font selected for the cell, as a user would, when it is installed.
    let vazirmatn = !fonts::faces_of("Vazirmatn").is_empty();
    if vazirmatn {
        session.execute("home.fontName", json!({"range": "A1", "name": "Vazirmatn"})).expect("set font");
    }
    let saved = session.execute("file.saveBytes", json!({"format": "xlsx"})).expect("save");
    let base64 = saved.get("base64").and_then(|b| b.as_str()).expect("base64");
    let name = saved.get("name").and_then(|n| n.as_str()).unwrap_or("Book.xlsx");
    session.execute("file.open", json!({"name": name, "base64": base64})).expect("reopen");

    let (font, family_name) = {
        let doc = session.active().expect("document");
        let sh = doc.wb.active().expect("sheet");
        assert_eq!(sh.value(CellRef::new(0, 0)), Value::Text(PERSIAN.into()), "A1 survives the round-trip");
        assert_eq!(sh.value(CellRef::new(1, 0)), Value::Text(MIXED.into()), "A2 survives the round-trip");
        let st = doc.wb.styles.get(sh.style_id(CellRef::new(0, 0)));
        (st.font.clone(), st.font.name.clone())
    };
    if vazirmatn {
        assert_eq!(font.name, "Vazirmatn", "the cell font survives the round-trip");
    }
    // Build the reopened workbook's font set and render the cell text in its own family.
    let used = vec![font.name.clone()];
    let ctx = egui::Context::default();
    ctx.set_fonts(theme::font_definitions_with(gridcraft_ui_egui::i18n::Language::En, &used));
    let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
    out.textures_delta.clear();
    let family = theme::cell_family(&family_name, font.bold, font.italic);
    if let FontFamily::Name(role) = &family {
        assert!(fonts::is_bound(role), "the cell font role {role:?} must be bound to font data");
    }
    let missing = tofu_chars(&ctx, &family, PERSIAN);
    assert!(missing.is_empty(), "the reopened cell's Persian renders as boxes: {missing:?}");
}
