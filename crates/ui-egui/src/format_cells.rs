//! Format Cells, laid out like the classic desktop dialog: a light title row, a
//! tab strip (Number, Alignment, Font, Border, Fill, Protection), a light-grey page with
//! group boxes, list boxes and drop-downs, and OK / Cancel at the bottom right.
//!
//! The dialog edits a draft `Style` (in `Dialog::values["style"]`); OK applies it with
//! `home.formatCells` (one undo step), plus a merge or unmerge when "Merge cells" changed.
//! Wording, artwork and layout code are this project's own.

use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use gridcraft_engine::core::Value;
use gridcraft_engine::model::{BorderLine, BorderStyle, Color, Fill, HAlign, PatternType, Style, Underline, VAlign, VertAlign};
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::dialogs::Dialog;
use crate::theme::{self, Tokens};

const PAGE_W: f32 = 540.0;
const PAGE_H: f32 = 420.0;
const SELECT_BG: Color32 = Color32::from_rgb(0, 120, 215);

// ---- small widgets ---------------------------------------------------------------------------

fn page_bg(t: &Tokens) -> Color32 {
    if t.dark { Color32::from_gray(40) } else { Color32::from_rgb(249, 249, 249) }
}

fn line_color(t: &Tokens) -> Color32 {
    if t.dark { Color32::from_gray(80) } else { Color32::from_gray(214) }
}

/// A label with its access key underlined (`"&Category:"`).
fn key_label(ui: &mut Ui, label: &str) {
    let t = Tokens::get(ui.ctx());
    let lang = crate::i18n::current(ui.ctx());
    let plain = label.replace('&', "");
    let shown = lang.tr(&plain);
    let job = if shown == plain {
        crate::border_preview::access_key_job(label, theme::ui_font(13.0), t.text)
    } else {
        egui::text::LayoutJob::simple_singleline(shown.to_string(), theme::ui_font(13.0), t.text)
    };
    ui.label(job);
}

/// A section title followed by a thin rule to the right edge ("Text alignment ────").
fn section(ui: &mut Ui, title: &str, width: f32) {
    let t = Tokens::get(ui.ctx());
    let lang = crate::i18n::current(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(width, 22.0), Sense::hover());
    let g = ui.painter().layout_no_wrap(lang.tr(title).to_string(), theme::ui_font(13.0), t.text);
    let tw = g.size().x;
    ui.painter().galley(pos2(r.left(), r.center().y - g.size().y / 2.0), g, t.text);
    ui.painter().hline(r.left() + tw + 8.0..=r.right(), r.center().y, Stroke::new(1.0, line_color(&t)));
}

/// A framed group with its title set into the top edge ("Effects", "Preview", "Sample").
fn group_box<R>(ui: &mut Ui, title: &str, size: egui::Vec2, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Tokens::get(ui.ctx());
    let lang = crate::i18n::current(ui.ctx());
    let (outer, _) = ui.allocate_exact_size(size, Sense::hover());
    let frame = Rect::from_min_max(outer.min + vec2(0.0, 8.0), outer.max);
    ui.painter().rect_stroke(frame, CornerRadius::same(2), Stroke::new(1.0, line_color(&t)), StrokeKind::Inside);
    let g = ui.painter().layout_no_wrap(lang.tr(title).to_string(), theme::ui_font(13.0), t.text);
    let label = Rect::from_min_size(pos2(frame.left() + 8.0, outer.top()), vec2(g.size().x + 6.0, 16.0));
    ui.painter().rect_filled(label, CornerRadius::ZERO, page_bg(&t));
    ui.painter().galley(pos2(label.left() + 3.0, outer.top()), g, t.text);
    let inner = frame.shrink2(vec2(10.0, 10.0)).translate(vec2(0.0, 4.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)), add).inner
}

/// A bordered white list box; returns the index clicked. Each row is a labelled button for
/// screen readers and tests.
fn list_box(ui: &mut Ui, id: &str, items: &[String], selected: Option<usize>, size: egui::Vec2) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    let mut clicked = None;
    let bg = if t.dark { Color32::from_gray(30) } else { Color32::WHITE };
    egui::Frame::new().fill(bg).stroke(Stroke::new(1.0, Color32::from_gray(150))).inner_margin(egui::Margin::same(2)).show(ui, |ui| {
        ui.set_width(size.x - 6.0);
        ui.set_height(size.y - 6.0);
        egui::ScrollArea::vertical().id_salt(id).max_height(size.y - 6.0).auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, item) in items.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::click());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::SelectableLabel, true, item));
                let sel = selected == Some(i);
                if sel {
                    ui.painter().rect_filled(r, CornerRadius::ZERO, SELECT_BG);
                    if resp.gained_focus() || resp.clicked() {
                        resp.scroll_to_me(None);
                    }
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, CornerRadius::ZERO, t.hover);
                }
                let color = if sel { Color32::WHITE } else { t.text };
                ui.painter().text(pos2(r.left() + 4.0, r.center().y), Align2::LEFT_CENTER, item, theme::ui_font(13.0), color);
                if resp.clicked() {
                    clicked = Some(i);
                }
            }
        });
    });
    clicked
}

/// A number box with up/down arrows; returns true when the value changed.
fn spin(ui: &mut Ui, id: &str, v: &mut i32, lo: i32, hi: i32, width: f32) -> bool {
    let before = *v;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_sized([width, 26.0], egui::DragValue::new(v).range(lo..=hi).speed(0.2));
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let small = |ui: &mut Ui, text: &str, tip: &str| {
                ui.add_sized([18.0, 13.0], egui::Button::new(egui::RichText::new(text).size(8.0)).corner_radius(0)).on_hover_text(tip)
            };
            if small(ui, "▲", "Up").clicked() {
                *v = (*v + 1).min(hi);
            }
            if small(ui, "▼", "Down").clicked() {
                *v = (*v - 1).max(lo);
            }
        });
        let _ = id;
    });
    *v != before
}

/// A drop-down; returns the chosen index.
fn combo(ui: &mut Ui, id: &str, items: &[String], current: usize, width: f32) -> Option<usize> {
    let mut picked = None;
    let shown = items.get(current).cloned().unwrap_or_default();
    egui::ComboBox::from_id_salt(id).selected_text(shown).width(width).height(300.0).show_ui(ui, |ui| {
        for (i, it) in items.iter().enumerate() {
            if ui.selectable_label(i == current, it).clicked() {
                picked = Some(i);
            }
        }
    });
    picked
}

/// A colour drop-down showing a swatch; returns the chosen colour (`None` inside = Automatic).
fn color_combo(ui: &mut Ui, id: &str, current: Option<Color32>, theme_colors: &[u32; 12], auto_label: &str, width: f32) -> Option<Option<String>> {
    let t = Tokens::get(ui.ctx());
    let lang = crate::i18n::current(ui.ctx());
    let resp = ui.add_sized([width, 26.0], egui::Button::new("").corner_radius(2));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, id));
    let sw = resp.rect.shrink2(vec2(6.0, 6.0));
    let sw = Rect::from_min_max(sw.min, pos2(sw.right() - 20.0, sw.bottom()));
    match current {
        Some(c) => {
            ui.painter().rect_filled(sw, CornerRadius::ZERO, c);
        }
        None => {
            ui.painter().text(sw.center(), Align2::CENTER_CENTER, lang.tr(auto_label), theme::ui_font(13.0), t.text);
        }
    }
    crate::icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(resp.rect.right() - 12.0, resp.rect.center().y), vec2(10.0, 10.0)),
        crate::icons::Icon::Chevron,
        t.text,
    );
    let mut out = crate::color_picker::take_result(ui.ctx(), id).map(Some);
    egui::Popup::menu(&resp).show(|ui| {
        if let Some(c) = crate::widgets::color_palette(ui, theme_colors, auto_label, id) {
            out = Some(if c == "none" { None } else { Some(c) });
            ui.close();
        }
    });
    out
}

