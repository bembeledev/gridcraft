use super::show::ok_cancel;
use super::*;

/// Every chart type the All Charts tab offers: `(type, subtype, label)` for `insert.chart`.
const ALL_CHARTS: [(&str, &str, &str); 21] = [
    ("column", "", "Clustered Column"),
    ("column", "stacked", "Stacked Column"),
    ("column", "stacked100", "100% Stacked Column"),
    ("bar", "", "Clustered Bar"),
    ("bar", "stacked", "Stacked Bar"),
    ("bar", "stacked100", "100% Stacked Bar"),
    ("line", "", "Line"),
    ("line", "markers", "Line with Markers"),
    ("line", "stacked", "Stacked Line"),
    ("pie", "", "Pie"),
    ("doughnut", "", "Doughnut"),
    ("area", "", "Area"),
    ("area", "stacked", "Stacked Area"),
    ("area", "stacked100", "100% Stacked Area"),
    ("scatter", "", "Scatter"),
    ("scatter", "lines", "Scatter with Lines"),
    ("radar", "", "Radar"),
    ("histogram", "", "Histogram"),
    ("waterfall", "", "Waterfall"),
    ("funnel", "", "Funnel"),
    ("combo", "", "Combo"),
];

/// The chart types listed on the dialog's current tab.
fn chart_choices(d: &Dialog) -> Vec<(String, String, String)> {
    if d.tab == "All Charts" {
        return ALL_CHARTS.iter().map(|(t, s, l)| (t.to_string(), s.to_string(), l.to_string())).collect();
    }
    let field = |x: &Json, k: &str| x.get(k).and_then(Json::as_str).unwrap_or("").to_string();
    d.result
        .as_ref()
        .and_then(Json::as_array)
        .map(|a| a.iter().map(|x| (field(x, "type"), field(x, "subtype"), field(x, "label"))).collect())
        .unwrap_or_default()
}

/// Selects entry `i` of the current tab: OK inserts it.
pub(super) fn select_chart(d: &mut Dialog, i: usize) {
    let Some((t, sub, _)) = chart_choices(d).into_iter().nth(i) else { return };
    d.list_index = i;
    d.values.insert("type".into(), json!(t));
    d.values.insert("subtype".into(), json!(sub));
}

fn chart_icon(t: &str) -> Icon {
    match t {
        "column" | "bar" | "histogram" | "waterfall" | "funnel" | "combo" => Icon::ChartBar,
        "line" | "area" | "radar" => Icon::ChartLine,
        "pie" | "doughnut" => Icon::ChartPie,
        "scatter" | "bubble" => Icon::ChartScatter,
        _ => Icon::Chart,
    }
}

pub(super) fn recommended_charts(app: &mut SheetApp, ui: &mut egui::Ui, d: &mut Dialog, confirm: &mut bool, open: &mut bool) {
    ui.horizontal(|ui| {
        for tab in ["Recommended Charts", "All Charts"] {
            if ui.selectable_label(d.tab == tab, tab).clicked() && d.tab != tab {
                d.tab = tab.to_string();
                select_chart(d, 0);
            }
        }
    });
    ui.separator();
    let choices = chart_choices(d);
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(190.0);
            egui::ScrollArea::vertical().id_salt("chart_choices").max_height(300.0).show(ui, |ui| {
                for (i, (kind, _, label)) in choices.iter().enumerate() {
                    let resp = ui
                        .horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                            icons::paint(ui.painter(), r.shrink(2.0), chart_icon(kind), t.text);
                            ui.selectable_label(i == d.list_index, label.as_str())
                        })
                        .inner;
                    if resp.clicked() {
                        select_chart(d, i);
                    }
                    if resp.double_clicked() {
                        select_chart(d, i);
                        *confirm = true;
                    }
                }
            });
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            let label = choices.get(d.list_index).map(|c| c.2.clone()).unwrap_or_default();
            ui.strong(label);
            let (rect, _) = ui.allocate_exact_size(vec2(360.0, 216.0), Sense::hover());
            ui.painter().rect_filled(rect, 2.0, Color32::WHITE);
            ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, t.separator), StrokeKind::Inside);
            let params = json!({"type": d.values.get("type").cloned().unwrap_or(Json::Null), "subtype": d.values.get("subtype").cloned().unwrap_or(Json::Null)});
            if let Some(doc) = app.session.active()
                && let Ok((sheet, chart)) = gridcraft_engine::cmd::insert::build_chart(&app.session, &params)
            {
                crate::chartview::paint_chart(ui.painter(), rect, &doc.wb, sheet, &chart);
            }
        });
    });
    ok_cancel(ui, confirm, open);
    if *confirm {
        *confirm = false;
        let params =
            json!({"type": d.values.get("type").cloned().unwrap_or(Json::Null), "subtype": d.values.get("subtype").cloned().unwrap_or(Json::Null)});
        match app.run("insert.chart", params) {
            Ok(r) => {
                // The new chart is selected, as after inserting any object.
                app.selected_chart = r.get("chart").and_then(Json::as_u64).and_then(|id| u32::try_from(id).ok());
                *open = false;
            }
            Err(e) => app.message = Some((d.title.clone(), crate::clean_error(&e))),
        }
    }
}

