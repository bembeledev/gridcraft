//! Writes primitives as an SVG document (Save as Picture). Text stays text, set in a sans-serif
//! with Helvetica metrics (what the PDF export measures with), so it stays sharp at any size.

use std::f32::consts::{PI, TAU};
use std::fmt::Write as _;

use crate::{HAlign, Prim, Rgba, VAlign};

const FONT: &str = "Helvetica, Arial, sans-serif";

/// The primitives of a `w` × `h` chart as a standalone SVG document.
pub fn to_svg(prims: &[Prim], w: f32, h: f32) -> String {
    let (w, h) = (num(w), num(h));
    let mut out = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">"#);
    out.push('\n');
    for p in prims {
        // Writing to a String can't fail.
        let _ = match p {
            Prim::Rect { x, y, w, h, fill, stroke, radius } => {
                // A negative size (a bar below the axis) spans back from the anchor.
                let (x, w) = if *w < 0.0 { (x + w, -w) } else { (*x, *w) };
                let (y, h) = if *h < 0.0 { (y + h, -h) } else { (*y, *h) };
                let rx = if *radius > 0.0 { format!(r#" rx="{}""#, num(*radius)) } else { String::new() };
                writeln!(
                    out,
                    r#"<rect x="{}" y="{}" width="{}" height="{}"{rx}{}{}/>"#,
                    num(x),
                    num(y),
                    num(w),
                    num(h),
                    fill_of(*fill),
                    stroke_of(*stroke)
                )
            }
            Prim::Line { pts, color, width, dash } => {
                let dash = if *dash { r#" stroke-dasharray="3 2""# } else { "" };
                writeln!(out, r#"<polyline points="{}" fill="none"{}{dash}/>"#, points(pts), stroke_of(Some((*color, *width))))
            }
            Prim::Polygon { pts, fill, stroke } => {
                writeln!(out, r#"<polygon points="{}"{}{}/>"#, points(pts), fill_of(Some(*fill)), stroke_of(*stroke))
            }
            Prim::Wedge { cx, cy, r_outer, r_inner, a0, a1, fill, stroke } => {
                writeln!(
                    out,
                    r#"<path d="{}" fill-rule="evenodd"{}{}/>"#,
                    wedge(*cx, *cy, *r_outer, *r_inner, *a0, *a1),
                    fill_of(Some(*fill)),
                    stroke_of(*stroke)
                )
            }
            Prim::Circle { cx, cy, r, fill, stroke } => {
                writeln!(out, r#"<circle cx="{}" cy="{}" r="{}"{}{}/>"#, num(*cx), num(*cy), num(r.abs()), fill_of(*fill), stroke_of(*stroke))
            }
            Prim::Text { x, y, text, size, color, bold, align, valign, rotation } => {
                let anchor = match align {
                    HAlign::Left => "start",
                    HAlign::Center => "middle",
                    HAlign::Right => "end",
                };
                // Baseline offsets from the anchor, as the PDF export places text.
                let dy = match valign {
                    VAlign::Top => size * 0.78,
                    VAlign::Middle => size * 0.35,
                    VAlign::Bottom => -size * 0.21,
                };
                let rotate =
                    if rotation.is_finite() && *rotation != 0.0 { format!(" rotate({})", num(rotation.to_degrees())) } else { String::new() };
                let weight = if *bold { r#" font-weight="bold""# } else { "" };
                writeln!(
                    out,
                    r#"<text xml:space="preserve" transform="translate({} {}){rotate}" y="{}" font-family="{FONT}" font-size="{}"{weight} text-anchor="{anchor}"{}>{}</text>"#,
                    num(*x),
                    num(*y),
                    num(dy),
                    num(*size),
                    fill_of(Some(*color)),
                    escape(text)
                )
            }
        };
    }
    out.push_str("</svg>\n");
    out
}

/// A coordinate with at most two decimals; anything not finite is 0.
fn num(v: f32) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

fn color(attr: &str, c: Rgba) -> String {
    let [r, g, b, a] = c;
    let mut s = format!(r##" {attr}="#{r:02x}{g:02x}{b:02x}""##);
    if a < 255 {
        s.push_str(&format!(r#" {attr}-opacity="{}""#, num(f32::from(a) / 255.0)));
    }
    s
}

fn fill_of(fill: Option<Rgba>) -> String {
    fill.map_or_else(|| r#" fill="none""#.into(), |c| color("fill", c))
}

fn stroke_of(stroke: Option<(Rgba, f32)>) -> String {
    stroke.map_or_else(String::new, |(c, w)| format!(r#"{} stroke-width="{}""#, color("stroke", c), num(w.max(0.0))))
}

fn points(pts: &[[f32; 2]]) -> String {
    pts.iter().map(|p| format!("{},{}", num(p[0]), num(p[1]))).collect::<Vec<_>>().join(" ")
}

/// A pie or doughnut sector. Angles are radians from 12 o'clock, clockwise (the SVG sweep).
fn wedge(cx: f32, cy: f32, r_outer: f32, r_inner: f32, a0: f32, a1: f32) -> String {
    let at = |a: f32, r: f32| format!("{} {}", num(cx + r * a.sin()), num(cy - r * a.cos()));
    let (ro, ri) = (r_outer.abs(), r_inner.abs());
    let span = a1 - a0;
    if !span.is_finite() {
        return String::new();
    }
    if span.abs() >= TAU - 1e-4 {
        // A whole ring: two half arcs each way (one arc can't start and end at the same point).
        let ring = |r: f32| format!("M {} A {r} {r} 0 1 1 {} A {r} {r} 0 1 1 {} Z", at(0.0, r), at(PI, r), at(0.0, r), r = num(r));
        return if ri > 0.0 { format!("{} {}", ring(ro), ring(ri)) } else { ring(ro) };
    }
    let large = u8::from(span.abs() > PI);
    let (sweep, back) = if span >= 0.0 { (1, 0) } else { (0, 1) };
    let outer = format!("{} A {r} {r} 0 {large} {sweep} {}", at(a0, ro), at(a1, ro), r = num(ro));
    if ri > 0.0 {
        format!("M {outer} L {} A {r} {r} 0 {large} {back} {} Z", at(a1, ri), at(a0, ri), r = num(ri))
    } else {
        format!("M {} {} L {outer} Z", num(cx), num(cy))
    }
}

/// Text content with the five XML special characters escaped; control characters, which XML
/// can't hold, are dropped.
fn escape(text: &str) -> String {
    let mut s = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => s.push_str("&amp;"),
            '<' => s.push_str("&lt;"),
            '>' => s.push_str("&gt;"),
            '"' => s.push_str("&quot;"),
            '\'' => s.push_str("&apos;"),
            c if c.is_control() && c != '\t' && c != '\n' => {}
            c => s.push(c),
        }
    }
    s
}
