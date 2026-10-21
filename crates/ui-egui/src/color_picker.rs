//! Colour picking in the desktop layout.
//!
//! - [`palette`]: the drop-down palette — "High-contrast only" switch, Theme Colors (a row of
//!   base colours over columns of tints and shades), Standard Colors, the no-colour row
//!   ("No Fill" / "Automatic"), and More Colors….
//! - [`window`]: the Colors window opened by More Colors…, with a Standard tab (a honeycomb of
//!   colours and a grey scale) and a Custom tab (spectrum, lightness bar, RGB and hex), New and
//!   Current swatches, OK and Cancel. Its result goes back to the palette that opened it through
//!   [`take_result`], keyed by the caller's `key`.
//!
//! Layout and artwork are this project's own.

use egui::{Align2, Color32, CornerRadius, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::theme::{self, Tokens};

const SWATCH: f32 = 22.0;
const GAP: f32 = 6.0;
const COLS: usize = 10;
const TINTS: [f64; 5] = [0.8, 0.6, 0.4, -0.25, -0.5];
const STANDARD: [&str; 10] = ["#C00000", "#FF0000", "#FFC000", "#FFFF00", "#92D050", "#00B050", "#00B0F0", "#0070C0", "#002060", "#7030A0"];
const HOVER: Color32 = Color32::from_rgb(0xE8, 0x7D, 0x2B);

fn hex_of(c: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())
}

fn parse_hex(s: &str) -> Option<Color32> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

fn theme_color(base: u32, tint: f64) -> Color32 {
    let rgb = [(base >> 16) as u8, (base >> 8) as u8, base as u8];
    let c = gridcraft_engine::model::style::apply_tint(rgb, tint);
    Color32::from_rgb(c[0], c[1], c[2])
}

/// The five tint/shade steps under a theme colour. Very light colours get darker shades and very
/// dark ones lighter tints, so every column shows visible steps.
fn tints_for(base: u32) -> [f64; 5] {
    let (r, g, b) = (f64::from((base >> 16) as u8), f64::from((base >> 8) as u8), f64::from(base as u8));
    let lum = (0.299 * r + 0.587 * g + 0.114 * b) / 255.0;
    if lum > 0.9 {
        [-0.05, -0.15, -0.25, -0.35, -0.5]
    } else if lum < 0.1 {
        [0.5, 0.35, 0.25, 0.15, 0.05]
    } else {
        TINTS
    }
}

fn high_contrast_id() -> Id {
    Id::new("nux_color_high_contrast")
}

fn more_id() -> Id {
    Id::new("nux_more_colors")
}

fn result_id(key: &str) -> Id {
    Id::new(("nux_more_colors_result", key))
}

/// State of the Colors window while it is open.
#[derive(Clone, Debug, Default)]
struct MoreColors {
    key: String,
    current: Option<Color32>,
    new: Color32,
    custom: bool,
    hex: String,
}

/// The colour chosen in a Colors window opened by the palette with this `key`, once. The answer
/// is kept for the frame OK was clicked in and the next one only: a caller that isn't drawn by
/// then (its menu or dialog closed) never receives it later.
pub fn take_result(ctx: &egui::Context, key: &str) -> Option<String> {
    let now = ctx.cumulative_frame_nr();
    let (color, made) = ctx.data_mut(|d| d.remove_temp::<(String, u64)>(result_id(key)))?;
    (now <= made.saturating_add(1)).then_some(color)
}

/// True while the Colors window (More Colors…) is open.
pub fn is_open(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<MoreColors>(more_id()).is_some())
}

/// A swatch; returns true when clicked.
fn swatch(ui: &mut Ui, r: Rect, c: Color32, highlight: bool, label: &str) -> bool {
    let resp = ui.interact(r, ui.id().with(("swatch", r.min.x as i32, r.min.y as i32)), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    ui.painter().rect_filled(r, CornerRadius::ZERO, c);
    ui.painter().rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.0, Color32::from_gray(214)), StrokeKind::Inside);
    if resp.hovered() || highlight {
        ui.painter().rect_stroke(r.expand(1.0), CornerRadius::ZERO, Stroke::new(2.0, HOVER), StrokeKind::Outside);
        ui.painter().rect_stroke(r.shrink(1.0), CornerRadius::ZERO, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
    }
    resp.clicked()
}

