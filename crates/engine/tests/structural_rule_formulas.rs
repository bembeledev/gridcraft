use gridcraft_core::CellRef;
use gridcraft_engine::{
    Session,
    cf::{CfCache, CfLook},
};
use gridcraft_model::{CfRule, CfValueKind, CondFormat, Validation};
use serde_json::{Value as Json, json};

fn workbook() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s
}

fn run(s: &mut Session, command: &str, params: Json) -> Json {
    s.execute(command, params).unwrap()
}

fn set(s: &mut Session, cell: &str, input: &str) {
    run(s, "cell.set", json!({"cell": cell, "input": input}));
}

fn cf(s: &Session, sheet: &str, range: &str) -> CondFormat {
    let wb = &s.doc().unwrap().wb;
    wb.sheet(wb.sheet_index(sheet).unwrap()).unwrap().cond_formats.iter().find(|cf| cf.ranges[0].a1() == range).unwrap().clone()
}

fn dv(s: &Session, sheet: &str, range: &str) -> Validation {
    let wb = &s.doc().unwrap().wb;
    wb.sheet(wb.sheet_index(sheet).unwrap()).unwrap().validations.iter().find(|dv| dv.ranges[0].a1() == range).unwrap().clone()
}

fn expression(s: &Session, sheet: &str, range: &str) -> String {
    match cf(s, sheet, range).rule {
        CfRule::Expression { formula, .. } => formula,
        other => panic!("expected expression, got {other:?}"),
    }
}

fn look(s: &Session, sheet: &str, cell: &str) -> Option<CfLook> {
    let wb = &s.doc().unwrap().wb;
    CfCache::new().look(wb, wb.sheet_index(sheet).unwrap(), CellRef::parse(cell).unwrap())
}

fn valid(s: &mut Session, cell: &str, input: &str) -> bool {
    run(s, "data.validate", json!({"cell": cell, "input": input}))["ok"] == true
}

fn add_expression(s: &mut Session, range: &str, formula: &str) {
    run(s, "home.conditionalFormat", json!({"range": range, "rule": {"type": "expression", "formula": formula}}));
}

fn assert_row_rules(s: &mut Session, first: usize) {
    let range = format!("C{first}:C{}", first + 1);
    assert_eq!(expression(s, "Sheet1", &range), format!("$A{first}>$B$1"));
    assert_eq!(dv(s, "Sheet1", &range).f1, format!("C{first}<$A{first}"));
    assert!(look(s, "Sheet1", &format!("C{first}")).is_some());
    assert!(look(s, "Sheet1", &format!("C{}", first + 1)).is_none());
    assert!(valid(s, &format!("C{first}"), "8"));
    assert!(!valid(s, &format!("C{first}"), "11"));
    assert!(!valid(s, &format!("C{}", first + 1), "3"));
}

#[test]
fn row_edits_preserve_rule_behavior_through_history_and_xlsx() {
    let mut s = workbook();
    set(&mut s, "B1", "5");
    run(&mut s, "range.setValues", json!({"range": "A4", "values": [[10, null, 8], [2, null, 1]]}));
    add_expression(&mut s, "C4:C5", "=$A4>$B$1");
    run(&mut s, "data.validation", json!({"range": "C4:C5", "type": "custom", "formula1": "=C4<$A4"}));
    assert_row_rules(&mut s, 4);

    run(&mut s, "home.insertRows", json!({"rows": "2:2"}));
    assert_row_rules(&mut s, 5);
    run(&mut s, "edit.undo", json!({}));
    assert_row_rules(&mut s, 4);
    run(&mut s, "edit.redo", json!({}));
    assert_row_rules(&mut s, 5);
    let saved = run(&mut s, "file.saveBytes", json!({"format": "xlsx"}));
    run(&mut s, "file.open", json!({"name": "rules.xlsx", "base64": saved["base64"]}));
    assert_row_rules(&mut s, 5);
    run(&mut s, "home.deleteRows", json!({"rows": "2:2"}));
    assert_row_rules(&mut s, 4);
}

