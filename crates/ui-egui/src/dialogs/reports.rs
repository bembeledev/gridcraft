//! Workbook Statistics, Accessibility, Error Checking, Evaluate Formula and the sheet's comments:
//! the engine's report drawn as labelled rows in the interface language, not as raw JSON.

use super::Json;
use crate::i18n::Language;

/// A report value as the user reads it: text and numbers unquoted, a cell error as its code.
fn text(v: &Json) -> String {
    match v {
        Json::Null => String::new(),
        Json::String(s) => s.clone(),
        // JSON writes a whole float as `3.0`; the sheet shows 3.
        Json::Number(n) => n.as_f64().map_or_else(|| n.to_string(), |f| f.to_string()),
        Json::Object(m) if m.contains_key("error") => text(&m["error"]),
        other => other.to_string(),
    }
}

fn rows(ui: &mut egui::Ui, lang: Language, id: &str, report: &Json, rows: &[(&str, &str)]) {
    egui::Grid::new(id).num_columns(2).spacing([24.0, 4.0]).striped(true).show(ui, |ui| {
        for (label, key) in rows {
            ui.label(lang.tr(label));
            ui.label(text(&report[*key]));
            ui.end_row();
        }
    });
}

fn table(ui: &mut egui::Ui, lang: Language, id: &str, headers: &[&str], body: &[Vec<String>]) {
    egui::Grid::new(id).num_columns(headers.len()).spacing([24.0, 4.0]).striped(true).show(ui, |ui| {
        for h in headers {
            ui.strong(lang.tr(h));
        }
        ui.end_row();
        for row in body {
            for cell in row {
                ui.label(cell);
            }
            ui.end_row();
        }
    });
}

fn list(v: &Json) -> &[Json] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}

/// Draws the report dialog `name` from its engine result `r`.
pub(super) fn report(ui: &mut egui::Ui, name: &str, r: &Json) {
    let lang = crate::i18n::current(ui.ctx());
    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| match name {
        "statistics" => {
            let sheet = &r["sheet"];
            ui.strong(lang.tr("Current sheet: {name}").replace("{name}", &text(&sheet["name"])));
            rows(
                ui,
                lang,
                "stats_sheet",
                sheet,
                &[
                    ("End of sheet", "endOfSheet"),
                    ("Cells with data", "cellsWithData"),
                    ("Tables", "tables"),
                    ("Formulas", "formulas"),
                    ("Charts", "charts"),
                    ("Comments", "comments"),
                ],
            );
            ui.add_space(8.0);
            ui.strong(lang.tr("Workbook:"));
            rows(
                ui,
                lang,
                "stats_workbook",
                &r["workbook"],
                &[("Sheets", "sheets"), ("Cells with data", "cellsWithData"), ("Tables", "tables"), ("Charts", "charts"), ("Names", "names")],
            );
        }
        "accessibility" => {
            let issues = list(r);
            if issues.is_empty() {
                ui.label(lang.tr("No accessibility issues found."));
                return;
            }
            let body: Vec<Vec<String>> = issues
                .iter()
                .map(|i| {
                    let detail = match i.get("count") {
                        Some(n) => lang.tr("Merged ranges: {count}").replace("{count}", &text(n)),
                        None => text(&i["object"]),
                    };
                    vec![text(&i["sheet"]), lang.tr(&text(&i["issue"])).to_string(), detail]
                })
                .collect();
            table(ui, lang, "accessibility_issues", &["Sheet", "Issue", "Details"], &body);
        }
        "errorChecking" => {
            let (errors, circular) = (list(&r["errors"]), list(&r["circular"]));
            if errors.is_empty() && circular.is_empty() {
                ui.label(lang.tr("No errors found."));
                return;
            }
            if !errors.is_empty() {
                let body: Vec<Vec<String>> = errors
                    .iter()
                    .map(|e| vec![format!("{}!{}", text(&e["sheet"]), text(&e["cell"])), text(&e["error"]), text(&e["formula"])])
                    .collect();
                table(ui, lang, "error_cells", &["Cell", "Error", "Formula"], &body);
            }
            if !circular.is_empty() {
                ui.add_space(8.0);
                ui.strong(lang.tr("Circular References"));
                for c in circular {
                    ui.label(text(c));
                }
            }
        }
        "evaluateFormula" => {
            if let Some(e) = r.get("error") {
                ui.label(lang.tr_error(&text(e)));
                return;
            }
            rows(ui, lang, "evaluate_formula", r, &[("Formula", "formula"), ("Result", "result")]);
            let steps: Vec<Vec<String>> = list(&r["steps"]).iter().map(|s| vec![text(&s["expression"]), text(&s["value"])]).collect();
            if !steps.is_empty() {
                ui.add_space(8.0);
                table(ui, lang, "evaluate_steps", &["Expression", "Value"], &steps);
            }
        }
        _ => {
            let comments = list(r);
            if comments.is_empty() {
                ui.label(lang.tr("This sheet has no comments."));
                return;
            }
            let body: Vec<Vec<String>> = comments.iter().map(|c| vec![text(&c["cell"]), text(&c["author"]), text(&c["text"])]).collect();
            table(ui, lang, "sheet_comments", &["Cell", "Author", "Text"], &body);
        }
    });
}
