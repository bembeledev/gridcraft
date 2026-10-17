//! Linked workbooks in the interface: the Workbook Links pane (Data › Queries & Connections ›
//! Workbook Links, Excel 365), the classic Edit Links dialog (File › Info › Edit Links to Files)
//! and the dialogs they open: Change Source, Startup Prompt and the Break Links confirmation.
//! Thin: everything reads `DocState::links` and acts through the `data.*` link commands.

use egui::{Color32, RichText, Ui, vec2};
use gridcraft_engine::cmd::workbook_links::{self as wl, LinkRow, LinkStatus};
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::dialogs::Dialog;
use crate::i18n::Language;
use crate::theme::{self, Tokens};

/// The dialogs this module draws.
pub const DIALOGS: [&str; 4] = ["editLinks", "changeLinkSource", "linkStartupPrompt", "breakLinks"];

/// The startup prompt's choices: (mode, label).
const PROMPTS: [(&str, &str); 3] = [
    ("userSet", "Let users choose to display the alert or not"),
    ("never", "Don't display the alert and don't update automatic links"),
    ("always", "Don't display the alert and update links"),
];

/// A label of the pane and dialogs, translated. Keys carry the `Workbook Links|` context, so
/// these generic words (Source, Status, Refresh…) get their own translations here.
fn tr(lang: Language, label: &str) -> String {
    lang.tr(&format!("Workbook Links|{label}")).to_string()
}

fn rows(app: &SheetApp) -> Vec<LinkRow> {
    app.session.active().map(wl::rows).unwrap_or_default()
}

/// Commands that read the source files are off without a file system (the browser).
fn files(app: &SheetApp) -> bool {
    app.session.link_files.available()
}

fn status_color(t: &Tokens, s: LinkStatus) -> Color32 {
    match s {
        LinkStatus::Ok => Color32::from_rgb(0x10, 0x7C, 0x41),
        LinkStatus::ValuesNotUpdated => Color32::from_rgb(0xB0, 0x6A, 0x00),
        s if s.is_error() => t.danger,
        _ => t.text_dim,
    }
}

/// "just now", "5 min ago", "3 h ago", or the date (UTC) for older refreshes.
fn ago(ms: f64) -> String {
    let secs = ((crate::now_ms() - ms) / 1000.0).max(0.0);
    if secs < 60.0 {
        "just now".into()
    } else if secs < 3600.0 {
        format!("{} min ago", (secs / 60.0) as u64)
    } else if secs < 86_400.0 {
        format!("{} h ago", (secs / 3600.0) as u64)
    } else {
        let (y, m, d) = gridcraft_engine::core::date::civil_from_days((ms / 86_400_000.0).floor() as i64);
        format!("{y:04}-{m:02}-{d:02}")
    }
}

/// The dialog for `name` (one of [`DIALOGS`]), or `None` when it can't open.
pub fn open(app: &mut SheetApp, name: &str, params: &Json) -> Option<Dialog> {
    let link = params.get("link").and_then(Json::as_u64).unwrap_or(0);
    match name {
        "editLinks" => Some(Dialog::custom("editLinks", "Edit Links", json!({}))),
        "changeLinkSource" => {
            let row = rows(app).into_iter().find(|r| r.index as u64 == link)?;
            let title = format!("{}: {}", tr(app.ui.language, "Change Source"), row.name);
            let path = params.get("path").and_then(Json::as_str).map(str::to_string).unwrap_or(row.location);
            Some(Dialog::custom("changeLinkSource", &title, json!({"link": link, "path": path})))
        }
        "linkStartupPrompt" => {
            let mode = app.session.active().map(|d| d.wb.update_links.as_str()).unwrap_or("userSet");
            Some(Dialog::custom("linkStartupPrompt", "Startup Prompt", json!({"mode": params.get("mode").and_then(Json::as_str).unwrap_or(mode)})))
        }
        "breakLinks" => Some(Dialog::custom("breakLinks", "Break Links", json!({"link": params.get("link").cloned().unwrap_or(Json::Null)}))),
        _ => None,
    }
}

