use super::font_setup::candidates;

/// Regular and bold system fonts for printed and exported text the standard PDF fonts can't
/// show (Greek, Cyrillic, …): the first sheet-font candidate that parses as an embeddable
/// TrueType file and has Greek or Cyrillic letters (`pdf_font_covers_greek_or_cyrillic`).
/// Pass to `gridcraft_engine::pdf::set_unicode_font_loader` (the desktop app and the CLI do).
pub fn pdf_unicode_fonts() -> [Option<Vec<u8>>; 2] {
    ["cell", "cell-bold"].map(|role| {
        candidates(role)
            .into_iter()
            .filter(|(path, index)| *index == 0 && path.to_ascii_lowercase().ends_with(".ttf"))
            .filter_map(|(path, _)| read_pdf_font(&path))
            .find(|bytes| gridcraft_engine::pdf::UnicodeFont::parse(bytes.clone()).is_some_and(|f| pdf_font_covers_greek_or_cyrillic(&f)))
    })
}

/// Bound the native fallback read before parsing or cloning bytes. A system-font symlink may
/// point at a device or a growing file, so metadata alone never limits the allocation.
pub(super) fn read_pdf_font(path: &str) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let limit = gridcraft_engine::pdf::MAX_FONT_BYTES as u64;
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > limit {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

/// Whether a PDF fallback font has real Greek or Cyrillic letters, not just Latin: a capital and
/// a lowercase letter of each script. Greek: `Ω` (U+03A9) and `ω` (U+03C9), the last of each
/// case. Cyrillic: `Ж` (U+0416, a capital with no Latin look-alike, unlike `А`) and `џ`
/// (U+045F, the end of the U+0400–U+045F block), so a font with only the 33 Russian letters (the
/// JIS Gothic faces) doesn't qualify and Serbian, Macedonian, Ukrainian and Belarusian text gets
/// real glyphs too.
pub(super) fn pdf_font_covers_greek_or_cyrillic(f: &gridcraft_engine::pdf::UnicodeFont) -> bool {
    (f.has('Ω') && f.has('ω')) || (f.has('Ж') && f.has('џ'))
}
