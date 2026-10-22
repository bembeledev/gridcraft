//! The context menu of a chart (or another object floating over the cells): its Bring to Front
//! and Send to Back submenus, through real input.

use egui::{Event, Key, Modifiers, PointerButton, accesskit::Role, vec2};
use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_ui_egui::{SheetApp, grid::Geo};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn fixture() -> (Harness, u32) {
    let mut session = Session::new();
    session.new_workbook();
    session.execute("cell.set", json!({"cell": "A1", "input": "1"})).unwrap();
    session.execute("cell.set", json!({"cell": "A2", "input": "2"})).unwrap();
    session.execute("selection.set", json!({"range": "A1:A2"})).unwrap();
    let chart = session.execute("insert.chart", json!({"type": "column", "at": "D3", "width": 300, "height": 200})).unwrap();
    let id = chart["chart"].as_u64().unwrap() as u32;
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(session, Default::default()),
    );
    h.run_steps(4);
    (h, id)
}

fn geo(h: &Harness) -> Geo {
    let app = h.state();
    let sh = app.session.doc().unwrap().wb.active().unwrap();
    Geo::new(sh, app.grid.rect.unwrap(), app.view().scroll)
}

/// A point inside the chart, `(fx, fy)` of the way across and down it.
fn inside_chart(h: &Harness, id: u32, fx: f32, fy: f32) -> egui::Pos2 {
    let geo = geo(h);
    let sh = h.state().session.doc().unwrap().wb.active().unwrap();
    let a = sh.charts.iter().find(|c| c.id == id).unwrap().anchor;
    geo.cell_rect(sh, a.cell).min + vec2(a.dx + a.width * fx, a.dy + a.height * fy) * geo.z
}

