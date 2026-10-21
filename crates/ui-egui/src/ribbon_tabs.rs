//! The Insert, Draw, Formulas, Data, Review, View, Automate, Developer and Help tabs in the
//! desktop layout: the same groups, order, labels and button sizes, with this
//! project's own artwork (`ribbon_art`).
//!
//! Every command the engine has is wired. A control the app doesn't have yet is still shown in its
//! place, greyed out: it says "Not available yet" when pointed at and does nothing when clicked.

use super::*;
use crate::ribbon_art::{Art, paint};
use crate::ribbon_extra::{big, end_group, small};

const SOON: &str = "Not available yet";

fn art(a: Art) -> impl FnOnce(&egui::Painter, Rect, Color32) {
    move |p: &egui::Painter, r: Rect, c: Color32| paint(p, r, a, c)
}

/// A large button with original artwork.
fn abig(ui: &mut Ui, a: Art, label: &str, tip: &str, dropdown: bool) -> egui::Response {
    big(ui, art(a), label, tip, dropdown, true, false)
}

/// A small button with original artwork.
fn asmall(ui: &mut Ui, a: Art, label: &str, tip: &str, dropdown: bool) -> egui::Response {
    small(ui, art(a), label, tip, dropdown, true)
}

/// A large button for a command GridCraft doesn't have yet: shown greyed out, with a tooltip.
fn soon_big(_app: &mut SheetApp, ui: &mut Ui, a: Art, label: &str, dropdown: bool) -> egui::Response {
    big(ui, art(a), label, SOON, dropdown, false, false)
}

/// A small button for a command GridCraft doesn't have yet: shown greyed out, with a tooltip.
fn soon_small(_app: &mut SheetApp, ui: &mut Ui, a: Art, label: &str, dropdown: bool) -> egui::Response {
    small(ui, art(a), label, SOON, dropdown, false)
}

/// A large button shown greyed out, as the desktop app does when it doesn't apply.
fn off_big(ui: &mut Ui, a: Art, label: &str, dropdown: bool) {
    big(ui, art(a), label, SOON, dropdown, false, false);
}

/// A small button shown greyed out.
fn off_small(ui: &mut Ui, a: Art, label: &str, dropdown: bool) {
    small(ui, art(a), label, SOON, dropdown, false);
}

/// A column of small buttons (three rows fill the ribbon body).
fn stack(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        add(ui);
    });
}

/// An icon-only small button with a drop-down arrow (the chart and Sort buttons).
fn icon_drop(ui: &mut Ui, a: Art, tip: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(38.0, 24.0), Sense::click());
    if resp.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, 4.0, t.pressed);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + 13.0, rect.center().y), vec2(18.0, 18.0)), a, t.text);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.right() - 8.0, rect.center().y + 1.0), vec2(10.0, 10.0)), Icon::Chevron, t.text_dim);
    let label = app_tr(ui, tip);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    resp.on_hover_text(label)
}

/// An icon-only small button.
fn icon_only(ui: &mut Ui, a: Art, tip: &str, enabled: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 24.0), if enabled { Sense::click() } else { Sense::hover() });
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(18.0, 18.0)), a, if enabled { t.text } else { t.text_disabled });
    let label = app_tr(ui, tip);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &label));
    resp.on_hover_text(label)
}

/// Chart type menus shared by the Charts group's small buttons.
fn chart_menu(app: &mut SheetApp, ui: &mut Ui, a: Art, tip: &str, cid: &str, subs: &[(&str, &str, &str)]) {
    let r = icon_drop(ui, a, tip);
    control_keytip(app, ui, &r, cid);
    let forced = invoke(app, cid);
    egui::Popup::menu(&r).open_memory(popup_command(forced, r.clicked())).show(|ui| {
        for (label, k, s) in subs {
            if ui.button(app.ui.language.tr(label)).clicked() {
                act(app, "insert.chart", json!({"type": k, "subtype": s}));
            }
        }
    });
}

// ---------------------------------------------------------------------------------------- Insert