fn resolve(c: &Color, theme: &gridcraft_engine::model::Theme) -> Option<Color32> {
    c.resolve(theme).map(theme::color32)
}

// ---- number formats ----------------------------------------------------------------------------

const CATEGORIES: [&str; 12] =
    ["General", "Number", "Currency", "Accounting", "Date", "Time", "Percentage", "Fraction", "Scientific", "Text", "Special", "Custom"];

const DATE_CODES: [&str; 12] = [
    "m/d/yyyy",
    "dddd, mmmm d, yyyy",
    "m/d",
    "m/d/yy",
    "mm/dd/yy",
    "d-mmm",
    "d-mmm-yy",
    "dd-mmm-yy",
    "mmm-yy",
    "mmmm-yy",
    "mmmm d, yyyy",
    "yyyy-mm-dd",
];
const TIME_CODES: [&str; 8] = ["h:mm", "h:mm AM/PM", "h:mm:ss", "h:mm:ss AM/PM", "mm:ss", "mm:ss.0", "[h]:mm:ss", "m/d/yy h:mm"];
const FRACTION_CODES: [(&str, &str); 9] = [
    ("Up to one digit (1/4)", "# ?/?"),
    ("Up to two digits (21/25)", "# ??/??"),
    ("Up to three digits (312/943)", "# ???/???"),
    ("As halves (1/2)", "# ?/2"),
    ("As quarters (2/4)", "# ?/4"),
    ("As eighths (4/8)", "# ?/8"),
    ("As sixteenths (8/16)", "# ??/16"),
    ("As tenths (3/10)", "# ?/10"),
    ("As hundredths (30/100)", "# ??/100"),
];
const SPECIAL_CODES: [(&str, &str); 4] = [
    ("Zip Code", "00000"),
    ("Zip Code + 4", "00000-0000"),
    ("Phone Number", "[<=9999999]###-####;(###) ###-####"),
    ("Social Security Number", "000-00-0000"),
];
const CUSTOM_CODES: [&str; 24] = [
    "General",
    "0",
    "0.00",
    "#,##0",
    "#,##0.00",
    "#,##0_);(#,##0)",
    "#,##0_);[Red](#,##0)",
    "#,##0.00_);(#,##0.00)",
    "#,##0.00_);[Red](#,##0.00)",
    "$#,##0_);($#,##0)",
    "$#,##0.00_);[Red]($#,##0.00)",
    "0%",
    "0.00%",
    "0.00E+00",
    "##0.0E+0",
    "# ?/?",
    "# ??/??",
    "m/d/yyyy",
    "d-mmm-yy",
    "mmm-yy",
    "h:mm AM/PM",
    "h:mm:ss",
    "[h]:mm:ss",
    "@",
];
const SYMBOLS: [(&str, &str); 5] = [("None", ""), ("$", "$"), ("€", "€"), ("£", "£"), ("¥", "¥")];

/// `code` without its bracketed parts (`[Red]`, `[$-409]`, `[h]`); quoted text stays.
fn unbracketed(code: &str) -> String {
    let mut out = String::new();
    let (mut q, mut b) = (false, false);
    for c in code.chars() {
        match c {
            '"' if !b => {
                q = !q;
                out.push(c);
            }
            '[' if !q => b = true,
            ']' if !q && b => b = false,
            _ if !b => out.push(c),
            _ => {}
        }
    }
    out
}

/// A bracketed currency tag with a symbol: `[$€-407]`, `[$USD]`. `[$-409]` is a locale alone.
fn currency_tag(code: &str) -> bool {
    code.match_indices("[$").any(|(i, _)| code[i + 2..].chars().next().is_some_and(|c| c != '-' && c != ']'))
}

fn category_of(code: &str) -> &'static str {
    let lower = code.to_ascii_lowercase();
    // The code without quoted text and bracketed parts: `[Red]` holds a `d` and `[$-409]` a `$`,
    // neither of which says Date or Currency.
    let unquoted: String = {
        let mut out = String::new();
        let (mut q, mut b) = (false, false);
        for c in lower.chars() {
            match c {
                '"' if !b => q = !q,
                '[' if !q => b = true,
                ']' if !q && b => b = false,
                _ if !q && !b => out.push(c),
                _ => {}
            }
        }
        out
    };
    if lower == "general" || lower.is_empty() {
        "General"
    } else if lower == "@" {
        "Text"
    } else if SPECIAL_CODES.iter().any(|(_, c)| *c == code) {
        "Special"
    } else if unquoted.contains("e+") || unquoted.contains("e-") {
        "Scientific"
    } else if unquoted.contains('%') {
        "Percentage"
    } else if unquoted.contains("?/") {
        "Fraction"
    } else if code.starts_with("_(") || code.contains("* ") {
        "Accounting"
    } else if SYMBOLS.iter().skip(1).any(|(_, s)| unbracketed(code).contains(s)) || currency_tag(code) {
        "Currency"
    } else if unquoted.contains('h') || unquoted.contains('s') {
        "Time"
    } else if unquoted.contains('d') || unquoted.contains('y') || (unquoted.contains('m') && !unquoted.contains('0')) {
        "Date"
    } else if unquoted.chars().all(|c| "0#,.;_()- ".contains(c)) {
        "Number"
    } else {
        "Custom"
    }
}

fn decimals_of(code: &str) -> i32 {
    code.split(';').next().and_then(|first| first.split_once('.')).map_or(0, |(_, rest)| rest.chars().take_while(|c| *c == '0').count() as i32)
}

fn number_code(decimals: i32, sep: bool, neg: usize, symbol: &str) -> String {
    let base = format!(
        "{symbol}{}{}",
        if sep { "#,##0" } else { "0" },
        if decimals > 0 { format!(".{}", "0".repeat(decimals as usize)) } else { String::new() }
    );
    match neg {
        1 => format!("{base};[Red]{base}"),
        2 => format!("{base}_);({base})"),
        3 => format!("{base}_);[Red]({base})"),
        _ => base,
    }
}

fn accounting_code(decimals: i32, symbol: &str) -> String {
    let d = if decimals > 0 { format!(".{}", "0".repeat(decimals as usize)) } else { String::new() };
    let q = if decimals > 0 { "?".repeat(decimals as usize) } else { String::new() };
    let s = if symbol.is_empty() { String::new() } else { format!("\"{symbol}\"") };
    format!("_({s}* #,##0{d}_);_({s}* (#,##0{d});_({s}* \"-\"{q}_);_(@_)")
}

fn sample_text(app: &SheetApp, value: &Value, code: &str) -> String {
    app.session.active().map(|doc| gridcraft_engine::display::format(value, code, &doc.wb).text).unwrap_or_default()
}