#[test]
fn column_edits_rewrite_both_comparison_and_validation_bounds() {
    let mut s = workbook();
    run(&mut s, "range.setValues", json!({"range": "B1", "values": [[2, 5]]}));
    run(&mut s, "range.setValues", json!({"range": "D3", "values": [[3], [9]]}));
    run(
        &mut s,
        "home.conditionalFormat",
        json!({"range": "D3:D4", "rule": {"type": "cellIs", "operator": "between", "value": "=$B$1", "value2": "=$C$1"}}),
    );
    run(&mut s, "data.validation", json!({"range": "D3:D4", "type": "whole", "operator": "between", "formula1": "=$B$1", "formula2": "=$C$1"}));
    for (command, target, low, high) in [("home.insertColumns", "E", "$C$1", "$D$1"), ("home.deleteColumns", "D", "$B$1", "$C$1")] {
        run(&mut s, command, json!({"cols": "B:B"}));
        let range = format!("{target}3:{target}4");
        let CfRule::CellIs { a, b, .. } = cf(&s, "Sheet1", &range).rule else { panic!("expected cell comparison") };
        assert_eq!((a.as_str(), b.as_deref()), (low, Some(high)));
        let validation = dv(&s, "Sheet1", &range);
        assert_eq!((validation.f1.as_str(), validation.f2.as_deref()), (low, Some(high)));
        assert!(look(&s, "Sheet1", &format!("{target}3")).is_some());
        assert!(look(&s, "Sheet1", &format!("{target}4")).is_none());
        assert!(valid(&mut s, &format!("{target}3"), "3"));
        assert!(!valid(&mut s, &format!("{target}3"), "1"));
        assert!(!valid(&mut s, &format!("{target}3"), "6"));
    }
}

#[test]
fn partial_cell_edits_move_references_outside_the_rule_target() {
    for (insert, delete, values, original, moved, first) in
        [("down", "up", json!([[12], [2]]), "$A$3:$A$4", "$A$4:$A$5", "$A$4"), ("right", "left", json!([[12, 2]]), "$A$3:$B$3", "$B$3:$C$3", "$B$3")]
    {
        let mut s = workbook();
        run(&mut s, "range.setValues", json!({"range": "A3", "values": values}));
        set(&mut s, "C5", "12");
        add_expression(&mut s, "C5", "=$A$3>5");
        run(&mut s, "data.validation", json!({"range": "C5", "type": "list", "formula1": original}));
        for (command, shift, reference, formula) in
            [("home.insertCells", insert, moved, format!("{first}>5")), ("home.deleteCells", delete, original, "$A$3>5".to_string())]
        {
            run(&mut s, command, json!({"range": "A3", "shift": shift}));
            assert_eq!(expression(&s, "Sheet1", "C5"), formula);
            assert_eq!(dv(&s, "Sheet1", "C5").f1, reference);
            assert!(look(&s, "Sheet1", "C5").is_some());
            assert!(valid(&mut s, "C5", "12"));
            assert!(valid(&mut s, "C5", "2"));
            assert!(!valid(&mut s, "C5", "99"));
        }
    }
}