fn heading(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    let lang = crate::i18n::current(ui.ctx());
    ui.add_space(6.0);
    ui.label(egui::RichText::new(lang.tr(text)).font(theme::ui_bold(13.5)).color(t.text));
    ui.add_space(6.0);
}

/// A full-width menu row with an access-key label; returns true when clicked.
fn row(ui: &mut Ui, label: &str, icon: impl FnOnce(&egui::Painter, Rect)) -> bool {
    let t = Tokens::get(ui.ctx());
    let lang = crate::i18n::current(ui.ctx());
    let width = COLS as f32 * SWATCH + (COLS - 1) as f32 * GAP;
    let (r, resp) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click());
    let plain = label.replace('&', "");
    let shown = lang.tr(&plain);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, shown));
    if resp.hovered() {
        ui.painter().rect_filled(r.expand2(vec2(6.0, 0.0)), CornerRadius::same(2), t.hover);
    }
    icon(ui.painter(), Rect::from_center_size(pos2(r.left() + 12.0, r.center().y), vec2(20.0, 20.0)));
    let job = if shown == plain {
        crate::border_preview::access_key_job(label, theme::ui_font(13.5), t.text)
    } else {
        egui::text::LayoutJob::simple_singleline(shown.to_string(), theme::ui_font(13.5), t.text)
    };
    let g = ui.painter().layout_job(job);
    ui.painter().galley(pos2(r.left() + 36.0, r.center().y - g.size().y / 2.0), g, t.text);
    resp.clicked()
}

/// The small painter's-palette icon of More Colors….
fn palette_icon(p: &egui::Painter, r: Rect) {
    let c = r.center();
    p.circle(c, r.width() * 0.45, Color32::from_rgb(250, 240, 220), Stroke::new(1.0, Color32::from_gray(90)));
    for (dx, dy, col) in [
        (-0.2f32, -0.2f32, Color32::from_rgb(220, 50, 50)),
        (0.05, -0.27, Color32::from_rgb(240, 180, 30)),
        (0.25, -0.1, Color32::from_rgb(60, 160, 70)),
        (0.22, 0.15, Color32::from_rgb(40, 110, 210)),
        (-0.25, 0.08, Color32::from_rgb(140, 60, 170)),
    ] {
        p.circle_filled(c + vec2(dx, dy) * r.width(), r.width() * 0.08, col);
    }
    p.circle_filled(c + vec2(0.0, 0.22) * r.width(), r.width() * 0.09, Color32::WHITE);
}

/// The no-colour square in front of "No Fill" / "Automatic".
fn empty_icon(p: &egui::Painter, r: Rect, automatic: bool) {
    let b = r.shrink(2.0);
    if automatic {
        p.rect_filled(b, CornerRadius::ZERO, Color32::BLACK);
    } else {
        p.rect(b, CornerRadius::ZERO, Color32::from_gray(245), Stroke::new(1.0, Color32::from_gray(200)), StrokeKind::Inside);
    }
}