/// Draws one of [`DIALOGS`]. Actions run their command and close the dialog; errors show in a
/// message box and leave it open.
pub fn dialog(app: &mut SheetApp, ui: &mut Ui, d: &mut Dialog, open: &mut bool) {
    match d.name() {
        "editLinks" => edit_links(app, ui, d, open),
        "changeLinkSource" => change_source(app, ui, d, open),
        "linkStartupPrompt" => startup_prompt(app, ui, d, open),
        "breakLinks" => break_links(app, ui, d, open),
        _ => *open = false,
    }
}

/// Runs a command for a dialog button: `true` when it worked, else the error shows.
fn run(app: &mut SheetApp, id: &str, params: Json) -> bool {
    match app.run(id, params) {
        Ok(_) => true,
        Err(e) => {
            app.message = Some(("GridCraft".into(), crate::clean_error(&e)));
            false
        }
    }
}

fn buttons_right(ui: &mut Ui, f: impl FnOnce(&mut Ui)) {
    ui.add_space(8.0);
    ui.horizontal(|ui| ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), f));
}

/// File › Info › Edit Links to Files: the classic dialog, a list of links (Source, Type, Update,
/// Status) with Update Values, Change Source…, Open Source, Break Link and Check Status beside it
/// and Startup Prompt… below.
fn edit_links(app: &mut SheetApp, ui: &mut Ui, d: &mut Dialog, open: &mut bool) {
    let lang = app.ui.language;
    let t = Tokens::get(ui.ctx());
    let list = rows(app);
    let selected = list.get(d.list_index).or(list.first()).cloned();
    if let Some(r) = &selected {
        d.list_index = r.index - 1;
    }
    let link = selected.as_ref().map(|r| json!(r.index)).unwrap_or(Json::Null);
    let files = files(app);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(380.0);
            egui::Frame::NONE.stroke(egui::Stroke::new(1.0, t.input_border)).inner_margin(4.0).show(ui, |ui| {
                ui.set_min_height(150.0);
                egui::Grid::new("edit_links_list").num_columns(4).striped(true).spacing(vec2(12.0, 4.0)).show(ui, |ui| {
                    for h in ["Source", "Type", "Update", "Status"] {
                        ui.label(RichText::new(tr(lang, h)).strong());
                    }
                    ui.end_row();
                    for (i, r) in list.iter().enumerate() {
                        if ui.selectable_label(d.list_index == i, &r.name).clicked() {
                            d.list_index = i;
                        }
                        ui.label(tr(lang, r.kind));
                        ui.label(r.update);
                        ui.label(RichText::new(tr(lang, r.status_text)).color(status_color(&t, r.status)));
                        ui.end_row();
                    }
                });
            });
            ui.add_space(6.0);
            let folder = selected
                .as_ref()
                .map(|r| std::path::Path::new(&r.location).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default())
                .unwrap_or_default();
            egui::Grid::new("edit_links_detail").num_columns(2).spacing(vec2(8.0, 4.0)).show(ui, |ui| {
                ui.label(tr(lang, "Location:"));
                ui.label(folder);
                ui.end_row();
                ui.label(tr(lang, "Item:"));
                ui.label("");
                ui.end_row();
                ui.label(tr(lang, "Update:"));
                ui.add_enabled_ui(false, |ui| {
                    ui.horizontal(|ui| {
                        let _ = ui.radio(true, tr(lang, "Automatic"));
                        let _ = ui.radio(false, tr(lang, "Manual"));
                    });
                });
                ui.end_row();
            });
        });
        ui.vertical(|ui| {
            let any = selected.is_some();
            let button = |ui: &mut Ui, label: &str, on: bool| {
                let r = ui.add_enabled(on, egui::Button::new(tr(lang, label)).min_size(vec2(150.0, 24.0)));
                if !files && !r.enabled() {
                    r.clone().on_disabled_hover_text(tr(lang, "Updating links needs the desktop app: it reads the linked files."));
                }
                r.clicked()
            };
            if button(ui, "Update Values", any && files) && run(app, "data.refreshLinks", json!({"link": link})) {
                d.result = None;
            }
            if button(ui, "Change Source…", any && files) {
                app.open_dialog("changeLinkSource", json!({"link": link}));
            }
            if button(ui, "Open Source", any && files) && run(app, "data.openLinkSource", json!({"link": link})) {
                *open = false;
            }
            if button(ui, "Break Link", any) {
                app.open_dialog("breakLinks", json!({"link": link}));
            }
            if button(ui, "Check Status", any && files) {
                run(app, "data.checkLinkStatus", json!({}));
            }
        });
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button(tr(lang, "Startup Prompt…")).clicked() {
            app.open_dialog("linkStartupPrompt", json!({}));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(format!("  {}  ", tr(lang, "Close"))).clicked() {
                *open = false;
            }
        });
    });
}

