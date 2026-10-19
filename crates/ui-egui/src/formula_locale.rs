//! UI formula syntax boundary. Engine commands, control/API callers and XLSX keep English.
use gridcraft_engine::formula::{self, FormulaLocale, ParseError};
use gridcraft_engine::model::Workbook;

pub(crate) fn canonical(
    text: &str,
    locale: FormulaLocale,
    wb: &Workbook,
    sheet: usize,
    cell: gridcraft_engine::core::CellRef,
) -> Result<String, ParseError> {
    // The editor hid the `_xludf.` prefixes of this cell's formula; confirming it unchanged keeps
    // the user-defined calls instead of binding them to the built-ins of the same name.
    if let Some(stored) = wb.sheet(sheet).and_then(|sh| sh.cell(cell)).filter(|c| c.formula.is_some()).map(|c| c.input_text())
        && has_udf_prefix(&stored)
        && text == display(&stored, locale, wb, sheet)
    {
        return Ok(stored);
    }
    if locale == FormulaLocale::En || wb.sheet(sheet).is_some_and(|sh| wb.styles.get(sh.style_id(cell)).num_fmt.as_str() == "@") {
        return Ok(text.to_string());
    }
    // Same formula detection as the engine's `input_to_cell` (no trimming: " =1" is text).
    let body = text;
    if body.starts_with('=')
        || ((body.starts_with('+') || body.starts_with('-')) && body.chars().nth(1).is_some_and(|c| c.is_alphabetic() || c == '('))
    {
        let known = |name: &str| wb.name(name, sheet).is_some() || wb.table(name).is_some();
        // Match the engine's existing convenience for missing closing parentheses.
        let mut body = body.to_string();
        let mut expr = formula::parse_input(&body, locale, known);
        for _ in 0..8 {
            if expr.is_ok() {
                break;
            }
            body.push(')');
            expr = formula::parse_input(&body, locale, known);
        }
        return Ok(format!("={}", formula::print(&expr?)));
    }
    // Decimal-comma number entry follows the same preference, without rewriting text or dates.
    let normalized = body.trim().replace(locale.decimal_separator(), ".");
    let number = normalized.strip_suffix('%').unwrap_or(&normalized);
    if number.parse::<f64>().is_ok_and(f64::is_finite) {
        return Ok(normalized);
    }
    Ok(text.to_string())
}

pub(crate) fn display(text: &str, locale: FormulaLocale, wb: &Workbook, sheet: usize) -> String {
    // English (and every language without a formula table) shows the stored text, `_xludf.`
    // prefixes aside, and skips a parse per frame.
    if locale == FormulaLocale::En {
        return hide_udf_prefix(text, locale);
    }
    let Some(body) = text.strip_prefix('=') else { return text.to_string() };
    let shown = formula::parse(body)
        .and_then(|expr| formula::print_input(&expr, locale, |name| wb.name(name, sheet).is_some() || wb.table(name).is_some()))
        .map(|body| format!("={body}"))
        .unwrap_or_else(|_| text.to_string());
    hide_udf_prefix(&shown, locale)
}

const UDF_PREFIX: &str = "_xludf.";

fn has_udf_prefix(text: &str) -> bool {
    text.to_ascii_lowercase().contains(UDF_PREFIX)
}

/// Excel shows a user-defined call such as `_xludf.EOMONTH()` as `=EOMONTH()`. Only function
/// names lose the prefix: text literals and names keep it.
fn hide_udf_prefix(text: &str, locale: FormulaLocale) -> String {
    let Some(body) = text.strip_prefix('=').filter(|b| has_udf_prefix(b)) else { return text.to_string() };
    let Ok(tokens) = formula::lexer::tokenize_locale(body, locale) else { return text.to_string() };
    let mut out = String::from("=");
    let mut from = 0;
    for t in tokens {
        if matches!(t.tok, formula::lexer::Tok::Func(_))
            && body.get(t.start..t.start + UDF_PREFIX.len()).is_some_and(|p| p.eq_ignore_ascii_case(UDF_PREFIX))
        {
            out.push_str(body.get(from..t.start).unwrap_or(""));
            from = t.start + UDF_PREFIX.len();
        }
    }
    out.push_str(body.get(from..).unwrap_or(""));
    out
}

// Descriptions/signatures for the supported Spanish function table; untranslated functions
// retain their English help. Parameter punctuation follows the same locale as entry.
const SPANISH_HELP: &[(&str, &str, &str)] = &[
    ("SUM", "SUMA(número1; [número2]; ...)", "Suma los números."),
    ("AVERAGE", "PROMEDIO(número1; [número2]; ...)", "Devuelve el promedio de los números."),
    ("IF", "SI(prueba_lógica; valor_si_verdadero; [valor_si_falso])", "Devuelve un valor según una condición."),
    ("COUNT", "CONTAR(valor1; [valor2]; ...)", "Cuenta las celdas que contienen números."),
    ("SUMIF", "SUMAR.SI(rango; criterio; [rango_suma])", "Suma las celdas que cumplen un criterio."),
    ("COUNTIF", "CONTAR.SI(rango; criterio)", "Cuenta las celdas que cumplen un criterio."),
    ("VLOOKUP", "BUSCARV(valor_buscado; matriz; indicador_columnas; [ordenado])", "Busca un valor en la primera columna de una tabla."),
    ("TODAY", "HOY()", "Devuelve la fecha actual."),
    ("TRUE", "VERDADERO()", "Devuelve el valor lógico VERDADERO."),
    ("FALSE", "FALSO()", "Devuelve el valor lógico FALSO."),
];

