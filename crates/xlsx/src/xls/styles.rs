//! Cell formatting of an `.xls` workbook: the Font ([MS-XLS] 2.4.122), XF (2.4.353) and Palette
//! (2.4.192) records, turned into model styles (font, fill, borders, alignment, protection).

use gridcraft_model::style::Protection;
use gridcraft_model::{
    Alignment, BorderLine, BorderStyle, Borders, Color, Fill, Font, HAlign, NumFmt, PatternType, Style, Underline, VAlign, VertAlign,
};

use super::records::Cursor;
use crate::IoError;

/// The colours of palette entries 8 to 63 ([MS-XLS] 2.2.2) a workbook starts with, as 0xRRGGBB.
const DEFAULT_PALETTE: [u32; 56] = [
    0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x800000, 0x008000, 0x000080, 0x808000, 0x800080, 0x008080,
    0xC0C0C0, 0x808080, 0x9999FF, 0x993366, 0xFFFFCC, 0xCCFFFF, 0x660066, 0xFF8080, 0x0066CC, 0xCCCCFF, 0x000080, 0xFF00FF, 0xFFFF00, 0x00FFFF,
    0x800080, 0x800000, 0x008080, 0x0000FF, 0x00CCFF, 0xCCFFFF, 0xCCFFCC, 0xFFFF99, 0x99CCFF, 0xFF99CC, 0xCC99FF, 0xFFCC99, 0x3366FF, 0x33CCCC,
    0x99CC00, 0xFFCC00, 0xFF9900, 0xFF6600, 0x666699, 0x969696, 0x003366, 0x339966, 0x003300, 0x333300, 0x993300, 0x993366, 0x333399, 0x333333,
];

/// Colours a Palette record may redefine at most (entries 8 to 63).
pub(super) const MAX_PALETTE: usize = 56;

/// Index of the first palette entry a Palette record redefines.
const PALETTE_BASE: usize = 8;

/// A colour index ([MS-XLS] 2.5.161 `IcvXF`/`Icv`) as a model colour. 64 (system text) and
/// 0x7FFF (automatic font colour) are automatic; 65 (system background) is white.
pub(super) fn color(icv: u16, palette: &[u32]) -> Color {
    let i = usize::from(icv);
    match i {
        0..=7 => Color::Rgb(DEFAULT_PALETTE.get(i).copied().unwrap_or(0)),
        8..=63 => Color::Rgb(palette.get(i - PALETTE_BASE).or_else(|| DEFAULT_PALETTE.get(i - PALETTE_BASE)).copied().unwrap_or(0)),
        65 => Color::Rgb(0xFFFFFF),
        _ => Color::Auto,
    }
}

/// A Font record.
#[derive(Clone, Debug, Default)]
pub(super) struct FontRec {
    name: String,
    /// Height in points.
    size: f32,
    bold: bool,
    italic: bool,
    strike: bool,
    underline: Underline,
    vert: VertAlign,
    icv: u16,
    family: u8,
    charset: u8,
}

impl FontRec {
    pub(super) fn parse(c: &mut Cursor<'_, '_>) -> Result<FontRec, IoError> {
        let height = c.u16()?;
        let flags = c.u16()?;
        let icv = c.u16()?;
        let weight = c.u16()?;
        let script = c.u16()?;
        let underline = c.u8()?;
        let family = c.u8()?;
        let charset = c.u8()?;
        c.skip(1)?; // reserved
        let name = c.short_string()?;
        Ok(FontRec {
            name,
            size: f32::from(height) / 20.0,
            bold: weight >= 700,
            italic: flags & 0x0002 != 0,
            strike: flags & 0x0008 != 0,
            underline: match underline {
                1 => Underline::Single,
                2 => Underline::Double,
                0x21 => Underline::SingleAccounting,
                0x22 => Underline::DoubleAccounting,
                _ => Underline::None,
            },
            vert: match script {
                1 => VertAlign::Superscript,
                2 => VertAlign::Subscript,
                _ => VertAlign::Baseline,
            },
            icv,
            family,
            charset,
        })
    }

    fn font(&self, palette: &[u32]) -> Font {
        let base = Font::default();
        Font {
            name: if self.name.is_empty() { base.name } else { self.name.clone() },
            size: if self.size >= 1.0 { self.size } else { base.size },
            bold: self.bold,
            italic: self.italic,
            underline: self.underline,
            strike: self.strike,
            color: color(self.icv, palette),
            vert: self.vert,
            charset: (self.charset != 0).then_some(self.charset),
            family: (self.family != 0).then_some(self.family),
            // An .xls font names its face; it never follows a theme.
            minor: false,
        }
    }
}

/// An XF record: how a cell (or a cell style) is formatted.
#[derive(Clone, Debug, Default)]
pub(super) struct Xf {
    font: u16,
    /// Number format index.
    pub(super) format: u16,
    locked: bool,
    hidden: bool,
    h: u8,
    v: u8,
    wrap: bool,
    rotation: u8,
    indent: u8,
    shrink: bool,
    /// Left, right, top, bottom border styles and colours.
    sides: [(u8, u16); 4],
    diag: u8,
    diag_side: (u8, u16),
    pattern: u8,
    fore: u16,
    back: u16,
}

