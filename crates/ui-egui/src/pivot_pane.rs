//! The PivotTable Fields task pane: shown on the right when the active cell is in a pivot
//! report. Ticking a field adds it (text → Rows, numbers → Values); each area lists its fields
//! with move/remove menus. Everything runs `pivot.*` commands.

use egui::{Ui, vec2};
use serde_json::json;

use crate::SheetApp;
use crate::i18n;
use crate::theme::{self, Tokens};

#[derive(Clone)]
struct PivotFieldDrag {
    field: String,
    from: Option<String>,
}

/// Whether the active cell is inside a pivot report (so the pane should show).
pub fn active_pivot(app: &SheetApp) -> Option<String> {
    let d = app.session.active()?;
    let sh = d.wb.active()?;
    let a = d.selection.active;
    sh.pivots.iter().find(|p| p.last_range.is_some_and(|r| r.contains(a)) || p.anchor == a).map(|p| p.name.clone())
}

pub fn show(app: &mut SheetApp, ui: &mut Ui) {
    let Some(name) = active_pivot(app) else { return };
    let t = Tokens::get(ui.ctx());
    let lang = i18n::current(ui.ctx());
    let Some(key) = app.session.active().map(|d| (d.uid, d.revision, name.clone())) else { return };
    if app.grid.pivot_fields.as_ref().is_none_or(|(k, _)| *k != key) {
        let info = match app.session.run("pivot.fields", json!({"pivot": name})) {
            Ok(v) => v,
            Err(_) => return,
        };
        app.grid.pivot_fields = Some((key, info));
    }
    let Some((_, info)) = app.grid.pivot_fields.clone() else { return };
    egui::Panel::right("pivot_pane").default_size(280.0).resizable(true).frame(egui::Frame::NONE.fill(t.ribbon).inner_margin(10)).show(ui, |ui| {
        ui.label(egui::RichText::new(lang.tr("PivotTable Fields")).font(theme::ui_bold(15.0)));
        ui.label(egui::RichText::new(lang.tr("Choose fields to add to report:")).small().color(t.text_dim));
        ui.add_space(4.0);
        let fields = info["fields"].as_array().cloned().unwrap_or_default();
        egui::ScrollArea::vertical().max_height(220.0).auto_shrink([false, true]).id_salt("pv_fields").show(ui, |ui| {
            for f in &fields {
                let fname = f["name"].as_str().unwrap_or("").to_string();
                let areas: Vec<String> =
                    f["areas"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
                let mut on = !areas.is_empty();
                let numeric = f["numeric"].as_bool().unwrap_or(false);
                let drag = PivotFieldDrag { field: fname.clone(), from: None };
                let r = ui
                    .dnd_drag_source(ui.make_persistent_id(format!("pv_field_{fname}")), drag, |ui| {
                        ui.checkbox(&mut on, egui::RichText::new(&fname).font(theme::ui_font(13.0)))
                    })
                    .inner;
                // Excel: right-click a field name to add it to a chosen area.
                let popup_id = ui.make_persistent_id(format!("pv_field_{fname}")).with("ctx");
                if ui.input(|i| i.pointer.button_released(egui::PointerButton::Secondary)) && r.hovered() {
                    egui::Popup::open_id(ui.ctx(), popup_id);
                }
                let anchor_rect = r.rect;
                egui::Popup::new(popup_id, ui.ctx().clone(), egui::PopupAnchor::ParentRect(anchor_rect), ui.layer_id()).open_memory(None).show(
                    |ui| {
                        for (to, l) in [
                            ("rows", "Add to Row Label"),
                            ("columns", "Add to Column Label"),
                            ("values", "Add to Values"),
                            ("filters", "Add to Report Filter"),
                        ] {
                            if ui.button(lang.tr(l)).clicked() {
                                app.run_or_alert("pivot.addField", json!({"pivot": name, "field": fname, "area": to}));
                            }
                        }
                    },
                );
                if r.changed() && on {
                    // Only adding via the checkbox; removal happens via drag-out or "Remove Field".
                    // (Excel unchecks to remove, but the user asked for removal only on drag-out.)
                    if let Err(e) = app.run("pivot.addField", json!({"pivot": name, "field": fname, "area": if numeric { "values" } else { "rows" }}))
                    {
                        app.message = Some(("PivotTable".into(), crate::clean_error(&e)));
                    }
                }
                let samples: Vec<String> =
                    f["samples"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
                r.on_hover_text(samples.join(", "));
            }
        });
        ui.separator();
        ui.label(egui::RichText::new(lang.tr("Use a field's menu to move it between areas:")).small().color(t.text_dim));
        let area = |key: &str| -> Vec<String> {
            info["pivot"][key]
                .as_array()
                .or(info[key].as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x["name"].as_str().or(x["field"].as_str()).or(x["sourceCol"].as_str()).or(x.as_str()).map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        // Σ Values (two or more value fields) sits in Rows or Columns; it moves only between them.
        const VALUES_FIELD: &str = "Σ Values";
        let values_key = if info["valuesOnRows"].as_bool().unwrap_or(false) { "rows" } else { "columns" };
        let values_at = info["valuesPosition"].as_u64().map(|p| p as usize);
        let area_with_values = |key: &str| -> Vec<String> {
            let mut v = area(key);
            if key == values_key && area("values").len() > 1 {
                let at = values_at.unwrap_or(v.len()).min(v.len());
                v.insert(at, VALUES_FIELD.to_string());
            }
            v
        };
        ui.columns(2, |cols| {
            for (i, (key, label)) in [("filters", "Filters"), ("columns", "Columns"), ("rows", "Rows"), ("values", "Values")].iter().enumerate() {
                let ui = &mut cols[i % 2];
                ui.label(egui::RichText::new(lang.tr(label)).strong());
                let frame = egui::Frame::NONE.fill(t.window).corner_radius(4.0).inner_margin(4.0);
                // Displayed slots: the area's fields, plus Σ Values (a label for the value fields,
                // not a field) shown at its position in Rows/Columns.
                let mut slots: Vec<(String, bool, egui::Rect)> = Vec::new();
                let (_, dropped) = ui.dnd_drop_zone::<PivotFieldDrag, _>(frame, |ui| {
                    ui.set_min_size(vec2(ui.available_width(), 60.0));
                    for f in area_with_values(key) {
                        let is_values = f == VALUES_FIELD;
                        let drag = PivotFieldDrag { field: f.clone(), from: Some(key.to_string()) };
                        let inner = ui.dnd_drag_source(ui.make_persistent_id(format!("pv_{key}_{f}")), drag, |ui| {
                            ui.add(
                                egui::Button::new(egui::RichText::new(format!("{f} ▾")).font(theme::ui_font(12.0)))
                                    .fill(t.ribbon)
                                    .min_size(vec2(ui.available_width(), 20.0)),
                            )
                        });
                        let b = inner.inner;
                        slots.push((f.clone(), is_values, b.rect));
                        let secondary = ui.input(|i| i.pointer.button_released(egui::PointerButton::Secondary) && b.hovered());
                        if secondary {
                            app.open_dialog("pivotFieldSettings", json!({"pivot": name, "field": f, "area": key}));
                        }
                    }
                });
                if let Some(payload) = dropped {
                    let p = payload.as_ref().clone();
                    // Landing slot: compare the release point with every displayed slot's centre.
                    let y = ui.input(|i| i.pointer.interact_pos().map(|pt| pt.y).unwrap_or(f32::INFINITY));
                    let dragged = slots.iter().position(|(n, _, _)| *n == p.field);
                    let (mut di, mut sigma_above, mut others_fields) = (0, 0, 0);
                    for (idx, (_, is_values, rect)) in slots.iter().enumerate() {
                        if rect.center().y < y {
                            di += 1;
                            if !*is_values && Some(idx) != dragged {
                                others_fields += 1;
                            }
                            if *is_values {
                                sigma_above += 1;
                            }
                        }
                    }
                    let same = p.from.as_deref() == Some(key);
                    if p.field == VALUES_FIELD && !matches!(*key, "rows" | "columns") {
                        // Σ Values only lives in Rows/Columns; dropping it elsewhere does nothing.
                    } else if p.field == VALUES_FIELD && *key == "rows" {
                        // Σ position counts fields only; in Columns it always sits after the last field.
                        app.run_or_alert(
                            "pivot.moveField",
                            json!({"pivot": name, "field": p.field, "from": p.from, "to": key, "position": di - sigma_above}),
                        );
                    } else if same {
                        // Same area: the engine removes the field first, so subtract it.
                        app.run_or_alert(
                            "pivot.moveField",
                            json!({"pivot": name, "field": p.field, "from": key, "to": key, "position": others_fields}),
                        );
                    } else {
                        let pos = if matches!(*key, "rows" | "columns") { di - sigma_above } else { di };
                        match &p.from {
                            Some(from) => app
                                .run_or_alert("pivot.moveField", json!({"pivot": name, "field": p.field, "from": from, "to": key, "position": pos})),
                            None => app.run_or_alert("pivot.addField", json!({"pivot": name, "field": p.field, "area": key, "position": pos})),
                        }
                    }
                }
            }
        });

        // If a field was dragged out of every area, remove it (Excel behaviour). A click, or a
        // click with a hand-tremor wobble, is not a drag: `is_decidedly_dragging` is false for
        // those, so only a real drag-out removes the field.
        let dragged_out = ui.input(|i| i.pointer.primary_released() && i.pointer.is_decidedly_dragging());
        if dragged_out
            && egui::DragAndDrop::has_payload_of_type::<PivotFieldDrag>(ui.ctx())
            && let Some(payload) = egui::DragAndDrop::take_payload::<PivotFieldDrag>(ui.ctx())
            && let Some(from) = payload.from.clone()
        {
            app.run_or_alert("pivot.removeField", json!({"pivot": name, "field": payload.field, "area": from}));
        }

        ui.separator();
        ui.horizontal(|ui| {
            if ui.button(lang.tr("Refresh")).clicked() {
                app.run_or_alert("pivot.refresh", json!({"pivot": name}));
            }
            if ui.button(lang.tr("Calculated Field…")).clicked() {
                app.run_or_alert("pivot.calculatedField", json!({"pivot": name}));
            }
            egui::ComboBox::from_id_salt("pv_layout").selected_text(lang.tr("Report Layout")).show_ui(ui, |ui| {
                for (l, v) in [("Compact", "compact"), ("Outline", "outline"), ("Tabular", "tabular")] {
                    if ui.button(lang.tr(l)).clicked() {
                        app.run_or_alert("pivot.layout", json!({"pivot": name, "layout": v}));
                    }
                }
            });
        });
    });
}
