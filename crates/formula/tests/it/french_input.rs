use gridcraft_formula::{FormulaLocale, parse, parse_input, print, print_input};

const FR: FormulaLocale = FormulaLocale::Fr;

fn input(src: &str) -> String {
    print(&parse_input(src, FR, |_| false).unwrap())
}

fn shown(english: &str) -> String {
    print_input(&parse(english).unwrap(), FR, |_| false).unwrap()
}

#[test]
fn french_aliases_and_argument_separators() {
    for (src, expected) in [
        ("=somme(A1;A2)", "SUM(A1,A2)"),
        ("MOYENNE(A1:A10)", "AVERAGE(A1:A10)"),
        ("SI(A1>0;\"Oui\";\"Non\")", "IF(A1>0,\"Oui\",\"Non\")"),
        ("NB(A1:A10)", "COUNT(A1:A10)"),
        ("NBVAL(A1:A10)", "COUNTA(A1:A10)"),
        ("SOMME.SI(A1:A10;\">0\")", "SUMIF(A1:A10,\">0\")"),
        ("SOMME.SI.ENS(C1:C9;A1:A9;\">0\")", "SUMIFS(C1:C9,A1:A9,\">0\")"),
        ("NB.SI(A1:A10;\">0\")", "COUNTIF(A1:A10,\">0\")"),
        ("RECHERCHEV(A2;D:E;2;FAUX)", "VLOOKUP(A2,D:E,2,FALSE)"),
        ("RECHERCHEX(A2;D:D;E:E)", "XLOOKUP(A2,D:D,E:E)"),
        ("AUJOURDHUI()", "TODAY()"),
        ("VAN(0,1;A1:A5)", "NPV(0.1,A1:A5)"),
        ("TRI(A1:A5;0,1)", "IRR(A1:A5,0.1)"),
        ("TRIER(A1:A5)", "SORT(A1:A5)"),
        ("VPM(0,08/12;10;10000)", "PMT(0.08/12,10,10000)"),
        ("VA(0,05;12;100)", "PV(0.05,12,100)"),
        ("VC(0,05;12;100)", "FV(0.05,12,100)"),
        ("LIEN_HYPERTEXTE(\"x\")", "HYPERLINK(\"x\")"),
        ("=vrai", "TRUE"),
        ("=faux", "FALSE"),
        ("VRAI()", "TRUE()"),
        ("SOMME(1;SUM(2;3);MOYENNE(4;6))", "SUM(1,SUM(2,3),AVERAGE(4,6))"),
        ("ABS(-1,5)", "ABS(-1.5)"),
        ("LET(x;2;x*3)", "LET(x,2,x*3)"),
        ("LAMBDA(x;x+1)(2)", "LAMBDA(x,x+1)(2)"),
    ] {
        assert_eq!(input(src), expected, "{src}");
    }
}

#[test]
fn french_display_and_round_trip() {
    for (english, french) in [
        ("SUM(A1,A2)", "SOMME(A1;A2)"),
        ("IF(TRUE,SUM(1.5,2.25),FALSE)", "SI(VRAI;SOMME(1,5;2,25);FAUX)"),
        ("COUNTIF(A1:A9,\">0\")+SUMIF(A1:A9,\">0\")", "NB.SI(A1:A9;\">0\")+SOMME.SI(A1:A9;\">0\")"),
        ("SUM({1.5,2;3,4})", "SOMME({1,5.2;3.4})"),
        ("AND({TRUE,FALSE})", "ET({VRAI.FAUX})"),
        ("ABS(1)+LET(x,1,x)", "ABS(1)+LET(x;1;x)"),
    ] {
        assert_eq!(shown(english), french, "{english}");
        assert_eq!(input(french), english, "{french}");
    }
}

