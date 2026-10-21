//! Title bar (Quick Access Toolbar), ribbon tabs and ribbon groups, and keyboard shortcuts.

use egui::{Align2, Color32, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use gridcraft_engine::model::{HAlign, Style, VAlign};
use serde_json::json;

use crate::SheetApp;
use crate::icons::{self, Icon};
use crate::keytips::keytip_table;
use crate::theme::{self, Tokens};
use crate::widgets::{big_button, color_palette, icon_button, small_button, split_button, toggle_button};

pub const TABS: &[&str] = &["Home", "Insert", "Draw", "Page Layout", "Formulas", "Data", "Review", "View", "Automate", "Developer", "Help"];

/// The desktop-layout tabs.
#[path = "ribbon_tabs.rs"]
mod desk;

/// macOS draws the window into the title bar (`with_fullsize_content_view`) and AppKit paints the
/// traffic lights itself, centred in its standard 28 pt title-bar band. Moving them would take
/// AppKit calls (`unsafe`, forbidden here), so on macOS the title bar is that band: the lights sit
/// vertically centred in it, with the controls beside them (#170).
const MAC: bool = cfg!(target_os = "macos");
pub const MAC_TITLE_BAR: f32 = 28.0;
/// The traffic lights as AppKit lays them out: button diameter, centre-to-centre pitch and the
/// first button's left edge (8–10 pt depending on the macOS version; the larger value keeps the
/// gap after them from shrinking).
const MAC_LIGHT: f32 = 12.0;
const MAC_LIGHT_PITCH: f32 = 20.0;
const MAC_LIGHTS_LEFT: f32 = 10.0;
/// Where title-bar content starts on macOS: the end of the three lights plus the gap the lights
/// keep from the top of the window.
pub const MAC_CONTENT_LEFT: f32 = MAC_LIGHTS_LEFT + 2.0 * MAC_LIGHT_PITCH + MAC_LIGHT + (MAC_TITLE_BAR - MAC_LIGHT) / 2.0;

/// Leave room for the macOS traffic lights when the content extends into the title bar.
pub const TITLE_LEFT_PAD: f32 = if MAC { MAC_CONTENT_LEFT } else { 8.0 };
const TITLE_BAR_H: f32 = if MAC { MAC_TITLE_BAR } else { 38.0 };
/// The tallest title-bar controls, smaller on macOS to keep a margin in the shorter bar.
const TITLE_CONTROL_H: f32 = if MAC { 22.0 } else { 26.0 };
const _: () = assert!(TITLE_CONTROL_H < TITLE_BAR_H, "title-bar controls keep a margin above and below");

pub fn title_bar(app: &mut SheetApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let ch = TITLE_CONTROL_H;
    egui::Panel::top("title_bar").show_separator_line(false).exact_size(TITLE_BAR_H).frame(egui::Frame::NONE.fill(t.window)).show(ui, |ui| {
        let rect = ui.max_rect();
        // Dragging the empty title area moves the window.
        let bg = ui.interact(rect, ui.id().with("drag_title"), Sense::click_and_drag());
        if bg.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if bg.double_clicked() {
            let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
        }
        ui.horizontal_centered(|ui| {
            ui.add_space(TITLE_LEFT_PAD);
            if !MAC {
                let (r, _) = ui.allocate_exact_size(vec2(20.0, ch), Sense::hover());
                crate::ribbon_extra::app_mark(ui.painter(), Rect::from_center_size(r.center(), vec2(18.0, 18.0)));
                ui.add_space(8.0);
            }
            // AutoSave toggle (saves after each change when the workbook has a file).
            ui.label(egui::RichText::new("AutoSave").font(theme::ui_font(12.5)).color(t.text_dim));
            let on = app.grid.autosave();
            let (r, resp) = ui.allocate_exact_size(vec2(30.0, 16.0), Sense::click());
            ui.painter().rect_filled(r, 8.0, if on { t.accent } else { Color32::TRANSPARENT });
            ui.painter().rect_stroke(r, 8.0, Stroke::new(1.0, if on { t.accent } else { t.text_dim }), StrokeKind::Inside);
            let knob = if on { pos2(r.right() - 8.0, r.center().y) } else { pos2(r.left() + 8.0, r.center().y) };
            ui.painter().circle_filled(knob, 5.0, if on { Color32::WHITE } else { t.text_dim });
            if resp.on_hover_text("AutoSave: save after every change (workbooks saved to a file)").clicked() {
                app.grid.toggle_autosave();
            }
            ui.add_space(6.0);
            if !app.ui.qat_hidden {
                quick_access(app, ui, ch);
            }
            let qat_right = ui.cursor().min.x;
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("title_qat_rect"), Rect::from_x_y_ranges(rect.left()..=qat_right, rect.y_range())));
            // Centered title, dim as in desktop suites: "Book1 - GridCraft".
            let title = app.session.active().map(|d| d.display_title()).unwrap_or_default();
            let dirty = app.session.active().is_some_and(|d| d.is_dirty());
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                format!("{title}{} - GridCraft", if dirty { " •" } else { "" }),
                theme::ui_font(13.0),
                t.text_dim,
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                let __k = icon_button(ui, Icon::Search, t.text_dim, "Search commands (⌘⇧Space)", vec2(28.0, ch));
                control_keytip(app, ui, &__k, "commandSearch");
                if __k.clicked() {
                    app.open_dialog("commandSearch", json!({}));
                }
                let share = small_button(ui, Icon::Share, "Share", "Share: export or save a copy", true);
                egui::Popup::menu(&share).show(|ui| {
                    if ui.button("Export as PDF…").clicked() {
                        app.run_or_alert("file.exportPdf", json!({}));
                    }
                    for (label, fmt) in
                        [("Save a Copy as Excel Workbook (.xlsx)…", "xlsx"), ("Export as CSV…", "csv"), ("Export as Web Page (.html)…", "html")]
                    {
                        if ui.button(label).clicked() {
                            app.open_dialog("saveCopy", json!({"format": fmt}));
                        }
                    }
                });
                let __k = icon_button(ui, Icon::Comment, t.text_dim, "Comments", vec2(28.0, ch));
                control_keytip(app, ui, &__k, "comments");
                if __k.clicked() {
                    app.open_dialog("comments", json!({}));
                }
                // Quick light/dark switch (#194); right-click offers System as well. The same
                // choice lives in File › Display theme and the View tab.
                let lang = app.ui.language;
                let theme = icon_button(ui, Icon::Appearance, t.text_dim, "Dark Mode", vec2(28.0, 26.0));
                let dark = ui.ctx().theme() == egui::Theme::Dark;
                theme.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, dark, lang.tr("Dark Mode")));
                if theme.clicked() {
                    act(app, "view.darkMode", json!({}));
                }
                theme.context_menu(|ui| theme_choices(app, ui));
            });
        });
    });
}

/// The Quick Access Toolbar in the title bar: Home, Save, Undo, Redo and its customize menu.
fn quick_access(app: &mut SheetApp, ui: &mut Ui, ch: f32) {
    let t = Tokens::get(ui.ctx());
    let __k = icon_button(ui, Icon::Home, t.text_dim, "Home", vec2(26.0, ch));
    control_keytip(app, ui, &__k, "start");
    if __k.clicked() {
        app.open_dialog("start", json!({}));
    }
    if icon_button(ui, Icon::Save, t.text_dim, "Save (⌘S)", vec2(26.0, ch)).clicked()
        // File operations must include the cell the user is still editing (as the File > Save menu item already does).
        && app.commit_edit(0, 0, false, false)
    {
        app.run_or_alert("file.save", json!({}));
    }
    let can_undo = app.session.active().is_some_and(|d| !d.undo.is_empty());
    let can_redo = app.session.active().is_some_and(|d| !d.redo.is_empty());
    if icon_button(ui, Icon::Undo, if can_undo { t.text_dim } else { t.text_disabled }, "Undo (⌘Z)", vec2(26.0, ch)).clicked() && can_undo {
        app.run_or_alert("edit.undo", json!({}));
    }
    let undo_list = icon_button(ui, Icon::Chevron, t.text_dim, "Undo list", vec2(14.0, ch));
    egui::Popup::menu(&undo_list).show(|ui| {
        let labels: Vec<String> = app.session.active().map(|d| d.undo.iter().rev().take(20).map(|e| e.label.clone()).collect()).unwrap_or_default();
        if labels.is_empty() {
            ui.label("Can't Undo");
        }
        for (i, l) in labels.iter().enumerate() {
            if ui.button(format!("Undo {l}")).clicked() {
                app.run_or_alert("edit.undo", json!({"steps": i + 1}));
            }
        }
    });
    if icon_button(ui, Icon::Redo, if can_redo { t.text_dim } else { t.text_disabled }, "Redo (⌘Y)", vec2(26.0, ch)).clicked() && can_redo {
        app.run_or_alert("edit.redo", json!({}));
    }
    let redo_list = icon_button(ui, Icon::Chevron, t.text_dim, "Redo list", vec2(14.0, ch));
    egui::Popup::menu(&redo_list).show(|ui| {
        let labels: Vec<String> = app.session.active().map(|d| d.redo.iter().rev().take(20).map(|e| e.label.clone()).collect()).unwrap_or_default();
        if labels.is_empty() {
            ui.label("Can't Redo");
        }
        for (i, l) in labels.iter().enumerate() {
            if ui.button(format!("Redo {l}")).clicked() {
                app.run_or_alert("edit.redo", json!({"steps": i + 1}));
            }
        }
    });
    let more = icon_button(ui, Icon::Chevron, t.text_dim, "Customize Quick Access Toolbar", vec2(18.0, ch));
    egui::Popup::menu(&more).show(|ui| {
        ui.label(egui::RichText::new("Customize Quick Access Toolbar").strong());
        ui.separator();
        for (label, id) in [
            ("New Workbook", "file.new"),
            ("Open…", "file.open"),
            ("Save As…", "file.saveAs"),
            ("Sort A to Z", "data.sortAscending"),
            ("Calculate Now", "formulas.calculateNow"),
        ] {
            if ui.button(label).clicked() {
                app.run_or_alert(id, json!({}));
            }
        }
        if ui.button("Print…").clicked() {
            app.print_now(json!({}));
        }
        ui.separator();
        if ui.button("Hide Quick Access Toolbar").clicked() {
            app.ui.qat_hidden = true;
        }
    });
}

pub fn show(app: &mut SheetApp, ui: &mut Ui) {
    show_ribbon(app, ui);
    let ribbon: Option<Rect> = ui.ctx().data(|d| d.get_temp(egui::Id::new("ribbon_rect")));
    let qat: Option<Rect> = ui.ctx().data(|d| d.get_temp(egui::Id::new("title_qat_rect")));
    let areas: Vec<Rect> = ribbon.into_iter().chain(qat).collect();
    match crate::ribbon_extra::context_menu(ui.ctx(), &areas, app.ui.qat_hidden, app.ui.ribbon_collapsed) {
        Some(crate::ribbon_extra::RibbonMenu::ToggleQuickAccess) => app.ui.qat_hidden = !app.ui.qat_hidden,
        Some(crate::ribbon_extra::RibbonMenu::Customize) => app.open_dialog("options", json!({})),
        Some(crate::ribbon_extra::RibbonMenu::ToggleCollapse) => app.ui.ribbon_collapsed = !app.ui.ribbon_collapsed,
        None => {}
    }
}