/// The drop-down palette. Returns the hex colour picked, or `"none"` for the no-colour row.
/// `key` identifies the caller so that a colour chosen in the Colors window (More Colors…)
/// can be collected with [`take_result`].
pub fn palette(ui: &mut Ui, theme_colors: &[u32; 12], none_label: &str, key: &str) -> Option<String> {
    let t = Tokens::get(ui.ctx());
    let lang = crate::i18n::current(ui.ctx());
    let mut picked = None;
    let mut high_contrast: bool = ui.ctx().data_mut(|d| d.get_persisted(high_contrast_id())).unwrap_or(false);
    let width = COLS as f32 * SWATCH + (COLS - 1) as f32 * GAP;
    ui.set_width(width);
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);

    // Keyboard: arrows move a highlight (row 0 theme, 1-5 tints, 6 standard), Enter applies.
    let pal_id = ui.id().with("__color_palette");
    let mut cur: Option<(usize, usize)> = ui.data_mut(|d| d.get_temp(pal_id)).unwrap_or(None);
    let keys = ui.input(|i| {
        [egui::Key::ArrowLeft, egui::Key::ArrowRight, egui::Key::ArrowUp, egui::Key::ArrowDown, egui::Key::Enter].map(|k| i.key_pressed(k))
    });
    // The Colors window owns the keyboard while it is open (its Hex field, Enter for OK), and
    // Enter with no highlighted swatch belongs to whatever holds the palette (OK in a dialog).
    let keys = if is_open(ui.ctx()) { [false; 5] } else { keys };
    let keys = [keys[0], keys[1], keys[2], keys[3], keys[4] && cur.is_some()];
    let rows: Vec<usize> = if high_contrast { vec![0, 6] } else { vec![0, 1, 2, 3, 4, 5, 6] };
    // High-contrast only hides the tint rows: a highlight left on one of them is dropped, so Enter
    // can't apply a colour the palette no longer shows.
    if high_contrast && cur.is_some_and(|(r, _)| (1..=5).contains(&r)) {
        cur = None;
    }
    if keys[..4].iter().any(|k| *k) {
        cur = Some(match cur {
            None => (0, 0),
            Some((r, c)) => {
                let ri = rows.iter().position(|x| *x == r).unwrap_or(0);
                let r2 = if keys[2] {
                    rows[ri.saturating_sub(1)]
                } else if keys[3] {
                    rows[(ri + 1).min(rows.len() - 1)]
                } else {
                    r
                };
                let c2 = if keys[0] {
                    (c + COLS - 1) % COLS
                } else if keys[1] {
                    (c + 1) % COLS
                } else {
                    c
                };
                (r2, c2)
            }
        });
    }
    let (arrows, enter) = (keys[..4].iter().any(|k| *k), keys[4]);
    if arrows || enter {
        ui.ctx().input_mut(|i| {
            i.events.retain(|e| match e {
                egui::Event::Key {
                    key: egui::Key::ArrowLeft | egui::Key::ArrowRight | egui::Key::ArrowUp | egui::Key::ArrowDown,
                    pressed: true,
                    ..
                } => !arrows,
                egui::Event::Key { key: egui::Key::Enter, pressed: true, .. } => !enter,
                _ => true,
            });
        });
    }
    let color_at = |r: usize, c: usize| -> Color32 {
        let base = theme_colors.get(c).copied().unwrap_or(0);
        match r {
            0 => theme_color(base, 0.0),
            1..=5 => theme_color(base, tints_for(base)[r - 1]),
            _ => parse_hex(STANDARD[c]).unwrap_or(Color32::BLACK),
        }
    };
    if keys[4]
        && let Some((r, c)) = cur
    {
        picked = Some(hex_of(color_at(r, c)));
    }

    // High-contrast only.
    ui.horizontal(|ui| {
        ui.set_width(width);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (r, resp) = ui.allocate_exact_size(vec2(64.0, 28.0), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, high_contrast, "High-contrast only"));
            ui.painter().rect(r, CornerRadius::same(14), Color32::TRANSPARENT, Stroke::new(1.0, t.text_dim), StrokeKind::Inside);
            let knob = if high_contrast { pos2(r.right() - 14.0, r.center().y) } else { pos2(r.left() + 14.0, r.center().y) };
            ui.painter().circle_filled(knob, 8.0, if high_contrast { t.accent } else { t.text_dim });
            ui.painter().text(
                pos2(if high_contrast { r.left() + 10.0 } else { r.right() - 10.0 }, r.center().y),
                if high_contrast { Align2::LEFT_CENTER } else { Align2::RIGHT_CENTER },
                lang.tr(if high_contrast { "On" } else { "Off" }),
                theme::ui_font(12.5),
                t.text,
            );
            if resp.clicked() {
                high_contrast = !high_contrast;
                ui.ctx().data_mut(|d| d.insert_persisted(high_contrast_id(), high_contrast));
            }
            ui.add_space(8.0);
            ui.label(crate::border_preview::access_key_job("&High-contrast only", theme::ui_font(13.5), t.text));
        });
    });
    ui.add_space(6.0);
    ui.separator();

    heading(ui, "Theme Colors");
    let names = ["Background 1", "Text 1", "Background 2", "Text 2", "Accent 1", "Accent 2", "Accent 3", "Accent 4", "Accent 5", "Accent 6"];
    let (row0, _) = ui.allocate_exact_size(vec2(width, SWATCH), Sense::hover());
    for (c, name) in names.iter().enumerate() {
        let r = Rect::from_min_size(pos2(row0.left() + c as f32 * (SWATCH + GAP), row0.top()), vec2(SWATCH, SWATCH));
        if swatch(ui, r, color_at(0, c), cur == Some((0, c)), name) {
            picked = Some(hex_of(color_at(0, c)));
        }
    }
    if !high_contrast {
        ui.add_space(14.0);
        let (block, _) = ui.allocate_exact_size(vec2(width, SWATCH * 5.0), Sense::hover());
        for (c, name) in names.iter().enumerate() {
            for k in 0..5 {
                let r = Rect::from_min_size(pos2(block.left() + c as f32 * (SWATCH + GAP), block.top() + k as f32 * SWATCH), vec2(SWATCH, SWATCH));
                let pct = tints_for(theme_colors.get(c).copied().unwrap_or(0))[k];
                let label = if pct > 0.0 {
                    format!("{name}, Lighter {}%", (pct * 100.0).round())
                } else {
                    format!("{name}, Darker {}%", (-pct * 100.0).round())
                };
                if swatch(ui, r, color_at(k + 1, c), cur == Some((k + 1, c)), &label) {
                    picked = Some(hex_of(color_at(k + 1, c)));
                }
            }
        }
    }
    ui.add_space(8.0);
    ui.separator();
    heading(ui, "Standard Colors");
    let std_names = ["Dark Red", "Red", "Orange", "Yellow", "Light Green", "Green", "Light Blue", "Blue", "Dark Blue", "Purple"];
    let (row6, _) = ui.allocate_exact_size(vec2(width, SWATCH), Sense::hover());
    for (c, name) in std_names.iter().enumerate() {
        let r = Rect::from_min_size(pos2(row6.left() + c as f32 * (SWATCH + GAP), row6.top()), vec2(SWATCH, SWATCH));
        if swatch(ui, r, color_at(6, c), cur == Some((6, c)), name) {
            picked = Some(hex_of(color_at(6, c)));
        }
    }
    ui.add_space(8.0);
    ui.separator();
    let automatic = none_label == "Automatic";
    let none_key = if automatic { "&Automatic".to_string() } else { format!("&{none_label}") };
    if row(ui, &none_key, |p, r| empty_icon(p, r, automatic)) {
        picked = Some("none".into());
    }
    ui.separator();
    if row(ui, "&More Colors…", palette_icon) {
        let start = cur.map(|(r, c)| color_at(r, c)).unwrap_or(Color32::WHITE);
        ui.ctx().data_mut(|d| {
            d.insert_temp(more_id(), MoreColors { key: key.to_string(), current: None, new: start, custom: false, hex: hex_of(start) })
        });
        if ui.stack().iter().any(|s| matches!(s.info.kind, Some(egui::UiKind::Menu | egui::UiKind::Popup))) {
            ui.close();
        }
    }
    if picked.is_some() {
        ui.data_mut(|d| d.remove_temp::<Option<(usize, usize)>>(pal_id));
    } else {
        ui.data_mut(|d| d.insert_temp(pal_id, cur));
    }
    picked
}