/// Original Hungarian help for common functions; other functions keep English help.
const HUNGARIAN_HELP: &[(&str, &str, &str)] = &[
    ("SUM", "SZUM(szám1; [szám2]; ...)", "Összeadja a megadott számokat."),
    ("AVERAGE", "ÁTLAG(szám1; [szám2]; ...)", "Kiszámítja a számok számtani átlagát."),
    ("IF", "HA(logikai_vizsgálat; [érték_ha_igaz]; [érték_ha_hamis])", "A feltétel eredményétől függően adja vissza az egyik értéket."),
    ("COUNT", "DARAB(érték1; [érték2]; ...)", "Megszámolja a számértékeket."),
    ("COUNTA", "DARAB2(érték1; [érték2]; ...)", "Megszámolja a nem üres értékeket."),
    ("COUNTBLANK", "DARABÜRES(tartomány)", "Megszámolja a tartomány üres celláit."),
    ("SUMIF", "SZUMHA(tartomány; feltétel; [összegtartomány])", "Összeadja a megadott feltételnek megfelelő cellák értékeit."),
    ("SUMIFS", "SZUMHATÖBB(összegtartomány; feltételtartomány1; feltétel1; ...)", "Összeadja a minden megadott feltételnek megfelelő értékeket."),
    ("COUNTIF", "DARABTELI(tartomány; feltétel)", "Megszámolja a feltételnek megfelelő cellákat."),
    ("COUNTIFS", "DARABHATÖBB(feltételtartomány1; feltétel1; ...)", "Megszámolja a minden megadott feltételnek megfelelő cellákat."),
    ("MIN", "MIN(szám1; [szám2]; ...)", "Visszaadja a legkisebb számot."),
    ("MAX", "MAX(szám1; [szám2]; ...)", "Visszaadja a legnagyobb számot."),
    ("ROUND", "KEREKÍTÉS(szám; számjegyek)", "A megadott számú számjegyre kerekít; félértéknél a nullától távolodva."),
    ("ROUNDUP", "KEREK.FEL(szám; számjegyek)", "A nullától távolodva kerekít a megadott számú számjegyre."),
    ("ROUNDDOWN", "KEREK.LE(szám; számjegyek)", "A nulla felé kerekít a megadott számú számjegyre."),
    ("ABS", "ABS(szám)", "Visszaadja a szám abszolút értékét."),
    ("IFERROR", "HAHIBA(érték; érték_ha_hiba)", "Hiba esetén a helyettesítő értéket, egyébként az eredeti értéket adja vissza."),
    ("AND", "ÉS(logikai1; [logikai2]; ...)", "IGAZ, ha minden megadott feltétel igaz."),
    ("OR", "VAGY(logikai1; [logikai2]; ...)", "IGAZ, ha legalább egy megadott feltétel igaz."),
    ("NOT", "NEM(logikai)", "Megfordítja a logikai értéket."),
    ("TRUE", "IGAZ()", "Az IGAZ logikai értéket adja vissza."),
    ("FALSE", "HAMIS()", "A HAMIS logikai értéket adja vissza."),
    ("TODAY", "MA()", "Visszaadja az aktuális dátumot."),
    ("NOW", "MOST()", "Visszaadja az aktuális dátumot és időt."),
    ("DATE", "DÁTUM(év; hónap; nap)", "A megadott évből, hónapból és napból dátumértéket állít elő."),
    ("YEAR", "ÉV(dátumérték)", "Visszaadja a dátum évét."),
    ("MONTH", "HÓNAP(dátumérték)", "Visszaadja a dátum hónapját 1 és 12 között."),
    ("DAY", "NAP(dátumérték)", "Visszaadja a dátum hónapon belüli napját."),
    ("LEFT", "BAL(szöveg; [karakterszám])", "Visszaadja a szöveg első karaktereit."),
    ("RIGHT", "JOBB(szöveg; [karakterszám])", "Visszaadja a szöveg utolsó karaktereit."),
    ("MID", "KÖZÉP(szöveg; kezdőpozíció; karakterszám)", "A megadott kezdőpozíciótól ad vissza karaktereket a szövegből."),
    ("LEN", "HOSSZ(szöveg)", "Visszaadja a szöveg karaktereinek számát."),
    ("TRIM", "KIMETSZ(szöveg)", "Eltávolítja a szélső szóközöket, és a szavak között egy szóközt hagy."),
    (
        "VLOOKUP",
        "FKERES(keresett_érték; tábla; oszlopszám; [közelítő_keresés])",
        "A tábla első oszlopában keres, és a megadott oszlop azonos sorából ad vissza értéket.",
    ),
    (
        "HLOOKUP",
        "VKERES(keresett_érték; tábla; sorszám; [közelítő_keresés])",
        "A tábla első sorában keres, és a megadott sor azonos oszlopából ad vissza értéket.",
    ),
    ("MATCH", "HOL.VAN(keresett_érték; keresési_tömb; [egyezés_típusa])", "Visszaadja a keresett érték tömbön belüli relatív helyét."),
    ("INDEX", "INDEX(tömb; sorszám; [oszlopszám]; [területszám])", "Visszaadja a megadott helyen található értéket vagy hivatkozást."),
    ("VAR.S", "VAR.M(szám1; [szám2]; ...)", "Minta alapján becsüli a sokaság varianciáját."),
    ("VAR.P", "VAR.S(szám1; [szám2]; ...)", "Kiszámítja a teljes sokaság varianciáját."),
];