fn show_ribbon(app: &mut SheetApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let collapsed = app.ui.ribbon_collapsed;
    let h = if collapsed { 34.0 } else { 34.0 + 116.0 };
    egui::Panel::top("ribbon")
        .show_separator_line(false)
        .exact_size(h)
        .frame(egui::Frame::NONE.fill(t.window).inner_margin(egui::Margin { left: 10, right: 10, top: 0, bottom: 6 }))
        .show(ui, |ui| {
            let whole = ui.max_rect();
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("ribbon_rect"), whole));
            // Tabs row.
            let tabs_rect = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), 32.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(tabs_rect), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    ui.add_space(8.0);
                    if app.chrome.file_tab {
                        file_menu(app, ui);
                    }
                    let mut tabs: Vec<&str> = TABS.to_vec();
                    let ctx_tabs = contextual_tabs(app);
                    tabs.extend(ctx_tabs.iter());
                    for tab in tabs {
                        let active = app.ui.ribbon_tab == tab;
                        let contextual = ctx_tabs.contains(&tab);
                        let font = theme::ui_font(13.5);
                        let label = app.ui.language.tr(tab);
                        let tw = ui.painter().layout_no_wrap(label.to_string(), font.clone(), t.text).size().x;
                        let (r, resp) = ui.allocate_exact_size(vec2(tw + 22.0, 30.0), Sense::click());
                        // Named for screen readers (and tests), selected when it is the shown tab.
                        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, active, label));
                        if resp.hovered() && !active {
                            ui.painter().rect_filled(r.shrink2(vec2(2.0, 4.0)), 5.0, t.hover);
                        }
                        let color = if contextual {
                            t.accent
                        } else if active {
                            t.text
                        } else {
                            t.text_dim
                        };
                        ui.painter().text(r.center(), Align2::CENTER_CENTER, label, if active { theme::ui_bold(13.5) } else { font }, color);
                        if active {
                            let ul = Rect::from_center_size(pos2(r.center().x, r.bottom() - 3.0), vec2(tw.clamp(20.0, 40.0), 3.0));
                            ui.painter().rect_filled(ul, 1.5, t.accent);
                        }
                        if resp.clicked() {
                            if active && !collapsed {
                                app.ui.ribbon_collapsed = true;
                            } else {
                                app.ui.ribbon_tab = tab.to_string();
                                app.ui.ribbon_collapsed = false;
                            }
                        }
                        if resp.double_clicked() {
                            app.ui.ribbon_collapsed = !collapsed;
                        }
                        if app.keytips.prefix() == Some("")
                            && let Some(sequence) = keytip_table::tab_sequence(tab)
                        {
                            keytip(app, ui, sequence, pos2(r.center().x, r.bottom() - 1.0));
                        }
                    }
                });
            });
            if collapsed {
                return;
            }
            // Ribbon card.
            let card = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), 106.0));
            let shadow = egui::epaint::Shadow { offset: [0, 2], blur: 8, spread: 0, color: t.shadow };
            ui.painter().add(shadow.as_shape(card, egui::CornerRadius::same(8)));
            ui.painter().rect_filled(card, 8.0, t.ribbon);
            ui.painter().rect_stroke(card, 8.0, Stroke::new(1.0, t.ribbon_border), StrokeKind::Inside);
            let inner = card.shrink2(vec2(10.0, 5.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                egui::ScrollArea::horizontal().scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
                        crate::ribbon_extra::begin_groups(ui, inner);
                        match app.ui.ribbon_tab.as_str() {
                            "Home" => home(app, ui),
                            "Insert" => desk::insert(app, ui),
                            "Draw" => desk::draw(app, ui),
                            "Page Layout" => page_layout(app, ui),
                            "Formulas" => desk::formulas(app, ui),
                            "Data" => desk::data(app, ui),
                            "Review" => desk::review(app, ui),
                            "View" => desk::view(app, ui),
                            "Automate" => desk::automate(app, ui),
                            "Developer" => desk::developer(app, ui),
                            "Help" => desk::help(app, ui),
                            "Table Design" => table_design(app, ui),
                            "Chart Design" => chart_design(app, ui),
                            crate::paper_view::TAB => crate::paper_view::ribbon_tab(app, ui),
                            _ => home(app, ui),
                        }
                    });
                });
            });
        });
}

/// The File tab opens the Backstage view (New, Open, Info, Save As, Print, Export, Options…), like
/// Excel's; the ribbon's tab and collapsed state are left as they were for when it closes.
fn file_menu(app: &mut SheetApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let label = egui::RichText::new(app.ui.language.tr("File")).font(theme::ui_font(13.5)).color(t.accent);
    let file = ui.add(egui::Button::new(label).frame(false).min_size(vec2(46.0, 30.0)));
    if file.clicked() {
        app.open_backstage(crate::backstage::Page::Home);
    }
}

/// System / Light / Dark, as in the View tab's Display theme box (`view.theme`).
fn theme_choices(app: &mut SheetApp, ui: &mut Ui) {
    let lang = app.ui.language;
    let mode = app.ui.theme_mode();
    for (value, label) in [("system", "System"), ("light", "Light"), ("dark", "Dark")] {
        if ui.radio(mode == value, lang.tr(label)).clicked() {
            act(app, "view.theme", json!({"mode": value}));
            ui.close();
        }
    }
}

fn contextual_tabs(app: &SheetApp) -> Vec<&'static str> {
    let mut v = Vec::new();
    if let Some(d) = app.session.active()
        && let Some(sh) = d.wb.active()
    {
        if sh.table_at(d.selection.active).is_some() {
            v.push("Table Design");
        }
        if app.selected_chart.is_some_and(|id| sh.charts.iter().any(|c| c.id == id)) {
            v.push("Chart Design");
        }
        if app.grid.hf_edit.is_some() {
            v.push(crate::paper_view::TAB);
        }
    }
    v
}

fn active_style(app: &SheetApp) -> Style {
    app.session
        .active()
        .and_then(|d| {
            let sh = d.wb.active()?;
            Some(d.wb.styles.get(sh.style_id(d.selection.active)).clone())
        })
        .unwrap_or_default()
}

fn theme_colors(app: &SheetApp) -> [u32; 12] {
    app.session.active().map(|d| d.wb.theme.colors).unwrap_or(gridcraft_engine::model::Theme::default().colors)
}

/// Runs a command from a ribbon click (menu-style: no params may open a dialog).
fn act(app: &mut SheetApp, id: &str, params: serde_json::Value) {
    app.run_or_alert(id, params);
}

/// Keytips are painted, not focusable widgets: the grid retains keyboard focus.
fn keytip(app: &SheetApp, ui: &Ui, sequence: &str, at: egui::Pos2) {
    let Some(rest) = app.keytips.prefix().and_then(|prefix| sequence.strip_prefix(prefix)).filter(|s| !s.is_empty()) else { return };
    let t = Tokens::get(ui.ctx());
    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("ribbon_keytips"))).with_clip_rect(ui.clip_rect());
    let text = painter.layout_no_wrap(rest.into(), theme::ui_bold(10.5), t.text);
    let rect = Rect::from_center_size(at, text.size() + vec2(7.0, 3.0));
    painter.rect_filled(rect, 2.0, t.input_bg);
    painter.rect_stroke(rect, 2.0, Stroke::new(1.0, t.text_dim), StrokeKind::Inside);
    painter.galley(rect.min + vec2(3.5, 1.5), text, t.text);
}

/// Paint the badge for a control that runs command `id`, when the armed prefix matches.
fn control_keytip(app: &SheetApp, ui: &Ui, response: &egui::Response, id: &str) {
    if let Some(sequence) = keytip_table::control_badge(app.keytips.prefix().unwrap_or_default(), id) {
        keytip(app, ui, sequence, response.rect.center_bottom());
    }
}

/// Paint the single badge for a menu anchor whose popup items run `targets`.
fn anchor_keytip(app: &SheetApp, ui: &Ui, response: &egui::Response, targets: &[&str]) {
    if let Some(sequence) = keytip_table::anchor_badge(app.keytips.prefix().unwrap_or_default(), targets) {
        keytip(app, ui, sequence, response.rect.center_bottom());
    }
}

/// The `on` value for a ribbon checkbox command. A click has already flipped the checkbox bool, so
/// it is sent as is; a key tip leaves the bool untouched, so the command gets the opposite.
fn checkbox_on(clicked: bool, shown: bool) -> bool {
    shown == clicked
}

/// Consume a one-shot keytip request for control `id` (set by `Action::Control`).
fn invoke(app: &mut SheetApp, id: &str) -> bool {
    if app.ui.keytip_invoke == Some(id) {
        app.ui.keytip_invoke = None;
        true
    } else {
        false
    }
}

/// Force-open command for a popup: either a keytip invoke (open it) or a click (toggle).
fn popup_command(forced: bool, clicked: bool) -> Option<egui::SetOpenCommand> {
    if forced { Some(egui::SetOpenCommand::Bool(true)) } else { clicked.then_some(egui::SetOpenCommand::Toggle) }
}

/// Per-control state of an [`editable_combo`]: the filter text, highlighted row, and whether an
/// IME composition is live on the filter field.
#[derive(Clone, Default)]
struct ComboState {
    text: String,
    sel: i32,
    was_open: bool,
    composing: bool,
}

/// What the user did with an [`editable_combo`].
pub(crate) enum ComboResult {
    None,
    /// A row was applied: clicked or Enter on the highlighted row.
    Picked(String),
    /// Enter with free text that matches no row (e.g. a custom font size).
    Entered(String),
}

/// Excel-style dropdown: a filter field (type to narrow the list) plus arrow/Enter selection.
/// `items` are `(value, display)`; the value is what comes back. When `forced` (keytip invoke)
/// or clicked, the popup opens, focuses the filter field, and selects its text.
pub(crate) fn editable_combo(
    ui: &mut Ui,
    anchor: &egui::Response,
    salt: &'static str,
    current_value: &str,
    items: &[(&str, String)],
    forced: bool,
) -> ComboResult {
    let popup_id = anchor.id.with("popup");
    let state_id = anchor.id.with(salt).with("__combo_state");
    let mut st = ui.data_mut(|d| d.get_temp::<ComboState>(state_id)).unwrap_or_default();
    let open_now = egui::Popup::is_id_open(ui.ctx(), popup_id);
    if open_now && !st.was_open {
        st.text = String::new();
        st.sel = items.iter().position(|(v, _)| *v == current_value).map(|i| i as i32).unwrap_or(0);
        // The reset lands on the frame AFTER the popup opens, which usually has no input of
        // its own — without a repaint request the stale filtered list stays on screen.
        ui.ctx().request_repaint();
    }
    st.was_open = open_now;
    let mut result = ComboResult::None;
    egui::Popup::menu(anchor).open_memory(popup_command(forced, anchor.clicked())).show(|ui| {
        // Arrows and Enter navigate the list, not the filter field's cursor. Capture them and
        // strip them from the input so the TextEdit never sees them — except while an IME
        // composition is live: those keys belong to the conversion window, and the Enter that
        // confirms a conversion must not fire the list.
        let (mut down, mut up, mut enter) = (false, false, false);
        let mut ime_this_frame = false;
        ui.input(|i| {
            for e in &i.events {
                match e {
                    egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => st.composing = !text.is_empty(),
                    egui::Event::Ime(egui::ImeEvent::Commit(_)) => st.composing = false,
                    egui::Event::Ime(_) => {}
                    _ => continue,
                }
                ime_this_frame = true;
            }
            if !(st.composing || ime_this_frame) {
                down = i.key_pressed(egui::Key::ArrowDown);
                up = i.key_pressed(egui::Key::ArrowUp);
                enter = i.key_pressed(egui::Key::Enter);
            }
        });
        if down || up || enter {
            ui.ctx().input_mut(|i| {
                i.events.retain(|e| {
                    !matches!(e, egui::Event::Key { key: egui::Key::ArrowDown | egui::Key::ArrowUp | egui::Key::Enter, pressed: true, .. })
                });
            });
        }
        let text_id = anchor.id.with(salt).with("__combo_text");
        let te = ui.add(egui::TextEdit::singleline(&mut st.text).id(text_id).desired_width(220.0).hint_text("Type to filter…"));
        if open_now && !te.has_focus() {
            ui.ctx().memory_mut(|m| m.request_focus(text_id));
            // Select all once focus lands so typing replaces the filter.
            if let Some(mut s) = egui::text_edit::TextEditState::load(ui.ctx(), text_id) {
                let len = st.text.chars().count();
                s.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(len))));
                s.store(ui.ctx(), text_id);
            }
        }
        let lower = st.text.to_lowercase();
        let filtered: Vec<usize> = items.iter().enumerate().filter(|(_, (_, d))| d.to_lowercase().contains(&lower)).map(|(i, _)| i).collect();
        if !filtered.is_empty() {
            let n = filtered.len() as i32;
            if down {
                st.sel = (st.sel + 1).rem_euclid(n);
            }
            if up {
                st.sel = (st.sel - 1).rem_euclid(n);
            }
        }
        if enter {
            if filtered.is_empty() {
                let t = st.text.trim().to_string();
                if !t.is_empty() {
                    result = ComboResult::Entered(t);
                }
            } else {
                let idx = filtered[st.sel.clamp(0, filtered.len() as i32 - 1) as usize];
                result = ComboResult::Picked(items[idx].0.to_string());
            }
        }
        ui.separator();
        egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
            for i in &filtered {
                let (value, disp) = &items[*i];
                let row = ui.add(egui::Button::selectable(*i as i32 == st.sel, egui::RichText::new(disp)).min_size(vec2(220.0, 20.0)));
                if row.clicked() {
                    result = ComboResult::Picked(value.to_string());
                }
                if row.hovered() {
                    st.sel = *i as i32;
                }
            }
        });
        if !matches!(result, ComboResult::None) {
            ui.close();
        }
    });
    ui.data_mut(|d| d.insert_temp(state_id, st));
    result
}

fn menu_items(app: &mut SheetApp, ui: &mut Ui, items: &[(&str, &str, serde_json::Value)]) {
    for (label, id, p) in items {
        if *label == "-" {
            ui.separator();
            continue;
        }
        let lang = app.ui.language;
        let response = ui.add(egui::Button::new(lang.tr(label)).frame(false).min_size(vec2(220.0, 22.0)));
        if let Some(sequence) = keytip_table::item_letter(app.keytips.prefix().unwrap_or_default(), id) {
            keytip(app, ui, sequence, pos2(response.rect.right() - 10.0, response.rect.center().y));
        }
        if response.clicked() {
            if let Some(d) = id.strip_prefix("dialog:") {
                app.open_dialog(d, p.clone());
            } else if *id == "edit.paste" {
                app.paste_from_clipboard();
            } else {
                act(app, id, p.clone());
            }
        }
    }
}

