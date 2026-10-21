//! Ribbon layout pieces in the desktop style: group captions under each group with
//! an optional dialog launcher, buttons that take an original icon painter, and the extra icons
//! the Page Layout tab needs. All artwork is this project's own.

use egui::{Align2, Color32, CornerRadius, Id, Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::theme::{self, Tokens};

fn group_x_id() -> Id {
    Id::new("ribbon_group_x")
}
fn body_id() -> Id {
    Id::new("ribbon_body")
}

/// Records where the ribbon body is (top and bottom) and starts the first group at the cursor.
pub(crate) fn begin_groups(ui: &mut Ui, body: Rect) {
    let x = ui.cursor().left();
    ui.ctx().data_mut(|d| {
        d.insert_temp(group_x_id(), x);
        d.insert_temp(body_id(), body);
    });
}

/// Ends the current group: writes `label` centred under it, draws the dialog launcher when
/// `launcher` is set, and a separator line. Returns true when the launcher was clicked.
pub(crate) fn end_group(ui: &mut Ui, label: &str, launcher: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let lang = crate::i18n::current(ui.ctx());
    let (x0, body): (f32, Rect) = ui.ctx().data_mut(|d| (d.get_temp(group_x_id()).unwrap_or(0.0), d.get_temp(body_id()).unwrap_or(Rect::NOTHING)));
    ui.add_space(4.0);
    let x1 = ui.cursor().left();
    let y = body.bottom() - 9.0;
    let text = lang.tr(label);
    ui.painter().text(pos2((x0 + x1) / 2.0, y), Align2::CENTER_CENTER, text, theme::ui_font(12.0), t.text_dim);
    let mut clicked = false;
    if launcher {
        let r = Rect::from_center_size(pos2(x1 - 8.0, y), vec2(14.0, 14.0));
        let resp = ui.interact(r, Id::new(("launcher", label)), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{text} settings")));
        if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(2), t.hover);
        }
        let b = r.shrink(3.5);
        let s = Stroke::new(1.0, t.text_dim);
        ui.painter().line_segment([b.left_top(), b.right_top() - vec2(3.0, 0.0)], s);
        ui.painter().line_segment([b.left_top(), b.left_bottom() - vec2(0.0, 3.0)], s);
        ui.painter().arrow(b.center() - vec2(1.0, 1.0), vec2(3.5, 3.5), s);
        clicked = resp.on_hover_text(lang.tr("Show more options")).clicked();
    }
    let (r, _) = ui.allocate_exact_size(vec2(9.0, 10.0), Sense::hover());
    let sx = r.center().x;
    ui.painter().line_segment([pos2(sx, body.top() + 4.0), pos2(sx, body.bottom() - 4.0)], Stroke::new(1.0, t.separator));
    let next = ui.cursor().left();
    ui.ctx().data_mut(|d| d.insert_temp(group_x_id(), next));
    clicked
}

/// Extra ribbon icons, drawn in code.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RibIcon {
    ThemeColors,
    ThemeFonts,
    ThemeEffects,
    Background,
    BringForward,
    SendBackward,
    SelectionPane,
    Align,
    GroupObjects,
    Rotate,
    Width,
    Height,
    Scale,
    /// "A" with a small up caret (Increase Font Size).
    GrowFont,
    /// "A" with a small down caret (Decrease Font Size).
    ShrinkFont,
    /// A pilcrow with a left-right arrow (Text Direction).
    TextDirection,
}