#[test]
fn edits_rewrite_other_sheets_qualified_rules_but_leave_local_refs_alone() {
    let mut s = workbook();
    run(&mut s, "sheet.rename", json!({"name": "Source Data"}));
    run(&mut s, "range.setValues", json!({"range": "A3", "values": [[12], [2]]}));
    run(&mut s, "home.insertSheet", json!({"name": "Rules"}));
    set(&mut s, "A3", "99");
    add_expression(&mut s, "D2:D3", "='Source Data'!$A3>5");
    add_expression(&mut s, "F2", "=A3=99");
    run(&mut s, "data.validation", json!({"range": "D2:D3", "type": "whole", "operator": "less", "formula1": "='Source Data'!$A3"}));
    run(&mut s, "data.validation", json!({"range": "F2", "type": "custom", "formula1": "=F2<A3"}));
    run(&mut s, "sheet.activate", json!({"sheet": "Source Data"}));
    run(&mut s, "home.insertRows", json!({"rows": "2:2"}));
    run(&mut s, "sheet.activate", json!({"sheet": "Rules"}));
    assert_eq!(expression(&s, "Rules", "D2:D3"), "'Source Data'!$A4>5");
    assert_eq!(dv(&s, "Rules", "D2:D3").f1, "'Source Data'!$A4");
    assert_eq!(expression(&s, "Rules", "F2"), "A3=99");
    assert_eq!(dv(&s, "Rules", "F2").f1, "F2<A3");
    assert!(look(&s, "Rules", "D2").is_some());
    assert!(look(&s, "Rules", "D3").is_none());
    assert!(look(&s, "Rules", "F2").is_some());
    assert!(valid(&mut s, "D2", "8"));
    assert!(!valid(&mut s, "D2", "13"));
    assert!(!valid(&mut s, "D3", "3"));
}

#[test]
fn data_bar_formula_thresholds_follow_edits_and_literal_lists_stay_literal() {
    let mut s = workbook();
    run(&mut s, "range.setValues", json!({"range": "A3", "values": [[2], [10]]}));
    run(&mut s, "range.setValues", json!({"range": "C5", "values": [[4], [8]]}));
    run(
        &mut s,
        "home.conditionalFormat",
        json!({"range": "C5:C6", "rule": {"type": "dataBar", "min": {"type": "formula", "value": "$A$3"}, "max": {"type": "formula", "value": "$A$4"}}}),
    );
    run(&mut s, "data.validation", json!({"range": "E5", "type": "list", "formula1": "\"A3,A4\""}));
    run(&mut s, "data.validation", json!({"range": "E6", "type": "list", "formula1": "A3,A4"}));
    run(&mut s, "range.setValues", json!({"range": "F5", "values": [[4, 4], [8, 8]]}));
    run(&mut s, "home.conditionalFormat", json!({"range": "F5:F6", "rule": {"type": "colorScale"}}));
    run(&mut s, "home.conditionalFormat", json!({"range": "G5:G6", "rule": {"type": "iconSet"}}));
    // Imported files can provide formula thresholds that these two command dialogs do not expose.
    for cf in &mut std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().wb).sheet_mut(0).unwrap().cond_formats {
        match &mut cf.rule {
            CfRule::ColorScale { stops } => {
                stops.first_mut().unwrap().0 = CfValueKind::Formula("$A$3".into());
                stops.last_mut().unwrap().0 = CfValueKind::Formula("$A$4".into());
            }
            CfRule::IconSet { thresholds, .. } => {
                *thresholds = vec![CfValueKind::Number(0.0), CfValueKind::Formula("$A$3".into()), CfValueKind::Formula("$A$4".into())];
            }
            _ => {}
        }
    }
    let before = look(&s, "Sheet1", "C5");
    let colors_before = look(&s, "Sheet1", "F5");
    let icons_before = look(&s, "Sheet1", "G5");
    assert!(before.as_ref().unwrap().bar.is_some());
    assert!(colors_before.as_ref().unwrap().scale_fill.is_some());
    assert!(icons_before.as_ref().unwrap().icon.is_some());
    run(&mut s, "home.insertCells", json!({"range": "A3", "shift": "down"}));
    let CfRule::DataBar { min, max, .. } = cf(&s, "Sheet1", "C5:C6").rule else { panic!("expected data bar") };
    assert_eq!(min, CfValueKind::Formula("$A$4".into()));
    assert_eq!(max, CfValueKind::Formula("$A$5".into()));
    assert_eq!(look(&s, "Sheet1", "C5"), before);
    let CfRule::ColorScale { stops } = cf(&s, "Sheet1", "F5:F6").rule else { panic!("expected color scale") };
    assert_eq!(stops.first().unwrap().0, CfValueKind::Formula("$A$4".into()));
    assert_eq!(stops.last().unwrap().0, CfValueKind::Formula("$A$5".into()));
    let CfRule::IconSet { thresholds, .. } = cf(&s, "Sheet1", "G5:G6").rule else { panic!("expected icon set") };
    assert_eq!(thresholds, vec![CfValueKind::Number(0.0), CfValueKind::Formula("$A$4".into()), CfValueKind::Formula("$A$5".into())]);
    assert_eq!(look(&s, "Sheet1", "F5"), colors_before);
    assert_eq!(look(&s, "Sheet1", "G5"), icons_before);
    assert_eq!(dv(&s, "Sheet1", "E5").f1, "\"A3,A4\"");
    assert_eq!(dv(&s, "Sheet1", "E6").f1, "A3,A4");
    for cell in ["E5", "E6"] {
        assert!(valid(&mut s, cell, "A3"));
        assert!(valid(&mut s, cell, "A4"));
        assert!(!valid(&mut s, cell, "A5"));
    }
}

