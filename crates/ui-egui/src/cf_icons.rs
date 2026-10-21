//! Original icons for the Conditional Formatting menu, drawn in code.
//!
//! Each icon is a small worksheet grid with some cells highlighted, plus a symbol that names the
//! rule (a comparison sign, a letter, a calendar, a rank, bars, colours or signal dots). The
//! drawings are this project's own; no vendor artwork is used.

use egui::{Align2, Color32, CornerRadius, Painter, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};

/// Which icon to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CfIcon {
    HighlightRules,
    TopBottom,
    DataBars,
    ColorScales,
    IconSets,
    NewRule,
    ClearRules,
    ManageRules,
    GreaterThan,
    LessThan,
    Between,
    EqualTo,
    TextContains,
    DateOccurring,
    Duplicate,
    Top10,
    Top10Percent,
    Bottom10,
    Bottom10Percent,
    AboveAverage,
    BelowAverage,
    /// A mini data bar in this colour.
    Bar(Color32),
    /// A mini colour scale through these colours (top to bottom).
    Scale([Color32; 3]),
    /// Signal dots in these colours.
    Dots([Color32; 3]),
}

const LINE: Color32 = Color32::from_rgb(118, 118, 118);
const PAPER: Color32 = Color32::WHITE;
const AMBER: Color32 = Color32::from_rgb(250, 196, 120);
const AMBER_DARK: Color32 = Color32::from_rgb(226, 150, 52);
const BLUE: Color32 = Color32::from_rgb(91, 155, 213);
const ROSE: Color32 = Color32::from_rgb(240, 120, 128);
const INK: Color32 = Color32::from_rgb(64, 64, 64);

/// A grid of `cols` x `rows` cells filling `r`; returns each cell's rectangle (row-major).
fn grid(p: &Painter, r: Rect, cols: usize, rows: usize) -> Vec<Rect> {
    p.rect(r, CornerRadius::ZERO, PAPER, Stroke::NONE, StrokeKind::Inside);
    let w = r.width() / cols as f32;
    let h = r.height() / rows as f32;
    let cells = (0..rows)
        .flat_map(|y| (0..cols).map(move |x| Rect::from_min_size(pos2(r.left() + x as f32 * w, r.top() + y as f32 * h), vec2(w, h))))
        .collect();
    let stroke = Stroke::new(1.0, LINE);
    for x in 0..=cols {
        let px = (r.left() + x as f32 * w).round() + 0.5;
        p.line_segment([pos2(px, r.top()), pos2(px, r.bottom())], stroke);
    }
    for y in 0..=rows {
        let py = (r.top() + y as f32 * h).round() + 0.5;
        p.line_segment([pos2(r.left(), py), pos2(r.right(), py)], stroke);
    }
    cells
}

fn fill(p: &Painter, cell: Rect, c: Color32) {
    p.rect_filled(cell.shrink(1.0), CornerRadius::ZERO, c);
}

/// A small white tag in the bottom-right corner holding a symbol.
fn tag(p: &Painter, r: Rect, text: &str) {
    let s = r.width() * 0.5;
    let t = Rect::from_min_size(pos2(r.right() - s, r.bottom() - s), vec2(s, s));
    p.rect(t, CornerRadius::same(1), PAPER, Stroke::new(1.0, INK), StrokeKind::Inside);
    p.text(t.center(), Align2::CENTER_CENTER, text, egui::FontId::proportional(s * 0.78), INK);
}

fn calendar(p: &Painter, r: Rect) {
    let s = r.width() * 0.5;
    let t = Rect::from_min_size(pos2(r.right() - s, r.bottom() - s), vec2(s, s));
    p.rect(t, CornerRadius::same(1), PAPER, Stroke::new(1.0, INK), StrokeKind::Inside);
    p.rect_filled(Rect::from_min_size(t.min, vec2(t.width(), t.height() * 0.28)), CornerRadius::ZERO, INK);
    for (dx, dy) in [(0.3, 0.55), (0.55, 0.55), (0.8, 0.55), (0.3, 0.8), (0.55, 0.8)] {
        p.circle_filled(pos2(t.left() + t.width() * dx - 1.0, t.top() + t.height() * dy), 0.9, INK);
    }
}

fn arrow(p: &Painter, from: Pos2, to: Pos2, c: Color32) {
    p.arrow(from, to - from, Stroke::new(1.4, c));
}