pub(super) fn insert(app: &mut SheetApp, ui: &mut Ui) {
    // Tables
    let pt = abig(ui, Art::PivotTable, "PivotTable", "PivotTable", true);
    control_keytip(app, ui, &pt, "insert.pivotTable");
    egui::Popup::menu(&pt).show(|ui| {
        if ui.button(app.ui.language.tr("From Table/Range")).clicked() {
            act(app, "insert.pivotTable", json!({}));
        }
        ui.add_enabled(false, egui::Button::new(app.ui.language.tr("From External Data Source"))).on_disabled_hover_text(SOON);
    });
    soon_big(app, ui, Art::RecommendedPivot, "Recommended\nPivotTables", false);
    let __k = abig(ui, Art::Table, "Table", "Table (Ctrl+T)", false);
    control_keytip(app, ui, &__k, "insert.table");
    if __k.clicked() {
        act(app, "insert.table", json!({}));
    }
    soon_big(app, ui, Art::Forms, "Forms", true);
    end_group(ui, "Tables", false);
    // Illustrations
    stack(ui, |ui| {
        let pic = asmall(ui, Art::Pictures, "Pictures", "Insert a picture", true);
        anchor_keytip(app, ui, &pic, &["insert.picture"]);
        egui::Popup::menu(&pic).show(|ui| {
            for (label, placement) in [("Place in Cell", "cell"), ("Place over Cells", "overCells")] {
                if ui.button(app.ui.language.tr(label)).clicked() {
                    app.open_dialog("insertPicture", json!({"placement": placement}));
                    ui.close();
                }
            }
        });
        let shapes = asmall(ui, Art::Shapes, "Shapes", "Shapes", true);
        anchor_keytip(app, ui, &shapes, &["insert.shape"]);
        egui::Popup::menu(&shapes).show(|ui| {
            for (label, kind) in [
                ("Rectangle", "rectangle"),
                ("Rounded Rectangle", "roundedRectangle"),
                ("Oval", "ellipse"),
                ("Triangle", "triangle"),
                ("Line", "line"),
                ("Arrow", "arrow"),
                ("Text Box", "textBox"),
            ] {
                if ui.button(app.ui.language.tr(label)).clicked() {
                    act(app, "insert.shape", json!({"kind": kind}));
                }
            }
        });
        let icn = asmall(ui, Art::Icons, "Icons", "Insert an icon", false);
        anchor_keytip(app, ui, &icn, &["insert.icons"]);
        egui::Popup::menu(&icn).show(|ui| {
            ui.set_width(260.0);
            egui::Grid::new("icon_lib").show(ui, |ui| {
                for (i, (name, icon)) in icons::LIBRARY.iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(r, 4.0, Tokens::get(ui.ctx()).hover);
                    }
                    icons::paint(ui.painter(), r.shrink(6.0), *icon, Tokens::get(ui.ctx()).text);
                    if resp.on_hover_text(*name).clicked() {
                        act(app, "insert.icons", json!({"name": name}));
                        ui.close();
                    }
                    if (i + 1) % 6 == 0 {
                        ui.end_row();
                    }
                }
            });
        });
    });
    stack(ui, |ui| {
        soon_small(app, ui, Art::Models3d, "3D Models", true);
        soon_small(app, ui, Art::SmartArt, "SmartArt", false);
        soon_small(app, ui, Art::Screenshot, "Screenshot", true);
    });
    end_group(ui, "Illustrations", false);
    // Controls
    let __k = abig(ui, Art::Checkbox, "Checkbox", "Insert checkboxes in the selected cells", false);
    if __k.clicked() {
        act(app, "insert.checkbox", json!({}));
    }
    end_group(ui, "Controls", false);
    // Charts
    let __k = abig(ui, Art::RecommendedCharts, "Recommended\nCharts", "Recommended Charts", false);
    control_keytip(app, ui, &__k, "insert.recommendedCharts");
    if __k.clicked() {
        // With no chart type, run_or_alert opens the Insert Chart dialog (as in Excel).
        app.run_or_alert("insert.recommendedCharts", json!({}));
    }
    stack(ui, |ui| {
        ui.horizontal(|ui| {
            chart_menu(
                app,
                ui,
                Art::ChartColumn,
                "Insert Column or Bar Chart",
                "insert.chart.column",
                &[
                    ("Clustered Column", "column", ""),
                    ("Stacked Column", "column", "stacked"),
                    ("100% Stacked Column", "column", "stacked100"),
                    ("Clustered Bar", "bar", ""),
                    ("Stacked Bar", "bar", "stacked"),
                ],
            );
            chart_menu(
                app,
                ui,
                Art::ChartHierarchy,
                "Insert Hierarchy Chart",
                "insert.chart.hierarchy",
                &[("Treemap", "treemap", ""), ("Sunburst", "sunburst", "")],
            );
            chart_menu(
                app,
                ui,
                Art::ChartWaterfall,
                "Insert Waterfall, Funnel, Stock or Radar Chart",
                "insert.chart.waterfall",
                &[("Waterfall", "waterfall", ""), ("Funnel", "funnel", ""), ("Stock", "stock", ""), ("Radar", "radar", "")],
            );
        });
        ui.horizontal(|ui| {
            chart_menu(
                app,
                ui,
                Art::ChartLine,
                "Insert Line or Area Chart",
                "insert.chart.line",
                &[
                    ("Line", "line", ""),
                    ("Line with Markers", "line", "markers"),
                    ("Stacked Line", "line", "stacked"),
                    ("Area", "area", ""),
                    ("Stacked Area", "area", "stacked"),
                ],
            );
            chart_menu(
                app,
                ui,
                Art::ChartStatistic,
                "Insert Statistic Chart",
                "insert.chart.statistic",
                &[("Histogram", "histogram", ""), ("Box & Whisker", "boxWhisker", "")],
            );
            chart_menu(app, ui, Art::ChartCombo, "Insert Combo Chart", "insert.chart.combo", &[("Combo", "combo", "")]);
        });
        ui.horizontal(|ui| {
            chart_menu(
                app,
                ui,
                Art::ChartPie,
                "Insert Pie or Doughnut Chart",
                "insert.chart.pie",
                &[("Pie", "pie", ""), ("Doughnut", "doughnut", "")],
            );
            chart_menu(
                app,
                ui,
                Art::ChartScatter,
                "Insert Scatter (X, Y) or Bubble Chart",
                "insert.chart.scatter",
                &[("Scatter", "scatter", ""), ("Scatter with Lines", "scatter", "lines"), ("Bubble", "bubble", "")],
            );
        });
    });
    soon_big(app, ui, Art::Maps, "Maps", true);
    soon_big(app, ui, Art::PivotChart, "PivotChart", true);
    end_group(ui, "Charts", false);
    // Sparklines
    for (a, label, kind) in [(Art::SparkLine, "Line", "line"), (Art::SparkColumn, "Column", "column"), (Art::SparkWinLoss, "Win/\nLoss", "winLoss")] {
        let r = abig(ui, a, label, "Insert Sparklines", false);
        if r.clicked() {
            app.open_dialog("sparkline", json!({"type": kind}));
        }
    }
    end_group(ui, "Sparklines", false);
    // Filters
    if abig(ui, Art::Slicer, "Slicer", "Insert a slicer to filter a table", false).clicked() {
        act(app, "insert.slicer", json!({}));
    }
    soon_big(app, ui, Art::Timeline, "Timeline", false);
    end_group(ui, "Filters", false);
    // Links
    let __k = abig(ui, Art::Link, "Link", "Insert Link (Ctrl+K)", true);
    control_keytip(app, ui, &__k, "dialog:insertLink");
    if __k.clicked() {
        app.open_dialog("insertLink", json!({}));
    }
    end_group(ui, "Links", false);
    // Comments
    let __k = abig(ui, Art::Comment, "Comment", "New Comment", false);
    control_keytip(app, ui, &__k, "dialog:comment");
    if __k.clicked() {
        app.open_dialog("comment", json!({"threaded": true}));
    }
    end_group(ui, "Comments", false);
    // Text
    let __k = abig(ui, Art::TextBox, "Text\nBox", "Text Box", false);
    control_keytip(app, ui, &__k, "insert.textBox");
    if __k.clicked() {
        act(app, "insert.textBox", json!({}));
    }
    let __k = abig(ui, Art::HeaderFooter, "Header\n& Footer", "Header & Footer", false);
    control_keytip(app, ui, &__k, "dialog:headerFooter");
    if __k.clicked() {
        // As in the desktop app: Page Layout view, editing the header in place.
        crate::paper_view::start_editing(app);
    }
    stack(ui, |ui| {
        soon_small(app, ui, Art::WordArt, "WordArt", true);
        soon_small(app, ui, Art::SignatureLine, "Signature Line", true);
        soon_small(app, ui, Art::Object, "Object", false);
    });
    end_group(ui, "Text", false);
    // Symbols
    stack(ui, |ui| {
        soon_small(app, ui, Art::Equation, "Equation", true);
        let sym = asmall(ui, Art::Symbol, "Symbol", "Symbol", false);
        control_keytip(app, ui, &sym, "insert.symbol");
        egui::Popup::menu(&sym).show(|ui| {
            ui.set_width(260.0);
            ui.horizontal_wrapped(|ui| {
                for ch in [
                    "€", "£", "¥", "©", "®", "™", "±", "≠", "≤", "≥", "÷", "×", "∞", "µ", "α", "β", "π", "Σ", "Ω", "°", "•", "…", "→", "←", "↑", "↓",
                    "✓", "★", "½", "¼",
                ] {
                    if ui.button(ch).clicked() {
                        act(app, "insert.symbol", json!({"char": ch}));
                    }
                }
            });
        });
    });
    end_group(ui, "Symbols", false);
}