/// A 24-point icon button drawing one of [`crate::ribbon_extra::RibIcon`].
fn rib_icon_button(ui: &mut Ui, icon: crate::ribbon_extra::RibIcon, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 24.0), Sense::click());
    let t = Tokens::get(ui.ctx());
    if resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, 4.0, t.pressed);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    crate::ribbon_extra::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(18.0, 16.0)), icon, t.text);
    let label = app_tr(ui, tip);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    resp.on_hover_text(label)
}

fn app_tr(ui: &Ui, text: &str) -> String {
    crate::i18n::current(ui.ctx()).tr(text).to_string()
}

fn home(app: &mut SheetApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let st = active_style(app);
    // Clipboard
    let paste = big_button(ui, Icon::Paste, "Paste", "Paste (⌘V)", true);
    paste.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Paste"));
    for sequence in
        keytip_table::group_extension(app.keytips.prefix().unwrap_or_default(), &["edit.paste", "edit.pasteSpecial", "dialog:pasteSpecial"])
    {
        keytip(app, ui, sequence, paste.rect.center_bottom());
    }
    if paste.clicked() {
        app.paste_from_clipboard();
    }
    let paste_keys = matches!(app.keytips.prefix(), Some("HV" | "E"));
    // Keep a keyboard-opened menu anchored to the button when a pointer click
    // cancels keytips, so its items remain in place and can still be clicked.
    let anchor_id = paste.id.with("keytip_anchor");
    // Query response state before taking the egui context write lock.
    // Calling secondary_clicked() inside data_mut can deadlock.
    let paste_secondary_clicked = paste.secondary_clicked();
    let anchored = ui.ctx().data_mut(|data| {
        let anchored = if paste_keys {
            true
        } else if paste_secondary_clicked {
            false
        } else {
            data.get_temp::<bool>(anchor_id).unwrap_or(false)
        };
        data.insert_temp(anchor_id, anchored);
        anchored
    });
    let mut paste_popup = egui::Popup::context_menu(&paste);
    if anchored {
        paste_popup = paste_popup.anchor(&paste);
    }
    if paste_keys {
        paste_popup = paste_popup.open_memory(egui::SetOpenCommand::Bool(true));
    }
    paste_popup.show(|ui| {
        menu_items(
            app,
            ui,
            &[
                ("Paste", "edit.paste", json!({})),
                ("Paste Values", "edit.pasteSpecial", json!({"what": "values"})),
                ("Paste Formulas", "edit.pasteSpecial", json!({"what": "formulas"})),
                ("Paste Formatting", "edit.pasteSpecial", json!({"what": "formats"})),
                ("Transpose", "edit.pasteSpecial", json!({"what": "all", "transpose": true})),
                ("Paste Link", "edit.pasteSpecial", json!({"what": "all", "link": true})),
                ("-", "", json!(null)),
                ("Paste Special…", "dialog:pasteSpecial", json!({})),
            ],
        );
    });
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let ic = |icon: Icon| move |p: &egui::Painter, r: Rect, c: Color32| icons::paint(p, r, icon, c);
        let __k = crate::ribbon_extra::small_toggle(ui, ic(Icon::Cut), "Cut", "Cut (⌘X)", false, false);
        control_keytip(app, ui, &__k, "edit.cut");
        if __k.clicked() {
            app.copy_to_clipboard(ui.ctx(), "edit.cut");
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let __k = crate::ribbon_extra::small_toggle(ui, ic(Icon::Copy), "Copy", "Copy (⌘C)", false, false);
            control_keytip(app, ui, &__k, "edit.copy");
            if __k.clicked() {
                app.copy_to_clipboard(ui.ctx(), "edit.copy");
            }
            let more = icon_button(ui, Icon::Chevron, t.text_dim, "Copy options", vec2(14.0, 24.0));
            egui::Popup::menu(&more).show(|ui| {
                if ui.button(app.ui.language.tr("Copy")).clicked() {
                    app.copy_to_clipboard(ui.ctx(), "edit.copy");
                }
                ui.add_enabled(false, egui::Button::new(app.ui.language.tr("Copy as Picture…")));
            });
        });
        let fp_on = app.session.format_painter.is_some();
        let fp =
            crate::ribbon_extra::small_toggle(ui, ic(Icon::Brush), "Format Painter", "Format Painter (double-click to keep it on)", fp_on, false);
        control_keytip(app, ui, &fp, "edit.formatPainter");
        if fp.double_clicked() {
            act(app, "edit.formatPainter", json!({"sticky": true}));
        } else if fp.clicked() {
            if fp_on {
                app.session.format_painter = None;
            } else {
                act(app, "edit.formatPainter", json!({}));
            }
        }
    });
    crate::ribbon_extra::end_group(ui, "Clipboard", false);
    // Font
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            font_combo(app, ui, &st);
            size_combo(app, ui, &st);
            let __k = rib_icon_button(ui, crate::ribbon_extra::RibIcon::GrowFont, "Increase Font Size");
            control_keytip(app, ui, &__k, "home.increaseFontSize");
            if __k.clicked() {
                act(app, "home.increaseFontSize", json!({}));
            }
            let __k = rib_icon_button(ui, crate::ribbon_extra::RibIcon::ShrinkFont, "Decrease Font Size");
            control_keytip(app, ui, &__k, "home.decreaseFontSize");
            if __k.clicked() {
                act(app, "home.decreaseFontSize", json!({}));
            }
        });
        ui.horizontal(|ui| {
            let __k = toggle_button(ui, Icon::Bold, st.font.bold, "Bold (⌘B)");
            control_keytip(app, ui, &__k, "home.bold");
            if __k.clicked() {
                act(app, "home.bold", json!({}));
            }
            let __k = toggle_button(ui, Icon::Italic, st.font.italic, "Italic (⌘I)");
            control_keytip(app, ui, &__k, "home.italic");
            if __k.clicked() {
                act(app, "home.italic", json!({}));
            }
            let (u, ua) = split_button(ui, Icon::Underline, None, "Underline (⌘U)");
            control_keytip(app, ui, &ua, "home.underline");
            if u {
                act(app, "home.underline", json!({}));
            }
            egui::Popup::menu(&ua).show(|ui| {
                menu_items(
                    app,
                    ui,
                    &[
                        ("Underline", "home.underline", json!({"style": "single"})),
                        ("Double Underline", "home.underline", json!({"style": "double"})),
                    ],
                );
            });
            let (b, ba) = split_button(ui, Icon::Borders, None, "Borders");
            control_keytip(app, ui, &ba, "home.borders");
            if b {
                act(app, "home.borders", json!({"preset": app.grid.last_border.clone()}));
            }
            ba.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Border presets"));
            egui::Popup::menu(&ba).show(|ui| {
                // Same groups and order as the desktop Borders menu.
                ui.spacing_mut().item_spacing.y = 0.0;
                let lang = app.ui.language;
                let heading = |ui: &mut Ui, text: &str| {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(lang.tr(text)).font(theme::ui_bold(13.0)));
                    ui.add_space(4.0);
                };
                heading(ui, "Borders");
                let groups: [&[(&str, &str)]; 3] = [
                    &[("&Bottom Border", "bottom"), ("To&p Border", "top"), ("&Left Border", "left"), ("&Right Border", "right")],
                    &[("&No Border", "none"), ("&All Borders", "all"), ("Out&side Borders", "outside"), ("&Thick Outside Borders", "thickOutside")],
                    &[
                        ("&Bottom Double Border", "doubleBottom"),
                        ("T&hick Bottom Border", "thickBottom"),
                        ("Top an&d Bottom Border", "topBottom"),
                        ("Top and Thi&ck Bottom Border", "topThickBottom"),
                        ("Top and Do&uble Bottom Border", "topDoubleBottom"),
                    ],
                ];
                for (gi, group) in groups.iter().enumerate() {
                    if gi > 0 {
                        ui.separator();
                    }
                    for (label, preset) in group.iter() {
                        if crate::border_preview::preset_button(ui, label, preset).clicked() {
                            app.grid.last_border = preset.to_string();
                            act(app, "home.borders", json!({"preset": preset}));
                        }
                    }
                }
                ui.separator();
                heading(ui, "Draw Borders");
                // Drawing borders with the pointer is not implemented yet: shown, but disabled.
                ui.add_enabled_ui(false, |ui| {
                    for label in ["Dra&w Border", "Draw Border &Grid", "&Erase Border", "&Line Color", "Line St&yle"] {
                        crate::border_preview::preset_button(ui, label, "draw").on_disabled_hover_text(lang.tr("Not available yet"));
                    }
                });
                ui.separator();
                if crate::border_preview::preset_button(ui, "&More Borders…", "all").clicked() {
                    app.open_dialog("formatCells", json!({"tab": "Border"}));
                }
            });
            let fill = app.grid.last_fill.clone();
            let fill_c = gridcraft_engine::model::Color::from_hex(&fill).and_then(|c| c.resolve(&Default::default())).map(theme::color32);
            let (f, fa) = split_button(ui, Icon::Fill, fill_c, "Fill Color");
            control_keytip(app, ui, &fa, "home.fillColor");
            if f {
                act(app, "home.fillColor", json!({"color": fill}));
            }
            let forced = invoke(app, "home.fillColor");
            egui::Popup::menu(&fa).open_memory(popup_command(forced, fa.clicked())).show(|ui| {
                if let Some(c) = color_palette(ui, &theme_colors(app), "No Fill", "fillColor") {
                    if c != "none" {
                        app.grid.last_fill = c.clone();
                    }
                    act(app, "home.fillColor", json!({"color": c}));
                    ui.close();
                }
            });
            if let Some(c) = crate::color_picker::take_result(ui.ctx(), "fillColor") {
                app.grid.last_fill = c.clone();
                act(app, "home.fillColor", json!({"color": c}));
            }
            let fc = app.grid.last_font_color.clone();
            let fc_c = gridcraft_engine::model::Color::from_hex(&fc).and_then(|c| c.resolve(&Default::default())).map(theme::color32);
            let (c, ca) = split_button(ui, Icon::FontColor, fc_c, "Font Color");
            control_keytip(app, ui, &ca, "home.fontColor");
            if c {
                act(app, "home.fontColor", json!({"color": fc}));
            }
            let forced = invoke(app, "home.fontColor");
            egui::Popup::menu(&ca).open_memory(popup_command(forced, ca.clicked())).show(|ui| {
                if let Some(c) = color_palette(ui, &theme_colors(app), "Automatic", "fontColor") {
                    if c != "none" {
                        app.grid.last_font_color = c.clone();
                    }
                    act(app, "home.fontColor", json!({"color": if c == "none" { "auto".to_string() } else { c }}));
                    ui.close();
                }
            });
            if let Some(c) = crate::color_picker::take_result(ui.ctx(), "fontColor") {
                app.grid.last_font_color = c.clone();
                act(app, "home.fontColor", json!({"color": c}));
            }
        });
    });
    if crate::ribbon_extra::end_group(ui, "Font", true) {
        app.open_dialog("formatCells", json!({"tab": "Font"}));
    }
    // Alignment
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            for (icon, v, id, tip) in [
                (Icon::AlignTop, VAlign::Top, "home.alignTop", "Top Align"),
                (Icon::AlignMiddle, VAlign::Center, "home.alignMiddle", "Middle Align"),
                (Icon::AlignBottom, VAlign::Bottom, "home.alignBottom", "Bottom Align"),
            ] {
                let __k = toggle_button(ui, icon, st.align.v == v, tip);
                control_keytip(app, ui, &__k, id);
                if __k.clicked() {
                    act(app, id, json!({}));
                }
            }
            let o = small_button(ui, Icon::Orientation, "", "Orientation", true);
            control_keytip(app, ui, &o, "home.orientation");
            let forced_o = invoke(app, "home.orientation");
            egui::Popup::menu(&o).open_memory(popup_command(forced_o, o.clicked())).show(|ui| {
                menu_items(
                    app,
                    ui,
                    &[
                        ("Angle Counterclockwise", "home.orientation", json!({"angle": "counterclockwise"})),
                        ("Angle Clockwise", "home.orientation", json!({"angle": "clockwise"})),
                        ("Vertical Text", "home.orientation", json!({"angle": "vertical"})),
                        ("Rotate Text Up", "home.orientation", json!({"angle": "up"})),
                        ("Rotate Text Down", "home.orientation", json!({"angle": "down"})),
                        ("Horizontal", "home.orientation", json!({"angle": 0})),
                        ("-", "", json!(null)),
                        ("Format Cell Alignment…", "dialog:formatCells", json!({"tab": "Alignment"})),
                    ],
                );
            });
            // Text Direction (Context / Left-to-Right / Right-to-Left) needs a reading order in the
            // style model; shown, but disabled until it has one.
            ui.add_enabled_ui(false, |ui| {
                let (r, resp) = ui.allocate_exact_size(vec2(36.0, 24.0), Sense::hover());
                let ink = Tokens::get(ui.ctx()).text_disabled;
                crate::ribbon_extra::paint(
                    ui.painter(),
                    Rect::from_center_size(r.center() - vec2(5.0, 0.0), vec2(16.0, 16.0)),
                    crate::ribbon_extra::RibIcon::TextDirection,
                    ink,
                );
                icons::paint(ui.painter(), Rect::from_center_size(pos2(r.right() - 7.0, r.center().y + 1.0), vec2(10.0, 10.0)), Icon::Chevron, ink);
                resp.on_disabled_hover_text(app.ui.language.tr("Text Direction (not available yet)"));
            });
            ui.add_space(6.0);
            let wrap = crate::ribbon_extra::small_toggle(
                ui,
                |p: &egui::Painter, r: Rect, c: Color32| icons::paint(p, r, Icon::Wrap, c),
                "Wrap Text",
                "Wrap Text",
                st.align.wrap,
                false,
            );
            control_keytip(app, ui, &wrap, "home.wrapText");
            if wrap.clicked() {
                act(app, "home.wrapText", json!({}));
            }
        });
        ui.horizontal(|ui| {
            for (icon, h, id, tip) in [
                (Icon::AlignLeft, HAlign::Left, "home.alignLeft", "Align Left"),
                (Icon::AlignCenter, HAlign::Center, "home.alignCenter", "Center"),
                (Icon::AlignRight, HAlign::Right, "home.alignRight", "Align Right"),
            ] {
                let response = toggle_button(ui, icon, st.align.h == h, tip);
                for sequence in keytip_table::control_badges(app.keytips.prefix().unwrap_or_default(), id) {
                    keytip(app, ui, sequence, response.rect.center_bottom());
                }
                if response.clicked() {
                    act(app, id, json!({}));
                }
            }
            let __k = icon_button(ui, Icon::IndentDec, t.text, "Decrease Indent", vec2(24.0, 23.0));
            control_keytip(app, ui, &__k, "home.decreaseIndent");
            if __k.clicked() {
                act(app, "home.decreaseIndent", json!({}));
            }
            let __k = icon_button(ui, Icon::IndentInc, t.text, "Increase Indent", vec2(24.0, 23.0));
            control_keytip(app, ui, &__k, "home.increaseIndent");
            if __k.clicked() {
                act(app, "home.increaseIndent", json!({}));
            }
            let (m, ma) = split_button(ui, Icon::Merge, None, "Merge & Center");
            control_keytip(app, ui, &ma, "home.mergeCenter");
            if m {
                act(app, "home.mergeCenter", json!({}));
            }
            ui.label(egui::RichText::new(app.ui.language.tr("Merge & Center")).font(theme::ui_font(12.5)));
            egui::Popup::menu(&ma).show(|ui| {
                // Desktop order, each with its own icon and access key.
                ui.spacing_mut().item_spacing.y = 0.0;
                let t = Tokens::get(ui.ctx());
                for (label, cmd, kind) in [
                    ("Merge && &Center", "home.mergeCenter", 0u8),
                    ("Merge &Across", "home.mergeAcross", 1),
                    ("&Merge Cells", "home.mergeCells", 2),
                    ("&Unmerge Cells", "home.unmergeCells", 3),
                    ("Center Across &Selection", "home.formatCells", 4),
                ] {
                    let resp = ui.add(egui::Button::new("").frame(false).min_size(vec2(260.0, 30.0)));
                    let plain = label.replace("&&", "\u{1}").replace('&', "").replace('\u{1}', "&");
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &plain));
                    let icon = Rect::from_center_size(pos2(resp.rect.left() + 18.0, resp.rect.center().y), vec2(18.0, 16.0));
                    merge_icon(ui.painter(), icon, kind, t.text, t.accent);
                    let galley = ui.painter().layout_job(crate::border_preview::access_key_job(label, theme::ui_font(13.0), t.text));
                    ui.painter().galley(pos2(resp.rect.left() + 40.0, resp.rect.center().y - galley.size().y / 2.0), galley, t.text);
                    if resp.clicked() {
                        if kind == 4 {
                            act(app, cmd, json!({"style": {"align": {"h": "CenterAcross"}}}));
                        } else {
                            act(app, cmd, json!({}));
                        }
                        ui.close();
                    }
                }
            });
        });
    });
    if crate::ribbon_extra::end_group(ui, "Alignment", true) {
        app.open_dialog("formatCells", json!({"tab": "Alignment"}));
    }
    // Number
    ui.vertical(|ui| {
        number_combo(app, ui, &st);
        ui.horizontal(|ui| {
            let (c, ca) = split_button(ui, Icon::Currency, None, "Accounting Number Format");
            control_keytip(app, ui, &ca, "home.accounting");
            if c {
                act(app, "home.accounting", json!({}));
            }
            egui::Popup::menu(&ca).show(|ui| {
                menu_items(
                    app,
                    ui,
                    &[
                        (
                            "$ English (United States)",
                            "home.numberFormat",
                            json!({"code": "_(\"$\"* #,##0.00_);_(\"$\"* \\(#,##0.00\\);_(\"$\"* \"-\"??_);_(@_)"}),
                        ),
                        (
                            "£ English (United Kingdom)",
                            "home.numberFormat",
                            json!({"code": "_-[$£-809]* #,##0.00_-;-[$£-809]* #,##0.00_-;_-[$£-809]* \"-\"??_-;_-@_-"}),
                        ),
                        (
                            "€ Euro",
                            "home.numberFormat",
                            json!({"code": "_-[$€-x-euro2] * #,##0.00_-;-[$€-x-euro2] * #,##0.00_-;_-[$€-x-euro2] * \"-\"??_-;_-@_-"}),
                        ),
                        ("¥ Japanese", "home.numberFormat", json!({"code": "_-[$¥-411]* #,##0_-;-[$¥-411]* #,##0_-;_-[$¥-411]* \"-\"_-;_-@_-"})),
                        ("-", "", json!(null)),
                        ("More Accounting Formats…", "dialog:formatCells", json!({"tab": "Number"})),
                    ],
                );
            });
            let __k = icon_button(ui, Icon::Percent, t.text, "Percent Style", vec2(24.0, 23.0));
            control_keytip(app, ui, &__k, "home.percent");
            if __k.clicked() {
                act(app, "home.percent", json!({}));
            }
            let __k = icon_button(ui, Icon::Comma, t.text, "Comma Style", vec2(24.0, 23.0));
            control_keytip(app, ui, &__k, "home.comma");
            if __k.clicked() {
                act(app, "home.comma", json!({}));
            }
            let __k = icon_button(ui, Icon::DecInc, t.text, "Increase Decimal", vec2(26.0, 23.0));
            control_keytip(app, ui, &__k, "home.increaseDecimal");
            if __k.clicked() {
                act(app, "home.increaseDecimal", json!({}));
            }
            let __k = icon_button(ui, Icon::DecDec, t.text, "Decrease Decimal", vec2(26.0, 23.0));
            control_keytip(app, ui, &__k, "home.decreaseDecimal");
            if __k.clicked() {
                act(app, "home.decreaseDecimal", json!({}));
            }
        });
    });
    if crate::ribbon_extra::end_group(ui, "Number", true) {
        app.open_dialog("formatCells", json!({"tab": "Number"}));
    }
    // Styles
    let cf = big_button(ui, Icon::CondFormat, "Conditional\nFormatting", "Conditional Formatting", true);
    anchor_keytip(app, ui, &cf, &["home.conditionalFormat", "home.manageRules", "home.clearRules"]);
    egui::Popup::menu(&cf).show(|ui| cf_menu(app, ui));
    let ft = big_button(ui, Icon::FormatTable, "Format\nas Table", "Format as Table", true);
    anchor_keytip(app, ui, &ft, &["table.style"]);
    egui::Popup::menu(&ft).show(|ui| table_gallery(app, ui, "home.formatAsTable"));
    let cs = big_button(ui, Icon::CellStyles, "Cell\nStyles", "Cell Styles", true);
    anchor_keytip(app, ui, &cs, &["home.newCellStyle"]);
    egui::Popup::menu(&cs).show(|ui| cell_style_gallery(app, ui));
    crate::ribbon_extra::end_group(ui, "Styles", false);
    // Cells
    {
        let ins = big_button(ui, Icon::Insert, "Insert", "Insert", true);
        for sequence in keytip_table::group_extension(
            app.keytips.prefix().unwrap_or_default(),
            &["dialog:insertCells", "home.insertRows", "home.insertColumns", "home.insertSheet"],
        ) {
            keytip(app, ui, sequence, ins.rect.center_bottom());
        }
        egui::Popup::menu(&ins).show(|ui| {
            menu_items(
                app,
                ui,
                &[
                    ("Insert Cells…", "dialog:insertCells", json!({})),
                    ("Insert Sheet Rows", "home.insertRows", json!({})),
                    ("Insert Sheet Columns", "home.insertColumns", json!({})),
                    ("Insert Sheet", "home.insertSheet", json!({})),
                ],
            );
        });
        let del = big_button(ui, Icon::Delete, "Delete", "Delete", true);
        for sequence in keytip_table::group_extension(
            app.keytips.prefix().unwrap_or_default(),
            &["dialog:deleteCells", "home.deleteRows", "home.deleteColumns", "home.deleteSheet"],
        ) {
            keytip(app, ui, sequence, del.rect.center_bottom());
        }
        egui::Popup::menu(&del).show(|ui| {
            menu_items(
                app,
                ui,
                &[
                    ("Delete Cells…", "dialog:deleteCells", json!({})),
                    ("Delete Sheet Rows", "home.deleteRows", json!({})),
                    ("Delete Sheet Columns", "home.deleteColumns", json!({})),
                    ("Delete Sheet", "home.deleteSheet", json!({})),
                ],
            );
        });
        let fmt = big_button(ui, Icon::Format, "Format", "Format", true);
        for sequence in keytip_table::group_extension(
            app.keytips.prefix().unwrap_or_default(),
            &[
                "dialog:rowHeight",
                "home.autofitRowHeight",
                "dialog:columnWidth",
                "home.autofitColumnWidth",
                "dialog:defaultWidth",
                "home.hideRows",
                "home.hideColumns",
                "home.unhideRows",
                "home.unhideColumns",
                "sheet.hide",
                "sheet.unhide",
                "dialog:renameSheet",
                "dialog:moveSheet",
                "dialog:protectSheet",
                "home.lockCell",
                "dialog:formatCells",
            ],
        ) {
            keytip(app, ui, sequence, fmt.rect.center_bottom());
        }
        let format_keys = matches!(app.keytips.prefix(), Some("HO" | "OC"));
        let mut format_popup = egui::Popup::menu(&fmt);
        if format_keys {
            format_popup = format_popup.open_memory(egui::SetOpenCommand::Bool(true));
        }
        let protect_label = if app.session.active().and_then(|d| d.wb.active()).is_some_and(|s| s.is_protected()) {
            "Unprotect Sheet…"
        } else {
            "Protect Sheet…"
        };
        format_popup.show(|ui| {
            menu_items(
                app,
                ui,
                &[
                    ("Row Height…", "dialog:rowHeight", json!({})),
                    ("AutoFit Row Height", "home.autofitRowHeight", json!({})),
                    ("Column Width…", "dialog:columnWidth", json!({})),
                    ("AutoFit Column Width", "home.autofitColumnWidth", json!({})),
                    ("Default Width…", "dialog:defaultWidth", json!({})),
                    ("-", "", json!(null)),
                    ("Hide Rows", "home.hideRows", json!({})),
                    ("Hide Columns", "home.hideColumns", json!({})),
                    ("Unhide Rows", "home.unhideRows", json!({})),
                    ("Unhide Columns", "home.unhideColumns", json!({})),
                    ("Hide Sheet", "sheet.hide", json!({})),
                    ("Unhide Sheet…", "sheet.unhide", json!({})),
                    ("-", "", json!(null)),
                    ("Rename Sheet", "dialog:renameSheet", json!({})),
                    ("Move or Copy Sheet…", "dialog:moveSheet", json!({})),
                    ("-", "", json!(null)),
                    (protect_label, "dialog:protectSheet", json!({})),
                    ("Lock Cell", "home.lockCell", json!({})),
                    ("Format Cells…", "dialog:formatCells", json!({})),
                ],
            );
        });
    }
    crate::ribbon_extra::end_group(ui, "Cells", false);
    // Editing
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let s = crate::ribbon_extra::small_toggle(
                ui,
                |p: &egui::Painter, r: Rect, c: Color32| icons::paint(p, r, Icon::Sum, c),
                "AutoSum",
                "AutoSum (⌘⇧T)",
                false,
                false,
            )
            .clicked();
            let sa = icon_button(ui, Icon::Chevron, t.text_dim, "AutoSum functions", vec2(14.0, 24.0));
            control_keytip(app, ui, &sa, "formulas.autoSum");
            if s {
                act(app, "formulas.autoSum", json!({}));
            }
            egui::Popup::menu(&sa).show(|ui| {
                for f in ["SUM", "AVERAGE", "COUNT", "MAX", "MIN"] {
                    if ui.button(f).clicked() {
                        act(app, "formulas.autoSum", json!({"function": f}));
                    }
                }
                ui.separator();
                if ui.button("More Functions…").clicked() {
                    app.open_dialog("insertFunction", json!({}));
                }
            });
        });
        let fill = small_button(ui, Icon::FillDown, "Fill", "Fill", true);
        for sequence in keytip_table::group_extension(
            app.keytips.prefix().unwrap_or_default(),
            &["edit.fillDown", "edit.fillRight", "edit.fillUp", "edit.fillLeft", "dialog:series", "edit.flashFill"],
        ) {
            keytip(app, ui, sequence, fill.rect.center_bottom());
        }
        egui::Popup::menu(&fill).show(|ui| {
            menu_items(
                app,
                ui,
                &[
                    ("Down", "edit.fillDown", json!({})),
                    ("Right", "edit.fillRight", json!({})),
                    ("Up", "edit.fillUp", json!({})),
                    ("Left", "edit.fillLeft", json!({})),
                    ("Series…", "dialog:series", json!({})),
                    ("Flash Fill", "edit.flashFill", json!({})),
                ],
            );
        });
        let clear = small_button(ui, Icon::Eraser, "Clear", "Clear", true);
        for sequence in keytip_table::group_extension(
            app.keytips.prefix().unwrap_or_default(),
            &["edit.clearAll", "edit.clearFormats", "edit.clearContents", "edit.clearComments", "edit.clearHyperlinks"],
        ) {
            keytip(app, ui, sequence, clear.rect.center_bottom());
        }
        egui::Popup::menu(&clear).show(|ui| {
            menu_items(
                app,
                ui,
                &[
                    ("Clear All", "edit.clearAll", json!({})),
                    ("Clear Formats", "edit.clearFormats", json!({})),
                    ("Clear Contents", "edit.clearContents", json!({})),
                    ("Clear Comments and Notes", "edit.clearComments", json!({})),
                    ("Clear Hyperlinks", "edit.clearHyperlinks", json!({})),
                ],
            );
        });
    });
    let sf = big_button(ui, Icon::SortFilter, "Sort &\nFilter", "Sort & Filter", true);
    for sequence in keytip_table::group_extension(
        app.keytips.prefix().unwrap_or_default(),
        &["data.sortAscending", "data.sortDescending", "dialog:sort", "data.filter", "data.clearFilter", "data.reapply"],
    ) {
        keytip(app, ui, sequence, sf.rect.center_bottom());
    }
    egui::Popup::menu(&sf).show(|ui| {
        menu_items(
            app,
            ui,
            &[
                ("Sort A to Z", "data.sortAscending", json!({})),
                ("Sort Z to A", "data.sortDescending", json!({})),
                ("Custom Sort…", "dialog:sort", json!({})),
                ("-", "", json!(null)),
                ("Filter", "data.filter", json!({})),
                ("Clear", "data.clearFilter", json!({})),
                ("Reapply", "data.reapply", json!({})),
            ],
        );
    });
    let fs = big_button(ui, Icon::Find, "Find &\nSelect", "Find & Select", true);
    for sequence in keytip_table::group_extension(
        app.keytips.prefix().unwrap_or_default(),
        &["dialog:find", "dialog:goTo", "dialog:goToSpecial", "edit.goToSpecial"],
    ) {
        keytip(app, ui, sequence, fs.rect.center_bottom());
    }
    egui::Popup::menu(&fs).show(|ui| {
        menu_items(
            app,
            ui,
            &[
                ("Find…", "dialog:find", json!({})),
                ("Replace…", "dialog:find", json!({"replace": true})),
                ("Go To…", "dialog:goTo", json!({})),
                ("Go To Special…", "dialog:goToSpecial", json!({})),
                ("-", "", json!(null)),
                ("Formulas", "edit.goToSpecial", json!({"kind": "formulas"})),
                ("Notes", "edit.goToSpecial", json!({"kind": "notes"})),
                ("Conditional Formatting", "edit.goToSpecial", json!({"kind": "conditionalFormats"})),
                ("Constants", "edit.goToSpecial", json!({"kind": "constants"})),
                ("Data Validation", "edit.goToSpecial", json!({"kind": "dataValidation"})),
            ],
        );
    });
    crate::ribbon_extra::end_group(ui, "Editing", false);
}

