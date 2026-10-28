//! Pictures of an `.xls` workbook: the Office Drawing ("Escher") records of
//! [MS-ODRAWING], carried by the MsoDrawingGroup and MsoDrawing records of [MS-XLS].
//!
//! The workbook's drawing group holds the picture store: one BSE record per picture, with the
//! picture itself embedded. Each sheet's drawing lists its shapes; a picture shape names its store
//! entry (property `pib`, 1-based) and carries a client anchor: the cells it spans.
//!
//! PNG and JPEG pictures are kept as they are, DIB bitmaps are re-encoded as PNG, and EMF
//! metafiles are drawn to a PNG ([`super::emf`]), since GridCraft shows PNG and JPEG. Other
//! formats (WMF, PICT, TIFF) are reported as not imported.

use std::sync::Arc;

use miniz_oxide::inflate::decompress_to_vec_zlib_with_limit;

use super::{emf, png};

/// Most Escher records read from one drawing at most.
const MAX_RECORDS: usize = 1_000_000;
/// Deepest container nesting followed.
const MAX_DEPTH: usize = 16;
/// Largest picture kept after decompression or conversion.
const MAX_PICTURE: usize = 64 * 1024 * 1024;
/// Bytes of pictures (decompressed or converted) a whole workbook's store may hold.
const MAX_STORE_BYTES: usize = 256 * 1024 * 1024;
/// Pixels of bitmaps (DIB) a whole workbook's store may decode: each one is expanded to four
/// bytes a pixel before it is re-encoded.
const MAX_STORE_PIXELS: usize = 64 * 1024 * 1024;
/// Most pictures kept for a workbook or a sheet.
pub(super) const MAX_PICTURES: usize = 10_000;

/// A picture of the workbook's store, ready to place on a sheet.
#[derive(Clone, Debug)]
pub(super) struct Blip {
    /// Shared by every shape that shows the picture.
    pub data: Arc<Vec<u8>>,
    pub mime: &'static str,
}

/// What a picture of the store turned out to be.
pub(super) enum Stored {
    Picture(Blip),
    /// A picture GridCraft can't show (WMF, PICT, TIFF, an unreadable one).
    Unsupported,
}

/// A picture shape of a sheet: which store entry, and the cells it spans.
#[derive(Clone, Copy, Debug)]
pub(super) struct PictureShape {
    /// 1-based index into the store.
    pub blip: usize,
    pub flags: u16,
    /// Column, offset in 1/1024 of the column, row, offset in 1/256 of the row; top left, then bottom right.
    pub from: (u16, u16, u16, u16),
    pub to: (u16, u16, u16, u16),
}

/// The shapes of a sheet's drawing.
#[derive(Default)]
pub(super) struct SheetShapes {
    pub pictures: Vec<PictureShape>,
    /// Shapes that are not pictures (charts, text boxes, lines, …).
    pub others: usize,
}

struct Header {
    ver: u16,
    inst: u16,
    ty: u16,
    len: usize,
}

fn header(d: &[u8], at: usize) -> Option<Header> {
    let h = d.get(at..at.checked_add(8)?)?;
    let vi = u16::from_le_bytes([h[0], h[1]]);
    Some(Header { ver: vi & 0xF, inst: vi >> 4, ty: u16::from_le_bytes([h[2], h[3]]), len: u32::from_le_bytes([h[4], h[5], h[6], h[7]]) as usize })
}

/// Calls `leaf` with every non-container record, in order, descending into containers.
fn walk(d: &[u8], depth: usize, count: &mut usize, leaf: &mut dyn FnMut(&Header, &[u8])) {
    let mut at = 0usize;
    while let Some(h) = header(d, at) {
        *count += 1;
        if *count > MAX_RECORDS {
            return;
        }
        let start = at + 8;
        let end = start.saturating_add(h.len).min(d.len());
        let body = d.get(start..end).unwrap_or(&[]);
        if h.ver == 0xF {
            if depth < MAX_DEPTH {
                walk(body, depth + 1, count, leaf);
            }
        } else {
            leaf(&h, body);
        }
        at = end;
    }
}

