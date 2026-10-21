//! Sheet tabs and the status bar.

use std::sync::Arc;

use egui::{Align2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use gridcraft_engine::model::{Sheet, Visibility};
use serde_json::json;

use crate::SheetApp;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};

/// The rename field asks for focus only while it has not got it yet. Asking again on the frame it lost focus (Enter or a click elsewhere)
/// would keep the field focused after it is closed, so the keyboard stayed stuck in the sheet name.
fn rename_needs_focus(has_focus: bool, lost_focus: bool) -> bool {
    !has_focus && !lost_focus
}

/// Rows of the All Sheets navigator as `(workbook index, name)`. Hidden sheets stay out, like on the tab strip:
/// Unhide is the way to reach them.
fn navigator_items(sheets: &[Arc<Sheet>]) -> Vec<(String, String)> {
    sheets.iter().enumerate().filter(|(_, s)| s.visibility == Visibility::Visible).map(|(i, s)| (i.to_string(), s.name.clone())).collect()
}

/// Switch to sheet `i` from the tab strip or the navigator. In point mode the formula being typed keeps going on the
/// other sheet; otherwise the edit is committed first.
fn activate_sheet(app: &mut SheetApp, i: usize) {
    if app.editor.as_ref().is_some_and(|e| e.can_point()) {
        let _ = app.session.run("sheet.activate", json!({"sheet": i}));
    } else {
        if app.editor.is_some() {
            app.commit_edit(0, 0, false, false);
        }
        app.run_or_alert("sheet.activate", json!({"sheet": i}));
    }
}