// ------------------------------------------------------------------------------------------ Draw

pub(super) fn draw(app: &mut SheetApp, ui: &mut Ui) {
    let can_undo = app.session.active().is_some_and(|d| !d.undo.is_empty());
    let can_redo = app.session.active().is_some_and(|d| !d.redo.is_empty());
    stack(ui, |ui| {
        ui.add_space(6.0);
        if icon_only(ui, Art::Undo, "Undo", can_undo).clicked() {
            app.run_or_alert("edit.undo", json!({}));
        }
        if icon_only(ui, Art::Redo, "Redo", can_redo).clicked() {
            app.run_or_alert("edit.redo", json!({}));
        }
    });
    end_group(ui, "Undo", false);
    let tool = app.session.draw_tool.clone();
    let accent = Tokens::get(ui.ctx()).accent;
    let mark = |ui: &mut Ui, r: &egui::Response, on: bool| {
        if on {
            ui.painter().rect_stroke(r.rect, 5.0, Stroke::new(2.0, accent), StrokeKind::Inside);
        }
    };
    let sel = abig(ui, Art::Select, "", "Select Objects", false);
    control_keytip(app, ui, &sel, "draw.lasso");
    mark(ui, &sel, tool == "lasso");
    if sel.clicked() {
        act(app, "draw.lasso", json!({}));
    }
    let lasso = abig(ui, Art::Lasso, "", "Lasso Select", false);
    if lasso.clicked() {
        act(app, "draw.lasso", json!({}));
    }
    let er = abig(ui, Art::Eraser, "", "Eraser", false);
    mark(ui, &er, tool == "eraser");
    if er.clicked() {
        act(app, "draw.eraser", json!({}));
    }
    let black = Color32::from_rgb(0x20, 0x20, 0x20);
    let red = Color32::from_rgb(0xD1, 0x3B, 0x2F);
    let yellow = Color32::from_rgb(0xF6, 0xF0, 0x3C);
    let blue = Color32::from_rgb(0x1F, 0x4E, 0x96);
    let green = Color32::from_rgb(0x10, 0x8A, 0x3E);
    for (a, color, tip, width) in [
        (Art::Pen(black), "#000000", "Pen: Black, 0.5 mm", 2.0),
        (Art::Pen(red), "#D13B2F", "Pen: Red, 0.5 mm", 2.0),
        (Art::Highlighter(yellow), "#F2E33A", "Highlighter: Yellow, 6 mm", 12.0),
        (Art::Pencil(blue), "#1F4E96", "Pen: Blue, 1 mm", 3.0),
        (Art::Pencil(green), "#108A3E", "Pen: Green, 1 mm", 3.0),
    ] {
        let on = tool == "pen" && app.session.draw_color.eq_ignore_ascii_case(color);
        let r = abig(ui, a, "", tip, false);
        mark(ui, &r, on);
        if r.clicked() {
            act(app, "draw.pen", json!({"on": !on, "color": color, "width": width}));
        }
    }
    big(ui, art(Art::GalaxyPen), "", "Galaxy Pen: not available yet", false, false, false);
    soon_big(app, ui, Art::AddPen, "Add", true);
    end_group(ui, "Drawing Tools", false);
    let __k = abig(ui, Art::InkToShape, "Ink to\nShape", "Convert the last ink stroke to a shape", false);
    control_keytip(app, ui, &__k, "draw.inkToShape");
    if __k.clicked() {
        act(app, "draw.inkToShape", json!({}));
    }
    soon_big(app, ui, Art::InkToMath, "Ink to\nMath", false);
    end_group(ui, "Convert", false);
    soon_big(app, ui, Art::InkReplay, "Ink\nReplay", false);
    end_group(ui, "Replay", false);
    soon_big(app, ui, Art::Help, "Ink\nHelp", false);
    end_group(ui, "Help", false);
}

// -------------------------------------------------------------------------------------- Formulas

