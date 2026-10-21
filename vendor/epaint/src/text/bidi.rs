//! Bidirectional text: the Unicode Bidirectional Algorithm (UAX #9) applied to laid-out rows.
//!
//! Text is shaped and line-broken in logical (memory) order, as before. Afterwards every row that
//! holds right-to-left text (Hebrew, Arabic…) is reordered for display: the glyphs keep their
//! logical order in [`Row::glyphs`] (so cursor indices, selection and copying are unchanged), but
//! their x positions are moved to their visual places, and paired punctuation such as brackets is
//! mirrored inside right-to-left runs. Rows without right-to-left text are left untouched.
//!
//! The paragraph direction follows the first strong character (rules P2–P3), so "שלום world"
//! reads right to left and "Hello עולם" left to right.

use unicode_bidi::{BidiClass, BidiInfo, Level, bidi_class};

use super::{Glyph, Row};

/// Whether `c` can make a paragraph contain right-to-left text: a strong right-to-left letter or
/// an explicit right-to-left embedding, override or isolate.
#[inline]
pub(crate) fn is_rtl_trigger(c: char) -> bool {
    // Fast path: everything below the Hebrew block is never right-to-left.
    if (c as u32) < 0x0590 {
        return false;
    }
    matches!(
        bidi_class(c),
        BidiClass::R | BidiClass::AL | BidiClass::RLE | BidiClass::RLO | BidiClass::RLI
    )
}

/// The character drawn for `c` inside a right-to-left run (Bidi_Mirroring_Glyph), for the paired
/// punctuation that commonly appears in text. `None` when `c` is not mirrored.
pub(crate) fn mirrored(c: char) -> Option<char> {
    Some(match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        '‹' => '›',
        '›' => '‹',
        '⁅' => '⁆',
        '⁆' => '⁅',
        '⁽' => '⁾',
        '⁾' => '⁽',
        '₍' => '₎',
        '₎' => '₍',
        '≤' => '≥',
        '≥' => '≤',
        '≪' => '≫',
        '≫' => '≪',
        '⟨' => '⟩',
        '⟩' => '⟨',
        '〈' => '〉',
        '〉' => '〈',
        '⦃' => '⦄',
        '⦄' => '⦃',
        '﴾' => '﴿',
        '﴿' => '﴾',
        '（' => '）',
        '）' => '（',
        '［' => '］',
        '］' => '［',
        '｛' => '｝',
        '｝' => '｛',
        '＜' => '＞',
        '＞' => '＜',
        _ => return None,
    })
}

/// A glyph that belongs to the cluster of the glyph before it: a combining mark (Hebrew points,
/// cantillation, Arabic harakat) or a zero-width continuation glyph of a multi-character cluster.
fn continues_cluster(glyph: &Glyph) -> bool {
    super::text_layout::is_combining_mark(glyph.chr)
        || (glyph.advance_width == 0.0 && glyph.uv_rect.is_nothing())
        || matches!(glyph.chr, '\u{200D}' | '\u{FE00}'..='\u{FE0F}')
}

/// Resolved embedding levels for one paragraph, split into its rows.
pub(crate) struct ParagraphLevels {
    /// One entry per row of the paragraph, one level per glyph of that row (rule L1 applied).
    pub rows: Vec<Vec<Level>>,
}