pub fn sheet_tabs(app: &mut SheetApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::bottom("sheet_tabs").exact_size(32.0).frame(egui::Frame::NONE.fill(t.tab_bar)).show(ui, |ui| {
        let rect = ui.max_rect();
        ui.painter().line_segment([rect.left_top(), rect.right_top()], Stroke::new(1.0, t.ribbon_border));
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(8.0);
            if crate::widgets::icon_button(ui, Icon::Left, t.text_dim, "Previous sheet", vec2(24.0, 24.0)).clicked() {
                app.run_or_alert("sheet.previous", json!({}));
            }
            if crate::widgets::icon_button(ui, Icon::Right, t.text_dim, "Next sheet", vec2(24.0, 24.0)).clicked() {
                app.run_or_alert("sheet.next", json!({}));
            }
            let all = crate::widgets::icon_button(ui, Icon::SheetList, t.text_dim, "All sheets", vec2(24.0, 24.0));
            all.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "All sheets"));
            let items = app.session.active().map(|d| navigator_items(&d.wb.sheets)).unwrap_or_default();
            let active = app.session.active().map(|d| d.wb.active_sheet.to_string()).unwrap_or_default();
            let rows: Vec<(&str, String)> = items.iter().map(|(i, name)| (i.as_str(), name.clone())).collect();
            if let crate::ribbon::ComboResult::Picked(i) = crate::ribbon::editable_combo(ui, &all, "sheet_navigator", &active, &rows, false)
                && let Ok(i) = i.parse::<usize>()
            {
                activate_sheet(app, i);
            }
            ui.add_space(8.0);
            let Some(d) = app.session.active() else { return };
            let sheets: Vec<(usize, String, Option<[u8; 3]>, bool)> =
                d.wb.sheets
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.visibility == Visibility::Visible)
                    .map(|(i, s)| (i, s.name.clone(), s.tab_color.and_then(|c| c.resolve(&d.wb.theme)), i == d.wb.active_sheet))
                    .collect();
            // Keep worksheet tabs in their own horizontally scrollable viewport. As in Excel, the
            // tabs get 60% of the bar and the grid's horizontal scrollbar the rest.
            let tabs_width = (ui.available_width() * 0.6).max(100.0);
            let sheet_count = d.wb.sheets.len();
            let visible: Vec<usize> = sheets.iter().map(|s| s.0).collect();
            // Reveal the selected sheet after navigation or layout changes, but
            // allow browsing other tabs without snapping back on every frame.
            let reveal_key = egui::Id::new((d.uid, d.wb.active_sheet, &sheets, tabs_width.to_bits()));
            let reveal_id = ui.id().with("worksheet_tabs_reveal");
            let reveal_active = ui.ctx().data_mut(|data| {
                let changed = data.get_temp::<egui::Id>(reveal_id) != Some(reveal_key);
                data.insert_temp(reveal_id, reveal_key);
                changed
            });
            let mut tab_rects: Vec<(usize, Rect)> = vec![];
            let mut dragging: Option<(usize, bool)> = None;
            ui.allocate_ui_with_layout(vec2(tabs_width, 30.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                // Translate vertical wheel movement into horizontal
                // worksheet-tab scrolling.
                let wheel_scroll = if ui.rect_contains_pointer(ui.max_rect()) {
                    ui.input(|i| {
                        let d = i.smooth_scroll_delta;
                        if d.x.abs() > d.y.abs() { d.x } else { d.y }
                    })
                } else {
                    0.0
                };

                egui::ScrollArea::horizontal()
                    .id_salt("worksheet_tabs_horizontal")
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        if wheel_scroll.abs() > 0.0 {
                            ui.scroll_with_delta(vec2(wheel_scroll, 0.0));
                        }
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            let count = sheets.len();
                            for (k, (i, name, color, active)) in sheets.iter().cloned().enumerate() {
                                let renaming = app.grid.renaming_tab == Some(i);
                                let font = if active { theme::ui_bold(13.0) } else { theme::ui_font(13.0) };
                                let tw = ui.painter().layout_no_wrap(name.clone(), font.clone(), t.text).size().x;
                                let w = (tw + 32.0).max(72.0);
                                let (r, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click_and_drag());
                                tab_rects.push((i, r));
                                if active && reveal_active {
                                    resp.scroll_to_me(None);
                                }
                                // A gap under the tab, so it floats above the bar's edge as in the desktop strip.
                                let r = Rect::from_min_max(pos2(r.left() + 1.0, r.top() + 1.0), pos2(r.right() - 1.0, r.bottom() - 5.0));
                                let lifted = Rect::from_min_max(pos2(r.left(), r.top() - 2.0), r.max);
                                if active {
                                    ui.painter().rect_filled(lifted, 4.0, t.tab_active);
                                    ui.painter().rect_stroke(lifted, 4.0, Stroke::new(1.0, t.ribbon_border), StrokeKind::Inside);
                                } else if resp.hovered() {
                                    ui.painter().rect_filled(r, 4.0, t.hover);
                                }
                                // A thin divider between two inactive tabs.
                                if k + 1 < count && !active && sheets.get(k + 1).is_some_and(|n| !n.3) {
                                    ui.painter().vline(r.right(), (r.top() + 8.0)..=(r.bottom() - 8.0), Stroke::new(1.0, t.separator));
                                }
                                if let Some(c) = color {
                                    let band = Rect::from_min_size(pos2(r.left() + 6.0, r.bottom() - 4.0), vec2(r.width() - 12.0, 3.0));
                                    ui.painter().rect_filled(band, 1.0, theme::color32(c));
                                } else if active {
                                    let band = Rect::from_center_size(pos2(r.center().x, r.bottom() - 2.0), vec2(tw.min(48.0), 2.5));
                                    ui.painter().rect_filled(band, 1.0, t.accent);
                                }
                                if renaming {
                                    let mut text = app.grid.rename_text.clone();
                                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(6.0, 5.0))));
                                    let ed_id = egui::Id::new("sheet_tab_rename").with(i);
                                    let ed = child.add(
                                        egui::TextEdit::singleline(&mut text).id(ed_id).font(theme::ui_font(13.0)).desired_width(r.width() - 12.0),
                                    );
                                    app.grid.rename_text = text.clone();
                                    if rename_needs_focus(ed.has_focus(), ed.lost_focus()) {
                                        ed.request_focus();
                                        // Select the whole name, so typing replaces it (as in Excel).
                                        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), ed_id) {
                                            let all = egui::text::CCursorRange::two(
                                                egui::text::CCursor::new(0),
                                                egui::text::CCursor::new(text.chars().count()),
                                            );
                                            state.cursor.set_char_range(Some(all));
                                            state.store(ui.ctx(), ed_id);
                                        }
                                    }
                                    if ed.lost_focus() {
                                        if !ui.input(|i| i.key_pressed(egui::Key::Escape)) && text != name {
                                            app.run_or_alert("sheet.rename", json!({"sheet": i, "name": text}));
                                        }
                                        app.grid.renaming_tab = None;
                                    }
                                } else {
                                    ui.painter().text(r.center(), Align2::CENTER_CENTER, &name, font, if active { t.accent } else { t.text });
                                }
                                if resp.clicked() {
                                    activate_sheet(app, i);
                                }
                                if resp.double_clicked() {
                                    app.grid.renaming_tab = Some(i);
                                    app.grid.rename_text = name.clone();
                                }
                                resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, &name));
                                if resp.secondary_clicked() {
                                    // A right-click selects the tab first, as in Excel.
                                    if !active {
                                        activate_sheet(app, i);
                                    }
                                    let at = resp.interact_pointer_pos().unwrap_or(r.center_top());
                                    crate::sheet_tab_menu::open(ui.ctx(), at, i);
                                }
                                if resp.dragged() && !renaming {
                                    dragging = Some((i, false));
                                }
                                if resp.drag_stopped() && !renaming {
                                    dragging = Some((i, true));
                                }
                            }
                        });
                    });
            });
            ui.add_space(6.0);
            if crate::widgets::icon_button(ui, Icon::Plus, t.text_dim, "New sheet (⇧F11)", vec2(26.0, 26.0)).clicked() {
                app.run_or_alert("home.insertSheet", json!({}));
            }
            // Dragging a tab: a small arrow marks where it lands; holding Ctrl (Cmd) copies the sheet.
            if let Some((i, stopped)) = dragging
                && let Some(p) = ui.input(|inp| inp.pointer.interact_pos())
            {
                let copy = ui.input(|inp| inp.modifiers.command || inp.modifiers.ctrl);
                // The slot is the first tab whose middle lies after the pointer.
                let slot = tab_rects.iter().position(|(_, r)| p.x < r.center().x).unwrap_or(tab_rects.len());
                let before = tab_rects.get(slot).map_or(sheet_count, |(j, _)| *j);
                let edge = match tab_rects.get(slot) {
                    Some((_, r)) => r.left(),
                    None => tab_rects.last().map_or(p.x, |(_, r)| r.right()),
                };
                let next_visible = visible.iter().copied().find(|j| *j > i).unwrap_or(sheet_count);
                let same_place = !copy && (before == i || before == next_visible);
                if !stopped {
                    ui.ctx().set_cursor_icon(if copy { egui::CursorIcon::Copy } else { egui::CursorIcon::Grabbing });
                    if !same_place {
                        let top = rect.top() + 1.0;
                        ui.painter().add(egui::Shape::convex_polygon(
                            vec![pos2(edge - 5.0, top), pos2(edge + 5.0, top), pos2(edge, top + 6.0)],
                            t.text,
                            Stroke::NONE,
                        ));
                    }
                } else if !same_place {
                    // `sheet.move` takes the final index: a moved sheet leaves its old slot first.
                    let to = if copy || before <= i { before } else { before - 1 };
                    app.run_or_alert("sheet.move", json!({"sheet": i, "to": to, "copy": copy}));
                }
            }
            // The track ends where the grid does, so it stops under the vertical scrollbar.
            let avail = ui.available_rect_before_wrap();
            let right = app.grid.rect.map_or(avail.right(), |r| r.right()).min(avail.right());
            let track = Rect::from_min_max(pos2(avail.left() + 8.0, avail.center().y - 7.0), pos2(right, avail.center().y + 7.0));
            crate::grid::horizontal_scrollbar(app, ui, track, &t);
        });
    });
    tab_menu(app, ui.ctx());
}

