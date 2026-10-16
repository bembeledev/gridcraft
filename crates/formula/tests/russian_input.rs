use gridcraft_formula::{FormulaLocale, parse, parse_input, print, print_input};

fn input(src: &str) -> String {
    print(&parse_input(src, FormulaLocale::Ru, |_| false).unwrap())
}

#[test]
fn russian_aliases_and_argument_separators() {
    for (src, expected) in [
        ("=сумм(A1:A10)", "SUM(A1:A10)"),
        ("СРЗНАЧ(A1:A10)", "AVERAGE(A1:A10)"),
        ("ЕСЛИ(A1>0;\"Да\";\"Нет\")", "IF(A1>0,\"Да\",\"Нет\")"),
        ("СЧЁТ(A1:A10)", "COUNT(A1:A10)"),
        ("счет(A1:A10)", "COUNT(A1:A10)"),
        ("СУММЕСЛИ(A1:A10;\">0\")", "SUMIF(A1:A10,\">0\")"),
        ("СЧЁТЕСЛИ(A1:A10;\">0\")", "COUNTIF(A1:A10,\">0\")"),
        ("ВПР(A2;D:E;2;ЛОЖЬ)", "VLOOKUP(A2,D:E,2,FALSE)"),
        ("СЕГОДНЯ()", "TODAY()"),
        ("ПРОМЕЖУТОЧНЫЕ.ИТОГИ(9;A1:A10)", "SUBTOTAL(9,A1:A10)"),
        ("=истина", "TRUE"),
        ("=ложь", "FALSE"),
        ("ИСТИНА()", "TRUE()"),
        ("ЛОЖЬ()", "FALSE()"),
        ("И(ИСТИНА;ЛОЖЬ)", "AND(TRUE,FALSE)"),
        ("СУММ(1;SUM(2;3);СРЗНАЧ(4;6))", "SUM(1,SUM(2,3),AVERAGE(4,6))"),
        ("ЕСЛИ(ИСТИНА;;)", "IF(TRUE,,)"),
        ("LAMBDA(x;x+1)(2)", "LAMBDA(x,x+1)(2)"),
    ] {
        assert_eq!(input(src), expected, "{src}");
    }
}

#[test]
fn russian_input_preserves_reference_and_array_syntax() {
    for (src, expected) in [
        ("СУММ({1\\2;3\\4};5)", "SUM({1,2;3,4},5)"),
        ("{ИСТИНА\\ЛОЖЬ;TRUE\\FALSE}", "{TRUE,FALSE;TRUE,FALSE}"),
        ("СУММ((A1:A3;C1:C3))", "SUM((A1:A3,C1:C3))"),
        ("\"СУММ;ЛОЖЬ\"&\"a\"\"b\"", "\"СУММ;ЛОЖЬ\"&\"a\"\"b\""),
        ("'СУММ;ЛОЖЬ'!A1+СУММ!B2", "'СУММ;ЛОЖЬ'!A1+СУММ!B2"),
    ] {
        assert_eq!(input(src), expected, "{src}");
    }
    for bad in [r"СУММ({1;2\3})", "{СУММ(A1)}", "{ЛОЖЬ;}"] {
        assert!(parse_input(bad, FormulaLocale::Ru, |_| false).is_err(), "{bad}");
    }
    assert!(parse("СУММ(1;2)").is_err());
    assert!(parse("{ИСТИНА}").is_err());
}

#[test]
fn russian_input_preserves_defined_and_local_names() {
    let known = |name: &str| ["СУММ", "ЛОЖЬ"].iter().any(|n| n.to_lowercase() == name.to_lowercase());
    let expr = parse_input("СУММ(2;3)+ЛОЖЬ+ЕСЛИ(ИСТИНА;4;5)", FormulaLocale::Ru, known).unwrap();
    assert_eq!(print(&expr), "СУММ(2,3)+ЛОЖЬ+IF(TRUE,4,5)");
    for (src, expected) in [
        ("LET(ЛОЖЬ;7;ЛОЖЬ)", "LET(ЛОЖЬ,7,ЛОЖЬ)"),
        ("LET(ложь;ЛОЖЬ;ЕСЛИ(ложь;1;2))", "LET(ложь,FALSE,IF(ложь,1,2))"),
        ("LET(СУММ;LAMBDA(x;x+1);СУММ(2))", "LET(СУММ,LAMBDA(x,x+1),СУММ(2))"),
        ("LAMBDA(ЛОЖЬ;ЛОЖЬ)(7)", "LAMBDA(ЛОЖЬ,ЛОЖЬ)(7)"),
    ] {
        assert_eq!(input(src), expected, "{src}");
    }
}

#[test]
fn russian_locale_controls_decimal_list_array_separators_and_display() {
    let ru = FormulaLocale::Ru;
    let en = FormulaLocale::En;
    let expr = parse_input("ЕСЛИ(ИСТИНА;СУММ(1,5;2,25);0)", ru, |_| false).unwrap();
    assert_eq!(print(&expr), "IF(TRUE,SUM(1.5,2.25),0)");
    assert_eq!(print_input(&expr, ru, |_| false).unwrap(), "ЕСЛИ(ИСТИНА;СУММ(1,5;2,25);0)");
    assert_eq!(print_input(&expr, en, |_| false).unwrap(), print(&expr));
    assert!(parse_input("SUM(1;2)", en, |_| false).is_err());
    assert!(parse_input("СУММ(1.5;2)", ru, |_| false).is_err());
    let expr = parse("SUM({1.5,2.25;3,4})+SUM((A1:A3,C1:C3))").unwrap();
    let shown = print_input(&expr, ru, |_| false).unwrap();
    assert_eq!(print(&parse_input(&shown, ru, |_| false).unwrap()), print(&expr));
    let expr = parse("COUNT(A1:A3)+COUNTIF(B1:B3,\">0\")+MAX(1,2)").unwrap();
    let shown = print_input(&expr, ru, |_| false).unwrap();
    assert_eq!(shown, "СЧЁТ(A1:A3)+СЧЁТЕСЛИ(B1:B3;\">0\")+МАКС(1;2)");
    assert_eq!(print(&parse_input(&shown, ru, |_| false).unwrap()), print(&expr));
}

#[test]
fn russian_display_round_trips_text_containing_separators() {
    let ru = FormulaLocale::Ru;
    let expr = parse(r#"IF(A1="a,b;c",CONCAT("1.5",",",";"),SUM(0.25,A1:A3 B2))"#).unwrap();
    let shown = print_input(&expr, ru, |_| false).unwrap();
    assert_eq!(shown, r#"ЕСЛИ(A1="a,b;c";СЦЕП("1.5";",";";");СУММ(0,25;A1:A3 B2))"#);
    assert_eq!(print(&parse_input(&shown, ru, |_| false).unwrap()), print(&expr));
}

#[test]
fn russian_table_has_unique_names() {
    let mut seen = std::collections::HashSet::new();
    for name in ["СУММ", "СЧЁТ", "ЕСЛИ", "ВПР", "ЛОЖЬ"] {
        assert!(seen.insert(FormulaLocale::Ru.canonical_name(name).to_string()), "{name}");
    }
    assert_eq!(FormulaLocale::Ru.function_name("sum"), "СУММ");
    assert_eq!(FormulaLocale::Ru.canonical_name("счёт"), "COUNT");
    assert_eq!(FormulaLocale::Ru.canonical_name("СЧЕТ"), "COUNT");
    assert_eq!(FormulaLocale::Ru.canonical_name("UNKNOWN"), "UNKNOWN");
}