fn font_combo(app: &mut SheetApp, ui: &mut Ui, st: &Style) {
    let name = st.font.name.clone();
    let resp = ui.add_sized([150.0, 24.0], egui::Button::new(egui::RichText::new(&name).font(theme::ui_font(12.5))));
    control_keytip(app, ui, &resp, "home.fontName");
    let forced = invoke(app, "home.fontName");
    // Installed fonts after the standard set — a font the machine has (a Japanese face, say) can be
    // picked, and any other name can be typed in.
    let fams = crate::fonts::families();
    let items: Vec<(&str, String)> = fams.iter().map(|f| (f.as_str(), f.clone())).collect();
    match editable_combo(ui, &resp, "font", &name, &items, forced) {
        ComboResult::Picked(value) | ComboResult::Entered(value) => act(app, "home.fontName", json!({"name": value})),
        ComboResult::None => {}
    }
}

pub const FONTS: &[&str] = &[
    "Aptos",
    "Aptos Narrow",
    "Arial",
    "Calibri",
    "Cambria",
    "Candara",
    "Consolas",
    "Courier New",
    "Garamond",
    "Georgia",
    "Helvetica",
    "Helvetica Neue",
    "Segoe UI",
    "Tahoma",
    "Times New Roman",
    "Trebuchet MS",
    "Verdana",
];