/// The sheet tab's right-click menu and what its entries do.
fn tab_menu(app: &mut SheetApp, ctx: &egui::Context) {
    let color_sheet = egui::Id::new("tab_color_sheet");
    if let Some(open) = crate::sheet_tab_menu::is_open(ctx) {
        let name = app.session.active().and_then(|d| d.wb.sheets.get(open.sheet).map(|s| s.name.clone())).unwrap_or_default();
        ctx.data_mut(|d| d.insert_temp(color_sheet, name));
    }
    // A colour from the Colors window (More Colors…) lands on the sheet whose menu asked for it,
    // found by name: sheets may have moved while the window was open.
    if let Some(c) = crate::color_picker::take_result(ctx, "tabColor") {
        let name: String = ctx.data(|d| d.get_temp(color_sheet)).unwrap_or_default();
        let sheet = app.session.active().and_then(|d| d.wb.sheets.iter().position(|s| s.name == name));
        if let Some(sheet) = sheet {
            app.run_or_alert("sheet.tabColor", json!({"sheet": sheet, "color": c}));
        }
    }
    let Some(d) = app.session.active() else { return };
    let colors = d.wb.theme.colors;
    let protected = d.wb.active().is_some_and(|s| s.is_protected());
    let any_hidden = d.wb.sheets.iter().any(|s| s.visibility != Visibility::Visible);
    let name = |app: &SheetApp, i: usize| app.session.active().and_then(|d| d.wb.sheets.get(i).map(|s| s.name.clone())).unwrap_or_default();
    use crate::sheet_tab_menu::Choice;
    let Some((i, choice)) = crate::sheet_tab_menu::show(ctx, &colors, protected, any_hidden) else { return };
    match choice {
        Choice::Insert => app.run_or_alert("home.insertSheet", json!({"before": i})),
        Choice::Delete => app.run_or_alert("home.deleteSheet", json!({"sheet": i})),
        Choice::Rename => {
            app.grid.renaming_tab = Some(i);
            app.grid.rename_text = name(app, i);
        }
        Choice::MoveOrCopy => app.open_dialog("moveSheet", json!({"sheet": i})),
        Choice::Protect => app.open_dialog(if protected { "unprotectSheet" } else { "protectSheet" }, json!({})),
        Choice::TabColor(c) => app.run_or_alert("sheet.tabColor", json!({"sheet": i, "color": c})),
        Choice::Hide => app.run_or_alert("sheet.hide", json!({"sheet": i})),
        Choice::Unhide => app.run_or_alert("sheet.unhide", json!({})),
        Choice::Link => {
            // A hyperlink address to the sheet's first cell, ready to paste.
            let n = name(app, i);
            // Quoted as formulas quote it ('2024', 'A1', 'My Sheet'), so the address resolves.
            ctx.copy_text(format!("#{}!A1", gridcraft_engine::formula::quote_sheet(&n)));
        }
    }
}

