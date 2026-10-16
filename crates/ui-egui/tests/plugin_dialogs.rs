//! Protocol 1.2 in the interface: a plugin's ribbon button returns a dialog, the app draws it, and
//! its OK runs the next plugin command, whose result can bring another dialog. The plugin is a
//! small shell script (Unix only).
#![cfg(unix)]

use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

const SCRIPT: &str = r#"#!/bin/sh
# Requests arrive as compact JSON with sorted keys, one per line.
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/^{[^{]*"id":\([0-9][0-9]*\).*/\1/p')
  if [ -z "$id" ]; then exit 0; fi
  case "$line" in
    *'"method":"initialize"'*) result='{"protocolVersion":1}' ;;
    *'"command":"pick"'*) result='{"dialog":{"title":"Pick one","kind":"list","items":[{"id":"a","label":"Alpha","detail":"Folder","icon":"folder"},{"id":"b","label":"Beta","detail":"12 KB","icon":"workbook"}],"command":"next","params":{"step":2},"okLabel":"Choose"}}' ;;
    *'"command":"next"'*)
      item=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
      result="{\"commands\":[{\"command\":\"cell.set\",\"params\":{\"cell\":\"A1\",\"input\":\"$item\"}}],\"dialog\":{\"kind\":\"message\",\"title\":\"Done\",\"prompt\":\"Picked $item\"},\"message\":\"Recorded $item\",\"result\":{\"ok\":true}}" ;;
    *) result='{}' ;;
  esac
  printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$id" "$result"
done
"#;

fn load_plugin() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("gridcraft-ui-plugin-dialogs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("plugin.sh");
    std::fs::write(&script, SCRIPT).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let manifest = json!({
        "id": "org.gridcraft.test-dialogs", "name": "Dialogs", "run": {"command": "./plugin.sh"},
        "commands": [{"id": "pick", "label": "Pick File"}, {"id": "next", "label": "Next"}],
        "ui": {"ribbon": [{"tab": "Dialogs", "group": "Files", "buttons": [{"command": "pick", "icon": "folder"}]}]}
    });
    std::fs::write(dir.join("plugin.json"), manifest.to_string()).unwrap();
    gridcraft_engine::plugin::load(&dir).unwrap();
}

#[test]
fn a_plugin_dialog_shows_and_its_ok_runs_the_next_command() {
    load_plugin();
    let mut s = Session::new();
    s.new_workbook();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(s, Default::default()),
    );
    h.run_steps(4);
    h.state_mut().run_or_alert("ui.ribbonTab", json!({"tab": "Dialogs"}));
    h.run_steps(3);

    // The ribbon button runs the plugin command; its result's dialog is drawn by the app.
    h.get_by_label("Pick File").click();
    h.run_steps(3);
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().plugin_dialog.as_ref().map(|d| d.dialog.title.clone()), Some("Pick one".into()));
    h.get_by_label("Alpha");

    // Pick an item and press the dialog's OK ("Choose"): the next command runs with its id.
    h.get_by_label("Beta").click();
    h.run_steps(3);
    assert_eq!(h.state().plugin_dialog.as_ref().map(|d| d.selection()), Some(vec!["b".to_string()]));
    h.get_by_label("Choose").click();
    h.run_steps(4);
    let a1 = h.state().session.active().unwrap().wb.active().unwrap().value(CellRef::new(0, 0));
    assert_eq!(a1, Value::Text("b".into()));

    // Its result combines a message (the status bar, so it doesn't cover the dialog) and another dialog.
    assert_eq!(h.state().message, None);
    assert_eq!(h.state().status_message.as_ref().map(|(m, _)| m.clone()), Some("Recorded b".into()));
    // The chained dialog is a message, which OK closes.
    assert_eq!(h.state().plugin_dialog.as_ref().map(|d| d.dialog.prompt.clone()), Some("Picked b".into()));
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(h.state().plugin_dialog.is_none());

    // A programmatic run returns the dialog and shows nothing.
    let r = h.state_mut().run("plugins.run", json!({"plugin": "org.gridcraft.test-dialogs", "command": "pick"})).unwrap();
    assert_eq!(r["dialog"]["title"], "Pick one");
    h.run_steps(2);
    assert!(h.state().plugin_dialog.is_none());
}

#[test]
fn a_multiline_code_field_keeps_its_lines() {
    let mut s = Session::new();
    s.new_workbook();
    let mut app = SheetApp::new(s, Default::default());
    let dialog = gridcraft_engine::plugin::dialog::Dialog::parse(&json!({
        "kind": "form", "title": "Run R", "command": "next",
        "fields": [{"id": "code", "label": "R code", "value": "x <- 1", "multiline": true, "code": true, "rows": 6}]
    }))
    .unwrap();
    app.plugin_dialog = Some(gridcraft_ui_egui::plugin_ui::DialogState::new("org.gridcraft.test-dialogs", dialog));
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    // The field is a multi-line editor: Enter adds a line instead of pressing OK.
    // The field is drawn as a multi-line text box.
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "R code").click();
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "R code").type_text("\ny <- 2");
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(1);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "R code").type_text("x + y");
    h.run_steps(3);
    let st = h.state().plugin_dialog.as_ref().expect("the dialog is still up");
    assert_eq!(st.fields()["code"], json!("x <- 1\ny <- 2\nx + y"));
    // OK sends the lines as they are.
    assert_eq!(st.ok_run().unwrap()["params"]["fields"]["code"], json!("x <- 1\ny <- 2\nx + y"));
}
