//! A small renderer for the vector drawings Excel stores as EMF pictures ([MS-EMF]), so they can
//! be shown as PNG. It draws what logos and simple drawings are made of: filled and stroked
//! polygons, polylines, Bézier curves, rectangles, ellipses and paths, with solid brushes and
//! pens. Text, bitmaps, clipping and gradients are not drawn.

use super::png;

const MAX_RECORDS: usize = 2_000_000;
const MAX_HANDLES: usize = 65_536;
/// Edges drawn at most, and the longest side of the picture produced.
const MAX_EDGES: usize = 400_000;
const MAX_SIDE: f32 = 1024.0;
const MIN_SIDE: f32 = 256.0;
/// Horizontal strips per pixel row for anti-aliasing.
const STRIPS: usize = 4;
/// Units of raster work (an edge tested against a strip, a pixel blended) a picture may cost.
const RASTER_BUDGET: usize = 300_000_000;
/// Segments a Bézier curve is cut into.
const BEZIER_STEPS: usize = 12;

type Pt = (f32, f32);
type Rgb = [u8; 3];

#[derive(Clone, Copy)]
enum Obj {
    Pen { color: Rgb, width: f32, null: bool },
    Brush { color: Rgb, null: bool },
}

struct Canvas {
    w: usize,
    h: usize,
    /// Premultiplied RGBA.
    px: Vec<[f32; 4]>,
    /// Raster work left; drawing stops, keeping what is drawn, when it runs out.
    budget: usize,
    exhausted: bool,
}

struct Renderer {
    canvas: Canvas,
    left: f32,
    top: f32,
    scale: f32,
    // Mapping from logical to device units.
    anisotropic: bool,
    win_org: (f32, f32),
    win_ext: (f32, f32),
    view_org: (f32, f32),
    view_ext: (f32, f32),
    winding: bool,
    pen: Obj,
    brush: Obj,
    objects: Vec<Option<Obj>>,
    pos: Pt,
    path: Option<Vec<Vec<Pt>>>,
    finished: Vec<Vec<Pt>>,
    edges: usize,
}

fn i32_at(d: &[u8], o: usize) -> Option<i32> {
    d.get(o..o.checked_add(4)?).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    i32_at(d, o).map(|v| v as u32)
}

fn colorref(v: u32) -> Rgb {
    [v as u8, (v >> 8) as u8, (v >> 16) as u8]
}

/// Renders the EMF as a PNG (transparent background), or `None` when it can't be read or draws nothing.
pub(super) fn render_png(emf: &[u8]) -> Option<Vec<u8>> {
    if u32_at(emf, 0)? != 1 {
        return None;
    }
    let (l, t, r, b) = (i32_at(emf, 8)?, i32_at(emf, 12)?, i32_at(emf, 16)?, i32_at(emf, 20)?);
    let (bw, bh) = (f64::from(r) - f64::from(l) + 1.0, f64::from(b) - f64::from(t) + 1.0);
    if !(bw >= 1.0 && bh >= 1.0) {
        return None;
    }
    let scale = (f64::from(MAX_SIDE).min(f64::from(MIN_SIDE).max(bw.max(bh))) / bw.max(bh)) as f32;
    let (w, h) = (((bw as f32) * scale).ceil().max(1.0) as usize, ((bh as f32) * scale).ceil().max(1.0) as usize);
    if w * h > 4_000_000 {
        return None;
    }
    // nHandles is a 16-bit count; writers that undercount it still reference their objects.
    let handles = ((u32_at(emf, 56).unwrap_or(1) & 0xFFFF) as usize).clamp(1024, MAX_HANDLES);
    let mut rd = Renderer {
        canvas: Canvas { w, h, px: vec![[0.0; 4]; w * h], budget: RASTER_BUDGET, exhausted: false },
        left: l as f32,
        top: t as f32,
        scale,
        anisotropic: false,
        win_org: (0.0, 0.0),
        win_ext: (1.0, 1.0),
        view_org: (0.0, 0.0),
        view_ext: (1.0, 1.0),
        winding: false,
        pen: Obj::Pen { color: [0, 0, 0], width: 1.0, null: false },
        brush: Obj::Brush { color: [255, 255, 255], null: false },
        objects: vec![None; handles.max(1)],
        pos: (0.0, 0.0),
        path: None,
        finished: vec![],
        edges: 0,
    };
    rd.run(emf);
    rd.finish()
}

