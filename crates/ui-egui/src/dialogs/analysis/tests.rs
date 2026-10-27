use super::*;

fn regression_app() -> SheetApp {
    let mut session = gridcraft_engine::Session::new();
    session.new_workbook();
    session.execute("sheet.rename", json!({"name":"Y's data"})).unwrap();
    session.execute("home.insertSheet", json!({"name":"X's data"})).unwrap();
    session.execute("sheet.activate", json!({"sheet":"Y's data"})).unwrap();
    session.execute("selection.set", json!({"range":"A1:A5"})).unwrap();
    let mut app = SheetApp::new(session, Default::default());
    app.open_dialog("analysisToolPak", json!({"tool":"regression"}));
    app
}

#[test]
fn regression_model_choices_keep_labels_and_zero_as_independent_options() {
    let fields = toolpak_fields("regression");
    assert!(matches!(fields.first(), Some(Field::Range { key: "inputY", .. })));
    assert!(matches!(fields.get(1), Some(Field::Range { key: "inputX", .. })));
    assert!(matches!(fields.get(2), Some(Field::Check { key: "labels", .. })));
    assert!(
        matches!(fields.get(3), Some(Field::Choice { key: "model", options, .. }) if options.iter().map(|(id,_)| *id).collect::<Vec<_>>() == ["linear", "exponential", "logarithmic", "power"])
    );
    assert!(matches!(fields.get(4), Some(Field::Check { key: "constantZero", .. })));
    let mut app = regression_app();
    let d = app.dialog.as_mut().unwrap();
    d.set_value("labels", json!(true));
    d.set_value("constantZero", json!(true));
    d.set_value("model", json!("power"));
    let p = d.command_params(&app.session).unwrap();
    assert_eq!(p["model"], "power");
    assert_eq!(p["labels"], true);
    assert_eq!(p["constantZero"], true);
    assert_eq!(p["inputY"], "'Y''s data'!A1:A5");
}

#[test]
fn analysis_picker_switches_worksheets_and_quotes_each_reference() {
    let mut app = regression_app();
    let d = app.dialog.as_mut().unwrap();
    // A manually typed relative value must also retain its original worksheet.
    d.set_value("inputY", json!("A2:A5"));
    d.range_picker = Some(RangePicker { key: "inputX".into(), single_cell: false });
    app.run("sheet.activate", json!({"sheet":"X's data"})).unwrap();
    let si = app.session.active().unwrap().wb.active_sheet();
    assert!(dialog_range_picker_active(&app));
    update_dialog_range_picker(&mut app, si, CellRef::new(4, 2), CellRef::new(1, 1));
    let d = app.dialog.as_ref().unwrap();
    assert_eq!(d.values["inputX"], "'X''s data'!B2:C5");
    assert_eq!(d.values["inputY"], "'Y''s data'!A2:A5");
    assert_eq!(app.session.active().unwrap().selection.current().a1(), "B2:C5");
    app.dialog.as_mut().unwrap().range_picker = Some(RangePicker { key: "output".into(), single_cell: true });
    update_dialog_range_picker(&mut app, si, CellRef::new(5, 5), CellRef::new(8, 8));
    assert_eq!(app.dialog.as_ref().unwrap().values["output"], "'X''s data'!F6");
    assert_eq!(app.dialog.as_ref().unwrap().values["outputTo"], "range");
}

#[test]
fn analysis_picker_and_confirmation_cannot_rebind_to_another_document() {
    let mut app = regression_app();
    app.dialog.as_mut().unwrap().range_picker = Some(RangePicker { key: "inputX".into(), single_cell: false });
    let values = app.dialog.as_ref().unwrap().values.clone();
    app.session.new_workbook();
    let selection = app.session.active().unwrap().selection.current();
    assert!(!dialog_range_picker_active(&app));
    update_dialog_range_picker(&mut app, 0, CellRef::new(2, 2), CellRef::new(4, 4));
    assert_eq!(app.session.active().unwrap().selection.current(), selection);
    assert_eq!(app.dialog.as_ref().unwrap().values, values);
    assert!(app.dialog.as_mut().unwrap().command_params(&app.session).is_err());
    app.session.set_active(0);
    assert!(dialog_range_picker_active(&app));
    assert!(app.dialog.as_mut().unwrap().command_params(&app.session).is_ok());
    // Closing the owner and creating another workbook at index zero must also fail.
    app.session.close_document(0);
    assert!(app.dialog.as_mut().unwrap().command_params(&app.session).is_err());
}

#[test]
fn analysis_output_keeps_its_sheet_when_confirmation_happens_elsewhere() {
    let mut app = regression_app();
    let d = app.dialog.as_mut().unwrap();
    d.set_value("inputX", json!("'X''s data'!B2:B5"));
    d.set_value("output", json!("J2"));
    d.set_value("outputTo", json!("range"));
    app.run("sheet.activate", json!({"sheet":"X's data"})).unwrap();
    let p = app.dialog.as_mut().unwrap().command_params(&app.session).unwrap();
    assert_eq!(p["output"], "'Y''s data'!J2");
    assert_eq!(p["inputX"], "'X''s data'!B2:B5");
}
