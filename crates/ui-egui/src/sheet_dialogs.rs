//! Desktop-style Insert / Delete (cells) and Move or Copy dialogs.
//!
//! Layout follows the familiar Windows dialogs: a light title row with help and close, a captioned
//! group of options with underlined access keys, and OK / Cancel side by side.

use egui::{Color32, Key, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::dialogs::{Dialog, light_title};
use crate::theme::{self, Tokens};

/// Splits `"Entire &row"` into its plain text and lower-case access key.
fn access_key(label: &str) -> (String, Option<char>) {
    let plain = label.replace('&', "");
    let key = label.find('&').and_then(|i| label[i + 1..].chars().next()).map(|c| c.to_ascii_lowercase());
    (plain, key)
}

/// True when the access key `c` was typed this frame (with no text field focused).
fn typed(ui: &Ui, c: char) -> bool {
    if ui.ctx().text_edit_focused() {
        return false;
    }
    ui.input(|i| {
        i.events.iter().any(|e| matches!(e, egui::Event::Text(t) if t.chars().next().map(|x| x.to_ascii_lowercase()) == Some(c)))
            && !i.modifiers.ctrl
            && !i.modifiers.command
    })
}

/// A group caption followed by a thin rule to the right edge.
fn caption(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    let lang = crate::i18n::current(ui.ctx());
    ui.horizontal(|ui| {
        let r = ui.label(egui::RichText::new(lang.tr(text)).font(theme::ui_font(13.0)).color(t.text));
        let y = r.rect.center().y + 1.0;
        let x0 = r.rect.right() + 6.0;
        let x1 = ui.max_rect().right();
        ui.painter().hline(x0..=x1, y, Stroke::new(1.0, t.separator));
    });
}

/// A radio button whose label has an underlined access key. Returns true when chosen.
fn radio(ui: &mut Ui, selected: bool, label: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let lang = crate::i18n::current(ui.ctx());
    let shown = crate::border_preview::tr_access(lang, label);
    let (plain, key) = access_key(&shown);
    let job = crate::border_preview::access_key_job(&shown, theme::ui_font(13.0), t.text);
    let galley = ui.painter().layout_job(job);
    let (r, resp) = ui.allocate_exact_size(vec2(26.0 + galley.size().x + 8.0, 26.0), Sense::click());
    let c = pos2(r.left() + 10.0, r.center().y);
    if selected {
        ui.painter().circle_filled(c, 8.0, t.accent);
        ui.painter().circle_filled(c, 3.5, Color32::WHITE);
    } else {
        ui.painter().circle_stroke(c, 7.5, Stroke::new(1.0, if resp.hovered() { t.text } else { t.text_dim }));
    }
    ui.painter().galley(pos2(r.left() + 26.0, r.center().y - galley.size().y / 2.0), galley, t.text);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, &plain));
    resp.clicked() || key.is_some_and(|k| typed(ui, k))
}

/// OK (outlined as the default button) and Cancel, right-aligned.
fn ok_cancel(ui: &mut Ui, confirm: &mut bool, open: &mut bool) {
    let t = Tokens::get(ui.ctx());
    let lang = crate::i18n::current(ui.ctx());
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 32.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let button = |label: &str, primary: bool| {
            egui::Button::new(egui::RichText::new(lang.tr(label)).font(theme::ui_font(13.0)))
                .min_size(vec2(96.0, 30.0))
                .corner_radius(4)
                .stroke(if primary { Stroke::new(1.5, t.accent) } else { Stroke::new(1.0, t.input_border) })
        };
        if ui.add(button("Cancel", false)).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
            *open = false;
        }
        ui.add_space(8.0);
        if ui.add(button("OK", true)).clicked() || ui.input(|i| i.key_pressed(Key::Enter)) {
            *confirm = true;
        }
    });
}

/// Insert or Delete cells: four ways to shift, as in the desktop dialog.
pub(crate) fn cells(app: &SheetApp, ui: &mut Ui, d: &mut Dialog, confirm: &mut bool, open: &mut bool) {
    let delete = d.name == "deleteCells";
    // A right-to-left sheet shifts cells the other way, and the dialog says so.
    let rtl = app.session.active().and_then(|doc| doc.wb.active()).is_some_and(|s| s.right_to_left);
    let sideways = match (delete, rtl) {
        (true, false) => ("left", "Shift cells &left"),
        // "Entire &row" has R: the mirrored sideways shift takes I, as Insert's does.
        (true, true) => ("left", "Shift cells r&ight"),
        (false, false) => ("right", "Shift cells r&ight"),
        (false, true) => ("right", "Shift cells &left"),
    };
    let options: [(&str, &str); 4] = [
        sideways,
        if delete { ("up", "Shift cells &up") } else { ("down", "Shift cells &down") },
        ("row", "Entire &row"),
        ("column", "Entire &column"),
    ];
    light_title(ui, if delete { "Delete" } else { "Insert" }, open, "Choose how the surrounding cells move.");
    ui.add_space(6.0);
    caption(ui, if delete { "Delete" } else { "Insert" });
    ui.add_space(2.0);
    let current = d.values.get("shift").and_then(Json::as_str).unwrap_or(if delete { "up" } else { "down" }).to_string();
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (key, label) in options {
                if radio(ui, current == key, label) {
                    d.values.insert("shift".into(), json!(key));
                }
            }
        });
    });
    ui.add_space(12.0);
    ok_cancel(ui, confirm, open);
    ui.add_space(4.0);
}