impl Renderer {
    fn run(&mut self, d: &[u8]) {
        let mut at = 0usize;
        for _ in 0..MAX_RECORDS {
            let (Some(ty), Some(size)) = (u32_at(d, at), u32_at(d, at + 4)) else { return };
            let size = size as usize;
            if size < 8 || at.saturating_add(size) > d.len() || !size.is_multiple_of(4) {
                return;
            }
            let rec = &d[at..at + size];
            if ty == 14 {
                return;
            }
            self.record(ty, rec);
            if self.edges > MAX_EDGES || self.canvas.exhausted {
                return;
            }
            at += size;
        }
    }

    fn map(&self, x: f32, y: f32) -> Pt {
        let (mut dx, mut dy) = (x, y);
        if self.anisotropic && self.win_ext.0 != 0.0 && self.win_ext.1 != 0.0 {
            dx = (x - self.win_org.0) * self.view_ext.0 / self.win_ext.0 + self.view_org.0;
            dy = (y - self.win_org.1) * self.view_ext.1 / self.win_ext.1 + self.view_org.1;
        }
        ((dx - self.left) * self.scale, (dy - self.top) * self.scale)
    }

    fn points(&self, rec: &[u8], from: usize, n: usize, wide: bool) -> Vec<Pt> {
        let step = if wide { 4 } else { 2 };
        let mut out = Vec::with_capacity(n.min(rec.len() / 2));
        for i in 0..n {
            let o = from + i * step * 2;
            let (x, y) = if wide {
                (i32_at(rec, o), i32_at(rec, o + 4))
            } else {
                let g = |p: usize| rec.get(p..p + 2).map(|b| i32::from(i16::from_le_bytes([b[0], b[1]])));
                (g(o), g(o + 2))
            };
            let (Some(x), Some(y)) = (x, y) else { break };
            out.push(self.map(x as f32, y as f32));
        }
        out
    }

    fn select(&mut self, h: u32) {
        let obj = if h & 0x8000_0000 != 0 {
            match h & 0x7FFF_FFFF {
                0 => Some(Obj::Brush { color: [255, 255, 255], null: false }),
                1 => Some(Obj::Brush { color: [192, 192, 192], null: false }),
                2 => Some(Obj::Brush { color: [128, 128, 128], null: false }),
                3 => Some(Obj::Brush { color: [64, 64, 64], null: false }),
                4 => Some(Obj::Brush { color: [0, 0, 0], null: false }),
                5 => Some(Obj::Brush { color: [0, 0, 0], null: true }),
                6 => Some(Obj::Pen { color: [255, 255, 255], width: 1.0, null: false }),
                7 => Some(Obj::Pen { color: [0, 0, 0], width: 1.0, null: false }),
                8 => Some(Obj::Pen { color: [0, 0, 0], width: 0.0, null: true }),
                _ => None,
            }
        } else {
            self.objects.get(h as usize).copied().flatten()
        };
        match obj {
            Some(o @ Obj::Pen { .. }) => self.pen = o,
            Some(o @ Obj::Brush { .. }) => self.brush = o,
            None => {}
        }
    }

    fn create(&mut self, h: u32, obj: Obj) {
        if let Some(slot) = self.objects.get_mut(h as usize) {
            *slot = Some(obj);
        }
    }