fn number_tab(app: &SheetApp, ui: &mut Ui, d: &mut Dialog, st: &mut Style) {
    let lang = crate::i18n::current(ui.ctx());
    let t = Tokens::get(ui.ctx());
    let mut code = st.num_fmt.as_str().to_string();
    let category = d.values.get("category").and_then(Json::as_str).map(str::to_string).unwrap_or_else(|| category_of(&code).to_string());
    let value = app.session.active().and_then(|doc| doc.wb.active().map(|s| s.value(doc.selection.active))).unwrap_or_default();
    let sample_value = if matches!(value, Value::Number(_)) { value.clone() } else { Value::Number(40982.5604) };
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            key_label(ui, "&Category:");
            let names: Vec<String> = CATEGORIES.iter().map(|c| lang.tr(c).to_string()).collect();
            let sel = CATEGORIES.iter().position(|c| *c == category);
            if let Some(i) = list_box(ui, "fc_category", &names, sel, vec2(150.0, 300.0)) {
                let cat = CATEGORIES[i];
                d.values.insert("category".into(), json!(cat));
                code = match cat {
                    "General" => "General".into(),
                    "Number" => number_code(2, false, 0, ""),
                    "Currency" => number_code(2, true, 0, "$"),
                    "Accounting" => accounting_code(2, "$"),
                    "Date" => DATE_CODES[0].into(),
                    "Time" => TIME_CODES[3].into(),
                    "Percentage" => "0.00%".into(),
                    "Fraction" => FRACTION_CODES[0].1.into(),
                    "Scientific" => "0.00E+00".into(),
                    "Text" => "@".into(),
                    "Special" => SPECIAL_CODES[0].1.into(),
                    _ => code.clone(),
                };
            }
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.set_width(PAGE_W - 190.0);
            let category = d.values.get("category").and_then(Json::as_str).map(str::to_string).unwrap_or_else(|| category_of(&code).to_string());
            let sample = if value.is_empty() && category != "Text" { String::new() } else { sample_text(app, &value, &code) };
            group_box(ui, "Sample", vec2(PAGE_W - 190.0, 52.0), |ui| {
                ui.label(egui::RichText::new(sample).font(theme::ui_font(13.0)));
            });
            ui.add_space(6.0);
            let decimals_row = |ui: &mut Ui, code: &mut String, make: &dyn Fn(i32) -> String| {
                ui.horizontal(|ui| {
                    key_label(ui, "&Decimal places:");
                    let mut n = decimals_of(code);
                    if spin(ui, "fc_dec", &mut n, 0, 30, 50.0) {
                        *code = make(n);
                    }
                });
            };
            let describe = |ui: &mut Ui, text: &str| {
                ui.add_space(4.0);
                ui.add(egui::Label::new(egui::RichText::new(lang.tr(text)).font(theme::ui_font(13.0))).wrap());
            };
            match category.as_str() {
                "General" => describe(ui, "General format cells have no specific number format."),
                "Number" => {
                    let sep = code.contains(',');
                    let neg = if code.contains("[Red](") {
                        3
                    } else if code.contains("_);(") {
                        2
                    } else if code.contains(";[Red]") {
                        1
                    } else {
                        0
                    };
                    let dec = decimals_of(&code);
                    decimals_row(ui, &mut code, &|n| number_code(n, sep, neg, ""));
                    let mut s = sep;
                    if ui.checkbox(&mut s, lang.tr("Use 1000 Separator (,)")).changed() {
                        code = number_code(dec, s, neg, "");
                    }
                    key_label(ui, "&Negative numbers:");
                    let items: Vec<String> = (0..4).map(|k| sample_text(app, &Value::Number(-1234.1), &number_code(dec, sep, k, ""))).collect();
                    if let Some(k) = list_box(ui, "fc_neg", &items, Some(neg), vec2(220.0, 100.0)) {
                        code = number_code(dec, sep, k, "");
                    }
                    describe(
                        ui,
                        "Number is used for general display of numbers. Currency and Accounting offer specialized formatting for monetary value.",
                    );
                }
                "Currency" | "Accounting" => {
                    let symbol = SYMBOLS.iter().skip(1).find(|(_, s)| code.contains(s)).map_or("", |(_, s)| *s).to_string();
                    let dec = decimals_of(&code);
                    let acct = category == "Accounting";
                    let neg = if code.contains("[Red](") {
                        3
                    } else if code.contains("_);(") {
                        2
                    } else if code.contains(";[Red]") {
                        1
                    } else {
                        0
                    };
                    let sym = symbol.clone();
                    decimals_row(ui, &mut code, &|n| if acct { accounting_code(n, &sym) } else { number_code(n, true, neg, &sym) });
                    ui.horizontal(|ui| {
                        key_label(ui, "&Symbol:");
                        let names: Vec<String> = SYMBOLS.iter().map(|(n, _)| lang.tr(n).to_string()).collect();
                        let cur = SYMBOLS.iter().position(|(_, s)| *s == symbol).unwrap_or(0);
                        if let Some(i) = combo(ui, "fc_symbol", &names, cur, 150.0) {
                            let s = SYMBOLS[i].1;
                            code = if acct { accounting_code(dec, s) } else { number_code(dec, true, neg, s) };
                        }
                    });
                    if !acct {
                        key_label(ui, "&Negative numbers:");
                        let items: Vec<String> =
                            (0..4).map(|k| sample_text(app, &Value::Number(-1234.1), &number_code(dec, true, k, &symbol))).collect();
                        if let Some(k) = list_box(ui, "fc_neg", &items, Some(neg), vec2(220.0, 100.0)) {
                            code = number_code(dec, true, k, &symbol);
                        }
                        describe(
                            ui,
                            "Currency formats are used for general monetary values. Use Accounting formats to align decimal points in a column.",
                        );
                    } else {
                        describe(ui, "Accounting formats line up the currency symbols and decimal points in a column.");
                    }
                }
                "Date" | "Time" => {
                    key_label(ui, "&Type:");
                    let codes: &[&str] = if category == "Date" { &DATE_CODES } else { &TIME_CODES };
                    let items: Vec<String> = codes.iter().map(|c| sample_text(app, &sample_value, c)).collect();
                    let sel = codes.iter().position(|c| *c == code);
                    if let Some(i) = list_box(ui, "fc_dt", &items, sel, vec2(PAGE_W - 200.0, 180.0)) {
                        code = codes[i].to_string();
                    }
                    describe(
                        ui,
                        if category == "Date" {
                            "Date formats display date and time serial numbers as date values."
                        } else {
                            "Time formats display date and time serial numbers as time values."
                        },
                    );
                }
                "Percentage" => {
                    decimals_row(ui, &mut code, &|n| format!("{}%", number_code(n, false, 0, "")));
                    describe(ui, "Percentage formats multiply the cell value by 100 and display the result with a percent symbol.");
                }
                "Scientific" => {
                    decimals_row(ui, &mut code, &|n| format!("{}E+00", number_code(n, false, 0, "")));
                }
                "Fraction" => {
                    key_label(ui, "&Type:");
                    let items: Vec<String> = FRACTION_CODES.iter().map(|(n, _)| lang.tr(n).to_string()).collect();
                    let sel = FRACTION_CODES.iter().position(|(_, c)| *c == code);
                    if let Some(i) = list_box(ui, "fc_frac", &items, sel, vec2(PAGE_W - 200.0, 200.0)) {
                        code = FRACTION_CODES[i].1.to_string();
                    }
                }
                "Text" => {
                    describe(ui, "Text format cells are treated as text even when a number is in the cell. The cell is displayed exactly as entered.")
                }
                "Special" => {
                    key_label(ui, "&Type:");
                    let items: Vec<String> = SPECIAL_CODES.iter().map(|(n, _)| lang.tr(n).to_string()).collect();
                    let sel = SPECIAL_CODES.iter().position(|(_, c)| *c == code);
                    if let Some(i) = list_box(ui, "fc_special", &items, sel, vec2(PAGE_W - 200.0, 120.0)) {
                        code = SPECIAL_CODES[i].1.to_string();
                    }
                    describe(ui, "Special formats are useful for tracking list and database values.");
                }
                _ => {
                    key_label(ui, "&Type:");
                    ui.add_sized([PAGE_W - 200.0, 24.0], egui::TextEdit::singleline(&mut code));
                    let items: Vec<String> = CUSTOM_CODES.iter().map(|c| c.to_string()).collect();
                    let sel = CUSTOM_CODES.iter().position(|c| *c == code);
                    if let Some(i) = list_box(ui, "fc_custom", &items, sel, vec2(PAGE_W - 200.0, 200.0)) {
                        code = CUSTOM_CODES[i].to_string();
                    }
                    describe(ui, "Type the number format code, using one of the existing codes as a starting point.");
                }
            }
            let _ = t;
        });
    });
    st.num_fmt = gridcraft_engine::model::NumFmt::new(&code);
}

// ---- alignment -----------------------------------------------------------------------------------

