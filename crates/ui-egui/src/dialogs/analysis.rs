use super::show::{form_fields, ok_cancel, take_first_focus};
use super::*;

#[cfg(test)]
mod tests;

pub(super) fn goal_seek(app: &mut SheetApp, ui: &mut egui::Ui, d: &mut Dialog, confirm: &mut bool) {
    let get = |k: &str| d.values.get(k).and_then(Json::as_str).unwrap_or("").to_string();
    let (mut set, mut to, mut changing) = (get("set"), get("to"), get("changing"));
    egui::Grid::new("gs").show(ui, |ui| {
        ui.label("Set cell:");
        ui.text_edit_singleline(&mut set);
        ui.end_row();
        ui.label("To value:");
        ui.text_edit_singleline(&mut to);
        ui.end_row();
        ui.label("By changing cell:");
        ui.text_edit_singleline(&mut changing);
        ui.end_row();
    });
    d.values.insert("set".into(), json!(set.clone()));
    d.values.insert("to".into(), json!(to.clone()));
    d.values.insert("changing".into(), json!(changing.clone()));
    let mut open = true;
    ok_cancel(ui, confirm, &mut open);
    if *confirm {
        match app.run("data.goalSeek", json!({"set": set, "to": to.parse::<f64>().unwrap_or(0.0), "changing": changing})) {
            Ok(r) => app.toast = Some((format!("Goal Seeking found a solution: {}", r["value"]), crate::now_ms())),
            Err(e) => app.message = Some(("Goal Seek".into(), crate::clean_error(&e))),
        }
    }
    if !open {
        *confirm = true;
    }
}

/// Edits one analysis range and toggles worksheet selection for that field with Select… / Finish.
/// Typing cancels the active picker; editing `output` also selects range output. `single_cell`
/// makes subsequent grid picking keep only the drag's starting cell, as report output requires.
pub(super) fn range_editor(ui: &mut egui::Ui, d: &mut Dialog, key: &str, single_cell: bool) {
    ui.push_id(key, |ui| {
        ui.horizontal(|ui| {
            let mut text = d.values.get(key).and_then(Json::as_str).unwrap_or_default().to_string();
            let edit = ui.add(egui::TextEdit::singleline(&mut text).desired_width(240.0));
            take_first_focus(d, &edit);
            if edit.changed() {
                d.values.insert(key.into(), json!(text));
                d.range_picker = None;
                if key == "output" {
                    d.values.insert("outputTo".into(), json!("range"));
                }
            }
            let picking = d.range_picker.as_ref().is_some_and(|picker| picker.key == key);
            if ui.button(crate::widgets::tr(ui, if picking { "Finish" } else { "Select…" })).clicked() {
                d.range_picker = if picking { None } else { Some(RangePicker { key: key.into(), single_cell }) };
            }
        })
    });
}

/// Reports whether the grid may route pointer input to the dialog's pending range field.
/// Requires the owning document UID to be active, while permitting worksheet changes within it.
/// An inactive owner's picker is suspended, so returning to that document can resume selection.
pub(crate) fn dialog_range_picker_active(app: &SheetApp) -> bool {
    app.dialog.as_ref().is_some_and(|d| {
        d.range_picker.is_some()
            && d.analysis_context.as_ref().is_some_and(|context| app.session.active().is_some_and(|doc| doc.uid == context.document))
    })
}

/// Stores a grid-picked, worksheet-qualified range and mirrors it in the worksheet selection.
/// Ignores updates without a picker, from another document, or for a non-active worksheet.
/// Preserves other fields' relative-reference meaning before a sheet switch; output picking
/// uses only `start` and selects range output. Does not write report data or submit the analysis.
pub(crate) fn update_dialog_range_picker(app: &mut SheetApp, sheet: usize, start: CellRef, end: CellRef) {
    let Some(d) = app.dialog.as_mut() else { return };
    if d.prepare_analysis(&app.session).is_err() {
        return;
    }
    let Some(picker) = d.range_picker.clone() else { return };
    let Some(doc) = app.session.active().filter(|doc| doc.wb.active_sheet() == sheet) else { return };
    let Some(sh) = doc.wb.sheet(sheet) else { return };
    let range = RangeRef::new(start, if picker.single_cell { start } else { end });
    d.values.insert(picker.key.clone(), json!(format!("{}!{}", gridcraft_engine::formula::quote_sheet(&sh.name), range.a1())));
    if picker.key == "output" {
        d.values.insert("outputTo".into(), json!("range"));
    }
    let _ = app.session.run("selection.set", json!({"range": range.a1()}));
}