// ---- the Colors window ----------------------------------------------------------------------------

/// Axial hex coordinates of the honeycomb (radius 6: 127 cells).
fn honeycomb() -> Vec<(i32, i32)> {
    let n: i32 = 6;
    let mut out = Vec::new();
    for r in -n..=n {
        for q in -n..=n {
            if (q + r).abs() <= n {
                out.push((q, r));
            }
        }
    }
    out
}

/// The colour of a honeycomb cell: white at the centre, pure hues around the middle ring,
/// darker towards the rim.
fn honey_color(q: i32, r: i32) -> Color32 {
    let d = q.abs().max(r.abs()).max((q + r).abs()) as f32;
    if d == 0.0 {
        return Color32::WHITE;
    }
    let x = q as f32 + r as f32 / 2.0;
    let y = r as f32 * 0.866;
    // Clockwise from the top: blue, magenta, red, orange, yellow, green, cyan.
    let hue = (y.atan2(x).to_degrees() + 360.0 + 330.0) % 360.0 / 360.0;
    let (s, v) = if d <= 3.0 { (d / 3.0, 1.0) } else { (1.0, 1.0 - (d - 3.0) * 0.2) };
    egui::ecolor::Hsva::new(hue, s, v, 1.0).into()
}

fn hexagon(center: Pos2, size: f32) -> Vec<Pos2> {
    (0..6)
        .map(|i| {
            let a = (60.0 * i as f32 + 30.0).to_radians();
            center + vec2(a.cos(), a.sin()) * size
        })
        .collect()
}

