//! Text direction helpers for right-to-left scripts (Hebrew, Arabic…).
//!
//! The glyphs themselves are put in visual order by the text layout (epaint's bidi pass, see
//! `vendor/epaint/src/text/bidi.rs`). These helpers decide what the spreadsheet does around that:
//! General-aligned text that starts with a right-to-left letter aligns right, as Excel does with its
//! default "Context" reading order, the in-cell editor follows the text's side, and the arrow keys
//! move the caret the way they point on screen.

/// A strong right-to-left character (bidi class R or AL): Hebrew, Arabic, Syriac, Thaana, N'Ko and
/// their presentation forms, plus the right-to-left mark.
pub fn is_rtl(c: char) -> bool {
    matches!(c as u32,
        0x0590..=0x08FF // Hebrew, Arabic, Syriac, Arabic Supplement, Thaana, N'Ko, Samaritan, Mandaic, Arabic Extended
        | 0x200F // RIGHT-TO-LEFT MARK
        | 0xFB1D..=0xFDFF // Hebrew and Arabic presentation forms A
        | 0xFE70..=0xFEFF // Arabic presentation forms B
        | 0x10800..=0x10FFF // historic right-to-left scripts
        | 0x1E800..=0x1EFFF // Mende Kikakui, Adlam, Arabic mathematical symbols
    ) && !is_rtl_neutral(c)
}

/// Characters inside the right-to-left blocks that are not strong right-to-left letters: combining
/// marks (Hebrew points, Arabic harakat) and Arabic-Indic digits, which take their direction from
/// their neighbours.
fn is_rtl_neutral(c: char) -> bool {
    matches!(c as u32,
        0x0591..=0x05BD | 0x05BF | 0x05C1..=0x05C2 | 0x05C4..=0x05C5 | 0x05C7 // Hebrew points and accents
        | 0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06DC | 0x06DF..=0x06E4 | 0x06E7..=0x06E8 | 0x06EA..=0x06ED // Arabic marks
        | 0x0660..=0x0669 | 0x06F0..=0x06F9 // Arabic-Indic digits
        | 0xFB1E // Hebrew point judeo-spanish varika
    )
}

/// A strong left-to-right character: a letter of any other script, or the left-to-right mark.
pub fn is_ltr(c: char) -> bool {
    c == '\u{200E}' || (c.is_alphabetic() && !is_rtl(c) && !is_rtl_neutral(c))
}

/// The paragraph direction of `text` (Unicode rules P2–P3): right to left when its first strong
/// character is right-to-left. Text without letters (numbers, punctuation) is left to right.
pub fn starts_rtl(text: &str) -> bool {
    for c in text.chars() {
        if is_rtl(c) {
            return true;
        }
        if is_ltr(c) {
            return false;
        }
    }
    false
}

/// Whether the caret at char index `caret` of `text` sits in right-to-left text, so the Left and
/// Right arrow keys move it backwards and forwards the other way round (visually, as they point).
///
/// The character after the caret decides, then the one before it; digits count as left to right
/// (they are laid out left to right even inside Hebrew), and anything else falls back to the
/// paragraph direction.
pub fn caret_in_rtl(text: &str, caret: usize) -> bool {
    let side = |c: char| {
        if is_rtl(c) {
            Some(true)
        } else if is_ltr(c) || c.is_ascii_digit() {
            Some(false)
        } else {
            None
        }
    };
    let after = text.chars().nth(caret).and_then(side);
    let before = caret.checked_sub(1).and_then(|i| text.chars().nth(i)).and_then(side);
    after.or(before).unwrap_or_else(|| starts_rtl(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_strong_character_sets_the_direction() {
        assert!(starts_rtl("שלום עולם"));
        assert!(starts_rtl("  123 שלום world"));
        assert!(starts_rtl("مرحبا"));
        assert!(!starts_rtl("Hello עולם"));
        assert!(!starts_rtl("123"));
        assert!(!starts_rtl(""));
        // Points and digits are not strong: the letter after them decides.
        assert!(starts_rtl("\u{05B0}שלום"));
        assert!(!starts_rtl("٣ abc"));
        // The marks force a direction.
        assert!(starts_rtl("\u{200F}abc"));
        assert!(!starts_rtl("\u{200E}שלום"));
    }

    #[test]
    fn the_caret_follows_the_text_around_it() {
        let text = "שלום abc";
        assert!(caret_in_rtl(text, 0));
        assert!(caret_in_rtl(text, 4)); // after "שלום", before the space: the letter before decides
        assert!(!caret_in_rtl(text, 5)); // before "abc"
        assert!(!caret_in_rtl(text, 8)); // at the end, after "c"
        assert!(!caret_in_rtl("שלום 123", 6)); // among digits
        assert!(caret_in_rtl("", 0) == starts_rtl(""));
    }
}