fn size_combo(app: &mut SheetApp, ui: &mut Ui, st: &Style) {
    let size = st.font.size;
    let label = if size.fract() == 0.0 { format!("{}", size as i32) } else { format!("{size}") };
    let resp = ui.add_sized([52.0, 24.0], egui::Button::new(egui::RichText::new(&label).font(theme::ui_font(12.5))));
    control_keytip(app, ui, &resp, "home.fontSize");
    let forced = invoke(app, "home.fontSize");
    let sizes = [8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0, 36.0, 48.0, 72.0];
    let labels: Vec<String> = sizes.iter().map(|s| format!("{}", *s as i32)).collect();
    let items: Vec<(&str, String)> = labels.iter().map(|s| (s.as_str(), s.clone())).collect();
    let current = if size.fract() == 0.0 { format!("{}", size as i32) } else { format!("{size}") };
    let mut apply = |s: &str| {
        if let Ok(v) = s.parse::<f32>() {
            act(app, "home.fontSize", json!({"size": v}));
        }
    };
    match editable_combo(ui, &resp, "size", &current, &items, forced) {
        ComboResult::Picked(value) => apply(&value),
        ComboResult::Entered(text) => apply(&text),
        _ => {}
    }
}

fn number_combo(app: &mut SheetApp, ui: &mut Ui, st: &Style) {
    let code = st.num_fmt.as_str().to_string();
    let kind = gridcraft_engine::display::number_format(&code).kind();
    let current = format!("{kind:?}");
    let lang = app.ui.language;
    let sample = app.session.active().and_then(|d| d.wb.active().map(|sh| sh.value(d.selection.active))).unwrap_or_default();
    let resp = ui.add_sized([150.0, 24.0], egui::Button::new(egui::RichText::new(lang.tr(&current)).font(theme::ui_font(12.5))));
    control_keytip(app, ui, &resp, "home.numberFormat");
    let forced = invoke(app, "home.numberFormat");
    let names = ["General", "Number", "Currency", "Accounting", "Short Date", "Long Date", "Time", "Percentage", "Fraction", "Scientific", "Text"];
    let items: Vec<(&str, String)> = names
        .iter()
        .map(|n| {
            let code = gridcraft_engine::cmd::format::format_code_for(n);
            let preview = app.session.active().map(|d| gridcraft_engine::display::format(&sample, code, &d.wb).text).unwrap_or_default();
            (*n, format!("{:<12}   {preview}", lang.tr(n)))
        })
        .chain(std::iter::once(("__more", format!("{:>12}", lang.tr("More Number Formats…")))))
        .collect();
    if let ComboResult::Picked(value) = editable_combo(ui, &resp, "number", &current, &items, forced) {
        match value.as_str() {
            "__more" => app.open_dialog("formatCells", json!({"tab": "Number"})),
            n => act(app, "home.numberFormat", json!({"format": n})),
        }
    }
}

/// A conditional-formatting menu row: an icon column, then the label with its access key
/// underlined (`&`), bold for the five rule families as in the desktop menu.
fn cf_button(ui: &Ui, label: &str, big: bool) -> egui::Button<'static> {
    let t = Tokens::get(ui.ctx());
    let lang = crate::i18n::current(ui.ctx());
    let plain: String = label.replace('&', "");
    let shown = lang.tr(&plain).to_string();
    let key = if shown == plain { label.find('&').map(|b| label[..b].chars().count()) } else { None };
    let font = if big { theme::ui_bold(13.5) } else { theme::ui_font(13.0) };
    let mut job = egui::text::LayoutJob::default();
    for (i, c) in shown.chars().enumerate() {
        let underline = if Some(i) == key { Stroke::new(1.0, t.text) } else { Stroke::NONE };
        let lead = if i == 0 { if big { 46.0 } else { 30.0 } } else { 0.0 };
        job.append(&c.to_string(), lead, egui::TextFormat { font_id: font.clone(), color: t.text, underline, ..Default::default() });
    }
    egui::Button::new(job).frame(false).min_size(vec2(270.0, if big { 40.0 } else { 26.0 }))
}

fn cf_icon_at(ui: &Ui, row: Rect, icon: crate::cf_icons::CfIcon, big: bool) {
    let s = if big { 30.0 } else { 18.0 };
    let r = Rect::from_center_size(pos2(row.left() + 8.0 + s / 2.0, row.center().y), vec2(s, s));
    crate::cf_icons::paint(ui.painter(), r, icon);
}

/// A plain row; returns true when clicked.
fn cf_item(ui: &mut Ui, label: &str, icon: Option<crate::cf_icons::CfIcon>, big: bool) -> bool {
    let resp = ui.add(cf_button(ui, label, big));
    if let Some(icon) = icon {
        cf_icon_at(ui, resp.rect, icon, big);
    }
    resp.clicked()
}

/// A row that opens a submenu.
fn cf_submenu(ui: &mut Ui, label: &str, icon: Option<crate::cf_icons::CfIcon>, big: bool, content: impl FnOnce(&mut Ui)) {
    let (resp, _) = egui::containers::menu::SubMenuButton::from_button(cf_button(ui, label, big)).ui(ui, |ui| {
        ui.set_min_width(300.0);
        content(ui);
    });
    if let Some(icon) = icon {
        cf_icon_at(ui, resp.rect, icon, big);
    }
    let a = Rect::from_center_size(pos2(resp.rect.right() - 12.0, resp.rect.center().y), vec2(12.0, 12.0));
    icons::paint(ui.painter(), a, Icon::Right, Tokens::get(ui.ctx()).text);
}

fn cf_more_rules(app: &mut SheetApp, ui: &mut Ui) {
    ui.separator();
    if cf_item(ui, "&More Rules…", None, false) {
        app.open_dialog("cfQuick", json!({"type": "expression", "title": "New Formatting Rule"}));
    }
}

