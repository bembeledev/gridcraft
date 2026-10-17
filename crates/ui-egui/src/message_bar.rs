//! Message bars under the ribbon, as Excel shows them for content it holds back ("Security
//! Warning: Automatic update of links has been disabled [Enable Content]"). Each bar comes from
//! the active document's engine state ([`bars`]) and has an action command and a close button;
//! closing hides it for that document until it is opened again. A new kind of bar only adds a
//! source in [`bars`].

use egui::{RichText, Stroke, StrokeKind, Ui, vec2};
use serde_json::json;

use crate::SheetApp;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};

/// One message bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bar {
    /// Stable key: closing a bar remembers it per document.
    pub key: &'static str,
    /// The bold lead (`Security Warning`). Labels are translation keys and may carry a
    /// `context|` (see `Language::tr`).
    pub title: &'static str,
    pub text: &'static str,
    /// The action button: (label, command id).
    pub action: Option<(&'static str, &'static str)>,
}

/// The bars the active document asks for, closed ones included.
pub fn bars(app: &SheetApp) -> Vec<Bar> {
    let mut out = vec![];
    let Some(d) = app.session.active() else { return out };
    // Linked workbooks: opening disabled their automatic update. Without a file system they
    // can't be updated at all, so there is nothing to enable.
    if d.links.updates_disabled && app.session.link_files.available() {
        out.push(Bar {
            key: "links",
            title: "Workbook Links|Security Warning",
            text: "Workbook Links|Automatic update of links has been disabled",
            action: Some(("Workbook Links|Enable Content", "data.enableLinkUpdates")),
        });
    }
    out
}

fn closed_id(uid: u64, key: &str) -> egui::Id {
    egui::Id::new(("message_bar_closed", uid, key))
}

/// The bars showing now: [`bars`] without the ones closed for the active document.
pub fn shown(app: &SheetApp, ctx: &egui::Context) -> Vec<Bar> {
    let uid = app.session.active().map(|d| d.uid).unwrap_or_default();
    bars(app).into_iter().filter(|b| !ctx.data(|m| m.get_temp::<bool>(closed_id(uid, b.key)).unwrap_or(false))).collect()
}

pub fn show(app: &mut SheetApp, ui: &mut Ui) {
    let shown = shown(app, ui.ctx());
    if shown.is_empty() {
        return;
    }
    let uid = app.session.active().map(|d| d.uid).unwrap_or_default();
    let t = Tokens::get(ui.ctx());
    let lang = app.ui.language;
    let mut run: Option<&'static str> = None;
    let mut close: Option<&'static str> = None;
    for bar in &shown {
        egui::Panel::top(egui::Id::new(("message_bar", bar.key)))
            .frame(egui::Frame::NONE.fill(t.notice_warn_bg).inner_margin(egui::Margin::symmetric(12, 4)))
            .show(ui, |ui| {
                ui.painter().rect_stroke(ui.max_rect(), 0.0, Stroke::new(1.0, t.notice_warn_border), StrokeKind::Inside);
                ui.horizontal_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::hover());
                    icons::paint(ui.painter(), r, Icon::Lock, t.notice_warn_border);
                    ui.add_space(6.0);
                    ui.label(RichText::new(lang.tr(bar.title)).font(theme::ui_bold(12.5)).color(t.text));
                    ui.add_space(8.0);
                    ui.label(RichText::new(lang.tr(bar.text)).font(theme::ui_font(12.5)).color(t.text));
                    if let Some((label, id)) = bar.action {
                        ui.add_space(12.0);
                        if ui.button(RichText::new(lang.tr(label)).font(theme::ui_font(12.5))).clicked() {
                            run = Some(id);
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if crate::widgets::icon_button(ui, Icon::Close, t.text_dim, "Close", vec2(20.0, 20.0)).clicked() {
                            close = Some(bar.key);
                        }
                    });
                });
            });
    }
    if let Some(key) = close {
        ui.ctx().data_mut(|m| m.insert_temp(closed_id(uid, key), true));
    }
    if let Some(id) = run {
        app.run_or_alert(id, json!({}));
    }
}