/// Move or Copy: the destination book, the sheet to go before (or the end), and Create a copy.
pub(crate) fn move_copy(app: &SheetApp, ui: &mut Ui, d: &mut Dialog, confirm: &mut bool, open: &mut bool) {
    let t = Tokens::get(ui.ctx());
    let lang = crate::i18n::current(ui.ctx());
    let Some(doc) = app.session.active() else {
        *open = false;
        return;
    };
    let names: Vec<String> = doc.wb.sheets.iter().map(|s| s.name.clone()).collect();
    let source = d.values.get("sheet").and_then(Json::as_u64).map_or(doc.wb.active_sheet, |v| v as usize);
    let mut before = d.values.get("before").and_then(Json::as_u64).map_or(source, |v| v as usize).min(names.len());
    let mut copy = d.values.get("copy").and_then(Json::as_bool).unwrap_or(false);
    let label = |ui: &mut Ui, text: &str| {
        let shown = crate::border_preview::tr_access(lang, text);
        ui.label(crate::border_preview::access_key_job(&shown, theme::ui_font(13.0), t.text));
    };
    light_title(ui, "Move or Copy", open, "Moves or copies the selected sheet within this workbook.");
    ui.add_space(4.0);
    ui.label(egui::RichText::new(lang.tr("Move selected sheets")).font(theme::ui_font(13.0)));
    ui.add_space(8.0);
    label(ui, "&To book:");
    let width = ui.available_width();
    egui::ComboBox::from_id_salt("move_copy_book").width(width - 8.0).selected_text(&doc.title).show_ui(ui, |ui| {
        let _ = ui.selectable_label(true, &doc.title);
    });
    ui.add_space(8.0);
    label(ui, "&Before sheet:");
    // The list box: every sheet, then "(move to end)".
    let rows: Vec<String> = names.iter().cloned().chain(std::iter::once(lang.tr("(move to end)").to_string())).collect();
    let row_h = 24.0;
    let (list, _) = ui.allocate_exact_size(vec2(width - 8.0, 250.0), Sense::hover());
    ui.painter().rect_filled(list, 2.0, t.input_bg);
    ui.painter().rect_stroke(list, 2.0, Stroke::new(1.0, t.input_border), StrokeKind::Inside);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list.shrink(2.0)));
    egui::ScrollArea::vertical().id_salt("move_copy_list").show(&mut child, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (i, name) in rows.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(vec2(list.width() - 6.0, row_h), Sense::click());
            if i == before {
                ui.painter().rect_filled(r, 4.0, t.accent_soft);
                ui.painter().rect_stroke(r, 4.0, Stroke::new(1.5, t.text), StrokeKind::Inside);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, 4.0, t.hover);
            }
            ui.painter().text(pos2(r.left() + 6.0, r.center().y), egui::Align2::LEFT_CENTER, name, theme::ui_font(13.0), t.text);
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, i == before, name));
            if resp.clicked() {
                before = i;
            }
            if resp.double_clicked() {
                *confirm = true;
            }
        }
    });
    if ui.input(|i| i.key_pressed(Key::ArrowDown)) {
        before = (before + 1).min(names.len());
    }
    if ui.input(|i| i.key_pressed(Key::ArrowUp)) {
        before = before.saturating_sub(1);
    }
    ui.add_space(10.0);
    let resp = ui.checkbox(
        &mut copy,
        crate::border_preview::access_key_job(&crate::border_preview::tr_access(lang, "&Create a copy"), theme::ui_font(13.0), t.text),
    );
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, copy, "Create a copy"));
    if typed(ui, 'c') {
        copy = !copy;
    }
    ui.add_space(10.0);
    ok_cancel(ui, confirm, open);
    ui.add_space(4.0);
    // `sheet.move` takes the final index: a moved sheet leaves its old slot first.
    let to = if copy || before <= source { before } else { before - 1 };
    d.values.insert("before".into(), json!(before));
    d.values.insert("copy".into(), json!(copy));
    d.values.insert("to".into(), json!(to));
    d.values.insert("sheet".into(), json!(source));
}
