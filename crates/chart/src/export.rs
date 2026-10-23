//! Standalone chart SVG and lossless PNG, including text. No worksheet chrome.
use crate::{Chart, ChartData, HAlign, Measure, Prim, Rgba, VAlign, render};
use std::fmt::Write as _;

fn esc(text: &str) -> String {
    text.chars()
        .filter(|c| *c == '\n' || *c == '\r' || *c == '\t' || (*c >= ' ' && *c != '\u{fffe}' && *c != '\u{ffff}'))
        .map(|c| match c {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '"' => "&quot;".into(),
            '\'' => "&apos;".into(),
            _ => c.to_string(),
        })
        .collect()
}
fn color(c: Rgba) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}
fn fill(c: Option<Rgba>) -> String {
    c.map(|c| format!("fill=\"{}\" fill-opacity=\"{}\"", color(c), c[3] as f32 / 255.0)).unwrap_or_else(|| "fill=\"none\"".into())
}
fn stroke(s: Option<(Rgba, f32)>) -> String {
    s.filter(|(_, w)| *w > 0.0)
        .map(|(c, w)| format!("stroke=\"{}\" stroke-opacity=\"{}\" stroke-width=\"{w}\"", color(c), c[3] as f32 / 255.0))
        .unwrap_or_else(|| "stroke=\"none\"".into())
}

pub fn export_svg(chart: &Chart, data: &ChartData, measure: &dyn Measure, scale: f32, transparent: bool) -> Result<String, String> {
    let (w, h) = (chart.anchor.width, chart.anchor.height);
    if !w.is_finite() || !h.is_finite() || w < 1.0 || h < 1.0 || w > 1e5 || h > 1e5 || !scale.is_finite() || !(0.25..=8.0).contains(&scale) {
        return Err("Invalid chart export size".into());
    }
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {w} {h}\"><defs><clipPath id=\"chartClip\"><rect width=\"{w}\" height=\"{h}\"/></clipPath></defs><g clip-path=\"url(#chartClip)\">",
        w * scale,
        h * scale
    );
    for (index, mut primitive) in render(chart, data, w, h, measure).into_iter().enumerate() {
        if index == 0
            && transparent
            && let Prim::Rect { fill, .. } = &mut primitive
        {
            *fill = None;
        }
        match primitive {
            Prim::Rect { x, y, w, h, fill: f, stroke: s, radius } => {
                let _ = write!(svg, "<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" rx=\"{radius}\" {} {}/>", fill(f), stroke(s));
            }
            Prim::Circle { cx, cy, r, fill: f, stroke: s } => {
                let _ = write!(svg, "<circle cx=\"{cx}\" cy=\"{cy}\" r=\"{r}\" {} {}/>", fill(f), stroke(s));
            }
            Prim::Line { pts, color: c, width, dash } => {
                let points = pts.iter().map(|p| format!("{},{}", p[0], p[1])).collect::<Vec<_>>().join(" ");
                let _ = write!(
                    svg,
                    "<polyline points=\"{points}\" fill=\"none\" {} {}/>",
                    stroke(Some((c, width))),
                    if dash { "stroke-dasharray=\"6 4\"" } else { "" }
                );
            }
            Prim::Polygon { pts, fill: f, stroke: s } => {
                let points = pts.iter().map(|p| format!("{},{}", p[0], p[1])).collect::<Vec<_>>().join(" ");
                let _ = write!(svg, "<polygon points=\"{points}\" {} {}/>", fill(Some(f)), stroke(s));
            }
            Prim::Wedge { cx, cy, r_outer, r_inner, a0, a1, fill: f, stroke: s } => {
                let at = |a: f32, r: f32| (cx + r * a.sin(), cy - r * a.cos());
                let n = ((a1 - a0).abs() / std::f32::consts::PI).ceil().clamp(1.0, 4.0) as usize;
                let (x, y) = at(a0, r_outer);
                let mut path = format!("M{x} {y}");
                let clockwise = usize::from(a1 >= a0);
                for i in 1..=n {
                    let (x, y) = at(a0 + (a1 - a0) * i as f32 / n as f32, r_outer);
                    let _ = write!(path, " A{r_outer} {r_outer} 0 0 {clockwise} {x} {y}");
                }
                if r_inner > 0.0 {
                    let (x, y) = at(a1, r_inner);
                    let _ = write!(path, " L{x} {y}");
                    for i in 1..=n {
                        let (x, y) = at(a1 - (a1 - a0) * i as f32 / n as f32, r_inner);
                        let _ = write!(path, " A{r_inner} {r_inner} 0 0 {} {x} {y}", 1 - clockwise);
                    }
                } else {
                    let _ = write!(path, " L{cx} {cy}");
                }
                let _ = write!(svg, "<path d=\"{path} Z\" {} {}/>", fill(Some(f)), stroke(s));
            }
            Prim::Text { x, y, text, size, color: c, bold, format, align, valign, rotation, .. } => {
                let _ = write!(
                    svg,
                    "<text x=\"{x}\" y=\"{y}\" font-family=\"{}\" font-size=\"{size}\" font-weight=\"{}\" font-style=\"{}\" text-anchor=\"{}\" dominant-baseline=\"{}\" transform=\"rotate({} {x} {y})\" {}>{}</text>",
                    esc(format.name.as_deref().unwrap_or("sans-serif")),
                    if bold { "bold" } else { "normal" },
                    if format.italic.unwrap_or(false) { "italic" } else { "normal" },
                    match align {
                        HAlign::Left => "start",
                        HAlign::Center => "middle",
                        HAlign::Right => "end",
                    },
                    match valign {
                        VAlign::Top => "text-before-edge",
                        VAlign::Middle => "central",
                        VAlign::Bottom => "text-after-edge",
                    },
                    rotation.to_degrees(),
                    fill(Some(c)),
                    esc(&text)
                );
            }
        }
    }
    svg.push_str("</g></svg>");
    Ok(svg)
}

pub fn export_png(chart: &Chart, data: &ChartData, measure: &dyn Measure, scale: f32, transparent: bool) -> Result<Vec<u8>, String> {
    let svg = export_svg(chart, data, measure, scale, transparent)?;
    let width = (chart.anchor.width * scale).ceil() as u32;
    let height = (chart.anchor.height * scale).ceil() as u32;
    if width > 8192 || height > 8192 || u64::from(width) * u64::from(height) > 16_777_216 {
        return Err("PNG export is limited to 16 megapixels and 8192 pixels per side".into());
    }
    let mut options = resvg::usvg::Options::default();
    #[cfg(not(target_arch = "wasm32"))]
    options.fontdb_mut().load_system_fonts();
    if options.fontdb.faces().next().is_none() {
        return Err("PNG export needs installed fonts; use SVG on this platform".into());
    }
    let family = ["DejaVu Sans", "Arial", "Liberation Sans", "Helvetica", "Noto Sans"]
        .into_iter()
        .find(|name| options.fontdb.faces().any(|face| face.families.iter().any(|(f, _)| f == name)))
        .map(str::to_owned)
        .or_else(|| options.fontdb.faces().find_map(|face| face.families.first().map(|(name, _)| name.clone())));
    if let Some(family) = family {
        options.fontdb_mut().set_sans_serif_family(family);
    }
    let tree = resvg::usvg::Tree::from_str(&svg, &options).map_err(|e| e.to_string())?;
    let mut pixels = resvg::tiny_skia::Pixmap::new(width, height).ok_or("Could not allocate PNG image")?;
    resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pixels.as_mut());
    pixels.encode_png().map_err(|e| e.to_string())
}