/// Declares one Analysis ToolPak tool's input/options schema; output controls are rendered separately.
/// Regression keeps the Y/X ranges and label checkbox first, then model choice and fit options:
/// its renderer groups these slices so labels and the zero-intercept option are not model choices.
fn toolpak_fields(tool: &str) -> Vec<Field> {
    use Field::*;
    let grouped = || Choice { key: "groupedBy", label: "Grouped By:", options: vec![("columns", "Columns"), ("rows", "Rows")] };
    let labels = || Check { key: "labels", label: "Labels in first row" };
    let range = || Range { key: "range", label: "Input Range:" };
    let two = || vec![Range { key: "range1", label: "Variable 1 Range:" }, Range { key: "range2", label: "Variable 2 Range:" }];
    match tool {
        "descriptive" => vec![
            range(),
            grouped(),
            labels(),
            Check { key: "summary", label: "Summary statistics" },
            Text { key: "confidence", label: "Confidence Level for Mean (%), blank for none:" },
            Text { key: "kthLargest", label: "Kth Largest (blank for none):" },
            Text { key: "kthSmallest", label: "Kth Smallest (blank for none):" },
        ],
        "histogram" => vec![
            range(),
            Range { key: "bins", label: "Bin Range (blank for automatic bins):" },
            labels(),
            Check { key: "pareto", label: "Pareto (sorted histogram)" },
            Check { key: "cumulative", label: "Cumulative Percentage" },
        ],
        "correlation" | "covariance" | "rankPercentile" => vec![range(), grouped(), labels()],
        "anovaSingle" => vec![range(), grouped(), labels(), Text { key: "alpha", label: "Alpha:" }],
        "regression" => vec![
            Range { key: "inputY", label: "Input Y Range:" },
            Range { key: "inputX", label: "Input X Range:" },
            labels(),
            Choice {
                key: "model",
                label: "Regression Type:",
                options: vec![("linear", "Linear"), ("exponential", "Exponential"), ("logarithmic", "Logarithmic"), ("power", "Power / Geometric")],
            },
            Check { key: "constantZero", label: "Constant is Zero" },
            Text { key: "confidence", label: "Confidence Level (%):" },
            Check { key: "residuals", label: "Residuals" },
            Check { key: "standardizedResiduals", label: "Standardized Residuals" },
            Check { key: "normalProbability", label: "Normal Probability" },
        ],
        "movingAverage" => {
            vec![range(), labels(), Text { key: "interval", label: "Interval:" }, Check { key: "standardErrors", label: "Standard Errors" }]
        }
        "exponentialSmoothing" => {
            vec![range(), labels(), Text { key: "damping", label: "Damping factor:" }, Check { key: "standardErrors", label: "Standard Errors" }]
        }
        "fTest" => {
            let mut v = two();
            v.extend([labels(), Text { key: "alpha", label: "Alpha:" }]);
            v
        }
        _ => {
            let mut v = two();
            v.extend([
                Text { key: "hypothesizedDifference", label: "Hypothesized Mean Difference:" },
                labels(),
                Text { key: "alpha", label: "Alpha:" },
            ]);
            v
        }
    }
}

/// Initializes a tool's draft values with `sel` as input and a new worksheet as report output.
/// Regression defaults to a linear model and 95% confidence; the opener/chooser qualifies the
/// selection against its bound worksheet after installing these values on the dialog.
pub(super) fn toolpak_defaults(sel: &str, tool: &str) -> Map<String, Json> {
    let mut v = json!({
        "tool": tool, "range": sel, "inputY": "", "inputX": "", "range1": sel, "range2": "", "bins": "",
        "groupedBy": "columns", "labels": false, "summary": true, "confidence": "", "kthLargest": "", "kthSmallest": "",
        "interval": "3", "damping": "0.3", "alpha": "0.05", "hypothesizedDifference": "0",
        "model": "linear",
        "outputTo": "newSheet", "output": "", "newSheet": "",
    });
    if tool == "regression" {
        v["confidence"] = json!("95");
        v["inputY"] = json!(sel);
    }
    v.as_object().cloned().unwrap_or_default()
}

