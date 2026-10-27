use super::show::ok_cancel;
use super::*;

/// Data ▸ Queries & Connections: the workbook's saved From Web connections, each with a
/// one-click Sync, its last-synced time, and the refresh errors and unavailable-source
/// messages in the same window (Excel's Queries & Connections pane).
pub(super) fn queries_connections(app: &mut SheetApp, ui: &mut egui::Ui, _d: &mut Dialog, confirm: &mut bool, open: &mut bool) {
    let queries = app.run("data.queriesConnections", json!({})).ok().and_then(|r| r.as_array().cloned()).unwrap_or_default();
    if queries.is_empty() {
        ui.label("No saved connections. Import a table with Data ▸ From Web to create one.");
    } else {
        egui::ScrollArea::vertical().max_height(280.0).id_salt("qc_list").show(ui, |ui| {
            for q in &queries {
                let name = q["name"].as_str().unwrap_or_default();
                let sheet = q["sheet"].as_str().unwrap_or_default();
                egui::CollapsingHeader::new(name).default_open(true).show(ui, |ui| {
                    ui.label(format!("Sheet: {sheet}"));
                    if let Some(url) = q.get("url").and_then(Json::as_str) {
                        ui.label(format!("Source: {}", crate::clean_url_for_display(url)));
                    }
                    let synced = q.get("last_synced").and_then(Json::as_u64).filter(|t| *t > 0);
                    match synced {
                        Some(ms) => ui.weak(format!("Last synced: {}", crate::format_sync_time(ms))),
                        None => ui.weak("Last synced: never"),
                    };
                    ui.horizontal(|ui| {
                        if ui.button("Sync").clicked() {
                            match app.run("data.webRefresh", json!({"name": name})) {
                                Ok(r) => {
                                    let refreshed = r["refreshed"].as_u64().unwrap_or(0);
                                    let errors = r["errors"].as_array().map(Vec::len).unwrap_or(0);
                                    if refreshed > 0 && errors == 0 {
                                        app.toast = Some((format!("Synced {name}"), crate::now_ms()));
                                    } else if let Some(e) = r["errors"].as_array().and_then(|a| a.first()) {
                                        let msg = e["error"].as_str().unwrap_or_default();
                                        app.message = Some(("Queries & Connections".into(), format!("{name}: {msg}")));
                                    }
                                }
                                Err(e) => app.message = Some(("Queries & Connections".into(), crate::clean_error(&e))),
                            }
                        }
                        if ui.button("Go to table").clicked() {
                            let _ = app.run("sheet.activate", json!({"sheet": sheet}));
                            *open = false;
                        }
                    });
                });
                ui.separator();
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Refresh All").clicked() {
                match app.run("data.webRefresh", json!({})) {
                    Ok(r) => {
                        let refreshed = r["refreshed"].as_u64().unwrap_or(0);
                        let errors = r["errors"].as_array().cloned().unwrap_or_default();
                        if errors.is_empty() {
                            app.toast = Some((format!("Synced {refreshed} connection(s)"), crate::now_ms()));
                        } else {
                            let msg = errors.iter().filter_map(|e| e["error"].as_str()).collect::<Vec<_>>().join("; ");
                            app.message = Some(("Queries & Connections".into(), msg));
                        }
                    }
                    Err(e) => app.message = Some(("Queries & Connections".into(), crate::clean_error(&e))),
                }
            }
        });
    }
    let mut load = true;
    ok_cancel(ui, confirm, &mut load);
    if *confirm || !load {
        *open = false;
    }
}