/// Conditional Formatting, laid out like the desktop menu: five rule families with large icons
/// and submenus, then New Rule, Clear Rules and Manage Rules. Icons are this project's own
/// (`cf_icons`).
/// Original icons for the Merge menu: a 3-column cell block showing the merge result.
/// `kind`: 0 Merge & Center, 1 Merge Across, 2 Merge Cells, 3 Unmerge, 4 Center Across Selection.
fn merge_icon(p: &egui::Painter, r: Rect, kind: u8, ink: Color32, accent: Color32) {
    let stroke = Stroke::new(1.2, ink);
    p.rect_stroke(r, 0.0, stroke, StrokeKind::Inside);
    let third = r.width() / 3.0;
    let mid_y = r.center().y;
    match kind {
        0 | 4 => {
            // One wide cell with arrows pointing out from the centre.
            let y = mid_y;
            p.arrow(pos2(r.center().x, y), vec2(-(r.width() / 2.0 - 3.0), 0.0), Stroke::new(1.2, accent));
            p.arrow(pos2(r.center().x, y), vec2(r.width() / 2.0 - 3.0, 0.0), Stroke::new(1.2, accent));
            if kind == 4 {
                p.line_segment([pos2(r.left() + third, r.top()), pos2(r.left() + third, r.top() + 3.0)], stroke);
                p.line_segment([pos2(r.right() - third, r.bottom() - 3.0), pos2(r.right() - third, r.bottom())], stroke);
            }
        }
        1 => {
            p.line_segment([pos2(r.left(), mid_y), pos2(r.right(), mid_y)], stroke);
            p.arrow(pos2(r.left() + 3.0, r.top() + r.height() * 0.28), vec2(r.width() - 6.0, 0.0), Stroke::new(1.2, accent));
            p.arrow(pos2(r.left() + 3.0, r.bottom() - r.height() * 0.28), vec2(r.width() - 6.0, 0.0), Stroke::new(1.2, accent));
        }
        2 => {
            p.line_segment([pos2(r.left(), mid_y), pos2(r.right(), mid_y)], Stroke::new(1.2, accent));
        }
        _ => {
            for k in 1..3 {
                let x = r.left() + third * k as f32;
                p.line_segment([pos2(x, r.top()), pos2(x, r.bottom())], Stroke::new(1.2, accent));
            }
            p.line_segment([pos2(r.left(), mid_y), pos2(r.right(), mid_y)], Stroke::new(1.2, accent));
        }
    }
}

/// `Some(true)` when every selected area is whole rows, `Some(false)` when every one is whole
/// columns, `None` otherwise (including the whole sheet).
fn whole_lines(app: &SheetApp) -> Option<bool> {
    let sel = &app.session.active()?.selection;
    let rows = sel.ranges.iter().all(|r| r.is_full_rows() && !r.is_full_cols());
    let cols = sel.ranges.iter().all(|r| r.is_full_cols() && !r.is_full_rows());
    if rows {
        Some(true)
    } else if cols {
        Some(false)
    } else {
        None
    }
}

fn cf_menu(app: &mut SheetApp, ui: &mut Ui) {
    use crate::cf_icons::CfIcon;
    ui.spacing_mut().item_spacing.y = 1.0;
    ui.set_min_width(280.0);
    let hex = |s: &str| Color32::from_hex(s).unwrap_or(Color32::GRAY);
    cf_submenu(ui, "&Highlight Cells Rules", Some(CfIcon::HighlightRules), true, |ui| {
        for (label, op, icon) in [
            ("&Greater Than…", "greater", CfIcon::GreaterThan),
            ("&Less Than…", "less", CfIcon::LessThan),
            ("&Between…", "between", CfIcon::Between),
            ("&Equal To…", "equal", CfIcon::EqualTo),
        ] {
            if cf_item(ui, label, Some(icon), true) {
                app.open_dialog("cfQuick", json!({"type": "cellIs", "operator": op, "title": label.replace('&', "")}));
            }
        }
        if cf_item(ui, "&Text that Contains…", Some(CfIcon::TextContains), true) {
            app.open_dialog("cfQuick", json!({"type": "containsText", "title": "Text that Contains"}));
        }
        if cf_item(ui, "&A Date Occurring…", Some(CfIcon::DateOccurring), true) {
            app.open_dialog("cfQuick", json!({"type": "timePeriod", "title": "A Date Occurring"}));
        }
        if cf_item(ui, "&Duplicate Values…", Some(CfIcon::Duplicate), true) {
            act(app, "home.conditionalFormat", json!({"rule": {"type": "duplicate"}}));
        }
        cf_more_rules(app, ui);
    });
    cf_submenu(ui, "&Top/Bottom Rules", Some(CfIcon::TopBottom), true, |ui| {
        for (label, rule, icon) in [
            ("&Top 10 Items…", json!({"type": "top10", "rank": 10}), CfIcon::Top10),
            ("To&p 10%…", json!({"type": "top10", "rank": 10, "percent": true}), CfIcon::Top10Percent),
            ("&Bottom 10 Items…", json!({"type": "top10", "rank": 10, "bottom": true}), CfIcon::Bottom10),
            ("B&ottom 10%…", json!({"type": "top10", "rank": 10, "bottom": true, "percent": true}), CfIcon::Bottom10Percent),
            ("&Above Average…", json!({"type": "aboveAverage"}), CfIcon::AboveAverage),
            ("Below A&verage…", json!({"type": "aboveAverage", "below": true}), CfIcon::BelowAverage),
        ] {
            if cf_item(ui, label, Some(icon), true) {
                act(app, "home.conditionalFormat", json!({"rule": rule}));
            }
        }
        cf_more_rules(app, ui);
    });
    cf_submenu(ui, "&Data Bars", Some(CfIcon::DataBars), true, |ui| {
        for (label, c) in
            [("Blue", "#638EC6"), ("Green", "#63BE7B"), ("Red", "#F8696B"), ("Orange", "#FFB628"), ("Light Blue", "#008AEF"), ("Purple", "#D6007B")]
        {
            if cf_item(ui, label, Some(CfIcon::Bar(hex(c))), true) {
                act(app, "home.conditionalFormat", json!({"rule": {"type": "dataBar", "color": c}}));
            }
        }
        cf_more_rules(app, ui);
    });
    cf_submenu(ui, "Color &Scales", Some(CfIcon::ColorScales), true, |ui| {
        for (label, cols, two) in [
            ("Green - Yellow - Red", ["#63BE7B", "#FFEB84", "#F8696B"], false),
            ("Red - Yellow - Green", ["#F8696B", "#FFEB84", "#63BE7B"], false),
            ("Green - White - Red", ["#63BE7B", "#FCFCFF", "#F8696B"], false),
            ("Blue - White - Red", ["#5A8AC6", "#FCFCFF", "#F8696B"], false),
            ("White - Green", ["#FCFCFF", "#B1DEC1", "#63BE7B"], true),
            ("White - Red", ["#FCFCFF", "#FBB2B4", "#F8696B"], true),
        ] {
            if cf_item(ui, label, Some(CfIcon::Scale([hex(cols[0]), hex(cols[1]), hex(cols[2])])), true) {
                let colors: Vec<&str> = if two { vec![cols[0], cols[2]] } else { cols.to_vec() };
                act(app, "home.conditionalFormat", json!({"rule": {"type": "colorScale", "colors": colors}}));
            }
        }
        cf_more_rules(app, ui);
    });
    cf_submenu(ui, "&Icon Sets", Some(CfIcon::IconSets), true, |ui| {
        let (g, y, r, b) = (hex("#46A05A"), hex("#F2C12E"), hex("#E64646"), hex("#5B9BD5"));
        for (label, set, dots) in [
            ("3 Arrows", "3Arrows", [g, y, r]),
            ("3 Traffic Lights", "3TrafficLights1", [g, y, r]),
            ("3 Symbols", "3Symbols", [g, y, r]),
            ("4 Ratings", "4Rating", [b, b, b]),
            ("5 Arrows", "5Arrows", [g, y, r]),
            ("5 Quarters", "5Quarters", [b, b, b]),
        ] {
            if cf_item(ui, label, Some(CfIcon::Dots(dots)), true) {
                act(app, "home.conditionalFormat", json!({"rule": {"type": "iconSet", "set": set}}));
            }
        }
        cf_more_rules(app, ui);
    });
    ui.separator();
    if cf_item(ui, "&New Rule…", Some(CfIcon::NewRule), false) {
        app.open_dialog("cfQuick", json!({"type": "expression", "title": "New Formatting Rule"}));
    }
    cf_submenu(ui, "&Clear Rules", Some(CfIcon::ClearRules), false, |ui| {
        if cf_item(ui, "Clear Rules from &Selected Cells", None, false) {
            act(app, "home.clearRules", json!({}));
        }
        if cf_item(ui, "Clear Rules from &Entire Sheet", None, false) {
            act(app, "home.clearRules", json!({"sheet": true}));
        }
    });
    if cf_item(ui, "Manage &Rules…", Some(CfIcon::ManageRules), false) {
        app.open_dialog("manageRules", json!({}));
    }
}

/// A gallery of table styles drawn as mini tables.
fn table_gallery(app: &mut SheetApp, ui: &mut Ui, cmd: &str) {
    ui.set_width(430.0);
    let Some(wb) = app.session.active().map(|d| d.wb.clone()) else { return };
    for (fam, n) in [("Light", 21u32), ("Medium", 28), ("Dark", 11)] {
        ui.label(egui::RichText::new(fam).strong());
        egui::Grid::new(("tg", fam)).spacing(vec2(4.0, 4.0)).show(ui, |ui| {
            for i in 1..=n {
                let name = format!("TableStyle{fam}{i}");
                let (r, resp) = ui.allocate_exact_size(vec2(54.0, 38.0), Sense::click());
                paint_table_swatch(ui, r, &wb, &name);
                if resp.hovered() {
                    ui.painter().rect_stroke(r.expand(1.5), 2.0, Stroke::new(2.0, Color32::from_rgb(0xE8, 0x7D, 0x2B)), StrokeKind::Outside);
                }
                if resp.on_hover_text(&name).clicked() {
                    act(app, cmd, json!({"style": name}));
                    ui.close();
                }
                if i % 7 == 0 {
                    ui.end_row();
                }
            }
        });
    }
}

fn paint_table_swatch(ui: &Ui, r: Rect, wb: &gridcraft_engine::model::Workbook, style: &str) {
    let lk = gridcraft_engine::tables::look(style);
    let res = |c: Option<gridcraft_engine::model::Color>| c.and_then(|c| c.resolve(&wb.theme)).map(theme::color32);
    let p = ui.painter();
    p.rect_filled(r, 0.0, Color32::WHITE);
    let rows = 5;
    let rh = r.height() / rows as f32;
    for i in 0..rows {
        let rr = Rect::from_min_size(pos2(r.left(), r.top() + i as f32 * rh), vec2(r.width(), rh));
        let fill = if i == 0 {
            res(lk.header_fill)
        } else if i % 2 == 1 {
            res(lk.band_fill)
        } else {
            res(lk.body_fill)
        };
        if let Some(f) = fill {
            p.rect_filled(rr, 0.0, f);
        }
        if let Some(l) = res(lk.line) {
            p.line_segment([rr.left_bottom(), rr.right_bottom()], Stroke::new(1.0, l));
        }
        for k in 0..3 {
            let x = rr.left() + 4.0 + k as f32 * (r.width() - 8.0) / 3.0;
            let col = if i == 0 { res(Some(lk.header_font)).unwrap_or(Color32::BLACK) } else { Color32::from_gray(120) };
            p.line_segment([pos2(x, rr.center().y), pos2(x + 10.0, rr.center().y)], Stroke::new(1.0, col));
        }
    }
    p.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_gray(210)), StrokeKind::Inside);
}

fn cell_style_gallery(app: &mut SheetApp, ui: &mut Ui) {
    ui.set_width(520.0);
    let Some(wb) = app.session.active().map(|d| d.wb.clone()) else { return };
    let groups: &[(&str, &[&str])] = &[
        ("Good, Bad and Neutral", &["Normal", "Bad", "Good", "Neutral"]),
        ("Data and Model", &["Calculation", "Check Cell", "Explanatory Text", "Input", "Linked Cell", "Note", "Output", "Warning Text"]),
        ("Titles and Headings", &["Heading 1", "Heading 2", "Heading 3", "Heading 4", "Title", "Total"]),
        (
            "Themed Cell Styles",
            &[
                "20% - Accent1",
                "20% - Accent2",
                "20% - Accent3",
                "20% - Accent4",
                "20% - Accent5",
                "20% - Accent6",
                "40% - Accent1",
                "40% - Accent2",
                "40% - Accent3",
                "40% - Accent4",
                "40% - Accent5",
                "40% - Accent6",
                "60% - Accent1",
                "60% - Accent2",
                "60% - Accent3",
                "60% - Accent4",
                "60% - Accent5",
                "60% - Accent6",
                "Accent1",
                "Accent2",
                "Accent3",
                "Accent4",
                "Accent5",
                "Accent6",
            ],
        ),
        ("Number Format", &["Comma", "Comma [0]", "Currency", "Currency [0]", "Percent"]),
    ];
    for (g, names) in groups {
        ui.label(egui::RichText::new(*g).strong());
        egui::Grid::new(("csg", *g)).spacing(vec2(4.0, 4.0)).show(ui, |ui| {
            for (i, n) in names.iter().enumerate() {
                let st = gridcraft_engine::cmd::format::builtin_cell_style(n, &wb.theme).unwrap_or_default();
                let (r, resp) = ui.allocate_exact_size(vec2(82.0, 24.0), Sense::click());
                let fill = st
                    .fill
                    .fg
                    .resolve(&wb.theme)
                    .filter(|_| st.fill.pattern != gridcraft_engine::model::PatternType::None)
                    .map(theme::color32)
                    .unwrap_or(Color32::WHITE);
                ui.painter().rect_filled(r, 0.0, fill);
                ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_gray(210)), StrokeKind::Inside);
                if !st.borders.bottom.is_none() {
                    let c = st.borders.bottom.color.resolve(&wb.theme).map(theme::color32).unwrap_or(Color32::BLACK);
                    ui.painter().line_segment(
                        [r.left_bottom() + vec2(0.0, -1.5), r.right_bottom() + vec2(0.0, -1.5)],
                        Stroke::new(st.borders.bottom.style.width(), c),
                    );
                }
                let col = st.font.color.resolve(&wb.theme).map(theme::color32).unwrap_or(Color32::BLACK);
                let fam = theme::cell_family(&st.font.name, st.font.bold, st.font.italic);
                ui.painter().text(
                    r.left_center() + vec2(4.0, 0.0),
                    Align2::LEFT_CENTER,
                    *n,
                    egui::FontId::new((st.font.size).clamp(10.0, 15.0), fam),
                    col,
                );
                if resp.hovered() {
                    ui.painter().rect_stroke(r.expand(1.5), 2.0, Stroke::new(2.0, Color32::from_rgb(0xE8, 0x7D, 0x2B)), StrokeKind::Outside);
                }
                if resp.clicked() {
                    act(app, "home.cellStyle", json!({"name": n}));
                    ui.close();
                }
                if (i + 1) % 6 == 0 {
                    ui.end_row();
                }
            }
        });
    }
}