pub(super) fn formulas(app: &mut SheetApp, ui: &mut Ui) {
    let __k = abig(ui, Art::InsertFunction, "Insert\nFunction", "Insert Function (Shift+F3)", false);
    control_keytip(app, ui, &__k, "dialog:insertFunction");
    if __k.clicked() {
        app.open_dialog("insertFunction", json!({}));
    }
    end_group(ui, "", false);
    let s = abig(ui, Art::AutoSum, "AutoSum", "AutoSum", true);
    control_keytip(app, ui, &s, "formulas.autoSum");
    egui::Popup::menu(&s).show(|ui| {
        for f in ["SUM", "AVERAGE", "COUNT", "MAX", "MIN"] {
            if ui.button(f).clicked() {
                act(app, "formulas.autoSum", json!({"function": f}));
            }
        }
        ui.separator();
        if ui.button(app.ui.language.tr("More Functions…")).clicked() {
            app.open_dialog("insertFunction", json!({}));
        }
    });
    soon_big(app, ui, Art::RecentlyUsed, "Recently\nUsed", true);
    for (a, label, cat, cid) in [
        (Art::Financial, "Financial", "Financial", "formulas.financial"),
        (Art::Logical, "Logical", "Logical", "formulas.logical"),
        (Art::Text, "Text", "Text", "formulas.text"),
        (Art::DateTime, "Date &\nTime", "DateTime", "formulas.dateTime"),
        (Art::Lookup, "Lookup &\nReference", "Lookup", "formulas.lookup"),
        (Art::MathTrig, "Math &\nTrig", "MathTrig", "formulas.mathTrig"),
        (Art::MoreFunctions, "More\nFunctions", "Statistical", "formulas.moreFunctions"),
    ] {
        let r = abig(ui, a, label, label, true);
        control_keytip(app, ui, &r, cid);
        let forced_c = invoke(app, cid);
        egui::Popup::menu(&r).open_memory(popup_command(forced_c, r.clicked())).show(|ui| {
            ui.set_max_height(420.0);
            egui::ScrollArea::vertical().show(ui, |ui| {
                for f in gridcraft_engine::cmd::formulas::function_list() {
                    if f["category"].as_str() == Some(cat)
                        && let Some(n) = f["name"].as_str()
                    {
                        let locale = app.ui.language.formula_locale();
                        let name =
                            app.session.active().map(|d| crate::formula_locale::completion_name(n, locale, &d.wb, d.wb.active_sheet)).unwrap_or(n);
                        let desc = crate::formula_locale::description(n, locale).unwrap_or_default();
                        if ui.button(name).on_hover_text(desc).clicked() {
                            app.run_or_alert("formulas.insertFunction", json!({"name": n}));
                            ui.close();
                        }
                    }
                }
            });
        });
    }
    end_group(ui, "Function Library", false);
    soon_big(app, ui, Art::Python, "Insert\nPython", false);
    stack(ui, |ui| {
        soon_small(app, ui, Art::Reset, "Reset", true);
        soon_small(app, ui, Art::Editor, "Editor", false);
        soon_small(app, ui, Art::Initialization, "Initialization", false);
    });
    end_group(ui, "Python (Preview)", false);
    let __k = abig(ui, Art::NameManager, "Name\nManager", "Name Manager (Ctrl+F3)", false);
    control_keytip(app, ui, &__k, "dialog:nameManager");
    if __k.clicked() {
        app.open_dialog("nameManager", json!({}));
    }
    stack(ui, |ui| {
        let __k = asmall(ui, Art::DefineName, "Define Name", "Define Name", true);
        control_keytip(app, ui, &__k, "dialog:defineName");
        if __k.clicked() {
            app.open_dialog("defineName", json!({}));
        }
        let use_in = asmall(ui, Art::UseInFormula, "Use in Formula", "Use in Formula", true);
        anchor_keytip(app, ui, &use_in, &["formulas.pasteName"]);
        egui::Popup::menu(&use_in).show(|ui| {
            let names: Vec<String> = app.session.active().map(|d| d.wb.names.iter().map(|n| n.name.clone()).collect()).unwrap_or_default();
            for n in names {
                if ui.button(&n).clicked() {
                    let cur = app.editor.as_ref().map(|e| e.text.clone()).unwrap_or_else(|| "=".into());
                    app.begin_edit(Some(format!("{cur}{n}")), false);
                }
            }
        });
        let __k = asmall(ui, Art::CreateFromSelection, "Create from Selection", "Create from Selection", false);
        control_keytip(app, ui, &__k, "formulas.createFromSelection");
        if __k.clicked() {
            act(app, "formulas.createFromSelection", json!({}));
        }
    });
    end_group(ui, "Defined Names", false);
    stack(ui, |ui| {
        if asmall(ui, Art::TracePrecedents, "Trace Precedents", "Trace Precedents", false).clicked()
            && let Ok(r) = app.run("formulas.tracePrecedents", json!({}))
        {
            app.grid.trace = Some(r);
        }
        if asmall(ui, Art::TraceDependents, "Trace Dependents", "Trace Dependents", false).clicked()
            && let Ok(r) = app.run("formulas.traceDependents", json!({}))
        {
            app.grid.trace = Some(r);
        }
        if asmall(ui, Art::RemoveArrows, "Remove Arrows", "Remove Arrows", true).clicked() {
            app.grid.trace = None;
        }
    });
    stack(ui, |ui| {
        let __k = asmall(ui, Art::ShowFormulas, "Show Formulas", "Show Formulas (Ctrl+`)", false);
        control_keytip(app, ui, &__k, "formulas.showFormulas");
        if __k.clicked() {
            act(app, "formulas.showFormulas", json!({}));
        }
        let __k = asmall(ui, Art::ErrorChecking, "Error Checking", "Error Checking", true);
        control_keytip(app, ui, &__k, "dialog:errorChecking");
        if __k.clicked() {
            app.open_dialog("errorChecking", json!({}));
        }
        let __k = asmall(ui, Art::EvaluateFormula, "Evaluate Formula", "Evaluate Formula", false);
        control_keytip(app, ui, &__k, "dialog:evaluateFormula");
        if __k.clicked() {
            app.open_dialog("evaluateFormula", json!({}));
        }
    });
    end_group(ui, "Formula Auditing", false);
    if abig(ui, Art::WatchWindow, "Watch\nWindow", "Watch Window", false).clicked() {
        app.grid.pane = Some("watch".into());
    }
    end_group(ui, "", false);
    let co = abig(ui, Art::CalcOptions, "Calculation\nOptions", "Calculation Options", true);
    for sequence in keytip_table::group_extension(app.keytips.prefix().unwrap_or_default(), &["formulas.calculationOptions"]) {
        keytip(app, ui, sequence, co.rect.center_bottom());
    }
    egui::Popup::menu(&co).show(|ui| {
        menu_items(
            app,
            ui,
            &[
                ("Automatic", "formulas.calculationOptions", json!({"mode": "automatic"})),
                ("Automatic Except for Data Tables", "formulas.calculationOptions", json!({"mode": "automaticExceptTables"})),
                ("Manual", "formulas.calculationOptions", json!({"mode": "manual"})),
            ],
        );
    });
    stack(ui, |ui| {
        ui.add_space(6.0);
        let __k = asmall(ui, Art::CalcNow, "Calculate Now", "Calculate Now (F9)", false);
        control_keytip(app, ui, &__k, "formulas.calculateNow");
        if __k.clicked() {
            act(app, "formulas.calculateNow", json!({}));
        }
        ui.add_space(6.0);
        let __k = asmall(ui, Art::CalcSheet, "Calculate Sheet", "Calculate Sheet (Shift+F9)", false);
        control_keytip(app, ui, &__k, "formulas.calculateSheet");
        if __k.clicked() {
            act(app, "formulas.calculateSheet", json!({}));
        }
    });
    end_group(ui, "Calculation", false);
}

// ------------------------------------------------------------------------------------------ Data

