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

use miniz_oxide::inflate::decompress_to_vec_zlib_with_limit;

use super::{emf, png};

/// Most Escher records read from one drawing at most.
const MAX_RECORDS: usize = 1_000_000;
/// Deepest container nesting followed.
const MAX_DEPTH: usize = 16;
/// Largest picture kept after decompression or conversion.
const MAX_PICTURE: usize = 64 * 1024 * 1024;
/// Most pictures kept for a workbook or a sheet.
pub(super) const MAX_PICTURES: usize = 10_000;

/// A picture of the workbook's store, ready to place on a sheet.
#[derive(Clone, Debug)]
pub(super) struct Blip {
    pub data: Vec<u8>,
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
    let mut count = 0;
    walk(group, 0, &mut count, &mut |h, body| {
        // BSE: btWin32, btMacOS, uid(16), tag, size, cRef, foDelay, unused, cbName, unused×2, name, blip.
        if h.ty == 0xF007 && out.len() < MAX_PICTURES {
            out.push(read_bse(body));
        }
    });
    out
}

fn read_bse(body: &[u8]) -> Stored {
    const BSE_HEADER: usize = 36;
    let cb_name = usize::from(body.get(33).copied().unwrap_or(0));
    let Some(rest) = body.get(BSE_HEADER + cb_name..) else { return Stored::Unsupported };
    let Some(h) = header(rest, 0) else { return Stored::Unsupported };
    let Some(blip) = rest.get(8..8usize.saturating_add(h.len).min(rest.len())) else { return Stored::Unsupported };
    decode_blip(&h, blip).map_or(Stored::Unsupported, Stored::Picture)
}

fn decode_blip(h: &Header, body: &[u8]) -> Option<Blip> {
    // A second UID follows the first when the record instance is odd.
    let uids = if h.inst & 1 == 1 { 32 } else { 16 };
    match h.ty {
        // JPEG (and CMYK JPEG), PNG: one tag byte, then the file.
        0xF01D | 0xF02A => Some(Blip { data: body.get(uids + 1..)?.to_vec(), mime: "image/jpeg" }),
        0xF01E => Some(Blip { data: body.get(uids + 1..)?.to_vec(), mime: "image/png" }),
        0xF01F => Some(Blip { data: dib_to_png(body.get(uids + 1..)?)?, mime: "image/png" }),
        0xF01A => {
            // Metafile header: cbSize, bounds(16), size(8), cbSave, compression, filter.
            let head = body.get(uids..uids + 34)?;
            let cb_size = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
            let cb_save = u32::from_le_bytes([head[28], head[29], head[30], head[31]]) as usize;
            let data = body.get(uids + 34..)?.get(..cb_save.min(body.len()))?;
            let raw = if head[32] == 0 { decompress_to_vec_zlib_with_limit(data, cb_size.clamp(1, MAX_PICTURE)).ok()? } else { data.to_vec() };
            Some(Blip { data: emf::render_png(&raw)?, mime: "image/png" })
        }
        _ => None,
    }
}

/// A device-independent bitmap (a BITMAPINFOHEADER and pixels, no file header) as PNG. Only
/// uncompressed 1, 4, 8, 24 and 32-bit bitmaps are read.
fn dib_to_png(d: &[u8]) -> Option<Vec<u8>> {
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
    if w.checked_mul(h)? > 16 * 1024 * 1024 {
        return None;
    }
    if bits <= 8 && colors == 0 {
        colors = 1 << bits;
    }
    let palette = d.get(header..header + colors.min(256) * 4)?;
    let pixels = d.get(header + colors.min(256) * 4..)?;
    let stride = (w * bits as usize).div_ceil(32) * 4;
    let top_down = height < 0;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let src = if top_down { y } else { h - 1 - y };
        let row = pixels.get(src * stride..(src + 1) * stride)?;
        for x in 0..w {
            let px = match bits {
                24 => [row[x * 3 + 2], row[x * 3 + 1], row[x * 3], 255],
                32 => [row[x * 4 + 2], row[x * 4 + 1], row[x * 4], 255],
                _ => {
                    let per = 8 / bits as usize;
                    let byte = row[x / per];
                    let shift = 8 - bits as usize * (x % per + 1);
                    let i = usize::from((byte >> shift) & ((1u8 << bits) - 1));
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
