//! Original border diagrams, painted with the same strokes as worksheet cells.

use egui::{Color32, Rect, Response, Stroke, Ui, pos2, vec2};
use gridcraft_engine::model::BorderStyle;

use crate::grid::paint_border;
use crate::theme::{self, Tokens};

/// A labelled menu button with a two-by-two diagram to distinguish inside edges
/// from the selection outline. Labels stay available to accessibility tools.
pub(crate) fn preset_button(ui: &mut Ui, label: &str, preset: &str) -> Response {
    // `label` may mark its access key with `&` ("&Bottom Border"); it is drawn underlined, and
    // translated by its plain text (`tr_access`).
    let label = &tr_access(crate::i18n::current(ui.ctx()), label);
    let plain = label.replace('&', "");
    let response = ui.add(egui::Button::new("").frame(false).min_size(vec2(300.0, 30.0)));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &plain));
    let t = Tokens::get(ui.ctx());
    let rect = Rect::from_center_size(pos2(response.rect.left() + 18.0, response.rect.center().y), vec2(16.0, 16.0));
    let edges = [
        [rect.left_top(), rect.right_top()],
        [rect.left_bottom(), rect.right_bottom()],
        [rect.left_top(), rect.left_bottom()],
        [rect.right_top(), rect.right_bottom()],
        [rect.left_center(), rect.right_center()],
        [rect.center_top(), rect.center_bottom()],
    ];
    use BorderStyle::{Double, None, Thick, Thin};
    let styles = match preset {
        "bottom" => [None, Thin, None, None, None, None],
        "top" => [Thin, None, None, None, None, None],
        "left" => [None, None, Thin, None, None, None],
        "right" => [None, None, None, Thin, None, None],
        "all" => [Thin; 6],
        "outside" => [Thin, Thin, Thin, Thin, None, None],
        "thickOutside" => [Thick, Thick, Thick, Thick, None, None],
        "doubleBottom" => [None, Double, None, None, None, None],
        "thickBottom" => [None, Thick, None, None, None, None],
        "topBottom" => [Thin, Thin, None, None, None, None],
        "topThickBottom" => [Thin, Thick, None, None, None, None],
        "topDoubleBottom" => [Thin, Double, None, None, None, None],
        "inside" => [None, None, None, None, Thin, Thin],
        _ => [None; 6],
    };
    for (points, style) in edges.into_iter().zip(styles) {
        if style == None {
            ui.painter().extend(egui::Shape::dotted_line(&points, t.text_disabled, 3.0, 0.45));
        } else {
            paint_border(ui.painter(), points, style, t.text);
        }
    }
    let galley = ui.painter().layout_job(access_key_job(label, theme::ui_font(13.0), t.text));
    let at = pos2(response.rect.left() + 38.0, response.rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(at, galley, t.text);
    response
}

/// Translates a label that marks its access key with `&` ("Entire &row"). The catalogs hold
/// the plain text ("Entire row"); a translation is shown without an access key, and a label with
/// no translation keeps its English access key.
pub(crate) fn tr_access(lang: crate::i18n::Language, label: &str) -> String {
    let plain = label.replace('&', "");
    let shown = lang.tr(&plain);
    if shown == plain { label.to_string() } else { shown.to_string() }
}

/// Lays out `label` with the character after `&` underlined (Windows menu access key).
pub(crate) fn access_key_job(label: &str, font: egui::FontId, color: Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    // "&&" is a literal ampersand ("Merge && &Center").
    let chars: Vec<char> = label.chars().collect();
    let mut i = 0;
    let mut underline_next = false;
    while let Some(&c) = chars.get(i) {
        if c == '&' && chars.get(i + 1) == Some(&'&') {
            i += 1;
        } else if c == '&' {
            underline_next = true;
            i += 1;
            continue;
        }
        let underline = if underline_next { Stroke::new(1.0, color) } else { Stroke::NONE };
        underline_next = false;
        job.append(&c.to_string(), 0.0, egui::TextFormat { font_id: font.clone(), color, underline, ..Default::default() });
        i += 1;
    }
    job
}
