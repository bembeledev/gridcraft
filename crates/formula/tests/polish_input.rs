//! Polish formulas: entry and display syntax, as observed in Polish Excel (see `input/pl.rs`).
use gridcraft_formula::FormulaLocale::{self, Pl};
use gridcraft_formula::{parse, parse_input, print, print_input};

fn canonical(src: &str, locale: FormulaLocale) -> String {
    print(&parse_input(src, locale, |_| false).unwrap_or_else(|e| panic!("{locale:?} `{src}`: {e}")))
}

fn shown(canonical: &str, locale: FormulaLocale) -> String {
    print_input(&parse(canonical).unwrap(), locale, |_| false).unwrap()
}

#[test]
fn polish_formulas_read_and_show_like_polish_excel() {
    for (local, english) in [
        ("JEŻELI(PRAWDA;WYSZUKAJ.PIONOWO(A2;D:E;2;FAŁSZ);#N/D)", "IF(TRUE,VLOOKUP(A2,D:E,2,FALSE),#N/A)"),
        ("SUMA({1\\2;3\\4})+1,5", "SUM({1,2;3,4})+1.5"),
        ("SUMA.JEŻELI(Tabela1[[#Nagłówki];[a]];\">1,5\")", "SUMIF(Tabela1[[#Headers],[a]],\">1,5\")"),
        ("JEŻELI.BŁĄD(1/0;#DZIEL/0!)+X.WYSZUKAJ(1;A:A;B:B)", "IFERROR(1/0,#DIV/0!)+XLOOKUP(1,A:A,B:B)"),
        ("ILE.WIERSZY(Tabela1[#Wszystko])+MODUŁ.LICZBY(#ARG!)", "ROWS(Tabela1[#All])+ABS(#VALUE!)"),
    ] {
        assert_eq!(canonical(local, Pl), english, "entry");
        assert_eq!(shown(english, Pl), local, "display");
    }
}

#[test]
fn english_names_any_letter_case_and_what_keeps_its_english_name() {
    assert!(Pl.translations().len() > 400);
    assert_eq!(canonical("SUM(1;2)+SUMA(3)", Pl), "SUM(1,2)+SUM(3)");
    assert_eq!(canonical("suma.jeżeli(A1:A3;1)+fałsz+średnia(1)", Pl), "SUMIF(A1:A3,1)+FALSE+AVERAGE(1)");
    assert_eq!(canonical("złącz.tekst(\"a\";\"b\")", Pl), "CONCAT(\"a\",\"b\")");
    // Polish Excel keeps MAP and BYCOL in English while BYROW is WIERSZAMI (see `input/pl.rs`).
    assert_eq!(canonical("WIERSZAMI(A1:B3;LAMBDA(x;SUMA(x)))+MAP(1;LAMBDA(x;x))", Pl), "BYROW(A1:B3,LAMBDA(x,SUM(x)))+MAP(1,LAMBDA(x,x))");
    assert_eq!(shown("PERCENTOF(1,4)", Pl), "ODSETEK(1;4)");
    // English separators are not Polish syntax.
    assert!(parse_input("SUMA(1.5;2)", Pl, |_| false).is_err());
    for bad in ["SUMA({1\\}", "Tabela1[[#Nagłówki];", "#ARG"] {
        assert!(parse_input(bad, Pl, |_| false).is_err(), "{bad}");
    }
}