#[test]
fn deleting_the_first_rule_cell_rebases_from_the_first_survivor() {
    for (command, params, horizontal, first_area_removed) in [
        ("home.deleteRows", json!({"rows": "2:2"}), false, false),
        ("home.deleteColumns", json!({"cols": "C:C"}), true, false),
        ("home.deleteCells", json!({"range": "A2:E2", "shift": "up"}), false, false),
        ("home.deleteCells", json!({"range": "C1:C4", "shift": "left"}), true, false),
        ("home.deleteRows", json!({"rows": "2:2"}), false, true),
    ] {
        let mut s = workbook();
        let (source, values, relative, absolute, old_ref, old_cell, old_abs_cell, new_ref, new_cell, new_abs_cell, second, after, abs_after) =
            if horizontal {
                ("C1", json!([[99, 12, 2]]), "C2:E2", "C4:E4", "C1", "C2", "C4", "C1", "C2", "C4", "D2", "C2:D2", "C4:D4")
            } else if first_area_removed {
                ("A2", json!([[99], [0], [12], [2]]), "C2,C4:C5", "E2,E4:E5", "A2", "C2", "E2", "A3", "C3", "E3", "C4", "C3:C4", "E3:E4")
            } else {
                ("A2", json!([[99], [12], [2]]), "C2:C4", "E2:E4", "A2", "C2", "E2", "A2", "C2", "E2", "C3", "C2:C3", "E2:E3")
            };
        run(&mut s, "range.setValues", json!({"range": source, "values": values}));
        let absolute_ref = if horizontal { "$C$1" } else { "$A$2" };
        add_expression(&mut s, relative, &format!("{old_ref}>5"));
        add_expression(&mut s, absolute, &format!("{absolute_ref}>5"));
        run(&mut s, "data.validation", json!({"range": relative, "type": "custom", "formula1": format!("{old_cell}<{old_ref}")}));
        run(&mut s, "data.validation", json!({"range": absolute, "type": "custom", "formula1": format!("{old_abs_cell}<{absolute_ref}")}));
        run(&mut s, command, params);

        assert_eq!(expression(&s, "Sheet1", after), format!("{new_ref}>5"));
        assert_eq!(dv(&s, "Sheet1", after).f1, format!("{new_cell}<{new_ref}"));
        assert!(look(&s, "Sheet1", new_cell).is_some());
        assert!(look(&s, "Sheet1", second).is_none());
        assert!(valid(&mut s, new_cell, "8"));
        assert!(!valid(&mut s, new_cell, "13"));
        assert!(!valid(&mut s, second, "3"));
        // An absolute reference to the deleted cell must not retarget the surviving neighbour.
        assert_eq!(expression(&s, "Sheet1", abs_after), "#REF!>5");
        assert_eq!(dv(&s, "Sheet1", abs_after).f1, format!("{new_abs_cell}<#REF!"));
        assert!(look(&s, "Sheet1", new_abs_cell).is_none());
        assert!(!valid(&mut s, new_abs_cell, "8"));
    }
}