fn standard_tab(ui: &mut Ui, st: &mut MoreColors) {
    let lang = crate::i18n::current(ui.ctx());
    let t = Tokens::get(ui.ctx());
    ui.label(crate::border_preview::access_key_job(&format!("&{}", lang.tr("Colors:")), theme::ui_font(13.0), t.text));
    let (area, _) = ui.allocate_exact_size(vec2(240.0, 210.0), Sense::hover());
    let size = 9.0;
    let center = pos2(area.center().x, area.top() + 100.0);
    for (q, r) in honeycomb() {
        let p = center + vec2(size * 1.732 * (q as f32 + r as f32 / 2.0), size * 1.5 * r as f32);
        let c = honey_color(q, r);
        let cell = Rect::from_center_size(p, vec2(size * 1.7, size * 1.7));
        let resp = ui.interact(cell, ui.id().with(("honey", q, r)), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, hex_of(c)));
        ui.painter().add(egui::Shape::convex_polygon(hexagon(p, size), c, Stroke::NONE));
        if st.new == c || resp.hovered() {
            ui.painter().add(egui::Shape::closed_line(hexagon(p, size), Stroke::new(1.5, Color32::BLACK)));
        }
        if resp.clicked() {
            st.new = c;
            st.hex = hex_of(c);
        }
    }
    // Grey scale: white, 13 greys, black.
    let y = area.bottom() - 6.0;
    for k in 0..15 {
        let g = 255 - (k * 255 / 14) as u8;
        let c = Color32::from_gray(g);
        let big = k == 0 || k == 14;
        let s = if big { 14.0 } else { 7.0 };
        let x = area.left()
            + 12.0
            + if big && k == 14 {
                222.0
            } else if k == 0 {
                0.0
            } else {
                26.0 + k as f32 * 13.0
            };
        let p = pos2(x, y);
        let cell = Rect::from_center_size(p, vec2(s * 1.8, s * 1.8));
        let resp = ui.interact(cell, ui.id().with(("grey", k)), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, hex_of(c)));
        ui.painter().add(egui::Shape::convex_polygon(hexagon(p, s), c, Stroke::new(1.0, Color32::from_gray(120))));
        if st.new == c {
            ui.painter().add(egui::Shape::closed_line(hexagon(p, s), Stroke::new(1.5, Color32::BLACK)));
        }
        if resp.clicked() {
            st.new = c;
            st.hex = hex_of(c);
        }
    }
}