fn click(h: &mut Harness, pos: egui::Pos2, button: PointerButton) {
    h.input_mut().events.push(Event::PointerMoved(pos));
    h.run_steps(2);
    for pressed in [true, false] {
        h.input_mut().events.push(Event::PointerButton { pos, button, pressed, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }
}

/// Clicks the object menu's item, not a ribbon button with the same label.
fn click_menu_item(h: &mut Harness, label: &str) {
    let (at, _, _) = h.state().grid.object_menu.expect("the object menu is open");
    let item = h.get_all_by_role_and_label(Role::Button, label).find(|n| n.rect().top() >= at.y && n.rect().left() >= at.x - 1.0).unwrap();
    item.click();
    h.run_steps(3);
}

/// Hovers the object menu's `submenu` item, then clicks `item` in the submenu that opens beside it.
fn click_submenu_item(h: &mut Harness, submenu: &str, item: &str) {
    let (at, _, _) = h.state().grid.object_menu.expect("the object menu is open");
    let button = h.get_all_by_label_contains(submenu).find(|n| n.rect().top() >= at.y && n.rect().left() >= at.x - 1.0).unwrap().rect();
    h.input_mut().events.push(Event::PointerMoved(button.center()));
    h.run_steps(3);
    let target = h.get_all_by_role_and_label(Role::Button, item).find(|n| n.rect().left() > button.left() + 20.0).expect("the submenu opened").rect();
    // Along the button's row, then onto the item, as a pointer travels into a submenu.
    h.input_mut().events.push(Event::PointerMoved(egui::pos2(target.left() + 4.0, button.center().y)));
    h.run_steps(2);
    click(h, target.center(), PointerButton::Primary);
    h.run_steps(2);
}

fn chart_order(h: &Harness) -> Vec<u32> {
    h.state().session.doc().unwrap().wb.active().unwrap().charts.iter().map(|c| c.id).collect()
}

#[test]
fn bring_to_front_and_send_to_back_submenus_move_the_right_clicked_chart() {
    let (mut h, first) = fixture();
    let mut ids = vec![first];
    for at in ["E4", "F5"] {
        let r = h.state_mut().run("insert.chart", json!({"type": "line", "at": at, "width": 300, "height": 200})).unwrap();
        ids.push(r["chart"].as_u64().unwrap() as u32);
    }
    h.run_steps(2);
    let (second, third) = (ids[1], ids[2]);
    // Where all three overlap, the chart inserted last is on top and takes the click.
    let at = inside_chart(&h, first, 0.95, 0.95);
    let right_click_top = |h: &mut Harness| {
        click(h, at, PointerButton::Secondary);
        h.state().grid.object_menu.map(|(_, _, id)| id)
    };
    assert_eq!(right_click_top(&mut h), Some(third));
    click_submenu_item(&mut h, "Send to Back", "Send Backward");
    assert_eq!(chart_order(&h), vec![first, third, second], "Send Backward moves one step");
    assert!(h.state().grid.object_menu.is_none(), "choosing a submenu item closes the menu");

    assert_eq!(right_click_top(&mut h), Some(second));
    click_submenu_item(&mut h, "Send to Back", "Send to Back");
    assert_eq!(chart_order(&h), vec![second, first, third]);

    assert_eq!(right_click_top(&mut h), Some(third));
    click_submenu_item(&mut h, "Send to Back", "Send to Back");
    assert_eq!(chart_order(&h), vec![third, second, first]);

    // The bottom chart's corner that no other chart covers.
    let corner = inside_chart(&h, third, 0.98, 0.98);
    click(&mut h, corner, PointerButton::Secondary);
    assert_eq!(h.state().grid.object_menu.map(|(_, _, id)| id), Some(third));
    click_submenu_item(&mut h, "Bring to Front", "Bring Forward");
    assert_eq!(chart_order(&h), vec![second, third, first], "Bring Forward moves one step");

    click(&mut h, corner, PointerButton::Secondary);
    click_submenu_item(&mut h, "Bring to Front", "Bring to Front");
    assert_eq!(chart_order(&h), vec![second, first, third]);
}

#[test]
fn right_clicking_another_chart_while_the_menu_is_open_opens_its_menu() {
    let (mut h, first) = fixture();
    let r = h.state_mut().run("insert.chart", json!({"type": "line", "at": "L3", "width": 200, "height": 150})).unwrap();
    let second = r["chart"].as_u64().unwrap() as u32;
    h.run_steps(2);
    let at = inside_chart(&h, first, 0.5, 0.5);
    click(&mut h, at, PointerButton::Secondary);
    let at = inside_chart(&h, second, 0.5, 0.5);
    click(&mut h, at, PointerButton::Secondary);
    assert_eq!(h.state().grid.object_menu.map(|(_, _, id)| id), Some(second));
    assert_eq!(h.state().selected_chart, Some(second));
}

#[test]
fn escape_closes_the_chart_menu() {
    let (mut h, id) = fixture();
    let at = inside_chart(&h, id, 0.5, 0.5);
    click(&mut h, at, PointerButton::Secondary);
    assert!(h.state().grid.object_menu.is_some());
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(h.state().grid.object_menu.is_none());
}

#[test]
fn the_arrange_items_say_they_reorder_within_the_objects_kind() {
    let (mut h, id) = fixture();
    h.ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
    let tip = "Moves it among objects of the same kind: shapes are drawn under pictures, pictures under charts.";
    for (submenu, item) in [("Bring to Front", "Bring Forward"), ("Send to Back", "Send Backward")] {
        let at = inside_chart(&h, id, 0.5, 0.5);
        click(&mut h, at, PointerButton::Secondary);
        let (at, _, _) = h.state().grid.object_menu.expect("the chart's menu opened");
        let button = h.get_all_by_label_contains(submenu).find(|n| n.rect().top() >= at.y && n.rect().left() >= at.x - 1.0).unwrap().rect();
        h.input_mut().events.push(Event::PointerMoved(button.center()));
        h.run_steps(3);
        let target = h.get_all_by_role_and_label(Role::Button, item).next().expect("the submenu opened").rect();
        // Along the button's row, then onto the item, as a pointer travels into a submenu.
        h.input_mut().events.push(Event::PointerMoved(egui::pos2(target.left() + 4.0, button.center().y)));
        h.run_steps(2);
        h.input_mut().events.push(Event::PointerMoved(target.center()));
        h.run_steps(6);
        assert!(h.query_all_by_value(tip).next().is_some() || h.query_all_by_label(tip).next().is_some(), "{item} explains its kind-local order");
        h.key_press(Key::Escape);
        h.run_steps(2);
    }
}

#[test]
fn change_chart_type_from_the_menu_changes_the_right_clicked_chart() {
    let (mut h, id) = fixture();
    let at = inside_chart(&h, id, 0.5, 0.5);
    click(&mut h, at, PointerButton::Secondary);
    click_menu_item(&mut h, "Change Chart Type…");
    let d = h.state().dialog.as_ref().expect("Change Chart Type opened");
    assert_eq!((d.name(), d.title.as_str()), ("changeChartType", "Change Chart Type"));
    assert_eq!((d.tab.as_str(), d.values["type"].as_str()), ("All Charts", Some("column")), "opens on the chart's own type");

    h.get_by_label("Line with Markers").click();
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert!(h.state().dialog.is_none());
    let kind = |h: &Harness| h.state().session.doc().unwrap().wb.active().unwrap().charts.iter().find(|c| c.id == id).unwrap().kind;
    assert_eq!(kind(&h), gridcraft_engine::model::ChartKind::LineMarkers);
    assert_eq!(h.state().session.doc().unwrap().wb.active().unwrap().charts.len(), 1, "no chart is inserted");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(kind(&h), gridcraft_engine::model::ChartKind::ColumnClustered, "one undo step");
}

#[test]
fn change_chart_type_keeps_a_kind_the_list_does_not_have() {
    let (mut h, id) = fixture();
    h.state_mut().run("chart.changeType", json!({"chart": id, "type": "bubble"})).unwrap();
    h.run_steps(2);
    let at = inside_chart(&h, id, 0.5, 0.5);
    click(&mut h, at, PointerButton::Secondary);
    click_menu_item(&mut h, "Change Chart Type…");
    let d = h.state().dialog.as_ref().expect("Change Chart Type opened");
    assert!(d.values.get("type").is_none(), "no type is picked for a Bubble chart");
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert!(h.state().dialog.is_none(), "OK closes the dialog");
    let kind = h.state().session.doc().unwrap().wb.active().unwrap().charts.iter().find(|c| c.id == id).unwrap().kind;
    assert_eq!(kind, gridcraft_engine::model::ChartKind::Bubble, "OK without a pick keeps the chart's type");
}
