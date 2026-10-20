//! Searchable fallback glyph subsets. Type 3 char procedures embed only the used outlines;
//! a ToUnicode map preserves the source characters without relying on a viewer's CJK fonts.
use crate::{Font, n, win_ansi};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::{Arc, OnceLock};
use ttf_parser::{Face, OutlineBuilder};

include!(concat!(env!("OUT_DIR"), "/unicode_fonts.rs"));

struct Source {
    data: Arc<[u8]>,
    index: u32,
}

fn sources() -> &'static [Source] {
    static SOURCES: OnceLock<Vec<Source>> = OnceLock::new();
    SOURCES.get_or_init(|| {
        let mut sources: Vec<_> = BUNDLED_FONTS.iter().map(|(data, index)| Source { data: Arc::from(*data), index: *index }).collect();
        #[cfg(not(target_arch = "wasm32"))]
        if sources.is_empty() {
            // Installed fonts are read, never bundled or copied into the repository.
            for (path, index) in [
                ("/usr/share/fonts/opentype/noto/NotoSansCJKsc-Regular.otf", 0),
                ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 2),
                ("/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc", 2),
            ] {
                if std::fs::metadata(path).is_ok_and(|m| m.len() <= 64 * 1024 * 1024)
                    && let Ok(data) = std::fs::read(path)
                {
                    sources.push(Source { data: data.into(), index });
                }
            }
        }
        sources
    })
}

/// Nonprinting controls have no standalone glyph in this unshaped writer.
/// Keep tabs as spaces. Contextual shaping and variation-glyph selection remain unsupported.
pub fn nonprinting(c: char) -> bool {
    (c.is_control() && c != '\t')
        || matches!(c, '\u{200b}'..='\u{200d}' | '\u{2060}' | '\u{feff}' | '\u{fe00}'..='\u{fe0f}' | '\u{e0100}'..='\u{e01ef}')
}

pub fn needed(c: char) -> bool {
    win_ansi(c) == b'?' && c != '?'
}

fn glyph(c: char) -> Option<(&'static Source, ttf_parser::GlyphId, u16)> {
    sources().iter().find_map(|source| {
        let face = Face::parse(&source.data, source.index).ok()?;
        if !face.is_subsetting_allowed()
            || !face.is_outline_embedding_allowed()
            || matches!(face.permissions(), Some(ttf_parser::Permissions::Restricted) | None)
        {
            return None;
        }
        let id = face.glyph_index(c)?;
        let width = u32::from(face.glyph_hor_advance(id)?) * 1000 / u32::from(face.units_per_em().max(1));
        Some((source, id, width.min(u32::from(u16::MAX)) as u16))
    })
}

pub fn width(c: char) -> Option<u16> {
    glyph(c).map(|(_, _, width)| width)
}

struct Outline {
    path: String,
    scale: f32,
    at: (f32, f32),
    overflow: bool,
}
impl Outline {
    fn add(&mut self, s: String) {
        if self.path.len().saturating_add(s.len()) <= 1024 * 1024 {
            self.path.push_str(&s);
        } else {
            self.overflow = true;
        }
    }
}
impl OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.at = (x, y);
        self.add(format!("{} {} m\n", n(x * self.scale), n(y * self.scale)));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.at = (x, y);
        self.add(format!("{} {} l\n", n(x * self.scale), n(y * self.scale)));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (px, py) = self.at;
        self.curve_to(px + (x1 - px) * 2.0 / 3.0, py + (y1 - py) * 2.0 / 3.0, x + (x1 - x) * 2.0 / 3.0, y + (y1 - y) * 2.0 / 3.0, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.at = (x, y);
        self.add(format!(
            "{} {} {} {} {} {} c\n",
            n(x1 * self.scale),
            n(y1 * self.scale),
            n(x2 * self.scale),
            n(y2 * self.scale),
            n(x * self.scale),
            n(y * self.scale)
        ));
    }
    fn close(&mut self) {
        self.add("h\n".into());
    }
}

struct Glyph {
    character: char,
    width: u16,
    program: String,
}
#[derive(Default)]
pub struct Subset {
    ids: BTreeMap<(Font, char), usize>,
    glyphs: Vec<Glyph>,
    outline_bytes: usize,
}

