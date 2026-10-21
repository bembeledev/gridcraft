//! The ribbon in the desktop layout: every tab draws its groups, commands GridCraft doesn't have
//! yet are greyed out (not clickable), and the ribbon's right-click menu hides the Quick Access
//! Toolbar and collapses the ribbon.

use egui::{Event, Modifiers, PointerButton};
use egui_kittest::kittest::{NodeT, Queryable};
use gridcraft_engine::Session;
use gridcraft_ui_egui::{SheetApp, ribbon::TABS};
use serde_json::json;

type H = egui_kittest::Harness<'static, SheetApp>;

fn harness() -> H {
    let mut s = Session::new();
    s.new_workbook();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1600.0, 900.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(s, Default::default()),
    );
    h.run_steps(4);
    h
}

fn tab(h: &mut H, name: &str) {
    h.state_mut().ui.ribbon_tab = name.into();
    h.run_steps(3);
}

#[test]
fn every_tab_draws_its_commands() {
    let mut h = harness();
    // One command from each tab, as labelled on it.
    for (name, label) in [
        ("Home", "Format Painter"),
        ("Insert", "PivotTable"),
        ("Draw", "Eraser"),
        ("Page Layout", "Margins"),
        ("Formulas", "Insert Function"),
        ("Data", "Filter"),
        ("Review", "Spelling"),
        ("View", "Freeze Panes"),
        ("Automate", "Command Palette"),
        ("Developer", "Macros"),
        // "Help" also names the tab itself, so ask for a command only the tab's body has.
        ("Help", "Contact Support"),
    ] {
        assert!(TABS.contains(&name), "{name}");
        tab(&mut h, name);
        assert!(h.query_all_by_label(label).next().is_some(), "{name} tab shows {label}");
    }
}

#[test]
fn commands_gridcraft_lacks_are_greyed_out_and_do_nothing() {
    let mut h = harness();
    tab(&mut h, "Developer");
    let vb = h.get_by_label("Visual Basic");
    assert!(vb.accesskit_node().is_disabled(), "not available yet");
    vb.click();
    h.run_steps(2);
    assert!(h.state().dialog.is_none() && h.state().toast.is_none(), "a greyed-out button does nothing");
    // Macros is available.
    assert!(!h.get_by_label("Macros").accesskit_node().is_disabled());
}

#[test]
fn the_ribbon_menu_hides_the_quick_access_toolbar_and_collapses_the_ribbon() {
    let mut h = harness();
    // An empty spot on the ribbon's tab row, just right of the last tab (Paste has a menu of its own).
    let last_tab = h.get_all_by_label("Help").map(|n| n.rect()).min_by(|a, b| a.top().total_cmp(&b.top())).expect("the Help tab");
    let at = egui::pos2(last_tab.right() + 40.0, last_tab.center().y);
    let right_click = |h: &mut H| {
        h.input_mut().events.extend([
            Event::PointerMoved(at),
            Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE },
            Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE },
        ]);
        h.run_steps(3);
    };
    right_click(&mut h);
    h.get_by_label("Hide Quick Access Toolbar").click();
    h.run_steps(3);
    assert!(h.state().ui.qat_hidden);
    right_click(&mut h);
    h.get_by_label("Collapse the Ribbon").click();
    h.run_steps(3);
    assert!(h.state().ui.ribbon_collapsed);
    h.state_mut().run("ui.ribbonTab", json!({"tab": "Home"})).ok();
}

#[test]
fn icon_only_pens_are_named_apart() {
    let mut h = harness();
    tab(&mut h, "Draw");
    for pen in ["Pen: Black, 0.5 mm", "Pen: Red, 0.5 mm", "Pen: Blue, 1 mm", "Pen: Green, 1 mm"] {
        assert_eq!(h.query_all_by_label(pen).count(), 1, "{pen}");
    }
}
