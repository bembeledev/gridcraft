//! Real input paths for narrow ribbon tabs and viewport-bounded dialogs.

use egui::{Event, Modifiers, MouseWheelSource, MouseWheelUnit, Rect, TouchPhase, accesskit::Role, vec2};
use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness(width: f32, height: f32) -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(width, height)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        SheetApp::new(session, Default::default()),
    );
    h.run_steps(6);
    h
}

fn in_view(h: &Harness, rect: Rect) {
    let view = h.ctx.content_rect().expand(1.0);
    assert!(view.contains_rect(rect), "{rect:?} outside {view:?}");
}

#[test]
fn narrow_tabs_reveal_the_active_tab_and_keep_contextual_tabs_reachable() {
    let mut h = harness(640.0, 480.0);
    h.get_by_role_and_label(Role::Button, "More tabs").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "Automate").click();
    h.run_steps(6);
    assert_eq!(h.state().ui.ribbon_tab, "Automate");
    in_view(&h, h.get_by_role_and_label(Role::RadioButton, "Automate").rect());
    h.state_mut().session.execute("range.setValues", json!({"range":"A1:B2","values":[["Name","Qty"],["a",1]]})).unwrap();
    h.state_mut().session.execute("insert.table", json!({"range":"A1:B2"})).unwrap();
    h.run_steps(4);
    h.get_by_role_and_label(Role::Button, "More tabs").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "Table Design").click();
    h.run_steps(6);
    assert_eq!(h.state().ui.ribbon_tab, "Table Design");
    in_view(&h, h.get_by_role_and_label(Role::RadioButton, "Table Design").rect());

    h.state_mut().run("app.language.set", json!({"language":"pt"})).unwrap();
    h.run_steps(4);
    in_view(&h, h.get_by_role_and_label(Role::Button, "Mais abas").rect());
}

fn toolpak_rect(h: &Harness, tool: &str) -> Rect {
    h.ctx.memory(|memory| memory.area_rect(egui::Id::unique("dialog").with("analysisToolPak").with(tool))).unwrap()
}

#[test]
fn toolpak_actions_stay_fixed_while_the_body_scrolls() {
    for tool in ["", "regression", "descriptive"] {
        let mut h = harness(640.0, 360.0);
        h.state_mut().open_dialog("analysisToolPak", json!({"tool":tool}));
        h.run_steps(6);
        let ok = h.get_by_role_and_label(Role::Button, "   OK   ").rect();
        let cancel = h.get_by_role_and_label(Role::Button, " Cancel ").rect();
        in_view(&h, ok);
        in_view(&h, cancel);
        in_view(&h, toolpak_rect(&h, tool));
        let label = if tool.is_empty() {
            "Descriptive Statistics"
        } else if tool == "regression" {
            "Input Y Range:"
        } else {
            "Input Range:"
        };
        let before = h.get_by_label(label).rect();
        h.input_mut().events.extend([
            Event::PointerMoved(before.center()),
            Event::MouseWheel {
                unit: MouseWheelUnit::Point,
                source: MouseWheelSource::Unknown,
                delta: vec2(0.0, -700.0),
                phase: TouchPhase::Move,
                modifiers: Modifiers::NONE,
            },
        ]);
        h.run_steps(12);
        assert_eq!(h.get_by_role_and_label(Role::Button, "   OK   ").rect(), ok, "{tool}");
        assert_eq!(h.get_by_role_and_label(Role::Button, " Cancel ").rect(), cancel, "{tool}");
        // Prove that scrolling happened, rather than only checking two stationary buttons.
        assert!(h.query_by_label(label).is_none_or(|node| node.rect().top() < before.top() - 1.0), "{tool}: the body did not scroll");
        h.get_by_role_and_label(Role::Button, " Cancel ").click();
        h.run_steps(3);
        assert!(h.state().dialog.is_none());
    }
}

#[test]
fn toolpak_can_use_a_tall_viewport_without_a_fixed_height_cap() {
    let mut h = harness(1200.0, 1000.0);
    h.state_mut().open_dialog("analysisToolPak", json!({"tool":"regression"}));
    h.run_steps(6);
    let rect = toolpak_rect(&h, "regression");
    assert!(rect.height() > 700.0, "{rect:?}");
    in_view(&h, rect);
    in_view(&h, h.get_by_role_and_label(Role::Button, "   OK   ").rect());
}

#[test]
fn dialogs_follow_a_smaller_viewport_and_keep_compact_forms_compact() {
    let mut h = harness(1200.0, 1000.0);
    h.state_mut().open_dialog("columnWidth", json!({}));
    h.run_steps(6);
    let id = egui::Id::unique("dialog").with("columnWidth");
    let compact = h.ctx.memory(|memory| memory.area_rect(id)).unwrap();
    assert!(compact.height() < 250.0, "{compact:?}");
    h.set_size(vec2(320.0, 320.0));
    h.run_steps(8);
    in_view(&h, h.ctx.memory(|memory| memory.area_rect(id)).unwrap());
    in_view(&h, h.get_by_role_and_label(Role::Button, "   OK   ").rect());
    in_view(&h, h.get_by_role_and_label(Role::Button, " Cancel ").rect());
}

#[test]
fn scaled_portuguese_toolpak_keeps_its_actions_in_the_viewport() {
    let mut h = harness(640.0, 480.0);
    h.state_mut().run("app.language.set", json!({"language":"pt"})).unwrap();
    h.state_mut().run("view.interfaceScale", json!({"percent":150})).unwrap();
    h.state_mut().open_dialog("analysisToolPak", json!({"tool":"regression"}));
    h.run_steps(8);
    in_view(&h, h.get_by_role_and_label(Role::Button, "   OK   ").rect());
    in_view(&h, h.get_by_role_and_label(Role::Button, " Cancelar ").rect());
    in_view(&h, toolpak_rect(&h, "regression"));
}

#[test]
fn toolpak_overwrite_actions_stay_visible_in_a_short_viewport() {
    let mut h = harness(360.0, 170.0);
    h.state_mut().open_dialog("analysisToolPak", json!({}));
    let dialog = h.state_mut().dialog.as_mut();
    assert!(dialog.is_some(), "Analysis ToolPak dialog did not open");
    if let Some(dialog) = dialog {
        dialog.values.insert("askOverwrite".into(), json!(true));
    }
    h.run_steps(6);

    in_view(&h, h.get_by_role_and_label(Role::Button, "Replace").rect());
    in_view(&h, h.get_by_role_and_label(Role::Button, "Cancel").rect());
}