fn orientation(ui: &mut Ui, st: &mut Style) {
    let t = Tokens::get(ui.ctx());
    let ink = t.text;
    let paper = if t.dark { Color32::from_gray(30) } else { Color32::WHITE };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        // Stacked text.
        let (vr, vresp) = ui.allocate_exact_size(vec2(36.0, 170.0), Sense::click());
        vresp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Vertical text"));
        let stacked = st.align.rotation == 255;
        ui.painter().rect(
            vr,
            CornerRadius::ZERO,
            if stacked { t.accent_soft } else { paper },
            Stroke::new(1.0, Color32::from_gray(150)),
            StrokeKind::Inside,
        );
        for (i, c) in "Text".chars().enumerate() {
            ui.painter().text(pos2(vr.center().x, vr.center().y - 30.0 + i as f32 * 20.0), Align2::CENTER_CENTER, c, theme::ui_font(13.0), ink);
        }
        if vresp.clicked() {
            st.align.rotation = if stacked { 0 } else { 255 };
        }
        // The dial: drag or click to set the angle.
        let (dr, dresp) = ui.allocate_exact_size(vec2(100.0, 170.0), Sense::click_and_drag());
        dresp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, "Orientation"));
        ui.painter().rect(dr, CornerRadius::ZERO, paper, Stroke::new(1.0, Color32::from_gray(150)), StrokeKind::Inside);
        let center = pos2(dr.left() + 14.0, dr.center().y);
        let radius = 72.0;
        for k in -6..=6 {
            let a = (k as f32 * 15.0).to_radians();
            let p = center + vec2(a.cos(), -a.sin()) * radius;
            if k % 3 == 0 {
                let s = 3.5;
                ui.painter().add(egui::Shape::convex_polygon(
                    vec![p + vec2(0.0, -s), p + vec2(s, 0.0), p + vec2(0.0, s), p + vec2(-s, 0.0)],
                    ink,
                    Stroke::NONE,
                ));
            } else {
                ui.painter().circle_filled(p, 1.2, ink);
            }
        }
        if let Some(p) = dresp.interact_pointer_pos()
            && (dresp.clicked() || dresp.dragged())
        {
            let v = p - center;
            let deg = (-v.y).atan2(v.x.max(0.01)).to_degrees().round().clamp(-90.0, 90.0);
            st.align.rotation = deg as i16;
        }
        let deg = if stacked { 0.0 } else { f32::from(st.align.rotation) };
        let a = deg.to_radians();
        let dir = vec2(a.cos(), -a.sin());
        let tip = center + dir * radius;
        let label_end = center + dir * 30.0;
        ui.painter().line_segment([label_end, tip], Stroke::new(1.0, ink));
        ui.painter().add(egui::Shape::convex_polygon(
            vec![tip + vec2(0.0, -4.0), tip + vec2(4.0, 0.0), tip + vec2(0.0, 4.0), tip + vec2(-4.0, 0.0)],
            Color32::from_rgb(200, 40, 40),
            Stroke::NONE,
        ));
        let g = ui.painter().layout_no_wrap("Text".into(), theme::ui_font(13.0), ink);
        ui.painter().add(egui::epaint::TextShape::new(center + vec2(2.0, -g.size().y / 2.0), g, ink).with_angle(-a));
    });
    ui.horizontal(|ui| {
        let mut deg = if st.align.rotation == 255 { 0 } else { i32::from(st.align.rotation) };
        if spin(ui, "fc_deg", &mut deg, -90, 90, 60.0) {
            st.align.rotation = deg as i16;
        }
        key_label(ui, "&Degrees");
    });
}

fn alignment_tab(app: &SheetApp, ui: &mut Ui, d: &mut Dialog, st: &mut Style) {
    let lang = crate::i18n::current(ui.ctx());
    let hs: [(HAlign, &str); 8] = [
        (HAlign::General, "General"),
        (HAlign::Left, "Left (Indent)"),
        (HAlign::Center, "Center"),
        (HAlign::Right, "Right (Indent)"),
        (HAlign::Fill, "Fill"),
        (HAlign::Justify, "Justify"),
        (HAlign::CenterAcross, "Center Across Selection"),
        (HAlign::Distributed, "Distributed (Indent)"),
    ];
    let vs: [(VAlign, &str); 5] = [
        (VAlign::Top, "Top"),
        (VAlign::Center, "Center"),
        (VAlign::Bottom, "Bottom"),
        (VAlign::Justify, "Justify"),
        (VAlign::Distributed, "Distributed"),
    ];
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(PAGE_W - 200.0);
            section(ui, "Text alignment", PAGE_W - 210.0);
            ui.horizontal_top(|ui| {
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    key_label(ui, "&Horizontal:");
                    let names: Vec<String> = hs.iter().map(|(_, n)| lang.tr(n).to_string()).collect();
                    let cur = hs.iter().position(|(h, _)| *h == st.align.h).unwrap_or(0);
                    if let Some(i) = combo(ui, "fc_h", &names, cur, 190.0) {
                        st.align.h = hs[i].0;
                    }
                    key_label(ui, "&Vertical:");
                    let names: Vec<String> = vs.iter().map(|(_, n)| lang.tr(n).to_string()).collect();
                    let cur = vs.iter().position(|(v, _)| *v == st.align.v).unwrap_or(2);
                    if let Some(i) = combo(ui, "fc_v", &names, cur, 190.0) {
                        st.align.v = vs[i].0;
                    }
                    ui.add_enabled_ui(false, |ui| {
                        let mut jd = false;
                        ui.checkbox(&mut jd, lang.tr("Justify distributed"));
                    });
                });
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    let indentable = matches!(st.align.h, HAlign::Left | HAlign::Right | HAlign::Distributed);
                    ui.add_enabled_ui(indentable, |ui| {
                        key_label(ui, "&Indent:");
                        let mut n = i32::from(st.align.indent);
                        if spin(ui, "fc_indent", &mut n, 0, 15, 60.0) {
                            st.align.indent = n as u8;
                        }
                    });
                });
            });
            ui.add_space(6.0);
            section(ui, "Text control", PAGE_W - 210.0);
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    ui.checkbox(&mut st.align.wrap, lang.tr("Wrap text"));
                    ui.checkbox(&mut st.align.shrink, lang.tr("Shrink to fit"));
                    let mut merge = d.values.get("merge").and_then(Json::as_bool).unwrap_or_else(|| selection_merged(app));
                    if ui.checkbox(&mut merge, lang.tr("Merge cells")).changed() {
                        d.values.insert("merge".into(), json!(merge));
                    }
                });
            });
            ui.add_space(6.0);
            section(ui, "Right-to-left", PAGE_W - 210.0);
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    key_label(ui, "&Text direction:");
                    // Reading order is not stored in the style model yet.
                    ui.add_enabled_ui(false, |ui| {
                        combo(ui, "fc_dir", &[lang.tr("Context").to_string()], 0, 130.0);
                    })
                    .response
                    .on_disabled_hover_text(lang.tr("Not available yet"));
                });
            });
        });
        ui.vertical(|ui| {
            group_box(ui, "Orientation", vec2(170.0, 240.0), |ui| orientation(ui, st));
        });
    });
}

fn selection_merged(app: &SheetApp) -> bool {
    app.session.active().and_then(|doc| doc.wb.active().map(|s| s.merge_at(doc.selection.active).is_some_and(|m| !m.is_single()))).unwrap_or(false)
}

// ---- font ------------------------------------------------------------------------------------------

