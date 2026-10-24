//! The print preview's page, painted from the very drawing operations its PDF is written from
//! ([`gridcraft_engine::cmd::print::preview_page`]): what it shows is what prints, with its text,
//! borders, fills, pictures, charts, headers and footers.

use std::sync::{Arc, Mutex, PoisonError, Weak};

use egui::epaint::TextShape;
use egui::text::LayoutJob;
use egui::{Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, TextureHandle, pos2, vec2};
use gridcraft_engine::pdf::{Op, TextOp};

/// One printed page: its size in points and what it draws.
#[derive(Debug)]
pub struct PreviewPage {
    pub width: f32,
    pub height: f32,
    pub ops: Vec<Op>,
}

/// The textures of the page last painted, one per picture, made when first drawn. The page is held
/// weakly: while it is, no later page can be allocated where it was and pass for it.
#[derive(Default)]
struct Textures {
    page: Weak<PreviewPage>,
    by_op: Vec<(usize, TextureHandle)>,
}

fn textures(ctx: &egui::Context) -> Arc<Mutex<Textures>> {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Arc<Mutex<Textures>>>(egui::Id::new("gridcraft.print_preview")).clone())
}

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Paints `page` into `rect` (the paper, its width standing for `page.width` points).
pub fn paint(p: &Painter, rect: Rect, page: &Arc<PreviewPage>, ctx: &egui::Context) {
    let k = rect.width() / page.width.max(1.0);
    let at = |x: f32, y: f32| pos2(rect.left() + x * k, rect.top() + y * k);
    let stroke = |w: f32, c: [u8; 3]| Stroke::new((w * k).max(0.35), rgb(c));
    let tex = textures(ctx);
    let mut tex = tex.lock().unwrap_or_else(PoisonError::into_inner);
    let this = Arc::downgrade(page);
    if !Weak::ptr_eq(&tex.page, &this) {
        *tex = Textures { page: this, by_op: Vec::new() };
    }
    let mut clip = rect;
    let mut saved: Vec<Rect> = Vec::new();
    for (i, op) in page.ops.iter().enumerate() {
        let pc = p.with_clip_rect(clip);
        match op {
            Op::Save => saved.push(clip),
            Op::Restore => clip = saved.pop().unwrap_or(rect),
            Op::Clip { x, y, w, h } => clip = clip.intersect(Rect::from_min_size(at(*x, *y), vec2(*w, *h) * k)),
            Op::FillRect { x, y, w, h, color } => {
                pc.rect_filled(Rect::from_min_size(at(*x, *y), vec2(*w, *h) * k), 0.0, rgb(*color));
            }
            Op::StrokeRect { x, y, w, h, width, color } => {
                pc.rect_stroke(Rect::from_min_size(at(*x, *y), vec2(*w, *h) * k), 0.0, stroke(*width, *color), StrokeKind::Middle);
            }
            Op::Line { pts, width, color, dash } => {
                let points: Vec<Pos2> = pts.iter().map(|q| at(q[0], q[1])).collect();
                let s = stroke(*width, *color);
                match dash_pattern(dash, k) {
                    Some((dashes, gaps)) => pc.extend(Shape::dashed_line_with_offset(&points, s, &dashes, &gaps, 0.0)),
                    None => {
                        pc.add(Shape::line(points, s));
                    }
                }
            }
            Op::Polygon { pts, fill, stroke: line } => {
                let points: Vec<Pos2> = pts.iter().map(|q| at(q[0], q[1])).collect();
                let line = line.map_or(Stroke::NONE, |(c, w)| stroke(w, c));
                match fill {
                    Some(f) => pc.add(Shape::convex_polygon(points, rgb(*f), line)),
                    None => pc.add(Shape::closed_line(points, line)),
                };
            }
            Op::Circle { cx, cy, r, fill, stroke: line } => {
                let line = line.map_or(Stroke::NONE, |(c, w)| stroke(w, c));
                pc.circle(at(*cx, *cy), r * k, fill.map_or(Color32::TRANSPARENT, rgb), line);
            }
            Op::Text(t) => text(&pc, t, at(t.x, t.y), k),
            Op::Image { x, y, w, h, rgba, pw, ph } => {
                let handle = match tex.by_op.iter().find(|(o, _)| *o == i) {
                    Some((_, t)) => Some(t.clone()),
                    None => {
                        let made = texture(ctx, rgba, *pw, *ph);
                        if let Some(t) = &made {
                            tex.by_op.push((i, t.clone()));
                        }
                        made
                    }
                };
                if let Some(t) = handle {
                    let r = Rect::from_min_size(at(*x, *y), vec2(*w, *h) * k);
                    pc.image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
            }
        }
    }
}

/// A PDF dash array (`[on, off, on, off, …]` in points; an odd one repeats, as in PDF) as the
/// dash and gap lengths egui cycles through, scaled by `k`; `None` (draw solid) when there is none
/// or a part would be too short to see.
fn dash_pattern(dash: &[f32], k: f32) -> Option<(Vec<f32>, Vec<f32>)> {
    let times = if dash.len() % 2 == 1 { 2 } else { 1 };
    let all: Vec<f32> = dash.iter().cycle().take(dash.len() * times).map(|d| d * k).collect();
    if all.is_empty() || all.iter().any(|d| d.is_nan() || *d <= 0.25) {
        return None;
    }
    Some((all.iter().step_by(2).copied().collect(), all.iter().skip(1).step_by(2).copied().collect()))
}

/// A picture's texture, scaled down when it is larger than the GPU takes.
fn texture(ctx: &egui::Context, rgba: &[u8], pw: u32, ph: u32) -> Option<TextureHandle> {
    let px = (pw as usize).checked_mul(ph as usize)?;
    if pw == 0 || ph == 0 || rgba.len() < px.checked_mul(4)? {
        return None;
    }
    let max = ctx.input(|i| i.max_texture_side).clamp(64, 4096) as u32;
    let image = if pw.max(ph) > max {
        let full = image::RgbaImage::from_raw(pw, ph, rgba.get(..px * 4)?.to_vec())?;
        let s = max as f32 / pw.max(ph) as f32;
        let small = image::imageops::thumbnail(&full, ((pw as f32 * s) as u32).max(1), ((ph as f32 * s) as u32).max(1));
        egui::ColorImage::from_rgba_unmultiplied([small.width() as usize, small.height() as usize], small.as_raw())
    } else {
        egui::ColorImage::from_rgba_unmultiplied([pw as usize, ph as usize], rgba.get(..px * 4)?)
    };
    Some(ctx.load_texture("print-preview-picture", image, egui::TextureOptions::LINEAR))
}

/// Text with its baseline at `origin`, in the family and style it prints in (an installed family
/// as the cells bind it, else the matching built-in role).
fn text(p: &Painter, t: &TextOp, origin: Pos2, k: f32) {
    let size = t.size * k;
    if size.is_nan() || size < 0.5 || t.text.is_empty() {
        return;
    }
    let color = rgb(t.color);
    let font = FontId::new(size, crate::theme::cell_family(&t.family, t.bold, t.italic));
    let galley = p.layout_job(LayoutJob::simple_singleline(t.text.clone(), font, color));
    let Some(row) = galley.rows.first() else { return };
    let base = row.pos.y + row.row.glyphs.first().map_or(size * 0.8, |g| g.pos.y);
    let (sin, cos) = if t.angle.is_finite() { t.angle.sin_cos() } else { (0.0, 1.0) };
    // The galley's top-left, `base` above the baseline origin, turned with the text.
    let pos = origin + vec2(base * sin, -base * cos);
    p.add(TextShape::new(pos, galley, color).with_angle(t.angle));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_whole_dash_pattern_is_replayed() {
        // Dash-dot: dash 4, gap 1.5, dot 1, gap 1.5, at twice the size.
        assert_eq!(dash_pattern(&[4.0, 1.5, 1.0, 1.5], 2.0), Some((vec![8.0, 2.0], vec![3.0, 3.0])));
        // An odd array repeats, as in PDF: [3] is dash 3, gap 3.
        assert_eq!(dash_pattern(&[3.0], 1.0), Some((vec![3.0], vec![3.0])));
        assert_eq!(dash_pattern(&[], 1.0), None, "solid");
        assert_eq!(dash_pattern(&[4.0, 0.0], 1.0), None, "a part too short to see draws solid");
        assert_eq!(dash_pattern(&[4.0, f32::NAN], 1.0), None);
    }
}