    fn record(&mut self, ty: u32, rec: &[u8]) {
        let f = |o: usize| i32_at(rec, o).unwrap_or(0) as f32;
        match ty {
            // Mapping.
            17 => self.anisotropic = matches!(u32_at(rec, 8), Some(7 | 8)),
            9 => self.win_ext = (f(8), f(12)),
            10 => self.win_org = (f(8), f(12)),
            11 => self.view_ext = (f(8), f(12)),
            12 => self.view_org = (f(8), f(12)),
            19 => self.winding = u32_at(rec, 8) == Some(2),
            // Objects.
            37 => self.select(u32_at(rec, 8).unwrap_or(u32::MAX)),
            40 => {
                if let Some(slot) = u32_at(rec, 8).and_then(|h| self.objects.get_mut(h as usize)) {
                    *slot = None;
                }
            }
            39 => {
                let (h, style, color) = (u32_at(rec, 8), u32_at(rec, 12), u32_at(rec, 16));
                if let (Some(h), Some(style), Some(color)) = (h, style, color) {
                    self.create(h, Obj::Brush { color: colorref(color), null: style == 1 });
                }
            }
            38 => {
                let (h, style, width, color) = (u32_at(rec, 8), u32_at(rec, 12), u32_at(rec, 16), u32_at(rec, 24));
                if let (Some(h), Some(style), Some(width), Some(color)) = (h, style, width, color) {
                    self.create(h, Obj::Pen { color: colorref(color), width: width as f32, null: style & 0xF == 5 });
                }
            }
            95 => {
                let (h, style, width, color) = (u32_at(rec, 8), u32_at(rec, 28), u32_at(rec, 32), u32_at(rec, 40));
                if let (Some(h), Some(style), Some(width), Some(color)) = (h, style, width, color) {
                    self.create(h, Obj::Pen { color: colorref(color), width: width as f32, null: style & 0xF == 5 });
                }
            }
            // Position and lines.
            27 => {
                self.pos = self.map(f(8), f(12));
                if let Some(p) = self.path.as_mut() {
                    p.push(vec![self.pos]);
                }
            }
            54 => {
                let to = self.map(f(8), f(12));
                self.line_to(to);
            }
            // Polygons, polylines and curves: 32-bit points, then 16-bit points.
            3 | 86 => self.poly(rec, true, ty < 80),
            4 | 87 => self.poly(rec, false, ty < 80),
            2 | 85 => self.bezier(rec, false, ty < 80),
            5 | 88 => self.bezier(rec, true, ty < 80),
            6 | 89 => self.polyline_to(rec, ty < 80),
            8 | 91 => self.polypolygon(rec, true, ty < 80),
            7 | 90 => self.polypolygon(rec, false, ty < 80),
            43 => {
                let (a, b) = (self.map(f(8), f(12)), self.map(f(16), f(20)));
                self.shape(vec![vec![a, (b.0, a.1), b, (a.0, b.1)]], true);
            }
            42 => {
                let (a, b) = (self.map(f(8), f(12)), self.map(f(16), f(20)));
                let (cx, cy, rx, ry) = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0, (b.0 - a.0) / 2.0, (b.1 - a.1) / 2.0);
                let ring: Vec<Pt> = (0..48)
                    .map(|i| {
                        let t = i as f32 / 48.0 * std::f32::consts::TAU;
                        (cx + rx * t.cos(), cy + ry * t.sin())
                    })
                    .collect();
                self.shape(vec![ring], true);
            }
            // Paths.
            59 => {
                self.path = Some(vec![]);
                self.finished.clear();
            }
            60 => self.finished = self.path.take().unwrap_or_default(),
            61 => {
                if let Some(sub) = self.path.as_mut().and_then(|p| p.last_mut())
                    && let Some(first) = sub.first().copied()
                {
                    sub.push(first);
                }
            }
            62..=64 => {
                let polys = std::mem::take(&mut self.finished);
                let fill = ty != 64;
                let stroke = ty != 62;
                if fill && let Obj::Brush { color, null: false } = self.brush {
                    self.fill(&polys, color);
                }
                if stroke {
                    self.stroke(&polys, false);
                }
            }
            _ => {}
        }
    }

    fn line_to(&mut self, to: Pt) {
        if let Some(p) = self.path.as_mut() {
            match p.last_mut() {
                Some(sub) => sub.push(to),
                None => p.push(vec![self.pos, to]),
            }
        } else {
            let from = self.pos;
            self.stroke(&[vec![from, to]], false);
        }
        self.pos = to;
    }

    /// Fills closed shapes with the brush and outlines them with the pen, or adds them to the open path.
    fn shape(&mut self, polys: Vec<Vec<Pt>>, closed: bool) {
        if let Some(p) = self.path.as_mut() {
            for mut poly in polys {
                if closed && let Some(first) = poly.first().copied() {
                    poly.push(first);
                }
                p.push(poly);
            }
            return;
        }
        if let Obj::Brush { color, null: false } = self.brush {
            self.fill(&polys, color);
        }
        self.stroke(&polys, closed);
    }

    /// POLYGON and POLYLINE: bounds, count, points. `wide` is for 32-bit points (else 16-bit).
    fn poly(&mut self, rec: &[u8], closed: bool, wide: bool) {
        let n = u32_at(rec, 24).unwrap_or(0) as usize;
        let pts = self.points(rec, 28, n, wide);
        if pts.len() < 2 {
            return;
        }
        if closed {
            self.shape(vec![pts], true);
        } else if self.path.is_some() {
            self.shape(vec![pts], false);
        } else {
            self.stroke(&[pts], false);
        }
    }

    /// POLYBEZIER (a start point, then three points per curve) or POLYBEZIERTO (from the current position).
    fn bezier(&mut self, rec: &[u8], to: bool, wide: bool) {
        let n = u32_at(rec, 24).unwrap_or(0) as usize;
        let pts = self.points(rec, 28, n, wide);
        let mut all = vec![];
        let start = if to { Some(self.pos) } else { None };
        let seq: Vec<Pt> = start.into_iter().chain(pts).collect();
        for chunk in seq.windows(4).step_by(3) {
            let (p0, p1, p2, p3) = (chunk[0], chunk[1], chunk[2], chunk[3]);
            if all.is_empty() {
                all.push(p0);
            }
            for i in 1..=BEZIER_STEPS {
                let t = i as f32 / BEZIER_STEPS as f32;
                let u = 1.0 - t;
                all.push((
                    u * u * u * p0.0 + 3.0 * u * u * t * p1.0 + 3.0 * u * t * t * p2.0 + t * t * t * p3.0,
                    u * u * u * p0.1 + 3.0 * u * u * t * p1.1 + 3.0 * u * t * t * p2.1 + t * t * t * p3.1,
                ));
            }
        }
        if let Some(last) = all.last().copied() {
            self.pos = last;
        }
        if all.len() < 2 {
            return;
        }
        if self.path.is_some() {
            if to {
                if let Some(sub) = self.path.as_mut().and_then(|p| p.last_mut()) {
                    sub.extend(all.into_iter().skip(1));
                }
            } else {
                self.shape(vec![all], false);
            }
        } else {
            self.stroke(&[all], false);
        }
    }

    fn polyline_to(&mut self, rec: &[u8], wide: bool) {
        let n = u32_at(rec, 24).unwrap_or(0) as usize;
        let pts = self.points(rec, 28, n, wide);
        for p in pts {
            self.line_to(p);
        }
    }

    fn polypolygon(&mut self, rec: &[u8], closed: bool, wide: bool) {
        let n = u32_at(rec, 24).unwrap_or(0) as usize;
        let total = u32_at(rec, 28).unwrap_or(0) as usize;
        let counts_end = 32 + n.saturating_mul(4);
        let all = self.points(rec, counts_end, total, wide);
        let mut polys = Vec::with_capacity(n.min(rec.len() / 4));
        let mut at = 0usize;
        for i in 0..n {
            let Some(c) = u32_at(rec, 32 + i * 4).map(|c| c as usize) else { break };
            let Some(slice) = all.get(at..at.saturating_add(c).min(all.len())) else { break };
            at = at.saturating_add(c);
            if slice.len() >= 2 {
                polys.push(slice.to_vec());
            }
        }
        if closed {
            self.shape(polys, true);
        } else if self.path.is_some() {
            self.shape(polys, false);
        } else {
            self.stroke(&polys, false);
        }
    }

    fn fill(&mut self, polys: &[Vec<Pt>], color: Rgb) {
        let mut edges: Vec<(Pt, Pt)> = vec![];
        for poly in polys {
            for (i, a) in poly.iter().enumerate() {
                let b = poly.get(i + 1).or_else(|| poly.first()).copied().unwrap_or(*a);
                edges.push((*a, b));
            }
        }
        self.edges = self.edges.saturating_add(edges.len());
        self.canvas.fill_edges(&edges, color, self.winding);
    }

    fn stroke(&mut self, polys: &[Vec<Pt>], closed: bool) {
        let Obj::Pen { color, width, null: false } = self.pen else { return };
        let half = (width * self.scale).max(1.0) / 2.0;
        for poly in polys {
            let n = poly.len();
            let segments = if closed { n } else { n.saturating_sub(1) };
            for i in 0..segments {
                let (Some(&a), Some(&b)) = (poly.get(i), poly.get((i + 1) % n)) else { continue };
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                let len = (dx * dx + dy * dy).sqrt();
                if len < 1e-4 {
                    continue;
                }
                let (nx, ny) = (-dy / len * half, dx / len * half);
                let quad = vec![(a.0 + nx, a.1 + ny), (b.0 + nx, b.1 + ny), (b.0 - nx, b.1 - ny), (a.0 - nx, a.1 - ny)];
                self.fill(&[quad], color);
                if self.edges > MAX_EDGES || self.canvas.exhausted {
                    return;
                }
            }
        }
    }

    fn finish(self) -> Option<Vec<u8>> {
        let Canvas { w, h, px, .. } = self.canvas;
        let mut rgba = Vec::with_capacity(w * h * 4);
        let mut any = false;
        for [r, g, b, a] in px {
            if a > 0.0 {
                any = true;
                rgba.extend_from_slice(&[
                    (r / a * 255.0).round().clamp(0.0, 255.0) as u8,
                    (g / a * 255.0).round().clamp(0.0, 255.0) as u8,
                    (b / a * 255.0).round().clamp(0.0, 255.0) as u8,
                    (a * 255.0).round().clamp(0.0, 255.0) as u8,
                ]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
        if !any {
            return None;
        }
        png::encode(w as u32, h as u32, &rgba)
    }
}

impl Canvas {
    /// Fills the area the edges enclose (even-odd or non-zero winding), anti-aliased.
    fn fill_edges(&mut self, edges: &[(Pt, Pt)], color: Rgb, winding: bool) {
        if edges.is_empty() {
            return;
        }
        let (mut ymin, mut ymax) = (f32::MAX, f32::MIN);
        for (a, b) in edges {
            ymin = ymin.min(a.1).min(b.1);
            ymax = ymax.max(a.1).max(b.1);
        }
        if !(ymin.is_finite() && ymax.is_finite()) {
            return;
        }
        let y0 = ymin.floor().max(0.0) as usize;
        let y1 = (ymax.ceil().max(0.0) as usize).min(self.h);
        let rgb = [f32::from(color[0]) / 255.0, f32::from(color[1]) / 255.0, f32::from(color[2]) / 255.0];
        let mut cov = vec![0.0f32; self.w];
        let mut hits: Vec<(f32, i32)> = vec![];
        for y in y0..y1 {
            // The columns of this row anything covered, so only those are cleared and blended.
            let (mut lo, mut hi) = (usize::MAX, 0usize);
            for s in 0..STRIPS {
                let ys = y as f32 + (s as f32 + 0.5) / STRIPS as f32;
                if !self.spend(edges.len()) {
                    return;
                }
                hits.clear();
                for ((x0, ya), (x1, yb)) in edges {
                    if (*ya <= ys && ys < *yb) || (*yb <= ys && ys < *ya) {
                        let x = x0 + (ys - ya) * (x1 - x0) / (yb - ya);
                        hits.push((x, if yb > ya { 1 } else { -1 }));
                    }
                }
                hits.sort_by(|a, b| a.0.total_cmp(&b.0));
                if !self.spend(hits.len().saturating_mul(hits.len().max(2).ilog2() as usize + 1)) {
                    return;
                }
                let mut wind = 0i32;
                for pair in hits.windows(2) {
                    wind += if winding { pair[0].1 } else { 1 };
                    let inside = if winding { wind != 0 } else { wind % 2 == 1 };
                    if inside && let Some((first, last)) = self.span(&mut cov, pair[0].0, pair[1].0) {
                        lo = lo.min(first);
                        hi = hi.max(last);
                    }
                }
            }
            if lo >= hi {
                continue;
            }
            if !self.spend(hi - lo) {
                return;
            }
            for x in lo..hi {
                let a = (cov.get(x).copied().unwrap_or(0.0) / STRIPS as f32).min(1.0);
                if let Some(c) = cov.get_mut(x) {
                    *c = 0.0;
                }
                if a <= 0.0 {
                    continue;
                }
                if let Some(px) = self.px.get_mut(y * self.w + x) {
                    let keep = 1.0 - a;
                    *px = [rgb[0] * a + px[0] * keep, rgb[1] * a + px[1] * keep, rgb[2] * a + px[2] * keep, a + px[3] * keep];
                }
            }
        }
    }

    /// Takes `n` units from the raster budget. False once it is used up.
    fn spend(&mut self, n: usize) -> bool {
        self.budget = self.budget.saturating_sub(n);
        if self.budget == 0 {
            self.exhausted = true;
        }
        !self.exhausted
    }

    /// Adds the horizontal coverage of the span `[xa, xb)` to one strip of a row, and returns the
    /// columns it touched.
    fn span(&self, cov: &mut [f32], xa: f32, xb: f32) -> Option<(usize, usize)> {
        let (xa, xb) = (xa.max(0.0), xb.min(self.w as f32));
        if xb <= xa {
            return None;
        }
        let first = xa.floor() as usize;
        let last = (xb.ceil() as usize).min(self.w);
        for ix in first..last {
            let overlap = xb.min(ix as f32 + 1.0) - xa.max(ix as f32);
            if let Some(c) = cov.get_mut(ix) {
                *c += overlap.max(0.0);
            }
        }
        Some((first, last))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(ty: u32, body: &[u8]) -> Vec<u8> {
        let mut v = ty.to_le_bytes().to_vec();
        v.extend_from_slice(&((body.len() + 8) as u32).to_le_bytes());
        v.extend_from_slice(body);
        v
    }

    fn i32s(v: &[i32]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    /// A 20 x 20 metafile with one filled red rectangle covering its left half.
    fn red_half() -> Vec<u8> {
        let mut header = i32s(&[0, 0, 19, 19, 0, 0, 1000, 1000]);
        header.resize(80, 0);
        let mut out = rec(1, &header);
        out.extend(rec(39, &[&1u32.to_le_bytes()[..], &0u32.to_le_bytes(), &0x0000_00FFu32.to_le_bytes(), &0u32.to_le_bytes()].concat()));
        out.extend(rec(37, &1u32.to_le_bytes()));
        out.extend(rec(37, &0x8000_0008u32.to_le_bytes())); // null pen
        out.extend(rec(43, &i32s(&[0, 0, 10, 20])));
        out.extend(rec(14, &[0; 12]));
        out
    }

    #[test]
    fn a_filled_rectangle_is_drawn_and_the_rest_is_transparent() {
        let png = render_png(&red_half()).unwrap();
        let img = image::load_from_memory(&png).unwrap().to_rgba8();
        assert!(img.width() >= 20);
        let (w, h) = (img.width(), img.height());
        let left = img.get_pixel(w / 4, h / 2).0;
        let right = img.get_pixel(w * 3 / 4, h / 2).0;
        assert_eq!(left, [255, 0, 0, 255]);
        assert_eq!(right[3], 0);
    }

    #[test]
    fn broken_input_is_refused_not_a_crash() {
        assert!(render_png(&[]).is_none());
        assert!(render_png(&[1, 0, 0, 0, 8, 0, 0, 0]).is_none());
        let mut bad = red_half();
        bad.truncate(bad.len() - 30);
        let _ = render_png(&bad);
        let mut huge = red_half();
        huge[16..20].copy_from_slice(&i32::MAX.to_le_bytes());
        let _ = render_png(&huge);
    }

    #[test]
    fn a_flood_of_long_strokes_runs_out_of_budget_instead_of_running_long() {
        // A 1024-wide picture and 150,000 wide diagonal strokes: far more raster work than the
        // budget allows. It must stop (keeping what it drew), which is what keeps this test short.
        let mut header = i32s(&[0, 0, 1023, 1023, 0, 0, 1000, 1000]);
        header.resize(80, 0);
        let mut out = rec(1, &header);
        out.extend(rec(
            38,
            &[&1u32.to_le_bytes()[..], &0u32.to_le_bytes(), &200i32.to_le_bytes(), &0i32.to_le_bytes(), &0x0000_00FFu32.to_le_bytes()].concat(),
        ));
        out.extend(rec(37, &1u32.to_le_bytes()));
        for i in 0..75_000 {
            out.extend(rec(27, &i32s(&[i % 1000, 0])));
            out.extend(rec(54, &i32s(&[1023 - i % 1000, 1023])));
        }
        out.extend(rec(14, &[0; 12]));
        assert!(render_png(&out).is_some());
    }
}