/// Runs the bidi algorithm over a paragraph made of `rows` (each a slice of glyphs, in logical
/// order). Returns `None` when the paragraph holds no right-to-left text, the common case, which
/// costs one scan over the characters.
pub(crate) fn resolve_paragraph(rows: &[&[Glyph]]) -> Option<ParagraphLevels> {
    if !rows.iter().flat_map(|r| r.iter()).any(|g| is_rtl_trigger(g.chr)) {
        return None;
    }
    // The paragraph text, one character per glyph, and each row's byte range in it.
    let mut text = String::new();
    let mut ranges = Vec::with_capacity(rows.len());
    for row in rows {
        let start = text.len();
        for g in *row {
            // Line and paragraph separators can't appear inside a row; keep them neutral so
            // `BidiInfo` sees a single paragraph.
            text.push(match bidi_class(g.chr) {
                BidiClass::B => ' ',
                _ => g.chr,
            });
        }
        ranges.push(start..text.len());
    }
    let info = BidiInfo::new(&text, None);
    let para = info.paragraphs.first()?;
    let rows = ranges
        .into_iter()
        .map(|range| {
            if range.is_empty() {
                return Vec::new();
            }
            // Per character of the line, with trailing whitespace reset to the paragraph level.
            let per_byte = info.reordered_levels(para, range.clone());
            text.get(range.clone())
                .map(|line| {
                    line.char_indices()
                        .map(|(i, _)| per_byte.get(range.start + i).copied().unwrap_or(para.level))
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect();
    Some(ParagraphLevels { rows })
}

/// Moves the glyphs of `row` to their visual positions for `levels` (one per glyph). Clusters
/// (a base character and its marks) move as one, keeping the shaper's mark offsets. Glyphs stay
/// in logical order in `row.glyphs`; each is tagged with its direction for cursor placement.
///
/// `mirror` gives the glyph to draw for a mirrored character in a right-to-left run (the caller
/// owns the font atlas). Positions are kept on whole physical pixels.
pub(crate) fn reorder_row(
    row: &mut Row,
    levels: &[Level],
    pixels_per_point: f32,
    mut mirror: impl FnMut(&Glyph, char) -> Option<Glyph>,
) {
    if row.glyphs.is_empty() || levels.len() != row.glyphs.len() {
        return;
    }
    if levels.iter().all(|l| l.is_ltr()) {
        return;
    }

    // Group glyphs into clusters, in logical order.
    let mut clusters: Vec<std::ops::Range<usize>> = Vec::new();
    for (i, glyph) in row.glyphs.iter().enumerate() {
        match clusters.last_mut() {
            Some(last) if continues_cluster(glyph) => last.end = i + 1,
            _ => clusters.push(i..i + 1),
        }
    }

    // A cluster takes the level of its base character (marks inherit it anyway, rule W1).
    let cluster_levels: Vec<Level> = clusters.iter().map(|c| levels[c.start]).collect();

    // Where each cluster starts and how far the pen moves past it, from the logical layout
    // (this keeps kerning, letter spacing and tab widths).
    let starts: Vec<f32> = clusters.iter().map(|c| row.glyphs[c.start].pos.x).collect();
    let advances: Vec<f32> = clusters
        .iter()
        .enumerate()
        .map(|(ci, c)| match starts.get(ci + 1) {
            Some(next) => (next - starts[ci]).max(0.0),
            None => row.glyphs[c.clone()]
                .iter()
                .map(|g| g.max_x())
                .fold(starts[ci], f32::max)
                - starts[ci],
        })
        .collect();

    // Mirror paired punctuation inside right-to-left runs.
    for (ci, c) in clusters.iter().enumerate() {
        if !cluster_levels[ci].is_rtl() {
            continue;
        }
        let glyph = row.glyphs[c.start];
        if let Some(m) = mirrored(glyph.chr)
            && let Some(mut replacement) = mirror(&glyph, m)
        {
            // Keep the logical character (for copying and cursor logic), draw the mirror.
            replacement.chr = glyph.chr;
            replacement.pos.y = glyph.pos.y;
            row.glyphs[c.start] = replacement;
        }
    }

    // Visual order of the clusters (rule L2).
    let visual = BidiInfo::reorder_visual(&cluster_levels);

    let snap = |x: f32| (x * pixels_per_point).round() / pixels_per_point;
    let mut pen = starts[0];
    for &ci in &visual {
        let Some(c) = clusters.get(ci) else { continue };
        let shift = snap(pen - starts[ci]);
        let rtl = cluster_levels[ci].is_rtl();
        for glyph in &mut row.glyphs[c.clone()] {
            glyph.pos.x += shift;
            glyph.rtl = rtl;
        }
        pen += advances[ci];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_text_needs_no_bidi() {
        assert!(!"Hello, world (123)".chars().any(is_rtl_trigger));
        assert!("שלום".chars().all(is_rtl_trigger));
        assert!("مرحبا".chars().all(is_rtl_trigger));
    }

    #[test]
    fn brackets_mirror_in_pairs() {
        for (a, b) in [('(', ')'), ('[', ']'), ('{', '}'), ('<', '>'), ('«', '»')] {
            assert_eq!(mirrored(a), Some(b));
            assert_eq!(mirrored(b), Some(a));
        }
        assert_eq!(mirrored('a'), None);
    }
}