#[test]
fn french_names_that_are_english_names_of_other_functions_follow_french_excel() {
    // French Excel calls MIRR `TRIM` and VARP `VAR.P`; TRIM is `SUPPRESPACE`, VAR.P is `VAR.P.N`.
    assert_eq!(input("TRIM(A1:A5;0,1;0,12)"), "MIRR(A1:A5,0.1,0.12)");
    assert_eq!(input("SUPPRESPACE(A1)"), "TRIM(A1)");
    assert_eq!(input("VAR.P(A1:A5)"), "VARP(A1:A5)");
    assert_eq!(input("VAR.P.N(A1:A5)"), "VAR.P(A1:A5)");
    for english in ["MIRR(A1:A5,0.1,0.12)", "TRIM(A1)", "VARP(A1:A5)", "VAR.P(A1:A5)"] {
        assert_eq!(input(&shown(english)), english);
    }
}

#[test]
fn french_table_is_unambiguous() {
    let table = FR.function_table();
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    for (i, (en, fr)) in table.iter().enumerate() {
        for (en2, fr2) in table.iter().skip(i + 1) {
            assert!(!same(en, en2), "{en} listed twice");
            assert!(!same(fr, fr2), "{fr} is the French name of both {en} and {en2}");
        }
        assert!(!same(en, fr), "{en} needs no row");
        // A French name that is another function's English name hides that English name
        // while typing in French. Only French Excel's own two cases are allowed.
        let other = table.iter().any(|(e, _)| same(e, fr)) && !same(en, fr);
        assert_eq!(other, ["TRIM", "VAR.P"].contains(fr), "{fr}");
    }
    assert!(table.len() >= 400, "{}", table.len());
}

#[test]
fn french_input_preserves_defined_and_local_names() {
    let known = |name: &str| ["SOMME", "FAUX"].iter().any(|n| n.eq_ignore_ascii_case(name));
    let expr = parse_input("SOMME(2;3)+FAUX+SI(VRAI;4;5)", FR, known).unwrap();
    assert_eq!(print(&expr), "SOMME(2,3)+FAUX+IF(TRUE,4,5)");
    // Displaying keeps the canonical name when the French one would call the defined name.
    assert_eq!(print_input(&parse("SUM(1,2)+FALSE").unwrap(), FR, known).unwrap(), "SUM(1;2)+FALSE");
    for (src, expected) in [
        ("LET(FAUX;7;FAUX)", "LET(FAUX,7,FAUX)"),
        ("LET(faux;FAUX;SI(faux;1;2))", "LET(faux,FALSE,IF(faux,1,2))"),
        ("LET(SOMME;LAMBDA(x;x+1);SOMME(2))", "LET(SOMME,LAMBDA(x,x+1),SOMME(2))"),
        ("LAMBDA(SOMME;SOMME(2))(LAMBDA(x;x+1))", "LAMBDA(SOMME,SOMME(2))(LAMBDA(x,x+1))"),
        ("LET(FAUX;7;FAUX)+FAUX", "LET(FAUX,7,FAUX)+FALSE"),
    ] {
        assert_eq!(input(src), expected, "{src}");
    }
}

#[test]
fn french_input_keeps_text_references_and_rejects_english_separators() {
    for (src, expected) in [
        ("\"SOMME;FAUX\"&\"a,b\"", "\"SOMME;FAUX\"&\"a,b\""),
        ("'SOMME;FAUX'!A1+SOMME!B2", "'SOMME;FAUX'!A1+SOMME!B2"),
        ("SOMME((A1:A3;C1:C3))", "SUM((A1:A3,C1:C3))"),
    ] {
        assert_eq!(input(src), expected, "{src}");
    }
    for bad in ["SOMME(1.5;2)", "{SOMME(A1)}", "{FAUX;}"] {
        assert!(parse_input(bad, FR, |_| false).is_err(), "{bad}");
    }
    // File and API syntax stays English.
    assert_eq!(print(&parse("SOMME(1,2)+FAUX").unwrap()), "SOMME(1,2)+FAUX");
    assert!(parse("{VRAI}").is_err());
}

#[test]
fn syntax_errors_name_the_separator_as_typed() {
    // A `)` closed too early: the stray separator is the French `;`, not the canonical `,`.
    assert_eq!(parse_input("SI(G10<1000);\"a\";\"b\"", FR, |_| false).unwrap_err().msg, "unexpected `;`");
    assert_eq!(gridcraft_formula::parse("IF(A1<1),1").unwrap_err().msg, "unexpected `,`");
}