impl Xf {
    pub(super) fn parse(c: &mut Cursor<'_, '_>) -> Result<Xf, IoError> {
        let font = c.u16()?;
        let format = c.u16()?;
        let flags = c.u16()?;
        let align = c.u8()?;
        let rotation = c.u8()?;
        let indent = c.u8()?;
        c.u8()?; // used-attribute flags
        let b1 = c.u16()?;
        let b2 = c.u16()?;
        let b3 = c.u32()?;
        let fill = c.u16()?;
        Ok(Xf {
            font,
            format,
            locked: flags & 0x0001 != 0,
            hidden: flags & 0x0002 != 0,
            h: align & 7,
            v: (align >> 4) & 7,
            wrap: align & 0x08 != 0,
            rotation,
            indent: indent & 0x0F,
            shrink: indent & 0x10 != 0,
            sides: [
                ((b1 & 0xF) as u8, b2 & 0x7F),
                (((b1 >> 4) & 0xF) as u8, (b2 >> 7) & 0x7F),
                (((b1 >> 8) & 0xF) as u8, (b3 & 0x7F) as u16),
                (((b1 >> 12) & 0xF) as u8, ((b3 >> 7) & 0x7F) as u16),
            ],
            diag: ((b2 >> 14) & 3) as u8,
            diag_side: (((b3 >> 21) & 0xF) as u8, ((b3 >> 14) & 0x7F) as u16),
            pattern: ((b3 >> 26) & 0x3F) as u8,
            fore: fill & 0x7F,
            back: (fill >> 7) & 0x7F,
        })
    }

    /// Whether a blank cell with this format still shows something: a fill or a border.
    pub(super) fn paints(&self) -> bool {
        self.pattern != 0 || self.sides.iter().any(|(s, _)| *s != 0) || (self.diag != 0 && self.diag_side.0 != 0)
    }
}

fn border_style(code: u8) -> BorderStyle {
    match code {
        1 => BorderStyle::Thin,
        2 => BorderStyle::Medium,
        3 => BorderStyle::Dashed,
        4 => BorderStyle::Dotted,
        5 => BorderStyle::Thick,
        6 => BorderStyle::Double,
        7 => BorderStyle::Hair,
        8 => BorderStyle::MediumDashed,
        9 => BorderStyle::DashDot,
        0xA => BorderStyle::MediumDashDot,
        0xB => BorderStyle::DashDotDot,
        0xC => BorderStyle::MediumDashDotDot,
        0xD => BorderStyle::SlantDashDot,
        _ => BorderStyle::None,
    }
}

fn pattern(code: u8) -> PatternType {
    match code {
        1 => PatternType::Solid,
        2 => PatternType::Gray50,
        3 => PatternType::Gray75,
        4 => PatternType::Gray25,
        5 => PatternType::DarkHorizontal,
        6 => PatternType::DarkVertical,
        7 => PatternType::DarkDown,
        8 => PatternType::DarkUp,
        9 => PatternType::DarkGrid,
        0xA => PatternType::DarkTrellis,
        0xB => PatternType::LightHorizontal,
        0xC => PatternType::LightVertical,
        0xD => PatternType::LightDown,
        0xE => PatternType::LightUp,
        0xF => PatternType::LightGrid,
        0x10 => PatternType::LightTrellis,
        0x11 => PatternType::Gray125,
        0x12 => PatternType::Gray0625,
        _ => PatternType::None,
    }
}

/// The model style of an XF. `num_fmt` is its number format code (`None` for General).
pub(super) fn style(xf: &Xf, fonts: &[FontRec], palette: &[u32], num_fmt: Option<&str>) -> Style {
    // Font 4 is never written; later indices are shifted by one ([MS-XLS] 2.4.122).
    let font_at = |i: u16| match i {
        0..=3 => fonts.get(usize::from(i)),
        4 => None,
        _ => fonts.get(usize::from(i) - 1),
    };
    let side = |(code, icv): (u8, u16)| BorderLine { style: border_style(code), color: color(icv, palette) };
    let none = BorderLine::default();
    let [left, right, top, bottom] = xf.sides.map(side);
    let diag = side(xf.diag_side);
    let fill = match pattern(xf.pattern) {
        PatternType::None => Fill::default(),
        p => Fill { pattern: p, fg: color(xf.fore, palette), bg: color(xf.back, palette) },
    };
    Style {
        font: font_at(xf.font).map(|f| f.font(palette)).unwrap_or_default(),
        fill,
        borders: Borders {
            left,
            right,
            top,
            bottom,
            diag_down: if xf.diag & 1 != 0 { diag } else { none },
            diag_up: if xf.diag & 2 != 0 { diag } else { none },
        },
        align: Alignment {
            h: match xf.h {
                1 => HAlign::Left,
                2 => HAlign::Center,
                3 => HAlign::Right,
                4 => HAlign::Fill,
                5 => HAlign::Justify,
                6 => HAlign::CenterAcross,
                7 => HAlign::Distributed,
                _ => HAlign::General,
            },
            v: match xf.v {
                0 => VAlign::Top,
                1 => VAlign::Center,
                3 => VAlign::Justify,
                4 => VAlign::Distributed,
                _ => VAlign::Bottom,
            },
            wrap: xf.wrap,
            shrink: xf.shrink,
            indent: xf.indent,
            rotation: match xf.rotation {
                r @ 0..=90 => i16::from(r),
                r @ 91..=180 => -(i16::from(r) - 90),
                255 => 255,
                _ => 0,
            },
        },
        num_fmt: num_fmt.map(NumFmt::new).unwrap_or_default(),
        protection: Protection { locked: xf.locked, hidden: xf.hidden },
        ..Style::default()
    }
}