/// Data ▸ From Web: Excel's Navigator. Enter a URL, the page's detected tables are previewed
/// on the right, and Load imports the selected one into a new sheet.
pub(super) fn from_web(app: &mut SheetApp, ui: &mut egui::Ui, d: &mut Dialog, confirm: &mut bool, open: &mut bool) {
    let get = |k: &str| d.values.get(k).and_then(Json::as_str).unwrap_or("").to_string();
    let mut url = get("url");
    egui::Grid::new("fw").show(ui, |ui| {
        ui.label("URL:");
        ui.add(egui::TextEdit::singleline(&mut url).desired_width(360.0).hint_text("https://example.com/table.html"));
        // An edited address invalidates the preview: don't let Load pair the old page's
        // table names with the new URL.
        if d.values.get("preview_url").and_then(Json::as_str).is_some_and(|u| u != url.trim()) {
            d.values.remove("preview_url");
            d.values.remove("tables");
            d.values.remove("pick");
        }
        if ui.button("Go").clicked() && !url.trim().is_empty() {
            d.values.insert("url".into(), json!(url.trim()));
            d.values.insert("preview_url".into(), json!(url.trim()));
            d.values.insert("tables".into(), Json::Null);
            d.values.insert("pick".into(), json!(0));
            let r = app.run("data.webPreview", json!({"url": url.trim()}));
            match r {
                Ok(res) => d.values.insert("tables".into(), res["tables"].clone()),
                Err(e) => {
                    app.message = Some(("From Web".into(), crate::clean_error(&e)));
                    d.values.insert("tables".into(), Json::Null)
                }
            };
        }
        ui.end_row();
    });
    let tables: Vec<Json> = d.values.get("tables").and_then(Json::as_array).cloned().unwrap_or_default();
    if !tables.is_empty() {
        ui.separator();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.strong("Suggested Tables");
                let mut pick = d.values.get("pick").and_then(Json::as_u64).unwrap_or(0) as usize;
                for (i, t) in tables.iter().enumerate() {
                    let name = t["name"].as_str().unwrap_or("Table");
                    let rows = t["rows"].as_u64().unwrap_or(0);
                    if ui.selectable_label(i == pick, format!("{name}  ({rows} rows)")).clicked() {
                        pick = i;
                        d.values.insert("pick".into(), json!(i));
                    }
                }
            });
            ui.separator();
            let pick = d.values.get("pick").and_then(Json::as_u64).unwrap_or(0) as usize;
            if let Some(t) = tables.get(pick) {
                ui.vertical(|ui| {
                    ui.strong("Preview");
                    egui::ScrollArea::vertical().max_height(200.0).id_salt("fw_preview").show(ui, |ui| {
                        egui::Grid::new("fw_preview_grid").striped(true).show(ui, |ui| {
                            if let Some(rows) = t["preview"].as_array() {
                                for row in rows {
                                    for cell in row.as_array().into_iter().flatten() {
                                        ui.label(cell.as_str().unwrap_or_default());
                                    }
                                    ui.end_row();
                                }
                            }
                        });
                    });
                });
            }
        });
    } else if d.values.get("tables").is_some_and(|t| t.is_null()) {
        ui.label("No tables were found on this page.");
    } else {
        ui.label("Enter an address and choose Go to detect the page's tables.");
    }
    d.values.insert("url".into(), json!(url.clone()));
    let mut load = true;
    ok_cancel(ui, confirm, &mut load);
    if *confirm && url.trim().is_empty() {
        // OK with no address: stay open and say so, instead of closing with nothing.
        app.message = Some(("From Web".into(), "Enter an https:// address first.".into()));
        *confirm = false;
    }
    if *confirm && !url.trim().is_empty() {
        let pick = d.values.get("pick").and_then(Json::as_u64).unwrap_or(0) as usize;
        let table = tables.get(pick).and_then(|t| t["name"].as_str()).map(String::from);
        let params = match table {
            Some(t) => {
                // A page can hold duplicate table names: which one was picked is which one to
                // import, and the connection saves it so refreshes keep importing the same.
                let occurrence = tables.iter().take(pick).filter(|o| o["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(&t))).count();
                json!({"url": url.trim(), "table": t, "occurrence": occurrence})
            }
            None => json!({"url": url.trim()}),
        };
        match app.run("data.fromWeb", params) {
            Ok(r) => app.toast = Some((format!("Imported {} rows into {}", r["rows"], r["sheet"]), crate::now_ms())),
            Err(e) => {
                app.message = Some(("From Web".into(), crate::clean_error(&e)));
                *confirm = false;
            }
        }
    }
    if !load {
        *confirm = true;
        *open = false;
    }
}