/// Change Source: the file the link reads from now on.
fn change_source(app: &mut SheetApp, ui: &mut Ui, d: &mut Dialog, open: &mut bool) {
    let lang = app.ui.language;
    let link = d.values.get("link").cloned().unwrap_or(Json::Null);
    let mut path = d.values.get("path").and_then(Json::as_str).unwrap_or_default().to_string();
    ui.label(tr(lang, "File name:"));
    ui.horizontal(|ui| {
        let width = ui.available_width() - if app.services.pick_open.is_some() { 80.0 } else { 0.0 };
        if ui.add(egui::TextEdit::singleline(&mut path).desired_width(width.max(120.0))).changed() {
            d.values.insert("path".into(), json!(path.clone()));
        }
        if let Some(pick) = &app.services.pick_open
            && ui.button(tr(lang, "Browse…")).clicked()
            && let Some(p) = pick()
        {
            path = p;
            d.values.insert("path".into(), json!(path.clone()));
        }
    });
    let mut ok = false;
    buttons_right(ui, |ui| {
        if ui.button(format!(" {} ", tr(lang, "Cancel"))).clicked() {
            *open = false;
        }
        if ui.add(egui::Button::new(format!("   {}   ", tr(lang, "OK")))).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            ok = true;
        }
    });
    if ok && !path.trim().is_empty() && run(app, "data.changeLinkSource", json!({"link": link, "path": path})) {
        *open = false;
    }
}

/// Startup Prompt: what happens to the workbook's links when it opens (saved with it).
fn startup_prompt(app: &mut SheetApp, ui: &mut Ui, d: &mut Dialog, open: &mut bool) {
    let lang = app.ui.language;
    let mut mode = d.values.get("mode").and_then(Json::as_str).unwrap_or("userSet").to_string();
    ui.label(tr(lang, "When this workbook opens, ask whether to update its links to other workbooks:"));
    ui.add_space(4.0);
    for (value, label) in PROMPTS {
        if ui.radio(mode == value, tr(lang, label)).clicked() {
            mode = value.to_string();
            d.values.insert("mode".into(), json!(value));
        }
    }
    let mut ok = false;
    buttons_right(ui, |ui| {
        if ui.button(format!(" {} ", tr(lang, "Cancel"))).clicked() {
            *open = false;
        }
        if ui.button(format!("   {}   ", tr(lang, "OK"))).clicked() {
            ok = true;
        }
    });
    if ok && run(app, "data.linkStartupPrompt", json!({"mode": mode})) {
        *open = false;
    }
}

/// The confirmation Break Link asks for: the formulas become values.
fn break_links(app: &mut SheetApp, ui: &mut Ui, d: &mut Dialog, open: &mut bool) {
    let lang = app.ui.language;
    let link = d.values.get("link").cloned().unwrap_or(Json::Null);
    ui.label(tr(
        lang,
        "Breaking links turns every formula that uses them into the value it shows now. Defined names that use them keep their links. Undo brings the formulas back.",
    ));
    ui.add_space(4.0);
    ui.label(tr(lang, "Break the links?"));
    let mut ok = false;
    buttons_right(ui, |ui| {
        if ui.button(format!(" {} ", tr(lang, "Cancel"))).clicked() {
            *open = false;
        }
        if ui.button(tr(lang, "Break Links")).clicked() {
            ok = true;
        }
    });
    let params = if link.is_null() { json!({}) } else { json!({"link": link}) };
    if ok && run(app, "data.breakLink", params) {
        *open = false;
    }
}