/// Paints `icon` inside `r` (square; 32 points for the big menu items, 16-20 for small ones).
pub fn paint(p: &Painter, r: Rect, icon: CfIcon) {
    let big = r.width() >= 24.0;
    match icon {
        CfIcon::HighlightRules => {
            let cells = grid(p, r, 3, 3);
            fill(p, cells[1], AMBER);
            fill(p, cells[4], AMBER);
            tag(p, r, "≤");
        }
        CfIcon::TopBottom => {
            let cells = grid(p, r, 3, 3);
            for i in [0, 3] {
                fill(p, cells[i], AMBER);
            }
            arrow(p, pos2(cells[1].center().x, cells[4].bottom()), pos2(cells[1].center().x, cells[1].top() + 1.0), INK);
            tag(p, r, "10");
        }
        CfIcon::DataBars => {
            let cells = grid(p, r, 3, 3);
            let full = cells[2].right() - cells[0].left() - 2.0;
            for (row, frac) in [(0usize, 0.45f32), (1, 0.9), (2, 0.25)] {
                let c = cells[row * 3];
                p.rect_filled(Rect::from_min_size(c.min + vec2(1.0, 1.5), vec2(full * frac, c.height() - 3.0)), CornerRadius::ZERO, BLUE);
            }
        }
        CfIcon::ColorScales => {
            let cells = grid(p, r, 3, 3);
            for (row, c) in [(0usize, BLUE), (1, AMBER), (2, ROSE)] {
                for col in 0..3 {
                    fill(p, cells[row * 3 + col], c);
                }
            }
        }
        CfIcon::IconSets => {
            let cells = grid(p, r, 3, 3);
            for (row, c) in [(0usize, Color32::from_rgb(70, 160, 90)), (1, Color32::from_rgb(230, 70, 70)), (2, BLUE)] {
                p.circle_filled(cells[row * 3].center(), cells[0].width() * 0.22, c);
                let line = cells[row * 3 + 1];
                p.line_segment(
                    [pos2(line.left() + 2.0, line.center().y), pos2(cells[row * 3 + 2].right() - 3.0, line.center().y)],
                    Stroke::new(1.0, LINE),
                );
            }
        }
        CfIcon::NewRule | CfIcon::ManageRules => {
            let cells = grid(p, r, 3, 3);
            fill(p, cells[4], AMBER);
            if icon == CfIcon::ManageRules {
                fill(p, cells[0], AMBER);
                fill(p, cells[8], AMBER);
            }
        }
        CfIcon::ClearRules => {
            let cells = grid(p, r, 3, 3);
            fill(p, cells[4], AMBER);
            // An eraser across the corner.
            let c = pos2(r.right() - r.width() * 0.28, r.bottom() - r.height() * 0.28);
            let s = r.width() * 0.22;
            let pts = vec![c + vec2(-s, 0.0), c + vec2(0.0, -s), c + vec2(s, 0.0), c + vec2(0.0, s)];
            p.add(egui::Shape::convex_polygon(pts, Color32::from_rgb(190, 120, 210), Stroke::new(1.0, Color32::from_rgb(120, 60, 140))));
        }
        CfIcon::GreaterThan | CfIcon::LessThan | CfIcon::EqualTo | CfIcon::TextContains => {
            let cells = grid(p, r, 3, 3);
            fill(p, cells[1], AMBER);
            fill(p, cells[4], AMBER);
            let sym = match icon {
                CfIcon::GreaterThan => ">",
                CfIcon::LessThan => "<",
                CfIcon::EqualTo => "=",
                _ => "a",
            };
            tag(p, r, sym);
        }
        CfIcon::Between => {
            let cells = grid(p, r, 3, 3);
            for i in 0..3 {
                fill(p, cells[i], BLUE);
                fill(p, cells[3 + i], AMBER);
                fill(p, cells[6 + i], BLUE);
            }
        }
        CfIcon::DateOccurring => {
            let cells = grid(p, r, 3, 3);
            fill(p, cells[1], AMBER);
            fill(p, cells[4], AMBER);
            calendar(p, r);
        }
        CfIcon::Duplicate => {
            let cells = grid(p, r, 3, 3);
            fill(p, cells[1], AMBER);
            fill(p, cells[4], AMBER);
            fill(p, cells[7], AMBER);
            p.rect_stroke(cells[1].union(cells[7]).shrink(0.5), CornerRadius::ZERO, Stroke::new(1.0, AMBER_DARK), StrokeKind::Inside);
        }
        CfIcon::Top10 | CfIcon::Top10Percent | CfIcon::Bottom10 | CfIcon::Bottom10Percent => {
            let cells = grid(p, r, 3, 3);
            let top = matches!(icon, CfIcon::Top10 | CfIcon::Top10Percent);
            for i in if top { [0, 1, 2] } else { [6, 7, 8] } {
                fill(p, cells[i], AMBER);
            }
            tag(p, r, if matches!(icon, CfIcon::Top10Percent | CfIcon::Bottom10Percent) { "%" } else { "10" });
        }
        CfIcon::AboveAverage | CfIcon::BelowAverage => {
            let cells = grid(p, r, 3, 3);
            let above = icon == CfIcon::AboveAverage;
            for i in if above { [0, 1, 2] } else { [6, 7, 8] } {
                fill(p, cells[i], AMBER);
            }
            let y = r.center().y;
            p.line_segment([pos2(r.left(), y), pos2(r.right(), y)], Stroke::new(1.6, Color32::from_rgb(200, 60, 60)));
        }
        CfIcon::Bar(c) => {
            let cells = grid(p, r, 1, 3);
            for (i, frac) in [(0usize, 0.5f32), (1, 0.85), (2, 0.3)] {
                let cell = cells[i];
                p.rect_filled(
                    Rect::from_min_size(cell.min + vec2(1.5, 1.5), vec2((cell.width() - 3.0) * frac, cell.height() - 3.0)),
                    CornerRadius::ZERO,
                    c,
                );
            }
        }
        CfIcon::Scale(cs) => {
            let cells = grid(p, r, 1, 3);
            for (i, c) in cs.iter().enumerate() {
                fill(p, cells[i], *c);
            }
        }
        CfIcon::Dots(cs) => {
            let cells = grid(p, r, 1, 3);
            for (i, c) in cs.iter().enumerate() {
                p.circle_filled(pos2(cells[i].left() + cells[i].height() * 0.6, cells[i].center().y), cells[i].height() * 0.3, *c);
            }
        }
    }
    if !big {
        p.rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.0, LINE), StrokeKind::Inside);
    }
}
