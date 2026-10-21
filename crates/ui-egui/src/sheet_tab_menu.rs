//! The sheet tab's right-click menu in the desktop layout: an icon column, labels
//! with underlined access keys, separators, and a Tab Color submenu holding the colour palette.
//! The icons are drawn in code.

use egui::{Color32, CornerRadius, Id, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::theme::{self, Tokens};

/// What the menu asked for.
#[derive(Clone, Debug, PartialEq)]
pub enum Choice {
    Insert,
    Delete,
    Rename,
    MoveOrCopy,
    Protect,
    TabColor(String),
    Hide,
    Unhide,
    Link,
}

#[derive(Clone, Copy)]
enum Glyph {
    Delete,
    Rename,
    Code,
    Protect,
    Link,
}

/// The menu's open state: where it opened and for which sheet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Open {
    pub at: Pos2,
    pub sheet: usize,
}

fn id() -> Id {
    Id::new("sheet_tab_menu")
}

pub fn open(ctx: &egui::Context, at: Pos2, sheet: usize) {
    ctx.data_mut(|d| d.insert_temp(id(), Open { at, sheet }));
    ctx.data_mut(|d| d.insert_temp(id().with("just"), true));
}

pub fn is_open(ctx: &egui::Context) -> Option<Open> {
    ctx.data(|d| d.get_temp(id()))
}

fn close(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<Open>(id()));
}

/// A small sheet (a grid in a frame) with an overlay glyph.
fn paint(p: &Painter, r: Rect, glyph: Glyph, ink: Color32) {
    let sheet = Rect::from_min_size(r.min + vec2(1.0, 2.0), vec2(12.0, 11.0));
    let s = Stroke::new(1.0, ink);
    p.rect_stroke(sheet, CornerRadius::same(1), s, StrokeKind::Inside);
    p.hline(sheet.x_range(), sheet.top() + 3.5, s);
    p.hline(sheet.x_range(), sheet.top() + 7.0, s);
    p.vline(sheet.left() + 4.0, sheet.y_range(), s);
    p.vline(sheet.left() + 8.0, sheet.y_range(), s);
    let corner = Rect::from_min_size(r.min + vec2(8.0, 8.0), vec2(9.0, 9.0));
    p.rect_filled(corner.expand(0.5), CornerRadius::same(2), p.ctx().global_style().visuals.window_fill);
    let c = corner.center();
    match glyph {
        Glyph::Delete => {
            let red = Stroke::new(1.6, Color32::from_rgb(0xD1, 0x34, 0x38));
            p.line_segment([c - vec2(3.5, 3.5), c + vec2(3.5, 3.5)], red);
            p.line_segment([c + vec2(-3.5, 3.5), c + vec2(3.5, -3.5)], red);
        }
        Glyph::Rename => {
            let blue = Color32::from_rgb(0x1F, 0x6F, 0xC5);
            p.line_segment([c + vec2(-3.5, 3.5), c + vec2(3.0, -3.0)], Stroke::new(2.4, blue));
            p.line_segment([c + vec2(-4.5, 4.5), c + vec2(-3.0, 3.0)], Stroke::new(1.2, ink));
        }
        Glyph::Code => {
            let blue = Color32::from_rgb(0x1F, 0x6F, 0xC5);
            p.circle_stroke(c + vec2(-0.5, -0.5), 3.0, Stroke::new(1.3, blue));
            p.line_segment([c + vec2(1.6, 1.6), c + vec2(4.0, 4.0)], Stroke::new(1.6, blue));
        }
        Glyph::Protect => {
            let gold = Color32::from_rgb(0xE8, 0x9A, 0x1C);
            let body = Rect::from_center_size(c + vec2(0.0, 1.5), vec2(8.0, 6.0));
            p.rect_filled(body, CornerRadius::same(1), gold);
            p.circle_stroke(c + vec2(0.0, -2.0), 2.5, Stroke::new(1.2, ink));
            p.rect_filled(body, CornerRadius::same(1), gold);
        }
        Glyph::Link => {
            let blue = Stroke::new(1.3, Color32::from_rgb(0x1F, 0x6F, 0xC5));
            p.rect_stroke(Rect::from_center_size(c + vec2(-1.8, 1.8), vec2(5.0, 3.6)), CornerRadius::same(2), blue, StrokeKind::Middle);
            p.rect_stroke(Rect::from_center_size(c + vec2(1.8, -1.8), vec2(5.0, 3.6)), CornerRadius::same(2), blue, StrokeKind::Middle);
        }
    }
}