/// Draws the compact chart data dialog. Each series is edited in its own dialog so long
/// references do not force the parent dialog to become excessively wide.
pub(super) fn select_data(app: &mut SheetApp, ui: &mut egui::Ui, d: &mut Dialog, confirm: &mut bool, open: &mut bool) {
    let text = |d: &Dialog, k: &str| d.values.get(k).and_then(Json::as_str).unwrap_or("").to_string();
    let edit = |ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str| {
        let label = ui.label(label);
        let mut v = text(d, key);
        if ui.add(egui::TextEdit::singleline(&mut v).desired_width(960.0)).labelled_by(label.id).changed() {
            d.values.insert(key.into(), json!(v));
        }
    };
    edit(ui, d, "range", "Chart data range:");
    ui.add_space(6.0);
    ui.strong("Legend Entries (Series)");
    ui.label("Use Select… to edit the legend title and the independent X/Y ranges for each curve.");
    let mut rows: Vec<(String, String, String)> = d
        .values
        .get("series")
        .and_then(Json::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    (
                        item.get("name").and_then(Json::as_str).unwrap_or("").to_string(),
                        item.get("x").and_then(Json::as_str).unwrap_or("").to_string(),
                        item.get("y").and_then(Json::as_str).unwrap_or("").to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    egui::Grid::new("chart_series_editor").striped(true).num_columns(5).spacing([6.0, 4.0]).show(ui, |ui| {
        ui.strong("Legend title");
        ui.strong("X values");
        ui.strong("Y values");
        ui.strong("");
        ui.strong("");
        ui.end_row();
        let mut remove = None;
        let mut edit = None;
        let can_remove = rows.len() > 1;
        for (i, (name, x, y)) in rows.iter_mut().enumerate() {
            ui.add_sized([150.0, 24.0], egui::Label::new(egui::RichText::new(name.as_str()).strong()));
            ui.add_sized([175.0, 24.0], egui::Label::new(egui::RichText::new(short_ref(x)).monospace()));
            ui.add_sized([175.0, 24.0], egui::Label::new(egui::RichText::new(short_ref(y)).monospace()));
            if ui.button("Select…").clicked() {
                edit = Some(i);
            }
            if ui.add_enabled(can_remove, egui::Button::new("Remove").small()).clicked() {
                remove = Some(i);
            }
            ui.end_row();
        }
        if let Some(i) = remove {
            rows.remove(i);
        }
        if let Some(i) = edit {
            d.values.insert("series".into(), json!(rows.iter().map(|(name, x, y)| json!({"name": name, "x": x, "y": y})).collect::<Vec<_>>()));
            let (name, x, y) = rows.get(i).cloned().unwrap_or_default();
            app.open_dialog("chartSeries", json!({"index": i, "name": name, "x": x, "y": y}));
        }
    });
    if ui.button("Add Series").clicked() {
        let i = rows.len();
        let name = format!("Series{}", i + 1);
        rows.push((name.clone(), String::new(), String::new()));
        app.open_dialog("chartSeries", json!({"index": i, "name": name, "x": "", "y": ""}));
    }
    let series: Vec<Json> = rows.iter().map(|(name, x, y)| json!({"name": name, "x": x, "y": y})).collect();
    d.values.insert("series".into(), json!(series));
    let uses_independent_xy = rows.len() > 1 && rows.iter().all(|(_, x, y)| !x.trim().is_empty() && !y.trim().is_empty());
    if !uses_independent_xy {
        ui.add_space(6.0);
        edit(ui, d, "categories", "Horizontal (Category) Axis Labels:");
    }
    ok_cancel(ui, confirm, open);
    if *confirm {
        *confirm = false;
        let mut params = serde_json::Map::new();
        params.insert("chart".into(), d.values.get("chart").cloned().unwrap_or(Json::Null));
        let series_changed = d.values.get("series") != d.values.get("series0");
        let range_changed = text(d, "range") != text(d, "range0");
        if series_changed {
            params.insert("series".into(), d.values.get("series").cloned().unwrap_or_else(|| json!([])));
        } else if range_changed {
            params.insert("range".into(), json!(text(d, "range")));
        }
        // Keep the pre-existing command path usable for automation and older callers that still
        // set the legacy `remove` field directly.
        if let Some(remove) = d.values.get("remove").and_then(Json::as_array).filter(|items| !items.is_empty()) {
            params.insert("removeSeries".into(), json!(remove));
        }
        if !series_changed && (text(d, "categories") != text(d, "categories0") || (range_changed && !text(d, "categories").is_empty())) {
            params.insert("categories".into(), json!(text(d, "categories")));
        }
        if params.len() == 1 {
            *open = false;
            return;
        }
        match app.run("chart.selectData", Json::Object(params)) {
            Ok(_) => *open = false,
            Err(e) => app.message = Some((d.title.clone(), crate::clean_error(&e))),
        }
    }
}

fn short_ref(value: &str) -> String {
    const MAX: usize = 25;
    let value = value.strip_prefix('=').unwrap_or(value);
    if value.chars().count() <= MAX {
        return value.to_string();
    }
    let end: String = value.chars().skip(value.chars().count().saturating_sub(MAX - 1)).collect();
    format!("…{end}")
}

/// Edits one series at a time. While X or Y picking is active, clicks and drags on the sheet are
/// routed through Dialog::pick_reference, exactly like Excel's range selector.
pub(super) fn chart_series(app: &mut SheetApp, ui: &mut egui::Ui, d: &mut Dialog, confirm: &mut bool, open: &mut bool) {
    ui.label("Legend title");
    let mut name = series_text(d, "name");
    if ui.add(egui::TextEdit::singleline(&mut name).desired_width(360.0)).changed() {
        d.values.insert("name".into(), json!(name));
    }
    ui.add_space(6.0);
    chart_series_field(ui, d, "x", "X values");
    chart_series_field(ui, d, "y", "Y values");
    ui.add_space(8.0);
    ok_cancel(ui, confirm, open);
    if *confirm {
        *confirm = false;
        if series_text(d, "y").trim().is_empty() {
            app.message = Some((d.title.clone(), "Y values cannot be empty.".into()));
            return;
        }
        let name = series_text(d, "name");
        let x = series_text(d, "x").trim_start_matches('=').to_string();
        let y = series_text(d, "y").trim_start_matches('=').to_string();
        if let Some(parent) = d.back.as_mut()
            && let Some(index) = d.values.get("index").and_then(Json::as_u64).and_then(|i| usize::try_from(i).ok())
            && let Some(series) = parent.values.get_mut("series").and_then(Json::as_array_mut)
            && let Some(row) = series.get_mut(index).and_then(Json::as_object_mut)
        {
            row.insert("name".into(), json!(name));
            row.insert("x".into(), json!(x));
            row.insert("y".into(), json!(y));
        }
        *open = false;
    }
}

fn series_text(d: &Dialog, key: &str) -> String {
    d.values.get(key).and_then(Json::as_str).unwrap_or("").to_string()
}

fn chart_series_field(ui: &mut egui::Ui, d: &mut Dialog, key: &'static str, label: &str) {
    ui.label(label);
    ui.horizontal(|ui| {
        let mut value = d.values.get(key).and_then(Json::as_str).unwrap_or("").to_string();
        let r = ui.add(egui::TextEdit::singleline(&mut value).desired_width(360.0));
        if r.changed() {
            d.values.insert(key.into(), json!(value));
        }
        if ui.button("Select…").clicked() {
            // Point mode is formula-based. Existing chart references are stored without the
            // leading `=`, so add it before handing the field to the shared range picker.
            if !value.trim().is_empty() && !value.trim_start().starts_with('=') {
                value = format!("={value}");
                d.values.insert(key.into(), json!(value.clone()));
            }
            let pointed = crate::editor::formula_refs(&value).last().map(|(start, end, ..)| (*end, Some((*start, *end))));
            let (caret, point) = pointed.unwrap_or((value.chars().count(), None));
            d.pick = Some(super::RefPick {
                key,
                // Chart series keep an explicit sheet qualifier, including for the active sheet.
                sheet: String::new(),
                caret,
                point,
                anchor: gridcraft_engine::core::CellRef::new(0, 0),
                refocus: false,
                applies_to: String::new(),
            });
        }
        let typed = r.changed();
        if let Some(pick) = d.pick.as_mut().filter(|pick| pick.key == key) {
            super::show::track_pick(pick, &r, typed);
        }
    });
}