pub(crate) fn paint(p: &Painter, r: Rect, icon: RibIcon, ink: Color32) {
    let s = Stroke::new(1.3, ink);
    let amber = Color32::from_rgb(250, 196, 120);
    let blue = Color32::from_rgb(91, 155, 213);
    match icon {
        RibIcon::GrowFont | RibIcon::ShrinkFont => {
            let font = egui::FontId::proportional(r.height() * 0.95);
            p.text(pos2(r.left() + r.width() * 0.38, r.center().y + 1.0), Align2::CENTER_CENTER, "A", font, ink);
            let c = pos2(r.right() - r.width() * 0.16, r.top() + r.height() * 0.28);
            let w = r.width() * 0.16;
            let pts = if icon == RibIcon::GrowFont {
                vec![pos2(c.x - w, c.y + w * 0.6), pos2(c.x, c.y - w * 0.6), pos2(c.x + w, c.y + w * 0.6)]
            } else {
                vec![pos2(c.x - w, c.y - w * 0.6), pos2(c.x, c.y + w * 0.6), pos2(c.x + w, c.y - w * 0.6)]
            };
            p.add(egui::Shape::line(pts, Stroke::new(1.2, ink)));
        }
        RibIcon::TextDirection => {
            // Pilcrow: a filled bowl and two stems, with a double arrow underneath.
            let top = r.top() + 1.0;
            let x0 = r.left() + r.width() * 0.3;
            p.circle_filled(pos2(x0, top + 3.5), 3.2, ink);
            p.vline(x0 + 2.5, top..=(top + 10.0), s);
            p.vline(x0 + 5.5, top..=(top + 10.0), s);
            p.hline((x0 - 1.0)..=(x0 + 7.0), top, s);
            let y = r.bottom() - 2.5;
            p.hline((r.left() + 2.0)..=(r.right() - 2.0), y, s);
            for (tip, dir) in [(r.left() + 2.0, 1.0), (r.right() - 2.0, -1.0)] {
                p.line_segment([pos2(tip, y), pos2(tip + 3.0 * dir, y - 2.5)], s);
                p.line_segment([pos2(tip, y), pos2(tip + 3.0 * dir, y + 2.5)], s);
            }
        }
        RibIcon::ThemeColors => {
            let q = r.width() / 2.0;
            for (i, c) in
                [Color32::from_rgb(70, 110, 180), Color32::from_rgb(230, 120, 50), Color32::from_rgb(60, 150, 80), Color32::from_rgb(40, 40, 40)]
                    .iter()
                    .enumerate()
            {
                let x = r.left() + (i % 2) as f32 * q;
                let y = r.top() + (i / 2) as f32 * q;
                p.rect_filled(Rect::from_min_size(pos2(x, y), vec2(q, q)).shrink(0.5), CornerRadius::ZERO, *c);
            }
        }
        RibIcon::ThemeFonts => {
            p.rect_stroke(r.shrink(1.0), CornerRadius::same(1), s, StrokeKind::Inside);
            p.text(r.center(), Align2::CENTER_CENTER, "i", egui::FontId::proportional(r.height() * 0.8), ink);
        }
        RibIcon::ThemeEffects => {
            p.circle(r.center(), r.width() * 0.42, Color32::from_rgb(120, 150, 200), Stroke::new(1.0, ink));
            p.circle_filled(r.center() - vec2(r.width() * 0.12, r.width() * 0.12), r.width() * 0.12, Color32::from_rgb(220, 230, 245));
        }
        RibIcon::Background => {
            let f = r.shrink(1.0);
            p.rect(f, CornerRadius::same(1), Color32::from_rgb(210, 230, 245), s, StrokeKind::Inside);
            let pts = vec![
                pos2(f.left() + 2.0, f.bottom() - 2.0),
                pos2(f.left() + f.width() * 0.4, f.top() + f.height() * 0.45),
                pos2(f.left() + f.width() * 0.6, f.bottom() - 2.0),
            ];
            p.add(egui::Shape::convex_polygon(pts, Color32::from_rgb(90, 150, 90), Stroke::NONE));
            let pts = vec![
                pos2(f.left() + f.width() * 0.45, f.bottom() - 2.0),
                pos2(f.left() + f.width() * 0.72, f.top() + f.height() * 0.3),
                pos2(f.right() - 2.0, f.bottom() - 2.0),
            ];
            p.add(egui::Shape::convex_polygon(pts, Color32::from_rgb(60, 120, 70), Stroke::NONE));
            p.circle_filled(pos2(f.right() - f.width() * 0.25, f.top() + f.height() * 0.25), f.width() * 0.08, Color32::from_rgb(240, 190, 60));
        }
        RibIcon::BringForward | RibIcon::SendBackward => {
            let a = Rect::from_min_size(r.min + vec2(1.0, 1.0), r.size() * 0.6);
            let b = Rect::from_min_size(r.min + r.size() * 0.38, r.size() * 0.6);
            let (back, front) = if icon == RibIcon::BringForward { (b, a) } else { (a, b) };
            p.rect(back, CornerRadius::ZERO, Color32::WHITE, s, StrokeKind::Inside);
            p.rect(front, CornerRadius::ZERO, amber, s, StrokeKind::Inside);
        }
        RibIcon::SelectionPane => {
            let f = Rect::from_min_size(r.min + vec2(1.0, r.height() * 0.25), r.size() * vec2(0.7, 0.6));
            p.rect(f, CornerRadius::ZERO, amber, s, StrokeKind::Inside);
            // A pointer arrow.
            let tip = pos2(r.center().x + 1.0, r.center().y - 1.0);
            let pts = vec![
                tip,
                tip + vec2(0.0, r.height() * 0.5),
                tip + vec2(r.width() * 0.14, r.height() * 0.36),
                tip + vec2(r.width() * 0.36, r.height() * 0.36),
            ];
            p.add(egui::Shape::convex_polygon(pts, Color32::WHITE, s));
        }
        RibIcon::Align => {
            p.line_segment([pos2(r.left() + 2.0, r.top()), pos2(r.left() + 2.0, r.bottom())], s);
            p.rect(
                Rect::from_min_size(pos2(r.left() + 3.0, r.top() + r.height() * 0.15), vec2(r.width() * 0.75, r.height() * 0.25)),
                CornerRadius::ZERO,
                Color32::WHITE,
                s,
                StrokeKind::Inside,
            );
            p.rect(
                Rect::from_min_size(pos2(r.left() + 3.0, r.top() + r.height() * 0.58), vec2(r.width() * 0.5, r.height() * 0.25)),
                CornerRadius::ZERO,
                blue,
                s,
                StrokeKind::Inside,
            );
        }
        RibIcon::GroupObjects => {
            p.rect_stroke(r.shrink(1.0), CornerRadius::ZERO, Stroke::new(1.0, ink), StrokeKind::Inside);
            p.rect_filled(Rect::from_min_size(r.min + vec2(4.0, 4.0), r.size() * 0.4), CornerRadius::ZERO, Color32::from_gray(200));
            p.rect_filled(Rect::from_min_size(r.min + r.size() * 0.45, r.size() * 0.4), CornerRadius::ZERO, Color32::from_gray(200));
        }
        RibIcon::Rotate => {
            let pts = vec![
                pos2(r.left() + 2.0, r.bottom() - 2.0),
                pos2(r.right() - 2.0, r.bottom() - 2.0),
                pos2(r.center().x, r.top() + r.height() * 0.45),
            ];
            p.add(egui::Shape::convex_polygon(pts, Color32::from_gray(200), s));
            p.add(egui::Shape::Path(egui::epaint::PathShape::line(
                (0..8)
                    .map(|k| {
                        let a = (200.0 + k as f32 * 18.0f32).to_radians();
                        r.center() + vec2(a.cos(), a.sin()) * r.width() * 0.4
                    })
                    .collect(),
                s,
            )));
        }
        RibIcon::Width | RibIcon::Height => {
            let f = r.shrink(1.0);
            p.rect_stroke(f, CornerRadius::ZERO, Stroke::new(1.0, ink), StrokeKind::Inside);
            let (a, b) = if icon == RibIcon::Width {
                (pos2(f.left() + 2.0, f.center().y), pos2(f.right() - 2.0, f.center().y))
            } else {
                (pos2(f.center().x, f.top() + 2.0), pos2(f.center().x, f.bottom() - 2.0))
            };
            p.arrow(a.lerp(b, 0.5), b - a.lerp(b, 0.5), Stroke::new(1.2, blue));
            p.arrow(a.lerp(b, 0.5), a - a.lerp(b, 0.5), Stroke::new(1.2, blue));
        }
        RibIcon::Scale => {
            let f = r.shrink(1.0);
            p.rect_stroke(f, CornerRadius::ZERO, Stroke::new(1.0, ink), StrokeKind::Inside);
            p.rect_stroke(Rect::from_min_size(f.min, f.size() * 0.55), CornerRadius::ZERO, Stroke::new(1.0, blue), StrokeKind::Inside);
            p.arrow(f.min + f.size() * 0.3, f.size() * 0.5, Stroke::new(1.2, blue));
        }
    }
}