pub(super) fn data(app: &mut SheetApp, ui: &mut Ui) {
    // Get & Transform Data
    let gd = abig(ui, Art::GetData, "Get\nData", "Get Data", true);
    control_keytip(app, ui, &gd, "data.getData");
    egui::Popup::menu(&gd).show(|ui| {
        if ui.button(app.ui.language.tr("From File (Text/CSV, Workbook)…")).clicked() {
            app.open_dialog("open", json!({}));
        }
        for label in ["From Database", "From Azure", "From Online Services", "From Other Sources"] {
            ui.add_enabled(false, egui::Button::new(app.ui.language.tr(label))).on_disabled_hover_text(SOON);
        }
    });
    stack(ui, |ui| {
        if asmall(ui, Art::FromCsv, "From Text/CSV", "Import data from a text or CSV file", false).clicked() {
            app.open_dialog("open", json!({}));
        }
        soon_small(app, ui, Art::FromWeb, "From Web", false);
        soon_small(app, ui, Art::FromTable, "From Table/Range", false);
    });
    stack(ui, |ui| {
        soon_small(app, ui, Art::FromPicture, "From Picture", true);
        soon_small(app, ui, Art::RecentSources, "Recent Sources", false);
        soon_small(app, ui, Art::ExistingConnections, "Existing Connections", false);
    });
    end_group(ui, "Get & Transform Data", false);
    // Queries & Connections
    soon_big(app, ui, Art::RefreshAll, "Refresh\nAll", true);
    stack(ui, |ui| {
        soon_small(app, ui, Art::Queries, "Queries & Connections", false);
        off_small(ui, Art::Properties, "Properties", false);
        off_small(ui, Art::WorkbookLinks, "Workbook Links", false);
    });
    end_group(ui, "Queries & Connections", false);
    // Data Types
    soon_big(app, ui, Art::Stocks, "Stocks", false);
    soon_big(app, ui, Art::Currencies, "Currencies", false);
    end_group(ui, "Data Types", false);
    // Sort & Filter
    stack(ui, |ui| {
        ui.add_space(4.0);
        let __k = icon_only(ui, Art::SortAz, "Sort A to Z", true);
        control_keytip(app, ui, &__k, "data.sortAscending");
        if __k.clicked() {
            act(app, "data.sortAscending", json!({}));
        }
        ui.add_space(8.0);
        let __k = icon_only(ui, Art::SortZa, "Sort Z to A", true);
        control_keytip(app, ui, &__k, "data.sortDescending");
        if __k.clicked() {
            act(app, "data.sortDescending", json!({}));
        }
    });
    let __k = abig(ui, Art::Sort, "Sort", "Custom Sort", false);
    control_keytip(app, ui, &__k, "dialog:sort");
    if __k.clicked() {
        app.open_dialog("sort", json!({}));
    }
    let __k = abig(ui, Art::Filter, "Filter", "Filter (Ctrl+Shift+L)", false);
    control_keytip(app, ui, &__k, "data.filter");
    if __k.clicked() {
        act(app, "data.filter", json!({}));
    }
    stack(ui, |ui| {
        let __k = asmall(ui, Art::ClearFilter, "Clear", "Clear Filter", false);
        control_keytip(app, ui, &__k, "data.clearFilter");
        if __k.clicked() {
            act(app, "data.clearFilter", json!({}));
        }
        let __k = asmall(ui, Art::Reapply, "Reapply", "Reapply", false);
        control_keytip(app, ui, &__k, "data.reapply");
        if __k.clicked() {
            act(app, "data.reapply", json!({}));
        }
        soon_small(app, ui, Art::Advanced, "Advanced", false);
    });
    end_group(ui, "Sort & Filter", false);
    // Data Tools
    let __k = abig(ui, Art::TextToColumns, "Text to\nColumns", "Text to Columns", false);
    control_keytip(app, ui, &__k, "dialog:textToColumns");
    if __k.clicked() {
        app.open_dialog("textToColumns", json!({}));
    }
    stack(ui, |ui| {
        let __k = asmall(ui, Art::FlashFill, "Flash Fill", "Flash Fill (Ctrl+E)", false);
        control_keytip(app, ui, &__k, "edit.flashFill");
        if __k.clicked() {
            act(app, "edit.flashFill", json!({}));
        }
        let __k = asmall(ui, Art::RemoveDuplicates, "Remove Duplicates", "Remove Duplicates", false);
        control_keytip(app, ui, &__k, "dialog:removeDuplicates");
        if __k.clicked() {
            app.open_dialog("removeDuplicates", json!({}));
        }
        let dv = asmall(ui, Art::DataValidation, "Data Validation", "Data Validation", true);
        for sequence in keytip_table::group_extension(app.keytips.prefix().unwrap_or_default(), &["data.validation", "data.circleInvalid"]) {
            keytip(app, ui, sequence, dv.rect.center_bottom());
        }
        egui::Popup::menu(&dv).show(|ui| {
            menu_items(
                app,
                ui,
                &[
                    ("Data Validation…", "dialog:dataValidation", json!({})),
                    ("Circle Invalid Data", "data.circleInvalid", json!({})),
                    ("Clear Validation", "data.validation", json!({"clear": true})),
                ],
            );
        });
    });
    stack(ui, |ui| {
        soon_small(app, ui, Art::Consolidate, "Consolidate", false);
        soon_small(app, ui, Art::DataModel, "Data Model", true);
        if asmall(ui, Art::AnalyzeData, "Analyze Data", "Analysis ToolPak", false).clicked() {
            app.open_dialog("analysisToolPak", json!({}));
        }
    });
    end_group(ui, "Data Tools", false);
    // Forecast
    let wi = abig(ui, Art::WhatIf, "What-If\nAnalysis", "What-If Analysis", true);
    for sequence in
        keytip_table::group_extension(app.keytips.prefix().unwrap_or_default(), &["dialog:goalSeek", "dialog:scenarioManager", "data.dataTable"])
    {
        keytip(app, ui, sequence, wi.rect.center_bottom());
    }
    egui::Popup::menu(&wi).show(|ui| {
        menu_items(
            app,
            ui,
            &[
                ("Scenario Manager…", "dialog:scenarioManager", json!({})),
                ("Goal Seek…", "dialog:goalSeek", json!({})),
                ("Data Table…", "data.dataTable", json!({})),
            ],
        );
        if ui.button(app.ui.language.tr("Solver…")).clicked() {
            app.open_dialog("solver", json!({}));
            ui.close();
        }
    });
    if abig(ui, Art::ForecastSheet, "Forecast\nSheet", "Forecast Sheet", false).clicked() {
        app.open_dialog("forecastSheet", json!({}));
    }
    end_group(ui, "Forecast", false);
    // Outline
    let g = abig(ui, Art::Group, "Group", "Group", true);
    control_keytip(app, ui, &g, "data.group");
    egui::Popup::menu(&g).show(|ui| {
        menu_items(app, ui, &[("Group…", "data.group", json!({})), ("Auto Outline", "data.group", json!({"auto": true}))]);
    });
    let ug = abig(ui, Art::Ungroup, "Ungroup", "Ungroup", true);
    control_keytip(app, ui, &ug, "data.ungroup");
    egui::Popup::menu(&ug).show(|ui| {
        menu_items(app, ui, &[("Ungroup…", "data.ungroup", json!({})), ("Clear Outline", "data.ungroup", json!({"clear": true}))]);
    });
    let __k = abig(ui, Art::Subtotal, "Subtotal", "Subtotal", false);
    control_keytip(app, ui, &__k, "dialog:subtotal");
    if __k.clicked() {
        app.open_dialog("subtotal", json!({}));
    }
    stack(ui, |ui| {
        ui.add_space(4.0);
        if icon_only(ui, Art::ShowDetail, "Show Detail", true).clicked() {
            act(app, "data.showDetail", json!({}));
        }
        ui.add_space(8.0);
        if icon_only(ui, Art::HideDetail, "Hide Detail", true).clicked() {
            act(app, "data.hideDetail", json!({}));
        }
    });
    end_group(ui, "Outline", false);
}

