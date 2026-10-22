//! App-owned menu model for the shared Linux exporter. Interactive actions reuse the same
//! dialog, clipboard, print and command paths as local controls; no workbook logic lives here.

use craft_desktop::{Action, Item, Menu};
use serde_json::{Value, json};

use crate::SheetApp;

#[derive(Clone, Copy)]
enum ClipboardKind {
    Copy,
    Cut,
    Paste,
}

pub(crate) struct ClipboardRequest {
    frame: u64,
    document: u64,
    kind: ClipboardKind,
}

/// Revalidate delayed toolkit events before cell/formula/text-box editors can consume them.
pub(crate) fn guard_clipboard(app: &mut SheetApp, ctx: &egui::Context) {
    let current = document(app);
    let modal = blocked(app);
    for request in &app.desktop_clipboard {
        if current != request.document || modal {
            ctx.input_mut(|input| {
                input.events.retain(|event| {
                    !matches!(
                        (request.kind, event),
                        (ClipboardKind::Copy, egui::Event::Copy)
                            | (ClipboardKind::Cut, egui::Event::Cut)
                            | (ClipboardKind::Paste, egui::Event::Paste(_))
                    )
                });
            });
        }
    }
    // Request* is processed after this frame's UI and delivered on the next frame.
    app.desktop_clipboard.retain(|request| ctx.cumulative_frame_nr() <= request.frame);
}

fn document(app: &SheetApp) -> u64 {
    app.session.active().map_or(0, |d| d.uid)
}
fn blocked(app: &SheetApp) -> bool {
    app.dialog.is_some() || app.message.is_some() || app.printing.busy()
}

fn checked(app: &SheetApp, id: &str) -> Option<bool> {
    let doc = app.session.active();
    let sheet = doc.and_then(|d| d.wb.active());
    let style = doc.zip(sheet).map(|(d, s)| d.wb.styles.get(s.style_id(d.selection.active)));
    match id {
        "home.bold" => style.map(|s| s.font.bold),
        "home.italic" => style.map(|s| s.font.italic),
        "home.underline" => style.map(|s| s.font.underline != gridcraft_engine::model::Underline::None),
        "view.gridlines" => sheet.map(|s| s.show_gridlines),
        "view.headings" => sheet.map(|s| s.show_headings),
        "view.normal" => sheet.map(|s| s.view == gridcraft_engine::model::SheetViewMode::Normal),
        "view.pageLayout" => sheet.map(|s| s.view == gridcraft_engine::model::SheetViewMode::PageLayout),
        "view.pageBreakPreview" => sheet.map(|s| s.view == gridcraft_engine::model::SheetViewMode::PageBreakPreview),
        "view.formulaBar" => Some(app.session.prefs.formula_bar),
        "view.collapseRibbon" => Some(app.ui.ribbon_collapsed),
        _ => None,
    }
}

fn entry(app: &SheetApp, label: &str, id: &str) -> Option<Item> {
    let dialog = id.strip_prefix("dialog:");
    let spec = gridcraft_engine::find_command(id);
    let ui_command = matches!(id, "view.collapseRibbon");
    if spec.is_none() && dialog.is_none() && !ui_command {
        return None;
    }
    let no_document_needed = matches!(id, "file.new" | "file.open" | "dialog:about" | "dialog:start" | "dialog:commandSearch");
    let enabled = !blocked(app)
        && (no_document_needed || app.session.active().is_some())
        && (id == "edit.paste" || spec.is_none_or(|c| (c.enabled)(&app.session).is_ok()));
    let action = match dialog {
        Some(name) => Action { command: "ui.dialog".into(), params: json!({"name": name}) },
        None => Action { command: id.into(), params: json!({}) },
    };
    Some(Item {
        key: id.into(),
        label: app.ui.language.tr(label).into(),
        enabled,
        checked: checked(app, id),
        shortcut: spec.and_then(|c| c.shortcut).unwrap_or_default().replace("Cmd", "Ctrl").replace("Mod", "Ctrl"),
        action: Some(action),
        ..Default::default()
    })
}

fn section(app: &SheetApp, name: &str, entries: &[(&str, &str)]) -> Item {
    Item {
        key: name.into(),
        label: app.ui.language.tr(name).into(),
        enabled: true,
        children: entries.iter().filter_map(|(label, id)| entry(app, label, id)).collect(),
        ..Default::default()
    }
}