fn page_layout(app: &mut SheetApp, ui: &mut Ui) {
    use crate::ribbon_extra::{RibIcon, big, paint, small};
    let ic = |icon: Icon| move |p: &egui::Painter, r: Rect, c: Color32| icons::paint(p, r, icon, c);
    let rb = |icon: RibIcon| move |p: &egui::Painter, r: Rect, c: Color32| paint(p, r, icon, c);
    // Themes
    let th = big(ui, ic(Icon::Theme), "Themes", "Themes", true, true, false);
    anchor_keytip(app, ui, &th, &["pageLayout.theme"]);
    egui::Popup::menu(&th).show(|ui| {
        for (name, colors) in gridcraft_engine::cmd::view::themes() {
            ui.horizontal(|ui| {
                for c in colors.iter().skip(4).take(6) {
                    let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                    ui.painter().rect_filled(r, 1.0, Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, *c as u8));
                }
                if ui.button(name).clicked() {
                    act(app, "pageLayout.theme", json!({"name": name}));
                }
            });
        }
    });
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let colors = small(ui, rb(RibIcon::ThemeColors), "Colors", "Theme Colors", true, true);
        egui::Popup::menu(&colors).show(|ui| {
            for (name, cs) in gridcraft_engine::cmd::view::themes() {
                ui.horizontal(|ui| {
                    for c in cs.iter().skip(4).take(6) {
                        let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                        ui.painter().rect_filled(r, 1.0, Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, *c as u8));
                    }
                    if ui.button(name).clicked() {
                        act(app, "pageLayout.themeColors", json!({"name": name}));
                    }
                });
            }
        });
        let fonts = small(ui, rb(RibIcon::ThemeFonts), "Fonts", "Theme Fonts", true, true);
        egui::Popup::menu(&fonts).show(|ui| {
            for (major, minor) in [
                ("Calibri Light", "Calibri"),
                ("Aptos Display", "Aptos"),
                ("Arial", "Arial"),
                ("Cambria", "Calibri"),
                ("Times New Roman", "Times New Roman"),
                ("Segoe UI", "Segoe UI"),
            ] {
                if ui.button(format!("{major} / {minor}")).clicked() {
                    act(app, "pageLayout.themeFonts", json!({"major": major, "minor": minor}));
                }
            }
        });
        small(ui, rb(RibIcon::ThemeEffects), "Effects", "Theme Effects (not available yet)", true, false);
    });
    crate::ribbon_extra::end_group(ui, "Themes", false);
    // Page Setup
    let m = big(ui, ic(Icon::Margins), "Margins", "Margins", true, true, false);
    for sequence in keytip_table::group_extension(app.keytips.prefix().unwrap_or_default(), &["pageLayout.margins", "dialog:pageSetup"]) {
        keytip(app, ui, sequence, m.rect.center_bottom());
    }
    egui::Popup::menu(&m).show(|ui| {
        menu_items(
            app,
            ui,
            &[
                ("Normal", "pageLayout.margins", json!({"preset": "normal"})),
                ("Wide", "pageLayout.margins", json!({"preset": "wide"})),
                ("Narrow", "pageLayout.margins", json!({"preset": "narrow"})),
                ("Custom Margins…", "dialog:pageSetup", json!({})),
            ],
        );
    });
    let o = big(ui, ic(Icon::Orient), "Orientation", "Orientation", true, true, false);
    control_keytip(app, ui, &o, "pageLayout.orientation");
    egui::Popup::menu(&o).show(|ui| {
        menu_items(
            app,
            ui,
            &[
                ("Portrait", "pageLayout.orientation", json!({"orientation": "portrait"})),
                ("Landscape", "pageLayout.orientation", json!({"orientation": "landscape"})),
            ],
        );
    });
    let sz = big(ui, ic(Icon::Paper), "Size", "Paper Size", true, true, false);
    control_keytip(app, ui, &sz, "pageLayout.size");
    egui::Popup::menu(&sz).show(|ui| {
        for p in ["Letter", "Legal", "Tabloid", "Executive", "A3", "A4", "A5"] {
            if ui.button(p).clicked() {
                act(app, "pageLayout.size", json!({"paper": p}));
            }
        }
    });
    let pa = big(ui, ic(Icon::PrintArea), "Print\nArea", "Print Area", true, true, false);
    anchor_keytip(app, ui, &pa, &["pageLayout.printArea"]);
    egui::Popup::menu(&pa).show(|ui| {
        menu_items(
            app,
            ui,
            &[("Set Print Area", "pageLayout.printArea", json!({})), ("Clear Print Area", "pageLayout.printArea", json!({"clear": true}))],
        );
    });
    let br = big(ui, ic(Icon::PageBreak), "Breaks", "Page Breaks", true, true, false);
    for sequence in keytip_table::group_extension(app.keytips.prefix().unwrap_or_default(), &["pageLayout.breaks"]) {
        keytip(app, ui, sequence, br.rect.center_bottom());
    }
    egui::Popup::menu(&br).show(|ui| {
        menu_items(
            app,
            ui,
            &[
                ("Insert Page Break", "pageLayout.breaks", json!({"insert": true})),
                ("Remove Page Break", "pageLayout.breaks", json!({"remove": true})),
                ("Reset All Page Breaks", "pageLayout.breaks", json!({"reset": true})),
            ],
        );
    });
    let bg = big(ui, rb(RibIcon::Background), "Background", "Sheet Background", false, true, false);
    if bg.clicked()
        && let Some(pick) = &app.services.pick_picture
        && let Some(path) = pick()
    {
        app.run_or_alert("pageLayout.background", json!({"path": path}));
    }
    let __k = big(ui, ic(Icon::Sheet), "Print\nTitles", "Print Titles", false, true, false);
    control_keytip(app, ui, &__k, "pageLayout.printTitles");
    if __k.clicked() {
        app.open_dialog("pageSetup", json!({"tab": "Sheet"}));
    }
    if crate::ribbon_extra::end_group(ui, "Page Setup", true) {
        app.open_dialog("pageSetup", json!({}));
    }
    // Scale to Fit
    let (fit, scale) = app.session.active().and_then(|d| d.wb.active().map(|s| (s.print.fit_to, s.print.scale))).unwrap_or((None, 100));
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let pages = |n: u16| {
            if n == 0 {
                app.ui.language.tr("Automatic").to_string()
            } else if n == 1 {
                "1 page".to_string()
            } else {
                format!("{n} pages")
            }
        };
        let mut changed: Option<serde_json::Value> = None;
        for (label, icon, idx) in [("Width:", RibIcon::Width, 0usize), ("Height:", RibIcon::Height, 1)] {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(18.0, 22.0), Sense::hover());
                paint(ui.painter(), Rect::from_center_size(r.center(), vec2(16.0, 16.0)), icon, Tokens::get(ui.ctx()).text);
                ui.add_sized([52.0, 22.0], egui::Label::new(egui::RichText::new(app.ui.language.tr(label)).font(theme::ui_font(12.5))));
                let cur = fit.map_or(0, |f| if idx == 0 { f.0 } else { f.1 });
                egui::ComboBox::from_id_salt(("fit", idx)).width(100.0).selected_text(pages(cur)).show_ui(ui, |ui| {
                    for n in 0..=9u16 {
                        if ui.selectable_label(cur == n, pages(n)).clicked() {
                            let (w, h) = fit.unwrap_or((0, 0));
                            changed = Some(if idx == 0 { json!({"width": n, "height": h}) } else { json!({"width": w, "height": n}) });
                        }
                    }
                });
            });
        }
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(18.0, 22.0), Sense::hover());
            paint(ui.painter(), Rect::from_center_size(r.center(), vec2(16.0, 16.0)), RibIcon::Scale, Tokens::get(ui.ctx()).text);
            ui.add_sized([52.0, 22.0], egui::Label::new(egui::RichText::new(app.ui.language.tr("Scale:")).font(theme::ui_font(12.5))));
            let auto = fit.is_none_or(|f| f == (0, 0));
            let mut v = i32::from(scale);
            let resp = ui.add_enabled(auto, egui::DragValue::new(&mut v).range(10..=400).suffix("%").speed(1.0));
            if resp.changed() {
                changed = Some(json!({"scale": v}));
            }
        });
        if let Some(p) = changed {
            act(app, "pageLayout.scaleToFit", p);
        }
    });
    if crate::ribbon_extra::end_group(ui, "Scale to Fit", true) {
        app.open_dialog("pageSetup", json!({"tab": "Page"}));
    }
    // Sheet Options
    let rtl = app.session.active().and_then(|d| d.wb.active()).is_some_and(|s| s.right_to_left);
    let r = big(ui, ic(Icon::SheetRtl), "Sheet Right-\nto-Left", "Sheet Right-to-Left", false, true, rtl);
    if r.clicked() {
        act(app, "view.rightToLeft", json!({"on": !rtl}));
    }
    let sh = app
        .session
        .active()
        .and_then(|d| d.wb.active().map(|s| (s.show_gridlines, s.show_headings, s.print.gridlines, s.print.headings)))
        .unwrap_or((true, true, false, false));
    // These two checkboxes mean the screen view vs. the printed page. English shows "View" (as in
    // Excel), but the key is distinct from the "View" tab so languages can word them apart.
    let screen = app.ui.language.tr("Sheet Options|View");
    let print = app.ui.language.tr("Print");
    ui.vertical(|ui| {
        ui.label(egui::RichText::new(app.ui.language.tr("Gridlines")).font(theme::ui_font(12.5)));
        let mut v = sh.0;
        let resp = ui.checkbox(&mut v, screen);
        control_keytip(app, ui, &resp, "view.gridlines");
        if resp.changed() || invoke(app, "view.gridlines") {
            act(app, "view.gridlines", json!({"on": checkbox_on(resp.changed(), v)}));
        }
        let mut p = sh.2;
        let resp = ui.checkbox(&mut p, print);
        control_keytip(app, ui, &resp, "pageLayout.printGridlines");
        if resp.changed() || invoke(app, "pageLayout.printGridlines") {
            act(app, "pageLayout.printGridlines", json!({"on": checkbox_on(resp.changed(), p)}));
        }
    });
    ui.add_space(8.0);
    ui.vertical(|ui| {
        ui.label(egui::RichText::new(app.ui.language.tr("Headings")).font(theme::ui_font(12.5)));
        let mut v = sh.1;
        let resp = ui.checkbox(&mut v, screen);
        control_keytip(app, ui, &resp, "view.headings");
        if resp.changed() || invoke(app, "view.headings") {
            act(app, "view.headings", json!({"on": checkbox_on(resp.changed(), v)}));
        }
        let mut p = sh.3;
        let resp = ui.checkbox(&mut p, print);
        control_keytip(app, ui, &resp, "pageLayout.printHeadings");
        if resp.changed() || invoke(app, "pageLayout.printHeadings") {
            act(app, "pageLayout.printHeadings", json!({"on": checkbox_on(resp.changed(), p)}));
        }
    });
    if crate::ribbon_extra::end_group(ui, "Sheet Options", true) {
        app.open_dialog("pageSetup", json!({"tab": "Sheet"}));
    }
    // Arrange: acts on the selected chart.
    let selected = app.selected_chart;
    let has = selected.is_some();
    let target = |extra: serde_json::Value| {
        let mut v = json!({"kind": "chart", "id": selected.unwrap_or(0)});
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        v
    };
    let bf = big(ui, rb(RibIcon::BringForward), "Bring\nForward", "Bring Forward", true, has, false);
    egui::Popup::menu(&bf).show(|ui| {
        if ui.button(app.ui.language.tr("Bring Forward")).clicked() {
            act(app, "arrange.bringForward", target(json!({})));
        }
        if ui.button(app.ui.language.tr("Bring to Front")).clicked() {
            act(app, "arrange.bringForward", target(json!({"toFront": true})));
        }
    });
    let sb = big(ui, rb(RibIcon::SendBackward), "Send\nBackward", "Send Backward", true, has, false);
    egui::Popup::menu(&sb).show(|ui| {
        if ui.button(app.ui.language.tr("Send Backward")).clicked() {
            act(app, "arrange.sendBackward", target(json!({})));
        }
        if ui.button(app.ui.language.tr("Send to Back")).clicked() {
            act(app, "arrange.sendBackward", target(json!({"toBack": true})));
        }
    });
    if big(ui, rb(RibIcon::SelectionPane), "Selection\nPane", "Selection Pane", false, true, app.grid.pane.as_deref() == Some("selection")).clicked()
    {
        app.grid.pane = if app.grid.pane.as_deref() == Some("selection") { None } else { Some("selection".to_string()) };
    }
    let al = big(ui, rb(RibIcon::Align), "Align", "Align Objects", true, has, false);
    egui::Popup::menu(&al).show(|ui| {
        for (label, a) in [
            ("Align Left", "left"),
            ("Align Center", "center"),
            ("Align Right", "right"),
            ("Align Top", "top"),
            ("Align Middle", "middle"),
            ("Align Bottom", "bottom"),
        ] {
            if ui.button(app.ui.language.tr(label)).clicked() {
                act(app, "arrange.align", json!({"kind": "chart", "ids": [selected.unwrap_or(0)], "align": a}));
            }
        }
    });
    big(ui, rb(RibIcon::GroupObjects), "Group", "Group Objects (select several objects)", true, false, false);
    big(ui, rb(RibIcon::Rotate), "Rotate", "Rotate Objects (shapes only)", true, false, false);
    crate::ribbon_extra::end_group(ui, "Arrange", false);
}