// ---------------------------------------------------------------------------------------- Review

pub(super) fn review(app: &mut SheetApp, ui: &mut Ui) {
    let __k = abig(ui, Art::Spelling, "Spelling", "Spelling (F7)", false);
    control_keytip(app, ui, &__k, "dialog:spelling");
    if __k.clicked() {
        app.open_dialog("spelling", json!({}));
    }
    soon_big(app, ui, Art::Thesaurus, "Thesaurus", false);
    let __k = abig(ui, Art::WorkbookStats, "Workbook\nStatistics", "Workbook Statistics", false);
    control_keytip(app, ui, &__k, "statistics");
    if __k.clicked() {
        app.open_dialog("statistics", json!({}));
    }
    end_group(ui, "Proofing", false);
    soon_big(app, ui, Art::CheckPerformance, "Check\nPerformance", false);
    end_group(ui, "Performance", false);
    let __k = abig(ui, Art::Accessibility, "Check\nAccessibility", "Check Accessibility", true);
    control_keytip(app, ui, &__k, "dialog:accessibility");
    if __k.clicked() {
        app.open_dialog("accessibility", json!({}));
    }
    end_group(ui, "Accessibility", false);
    soon_big(app, ui, Art::Translate, "Translate", false);
    end_group(ui, "Language", false);
    off_big(ui, Art::ShowChanges, "Show\nChanges", false);
    end_group(ui, "Changes", false);
    let __k = abig(ui, Art::NewComment, "New\nComment", "New Comment", false);
    control_keytip(app, ui, &__k, "dialog:comment");
    if __k.clicked() {
        app.open_dialog("comment", json!({"threaded": true}));
    }
    let __k = abig(ui, Art::DeleteComment, "Delete", "Delete Comment", false);
    control_keytip(app, ui, &__k, "review.deleteComment");
    if __k.clicked() {
        act(app, "review.deleteComment", json!({}));
    }
    for (a, label, id) in
        [(Art::PreviousComment, "Previous\nComment", "review.previousComment"), (Art::NextComment, "Next\nComment", "review.nextComment")]
    {
        if abig(ui, a, label, label, false).clicked() {
            act(app, id, json!({}));
            app.grid.ensure_visible = true;
        }
    }
    let shown = app.session.comments_pane;
    let r = big(ui, art(Art::ShowComments), "Show\nComments", "Comments pane", false, true, shown);
    if r.clicked() {
        act(app, "review.showComments", json!({"on": !shown}));
        if app.session.comments_pane {
            app.grid.pane = None;
        }
    }
    end_group(ui, "Comments", false);
    let notes = abig(ui, Art::Notes, "Notes", "Notes", true);
    anchor_keytip(app, ui, &notes, &["review.nextComment"]);
    egui::Popup::menu(&notes).show(|ui| {
        menu_items(
            app,
            ui,
            &[
                ("New Note", "dialog:comment", json!({"threaded": false})),
                ("Show/Hide Note", "review.showNote", json!({})),
                ("Show All Notes", "review.showAllNotes", json!({})),
                ("-", "", json!({})),
                ("Convert to Comments", "review.convertToComments", json!({})),
            ],
        );
    });
    end_group(ui, "Notes", false);
    let protected = app.session.active().and_then(|d| d.wb.active().map(|s| s.is_protected())).unwrap_or(false);
    let __k = abig(ui, Art::ProtectSheet, if protected { "Unprotect\nSheet" } else { "Protect\nSheet" }, "Protect Sheet", false);
    control_keytip(app, ui, &__k, "dialog:protectSheet");
    if __k.clicked() {
        app.open_dialog(if protected { "unprotectSheet" } else { "protectSheet" }, json!({}));
    }
    let __k = abig(ui, Art::ProtectWorkbook, "Protect\nWorkbook", "Protect Workbook structure", false);
    control_keytip(app, ui, &__k, "review.protectWorkbook");
    if __k.clicked() {
        crate::backstage::protect_workbook(app);
    }
    if abig(ui, Art::AllowEditRanges, "Allow Edit\nRanges", "Allow Edit Ranges", false).clicked() {
        app.open_dialog("allowEditRanges", json!({}));
    }
    off_big(ui, Art::UnshareWorkbook, "Unshare\nWorkbook", false);
    end_group(ui, "Protect", false);
    soon_big(app, ui, Art::HideInk, "Hide\nInk", true);
    end_group(ui, "Ink", false);
}

// ------------------------------------------------------------------------------------------ View