fn font_tab(app: &SheetApp, ui: &mut Ui, st: &mut Style) {
    let lang = crate::i18n::current(ui.ctx());
    let t = Tokens::get(ui.ctx());
    let theme = app.session.active().map(|doc| doc.wb.theme.clone()).unwrap_or_default();
    let colors = theme.colors;
    let families = crate::fonts::families();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(220.0);
            key_label(ui, "&Font:");
            ui.add_sized([214.0, 24.0], egui::TextEdit::singleline(&mut st.font.name));
            let sel = families.iter().position(|f| f.eq_ignore_ascii_case(&st.font.name));
            if let Some(i) = list_box(ui, "fc_font", &families, sel, vec2(220.0, 120.0)) {
                st.font.name = families[i].clone();
            }
        });
        ui.add_space(10.0);
        ui.vertical(|ui| {
            ui.set_width(150.0);
            key_label(ui, "F&ont style:");
            let styles = ["Regular", "Italic", "Bold", "Bold Italic"];
            let cur = match (st.font.bold, st.font.italic) {
                (false, false) => 0,
                (false, true) => 1,
                (true, false) => 2,
                (true, true) => 3,
            };
            let mut shown = lang.tr(styles[cur]).to_string();
            ui.add_enabled(false, egui::TextEdit::singleline(&mut shown).desired_width(144.0));
            let names: Vec<String> = styles.iter().map(|s| lang.tr(s).to_string()).collect();
            if let Some(i) = list_box(ui, "fc_style", &names, Some(cur), vec2(150.0, 120.0)) {
                st.font.bold = i >= 2;
                st.font.italic = i % 2 == 1;
            }
        });
        ui.add_space(10.0);
        ui.vertical(|ui| {
            ui.set_width(130.0);
            key_label(ui, "&Size:");
            // While the box has focus its text is kept as typed ("10.", or empty); the size takes
            // each value that parses.
            let buf_id = egui::Id::new("fc_size_text");
            let mut size_text: String = ui.data(|d| d.get_temp(buf_id)).unwrap_or_else(|| format!("{}", st.font.size));
            let resp = ui.add_sized([124.0, 24.0], egui::TextEdit::singleline(&mut size_text));
            if resp.changed()
                && let Ok(v) = size_text.trim().parse::<f32>()
                && (1.0..=409.0).contains(&v)
            {
                st.font.size = v;
            }
            if resp.has_focus() {
                ui.data_mut(|d| d.insert_temp(buf_id, size_text));
            } else {
                ui.data_mut(|d| d.remove::<String>(buf_id));
            }
            let sizes = [8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0, 36.0, 48.0, 72.0];
            let names: Vec<String> = sizes.iter().map(|s| format!("{s}")).collect();
            let sel = sizes.iter().position(|s| (*s - st.font.size).abs() < 0.01);
            if let Some(i) = list_box(ui, "fc_size", &names, sel, vec2(130.0, 120.0)) {
                st.font.size = sizes[i];
            }
        });
    });
    ui.add_space(6.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(220.0);
            key_label(ui, "&Underline:");
            let us = [
                (Underline::None, "None"),
                (Underline::Single, "Single"),
                (Underline::Double, "Double"),
                (Underline::SingleAccounting, "Single Accounting"),
                (Underline::DoubleAccounting, "Double Accounting"),
            ];
            let names: Vec<String> = us.iter().map(|(_, n)| lang.tr(n).to_string()).collect();
            let cur = us.iter().position(|(u, _)| *u == st.font.underline).unwrap_or(0);
            if let Some(i) = combo(ui, "fc_underline", &names, cur, 206.0) {
                st.font.underline = us[i].0;
            }
        });
        ui.add_space(10.0);
        ui.vertical(|ui| {
            key_label(ui, "&Color:");
            let current = if matches!(st.font.color, Color::Auto) { None } else { resolve(&st.font.color, &theme) };
            if let Some(c) = color_combo(ui, "Font color", current, &colors, "Automatic", 150.0) {
                st.font.color = match c {
                    Some(hex) => Color::from_hex(&hex).unwrap_or_default(),
                    None => Color::Auto,
                };
            }
        });
        ui.add_space(10.0);
        ui.vertical(|ui| {
            ui.add_space(18.0);
            let default = Style::default().font;
            let mut normal = st.font == default;
            if ui.checkbox(&mut normal, lang.tr("Normal font")).changed() && normal {
                st.font = default;
            }
        });
    });
    ui.add_space(8.0);
    ui.horizontal_top(|ui| {
        group_box(ui, "Effects", vec2(220.0, 112.0), |ui| {
            ui.checkbox(&mut st.font.strike, lang.tr("Strikethrough"));
            let mut sup = st.font.vert == VertAlign::Superscript;
            if ui.checkbox(&mut sup, lang.tr("Superscript")).changed() {
                st.font.vert = if sup { VertAlign::Superscript } else { VertAlign::Baseline };
            }
            let mut sub = st.font.vert == VertAlign::Subscript;
            if ui.checkbox(&mut sub, lang.tr("Subscript")).changed() {
                st.font.vert = if sub { VertAlign::Subscript } else { VertAlign::Baseline };
            }
        });
        ui.add_space(10.0);
        group_box(ui, "Preview", vec2(PAGE_W - 240.0, 112.0), |ui| {
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 76.0), Sense::hover());
            let bg = if t.dark { Color32::from_gray(30) } else { Color32::WHITE };
            ui.painter().rect(r, CornerRadius::ZERO, bg, Stroke::new(1.0, Color32::from_gray(160)), StrokeKind::Inside);
            let size = (st.font.size * 96.0 / 72.0).clamp(8.0, 30.0);
            let font = egui::FontId::new(size, theme::cell_family(&st.font.name, st.font.bold, st.font.italic));
            let color = if matches!(st.font.color, Color::Auto) { Color32::BLACK } else { resolve(&st.font.color, &theme).unwrap_or(Color32::BLACK) };
            let g = ui.painter().layout_no_wrap(st.font.name.clone(), font, color);
            let pos = r.center() - g.size() / 2.0;
            let w = g.size().x;
            ui.painter().galley(pos, g, color);
            let y = r.center().y + size * 0.35;
            ui.painter().hline(r.left()..=pos.x - 6.0, y, Stroke::new(1.0, Color32::BLACK));
            ui.painter().hline(pos.x + w + 6.0..=r.right(), y, Stroke::new(1.0, Color32::BLACK));
            if st.font.strike {
                ui.painter().hline(pos.x..=pos.x + w, r.center().y, Stroke::new(1.0, color));
            }
        });
    });
    ui.add_space(6.0);
    ui.label(egui::RichText::new(lang.tr("The preview shows the font as it is drawn on screen and in print.")).font(theme::ui_font(12.5)));
}

// ---- border ------------------------------------------------------------------------------------------

const LINE_STYLES: [BorderStyle; 14] = [
    BorderStyle::None,
    BorderStyle::Hair,
    BorderStyle::Dotted,
    BorderStyle::DashDotDot,
    BorderStyle::DashDot,
    BorderStyle::Dashed,
    BorderStyle::Thin,
    BorderStyle::MediumDashDotDot,
    BorderStyle::SlantDashDot,
    BorderStyle::MediumDashDot,
    BorderStyle::MediumDashed,
    BorderStyle::Medium,
    BorderStyle::Thick,
    BorderStyle::Double,
];