fn chevron(p: &Painter, at: Pos2, t: &Tokens) {
    crate::icons::paint(p, Rect::from_center_size(at, vec2(10.0, 10.0)), crate::icons::Icon::Chevron, t.text_dim);
}

/// A large ribbon button (72 high) whose 30-point icon is drawn by `icon`.
pub(crate) fn big(
    ui: &mut Ui,
    icon: impl FnOnce(&Painter, Rect, Color32),
    label: &str,
    tip: &str,
    dropdown: bool,
    enabled: bool,
    pressed: bool,
) -> Response {
    let t = Tokens::get(ui.ctx());
    let lang = crate::i18n::current(ui.ctx());
    let font = theme::ui_font(12.5);
    let label = lang.tr(label).to_string();
    let lines: Vec<String> = label.split('\n').map(str::to_string).collect();
    let widths: Vec<f32> = lines.iter().map(|l| ui.painter().layout_no_wrap(l.clone(), font.clone(), t.text).size().x).collect();
    let w = widths.iter().enumerate().map(|(i, w)| w + if dropdown && i + 1 == lines.len() { 14.0 } else { 0.0 }).fold(0.0, f32::max) + 12.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w.max(46.0), 72.0), if enabled { Sense::click() } else { Sense::hover() });
    let ink = if enabled { t.text } else { t.text_disabled };
    if pressed {
        ui.painter().rect(rect, CornerRadius::same(4), t.pressed, Stroke::new(1.0, t.separator), StrokeKind::Inside);
    } else if enabled && resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), t.pressed);
    } else if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), t.hover);
    }
    // A button with no label (the Draw tab's pens) gives its artwork the whole height.
    let art = if label.is_empty() {
        Rect::from_center_size(pos2(rect.center().x, rect.top() + 32.0), vec2(34.0, 54.0))
    } else {
        Rect::from_center_size(pos2(rect.center().x, rect.top() + 20.0), vec2(30.0, 30.0))
    };
    icon(ui.painter(), art, ink);
    for (i, line) in lines.iter().enumerate() {
        let arrow = dropdown && i + 1 == lines.len();
        let lw = widths[i] + if arrow { 14.0 } else { 0.0 };
        let left = rect.center().x - lw / 2.0;
        let y = rect.top() + 45.0 + i as f32 * 14.0;
        ui.painter().text(pos2(left, y), Align2::LEFT_CENTER, line, font.clone(), ink);
        if arrow {
            chevron(ui.painter(), pos2(left + lw - 5.0, y + 1.0), &t);
        }
    }
    // An icon-only button (the Draw tab's pens and tools) is named by its whole translated
    // tooltip, so the pens read apart ("Pen: Red, 0.5 mm").
    let plain = if label.is_empty() { lang.tr(tip).to_string() } else { label.replace('\n', " ") };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &plain));
    resp.on_hover_text(lang.tr(tip))
}