/// Data › Data Analysis: pick a tool, then fill in its form. OK runs `data.analysisToolPak`
/// (through the dialog's command); a report that would replace data asks first.
/// A mismatched document offers cancellation instead of execution. While picking a range, shows
/// only that field so grid selection remains available; regression otherwise separates inputs,
/// model and fit options and explains diagnostics for a transformed response.
pub(super) fn analysis_toolpak(app: &mut SheetApp, ui: &mut egui::Ui, d: &mut Dialog, confirm: &mut bool, open: &mut bool) {
    use gridcraft_engine::cmd::analysis::TOOLS;
    let lang = app.ui.language;
    if let Err(error) = d.prepare_analysis(&app.session) {
        ui.label(lang.tr(&error));
        if ui.button(lang.tr("Cancel")).clicked() {
            *open = false;
        }
        return;
    }
    if let Some(picker) = d.range_picker.clone() {
        ui.label(lang.tr("Select a worksheet range, then choose Finish."));
        let label = d
            .fields
            .iter()
            .find_map(|field| match field {
                Field::Range { key, label } if *key == picker.key => Some(*label),
                _ => None,
            })
            .unwrap_or("Output Range:");
        ui.label(lang.tr(label));
        range_editor(ui, d, &picker.key, picker.single_cell);
        return;
    }
    if d.values.get("askOverwrite").and_then(Json::as_bool) == Some(true) {
        ui.label("The output range already holds data. Replace it with the report?");
        ui.horizontal(|ui| {
            if ui.button("Replace").clicked() {
                d.values.remove("askOverwrite");
                d.values.insert("overwrite".into(), json!(true));
                *confirm = true;
            }
            if ui.button("Cancel").clicked() {
                d.values.remove("askOverwrite");
            }
        });
        return;
    }
    let tool = d.values.get("tool").and_then(Json::as_str).unwrap_or("").to_string();
    let Some((_, label)) = TOOLS.iter().find(|(id, _)| *id == tool) else {
        ui.label(egui::RichText::new("Analysis Tools").strong());
        let pick = d.values.get("pick").and_then(Json::as_str).unwrap_or("").to_string();
        let mut chosen = false;
        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            for (id, label) in TOOLS {
                let r = ui.selectable_label(pick == *id, *label);
                if r.clicked() {
                    d.values.insert("pick".into(), json!(id));
                }
                if r.double_clicked() {
                    chosen = true;
                }
            }
        });
        let mut ok = false;
        ok_cancel(ui, &mut ok, open);
        let pick = d.values.get("pick").and_then(Json::as_str).unwrap_or("").to_string();
        if (ok || chosen) && TOOLS.iter().any(|(id, _)| *id == pick) {
            let sel = app.session.active().map(|doc| doc.selection.current().a1()).unwrap_or_default();
            d.values = toolpak_defaults(&sel, &pick);
            d.qualify_analysis_ranges();
        }
        return;
    };
    d.title = label.to_string();
    d.fields = toolpak_fields(&tool);
    if tool == "regression" {
        let fields = std::mem::take(&mut d.fields);
        d.fields = fields.iter().take(3).cloned().collect();
        ui.push_id("regression-inputs", |ui| form_fields(ui, d));
        ui.separator();
        ui.label(egui::RichText::new(lang.tr("Regression method")).strong());
        if let Some(Field::Choice { key, label, options }) = fields.get(3) {
            ui.label(lang.tr(label));
            let mut selected = d.values.get(*key).and_then(Json::as_str).unwrap_or("linear").to_string();
            let text = options.iter().find(|(value, _)| *value == selected).map(|(_, label)| lang.tr(label)).unwrap_or("");
            egui::ComboBox::from_id_salt("regression-model").selected_text(text).show_ui(ui, |ui| {
                for (value, label) in options {
                    ui.selectable_value(&mut selected, (*value).into(), lang.tr(label));
                }
            });
            d.values.insert((*key).into(), json!(selected));
        }
        ui.separator();
        ui.label(egui::RichText::new(lang.tr("Regression options")).strong());
        d.fields = fields.get(4..).unwrap_or_default().to_vec();
        ui.push_id("regression-options", |ui| form_fields(ui, d));
        d.fields = fields;
        if matches!(d.values.get("model").and_then(Json::as_str), Some("exponential" | "power")) {
            ui.add(egui::Label::new(lang.tr("Diagnostics and the intercept use ln(Y); a zero intercept fixes the amplitude to 1.")).wrap());
        }
    } else {
        form_fields(ui, d);
    }
    ui.separator();
    ui.label(egui::RichText::new("Output options").strong());
    let to = d.values.get("outputTo").and_then(Json::as_str).unwrap_or("newSheet").to_string();
    for (key, choice, text_label) in [("output", "range", "Output Range:"), ("newSheet", "newSheet", "New Worksheet Ply:")] {
        ui.horizontal(|ui| {
            if ui.radio(to == choice, text_label).clicked() {
                d.values.insert("outputTo".into(), json!(choice));
            }
            if choice == "range" {
                range_editor(ui, d, key, true);
            } else {
                let mut t = d.values.get(key).and_then(Json::as_str).unwrap_or("").to_string();
                if ui.add(egui::TextEdit::singleline(&mut t).desired_width(160.0)).changed() {
                    d.values.insert(key.into(), json!(t));
                    d.values.insert("outputTo".into(), json!(choice));
                }
            }
        });
    }
    ok_cancel(ui, confirm, open);
}