impl Subset {
    pub fn add(&mut self, font: Font, c: char) -> Result<(usize, u8, u16), String> {
        if let Some(&i) = self.ids.get(&(font, c)) {
            return Ok((i / 255, (i % 255 + 1) as u8, self.glyphs.get(i).map_or(1000, |g| g.width)));
        }
        if self.glyphs.len() >= 8192 {
            return Err("PDF page contains too many fallback glyphs".into());
        }
        let (source, id, width) =
            glyph(c).ok_or_else(|| format!("No PDF font covers U+{:04X}; provide the licensed craft-fonts build input", c as u32))?;
        let face = Face::parse(&source.data, source.index).map_err(|_| "PDF fallback font cannot be read")?;
        let mut outline = Outline { path: String::new(), scale: 1000.0 / f32::from(face.units_per_em().max(1)), at: (0.0, 0.0), overflow: false };
        if face.outline_glyph(id, &mut outline).is_none() && !c.is_whitespace() {
            return Err(format!("PDF font has no outline for U+{:04X}", c as u32));
        }
        if outline.overflow {
            return Err("PDF fallback glyph outline is too large".into());
        }
        let bold = matches!(
            font,
            Font::HelveticaBold | Font::HelveticaBoldOblique | Font::TimesBold | Font::TimesBoldItalic | Font::CourierBold | Font::CourierBoldOblique
        );
        let italic = matches!(
            font,
            Font::HelveticaOblique
                | Font::HelveticaBoldOblique
                | Font::TimesItalic
                | Font::TimesBoldItalic
                | Font::CourierOblique
                | Font::CourierBoldOblique
        );
        let program = format!(
            "{width} 0 -1000 -1000 3000 3000 d1\n{}{}{}{}\n",
            if italic { "1 0 0.21256 1 0 0 cm\n" } else { "" },
            if bold { "20 w\n" } else { "" },
            outline.path,
            if bold { "B" } else { "f" }
        );
        if self.outline_bytes.saturating_add(program.len()) > 32 * 1024 * 1024 {
            return Err("PDF page fallback outlines exceed the content limit".into());
        }
        self.outline_bytes += program.len();
        let i = self.glyphs.len();
        self.glyphs.push(Glyph { character: c, width, program });
        self.ids.insert((font, c), i);
        Ok((i / 255, (i % 255 + 1) as u8, width))
    }

    pub fn objects(&self, next: &mut u32, objects: &mut Vec<(u32, Vec<u8>)>) -> String {
        let mut resources = String::new();
        for (bank, glyphs) in self.glyphs.chunks(255).enumerate() {
            let font_id = *next;
            *next += 1;
            let mut procs = String::new();
            let mut differences = String::from("1 ");
            let mut widths = String::new();
            let mut cmap = String::from(
                "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /GridCraftUnicode def\n/CMapType 2 def\n1 begincodespacerange\n<00> <FF>\nendcodespacerange\n",
            );
            for (chunk_index, chunk) in glyphs.chunks(100).enumerate() {
                let _ = writeln!(cmap, "{} beginbfchar", chunk.len());
                for (index, g) in chunk.iter().enumerate() {
                    let code = chunk_index * 100 + index + 1;
                    let _ = write!(cmap, "<{code:02X}> <");
                    for unit in g.character.encode_utf16(&mut [0; 2]) {
                        let _ = write!(cmap, "{unit:04X}");
                    }
                    cmap.push_str(">\n");
                }
                cmap.push_str("endbfchar\n");
            }
            cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
            for (i, g) in glyphs.iter().enumerate() {
                let id = *next;
                *next += 1;
                let _ = write!(procs, "/g{} {id} 0 R ", i + 1);
                let _ = write!(differences, "/g{} ", i + 1);
                let _ = write!(widths, "{} ", g.width);
                objects.push((id, stream(&g.program)));
            }
            let map_id = *next;
            *next += 1;
            objects.push((map_id, stream(&cmap)));
            objects.push((font_id, format!("<< /Type /Font /Subtype /Type3 /Name /GridCraftUnicode{bank} /FontBBox [-1000 -1000 3000 3000] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << {procs} >> /Encoding << /Type /Encoding /Differences [{differences}] >> /FirstChar 1 /LastChar {} /Widths [{widths}] /Resources << >> /ToUnicode {map_id} 0 R >>", glyphs.len()).into_bytes()));
            let _ = write!(resources, "/U{bank} {font_id} 0 R ");
        }
        resources
    }
}

fn stream(content: &str) -> Vec<u8> {
    format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()).into_bytes()
}