/// Select help by UI locale without changing the canonical function identity.
fn localized_help(locale: FormulaLocale) -> &'static [(&'static str, &'static str, &'static str)] {
    match locale {
        FormulaLocale::Es => SPANISH_HELP,
        FormulaLocale::Hu => HUNGARIAN_HELP,
        _ => &[],
    }
}

/// Signature for a name typed in the selected formula language.
pub(crate) fn signature(name: &str, locale: FormulaLocale) -> Option<String> {
    canonical_signature(locale.canonical_name(name), locale)
}

/// Help for a canonical catalog name, without interpreting it as a localized alias.
pub(crate) fn canonical_signature(canonical: &str, locale: FormulaLocale) -> Option<String> {
    if let Some((_, signature, _)) = localized_help(locale).iter().find(|(name, ..)| *name == canonical) {
        return Some((*signature).into());
    }
    crate::formula_bar::function_signature(canonical).map(|s| {
        let localized = s.strip_prefix(canonical).map(|tail| format!("{}{tail}", locale.function_name(canonical))).unwrap_or(s);
        localized.replace(',', &locale.list_separator().to_string())
    })
}

/// Description for a name typed in the selected formula language.
pub(crate) fn description(name: &str, locale: FormulaLocale) -> Option<String> {
    canonical_description(locale.canonical_name(name), locale)
}

/// Description for a canonical catalog name, with an English fallback.
pub(crate) fn canonical_description(canonical: &str, locale: FormulaLocale) -> Option<String> {
    if let Some((_, _, description)) = localized_help(locale).iter().find(|(name, ..)| *name == canonical) {
        return Some((*description).into());
    }
    crate::formula_bar::function_description(canonical)
}

/// Avoid suggesting an alias that would call a workbook/table name instead of the function.
pub(crate) fn completion_name<'a>(name: &'a str, locale: FormulaLocale, wb: &Workbook, sheet: usize) -> &'a str {
    let alias = locale.function_name(name);
    if wb.name(alias, sheet).is_some() || wb.table(alias).is_some() { name } else { alias }
}

/// Engine-generated edit templates use canonical syntax. The only incomplete template is
/// the bare function opening emitted by Insert Function; typed user input never enters here.
pub(crate) fn display_template(text: &str, locale: FormulaLocale, wb: &Workbook, sheet: usize) -> String {
    if let Some(name) = text.strip_prefix('=').and_then(|s| s.strip_suffix('('))
        && !name.is_empty()
        && name.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '.'))
    {
        return format!("={}(", completion_name(name, locale, wb, sheet));
    }
    display(text, locale, wb, sheet)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hungarian_help_covers_registered_functions_and_preserves_fallbacks() {
        for (name, signature, description) in HUNGARIAN_HELP {
            assert!(crate::formula_bar::function_signature(name).is_some(), "{name}");
            assert!(signature.starts_with(&format!("{}(", FormulaLocale::Hu.function_name(name))));
            assert!(!description.is_empty());
            assert_eq!(canonical_signature(name, FormulaLocale::Hu).as_deref(), Some(*signature));
            assert_eq!(canonical_description(name, FormulaLocale::Hu).as_deref(), Some(*description));
            assert_eq!(canonical_description(name, FormulaLocale::En), crate::formula_bar::function_description(name));
        }
        assert_eq!(canonical_description("SIN", FormulaLocale::Hu), crate::formula_bar::function_description("SIN"));
        assert_eq!(canonical_signature("NOT_A_FUNCTION", FormulaLocale::Hu), None);
    }

    #[test]
    fn hungarian_help_distinguishes_catalog_names_from_typed_aliases() {
        let locale = FormulaLocale::Hu;
        assert_eq!(signature("VAR.S", locale), canonical_signature("VAR.P", locale));
        assert_eq!(description("VAR.S", locale), canonical_description("VAR.P", locale));
        assert!(canonical_signature("VAR.S", locale).unwrap().starts_with("VAR.M("));
        assert!(canonical_signature("VAR.P", locale).unwrap().starts_with("VAR.S("));
        assert!(canonical_description("VAR.S", locale).unwrap().starts_with("Minta"));
        assert!(description("VAR.S", locale).unwrap().contains("teljes sokaság"));
    }
}