/// Data › Solver: the Solver Parameters form. Solve runs `data.solver` through the dialog's
/// command; the result opens Solver Results.
pub(super) fn solver(ui: &mut egui::Ui, d: &mut Dialog, confirm: &mut bool, open: &mut bool) {
    let lang = crate::i18n::current(ui.ctx());
    let text = |d: &Dialog, k: &str| d.values.get(k).map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).unwrap_or_default();
    let edit = |ui: &mut egui::Ui, d: &mut Dialog, k: &str, width: f32, label: &str| {
        let mut t = text(d, k);
        if ui.add(egui::TextEdit::singleline(&mut t).desired_width(width)).accessible_name(lang.tr(label)).changed() {
            d.values.insert(k.into(), json!(t));
        }
    };
    ui.horizontal(|ui| {
        ui.label("Set Objective:");
        edit(ui, d, "objective", 200.0, "Set Objective:");
    });
    let goal = text(d, "goal");
    ui.horizontal(|ui| {
        ui.label("To:");
        for (g, label) in [("max", "Max"), ("min", "Min"), ("value", "Value Of:")] {
            if ui.radio(goal == g, label).clicked() {
                d.values.insert("goal".into(), json!(g));
            }
        }
        edit(ui, d, "value", 80.0, "Value Of:");
    });
    ui.label("By Changing Variable Cells:");
    edit(ui, d, "variables", 420.0, "By Changing Variable Cells:");
    ui.label("Subject to the Constraints:");
    let list: Vec<String> = d
        .values
        .get("constraints")
        .and_then(Json::as_array)
        .map(|a| a.iter().map(|c| c.as_str().map(str::to_string).unwrap_or_else(|| c.to_string())).collect())
        .unwrap_or_default();
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_size(vec2(420.0, 90.0));
        egui::ScrollArea::vertical().max_height(110.0).show(ui, |ui| {
            for (i, c) in list.iter().enumerate() {
                if ui.selectable_label(d.list_index == i, c).clicked() {
                    d.list_index = i;
                }
            }
        });
    });
    ui.horizontal(|ui| {
        edit(ui, d, "newLhs", 110.0, "Constraint Cell Reference");
        let op = text(d, "newOp");
        craft_ui::ComboBox::from_id_salt("solverOp")
            .selected_text(op.clone())
            .width(60.0)
            .show_ui(ui, |ui| {
                for o in ["<=", "=", ">=", "int", "bin"] {
                    if ui.selectable_label(op == o, o).clicked() {
                        d.values.insert("newOp".into(), json!(o));
                    }
                }
            })
            .response
            .accessible_name(lang.tr("Constraint Operator"));
        if !matches!(op.as_str(), "int" | "bin") {
            edit(ui, d, "newRhs", 110.0, "Constraint Value");
        }
        if ui.button("Add").clicked() {
            let lhs = text(d, "newLhs");
            if !lhs.trim().is_empty() {
                let rhs = if matches!(op.as_str(), "int" | "bin") { String::new() } else { format!(" {}", text(d, "newRhs").trim()) };
                let mut list = list.clone();
                list.push(format!("{} {op}{rhs}", lhs.trim()));
                d.values.insert("constraints".into(), json!(list));
                d.values.insert("newLhs".into(), json!(""));
                d.values.insert("newRhs".into(), json!(""));
            }
        }
        if ui.add_enabled(d.list_index < list.len(), egui::Button::new("Delete")).clicked() {
            let mut list = list.clone();
            if d.list_index < list.len() {
                list.remove(d.list_index);
            }
            d.values.insert("constraints".into(), json!(list));
        }
    });
    let mut nonneg = d.values.get("nonNegative").and_then(Json::as_bool).unwrap_or(true);
    if ui.checkbox(&mut nonneg, "Make Unconstrained Variables Non-Negative").changed() {
        d.values.insert("nonNegative".into(), json!(nonneg));
    }
    let method = text(d, "method");
    ui.horizontal(|ui| {
        ui.label("Select a Solving Method:");
        let label = |m: &str| match m {
            "simplex" => "Simplex LP",
            "evolutionary" => "Evolutionary",
            _ => "GRG Nonlinear",
        };
        craft_ui::ComboBox::from_id_salt("solverMethod")
            .selected_text(label(&method))
            .show_ui(ui, |ui| {
                for m in ["grg", "simplex", "evolutionary"] {
                    if ui.selectable_label(method == m, label(m)).clicked() {
                        d.values.insert("method".into(), json!(m));
                    }
                }
            })
            .response
            .accessible_name(lang.tr("Select a Solving Method:"));
    });
    ui.label(
        egui::RichText::new(match method.as_str() {
            "simplex" => "Simplex LP: for linear models; integer and binary constraints by branch and bound.",
            "evolutionary" => "Evolutionary: a genetic search for models that aren't smooth; every variable cell needs bounds.",
            _ => "GRG Nonlinear: a local optimum of a smooth model, from the current values.",
        })
        .small(),
    );
    ui.add_space(8.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let t = crate::theme::Tokens::get(ui.ctx());
        let solve = ui.add(egui::Button::new(egui::RichText::new("  Solve  ").color(t.on_accent)).fill(t.accent));
        if solve.clicked() && !text(d, "objective").trim().is_empty() {
            *confirm = true;
        }
        if ui.button(" Close ").clicked() {
            *open = false;
        }
    });
}