pub(super) fn view(app: &mut SheetApp, ui: &mut Ui) {
    let lang = app.ui.language;
    // Sheet View: shared sheet views need co-authoring; shown greyed out, as for a local file.
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.add_enabled_ui(false, |ui| {
            egui::ComboBox::from_id_salt("sheet_view").width(150.0).selected_text(lang.tr("Default")).show_ui(ui, |_| {});
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            off_small(ui, Art::Keep, "Keep", false);
            off_small(ui, Art::Exit, "Exit", false);
            off_small(ui, Art::NewView, "New", false);
            off_small(ui, Art::ViewOptions, "Options", false);
        });
    });
    end_group(ui, "Sheet View", false);
    // Workbook Views
    let mode = app.session.active().and_then(|d| d.wb.active().map(|s| s.view)).unwrap_or_default();
    for (a, label, cmd, m) in [
        (Art::Normal, "Normal", "view.normal", gridcraft_engine::model::SheetViewMode::Normal),
        (Art::PageBreakPreview, "Page Break\nPreview", "view.pageBreakPreview", gridcraft_engine::model::SheetViewMode::PageBreakPreview),
        (Art::PageLayoutView, "Page\nLayout", "view.pageLayout", gridcraft_engine::model::SheetViewMode::PageLayout),
    ] {
        let __k = big(ui, art(a), label, label, false, true, mode == m);
        control_keytip(app, ui, &__k, cmd);
        if __k.clicked() {
            act(app, cmd, json!({}));
        }
    }
    soon_big(app, ui, Art::CustomViews, "Custom\nViews", false);
    end_group(ui, "Workbook Views", false);
    // Show
    soon_big(app, ui, Art::Navigation, "Navigation", false);
    let s = app.session.active().and_then(|d| d.wb.active().map(|s| (s.show_gridlines, s.show_headings))).unwrap_or((true, true));
    let page_layout = mode == gridcraft_engine::model::SheetViewMode::PageLayout;
    ui.vertical(|ui| {
        // Rulers exist only in Page Layout view.
        let mut ruler = !app.session.hide_ruler;
        let ruler_resp = ui.add_enabled(page_layout, egui::Checkbox::new(&mut ruler, lang.tr("Ruler")));
        control_keytip(app, ui, &ruler_resp, "view.ruler");
        if ruler_resp.changed() {
            act(app, "view.ruler", json!({"on": ruler}));
        }
        let mut g = s.0;
        let resp = ui.checkbox(&mut g, lang.tr("Gridlines"));
        control_keytip(app, ui, &resp, "view.gridlines");
        if resp.changed() {
            act(app, "view.gridlines", json!({"on": g}));
        }
        let mut fb = app.session.prefs.formula_bar;
        let resp = ui.checkbox(&mut fb, lang.tr("Formula Bar"));
        control_keytip(app, ui, &resp, "view.formulaBar");
        if resp.changed() || invoke(app, "view.formulaBar") {
            act(app, "view.formulaBar", json!({"on": checkbox_on(resp.changed(), fb)}));
        }
    });
    ui.add_space(6.0);
    ui.vertical(|ui| {
        let mut h = s.1;
        let resp = ui.checkbox(&mut h, lang.tr("Headings"));
        control_keytip(app, ui, &resp, "view.headings");
        if resp.changed() {
            act(app, "view.headings", json!({"on": h}));
        }
        let mut icons_on = true;
        ui.add_enabled(false, egui::Checkbox::new(&mut icons_on, lang.tr("Data Type Icons"))).on_disabled_hover_text(SOON);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            soon_small(app, ui, Art::FocusCell, "Focus Cell", true);
        });
    });
    end_group(ui, "Show", false);
    // Zoom
    let __k = abig(ui, Art::Zoom, "Zoom", "Zoom…", false);
    control_keytip(app, ui, &__k, "dialog:zoom");
    if __k.clicked() {
        app.open_dialog("zoom", json!({}));
    }
    let __k = abig(ui, Art::Zoom100, "100%", "Zoom to 100%", false);
    control_keytip(app, ui, &__k, "view.zoom");
    if __k.clicked() || invoke(app, "view.zoom") {
        act(app, "view.zoom", json!({"percent": 100}));
    }
    let __k = abig(ui, Art::ZoomSelection, "Zoom to\nSelection", "Zoom to Selection", false);
    control_keytip(app, ui, &__k, "view.zoomToSelection");
    if __k.clicked() {
        let r = app.grid.cells_rect.unwrap_or(Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 700.0)));
        act(app, "view.zoomToSelection", json!({"viewWidth": r.width(), "viewHeight": r.height()}));
    }
    end_group(ui, "Zoom", false);
    // Window
    soon_big(app, ui, Art::NewWindow, "New\nWindow", false);
    soon_big(app, ui, Art::ArrangeAll, "Arrange\nAll", false);
    let fp = abig(ui, Art::FreezePanes, "Freeze\nPanes", "Freeze Panes", true);
    for sequence in
        keytip_table::group_extension(app.keytips.prefix().unwrap_or_default(), &["view.freezePanes", "view.freezeTopRow", "view.freezeFirstColumn"])
    {
        keytip(app, ui, sequence, fp.rect.center_bottom());
    }
    egui::Popup::menu(&fp).show(|ui| {
        menu_items(
            app,
            ui,
            &[
                ("Freeze Panes", "view.freezePanes", json!({})),
                ("Freeze Top Row", "view.freezeTopRow", json!({})),
                ("Freeze First Column", "view.freezeFirstColumn", json!({})),
                ("Unfreeze Panes", "view.unfreezePanes", json!({})),
            ],
        );
    });
    stack(ui, |ui| {
        soon_small(app, ui, Art::Split, "Split", false);
        soon_small(app, ui, Art::HideWindow, "Hide", false);
        soon_small(app, ui, Art::UnhideWindow, "Unhide", false);
    });
    stack(ui, |ui| {
        off_small(ui, Art::SideBySide, "View Side by Side", false);
        off_small(ui, Art::SyncScroll, "Synchronous Scrolling", false);
        off_small(ui, Art::ResetPosition, "Reset Window Position", false);
    });
    let sw = abig(ui, Art::SwitchWindows, "Switch\nWindows", "Switch Windows", true);
    egui::Popup::menu(&sw).show(|ui| {
        let titles: Vec<String> = app.session.documents().iter().map(|d| d.display_title()).collect();
        let active = app.session.active_index();
        for (i, title) in titles.into_iter().enumerate() {
            if ui.selectable_label(i == active, title).clicked() {
                act(app, "window.activate", json!({"index": i}));
                ui.close();
            }
        }
    });
    end_group(ui, "Window", false);
    // Macros
    if abig(ui, Art::Macros, "Macros", "View Macros (Alt+F8)", true).clicked() {
        app.open_dialog("macros", json!({}));
    }
    end_group(ui, "Macros", false);
}

// -------------------------------------------------------------------------------------- Automate