fn custom_tab(ui: &mut Ui, st: &mut MoreColors) {
    let lang = crate::i18n::current(ui.ctx());
    let t = Tokens::get(ui.ctx());
    ui.label(crate::border_preview::access_key_job(&format!("&{}", lang.tr("Colors:")), theme::ui_font(13.0), t.text));
    let mut hsva = egui::ecolor::Hsva::from(st.new);
    ui.horizontal(|ui| {
        // Spectrum: hue across, saturation down (full at the top).
        let (sp, sresp) = ui.allocate_exact_size(vec2(200.0, 130.0), Sense::click_and_drag());
        sresp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, "Spectrum"));
        let mut mesh = egui::Mesh::default();
        let steps = 36;
        for i in 0..steps {
            let h0 = i as f32 / steps as f32;
            let h1 = (i + 1) as f32 / steps as f32;
            let x0 = sp.left() + sp.width() * h0;
            let x1 = sp.left() + sp.width() * h1;
            let top0: Color32 = egui::ecolor::Hsva::new(h0, 1.0, 1.0, 1.0).into();
            let top1: Color32 = egui::ecolor::Hsva::new(h1, 1.0, 1.0, 1.0).into();
            let grey = Color32::from_gray(128);
            let base = mesh.vertices.len() as u32;
            mesh.colored_vertex(pos2(x0, sp.top()), top0);
            mesh.colored_vertex(pos2(x1, sp.top()), top1);
            mesh.colored_vertex(pos2(x0, sp.bottom()), grey);
            mesh.colored_vertex(pos2(x1, sp.bottom()), grey);
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base + 1, base + 3, base + 2);
        }
        ui.painter().add(egui::Shape::mesh(mesh));
        if let Some(p) = sresp.interact_pointer_pos()
            && (sresp.clicked() || sresp.dragged())
        {
            hsva.h = ((p.x - sp.left()) / sp.width()).clamp(0.0, 0.999);
            hsva.s = (1.0 - (p.y - sp.top()) / sp.height()).clamp(0.0, 1.0);
            if hsva.v < 0.05 {
                hsva.v = 1.0;
            }
            st.new = hsva.into();
            st.hex = hex_of(st.new);
        }
        let cx = sp.left() + sp.width() * hsva.h;
        let cy = sp.top() + sp.height() * (1.0 - hsva.s);
        ui.painter().line_segment([pos2(cx - 6.0, cy), pos2(cx + 6.0, cy)], Stroke::new(1.0, Color32::BLACK));
        ui.painter().line_segment([pos2(cx, cy - 6.0), pos2(cx, cy + 6.0)], Stroke::new(1.0, Color32::BLACK));
        ui.add_space(14.0);
        // Lightness bar.
        let (lb, lresp) = ui.allocate_exact_size(vec2(14.0, 130.0), Sense::click_and_drag());
        lresp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, "Lightness"));
        let mut mesh = egui::Mesh::default();
        let top: Color32 = egui::ecolor::Hsva::new(hsva.h, hsva.s, 1.0, 1.0).into();
        mesh.colored_vertex(lb.left_top(), top);
        mesh.colored_vertex(lb.right_top(), top);
        mesh.colored_vertex(lb.left_bottom(), Color32::BLACK);
        mesh.colored_vertex(lb.right_bottom(), Color32::BLACK);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(1, 3, 2);
        ui.painter().add(egui::Shape::mesh(mesh));
        if let Some(p) = lresp.interact_pointer_pos()
            && (lresp.clicked() || lresp.dragged())
        {
            hsva.v = (1.0 - (p.y - lb.top()) / lb.height()).clamp(0.0, 1.0);
            st.new = hsva.into();
            st.hex = hex_of(st.new);
        }
        let y = lb.top() + lb.height() * (1.0 - hsva.v);
        let x = lb.right() + 3.0;
        ui.painter().add(egui::Shape::convex_polygon(vec![pos2(x, y), pos2(x + 7.0, y - 5.0), pos2(x + 7.0, y + 5.0)], t.text, Stroke::NONE));
    });
    ui.add_space(8.0);
    egui::Grid::new("nux_custom_rgb").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
        ui.label(crate::border_preview::access_key_job("Color mo&del:", theme::ui_font(13.0), t.text));
        ui.label(egui::RichText::new("RGB").font(theme::ui_font(13.0)));
        ui.end_row();
        let mut rgb = [i32::from(st.new.r()), i32::from(st.new.g()), i32::from(st.new.b())];
        let mut changed = false;
        for (i, label) in ["&Red:", "&Green:", "&Blue:"].iter().enumerate() {
            ui.label(crate::border_preview::access_key_job(label, theme::ui_font(13.0), t.text));
            changed |= ui.add_sized([70.0, 24.0], egui::DragValue::new(&mut rgb[i]).range(0..=255)).changed();
            ui.end_row();
        }
        if changed {
            st.new = Color32::from_rgb(rgb[0] as u8, rgb[1] as u8, rgb[2] as u8);
            st.hex = hex_of(st.new);
        }
        ui.label(crate::border_preview::access_key_job("&Hex:", theme::ui_font(13.0), t.text));
        if ui.add_sized([90.0, 24.0], egui::TextEdit::singleline(&mut st.hex)).changed()
            && let Some(c) = parse_hex(&st.hex)
        {
            st.new = c;
        }
        ui.end_row();
    });
    let _ = lang;
}