/// The picture store of a drawing group (the concatenated MsoDrawingGroup data).
pub(super) fn read_store(group: &[u8]) -> Vec<Stored> {
    let mut out: Vec<Stored> = vec![];
    let mut work = Work { raster: emf::RASTER_BUDGET, bytes: MAX_STORE_BYTES, pixels: MAX_STORE_PIXELS };
    let mut count = 0;
    walk(group, 0, &mut count, &mut |h, body| {
        // BSE: btWin32, btMacOS, uid(16), tag, size, cRef, foDelay, unused, cbName, unused×2, name, blip.
        if h.ty == 0xF007 && out.len() < MAX_PICTURES {
            out.push(read_bse(body, &mut work));
        }
    });
    out
}

/// What decoding the whole picture store may still cost: every entry is decoded, whether or not a
/// sheet shows it, so the limits are for the store and not for each entry.
struct Work {
    raster: usize,
    bytes: usize,
    pixels: usize,
}

impl Work {
    /// Takes `n` bytes from the store's allowance, or `None` when it is used up.
    fn charge(&mut self, n: usize) -> Option<()> {
        self.bytes = self.bytes.checked_sub(n)?;
        Some(())
    }
}

fn read_bse(body: &[u8], work: &mut Work) -> Stored {
    const BSE_HEADER: usize = 36;
    let cb_name = usize::from(body.get(33).copied().unwrap_or(0));
    let Some(rest) = body.get(BSE_HEADER + cb_name..) else { return Stored::Unsupported };
    let Some(h) = header(rest, 0) else { return Stored::Unsupported };
    let Some(blip) = rest.get(8..8usize.saturating_add(h.len).min(rest.len())) else { return Stored::Unsupported };
    decode_blip(&h, blip, work).map_or(Stored::Unsupported, Stored::Picture)
}

fn decode_blip(h: &Header, body: &[u8], work: &mut Work) -> Option<Blip> {
    // A second UID follows the first when the record instance is odd.
    let uids = if h.inst & 1 == 1 { 32 } else { 16 };
    match h.ty {
        // JPEG (and CMYK JPEG), PNG: one tag byte, then the file.
        0xF01D | 0xF02A | 0xF01E => {
            let file = body.get(uids + 1..)?;
            work.charge(file.len())?;
            Some(Blip { data: Arc::new(file.to_vec()), mime: if h.ty == 0xF01E { "image/png" } else { "image/jpeg" } })
        }
        0xF01F => {
            let png = dib_to_png(body.get(uids + 1..)?, &mut work.pixels)?;
            work.charge(png.len())?;
            Some(Blip { data: Arc::new(png), mime: "image/png" })
        }
        0xF01A => {
            // Metafile header: cbSize, bounds(16), size(8), cbSave, compression, filter.
            let head = body.get(uids..uids + 34)?;
            let cb_size = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
            let cb_save = u32::from_le_bytes([head[28], head[29], head[30], head[31]]) as usize;
            let data = body.get(uids + 34..)?.get(..cb_save.min(body.len()))?;
            let raw = if head[32] == 0 {
                decompress_to_vec_zlib_with_limit(data, cb_size.clamp(1, MAX_PICTURE).min(work.bytes)).ok()?
            } else {
                data.to_vec()
            };
            work.charge(raw.len())?;
            let png = emf::render_png(&raw, &mut work.raster)?;
            work.charge(png.len())?;
            Some(Blip { data: Arc::new(png), mime: "image/png" })
        }
        _ => None,
    }
}