fn table_design(app: &mut SheetApp, ui: &mut Ui) {
    let Some((tname, header, totals, banded, banded_c, first, last, filter)) = app.session.active().and_then(|d| {
        let sh = d.wb.active()?;
        let t = sh.table_at(d.selection.active)?;
        Some((t.name.clone(), t.header_row, t.totals_row, t.banded_rows, t.banded_cols, t.first_col, t.last_col, t.filter_button))
    }) else {
        return;
    };
    ui.vertical(|ui| {
        ui.label(egui::RichText::new("Table Name:").small());
        let field = egui::Id::new(("gridcraft.table_name", &tname));
        if let Some(name) = crate::widgets::committed_text(ui, field, &tname, Some(120.0))
            && name != tname
        {
            act(app, "table.rename", json!({"table": tname, "name": name}));
        }
    });
    crate::ribbon_extra::end_group(ui, "Properties", false);
    let __k = big_button(ui, Icon::Duplicates, "Remove\nDuplicates", "Remove Duplicates", false);
    control_keytip(app, ui, &__k, "data.removeDuplicates");
    if __k.clicked() {
        act(app, "data.removeDuplicates", json!({}));
    }
    let __k = big_button(ui, Icon::Table, "Convert\nto Range", "Convert to Range", false);
    control_keytip(app, ui, &__k, "table.convertToRange");
    if __k.clicked() {
        act(app, "table.convertToRange", json!({"table": tname}));
    }
    ui.add_space(4.0);
    if big_button(ui, Icon::Filter, "Insert\nSlicer", "Insert Slicer", false).clicked() {
        act(app, "table.insertSlicer", json!({"table": tname}));
    }
    crate::ribbon_extra::end_group(ui, "Tools", false);
    ui.vertical(|ui| {
        for (label, cmd, v) in [
            ("Header Row", "table.headerRow", header && filter),
            ("Total Row", "table.totalRow", totals),
            ("Banded Rows", "table.bandedRows", banded),
        ] {
            let mut on = v;
            if ui.checkbox(&mut on, label).changed() {
                act(app, cmd, json!({"table": tname, "on": on}));
            }
        }
    });
    ui.vertical(|ui| {
        for (label, cmd, v) in [
            ("First Column", "table.firstColumn", first),
            ("Last Column", "table.lastColumn", last),
            ("Banded Columns", "table.bandedColumns", banded_c),
        ] {
            let mut on = v;
            if ui.checkbox(&mut on, label).changed() {
                act(app, cmd, json!({"table": tname, "on": on}));
            }
        }
    });
    crate::ribbon_extra::end_group(ui, "Table Style Options", false);
    let st = big_button(ui, Icon::FormatTable, "Table\nStyles", "Table Styles", true);
    anchor_keytip(app, ui, &st, &["table.style"]);
    egui::Popup::menu(&st).show(|ui| table_gallery(app, ui, "table.style"));
    crate::ribbon_extra::end_group(ui, "Table Styles", false);
}

fn chart_design(app: &mut SheetApp, ui: &mut Ui) {
    let Some(id) = app.selected_chart else { return };
    let ae = big_button(ui, Icon::Chart, "Add Chart\nElement", "Add Chart Element", true);
    for sequence in keytip_table::group_extension(app.keytips.prefix().unwrap_or_default(), &["chart.set"]) {
        keytip(app, ui, sequence, ae.rect.center_bottom());
    }
    egui::Popup::menu(&ae).show(|ui| {
        menu_items(
            app,
            ui,
            &[
                ("Legend: Bottom", "chart.set", json!({"chart": id, "legend": "bottom"})),
                ("Legend: Right", "chart.set", json!({"chart": id, "legend": "right"})),
                ("Legend: Top", "chart.set", json!({"chart": id, "legend": "top"})),
                ("Legend: None", "chart.set", json!({"chart": id, "legend": "none"})),
                ("-", "", json!(null)),
                ("Data Labels: Show", "chart.set", json!({"chart": id, "dataLabels": true})),
                ("Data Labels: None", "chart.set", json!({"chart": id, "dataLabels": false})),
                ("Gridlines: Show", "chart.set", json!({"chart": id, "gridlines": true})),
                ("Gridlines: None", "chart.set", json!({"chart": id, "gridlines": false})),
            ],
        );
    });
    let __k = big_button(ui, Icon::Redo, "Switch\nRow/Column", "Switch Row/Column", false);
    control_keytip(app, ui, &__k, "chart.switchRowColumn");
    if __k.clicked() {
        act(app, "chart.switchRowColumn", json!({"chart": id}));
    }
    if big_button(ui, Icon::Table, "Select\nData", "Select Data", false).clicked() {
        app.open_dialog("selectData", json!({"chart": id}));
    }
    let ct = big_button(ui, Icon::ChartBar, "Change\nChart Type", "Change Chart Type", true);
    for sequence in keytip_table::group_extension(
        app.keytips.prefix().unwrap_or_default(),
        &["column", "bar", "line", "area", "pie", "doughnut", "scatter", "radar", "waterfall", "funnel", "treemap"],
    ) {
        keytip(app, ui, sequence, ct.rect.center_bottom());
    }
    egui::Popup::menu(&ct).show(|ui| {
        for (label, k, s) in [
            ("Clustered Column", "column", ""),
            ("Stacked Column", "column", "stacked"),
            ("Bar", "bar", ""),
            ("Line", "line", "markers"),
            ("Area", "area", ""),
            ("Pie", "pie", ""),
            ("Doughnut", "doughnut", ""),
            ("Scatter", "scatter", ""),
            ("Radar", "radar", ""),
            ("Waterfall", "waterfall", ""),
            ("Funnel", "funnel", ""),
            ("Treemap", "treemap", ""),
        ] {
            if ui.button(label).clicked() {
                act(app, "chart.set", json!({"chart": id, "type": k, "subtype": s}));
            }
        }
    });
    if big_button(ui, Icon::Settings, "Format\nPane", "Format Chart pane", false).clicked() {
        app.grid.pane = Some("formatChart".into());
    }
    let __k = big_button(ui, Icon::TextBox, "Chart\nTitle", "Edit chart title", false);
    control_keytip(app, ui, &__k, "chartTitle");
    if __k.clicked() {
        app.open_dialog("chartTitle", json!({"chart": id}));
    }
    let __k = big_button(ui, Icon::Delete, "Delete\nChart", "Delete chart", false);
    control_keytip(app, ui, &__k, "chart.delete");
    if __k.clicked() {
        act(app, "chart.delete", json!({"chart": id}));
        app.selected_chart = None;
    }
    crate::ribbon_extra::end_group(ui, "Chart", false);
}

/// Command-key shortcuts when the grid has focus.
pub fn shortcut(app: &mut SheetApp, key: Key, m: Modifiers) {
    let shift = m.shift;
    let id: Option<(&str, serde_json::Value)> = match key {
        Key::B => Some(("home.bold", json!({}))),
        Key::I => Some(("home.italic", json!({}))),
        Key::U => Some(("home.underline", json!({}))),
        Key::Z if shift => Some(("edit.redo", json!({}))),
        Key::Z => Some(("edit.undo", json!({}))),
        Key::Y => Some(("edit.redo", json!({}))),
        Key::A => Some(("edit.selectAll", json!({}))),
        Key::D => Some(("edit.fillDown", json!({}))),
        Key::R => Some(("edit.fillRight", json!({}))),
        Key::S if shift => Some(("file.saveAs", json!({}))),
        Key::S => Some(("file.save", json!({}))),
        Key::O => Some(("file.open", json!({}))),
        Key::N => Some(("file.new", json!({}))),
        Key::W => Some(("file.close", json!({}))),
        Key::P => {
            app.print_now(json!({}));
            None
        }
        Key::K => {
            app.open_dialog("insertLink", json!({}));
            None
        }
        Key::F if shift => Some(("data.filter", json!({}))),
        Key::F => {
            app.open_dialog("find", json!({}));
            None
        }
        Key::H => {
            app.open_dialog("find", json!({"replace": true}));
            None
        }
        Key::G => {
            app.open_dialog("goTo", json!({}));
            None
        }
        Key::T if shift => Some(("formulas.autoSum", json!({}))),
        Key::T => Some(("insert.table", json!({}))),
        Key::E => Some(("edit.flashFill", json!({}))),
        Key::Num1 => {
            app.open_dialog("formatCells", json!({}));
            None
        }
        Key::Num9 if shift => Some(("home.unhideRows", json!({}))),
        Key::Num9 => Some(("home.hideRows", json!({}))),
        Key::Num0 if shift => Some(("home.unhideColumns", json!({}))),
        Key::Num0 => Some(("home.hideColumns", json!({}))),
        // As in Excel: with whole rows or whole columns selected, Ctrl+Shift+= / Ctrl+- insert or
        // delete them at once; any other selection asks which way to shift.
        Key::Minus => match whole_lines(app) {
            Some(true) => Some(("home.deleteRows", json!({}))),
            Some(false) => Some(("home.deleteColumns", json!({}))),
            None => {
                app.open_dialog("deleteCells", json!({}));
                None
            }
        },
        Key::Plus | Key::Equals if shift => match whole_lines(app) {
            Some(true) => Some(("home.insertRows", json!({}))),
            Some(false) => Some(("home.insertColumns", json!({}))),
            None => {
                app.open_dialog("insertCells", json!({}));
                None
            }
        },
        Key::Semicolon => {
            // Insert today's date.
            let text = app
                .session
                .active()
                .map(|d| {
                    let today = gridcraft_engine::core::date::serial_in_system(d.wb.date_system, gridcraft_engine::calc::now_serial()).floor();
                    gridcraft_engine::display::format(&gridcraft_engine::core::Value::Number(today), "m/d/yyyy", &d.wb).text
                })
                .unwrap_or_default();
            app.begin_edit(Some(text), false);
            None
        }
        Key::Backtick => Some(("formulas.showFormulas", json!({}))),
        _ => None,
    };
    if let Some((id, p)) = id {
        app.run_or_alert(id, p);
        app.grid.ensure_visible = true;
    }
}

#[allow(dead_code)]
fn unused(_: &Tokens, _: Icon) {
    let _ = icons::GREEN;
}

#[cfg(test)]
mod title_bar_tests {
    use super::*;

    #[test]
    fn mac_title_bar_centres_and_clears_the_traffic_lights() {
        // The lights are centred in the bar, and content keeps the same gap after them.
        let margin = (MAC_TITLE_BAR - MAC_LIGHT) / 2.0;
        let lights_end = MAC_LIGHTS_LEFT + 2.0 * MAC_LIGHT_PITCH + MAC_LIGHT;
        assert_eq!(MAC_CONTENT_LEFT - lights_end, margin);
    }
}

#[cfg(test)]
mod checkbox_on_tests {
    use super::*;

    #[test]
    fn click_sends_the_new_checkbox_value_and_key_tip_the_opposite() {
        // Click: the checkbox bool already holds the new choice.
        assert!(!checkbox_on(true, false));
        assert!(checkbox_on(true, true));
        // Key tip: the bool still holds the current setting, so toggle it.
        assert!(checkbox_on(false, false));
        assert!(!checkbox_on(false, true));
    }
}
