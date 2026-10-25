//! The PivotTable Fields pane drag & drop matches Excel: dragging within an area reorders the
//! field, a drop outside the areas removes it, and right-clicking a field (assigned or not)
//! opens the area menu.

use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::kittest::{NodeT, Queryable};
use gridcraft_engine::Session;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness() -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    session
        .execute(
            "range.setValues",
            json!({"range": "A1", "values": [["Region", "Product", "Sales", "Units"], ["East", "Tea", 100, 10], ["West", "Coffee", 80, 8]]}),
        )
        .unwrap();
    session
        .execute(
            "insert.pivotTable",
            json!({"source": "Sheet1!A1:D3", "destination": "Sheet1!F3", "rows": ["Region", "Product"], "values": ["Sales"]}),
        )
        .unwrap();
    session.execute("selection.set", json!({"cell": "F3"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        SheetApp::new(session, Default::default()),
    );
    h.run_steps(4);
    h
}

fn frame(h: &mut Harness, event: Event) {
    h.input_mut().events.push(event);
    h.run_steps(1);
}

fn right_click(h: &mut Harness, pos: Pos2) {
    frame(h, Event::PointerMoved(pos));
    frame(h, Event::PointerButton { pos, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    frame(h, Event::PointerButton { pos, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

fn drag(h: &mut Harness, from: Pos2, to: Pos2) {
    frame(h, Event::PointerMoved(from));
    frame(h, Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    let n = 6;
    for s in 1..=n {
        frame(h, Event::PointerMoved(from + (to - from) * (s as f32 / n as f32)));
    }
    frame(h, Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(4);
}

/// The report's row fields, in order (queried through the engine, `pivot.fields`).
fn rows(h: &mut Harness) -> Vec<String> {
    let info = h.state_mut().session.run("pivot.fields", json!({})).unwrap();
    info["rows"].as_array().unwrap().iter().filter_map(|f| f["field"].as_str().map(str::to_string)).collect()
}

#[test]
fn dragging_a_field_within_an_area_reorders_it() {
    let mut h = harness();
    assert_eq!(rows(&mut h), vec!["Region".to_string(), "Product".to_string()]);
    let product = h.get_by_label("Product ▾").rect().center();
    let region = h.get_by_label("Region ▾").rect();
    drag(&mut h, product, pos2(region.min.x + 8.0, region.min.y + 1.0));
    assert_eq!(rows(&mut h), vec!["Product".to_string(), "Region".to_string()]);
}

#[test]
fn right_clicking_an_area_field_opens_value_field_settings() {
    let mut h = harness();
    let region = h.get_by_label("Region ▾").rect().center();
    right_click(&mut h, region);
    h.run_steps(2);
    // The Value Field Settings dialog opens directly on right-click.
    let _ = h.get_by_label("Custom Name:");
}

#[test]
fn right_clicking_an_unassigned_field_adds_it_to_a_chosen_area() {
    let mut h = harness();
    let units = h.get_by_label("Units").rect().center();
    right_click(&mut h, units);
    let node = h.get_all_by_label("Add to Report Filter").next().unwrap();
    node.click();
    h.run_steps(2);
    let info = h.state_mut().session.run("pivot.fields", json!({})).unwrap();
    let filters: Vec<String> = info["filters"].as_array().unwrap().iter().filter_map(|f| f["field"].as_str().map(str::to_string)).collect();
    assert_eq!(filters, vec!["Units".to_string()]);
}

#[test]
fn the_value_field_settings_dialog_selects_a_summary_function() {
    let mut h = harness();
    h.state_mut().open_dialog("pivotFieldSettings", json!({"pivot": "", "field": "Sales", "area": "values"}));
    h.run_steps(3);
    h.get_by_label("Count").click();
    h.run_steps(2);
    for node in h.get_all_by_label_contains("OK") {
        node.click();
    }
    h.run_steps(3);
    let info = h.state_mut().session.run("pivot.fields", json!({})).unwrap();
    let func = info["values"].as_array().unwrap()[0]["func"].as_str().unwrap().to_string();
    assert_eq!(func, "count", "the picked summary function is applied on OK");
}

#[test]
fn the_value_field_settings_dialog_offers_a_currency_number_format() {
    let mut h = harness();
    h.state_mut().open_dialog("pivotFieldSettings", json!({"pivot": "", "field": "Sales", "area": "values"}));
    h.run_steps(3);
    h.get_by_label("Currency").click();
    h.run_steps(2);
    for node in h.get_all_by_label_contains("OK") {
        node.click();
    }
    h.run_steps(3);
    let info = h.state_mut().session.run("pivot.fields", json!({})).unwrap();
    let fmt = info["values"].as_array().unwrap()[0]["numberFormat"].as_str().unwrap().to_string();
    assert!(fmt.starts_with('$'), "the currency format is applied on OK, got {fmt:?}");
}

#[test]
fn the_value_field_settings_dialog_keeps_the_automatic_percent_format() {
    // Opening the dialog and pressing OK without touching Number Format must not stamp a
    // plain numeric format over "% of Grand Total" (that printed 12323.00% in the app).
    let mut h = harness();
    h.state_mut().session.run("pivot.valueSettings", json!({"field": "Sales", "showAs": "percentOfGrandTotal"})).unwrap();
    h.state_mut().open_dialog("pivotFieldSettings", json!({"field": "Sales", "area": "values"}));
    h.run_steps(3);
    for node in h.get_all_by_label_contains("OK") {
        node.click();
    }
    h.run_steps(3);
    let info = h.state_mut().session.run("pivot.fields", json!({})).unwrap();
    let v = &info["values"][0];
    assert_eq!(v["showAs"], "percentOfGrandTotal");
    let fmt = v["numberFormat"].as_str().unwrap_or("");
    assert!(fmt.contains('%'), "the field still renders a percentage after OK, got {fmt:?}");
}

#[test]
fn the_value_field_settings_dialog_changes_the_decimal_places() {
    let mut h = harness();
    h.state_mut().open_dialog("pivotFieldSettings", json!({"field": "Sales", "area": "values"}));
    h.run_steps(3);
    // What the Decimal places spinner and the Number category leave in the dialog.
    {
        let d = h.state_mut().dialog.as_mut().unwrap();
        d.values.insert("numCat".into(), json!("Number"));
        d.values.insert("numDec".into(), json!(4));
        d.values.insert("numTouched".into(), json!(true));
    }
    h.run_steps(2);
    for node in h.get_all_by_label_contains("OK") {
        node.click();
    }
    h.run_steps(3);
    let info = h.state_mut().session.run("pivot.fields", json!({})).unwrap();
    assert_eq!(info["values"][0]["numberFormat"], "#,##0.0000", "four decimal places are applied");
}

#[test]
fn the_value_field_settings_dialog_opens_on_the_current_format() {
    let mut h = harness();
    h.state_mut().session.run("pivot.valueSettings", json!({"field": "Sales", "numberFormat": "0.00%"})).unwrap();
    h.state_mut().open_dialog("pivotFieldSettings", json!({"field": "Sales", "area": "values"}));
    h.run_steps(3);
    let d = h.state().dialog.as_ref().unwrap();
    assert_eq!(d.values["numCat"], "Percentage", "the dialog opens on the field's own category");
    assert_eq!(d.values["numDec"], 2);
}

#[test]
fn a_plain_left_click_on_a_field_button_does_not_remove_it() {
    // The reported bug: clicking a field (no drag at all) dropped it from the report. Removal is
    // only for a real drag that ends outside every area.
    let mut h = harness();
    assert_eq!(rows(&mut h), vec!["Region".to_string(), "Product".to_string()]);
    let region = h.get_by_label("Region ▾").rect().center();
    for _ in 0..3 {
        frame(&mut h, Event::PointerMoved(region));
        frame(&mut h, Event::PointerButton { pos: region, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        frame(&mut h, Event::PointerButton { pos: region, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }
    assert_eq!(rows(&mut h), vec!["Region".to_string(), "Product".to_string()], "clicks must leave the fields alone");
}

#[test]
fn a_slightly_shaky_click_on_a_field_button_does_not_remove_it() {
    // A two-pixel wobble is a click, not a drag-out.
    let mut h = harness();
    let region = h.get_by_label("Region ▾").rect().center();
    frame(&mut h, Event::PointerMoved(region));
    frame(&mut h, Event::PointerButton { pos: region, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    frame(&mut h, Event::PointerMoved(region + vec2(2.0, 1.0)));
    frame(&mut h, Event::PointerButton { pos: region + vec2(2.0, 1.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert_eq!(rows(&mut h), vec!["Region".to_string(), "Product".to_string()]);
}

#[test]
fn dragging_a_field_out_of_the_areas_removes_it() {
    let mut h = harness();
    let region = h.get_by_label("Region ▾").rect().center();
    drag(&mut h, region, pos2(5.0, 700.0));
    assert_eq!(rows(&mut h), vec!["Product".to_string()], "a drag that ends outside every area removes the field");
}

#[test]
fn picking_percent_of_grand_total_keeps_a_percentage_format() {
    // The reported bug: choosing a percentage produced 12323.00% because OK also sent a plain
    // number format. The stored format must be the percent one.
    let mut h = harness();
    h.state_mut().open_dialog("pivotFieldSettings", json!({"field": "Sales", "area": "values"}));
    h.run_steps(3);
    h.get_by_label("Show Values As").click();
    h.run_steps(2);
    {
        let d = h.state_mut().dialog.as_mut().unwrap();
        d.values.insert("showAs".into(), json!("percentOfGrandTotal"));
    }
    h.run_steps(2);
    for node in h.get_all_by_label_contains("OK") {
        node.click();
    }
    h.run_steps(3);
    let info = h.state_mut().session.run("pivot.fields", json!({})).unwrap();
    let v = &info["values"][0];
    assert_eq!(v["showAs"], "percentOfGrandTotal");
    assert_eq!(v["numberFormat"], "0.00%", "a percentage mode keeps a percentage format");
}

#[test]
fn the_create_pivot_dialog_will_not_send_an_empty_existing_location() {
    // An empty Location made the engine silently create a new sheet (and skip the structure
    // protection check), so OK stays disabled until a place is given.
    let mut h = harness();
    h.state_mut().open_dialog("insertPivotTable", json!({"source": "Sheet1!A1:D3", "destination": "new", "location": "Sheet1!A1"}));
    h.run_steps(3);
    let ok = |h: &mut Harness| h.get_all_by_label_contains("OK").next().map(|n| !n.accesskit_node().is_disabled());
    assert_eq!(ok(&mut h), Some(true), "OK is available for a new worksheet");
    {
        let d = h.state_mut().dialog.as_mut().unwrap();
        d.values.insert("destination".into(), json!("existing"));
        d.values.insert("location".into(), json!(""));
    }
    h.run_steps(3);
    let open = h.state().dialog.is_some();
    assert!(open, "the dialog stays open");
    assert_eq!(ok(&mut h), Some(false), "OK is disabled while Existing Worksheet has no location");
    // And with a location it is usable again.
    h.state_mut().dialog.as_mut().unwrap().values.insert("location".into(), json!("Sheet1!F3"));
    h.run_steps(3);
    assert_eq!(ok(&mut h), Some(true), "OK is available once a location is typed");
}

#[test]
fn choosing_a_new_summary_renames_the_automatic_caption() {
    // OK used to send the caption the dialog opened on ("Sum of Sales"), undoing the engine's
    // automatic rename: the header said Sum while the cells held counts.
    let mut h = harness();
    h.state_mut().open_dialog("pivotFieldSettings", json!({"field": "Sales", "area": "values"}));
    h.run_steps(3);
    h.get_by_label("Count").click();
    h.run_steps(2);
    for node in h.get_all_by_label_contains("OK") {
        node.click();
    }
    h.run_steps(3);
    let info = h.state_mut().session.run("pivot.fields", json!({})).unwrap();
    let v = &info["values"][0];
    assert_eq!(v["func"], "count");
    assert_eq!(v["name"], "Count of Sales", "the caption follows the function");
}

#[test]
fn a_hand_typed_caption_still_wins() {
    let mut h = harness();
    h.state_mut().open_dialog("pivotFieldSettings", json!({"field": "Sales", "area": "values"}));
    h.run_steps(3);
    {
        let d = h.state_mut().dialog.as_mut().unwrap();
        d.values.insert("func".into(), json!("count"));
        d.values.insert("name".into(), json!("How many sales"));
    }
    h.run_steps(2);
    for node in h.get_all_by_label_contains("OK") {
        node.click();
    }
    h.run_steps(3);
    let info = h.state_mut().session.run("pivot.fields", json!({})).unwrap();
    assert_eq!(info["values"][0]["name"], "How many sales");
}

#[test]
fn picking_percent_of_parent_row_total_switches_to_percentage() {
    // It divides value by parent, so the dialog must treat it as a percent mode. Otherwise the
    // Number Format block stays on Number and a decimals change sends "#,##0.00", which the engine
    // overrides with 0.00% — so the decimals setting does nothing.
    let mut h = harness();
    h.state_mut().open_dialog("pivotFieldSettings", json!({"field": "Sales", "area": "values"}));
    h.run_steps(3);
    assert_eq!(h.state().dialog.as_ref().unwrap().values["numCat"], "Number", "a plain sum opens on Number");
    h.get_by_label("Show Values As").click();
    h.run_steps(2);
    {
        let d = h.state_mut().dialog.as_mut().unwrap();
        d.values.insert("showAs".into(), json!("percentOfParentRowTotal"));
    }
    h.run_steps(3);
    assert_eq!(h.state().dialog.as_ref().unwrap().values["numCat"], "Percentage", "choosing the mode switches the format block to percentage");
}