fn style_name(s: BorderStyle) -> &'static str {
    match s {
        BorderStyle::None => "none",
        BorderStyle::Thin => "thin",
        BorderStyle::Medium => "medium",
        BorderStyle::Thick => "thick",
        BorderStyle::Dashed => "dashed",
        BorderStyle::Dotted => "dotted",
        BorderStyle::Double => "double",
        BorderStyle::Hair => "hair",
        BorderStyle::MediumDashed => "medium dashed",
        BorderStyle::DashDot => "dash dot",
        BorderStyle::MediumDashDot => "medium dash dot",
        BorderStyle::DashDotDot => "dash dot dot",
        BorderStyle::MediumDashDotDot => "medium dash dot dot",
        BorderStyle::SlantDashDot => "slant dash dot",
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Edge {
    Top,
    Bottom,
    Left,
    Right,
    DiagUp,
    DiagDown,
}

fn edge_slot(st: &mut Style, e: Edge) -> &mut BorderLine {
    match e {
        Edge::Top => &mut st.borders.top,
        Edge::Bottom => &mut st.borders.bottom,
        Edge::Left => &mut st.borders.left,
        Edge::Right => &mut st.borders.right,
        Edge::DiagUp => &mut st.borders.diag_up,
        Edge::DiagDown => &mut st.borders.diag_down,
    }
}

/// A square button with a tiny diagram of the edge it toggles.
fn edge_button(ui: &mut Ui, label: &str, e: Edge, on: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let bg = if on {
        t.accent_soft
    } else if resp.hovered() {
        t.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect(r, CornerRadius::same(3), bg, Stroke::new(1.0, Color32::from_gray(170)), StrokeKind::Inside);
    let b = r.shrink(8.0);
    let dot = Color32::from_gray(150);
    for pts in
        [[b.left_top(), b.right_top()], [b.left_bottom(), b.right_bottom()], [b.left_top(), b.left_bottom()], [b.right_top(), b.right_bottom()]]
    {
        ui.painter().extend(egui::Shape::dotted_line(&pts, dot, 2.0, 0.4));
    }
    let pts = match e {
        Edge::Top => [b.left_top(), b.right_top()],
        Edge::Bottom => [b.left_bottom(), b.right_bottom()],
        Edge::Left => [b.left_top(), b.left_bottom()],
        Edge::Right => [b.right_top(), b.right_bottom()],
        Edge::DiagUp => [b.left_bottom(), b.right_top()],
        Edge::DiagDown => [b.left_top(), b.right_bottom()],
    };
    ui.painter().line_segment(pts, Stroke::new(1.5, t.text));
    resp.clicked()
}

fn border_tab(app: &SheetApp, ui: &mut Ui, d: &mut Dialog, st: &mut Style) {
    let lang = crate::i18n::current(ui.ctx());
    let t = Tokens::get(ui.ctx());
    let theme = app.session.active().map(|doc| doc.wb.theme.clone()).unwrap_or_default();
    let line_idx = d.values.get("lineStyle").and_then(Json::as_u64).map_or(6, |v| v as usize).min(LINE_STYLES.len() - 1);
    let line_color: Option<String> = d.values.get("lineColor").and_then(Json::as_str).map(str::to_string);
    let color_of = |hex: &Option<String>| hex.as_ref().and_then(|h| Color::from_hex(h)).unwrap_or(Color::Auto);
    ui.horizontal_top(|ui| {
        group_box(ui, "Line", vec2(180.0, 300.0), |ui| {
            key_label(ui, "&Style:");
            let (area, _) = ui.allocate_exact_size(vec2(158.0, 190.0), Sense::hover());
            let bg = if t.dark { Color32::from_gray(30) } else { Color32::WHITE };
            ui.painter().rect(area, CornerRadius::ZERO, bg, Stroke::new(1.0, Color32::from_gray(150)), StrokeKind::Inside);
            for (i, s) in LINE_STYLES.iter().enumerate() {
                let col = i / 7;
                let row = i % 7;
                let cell = Rect::from_min_size(pos2(area.left() + 4.0 + col as f32 * 77.0, area.top() + 4.0 + row as f32 * 26.0), vec2(73.0, 24.0));
                let resp = ui.interact(cell, ui.id().with(("fc_line", i)), Sense::click());
                let label = format!("Line {}", style_name(*s));
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::SelectableLabel, true, &label));
                if i == line_idx {
                    ui.painter().rect_stroke(cell, CornerRadius::ZERO, Stroke::new(1.0, Color32::from_gray(90)), StrokeKind::Inside);
                }
                if *s == BorderStyle::None {
                    ui.painter().text(cell.center(), Align2::CENTER_CENTER, lang.tr("None"), theme::ui_font(13.0), t.text);
                } else {
                    let y = cell.center().y;
                    crate::grid::paint_border(ui.painter(), [pos2(cell.left() + 6.0, y), pos2(cell.right() - 6.0, y)], *s, t.text);
                }
                if resp.clicked() {
                    d.values.insert("lineStyle".into(), json!(i));
                }
            }
            ui.add_space(8.0);
            key_label(ui, "&Color:");
            let current = line_color.as_ref().and_then(|h| resolve(&color_of(&Some(h.clone())), &theme));
            if let Some(c) = color_combo(ui, "Line color", current, &theme.colors, "Automatic", 158.0) {
                match c {
                    Some(hex) => d.values.insert("lineColor".into(), json!(hex)),
                    None => d.values.remove("lineColor"),
                };
            }
        });
        ui.add_space(14.0);
        ui.vertical(|ui| {
            let chosen = BorderLine { style: LINE_STYLES[line_idx], color: color_of(&line_color) };
            section(ui, "Presets", 340.0);
            ui.horizontal(|ui| {
                ui.add_space(60.0);
                for (label, k) in [("&None", 0), ("&Outline", 1), ("&Inside", 2)] {
                    ui.vertical(|ui| {
                        let (r, resp) = ui.allocate_exact_size(vec2(64.0, 54.0), Sense::click());
                        let plain = label.replace('&', "");
                        let enabled = k != 2;
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &plain));
                        let bg = if resp.hovered() && enabled { t.hover } else { Color32::TRANSPARENT };
                        ui.painter().rect(r, CornerRadius::same(4), bg, Stroke::new(1.0, Color32::from_gray(180)), StrokeKind::Inside);
                        let b = r.shrink(12.0);
                        let ink = if enabled { t.text } else { t.text_disabled };
                        let dot = Color32::from_gray(160);
                        for pts in [
                            [b.left_top(), b.right_top()],
                            [b.left_bottom(), b.right_bottom()],
                            [b.left_top(), b.left_bottom()],
                            [b.right_top(), b.right_bottom()],
                        ] {
                            if k == 1 {
                                ui.painter().line_segment(pts, Stroke::new(1.6, ink));
                            } else {
                                ui.painter().extend(egui::Shape::dotted_line(&pts, dot, 2.5, 0.5));
                            }
                        }
                        let (mx, my) = (b.center().x, b.center().y);
                        let inner = if k == 2 { Stroke::new(1.6, ink) } else { Stroke::new(1.0, dot) };
                        ui.painter().line_segment([pos2(mx, b.top()), pos2(mx, b.bottom())], inner);
                        ui.painter().line_segment([pos2(b.left(), my), pos2(b.right(), my)], inner);
                        ui.horizontal(|ui| {
                            ui.add_space(14.0);
                            ui.add_enabled_ui(enabled, |ui| key_label(ui, label));
                        });
                        if resp.clicked() && enabled {
                            if k == 0 {
                                st.borders = Default::default();
                            } else {
                                for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                                    *edge_slot(st, e) = chosen;
                                }
                            }
                        }
                    });
                    ui.add_space(4.0);
                }
            });
            section(ui, "Border", 340.0);
            let toggle = |st: &mut Style, e: Edge| {
                let slot = edge_slot(st, e);
                *slot = if *slot == chosen && !slot.is_none() { BorderLine::default() } else { chosen };
            };
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 18.0;
                    if edge_button(ui, "Top border", Edge::Top, !st.borders.top.is_none()) {
                        toggle(st, Edge::Top);
                    }
                    ui.add_enabled_ui(false, |ui| edge_button(ui, "Inside horizontal border", Edge::Top, false));
                    if edge_button(ui, "Bottom border", Edge::Bottom, !st.borders.bottom.is_none()) {
                        toggle(st, Edge::Bottom);
                    }
                    if edge_button(ui, "Diagonal up border", Edge::DiagUp, !st.borders.diag_up.is_none()) {
                        toggle(st, Edge::DiagUp);
                    }
                });
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    border_preview(ui, st, &theme);
                    ui.horizontal(|ui| {
                        if edge_button(ui, "Left border", Edge::Left, !st.borders.left.is_none()) {
                            toggle(st, Edge::Left);
                        }
                        ui.add_space(72.0);
                        ui.add_enabled_ui(false, |ui| edge_button(ui, "Inside vertical border", Edge::Left, false));
                        ui.add_space(72.0);
                        if edge_button(ui, "Right border", Edge::Right, !st.borders.right.is_none()) {
                            toggle(st, Edge::Right);
                        }
                        if edge_button(ui, "Diagonal down border", Edge::DiagDown, !st.borders.diag_down.is_none()) {
                            toggle(st, Edge::DiagDown);
                        }
                    });
                });
            });
        });
    });
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(lang.tr("Pick a line style and color, then click a preset, the preview or the buttons around it."))
            .font(theme::ui_font(12.5)),
    );
}