/// A small ribbon button (16-point icon, label, optional arrow).
pub(crate) fn small(ui: &mut Ui, icon: impl FnOnce(&Painter, Rect, Color32), label: &str, tip: &str, dropdown: bool, enabled: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let lang = crate::i18n::current(ui.ctx());
    let font = theme::ui_font(12.5);
    let label = lang.tr(label).to_string();
    let tw = ui.painter().layout_no_wrap(label.clone(), font.clone(), t.text).size().x;
    let (rect, resp) =
        ui.allocate_exact_size(vec2(26.0 + tw + if dropdown { 14.0 } else { 4.0 }, 24.0), if enabled { Sense::click() } else { Sense::hover() });
    let ink = if enabled { t.text } else { t.text_disabled };
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), t.hover);
    }
    icon(ui.painter(), Rect::from_center_size(pos2(rect.left() + 12.0, rect.center().y), vec2(16.0, 16.0)), ink);
    ui.painter().text(pos2(rect.left() + 25.0, rect.center().y), Align2::LEFT_CENTER, &label, font, ink);
    if dropdown {
        chevron(ui.painter(), pos2(rect.right() - 7.0, rect.center().y + 1.0), &t);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &label));
    resp.on_hover_text(lang.tr(tip))
}

/// What the ribbon's right-click menu asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RibbonMenu {
    ToggleQuickAccess,
    Customize,
    ToggleCollapse,
}

fn menu_id() -> Id {
    Id::new("ribbon_context_menu")
}