/// A device-independent bitmap (a BITMAPINFOHEADER and pixels, no file header) as PNG. Only
/// uncompressed 1, 4, 8, 24 and 32-bit bitmaps are read.
fn dib_to_png(d: &[u8], pixels_left: &mut usize) -> Option<Vec<u8>> {
    let u32_at = |o: usize| d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let header = u32_at(0)? as usize;
    if header < 40 {
        return None;
    }
    let width = u32_at(4)? as i32;
    let height = u32_at(8)? as i32;
    let bits = u32::from(d.get(14..16).map(|b| u16::from_le_bytes([b[0], b[1]]))?);
    let compression = u32_at(16)?;
    let mut colors = u32_at(32)? as usize;
    if compression != 0 || width <= 0 || height == 0 || !matches!(bits, 1 | 4 | 8 | 24 | 32) {
        return None;
    }
    let (w, h) = (width as usize, height.unsigned_abs() as usize);
    let count = w.checked_mul(h)?;
    if count > 16 * 1024 * 1024 {
        return None;
    }
    // Taken before the pixels are expanded, from the allowance of the whole store.
    *pixels_left = pixels_left.checked_sub(count)?;
    if bits <= 8 && colors == 0 {
        colors = 1 << bits;
    }
    let pixels_at = header.checked_add(colors.min(256).checked_mul(4)?)?;
    let palette = d.get(header..pixels_at)?;
    let pixels = d.get(pixels_at..)?;
    let stride = w.checked_mul(bits as usize)?.div_ceil(32).checked_mul(4)?;
    let top_down = height < 0;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let src = if top_down { y } else { h - 1 - y };
        let from = src.checked_mul(stride)?;
        let row = pixels.get(from..from.checked_add(stride)?)?;
        for x in 0..w {
            let px = match bits {
                24 => [row[x * 3 + 2], row[x * 3 + 1], row[x * 3], 255],
                32 => [row[x * 4 + 2], row[x * 4 + 1], row[x * 4], 255],
                _ => {
                    let per = 8 / bits as usize;
                    let byte = row[x / per];
                    let shift = 8 - bits as usize * (x % per + 1);
                    let i = usize::from((byte >> shift) & (u8::MAX >> (8 - bits)));
                    let c = palette.get(i * 4..i * 4 + 3)?;
                    [c[2], c[1], c[0], 255]
                }
            };
            rgba.extend_from_slice(&px);
        }
    }
    png::encode(w as u32, h as u32, &rgba)
}