enum Row {
    Item { label: &'static str, glyph: Option<Glyph>, enabled: bool, choice: Option<Choice>, submenu: bool },
    Sep,
}

/// Draws the menu while it is open. Returns the chosen entry; the menu closes on a choice,
/// Escape or a click outside it.
pub fn show(ctx: &egui::Context, theme_colors: &[u32; 12], protected: bool, any_hidden: bool) -> Option<(usize, Choice)> {
    let open = is_open(ctx)?;
    // More Colors… hands over to the Colors window: the menu closes, so it neither takes the
    // window's typing (Hex) nor its Escape. The colour still reaches the sheet (tabs::tab_menu).
    if crate::color_picker::is_open(ctx) {
        close(ctx);
        ctx.data_mut(|d| d.insert_temp(id().with("sub"), false));
        return None;
    }
    let just = ctx.data_mut(|d| d.remove_temp::<bool>(id().with("just"))).unwrap_or(false);
    let t = Tokens::get(ctx);
    let lang = crate::i18n::current(ctx);
    let rows = [
        Row::Item { label: "&Insert…", glyph: None, enabled: true, choice: Some(Choice::Insert), submenu: false },
        Row::Item { label: "&Delete", glyph: Some(Glyph::Delete), enabled: true, choice: Some(Choice::Delete), submenu: false },
        Row::Item { label: "&Rename", glyph: Some(Glyph::Rename), enabled: true, choice: Some(Choice::Rename), submenu: false },
        Row::Item { label: "&Move or Copy…", glyph: None, enabled: true, choice: Some(Choice::MoveOrCopy), submenu: false },
        Row::Item { label: "&View Code", glyph: Some(Glyph::Code), enabled: false, choice: None, submenu: false },
        Row::Item {
            label: if protected { "Un&protect Sheet…" } else { "&Protect Sheet…" },
            glyph: Some(Glyph::Protect),
            enabled: true,
            choice: Some(Choice::Protect),
            submenu: false,
        },
        Row::Item { label: "&Tab Color", glyph: None, enabled: true, choice: None, submenu: true },
        Row::Sep,
        Row::Item { label: "&Hide", glyph: None, enabled: true, choice: Some(Choice::Hide), submenu: false },
        Row::Item { label: "&Unhide…", glyph: None, enabled: any_hidden, choice: Some(Choice::Unhide), submenu: false },
        Row::Sep,
        // Needs sheet groups (several selected sheets), which GridCraft doesn't have yet.
        Row::Item { label: "&Select All Sheets", glyph: None, enabled: false, choice: None, submenu: false },
        Row::Sep,
        Row::Item { label: "&Link to this Sheet", glyph: Some(Glyph::Link), enabled: true, choice: Some(Choice::Link), submenu: false },
    ];
    const W: f32 = 226.0;
    const H: f32 = 32.0;
    let font = theme::ui_font(13.0);
    let mut chosen: Option<Choice> = None;
    let sub_id = id().with("sub");
    let mut sub_open: bool = ctx.data(|d| d.get_temp(sub_id)).unwrap_or(false);
    let mut sub_anchor: Option<Rect> = None;
    // Accelerator: a typed access key runs its item.
    // Typed text is the menu's while it is open: taken from the input, so the grid doesn't also
    // start editing the active cell with it.
    let typed: Option<char> = ctx.input_mut(|i| {
        let typed = i.events.iter().find_map(|e| match e {
            egui::Event::Text(s) => s.chars().next().map(|c| c.to_ascii_lowercase()),
            _ => None,
        });
        i.events.retain(|e| !matches!(e, egui::Event::Text(_)));
        typed
    });
    // The menu opens upwards from the tab bar, like the desktop one.
    let height: f32 = rows.iter().map(|r| if matches!(r, Row::Sep) { 9.0 } else { H }).sum::<f32>() + 12.0;
    let at = pos2(open.at.x, open.at.y - height);
    let main = egui::Area::new(id().with("area")).fixed_pos(at).order(egui::Order::Foreground).constrain(true).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::same(5)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for row in &rows {
                let Row::Item { label, glyph, enabled, choice, submenu } = row else {
                    let (r, _) = ui.allocate_exact_size(vec2(W, 9.0), Sense::hover());
                    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, t.separator));
                    continue;
                };
                let (r, resp) = ui.allocate_exact_size(vec2(W, H), if *enabled { Sense::click() } else { Sense::hover() });
                let hot = *enabled && (resp.hovered() || (*submenu && sub_open));
                if hot {
                    ui.painter().rect_filled(r, CornerRadius::same(4), t.hover);
                }
                let ink = if *enabled { t.text } else { t.text_disabled };
                if let Some(g) = glyph {
                    paint(ui.painter(), Rect::from_min_size(pos2(r.left() + 8.0, r.center().y - 9.0), vec2(18.0, 18.0)), *g, ink);
                }
                let shown = crate::border_preview::tr_access(lang, label);
                let job = crate::border_preview::access_key_job(&shown, font.clone(), ink);
                let galley = ui.painter().layout_job(job);
                ui.painter().galley(pos2(r.left() + 38.0, r.center().y - galley.size().y / 2.0), galley, ink);
                if *submenu {
                    crate::icons::paint(
                        ui.painter(),
                        Rect::from_center_size(pos2(r.right() - 12.0, r.center().y), vec2(10.0, 10.0)),
                        crate::icons::Icon::Right,
                        ink,
                    );
                    sub_anchor = Some(r);
                    if resp.hovered() || resp.clicked() {
                        sub_open = true;
                    }
                } else if resp.hovered() {
                    sub_open = false;
                }
                let plain = shown.replace('&', "");
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, *enabled, &plain));
                if *enabled && resp.clicked() && !*submenu {
                    chosen = choice.clone();
                }
                let key = label.find('&').and_then(|i| label[i + 1..].chars().next()).map(|c| c.to_ascii_lowercase());
                if *enabled && !just && !sub_open && key.is_some() && key == typed {
                    if *submenu {
                        sub_open = true;
                    } else {
                        chosen = choice.clone();
                    }
                }
            }
        });
    });
    let mut sub_rect = None;
    if sub_open && let Some(anchor) = sub_anchor {
        let sub = egui::Area::new(id().with("palette"))
            .fixed_pos(pos2(main.response.rect.right() - 2.0, anchor.top() - 120.0))
            .order(egui::Order::Foreground)
            .constrain(true)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                    if let Some(c) = crate::color_picker::palette(ui, theme_colors, "No Color", "tabColor") {
                        chosen = Some(Choice::TabColor(c));
                    }
                });
            });
        sub_rect = Some(sub.response.rect);
    }
    ctx.data_mut(|d| d.insert_temp(sub_id, sub_open));
    let pointer = ctx.input(|i| i.pointer.interact_pos());
    let inside = pointer.is_some_and(|p| main.response.rect.contains(p) || sub_rect.is_some_and(|r| r.contains(p)));
    let clicked_out = !just && ctx.input(|i| i.pointer.any_click()) && !inside;
    let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    if chosen.is_some() || clicked_out || escape {
        close(ctx);
        ctx.data_mut(|d| d.insert_temp(sub_id, false));
    }
    chosen.map(|c| (open.sheet, c))
}