/// Draws the Colors window when More Colors… opened it. Call once per frame.
pub fn window(ctx: &egui::Context) {
    let Some(mut st) = ctx.data_mut(|d| d.get_temp::<MoreColors>(more_id())) else { return };
    let lang = crate::i18n::current(ctx);
    let t = Tokens::get(ctx);
    let mut open = true;
    let mut result: Option<Option<Color32>> = None;
    egui::Window::new(lang.tr("Colors"))
        .id(Id::new("nux_colors_window"))
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, -20.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            crate::dialogs::light_title(ui, "Colors", &mut open, "Choose a colour from the honeycomb or mix your own.");
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for (custom, label) in [(false, "Standard"), (true, "Custom")] {
                            let text = lang.tr(label);
                            let g = ui.painter().layout_no_wrap(text.to_string(), theme::ui_font(13.5), t.text);
                            let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 30.0, 30.0), Sense::click());
                            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, st.custom == custom, text));
                            let sel = st.custom == custom;
                            let r = if sel { r } else { Rect::from_min_max(r.min + vec2(0.0, 3.0), r.max) };
                            ui.painter().rect_filled(r, CornerRadius::ZERO, if sel { t.input_bg } else { t.window });
                            let s = Stroke::new(1.0, t.input_border);
                            ui.painter().line_segment([r.left_bottom(), r.left_top()], s);
                            ui.painter().line_segment([r.left_top(), r.right_top()], s);
                            ui.painter().line_segment([r.right_top(), r.right_bottom()], s);
                            ui.painter().galley(r.center() - g.size() / 2.0, g, t.text);
                            if resp.clicked() {
                                st.custom = custom;
                            }
                        }
                    });
                    egui::Frame::new().fill(t.input_bg).stroke(Stroke::new(1.0, t.input_border)).inner_margin(egui::Margin::same(12)).show(
                        ui,
                        |ui| {
                            ui.set_min_size(vec2(260.0, 300.0));
                            ui.set_max_width(260.0);
                            if st.custom {
                                custom_tab(ui, &mut st);
                            } else {
                                standard_tab(ui, &mut st);
                            }
                        },
                    );
                });
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    let button =
                        |ui: &mut Ui, text: &str, primary: bool| {
                            ui.add_sized(
                                [118.0, 32.0],
                                egui::Button::new(egui::RichText::new(lang.tr(text)).font(theme::ui_font(13.5)))
                                    .corner_radius(4)
                                    .stroke(if primary { Stroke::new(1.5, t.accent) } else { Stroke::new(1.0, t.input_border) }),
                            )
                        };
                    if button(ui, "OK", true).clicked() {
                        result = Some(Some(st.new));
                    }
                    ui.add_space(8.0);
                    if button(ui, "Cancel", false).clicked() {
                        result = Some(None);
                    }
                    ui.add_space(170.0);
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new(lang.tr("New")).font(theme::ui_font(13.0)));
                        let (r, _) = ui.allocate_exact_size(vec2(80.0, 80.0), Sense::hover());
                        let half = Rect::from_min_max(r.min, pos2(r.right(), r.center().y));
                        ui.painter().rect_filled(half, CornerRadius::ZERO, st.new);
                        let low = Rect::from_min_max(pos2(r.left(), r.center().y), r.max);
                        ui.painter().rect_filled(low, CornerRadius::ZERO, st.current.unwrap_or(Color32::WHITE));
                        ui.painter().rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.0, t.text_dim), StrokeKind::Inside);
                        ui.label(egui::RichText::new(lang.tr("Current")).font(theme::ui_font(13.0)));
                    });
                });
            });
        });
    // The window owns Enter (OK) and Escape (Cancel): both are consumed here, before the dialogs
    // drawn after it could take them as their own OK or Cancel.
    if result.is_none() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
        result = Some(Some(st.new));
    }
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) || !open {
        result = Some(None);
    }
    match result {
        Some(Some(c)) => {
            let key = st.key.clone();
            let frame = ctx.cumulative_frame_nr();
            ctx.data_mut(|d| {
                d.remove_temp::<MoreColors>(more_id());
                d.insert_temp(result_id(&key), (hex_of(c), frame));
            });
        }
        Some(None) => {
            ctx.data_mut(|d| d.remove_temp::<MoreColors>(more_id()));
        }
        None => {
            ctx.data_mut(|d| d.insert_temp(more_id(), st));
        }
    }
}