pub(super) fn automate(app: &mut SheetApp, ui: &mut Ui) {
    soon_big(app, ui, Art::NewScript, "New\nScript", true);
    soon_big(app, ui, Art::ViewScripts, "View\nScripts", true);
    end_group(ui, "Office Scripts", false);
    // The scripts gallery: sample scripts, shown as in the desktop app.
    let t = Tokens::get(ui.ctx());
    const CELL_W: f32 = 236.0;
    let (frame, _) = ui.allocate_exact_size(vec2(CELL_W * 3.0 + 12.0, 68.0), Sense::hover());
    ui.painter().rect_stroke(frame, 4.0, Stroke::new(1.0, t.separator), StrokeKind::Inside);
    let names = [
        "Unhide All Rows and Columns",
        "Remove Hyperlinks from Sheet",
        "Freeze Selection",
        "Count Empty Rows",
        "Make a Subtable from a Selection",
        "Return Table Data as JSON",
    ];
    let font = theme::ui_font(12.5);
    for (i, name) in names.iter().enumerate() {
        let col = (i / 2) as f32;
        let row = (i % 2) as f32;
        let cell = Rect::from_min_size(pos2(frame.left() + 6.0 + col * CELL_W, frame.top() + 6.0 + row * 28.0), vec2(CELL_W - 4.0, 26.0));
        let resp = ui.interact(cell, ui.id().with(("script_gallery", i)), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(cell, 3.0, t.hover);
        }
        paint(ui.painter(), Rect::from_center_size(pos2(cell.left() + 11.0, cell.center().y), vec2(16.0, 16.0)), Art::ScriptItem, t.text);
        // Shortened with an ellipsis to the room it has, as the desktop gallery does.
        let room = cell.width() - 30.0;
        let mut shown = app.ui.language.tr(name).to_string();
        let width = |s: &str| ui.painter().layout_no_wrap(s.to_string(), font.clone(), t.text).size().x;
        if width(&shown) > room {
            while !shown.is_empty() && width(&format!("{shown}…")) > room {
                shown.pop();
            }
            shown = format!("{}…", shown.trim_end());
        }
        ui.painter().text(pos2(cell.left() + 24.0, cell.center().y), Align2::LEFT_CENTER, shown, font.clone(), t.text);
        // The scripts themselves aren't available yet: the gallery is shown, not run.
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, false, *name));
        resp.on_hover_text(SOON);
    }
    end_group(ui, "Office Scripts Gallery", false);
    off_big(ui, Art::AutomationTemplates, "Automation\nTemplates", false);
    end_group(ui, "Power Automate", false);
    // GridCraft's own automation tools.
    let __k = abig(ui, Art::Palette, "Command\nPalette", "Search and run any command", false);
    control_keytip(app, ui, &__k, "commandSearch");
    if __k.clicked() {
        app.open_dialog("commandSearch", json!({}));
    }
    let __k = abig(ui, Art::Agent, "Agent\nControl", "How agents drive GridCraft (MCP / control channel)", false);
    control_keytip(app, ui, &__k, "agents");
    if __k.clicked() {
        app.open_dialog("agents", json!({}));
    }
    let __k = abig(ui, Art::Journal, "Action\nJournal", "Every command run in this session (replayable)", false);
    control_keytip(app, ui, &__k, "journal");
    if __k.clicked() {
        app.open_dialog("journal", json!({}));
    }
    end_group(ui, "GridCraft", false);
}

// ------------------------------------------------------------------------------------- Developer

pub(super) fn developer(app: &mut SheetApp, ui: &mut Ui) {
    soon_big(app, ui, Art::VisualBasic, "Visual\nBasic", false);
    if abig(ui, Art::Macros, "Macros", "View Macros (Alt+F8)", false).clicked() {
        app.open_dialog("macros", json!({}));
    }
    stack(ui, |ui| {
        soon_small(app, ui, Art::RecordMacro, "Record Macro", false);
        soon_small(app, ui, Art::RelativeRefs, "Use Relative References", false);
        soon_small(app, ui, Art::MacroSecurity, "Macro Security", false);
    });
    end_group(ui, "Code", false);
    soon_big(app, ui, Art::AddIns, "Add-\nins", false);
    soon_big(app, ui, Art::ExcelAddIns, "Spreadsheet\nAdd-ins", false);
    soon_big(app, ui, Art::ComAddIns, "COM\nAdd-ins", false);
    end_group(ui, "Add-ins", false);
    soon_big(app, ui, Art::InsertControl, "Insert", true);
    off_big(ui, Art::DesignMode, "Design\nMode", false);
    stack(ui, |ui| {
        soon_small(app, ui, Art::ControlProperties, "Properties", false);
        soon_small(app, ui, Art::ViewCode, "View Code", false);
        soon_small(app, ui, Art::RunDialog, "Run Dialog", false);
    });
    end_group(ui, "Controls", false);
    soon_big(app, ui, Art::XmlSource, "Source", false);
    stack(ui, |ui| {
        off_small(ui, Art::MapProperties, "Map Properties", false);
        soon_small(app, ui, Art::ExpansionPacks, "Expansion Packs", false);
        off_small(ui, Art::RefreshData, "Refresh Data", false);
    });
    stack(ui, |ui| {
        soon_small(app, ui, Art::Import, "Import", false);
        off_small(ui, Art::Export, "Export", false);
    });
    end_group(ui, "XML", false);
}

// ------------------------------------------------------------------------------------------ Help

pub(super) fn help(app: &mut SheetApp, ui: &mut Ui) {
    const REPO: &str = "https://github.com/storytold/gridcraft";
    let open = |app: &mut SheetApp, url: &str| {
        if let Some(open_url) = &app.services.open_url {
            open_url(url);
        }
    };
    if abig(ui, Art::Help, "Help", "Help (F1)", false).clicked() {
        open(app, &format!("{REPO}#readme"));
    }
    if abig(ui, Art::ContactSupport, "Contact\nSupport", "Ask a question on GitHub", false).clicked() {
        open(app, &format!("{REPO}/issues"));
    }
    if abig(ui, Art::Feedback, "Feedback", "Report a problem or suggest an idea", false).clicked() {
        open(app, &format!("{REPO}/issues/new"));
    }
    soon_big(app, ui, Art::ShowTraining, "Show\nTraining", false);
    if abig(ui, Art::WhatsNew, "What's\nNew", "Release notes", false).clicked() {
        open(app, &format!("{REPO}/commits/main"));
    }
    end_group(ui, "Help", false);
    if abig(ui, Art::Community, "Community", "GridCraft on GitHub", false).clicked() {
        open(app, REPO);
    }
    soon_big(app, ui, Art::Blog, "Blog", false);
    end_group(ui, "Community", false);
    soon_big(app, ui, Art::MobileApp, "Get the\nMobile App", false);
    end_group(ui, "Mobile", false);
    if abig(ui, Art::PdfXps, "PDF", "Export as PDF", false).clicked() {
        app.run_or_alert("file.exportPdf", json!({}));
    }
    end_group(ui, "PDF", false);
}