pub fn status_bar(app: &mut SheetApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::bottom("status_bar").exact_size(26.0).frame(egui::Frame::NONE.fill(t.status_bar).inner_margin(egui::Margin::symmetric(12, 0))).show(
        ui,
        |ui| {
            ui.horizontal_centered(|ui| {
                let mode = match app.session.mode {
                    gridcraft_engine::Mode::Ready => "Ready",
                    gridcraft_engine::Mode::Enter => "Enter",
                    gridcraft_engine::Mode::Edit => "Edit",
                    gridcraft_engine::Mode::Point => "Point",
                };
                ui.label(egui::RichText::new(app.ui.language.tr(mode)).font(theme::ui_font(12.5)).color(t.text_dim));
                if let Some(d) = app.session.active() {
                    if d.wb.vba_project.is_some() {
                        ui.add_space(12.0);
                        ui.label(egui::RichText::new("Macros preserved (not executed)").font(theme::ui_font(12.5)).color(t.danger));
                    }
                    if d.wb.calc.mode == gridcraft_engine::model::CalcMode::Manual {
                        ui.add_space(12.0);
                        ui.label(egui::RichText::new(app.ui.language.tr("Calculate")).font(theme::ui_font(12.5)).color(t.text_dim));
                    }
                    if !d.calc.circular.is_empty() {
                        ui.add_space(12.0);
                        let (si, c) = d.calc.circular[0];
                        let name = d.wb.sheet(si).map(|s| s.name.clone()).unwrap_or_default();
                        ui.label(egui::RichText::new(format!("Circular References: {name}!{}", c.a1())).font(theme::ui_font(12.5)).color(t.danger));
                    }
                    if app.session.clipboard.is_some() && app.editor.is_none() {
                        ui.add_space(12.0);
                        ui.label(
                            egui::RichText::new(app.ui.language.tr("Select destination and press Enter or choose Paste"))
                                .font(theme::ui_font(12.5))
                                .color(t.text_dim),
                        );
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Zoom percentage and slider.
                    let zoom = app.session.active().and_then(|d| d.wb.active().map(|s| s.zoom)).unwrap_or(100);
                    let zl = ui.add(
                        egui::Label::new(egui::RichText::new(format!("{zoom}%")).font(theme::ui_font(12.5)).color(t.text_dim)).sense(Sense::click()),
                    );
                    if zl.clicked() {
                        app.open_dialog("zoom", json!({}));
                    }
                    ui.add_space(4.0);
                    if crate::widgets::icon_button(ui, Icon::Plus, t.text_dim, "Zoom In", vec2(18.0, 18.0)).clicked() {
                        let z = ((zoom as f32 / 10.0).floor() * 10.0 + 10.0).min(400.0);
                        app.run_or_alert("view.zoom", json!({"percent": z}));
                    }
                    let mut z = zoom as f32;
                    let slider = ui.add(egui::Slider::new(&mut z, 10.0..=400.0).show_value(false).logarithmic(true));
                    if slider.changed() {
                        let _ = app.session.run("view.zoom", json!({"percent": z.round()}));
                    }
                    if crate::widgets::icon_button(ui, Icon::Minus, t.text_dim, "Zoom Out", vec2(18.0, 18.0)).clicked() {
                        let z = ((zoom as f32 / 10.0).ceil() * 10.0 - 10.0).max(10.0);
                        app.run_or_alert("view.zoom", json!({"percent": z}));
                    }
                    ui.add_space(10.0);
                    for (icon, tip, mode, cmd) in [
                        (Icon::PageBreak, "Page Break Preview", "pageBreakPreview", "view.pageBreakPreview"),
                        (Icon::PageLayout, "Page Layout", "pageLayout", "view.pageLayout"),
                        (Icon::Normal, "Normal", "normal", "view.normal"),
                    ] {
                        let (r, resp) = ui.allocate_exact_size(vec2(26.0, 20.0), Sense::click());
                        let cur = app.session.active().and_then(|d| d.wb.active().map(|s| s.view.as_str())).unwrap_or("normal");
                        if cur == mode {
                            ui.painter().rect_filled(r, 3.0, t.pressed);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(r, 3.0, t.hover);
                        }
                        icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(14.0, 14.0)), icon, t.text_dim);
                        if resp.on_hover_text(tip).clicked() {
                            app.run_or_alert(cmd, json!({}));
                        }
                    }
                    ui.add_space(16.0);
                    // Average / Count / Sum.
                    if let Some(d) = app.session.active()
                        && let Some(sh) = d.wb.active()
                        && !d.selection.is_single_cell()
                    {
                        let st = gridcraft_engine::display::stats(sh, &d.selection.ranges);
                        let fmt = |v: f64| {
                            let code = d.wb.styles.get(sh.style_id(d.selection.active)).num_fmt.as_str().to_string();
                            let code = if code == "General" { "#,##0.##########".to_string() } else { code };
                            gridcraft_engine::display::format(&gridcraft_engine::core::Value::Number(v), &code, &d.wb)
                                .text
                                .trim_end_matches('.')
                                .to_string()
                        };
                        let mut parts = Vec::new();
                        if st.numeric > 0 {
                            parts.push(format!("Sum: {}", fmt(st.sum)));
                        }
                        if st.count > 0 {
                            parts.push(format!("Count: {}", st.count));
                        }
                        if let Some(a) = st.average {
                            parts.push(format!("Average: {}", fmt(a)));
                        }
                        for p in parts {
                            ui.label(egui::RichText::new(p).font(theme::ui_font(12.5)).color(t.text_dim));
                            ui.add_space(10.0);
                        }
                    }
                });
            });
        },
    );
}

#[cfg(test)]
mod tests {
    use super::{navigator_items, rename_needs_focus};
    use gridcraft_engine::model::{Sheet, Visibility};
    use std::sync::Arc;

    #[test]
    fn rename_field_takes_focus_once_and_lets_go_on_commit() {
        assert!(rename_needs_focus(false, false), "first frame: grab focus");
        assert!(!rename_needs_focus(true, false), "already focused: do not ask again");
        assert!(!rename_needs_focus(false, true), "Enter or click-away: must not take focus back");
    }

    #[test]
    fn navigator_lists_visible_sheets_by_workbook_index() {
        let mut hidden = Sheet::new("Hidden");
        hidden.visibility = Visibility::Hidden;
        let mut very_hidden = Sheet::new("VeryHidden");
        very_hidden.visibility = Visibility::VeryHidden;
        let sheets = [Sheet::new("Data"), hidden, very_hidden, Sheet::new("Summary")].map(Arc::new);
        assert_eq!(navigator_items(&sheets), vec![("0".to_string(), "Data".to_string()), ("3".to_string(), "Summary".to_string())]);
    }
}