/// The shapes of a sheet's drawing (the concatenated MsoDrawing data).
pub(super) fn read_shapes(drawing: &[u8]) -> SheetShapes {
    let mut shapes = SheetShapes::default();
    // Each shape is an SpContainer: an Sp, its Opt properties and a client anchor. The records
    // come in that order, so a shape is complete once the next Sp starts or the data ends.
    #[derive(Default)]
    struct Cur {
        open: bool,
        group: bool,
        blip: usize,
        anchor: Option<(u16, (u16, u16, u16, u16), (u16, u16, u16, u16))>,
    }
    fn close(cur: &mut Cur, shapes: &mut SheetShapes) {
        if cur.open && !cur.group {
            match (cur.blip, cur.anchor) {
                (blip, Some((flags, from, to))) if blip > 0 && shapes.pictures.len() < MAX_PICTURES => {
                    shapes.pictures.push(PictureShape { blip, flags, from, to });
                }
                _ => shapes.others += 1,
            }
        }
        *cur = Cur::default();
    }
    let mut cur = Cur::default();
    let mut count = 0;
    walk(drawing, 0, &mut count, &mut |h, body| match h.ty {
        0xF00A => {
            close(&mut cur, &mut shapes);
            let flags = body.get(4..8).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
            cur = Cur { open: true, group: flags & 0x5 != 0, ..Cur::default() };
        }
        0xF00B if cur.open => {
            // Properties: id (14 bits, flags), value. `pib` (0x0104) names the picture.
            for i in 0..usize::from(h.inst) {
                let Some(p) = body.get(i * 6..i * 6 + 6) else { break };
                let id = u16::from_le_bytes([p[0], p[1]]) & 0x3FFF;
                if id == 0x0104 {
                    cur.blip = u32::from_le_bytes([p[2], p[3], p[4], p[5]]) as usize;
                }
            }
        }
        0xF010 if cur.open && body.len() >= 18 => {
            let w = |i: usize| u16::from_le_bytes([body[i * 2], body[i * 2 + 1]]);
            cur.anchor = Some((w(0), (w(1), w(2), w(3), w(4)), (w(5), w(6), w(7), w(8))));
        }
        _ => {}
    });
    close(&mut cur, &mut shapes);
    shapes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bottom-up DIB: a 40-byte header, a palette of `colors` (B, G, R, 0) and the pixel rows.
    fn dib(width: i32, height: i32, bits: u16, colors: &[[u8; 3]], rows: &[&[u8]]) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&40u32.to_le_bytes());
        d.extend_from_slice(&width.to_le_bytes());
        d.extend_from_slice(&height.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&bits.to_le_bytes());
        d.extend_from_slice(&[0; 4]); // BI_RGB
        d.extend_from_slice(&[0; 12]);
        d.extend_from_slice(&(colors.len() as u32).to_le_bytes());
        d.extend_from_slice(&[0; 4]);
        for [b, g, r] in colors {
            d.extend_from_slice(&[*b, *g, *r, 0]);
        }
        for r in rows {
            d.extend_from_slice(r);
        }
        d
    }

    /// One bitmap against a fresh allowance.
    fn to_png(d: &[u8]) -> Option<Vec<u8>> {
        let mut left = 16 * 1024 * 1024;
        dib_to_png(d, &mut left)
    }

    fn pixels(png: &[u8]) -> Vec<[u8; 4]> {
        image::load_from_memory(png).unwrap().to_rgba8().pixels().map(|p| p.0).collect()
    }

    #[test]
    fn an_8_bit_bitmap_uses_every_palette_entry() {
        // 2 x 2, rows padded to 4 bytes, stored bottom-up: the bottom row (red, blue) comes first.
        let palette = [[0, 0, 255], [255, 0, 0], [0, 255, 0]];
        let d = dib(2, 2, 8, &palette, &[&[0, 1, 0, 0], &[2, 0, 0, 0]]);
        let px = pixels(&to_png(&d).unwrap());
        assert_eq!(px, [[0, 255, 0, 255], [255, 0, 0, 255], [255, 0, 0, 255], [0, 0, 255, 255]]);
    }

    #[test]
    fn small_bit_depths_and_true_colour_bitmaps_decode() {
        // 1 bit: eight pixels in a byte, the first in the top bit.
        let d = dib(2, 1, 1, &[[0, 0, 0], [255, 255, 255]], &[&[0b0100_0000, 0, 0, 0]]);
        assert_eq!(pixels(&to_png(&d).unwrap()), [[0, 0, 0, 255], [255, 255, 255, 255]]);
        // 4 bits: two pixels in a byte.
        let d = dib(2, 1, 4, &[[0, 0, 0], [0, 0, 255]], &[&[0x01, 0, 0, 0]]);
        assert_eq!(pixels(&to_png(&d).unwrap()), [[0, 0, 0, 255], [255, 0, 0, 255]]);
        // 24 bits: B, G, R.
        let d = dib(1, 1, 24, &[], &[&[10, 20, 30, 0]]);
        assert_eq!(pixels(&to_png(&d).unwrap()), [[30, 20, 10, 255]]);
    }

    #[test]
    fn broken_bitmaps_are_refused_not_a_crash() {
        assert!(to_png(&[]).is_none());
        // A palette longer than the data, a header size near the top of u32, a bitmap too short.
        assert!(to_png(&dib(2, 2, 8, &[[0, 0, 0]], &[&[0, 0, 0, 0]])).is_none());
        let mut huge = dib(1, 1, 24, &[], &[&[0, 0, 0, 0]]);
        huge[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(to_png(&huge).is_none());
        assert!(to_png(&dib(60_000, 60_000, 24, &[], &[])).is_none());
    }

    #[test]
    fn bitmaps_share_one_pixel_allowance() {
        let row = [0u8; 300];
        let rows: Vec<&[u8]> = (0..100).map(|_| &row[..]).collect();
        let d = dib(100, 100, 24, &[], &rows);
        let mut left = 15_000;
        assert!(dib_to_png(&d, &mut left).is_some());
        assert_eq!(left, 5_000);
        // The next 100 x 100 bitmap no longer fits.
        assert!(dib_to_png(&d, &mut left).is_none());
    }
}