/// The border preview: a white sheet with "Text" and the draft borders drawn as on the grid.
fn border_preview(ui: &mut Ui, st: &Style, theme: &gridcraft_engine::model::Theme) {
    let (area, response) = ui.allocate_exact_size(vec2(260.0, 160.0), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, "Border preview"));
    ui.painter().rect(area, CornerRadius::ZERO, Color32::WHITE, Stroke::new(1.0, Color32::from_gray(160)), StrokeKind::Inside);
    let r = area.shrink(10.0);
    let corner = Color32::from_gray(150);
    for (c, d) in
        [(r.left_top(), vec2(1.0, 1.0)), (r.right_top(), vec2(-1.0, 1.0)), (r.left_bottom(), vec2(1.0, -1.0)), (r.right_bottom(), vec2(-1.0, -1.0))]
    {
        ui.painter().line_segment([c, c + vec2(5.0 * d.x, 0.0)], Stroke::new(1.0, corner));
        ui.painter().line_segment([c, c + vec2(0.0, 5.0 * d.y)], Stroke::new(1.0, corner));
    }
    ui.painter().text(r.center(), Align2::CENTER_CENTER, "Text", theme::ui_font(13.0), Color32::BLACK);
    let b = &st.borders;
    for (line, pts) in [
        (b.top, [r.left_top(), r.right_top()]),
        (b.bottom, [r.left_bottom(), r.right_bottom()]),
        (b.left, [r.left_top(), r.left_bottom()]),
        (b.right, [r.right_top(), r.right_bottom()]),
        (b.diag_down, [r.left_top(), r.right_bottom()]),
        (b.diag_up, [r.left_bottom(), r.right_top()]),
    ] {
        if !line.is_none() {
            let color = line.color.resolve(theme).map(theme::color32).unwrap_or(Color32::BLACK);
            crate::grid::paint_border(ui.painter(), pts, line.style, color);
        }
    }
}

// ---- fill & protection -------------------------------------------------------------------------------

const PATTERNS: [(PatternType, &str); 19] = [
    (PatternType::None, "None"),
    (PatternType::Solid, "Solid"),
    (PatternType::Gray75, "75% Gray"),
    (PatternType::Gray50, "50% Gray"),
    (PatternType::Gray25, "25% Gray"),
    (PatternType::Gray125, "12.5% Gray"),
    (PatternType::Gray0625, "6.25% Gray"),
    (PatternType::DarkHorizontal, "Horizontal Stripe"),
    (PatternType::DarkVertical, "Vertical Stripe"),
    (PatternType::DarkDown, "Reverse Diagonal Stripe"),
    (PatternType::DarkUp, "Diagonal Stripe"),
    (PatternType::DarkGrid, "Diagonal Crosshatch"),
    (PatternType::DarkTrellis, "Thick Diagonal Crosshatch"),
    (PatternType::LightHorizontal, "Thin Horizontal Stripe"),
    (PatternType::LightVertical, "Thin Vertical Stripe"),
    (PatternType::LightDown, "Thin Reverse Diagonal Stripe"),
    (PatternType::LightUp, "Thin Diagonal Stripe"),
    (PatternType::LightGrid, "Thin Horizontal Crosshatch"),
    (PatternType::LightTrellis, "Thin Diagonal Crosshatch"),
];

/// The background colour (`None`: no fill). A solid fill is that colour; a patterned fill keeps
/// its pattern and pattern colour (`fg`) and takes it as its background (`bg`).
fn set_background(st: &mut Style, c: Option<Color>) {
    match (c, st.fill.pattern) {
        (None, _) => st.fill = Fill::default(),
        (Some(c), PatternType::None | PatternType::Solid) => st.fill = Fill::solid(c),
        (Some(c), _) => st.fill.bg = c,
    }
}

/// A new pattern style. Going from solid to a pattern keeps the solid colour as the background
/// and takes `pattern_color` (or Automatic) as the pattern's; going back to solid keeps the
/// background.
fn set_pattern(st: &mut Style, pattern: PatternType, pattern_color: Option<Color>) {
    let was = st.fill.pattern;
    let background = match was {
        PatternType::None => None,
        PatternType::Solid => Some(st.fill.fg),
        _ => Some(st.fill.bg),
    };
    st.fill = match pattern {
        PatternType::None => Fill::default(),
        PatternType::Solid => background.map(Fill::solid).unwrap_or_default(),
        _ => Fill {
            pattern,
            fg: match was {
                PatternType::None | PatternType::Solid => pattern_color.unwrap_or(Color::Auto),
                _ => pattern_color.unwrap_or(st.fill.fg),
            },
            bg: background.unwrap_or(Color::Auto),
        },
    };
}

fn fill_tab(app: &SheetApp, ui: &mut Ui, st: &mut Style) {
    let lang = crate::i18n::current(ui.ctx());
    let theme = app.session.active().map(|doc| doc.wb.theme.clone()).unwrap_or_default();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(270.0);
            key_label(ui, "Background &Color:");
            let picked = crate::widgets::color_palette(ui, &theme.colors, "No Color", "formatCellsFill")
                .or_else(|| crate::color_picker::take_result(ui.ctx(), "formatCellsFill"));
            if let Some(c) = picked {
                set_background(st, if c == "none" { None } else { Color::from_hex(&c) });
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_enabled(false, egui::Button::new(lang.tr("Fill Effects…")).min_size(vec2(120.0, 28.0)))
                    .on_disabled_hover_text(lang.tr("Not available yet"));
                let mut c32 = resolve(&st.fill.fg, &theme).unwrap_or(Color32::WHITE);
                let resp = ui.add(egui::Button::new(lang.tr("More Colors…")).min_size(vec2(120.0, 28.0)));
                egui::Popup::menu(&resp).show(|ui| {
                    if egui::color_picker::color_picker_color32(ui, &mut c32, egui::color_picker::Alpha::Opaque) {
                        let hex = format!("#{:02X}{:02X}{:02X}", c32.r(), c32.g(), c32.b());
                        set_background(st, Color::from_hex(&hex));
                    }
                });
            });
        });
        ui.add_space(20.0);
        ui.vertical(|ui| {
            key_label(ui, "P&attern Color:");
            // A pattern colour chosen before a pattern style waits for the style.
            let pending_id = egui::Id::new("fc_pattern_color");
            let patterned = !matches!(st.fill.pattern, PatternType::None | PatternType::Solid);
            let pending: Option<Color> = ui.data(|d| d.get_temp(pending_id));
            let current = if patterned { resolve(&st.fill.fg, &theme) } else { pending.as_ref().and_then(|c| resolve(c, &theme)) };
            if let Some(c) = color_combo(ui, "Pattern color", current, &theme.colors, "Automatic", 230.0) {
                let c = c.and_then(|hex| Color::from_hex(&hex)).unwrap_or(Color::Auto);
                if patterned {
                    st.fill.fg = c;
                } else {
                    ui.data_mut(|d| d.insert_temp(pending_id, c));
                }
            }
            key_label(ui, "&Pattern Style:");
            let names: Vec<String> = PATTERNS.iter().map(|(_, n)| lang.tr(n).to_string()).collect();
            let cur = PATTERNS.iter().position(|(p, _)| *p == st.fill.pattern).unwrap_or(0);
            if let Some(i) = combo(ui, "fc_pattern", &names, cur, 216.0) {
                let pending: Option<Color> = ui.data_mut(|d| d.remove_temp(pending_id));
                set_pattern(st, PATTERNS[i].0, pending);
            }
        });
    });
    ui.add_space(10.0);
    group_box(ui, "Sample", vec2(PAGE_W - 10.0, 90.0), |ui| {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::hover());
        let fill = match st.fill.pattern {
            PatternType::None => Color32::WHITE,
            PatternType::Solid => resolve(&st.fill.fg, &theme).unwrap_or(Color32::WHITE),
            _ => resolve(&st.fill.bg, &theme).unwrap_or(Color32::WHITE),
        };
        ui.painter().rect_filled(r, CornerRadius::ZERO, fill);
    });
}