/// Opens the ribbon's right-click menu when the secondary button is clicked inside one of
/// `areas`, draws it while open, and returns the chosen entry.
pub(crate) fn context_menu(ctx: &egui::Context, areas: &[Rect], qat_hidden: bool, collapsed: bool) -> Option<RibbonMenu> {
    let lang = crate::i18n::current(ctx);
    let t = Tokens::get(ctx);
    let (clicked, pos) = ctx.input(|i| (i.pointer.secondary_clicked(), i.pointer.interact_pos()));
    let mut open: Option<Pos2> = ctx.data(|d| d.get_temp(menu_id()));
    // A control with its own right-click menu (Paste's options, say) keeps the click.
    let just_opened = clicked && open.is_none() && !egui::Popup::is_any_open(ctx) && pos.is_some_and(|p| areas.iter().any(|a| a.contains(p)));
    if just_opened {
        open = pos;
    }
    let at = open?;
    let qat_label = if qat_hidden { "&Show Quick Access Toolbar" } else { "&Hide Quick Access Toolbar" };
    let collapse_label = if collapsed { "Pin the &Ribbon" } else { "Collapse the &Ribbon" };
    // (label, enabled, choice); `None` is a separator.
    let rows: [Option<(&str, bool, Option<RibbonMenu>)>; 5] = [
        Some(("&Add to Quick Access Toolbar", false, None)),
        Some((qat_label, true, Some(RibbonMenu::ToggleQuickAccess))),
        None,
        Some(("Customize the R&ibbon…", true, Some(RibbonMenu::Customize))),
        Some((collapse_label, true, Some(RibbonMenu::ToggleCollapse))),
    ];
    let font = theme::ui_font(12.5);
    let mut chosen = None;
    let area = egui::Area::new(menu_id()).fixed_pos(at).order(egui::Order::Foreground).constrain(true).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::same(5)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for row in rows {
                let Some((label, enabled, choice)) = row else {
                    let (r, _) = ui.allocate_exact_size(vec2(236.0, 9.0), Sense::hover());
                    ui.painter().hline(r.x_range().shrink(4.0), r.center().y, Stroke::new(1.0, t.separator));
                    continue;
                };
                let (r, resp) = ui.allocate_exact_size(vec2(236.0, 28.0), if enabled { Sense::click() } else { Sense::hover() });
                if enabled && resp.hovered() {
                    ui.painter().rect_filled(r, CornerRadius::same(4), t.hover);
                }
                let ink = if enabled { t.text } else { t.text_disabled };
                let shown = lang.tr(label).to_string();
                let job = crate::border_preview::access_key_job(&shown, font.clone(), ink);
                let galley = ui.painter().layout_job(job);
                ui.painter().galley(pos2(r.left() + 34.0, r.center().y - galley.size().y / 2.0), galley, ink);
                let plain = shown.replace('&', "");
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &plain));
                if enabled && resp.clicked() {
                    chosen = choice;
                }
            }
        });
    });
    let outside = !just_opened && ctx.input(|i| i.pointer.any_click()) && pos.is_some_and(|p| !area.response.rect.contains(p));
    let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let keep = chosen.is_none() && !outside && !escape;
    ctx.data_mut(|d| {
        if keep {
            d.insert_temp(menu_id(), at);
        } else {
            d.remove::<Pos2>(menu_id());
        }
    });
    chosen
}

/// The app's own mark for the title bar: a rounded green tile with a white grid.
pub(crate) fn app_mark(p: &Painter, r: Rect) {
    let green = Color32::from_rgb(0x10, 0x7C, 0x41);
    p.rect_filled(r, CornerRadius::same(3), green);
    let g = r.shrink(3.5);
    let s = Stroke::new(1.2, Color32::WHITE);
    p.rect_stroke(g, CornerRadius::same(1), s, StrokeKind::Middle);
    p.hline(g.x_range(), g.center().y, s);
    p.vline(g.center().x, g.y_range(), s);
}

/// A small labelled ribbon button that can show a pressed (on) state, such as Format Painter or
/// Wrap Text: a 16-point icon drawn by `icon`, then the label.
pub(crate) fn small_toggle(ui: &mut Ui, icon: impl FnOnce(&Painter, Rect, Color32), label: &str, tip: &str, on: bool, dropdown: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let lang = crate::i18n::current(ui.ctx());
    let font = theme::ui_font(12.5);
    let label = lang.tr(label).to_string();
    let tw = ui.painter().layout_no_wrap(label.clone(), font.clone(), t.text).size().x;
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0 + tw + if dropdown { 14.0 } else { 6.0 }, 24.0), Sense::click());
    if on {
        ui.painter().rect_filled(rect, CornerRadius::same(4), t.pressed);
        ui.painter().rect_stroke(rect, CornerRadius::same(4), Stroke::new(1.0, t.separator), StrokeKind::Inside);
    } else if resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), t.pressed);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), t.hover);
    }
    icon(ui.painter(), Rect::from_center_size(pos2(rect.left() + 13.0, rect.center().y), vec2(16.0, 16.0)), t.text);
    ui.painter().text(pos2(rect.left() + 26.0, rect.center().y), Align2::LEFT_CENTER, &label, font, t.text);
    if dropdown {
        chevron(ui.painter(), pos2(rect.right() - 7.0, rect.center().y + 1.0), &t);
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, &label));
    resp.on_hover_text(lang.tr(tip))
}
