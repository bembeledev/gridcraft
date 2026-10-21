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

/// Renders the horizontal sheet tab strip, providing controls to navigate, 
/// add, move, delete, rename, and recolor sheets.
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
