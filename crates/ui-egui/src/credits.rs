//! The About window's credits: every contributor (GitHub login always; display and real names only
//! with recorded consent) and the AI models that helped. The tables are baked into the binary by
//! build.rs through craft-credits from contributors/contributors.json (craftrules
//! standards/contributors.md); nothing is read at run time.

use craft_credits::fmt::{day, group, signed};
use craft_credits::{Contributor, NameMode, SortKey, sorted};
use egui::RichText;

include!(concat!(env!("OUT_DIR"), "/credits.rs"));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct View {
    names: NameMode,
    key: SortKey,
    ascending: bool,
    table: bool,
}

impl Default for View {
    fn default() -> Self {
        View { names: NameMode::Username, key: SortKey::FirstCommit, ascending: true, table: false }
    }
}

/// About ▸ Contributors: a name toggle, a sort, and the list as a grab bag or a table.
pub fn contributors_ui(ui: &mut egui::Ui) {
    let id = egui::Id::new("credits_view");
    let mut v = ui.data_mut(|d| d.get_temp::<View>(id)).unwrap_or_default();
    ui.horizontal_wrapped(|ui| {
        ui.label("Show");
        for m in NameMode::ALL {
            if ui.selectable_label(v.names == m, m.label()).clicked() {
                v.names = m;
            }
        }
        ui.separator();
        ui.label("Sort");
        egui::ComboBox::from_id_salt("credits_sort").selected_text(v.key.label().0).show_ui(ui, |ui| {
            for k in SortKey::ALL {
                if ui.selectable_label(v.key == k, k.label().0).clicked() {
                    v.key = k;
                    v.ascending = k.default_ascending();
                }
            }
        });
        if ui.button(if v.ascending { "▲" } else { "▼" }).on_hover_text("Reverse the order").clicked() {
            v.ascending = !v.ascending;
        }
        ui.separator();
        if ui.selectable_label(!v.table, "Grab bag").clicked() {
            v.table = false;
        }
        if ui.selectable_label(v.table, "Table").clicked() {
            v.table = true;
        }
    });
    let list = sorted(CONTRIBUTORS, v.names, v.key, v.ascending);
    ui.label(RichText::new(format!("{} contributors · {} commits", list.len(), group(TOTAL_COMMITS))).small().weak());
    ui.separator();
    egui::ScrollArea::both().auto_shrink([false, false]).max_height(340.0).show(ui, |ui| {
        if list.is_empty() {
            ui.label("No contributor data was built into this copy.");
        } else if v.table {
            table(ui, &list, &mut v);
        } else {
            ui.horizontal_wrapped(|ui| {
                for (i, c) in list.iter().enumerate() {
                    if i > 0 {
                        ui.label(RichText::new("·").weak());
                    }
                    ui.hyperlink_to(c.name(v.names), format!("https://github.com/{}", c.login)).on_hover_text(c.summary());
                }
            });
        }
    });
    ui.data_mut(|d| d.insert_temp(id, v));
}

fn table(ui: &mut egui::Ui, list: &[&Contributor], v: &mut View) {
    egui::Grid::new("credits_table").striped(true).num_columns(SortKey::ALL.len()).show(ui, |ui| {
        for k in SortKey::ALL {
            let arrow = if v.key == k { if v.ascending { " ▲" } else { " ▼" } } else { "" };
            if ui.button(RichText::new(format!("{}{arrow}", k.label().1)).strong()).on_hover_text(k.label().0).clicked() {
                if v.key == k {
                    v.ascending = !v.ascending;
                } else {
                    v.key = k;
                    v.ascending = k.default_ascending();
                }
            }
        }
        ui.end_row();
        for c in list {
            ui.hyperlink_to(c.name(v.names), format!("https://github.com/{}", c.login)).on_hover_text(c.summary());
            ui.label(group(c.prs));
            ui.label(group(c.commits));
            ui.label(group(c.lines_added));
            ui.label(group(c.lines_deleted));
            ui.label(signed(c.lines_delta()));
            ui.label(group(c.binary_added));
            ui.label(group(c.binary_deleted));
            ui.label(day(c.first_commit));
            ui.label(day(c.last_commit));
            ui.end_row();
        }
    });
}

/// About ▸ Models: AI models credited in Co-Authored-By trailers.
pub fn models_ui(ui: &mut egui::Ui) {
    if MODELS.is_empty() {
        ui.label("No model credits were built into this copy.");
        return;
    }
    egui::ScrollArea::both().auto_shrink([false, false]).max_height(340.0).show(ui, |ui| {
        egui::Grid::new("credits_models").striped(true).num_columns(6).show(ui, |ui| {
            for h in ["Company", "Model", "Version", "Commits", "% of all commits", "Lines +/−"] {
                ui.label(RichText::new(h).strong());
            }
            ui.end_row();
            for m in MODELS {
                ui.label(m.company);
                ui.label(m.model);
                ui.label(m.version);
                ui.label(group(m.commits));
                ui.label(format!("{:.0}%", craft_credits::model_share(m, MODELS, TOTAL_COMMITS)));
                ui.label(format!("+{} / −{}", group(m.lines_added), group(m.lines_deleted)));
                ui.end_row();
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The baked tables are craft-credits' types, so the About window gets the shared sorting.
    #[test]
    fn baked_tables_sort() {
        let v = sorted(CONTRIBUTORS, NameMode::Username, SortKey::Name, true);
        assert_eq!(v.len(), CONTRIBUTORS.len());
        assert!(v.windows(2).all(|w| w[0].login.to_lowercase() <= w[1].login.to_lowercase()));
    }
}
