use super::{Json, SheetApp, json};

pub(super) fn manage_rules(app: &mut SheetApp, ui: &mut egui::Ui) {
    let lang = app.ui.language;
    let list = app.session.run("home.manageRules", json!({})).unwrap_or(Json::Null);
    let rules = list["rules"].as_array().cloned().unwrap_or_default();
    if rules.is_empty() {
        ui.label(lang.tr("There are no conditional formatting rules on this sheet."));
    }
    for (i, r) in rules.iter().enumerate() {
        ui.horizontal(|ui| {
            let kind = r["rule"].as_object().and_then(|o| o.keys().next().cloned()).unwrap_or_default();
            let label = match kind.as_str() {
                "CellIs" => "Cell Value",
                "Expression" => "Formula",
                "ContainsText" => "Contains Text",
                "NotContainsText" => "Does Not Contain Text",
                "BeginsWith" => "Begins With",
                "EndsWith" => "Ends With",
                "Blanks" => "Blanks",
                "NoBlanks" => "No Blanks",
                "Errors" => "Errors",
                "NoErrors" => "No Errors",
                "Duplicate" => "Duplicate Values",
                "Unique" => "Unique Values",
                "Top10" => "Top/Bottom Rules",
                "AboveAverage" => "Above/Below Average",
                "TimePeriod" => "A Date Occurring",
                "ColorScale" => "Color Scales",
                "DataBar" => "Data Bars",
                "IconSet" => "Icon Sets",
                _ => &kind,
            };
            let ranges = r["ranges"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(",")).unwrap_or_default();
            ui.label(
                lang.tr("{number}. {kind}  applies to {ranges}")
                    .replace("{number}", &(i + 1).to_string())
                    .replace("{kind}", lang.tr(label))
                    .replace("{ranges}", &ranges),
            );
            if ui.small_button("▲").clicked() {
                app.run_or_alert("home.manageRules", json!({"moveUp": i}));
            }
            if ui.small_button("▼").clicked() {
                app.run_or_alert("home.manageRules", json!({"moveDown": i}));
            }
            if ui.small_button(lang.tr("Delete")).clicked() {
                app.run_or_alert("home.manageRules", json!({"delete": i}));
            }
        });
    }
}
