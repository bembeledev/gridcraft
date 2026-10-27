use gridcraft_formula::{FormulaLocale, parse, parse_input, print, print_input};

const TR: FormulaLocale = FormulaLocale::Tr;

fn input(src: &str) -> String {
    print(&parse_input(src, TR, |_| false).unwrap())
}

#[test]
fn turkish_function_names_and_separators_as_in_excel() {
    for (src, expected) in [
        ("=topla(A1:A10)", "SUM(A1:A10)"),
        ("EĞER(A1>0;\"Var\";\"Yok\")", "IF(A1>0,\"Var\",\"Yok\")"),
        ("eğer(A1>0;1;2)", "IF(A1>0,1,2)"),
        ("DÜŞEYARA(A2;D:E;2;YANLIŞ)", "VLOOKUP(A2,D:E,2,FALSE)"),
        ("ETOPLA(A1:A10;\">0\")", "SUMIF(A1:A10,\">0\")"),
        ("EĞERSAY(A1:A10;\">0\")", "COUNTIF(A1:A10,\">0\")"),
        ("ÇOKETOPLA(C:C;A:A;\"x\";B:B;\">5\")", "SUMIFS(C:C,A:A,\"x\",B:B,\">5\")"),
        ("BAĞ_DEĞ_SAY(A1:A9)", "COUNT(A1:A9)"),
        ("DEVRESEL_ÖDEME(0,08/12;10;10000)", "PMT(0.08/12,10,10000)"),
        ("İNDİS(A1:B5;KAÇINCI(\"x\";A1:A5;0);2)", "INDEX(A1:B5,MATCH(\"x\",A1:A5,0),2)"),
        ("indis(A1:B5;2;1)", "INDEX(A1:B5,2,1)"),
        ("ÇAPRAZARA(1;A:A;B:B)", "XLOOKUP(1,A:A,B:B)"),
        ("TOPLA.ÇARPIM(A1:A3;B1:B3)", "SUMPRODUCT(A1:A3,B1:B3)"),
        ("YUVARLA(1,2345;2)", "ROUND(1.2345,2)"),
        ("BUGÜN()", "TODAY()"),
        ("satır()", "ROW()"),
        ("MİN(1;2)+min(3;4)", "MIN(1,2)+MIN(3,4)"),
        // Excel's own Turkish names are kept, even where they look swapped.
        ("SANÇIKAR(\"1+i\";\"2\")", "IMSUM(\"1+i\",\"2\")"),
        ("SANTOPLA(\"1+i\";\"2\")", "IMSUB(\"1+i\",\"2\")"),
        ("=doğru", "TRUE"),
        ("=YANLIŞ", "FALSE"),
        ("DOĞRU()", "TRUE()"),
        // English names are always accepted, as are functions Turkish Excel leaves in English.
        ("SUM(1;2)+MOD(7;2)", "SUM(1,2)+MOD(7,2)"),
        ("LET(x;2;x*3)", "LET(x,2,x*3)"),
        ("LAMBDA(x;x+1)(2)", "LAMBDA(x,x+1)(2)"),
    ] {
        assert_eq!(input(src), expected, "{src}");
    }
}

#[test]
fn turkish_letters_follow_turkish_case_rules() {
    // `i` upper-cases to `İ` and `ı` to `I`, as Turkish Excel reads them.
    assert_eq!(TR.upper("eğer"), "EĞER");
    assert_eq!(TR.upper("indis"), "İNDİS");
    assert_eq!(TR.upper("satır"), "SATIR");
    assert_eq!(TR.canonical_name("min"), "MIN");
    assert_eq!(TR.canonical_name("MİN"), "MIN");
    assert_eq!(TR.canonical_name("Düşeyara"), "VLOOKUP");
    // Not a Turkish name: the text is left as typed (an English or user-defined name).
    assert_eq!(TR.canonical_name("MIN"), "MIN");
    assert_eq!(TR.function_name("vlookup"), "DÜŞEYARA");
    assert_eq!(TR.function_name("LAMBDA"), "LAMBDA");
}

#[test]
fn turkish_display_round_trips() {
    for english in [
        "IF(TRUE,SUM(1.5,2.25),0)",
        "VLOOKUP(A2,Sheet2!A:C,3,FALSE)",
        "IFERROR(INDEX(A1:A9,MATCH(B1,C1:C9,0)),\"\")",
        "SUM({1.5,2;3,4})+SUM((A1:A3,C1:C3))",
        "{TRUE,FALSE;FALSE,TRUE}",
        r#"IF(A1="a,b;c\d",CONCAT("1.5",",",";"),SUM(0.25,A1:A3 B2))"#,
    ] {
        let expr = parse(english).unwrap();
        let shown = print_input(&expr, TR, |_| false).unwrap();
        assert_eq!(print(&parse_input(&shown, TR, |_| false).unwrap()), print(&expr), "{english} shown as {shown}");
    }
    let expr = parse("IF(TRUE,SUM(1.5,2.25),VLOOKUP(1,A:B,2,FALSE))").unwrap();
    assert_eq!(print_input(&expr, TR, |_| false).unwrap(), "EĞER(DOĞRU;TOPLA(1,5;2,25);DÜŞEYARA(1;A:B;2;YANLIŞ))");
}

#[test]
fn turkish_array_constants_and_references() {
    // Turkish Excel: `;` between the columns of an array constant, `\` between its rows.
    assert_eq!(input("TOPLA({1;2\\3;4};5)"), "SUM({1,2;3,4},5)");
    assert_eq!(input("{DOĞRU;YANLIŞ\\TRUE;FALSE}"), "{TRUE,FALSE;TRUE,FALSE}");
    assert_eq!(print_input(&parse("{1.5,2;3,4}").unwrap(), TR, |_| false).unwrap(), "{1,5;2\\3;4}");
    for (src, expected) in [
        ("TOPLA((A1:A3;C1:C3))", "SUM((A1:A3,C1:C3))"),
        ("\"TOPLA;YANLIŞ\"&\"a\"\"b\"", "\"TOPLA;YANLIŞ\"&\"a\"\"b\""),
        ("'TOPLA;YANLIŞ'!A1+TOPLA!B2", "'TOPLA;YANLIŞ'!A1+TOPLA!B2"),
        ("Tablo1[EĞER]+YANLIŞ[TOPLA]", "Tablo1[EĞER]+YANLIŞ[TOPLA]"),
    ] {
        assert_eq!(input(src), expected, "{src}");
    }
    for bad in ["{TOPLA(A1)}", "{YANLIŞ\\}", "TOPLA(1.5;2)"] {
        assert!(parse_input(bad, TR, |_| false).is_err(), "{bad}");
    }
}

#[test]
fn turkish_input_preserves_defined_and_local_names() {
    let known = |name: &str| ["TOPLA", "YANLIŞ"].iter().any(|n| gridcraft_formula::same_name(n, name));
    let expr = parse_input("TOPLA(2;3)+YANLIŞ+EĞER(DOĞRU;4;5)", TR, known).unwrap();
    assert_eq!(print(&expr), "TOPLA(2,3)+YANLIŞ+IF(TRUE,4,5)");
    for (src, expected) in [
        ("LET(YANLIŞ;7;YANLIŞ)", "LET(YANLIŞ,7,YANLIŞ)"),
        ("LET(TOPLA;LAMBDA(x;x+1);TOPLA(2))", "LET(TOPLA,LAMBDA(x,x+1),TOPLA(2))"),
        ("LAMBDA(EĞER;EĞER+1)(2)", "LAMBDA(EĞER,EĞER+1)(2)"),
    ] {
        assert_eq!(input(src), expected, "{src}");
    }
}