fn protection_tab(ui: &mut Ui, st: &mut Style) {
    let lang = crate::i18n::current(ui.ctx());
    ui.checkbox(&mut st.protection.locked, lang.tr("Locked"));
    ui.checkbox(&mut st.protection.hidden, lang.tr("Hidden"));
    ui.add_space(4.0);
    ui.add(
        egui::Label::new(
            egui::RichText::new(lang.tr("Locking cells or hiding formulas has no effect until you protect the worksheet (Review › Protect Sheet)."))
                .font(theme::ui_font(13.0)),
        )
        .wrap(),
    );
}

// ---- frame -------------------------------------------------------------------------------------------

/// Draws the classic tab strip and returns the tab clicked, if any.
fn tab_strip(ui: &mut Ui, current: &str) -> Option<&'static str> {
    let t = Tokens::get(ui.ctx());
    let lang = crate::i18n::current(ui.ctx());
    let mut clicked = None;
    let tabs: [(&'static str, &str); 6] = [
        ("Number", "Number"),
        ("Alignment", "Alignment"),
        ("Font", "Font"),
        ("Border", "Border"),
        ("Fill", "Format Cells|Fill"),
        ("Protection", "Protection"),
    ];
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (tab, key) in tabs {
            let text = lang.tr(key);
            let g = ui.painter().layout_no_wrap(text.to_string(), theme::ui_font(13.5), t.text);
            let w = (g.size().x + 30.0).max(82.0);
            let selected = current == tab;
            let (r, resp) = ui.allocate_exact_size(vec2(w, 32.0), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, text));
            let r = if selected { r } else { Rect::from_min_max(r.min + vec2(0.0, 3.0), r.max) };
            let bg = if selected {
                page_bg(&t)
            } else if resp.hovered() {
                t.hover
            } else if t.dark {
                Color32::from_gray(50)
            } else {
                Color32::from_rgb(240, 240, 240)
            };
            ui.painter().rect_filled(r, CornerRadius { nw: 2, ne: 2, sw: 0, se: 0 }, bg);
            let stroke = Stroke::new(1.0, line_color(&t));
            ui.painter().line_segment([r.left_bottom(), r.left_top()], stroke);
            ui.painter().line_segment([r.left_top(), r.right_top()], stroke);
            ui.painter().line_segment([r.right_top(), r.right_bottom()], stroke);
            ui.painter().galley(r.center() - g.size() / 2.0, g, t.text);
            if resp.clicked() {
                clicked = Some(tab);
            }
        }
    });
    clicked
}

/// The whole dialog body.
pub(crate) fn show(app: &mut SheetApp, ui: &mut Ui, d: &mut Dialog, confirm: &mut bool, open: &mut bool) {
    let t = Tokens::get(ui.ctx());
    let lang = crate::i18n::current(ui.ctx());
    crate::dialogs::light_title(
        ui,
        &d.title,
        open,
        "Changes the number format, alignment, font, borders, fill and protection of the selected cells.",
    );
    ui.add_space(6.0);
    if let Some(tab) = tab_strip(ui, &d.tab) {
        d.tab = tab.to_string();
    }
    let mut st: Style = d.values.get("style").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
    let page = egui::Frame::new().fill(page_bg(&t)).stroke(Stroke::new(1.0, line_color(&t))).inner_margin(egui::Margin::same(12));
    page.show(ui, |ui| {
        ui.set_min_size(vec2(PAGE_W, PAGE_H));
        ui.set_max_width(PAGE_W);
        match d.tab.as_str() {
            "Number" => number_tab(app, ui, d, &mut st),
            "Alignment" => alignment_tab(app, ui, d, &mut st),
            "Font" => font_tab(app, ui, &mut st),
            "Border" => border_tab(app, ui, d, &mut st),
            "Fill" => fill_tab(app, ui, &mut st),
            _ => protection_tab(ui, &mut st),
        }
    });
    if let Ok(v) = serde_json::to_value(&st) {
        d.values.insert("style".into(), v);
    }
    ui.add_space(10.0);
    let mut ok = false;
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 34.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let button = |text: &str, primary: bool| {
            egui::Button::new(egui::RichText::new(lang.tr(text)).font(theme::ui_font(13.5)))
                .min_size(vec2(118.0, 32.0))
                .corner_radius(4)
                .stroke(if primary { Stroke::new(1.5, t.accent) } else { Stroke::new(1.0, Color32::from_gray(170)) })
        };
        if ui.add(button("Cancel", false)).clicked() {
            *open = false;
        }
        ui.add_space(8.0);
        if ui.add(button("OK", true)).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            ok = true;
        }
    });
    if ok {
        if let Some(style) = d.values.get("style").cloned() {
            app.run_or_alert("home.formatCells", json!({"style": style}));
        }
        if let Some(merge) = d.values.get("merge").and_then(Json::as_bool)
            && merge != selection_merged(app)
        {
            app.run_or_alert(if merge { "home.mergeCells" } else { "home.unmergeCells" }, json!({}));
        }
        *confirm = true;
    }
}

#[cfg(test)]
mod tests {
    use super::category_of;

    #[test]
    fn colour_and_locale_brackets_dont_change_the_category() {
        // Codes this dialog writes for red negatives: [Red] must not read as a Date's `d`.
        for code in ["0.00;[Red]0.00", "#,##0_);[Red](#,##0)", "#,##0.00;[Red]#,##0.00"] {
            assert_eq!(category_of(code), "Number", "{code}");
        }
        assert_eq!(category_of("[h]:mm:ss"), "Time", "an elapsed-hours code is still Time");
        assert_eq!(category_of("d-mmm-yy"), "Date");
        assert_eq!(category_of("0.00%"), "Percentage");
    }

    #[test]
    fn a_locale_tag_is_not_a_currency_but_a_bracketed_symbol_is() {
        assert_eq!(category_of("[$-409]mmmm d, yyyy"), "Date");
        assert_eq!(category_of("[$-409]h:mm:ss AM/PM"), "Time");
        assert_eq!(category_of("[$€-407]#,##0.00"), "Currency");
        assert_eq!(category_of("$#,##0.00"), "Currency");
    }

    #[test]
    fn background_and_pattern_colours_stay_apart() {
        use gridcraft_engine::model::{Color, PatternType, Style};
        let (yellow, blue) = (Color::from_hex("#FFFF00").unwrap_or_default(), Color::from_hex("#0000FF").unwrap_or_default());
        let mut st = Style::default();
        super::set_background(&mut st, Some(yellow));
        assert_eq!((st.fill.pattern, st.fill.fg), (PatternType::Solid, yellow));
        // A pattern style keeps yellow as the background and takes the chosen pattern colour.
        super::set_pattern(&mut st, PatternType::Gray75, Some(blue));
        assert_eq!((st.fill.pattern, st.fill.fg, st.fill.bg), (PatternType::Gray75, blue, yellow));
        // A new background on a patterned fill keeps the pattern.
        super::set_background(&mut st, Some(blue));
        assert_eq!((st.fill.pattern, st.fill.fg, st.fill.bg), (PatternType::Gray75, blue, blue));
        // Back to solid: the background colour.
        super::set_pattern(&mut st, PatternType::Solid, None);
        assert_eq!((st.fill.pattern, st.fill.fg), (PatternType::Solid, blue));
    }
}
