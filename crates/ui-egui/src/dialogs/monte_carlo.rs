//! Thin editor for the engine's generic Monte Carlo command.
use super::{Dialog, Json, Map, json};

/// Edits a labeled textual parameter while retaining typed numeric values until an actual edit.
/// Engine validation receives invalid text unchanged instead of silently substituting a default.
fn text(ui: &mut egui::Ui, values: &mut Map<String, Json>, key: &str, label: &str) {
    ui.horizontal(|ui| {
        let label = ui.label(label);
        let mut value = values
            .get(key)
            .map(|v| match v {
                Json::String(s) => s.clone(),
                _ => v.to_string(),
            })
            .unwrap_or_default();
        if ui.add(egui::TextEdit::singleline(&mut value).desired_width(110.0)).labelled_by(label.id).changed() {
            values.insert(key.into(), json!(value));
        }
    });
}

/// Draws the bounded input editor, sampling settings, matrix and outputs without evaluating
/// formulas. Replica settings apply only to LHS; confirmation delegates validation to the engine.
pub(super) fn form(ui: &mut egui::Ui, d: &mut Dialog, confirm: &mut bool, open: &mut bool) {
    ui.label(
        "Choose numeric input cells and formula outputs. Source values are kept unchanged; optional correlation is on latent Gaussian variables.",
    );
    ui.separator();
    let mut inputs = d.values.get("inputs").and_then(Json::as_array).cloned().unwrap_or_default();
    let mut remove = None;
    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
        for (index, item) in inputs.iter_mut().enumerate() {
            ui.push_id(index, |ui| {
                ui.group(|ui| {
                    if let Some(values) = item.as_object_mut() {
                        ui.horizontal(|ui| {
                            text(ui, values, "cell", "Cell:");
                            let mut kind = values.get("distribution").and_then(Json::as_str).unwrap_or("uniform").to_string();
                            let label = ui.label("Distribution:");
                            craft_ui::ComboBox::from_id_salt("distribution")
                                .selected_text(&kind)
                                .show_ui(ui, |ui| {
                                    for k in ["uniform", "triangular", "normal"] {
                                        ui.selectable_value(&mut kind, k.to_string(), k);
                                    }
                                })
                                .response
                                .labelled_by(label.id);
                            values.insert("distribution".into(), json!(kind));
                            if ui.button("Remove").clicked() {
                                remove = Some(index);
                            }
                        });
                        if values.get("distribution").and_then(Json::as_str) == Some("normal") {
                            ui.horizontal(|ui| {
                                text(ui, values, "mean", "Mean:");
                                text(ui, values, "stdDev", "Standard deviation:");
                            });
                        } else {
                            ui.horizontal(|ui| {
                                text(ui, values, "min", "Minimum:");
                                if values.get("distribution").and_then(Json::as_str) == Some("triangular") {
                                    text(ui, values, "mode", "Mode:");
                                }
                                text(ui, values, "max", "Maximum:");
                            });
                        }
                    }
                });
            });
        }
    });
    if let Some(i) = remove {
        inputs.remove(i);
    }
    if inputs.len() < 32 && ui.button("Add input").clicked() {
        inputs.push(json!({"cell":"","distribution":"uniform","min":"0","mode":"0.5","max":"1","mean":"0","stdDev":"1"}));
    }
    d.values.insert("inputs".into(), json!(inputs));
    ui.separator();
    ui.horizontal(|ui| {
        let label = ui.label("Sampling:");
        let mut method = d.values.get("method").and_then(Json::as_str).unwrap_or("random").to_string();
        let selected = if method == "latinHypercube" { "Latin Hypercube" } else { "Random Monte Carlo" };
        craft_ui::ComboBox::from_id_salt("mcMethod")
            .selected_text(selected)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut method, "random".into(), "Random Monte Carlo");
                ui.selectable_value(&mut method, "latinHypercube".into(), "Latin Hypercube");
            })
            .response
            .labelled_by(label.id);
        if method == "latinHypercube" {
            text(ui, &mut d.values, "replicates", "Independent replicas:");
        }
        d.values.insert("method".into(), json!(method));
    });
    let matrix_label = ui.label("Correlation matrix in input order (blank = independent). Rows on separate lines, numbers separated by spaces. Gaussian latent correlations, not marginal Pearson targets.");
    let mut matrix = match d.values.get("correlation") {
        Some(Json::String(s)) => s.clone(),
        Some(Json::Array(rows)) => rows
            .iter()
            .filter_map(Json::as_array)
            .map(|row| row.iter().map(Json::to_string).collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    let matrix_response = ui.add(egui::TextEdit::multiline(&mut matrix).desired_rows(2).desired_width(510.0)).labelled_by(matrix_label.id);
    let mut multiline_focused = matrix_response.has_focus();
    if matrix_response.changed() {
        d.values.insert("correlation".into(), json!(matrix));
    }
    let outputs_label = ui.label("Output cells, one per line (e.g. Model!F30). Qualify references to other sheets.");
    let mut outputs = d.values.get("outputs").and_then(Json::as_str).unwrap_or("").to_string();
    let outputs_response = ui.add(egui::TextEdit::multiline(&mut outputs).desired_rows(2).desired_width(510.0)).labelled_by(outputs_label.id);
    multiline_focused |= outputs_response.has_focus();
    if outputs_response.changed() {
        d.values.insert("outputs".into(), json!(outputs));
    }
    ui.horizontal(|ui| {
        text(ui, &mut d.values, "iterations", "Iterations:");
        text(ui, &mut d.values, "seed", "Seed:");
    });
    let mut samples = d.values.get("samples").and_then(Json::as_bool).unwrap_or(true);
    if ui.checkbox(&mut samples, "Include samples in the report").changed() {
        d.values.insert("samples".into(), json!(samples));
    }
    ui.label("Creates a static report with percentiles and a Monte Carlo tornado for the first output. Re-run after changing the model. Large models may take time.");
    super::show::ok_cancel_with_enter(ui, confirm, open, !multiline_focused);
}

/// Invalid typed numbers stay strings so the engine rejects them instead of silently defaulting.
/// Replica counts are sent only for LHS, allowing arbitrary valid random iteration counts;
/// output references stay line-delimited and correlation rows preserve invalid numeric tokens.
pub(super) fn params(d: &Dialog) -> Json {
    let mut values = d.values.clone();
    if values.get("method").and_then(Json::as_str) != Some("latinHypercube") {
        values.remove("replicates");
    }
    for key in ["iterations", "seed", "replicates"] {
        if let Some(n) = values.get(key).and_then(Json::as_str).and_then(|s| s.trim().parse::<u64>().ok()) {
            values.insert(key.into(), json!(n));
        }
    }
    if let Some(inputs) = values.get_mut("inputs").and_then(Json::as_array_mut) {
        for input in inputs {
            if let Some(input) = input.as_object_mut() {
                for key in ["min", "mode", "max", "mean", "stdDev"] {
                    if let Some(n) = input.get(key).and_then(Json::as_str).and_then(|s| s.trim().parse::<f64>().ok()).filter(|n| n.is_finite()) {
                        input.insert(key.into(), json!(n));
                    }
                }
            }
        }
    }
    if let Some(outputs) = values.get("outputs").and_then(Json::as_str) {
        let outputs = outputs.lines().map(str::trim).filter(|s| !s.is_empty()).map(|s| json!(s)).collect::<Vec<_>>();
        values.insert("outputs".into(), json!(outputs));
    }
    if let Some(text) = values.get("correlation").and_then(Json::as_str) {
        if text.trim().is_empty() {
            values.remove("correlation");
        } else {
            let matrix = text
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| {
                    Json::Array(
                        line.split(|c: char| c.is_whitespace() || c == ',' || c == ';')
                            .filter(|s| !s.is_empty())
                            .map(|v| v.parse::<f64>().ok().filter(|n| n.is_finite()).map(|n| json!(n)).unwrap_or_else(|| json!(v)))
                            .collect(),
                    )
                })
                .collect::<Vec<_>>();
            values.insert("correlation".into(), json!(matrix));
        }
    }
    Json::Object(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Invalid numeric text must survive UI conversion so the engine can report it to the user.
    #[test]
    fn monte_carlo_dialog_builds_typed_params_without_hiding_invalid_input() {
        let d = Dialog::custom(
            "monteCarlo",
            "Monte Carlo",
            json!({"iterations":"2000","seed":"42","inputs":[{"cell":"B2","distribution":"normal","mean":"10","stdDev":"bad"}],"outputs":"Sheet1!C1\n'Other, sheet'!B2\n"}),
        );
        let p = params(&d);
        assert_eq!(p["iterations"], 2000);
        assert_eq!(p["seed"], 42);
        assert_eq!(p["inputs"][0]["mean"], 10.0);
        assert_eq!(p["inputs"][0]["stdDev"], "bad");
        assert_eq!(p["outputs"].as_array().map(Vec::len), Some(2));
    }

    /// Switching a dialog from LHS to random must remove the stale replica restriction.
    /// Exercise the resulting command as well as the serialized parameters for odd trial counts.
    #[test]
    fn monte_carlo_random_dialog_accepts_iterations_not_divisible_by_lhs_replicates() {
        for iterations in [1001, 1500] {
            let mut d = Dialog::custom(
                "monteCarlo",
                "Monte Carlo",
                json!({"method":"latinHypercube","replicates":"8","iterations":iterations.to_string(),"inputs":[{"cell":"A1","min":"0","max":"1"}],"outputs":"B1","samples":false}),
            );
            let lhs = params(&d);
            assert_eq!(lhs["replicates"], 8);
            d.values.insert("method".into(), json!("random"));
            let p = params(&d);
            assert!(p.get("replicates").is_none());
            let mut session = gridcraft_engine::Session::new();
            session.new_workbook();
            session.execute("cell.set", json!({"cell":"A1","input":"0.5"})).unwrap();
            session.execute("cell.set", json!({"cell":"B1","input":"=A1*2"})).unwrap();
            let result = session.execute("data.monteCarlo", p).unwrap();
            assert_eq!(result["iterations"], iterations);
            assert_eq!(result["replicates"], 1);
        }
    }
}