/// The Workbook Links task pane: Refresh All and Break All, a search box, and a card per linked
/// workbook with its location, status, last refresh and actions.
pub fn pane(app: &mut SheetApp, ui: &mut Ui) {
    let lang = app.ui.language;
    let t = Tokens::get(ui.ctx());
    let files = files(app);
    let list = rows(app);
    if list.is_empty() {
        ui.label(RichText::new(tr(lang, "This workbook has no links to other workbooks.")).italics());
        return;
    }
    let search_id = ui.id().with("workbook_links_search");
    let mut search: String = ui.ctx().data(|m| m.get_temp(search_id)).unwrap_or_default();
    ui.horizontal(|ui| {
        if ui.add_enabled(files, egui::Button::new(tr(lang, "Refresh All"))).clicked() {
            app.run_or_alert("data.refreshLinks", json!({}));
        }
        if ui.button(tr(lang, "Break All")).clicked() {
            app.open_dialog("breakLinks", json!({}));
        }
    });
    if !files {
        ui.label(RichText::new(tr(lang, "Updating links needs the desktop app: it reads the linked files.")).small().color(t.text_dim));
    } else if app.session.active().is_some_and(|d| d.links.updates_disabled) {
        ui.label(RichText::new(tr(lang, "Automatic update of links has been disabled")).small().color(t.text_dim));
    }
    ui.add_space(4.0);
    if ui.add(egui::TextEdit::singleline(&mut search).hint_text(tr(lang, "Search")).desired_width(f32::INFINITY)).changed() {
        ui.ctx().data_mut(|m| m.insert_temp(search_id, search.clone()));
    }
    ui.add_space(6.0);
    let needle = search.trim().to_lowercase();
    for r in list.iter().filter(|r| needle.is_empty() || r.name.to_lowercase().contains(&needle) || r.location.to_lowercase().contains(&needle)) {
        egui::Frame::NONE.fill(t.input_bg).corner_radius(6.0).inner_margin(8.0).stroke(egui::Stroke::new(1.0, t.input_border)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(&r.name).font(theme::ui_bold(13.5)));
            if !r.location.is_empty() {
                ui.label(RichText::new(&r.location).small().color(t.text_dim));
            }
            ui.label(RichText::new(tr(lang, r.status_text)).color(status_color(&t, r.status)));
            if let Some(detail) = &r.detail
                && r.status.is_error()
            {
                ui.label(RichText::new(detail).small().color(t.text_dim));
            }
            if let Some(ms) = r.last_refreshed {
                ui.label(RichText::new(format!("{} {}", tr(lang, "Last refreshed:"), ago(ms))).small().color(t.text_dim));
            }
            ui.horizontal_wrapped(|ui| {
                let link = json!({"link": r.index});
                if ui.add_enabled(files, egui::Button::new(tr(lang, "Refresh")).small()).clicked() {
                    app.run_or_alert("data.refreshLinks", link.clone());
                }
                if ui.add_enabled(files && !r.path.is_empty(), egui::Button::new(tr(lang, "Open Workbook")).small()).clicked() {
                    app.run_or_alert("data.openLinkSource", link.clone());
                }
                if ui.add_enabled(files, egui::Button::new(tr(lang, "Change Source…")).small()).clicked() {
                    app.open_dialog("changeLinkSource", link.clone());
                }
                if ui.add(egui::Button::new(tr(lang, "Break Links")).small()).clicked() {
                    app.open_dialog("breakLinks", link);
                }
            });
        });
        ui.add_space(6.0);
    }
}