pub fn snapshot(app: &SheetApp) -> Menu {
    let mut items = vec![
        section(
            app,
            "File",
            &[
                ("New Workbook", "file.new"),
                ("Open…", "file.open"),
                ("Save", "file.save"),
                ("Save As…", "file.saveAs"),
                ("Export as PDF…", "file.exportPdf"),
                ("Page Setup…", "dialog:pageSetup"),
                ("Print…", "file.print"),
                ("Close", "file.close"),
            ],
        ),
        section(
            app,
            "Edit",
            &[
                ("Undo", "edit.undo"),
                ("Redo", "edit.redo"),
                ("Cut", "edit.cut"),
                ("Copy", "edit.copy"),
                ("Paste", "edit.paste"),
                ("Paste Special…", "dialog:pasteSpecial"),
                ("Fill Down", "edit.fillDown"),
                ("Fill Right", "edit.fillRight"),
                ("Clear Contents", "edit.clearContents"),
                ("Find…", "dialog:find"),
                ("Go To…", "dialog:goTo"),
            ],
        ),
        section(
            app,
            "Insert",
            &[
                ("Cells…", "dialog:insertCells"),
                ("Rows", "home.insertRows"),
                ("Columns", "home.insertColumns"),
                ("Sheet", "home.insertSheet"),
                ("Chart…", "dialog:recommendedCharts"),
                ("PivotTable", "insert.pivotTable"),
                ("Table", "insert.table"),
                ("Function…", "dialog:insertFunction"),
                ("Picture…", "dialog:insertPicture"),
                ("Link…", "dialog:insertLink"),
            ],
        ),
        section(
            app,
            "Format",
            &[
                ("Bold", "home.bold"),
                ("Italic", "home.italic"),
                ("Underline", "home.underline"),
                ("Cells…", "dialog:formatCells"),
                ("Row Height…", "dialog:rowHeight"),
                ("Column Width…", "dialog:columnWidth"),
                ("AutoFit Row Height", "home.autofitRowHeight"),
                ("AutoFit Column Width", "home.autofitColumnWidth"),
                ("Conditional Formatting…", "dialog:manageRules"),
            ],
        ),
        section(
            app,
            "Data",
            &[
                ("Sort…", "dialog:sort"),
                ("Sort A to Z", "data.sortAscending"),
                ("Sort Z to A", "data.sortDescending"),
                ("Filter", "data.filter"),
                ("Clear Filter", "data.clearFilter"),
                ("Text to Columns…", "dialog:textToColumns"),
                ("Remove Duplicates…", "dialog:removeDuplicates"),
                ("Data Validation…", "dialog:dataValidation"),
                ("Calculate Now", "formulas.calculateNow"),
            ],
        ),
        section(
            app,
            "View",
            &[
                ("Normal", "view.normal"),
                ("Page Layout", "view.pageLayout"),
                ("Page Break Preview", "view.pageBreakPreview"),
                ("Formula Bar", "view.formulaBar"),
                ("Gridlines", "view.gridlines"),
                ("Headings", "view.headings"),
                ("Zoom…", "dialog:zoom"),
                ("Freeze Panes", "view.freezePanes"),
                ("Unfreeze Panes", "view.unfreezePanes"),
                ("Collapse Ribbon", "view.collapseRibbon"),
            ],
        ),
        section(
            app,
            "Tools",
            &[
                ("Spelling…", "dialog:spelling"),
                ("Workbook Statistics", "dialog:statistics"),
                ("Protect Sheet…", "dialog:protectSheet"),
                ("Goal Seek…", "dialog:goalSeek"),
                ("Command Palette…", "dialog:commandSearch"),
            ],
        ),
        section(app, "Help", &[("About GridCraft", "dialog:about")]),
    ];
    if let Some(view) = items.iter_mut().find(|i| i.key == "View") {
        view.children.push(Item {
            key: "appearance".into(),
            label: app.ui.language.tr("Display Theme").into(),
            enabled: !blocked(app),
            children: [("system", "System"), ("light", "Light"), ("dark", "Dark")]
                .into_iter()
                .map(|(mode, label)| Item {
                    key: mode.into(),
                    label: app.ui.language.tr(label).into(),
                    enabled: !blocked(app),
                    checked: Some(app.ui.theme_mode() == mode),
                    action: Some(Action { command: "view.theme".into(), params: json!({"mode": mode}) }),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        });
    }
    Menu { document: document(app), items }
}

fn available(items: &[Item], action: &Action) -> bool {
    items.iter().any(|i| i.enabled && (i.action.as_ref() == Some(action) || available(&i.children, action)))
}

pub fn activate(app: &mut SheetApp, ctx: &egui::Context, doc: u64, action: &Action) {
    if doc != document(app) || blocked(app) || !available(&snapshot(app).items, action) {
        return;
    }
    let id = action.command.as_str();
    if matches!(id, "edit.copy" | "edit.cut" | "edit.paste") && (app.editor.is_some() || app.text_box_editor.is_some()) {
        let (kind, command) = match id {
            "edit.copy" => (ClipboardKind::Copy, egui::ViewportCommand::RequestCopy),
            "edit.cut" => (ClipboardKind::Cut, egui::ViewportCommand::RequestCut),
            _ => (ClipboardKind::Paste, egui::ViewportCommand::RequestPaste),
        };
        app.desktop_clipboard.push(ClipboardRequest { frame: ctx.cumulative_frame_nr(), document: doc, kind });
        ctx.send_viewport_cmd(command);
        ctx.request_repaint();
        return;
    }
    if !app.commit_edit(0, 0, false, false) {
        return;
    }
    match id {
        "ui.dialog" => {
            if let Some(name) = action.params.get("name").and_then(Value::as_str) {
                app.open_dialog(name, json!({}));
            }
        }
        "edit.copy" | "edit.cut" => app.copy_to_clipboard(ctx, id),
        "edit.paste" => app.paste_from_clipboard(),
        _ => app.run_or_alert(id, action.params.clone()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    fn app() -> SheetApp {
        let mut session = gridcraft_engine::Session::new();
        session.new_workbook();
        SheetApp::new(session, crate::Services::default())
    }
    fn find<'a>(items: &'a [Item], key: &str) -> Option<&'a Item> {
        items.iter().find_map(|i| if i.key == key { Some(i) } else { find(&i.children, key) })
    }
    #[test]
    fn desktop_menu_format_states_and_modal_guard() {
        let mut app = app();
        let menu = snapshot(&app);
        let bold = find(&menu.items, "home.bold").unwrap().action.clone().unwrap();
        activate(&mut app, &egui::Context::default(), menu.document, &bold);
        assert_eq!(find(&snapshot(&app).items, "home.bold").unwrap().checked, Some(true));
        assert!(find(&snapshot(&app).items, "edit.undo").unwrap().enabled);
        app.open_dialog("about", json!({}));
        activate(&mut app, &egui::Context::default(), menu.document, &bold);
        assert_eq!(find(&snapshot(&app).items, "home.bold").unwrap().checked, Some(true));
        assert!(!find(&snapshot(&app).items, "home.bold").unwrap().enabled);
    }
    #[test]
    fn desktop_menu_translation_and_document_replacement_keep_actions_safe() {
        let mut app = app();
        let menu = snapshot(&app);
        let bold = find(&menu.items, "home.bold").unwrap().action.clone().unwrap();
        app.ui.language = crate::i18n::Language::Ru;
        assert_eq!(find(&snapshot(&app).items, "home.bold").unwrap().action.as_ref(), Some(&bold));
        app.session.new_workbook();
        activate(&mut app, &egui::Context::default(), menu.document, &bold);
        assert_eq!(find(&snapshot(&app).items, "home.bold").unwrap().checked, Some(false));
    }
    #[test]
    fn desktop_menu_dialog_actions_use_interactive_paths() {
        let mut app = app();
        let menu = snapshot(&app);
        let action = find(&menu.items, "dialog:formatCells").unwrap().action.clone().unwrap();
        activate(&mut app, &egui::Context::default(), menu.document, &action);
        assert!(app.dialog.is_some());
    }

    #[test]
    fn desktop_menu_close_keeps_unsaved_work_until_confirmation() {
        let mut app = app();
        app.run("home.bold", json!({})).unwrap();
        let menu = snapshot(&app);
        let close = find(&menu.items, "file.close").unwrap().action.clone().unwrap();
        activate(&mut app, &egui::Context::default(), menu.document, &close);
        assert!(app.session.active().is_some());
        assert!(app.dialog.is_some());
    }

    #[test]
    fn desktop_menu_deferred_clipboard_rejects_replacement_documents_not_keyboard_events() {
        for (command, event) in
            [("edit.copy", egui::Event::Copy), ("edit.cut", egui::Event::Cut), ("edit.paste", egui::Event::Paste("old workbook".into()))]
        {
            let mut h = egui_kittest::Harness::builder().build_ui_state(
                |ui, (app, received): &mut (SheetApp, Vec<egui::Event>)| {
                    app.logic(ui.ctx());
                    received.extend(ui.input(|i| {
                        i.events
                            .iter()
                            .filter(|e| matches!(e, egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_)))
                            .cloned()
                            .collect::<Vec<_>>()
                    }));
                },
                (app(), Vec::new()),
            );
            h.run_steps(2);
            h.state_mut().0.begin_edit(Some("draft".into()), false);
            let ctx = h.ctx.clone();
            let doc = document(&h.state().0);
            activate(&mut h.state_mut().0, &ctx, doc, &Action { command: command.into(), params: json!({}) });
            assert_eq!(h.state().0.desktop_clipboard.len(), 1);
            h.state_mut().0.run("file.new", json!({})).unwrap();
            h.event(event.clone());
            h.run_steps(3);
            assert!(h.state().1.is_empty(), "native {command} belongs to the previous workbook");
            h.event(event);
            h.run_steps(2);
            assert_eq!(h.state().1.len(), 1, "ordinary keyboard {command} remains available");
        }
    }
}