/// Solver Results: the outcome, and keeping or restoring the variable cells.
pub(super) fn solver_results(app: &mut SheetApp, ui: &mut egui::Ui, d: &mut Dialog, open: &mut bool) {
    let message = d.values.get("message").and_then(Json::as_str).unwrap_or("").to_string();
    ui.label(message);
    let changed = d.values.get("changed").and_then(Json::as_bool).unwrap_or(false);
    let restore = d.values.get("restore").and_then(Json::as_bool).unwrap_or(false);
    if changed {
        ui.add_space(4.0);
        if ui.radio(!restore, "Keep Solver Solution").clicked() {
            d.values.insert("restore".into(), json!(false));
        }
        if ui.radio(restore, "Restore Original Values").clicked() {
            d.values.insert("restore".into(), json!(true));
        }
    }
    // Reports describe the solution whether it is kept or not.
    let solved = d.values.get("values").and_then(Json::as_object).is_some_and(|v| !v.is_empty());
    if solved {
        ui.add_space(4.0);
        ui.label("Reports:");
        for (key, label) in [("answerReport", "Answer"), ("limitsReport", "Limits")] {
            let mut on = d.values.get(key).and_then(Json::as_bool).unwrap_or(false);
            if ui.checkbox(&mut on, label).changed() {
                d.values.insert(key.into(), json!(on));
            }
        }
    }
    let mut confirm = false;
    ok_cancel(ui, &mut confirm, open);
    if confirm {
        // The solution (and the saved model) is one undo step.
        if changed && restore {
            app.run_or_alert("edit.undo", json!({}));
        }
        let reports: Vec<&str> = [("answerReport", "answer"), ("limitsReport", "limits")]
            .into_iter()
            .filter(|(key, _)| d.values.get(*key).and_then(Json::as_bool) == Some(true))
            .map(|(_, kind)| kind)
            .collect();
        if solved && !reports.is_empty() {
            let field = |k: &str| d.values.get(k).cloned().unwrap_or(Json::Null);
            let params = json!({"reports": reports, "model": field("model"), "values": field("values"), "original": field("original"), "message": field("message")});
            app.run_or_alert("data.solverReport", params);
        }
        *open = false;
    }
}
