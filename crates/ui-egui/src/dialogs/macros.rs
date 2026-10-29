use super::{Dialog, Json, SheetApp, json};

/// The Macros dialog: lists GridCraft's own recorded scripts and (if the file carries one) the
/// preserved VBA project. Mirrors Excel's Developer ▸ Macros list, but is honest that VBA is
/// preserved and not executed.
pub(super) fn macros(app: &mut SheetApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let lang = app.ui.language;
    ui.label(lang.tr("Automation in this workbook. GridCraft runs its own scripts; a file's VBA project is preserved but not executed."));
    ui.add_space(6.0);
    let items: Vec<Json> = d.result.as_ref().and_then(Json::as_array).cloned().unwrap_or_default();
    if items.is_empty() {
        ui.weak(lang.tr("This workbook has no scripts or macros."));
        return;
    }
    // Scripts that came with a file stay disabled until the user trusts the workbook (#2178).
    if items.iter().any(|it| it.get("trusted").and_then(Json::as_bool) == Some(false)) {
        ui.horizontal(|ui| {
            ui.weak(lang.tr("Scripts in this workbook came from a file and are disabled."));
            if ui.button(lang.tr("Trust Scripts")).on_hover_text(lang.tr("Let this workbook's scripts run for the rest of this session")).clicked() {
                let _ = app.run("automate.trustScripts", json!({"trusted": true}));
                d.result = app.session.run("view.macros", json!({})).ok();
            }
        });
        ui.add_space(6.0);
    }
    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
        for it in &items {
            let name = it.get("name").and_then(Json::as_str).unwrap_or_else(|| lang.tr("(unnamed)"));
            let is_vba = it.get("kind").and_then(Json::as_str) == Some("vba");
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(name).strong());
                if is_vba {
                    ui.weak(lang.tr("— VBA project (preserved, not run)"));
                } else {
                    let steps = it.get("steps").and_then(Json::as_u64).unwrap_or(0);
                    ui.weak(lang.tr(if steps == 1 { "— {steps} step" } else { "— {steps} steps" }).replace("{steps}", &steps.to_string()));
                }
            });
        }
    });
}
