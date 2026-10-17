//! HTML tables: the HTML flavour of the clipboard, and HTML files opened as workbooks.
//!
//! Banks and web apps often export "Excel" files that are an HTML page saved as `.xls`; Excel
//! opens them, and so do we. Only the table grid is read: cell text, `colspan`/`rowspan`, the
//! number Excel and LibreOffice record on a cell (`x:num`, `sdval`), `x:str` (keep as text),
//! bold (`<b>`, `<strong>`, `<th>`, `font-weight`), column widths (`width`) and row heights.
//! Simple `<style>` rules (`#id`, `.class`, `td.class`) and inline `style` attributes feed widths,
//! heights and bold. Everything else in the markup is ignored.
//!
//! Cell text is read like typed input. Numbers and dates written the continental way
//! (`1.234,56`, `16/10/2026`) are recognised when the document's own cells show that convention:
//! the file says how it writes numbers, whatever the reader's locale.

use std::sync::Arc;

use gridcraft_core::parse::parse_input;
use gridcraft_core::{CellRef, MAX_COLS, MAX_ROWS, RangeRef, Value};
use gridcraft_model::{Cell, NumFmt, Sheet, StyleId, Workbook};

use crate::{IoError, ReadReport};

/// Widest `colspan` honoured (the sheet's column count).
const MAX_SPAN: u32 = MAX_COLS;
/// Column steps spent placing spanned cells (marking the columns a `rowspan` covers, and
/// skipping them in the rows below). Past this, further `rowspan`s are read as 1, so hostile
/// markup can't make placement quadratic.
const MAX_SPAN_STEPS: u64 = 20_000_000;
/// Bytes looked at to tell an HTML page from text.
const SNIFF_BYTES: usize = 64 * 1024;
/// `<style>` rules kept.
const MAX_RULES: usize = 10_000;

/// One table cell as written in the markup.
#[derive(Clone, Debug, PartialEq)]
pub struct HtmlCell {
    /// Visible text: entities decoded, whitespace runs (and `<br>`) collapsed to one space, trimmed.
    pub text: String,
    /// The number the source recorded for the cell (`x:num`, `sdval`).
    pub num: Option<f64>,
    /// `x:str`: the source says the cell is text.
    pub force_text: bool,
    pub colspan: u32,
    pub rowspan: u32,
    pub bold: bool,
    /// Width in pixels (`width` attribute or CSS).
    pub width: Option<f32>,
}

impl Default for HtmlCell {
    fn default() -> Self {
        HtmlCell { text: String::new(), num: None, force_text: false, colspan: 1, rowspan: 1, bold: false, width: None }
    }
}

/// One table row: its cells and its height in pixels, if the markup sets one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HtmlRow {
    pub cells: Vec<HtmlCell>,
    pub height: Option<f32>,
}

/// What a `<style>` rule or inline style sets.
#[derive(Clone, Copy, Debug, Default)]
struct Props {
    width: Option<f32>,
    height: Option<f32>,
    bold: Option<bool>,
}

impl Props {
    fn merge(&mut self, o: Props) {
        self.width = o.width.or(self.width);
        self.height = o.height.or(self.height);
        self.bold = o.bold.or(self.bold);
    }
}

/// A simple selector: an optional tag name plus an optional `#id` or `.class`.
#[derive(Clone, Debug)]
struct Rule {
    tag: Option<String>,
    id: Option<String>,
    class: Option<String>,
    props: Props,
}

/// The rows of the tables in `html`. `first_only` stops after the first table and at a nested
/// table (the clipboard grid); otherwise every table is read, one after the other, and nested
/// tables' rows follow as rows of their own.
pub fn table_rows(html: &str, first_only: bool) -> Vec<HtmlRow> {
    let mut rows: Vec<HtmlRow> = Vec::new();
    let mut rules: Vec<Rule> = Vec::new();
    let mut cell: Option<HtmlCell> = None;
    let mut depth = 0usize;
    let mut bold_depth = 0usize;
    let mut rest = html;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            push_text(&mut cell, rest, bold_depth);
            break;
        };
        push_text(&mut cell, rest.get(..lt).unwrap_or_default(), bold_depth);
        rest = rest.get(lt..).unwrap_or_default();
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").and_then(|e| after.get(e + 3..)).unwrap_or_default();
            continue;
        }
        let Some(gt) = tag_end(rest) else { break };
        let tag = rest.get(1..gt).unwrap_or_default();
        rest = rest.get(gt + 1..).unwrap_or_default();
        let closing = tag.starts_with('/');
        let body = tag.trim_start_matches('/');
        let name_end = body.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(body.len());
        let name = body.get(..name_end).unwrap_or_default().to_ascii_lowercase();
        let attrs = body.get(name_end..).unwrap_or_default();
        let in_table = depth > 0;
        match (name.as_str(), closing) {
            ("style" | "script", false) => {
                // Raw text: skip to the closing tag, reading the style sheet on the way.
                let end = format!("</{name}");
                let close = find_ci(rest, &end).unwrap_or(rest.len());
                if name == "style" {
                    parse_rules(rest.get(..close).unwrap_or_default(), &mut rules);
                }
                rest = rest.get(close..).unwrap_or_default();
            }
            ("table", false) => {
                if in_table && first_only {
                    break; // a nested table: the outer one is the clipboard grid, stop here
                }
                finish(&mut rows, &mut cell);
                depth += 1;
            }
            ("table", true) if in_table => {
                finish(&mut rows, &mut cell);
                if first_only && !rows.is_empty() {
                    break;
                }
                depth -= 1;
            }
            ("tr", false) if in_table => {
                finish(&mut rows, &mut cell);
                let props = props_for(&rules, "tr", attrs);
                let height = props.height.or_else(|| attr(attrs, "height").and_then(length_px));
                rows.push(HtmlRow { cells: Vec::new(), height });
            }
            ("td" | "th", false) if in_table => {
                finish(&mut rows, &mut cell);
                if rows.is_empty() {
                    rows.push(HtmlRow::default());
                }
                let props = props_for(&rules, &name, attrs);
                let num = attr(attrs, "x:num").or_else(|| attr(attrs, "sdval")).and_then(|v| v.trim().parse::<f64>().ok()).filter(|n| n.is_finite());
                let span = |key, max| attr(attrs, key).and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(1).clamp(1, max);
                cell = Some(HtmlCell {
                    num,
                    force_text: has_attr(attrs, "x:str"),
                    colspan: span("colspan", MAX_SPAN),
                    rowspan: span("rowspan", MAX_ROWS),
                    bold: props.bold.unwrap_or(name == "th"),
                    width: attr(attrs, "width").and_then(length_px).or(props.width),
                    ..HtmlCell::default()
                });
            }
            ("td" | "th", true) | ("tr", true) => finish(&mut rows, &mut cell),
            ("b" | "strong", false) => bold_depth = bold_depth.saturating_add(1),
            ("b" | "strong", true) => bold_depth = bold_depth.saturating_sub(1),
            ("br", _) => push_text(&mut cell, " ", 0),
            _ => {}
        }
    }
    finish(&mut rows, &mut cell);
    rows
}

/// Adds markup text to the open cell; text inside `<b>` makes it bold.
fn push_text(cell: &mut Option<HtmlCell>, text: &str, bold_depth: usize) {
    if let Some(c) = cell.as_mut() {
        c.text.push_str(text);
        if bold_depth > 0 && !text.trim().is_empty() {
            c.bold = true;
        }
    }
}

/// Closes the open cell (if any) into the last row, decoding its text.
fn finish(rows: &mut Vec<HtmlRow>, cell: &mut Option<HtmlCell>) {
    let Some(mut c) = cell.take() else { return };
    c.text = decode(&c.text);
    if rows.is_empty() {
        rows.push(HtmlRow::default());
    }
    if let Some(row) = rows.last_mut() {
        row.cells.push(c);
    }
}

/// Byte index of the `>` that closes the tag `rest` starts with; a `>` inside a quoted attribute
/// value (an inline `mso-number-format` condition such as `[>=1000]`) doesn't end it.
fn tag_end(rest: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (i, c) in rest.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

/// Case-insensitive ASCII search.
fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    if n.is_empty() || h.len() < n.len() {
        return None;
    }
    (0..=h.len() - n.len()).find(|&i| h.get(i..i + n.len()).is_some_and(|w| w.eq_ignore_ascii_case(n)))
}

/// Every attribute of a tag's attribute text as (key, value); a bare attribute has no value.
fn attrs_of(attrs: &str) -> impl Iterator<Item = (&str, Option<&str>)> {
    let mut s = attrs;
    std::iter::from_fn(move || {
        loop {
            s = s.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
            if s.is_empty() {
                return None;
            }
            let key_end = s.find(|c: char| c.is_whitespace() || c == '=' || c == '/').unwrap_or(s.len());
            let key = s.get(..key_end).unwrap_or_default();
            s = s.get(key_end..).unwrap_or_default().trim_start();
            let mut value = None;
            if let Some(after) = s.strip_prefix('=') {
                let after = after.trim_start();
                let (v, next) = match after.chars().next() {
                    Some(q @ ('"' | '\'')) => {
                        let inner = after.get(1..).unwrap_or_default();
                        let end = inner.find(q).unwrap_or(inner.len());
                        (inner.get(..end).unwrap_or_default(), inner.get(end + 1..).unwrap_or_default())
                    }
                    _ => {
                        let end = after.find(char::is_whitespace).unwrap_or(after.len());
                        (after.get(..end).unwrap_or_default(), after.get(end..).unwrap_or_default())
                    }
                };
                value = Some(v);
                s = next;
            }
            if !key.is_empty() {
                return Some((key, value));
            }
            if value.is_none() {
                // Stray character (e.g. a lone '='): skip it so the loop always advances.
                let mut chars = s.chars();
                chars.next();
                s = chars.as_str();
            }
        }
    })
}

/// The value of attribute `name` (quoted or bare). A bare attribute without `=` has no value.
fn attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    attrs_of(attrs).find(|(k, _)| k.eq_ignore_ascii_case(name)).and_then(|(_, v)| v)
}

fn has_attr(attrs: &str, name: &str) -> bool {
    attrs_of(attrs).any(|(k, _)| k.eq_ignore_ascii_case(name))
}

/// A CSS or attribute length in pixels (the sheet's unit, 96 per inch): `13px`, `10pt` or a bare
/// pixel count. Others (`%`, `em`) aren't absolute and give `None`.
fn length_px(v: &str) -> Option<f32> {
    let v = v.trim().to_ascii_lowercase();
    let (num, scale) = if let Some(n) = v.strip_suffix("px") {
        (n, 1.0)
    } else if let Some(n) = v.strip_suffix("pt") {
        (n, 96.0 / 72.0)
    } else if let Some(n) = v.strip_suffix("in") {
        (n, 96.0)
    } else if let Some(n) = v.strip_suffix("cm") {
        (n, 96.0 / 2.54)
    } else {
        (v.as_str(), 1.0)
    };
    let n = num.trim().parse::<f32>().ok()? * scale;
    (n.is_finite() && n > 0.0 && n < 5000.0).then_some(n)
}

/// The width, height and weight a `style` declaration list sets.
fn declarations(css: &str) -> Props {
    let mut p = Props::default();
    for decl in css.split(';') {
        let Some((k, v)) = decl.split_once(':') else { continue };
        let v = v.trim().trim_end_matches("!important").trim();
        match k.trim().to_ascii_lowercase().as_str() {
            "width" => p.width = length_px(v).or(p.width),
            "height" => p.height = length_px(v).or(p.height),
            "font-weight" => {
                p.bold = match v.to_ascii_lowercase().as_str() {
                    "bold" | "bolder" => Some(true),
                    "normal" | "lighter" => Some(false),
                    n => n.parse::<u32>().ok().map(|w| w >= 600),
                }
            }
            _ => {}
        }
    }
    p
}

/// Reads the simple rules of a style sheet; selectors with combinators are skipped.
fn parse_rules(css: &str, rules: &mut Vec<Rule>) {
    let mut rest = css;
    while let Some(open) = rest.find('{') {
        let selectors = rest.get(..open).unwrap_or_default();
        let after = rest.get(open + 1..).unwrap_or_default();
        let close = after.find('}').unwrap_or(after.len());
        let props = declarations(after.get(..close).unwrap_or_default());
        rest = after.get(close + 1..).unwrap_or_default();
        // Comments and `@media`-style wrappers leave junk in front of the selector.
        let selectors = selectors.rsplit("*/").next().unwrap_or_default();
        for sel in selectors.split(',') {
            let sel = sel.trim();
            if sel.is_empty() || sel.contains(|c: char| c.is_whitespace() || matches!(c, '>' | '+' | '~' | ':' | '[' | '@')) {
                continue;
            }
            if rules.len() >= MAX_RULES {
                return;
            }
            let (tag, id, class) = if let Some((t, i)) = sel.split_once('#') {
                (t, Some(i), None)
            } else if let Some((t, c)) = sel.split_once('.') {
                (t, None, Some(c))
            } else {
                (sel, None, None)
            };
            let tag = (!tag.is_empty() && tag != "*").then(|| tag.to_ascii_lowercase());
            rules.push(Rule { tag, id: id.map(str::to_string), class: class.map(str::to_string), props });
        }
    }
}

/// What the style sheet and the inline `style` give an element: tag rules, then class and id
/// rules, then the inline style, later ones winning.
fn props_for(rules: &[Rule], tag: &str, attrs: &str) -> Props {
    let id = attr(attrs, "id");
    let classes = attr(attrs, "class").unwrap_or_default();
    let mut p = Props::default();
    for r in rules {
        if r.tag.as_deref().is_some_and(|t| t != tag) {
            continue;
        }
        let matches = match (&r.id, &r.class) {
            (Some(i), _) => id == Some(i.as_str()),
            (None, Some(c)) => classes.split_whitespace().any(|x| x == c),
            (None, None) => true,
        };
        if matches {
            p.merge(r.props);
        }
    }
    if let Some(style) = attr(attrs, "style") {
        p.merge(declarations(style));
    }
    p
}

/// Named character references besides `&#…;`: the five XML ones, the Latin-1 letters and signs
/// HTML spells by name (U+00A0–U+00FF, in order), and the common punctuation.
const LATIN1_NAMES: [&str; 96] = [
    "nbsp", "iexcl", "cent", "pound", "curren", "yen", "brvbar", "sect", "uml", "copy", "ordf", "laquo", "not", "shy", "reg", "macr", "deg",
    "plusmn", "sup2", "sup3", "acute", "micro", "para", "middot", "cedil", "sup1", "ordm", "raquo", "frac14", "frac12", "frac34", "iquest", "Agrave",
    "Aacute", "Acirc", "Atilde", "Auml", "Aring", "AElig", "Ccedil", "Egrave", "Eacute", "Ecirc", "Euml", "Igrave", "Iacute", "Icirc", "Iuml", "ETH",
    "Ntilde", "Ograve", "Oacute", "Ocirc", "Otilde", "Ouml", "times", "Oslash", "Ugrave", "Uacute", "Ucirc", "Uuml", "Yacute", "THORN", "szlig",
    "agrave", "aacute", "acirc", "atilde", "auml", "aring", "aelig", "ccedil", "egrave", "eacute", "ecirc", "euml", "igrave", "iacute", "icirc",
    "iuml", "eth", "ntilde", "ograve", "oacute", "ocirc", "otilde", "ouml", "divide", "oslash", "ugrave", "uacute", "ucirc", "uuml", "yacute",
    "thorn", "yuml",
];

fn named_entity(name: &str) -> Option<char> {
    let c = match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "euro" => '€',
        "ndash" => '–',
        "mdash" => '—',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "hellip" => '…',
        "bull" => '•',
        "trade" => '™',
        _ => {
            let i = LATIN1_NAMES.iter().position(|n| *n == name)?;
            return char::from_u32(0xA0 + i as u32);
        }
    };
    Some(c)
}

/// HTML text to plain text: entities decoded, whitespace runs collapsed to one space, trimmed.
/// Non-breaking spaces are kept.
fn decode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(amp) = rest.find('&') {
        out.push_str(rest.get(..amp).unwrap_or_default());
        rest = rest.get(amp..).unwrap_or_default();
        let semi = rest.get(..12).unwrap_or(rest).find(';');
        let entity = semi.and_then(|e| rest.get(1..e));
        let ch = entity.and_then(|e| {
            named_entity(e).or_else(|| {
                let n = e
                    .strip_prefix("#x")
                    .or_else(|| e.strip_prefix("#X"))
                    .map(|h| u32::from_str_radix(h, 16))
                    .or_else(|| e.strip_prefix('#').map(str::parse::<u32>));
                n.and_then(Result::ok).and_then(char::from_u32)
            })
        });
        match (ch, semi) {
            (Some(c), Some(e)) => {
                out.push(c);
                rest = rest.get(e + 1..).unwrap_or_default();
            }
            _ => {
                out.push('&');
                rest = rest.get(1..).unwrap_or_default();
            }
        }
    }
    out.push_str(rest);
    let mut text = String::with_capacity(out.len());
    let mut space = false;
    for c in out.chars() {
        if c.is_whitespace() && c != '\u{a0}' {
            space = true;
            continue;
        }
        if space && !text.is_empty() {
            text.push(' ');
        }
        space = false;
        text.push(c);
    }
    text
}

/// Whether `bytes` are an HTML page (or a bare HTML fragment) rather than delimited text: past
/// a byte-order mark, whitespace, an XML declaration and comments, the text starts with a tag,
/// and the first 64 KiB hold a `<table>` (or the page starts with `<html>` or an HTML doctype).
/// UTF-8, UTF-16 (with its byte-order mark) and single-byte encodings are recognised.
pub fn is_html(bytes: &[u8]) -> bool {
    let mut head = bytes.get(..bytes.len().min(SNIFF_BYTES)).unwrap_or(bytes);
    if head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]) {
        // Whole UTF-16 code units only.
        head = head.get(..head.len() & !1).unwrap_or(head);
    }
    let text = crate::csv::decode_text(head).to_ascii_lowercase();
    let mut rest = text.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    loop {
        if let Some(after) = rest.strip_prefix("<!--") {
            let Some(end) = after.find("-->") else { return false };
            rest = after.get(end + 3..).unwrap_or_default().trim_start();
        } else if rest.starts_with("<?xml") {
            let Some(end) = rest.find("?>") else { return false };
            rest = rest.get(end + 2..).unwrap_or_default().trim_start();
        } else {
            break;
        }
    }
    if rest.starts_with("<!doctype html") || rest.starts_with("<html") {
        return true;
    }
    // A fragment (`<meta …><table>`, `<style>…</style><table>`, `<div>Report</div><table>`): it
    // starts with a tag and has a table near the top.
    rest.strip_prefix('<').is_some_and(|r| r.starts_with(|c: char| c.is_ascii_alphabetic())) && rest.contains("<table")
}

/// The language the page declares (`<html lang>`, `xml:lang`, or a `Content-Language` meta),
/// lowercased, e.g. `es` or `en-gb`.
fn document_lang(html: &str) -> Option<String> {
    let mut rest = html;
    while let Some(lt) = find_ci(rest, "<") {
        rest = rest.get(lt..).unwrap_or_default();
        let gt = tag_end(rest)?;
        let tag = rest.get(1..gt).unwrap_or_default();
        rest = rest.get(gt + 1..).unwrap_or_default();
        let name_end = tag.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(tag.len());
        let attrs = tag.get(name_end..).unwrap_or_default();
        let lang = match tag.get(..name_end).unwrap_or_default().to_ascii_lowercase().as_str() {
            "html" => attr(attrs, "lang").or_else(|| attr(attrs, "xml:lang")),
            "meta" if attr(attrs, "http-equiv").is_some_and(|v| v.eq_ignore_ascii_case("content-language")) => attr(attrs, "content"),
            "body" | "table" => return None,
            _ => None,
        };
        if let Some(l) = lang.map(|l| l.trim().to_ascii_lowercase()).filter(|l| !l.is_empty()) {
            return Some(l);
        }
    }
    None
}

/// The number and date convention of a language tag: (comma decimals, day first), each `None`
/// when the language doesn't say.
fn lang_convention(lang: &str) -> (Option<bool>, Option<bool>) {
    let lang = lang.replace('_', "-");
    let primary = lang.split('-').next().unwrap_or_default();
    match primary {
        "en" => match lang.as_str() {
            "en" | "en-us" | "en-ca" | "en-ph" => (Some(false), Some(false)),
            _ => (Some(false), Some(true)), // en-GB, en-IE, en-AU, en-NZ, en-IN…
        },
        // Spanish America writes dot decimals in Mexico, Central America and the Caribbean.
        "es" => match lang.as_str() {
            "es-mx" | "es-us" | "es-pr" | "es-do" | "es-gt" | "es-hn" | "es-ni" | "es-pa" | "es-sv" => (Some(false), Some(true)),
            _ => (Some(true), Some(true)),
        },
        "de-ch" | "it-ch" | "fr-ch" => (Some(false), Some(true)),
        "fr" | "de" | "it" | "pt" | "nl" | "ru" | "uk" | "pl" | "cs" | "sk" | "sl" | "hr" | "sr" | "bs" | "bg" | "ro" | "hu" | "el" | "tr" | "sv"
        | "da" | "nb" | "nn" | "no" | "fi" | "et" | "lv" | "lt" | "is" | "ca" | "eu" | "gl" | "id" | "vi" => (Some(true), Some(true)),
        _ => (None, None),
    }
}

/// How the document writes numbers and dates, from its own cells, else its declared language.
/// `None` is undecided: values that read differently either way stay text.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Convention {
    /// `1.234,56`: comma decimals, dot grouping.
    decimal_comma: Option<bool>,
    /// `16/10/2026`: day before month.
    day_first: Option<bool>,
}

/// The digit groups of a number written with `.` and `,` (after an optional sign and before an
/// optional `%`), or `None` when `t` isn't one.
fn number_shape(t: &str) -> Option<&str> {
    let t = t.trim().trim_start_matches(['-', '+']).trim_end_matches('%').trim();
    (t.starts_with(|c: char| c.is_ascii_digit()) && t.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ',')).then_some(t)
}

/// Evidence for comma (`Some(true)`) or dot (`Some(false)`) decimals in one number: the last
/// separator is the decimal one when both appear, or when it isn't followed by exactly three
/// digits (a grouping separator always is).
fn decimal_evidence(t: &str) -> Option<bool> {
    let shape = number_shape(t)?;
    let last = shape.rfind(['.', ','])?;
    let sep = shape.get(last..=last)?;
    let both = shape.contains('.') && shape.contains(',');
    let digits_after = shape.len() - last - 1;
    (both || digits_after != 3).then_some(sep == ",")
}

/// The two leading numbers of `t` and its year, if it is a date written `a/b/y` (or with `-` or
/// `.`), or `a/b` without a year (with `/` only: `10.5` is a number).
fn date_parts(t: &str) -> Option<(u32, u32, Option<&str>)> {
    let t = t.trim();
    let sep = t.chars().find(|c| matches!(c, '/' | '-' | '.'))?;
    let mut parts = t.split(sep);
    let (a, b, y) = (parts.next()?, parts.next()?, parts.next());
    let digits = |s: &str, max: usize| !s.is_empty() && s.len() <= max && s.chars().all(|c| c.is_ascii_digit());
    if parts.next().is_some() || !digits(a, 2) || !digits(b, 2) {
        return None;
    }
    match y {
        Some(y) if (y.len() == 2 || y.len() == 4) && digits(y, 4) => {}
        None if sep == '/' => {}
        _ => return None,
    }
    Some((a.parse().ok()?, b.parse().ok()?, y))
}

fn convention(rows: &[HtmlRow], lang: Option<&str>) -> Convention {
    let (mut comma, mut dot, mut day_first, mut month_first) = (0usize, 0usize, 0usize, 0usize);
    for c in rows.iter().flat_map(|r| &r.cells) {
        if let Some((a, b, _)) = date_parts(&c.text) {
            if a > 12 && b <= 12 {
                day_first += 1;
            } else if b > 12 && a <= 12 {
                month_first += 1;
            }
            continue;
        }
        match decimal_evidence(&c.text) {
            Some(true) => comma += 1,
            Some(false) => dot += 1,
            None => {}
        }
    }
    let (lang_comma, lang_day_first) = lang.map(lang_convention).unwrap_or_default();
    let decide = |yes: usize, no: usize, fallback: Option<bool>| match yes.cmp(&no) {
        std::cmp::Ordering::Greater => Some(true),
        std::cmp::Ordering::Less => Some(false),
        std::cmp::Ordering::Equal => fallback,
    };
    let decimal_comma = decide(comma, dot, lang_comma);
    // Undecided dates follow the language, else the numbers: comma-decimal locales write the
    // day first.
    let day_first = decide(day_first, month_first, lang_day_first.or(decimal_comma.filter(|&c| c)));
    Convention { decimal_comma, day_first }
}

/// A cell's value and the number format Excel would give it, read like typed input in the
/// document's convention. Text that doesn't read as a value in that convention, or that reads
/// differently in the conventions the document leaves open, stays text.
fn cell_value(text: &str, conv: Convention, sys: gridcraft_core::DateSystem) -> (Value, Option<&'static str>) {
    let keep = || (Value::text(text), None);
    if let Some((a, b, y)) = date_parts(text) {
        let day_first = match conv.day_first {
            Some(d) => d,
            None if a > 12 && b <= 12 => true,
            None if b > 12 && a <= 12 => false,
            // 05/03/2026 is 5 March or 3 May: don't guess.
            None => return keep(),
        };
        if !day_first {
            let p = parse_input(text, sys);
            return (p.value, p.format);
        }
        let swapped = match y {
            Some(y) => format!("{b}/{a}/{y}"),
            None => format!("{b}/{a}"),
        };
        let p = parse_input(&swapped, sys);
        if !matches!(p.value, Value::Number(_)) {
            return keep();
        }
        let padded = text.trim().split(['/', '-', '.']).take(2).all(|s| s.len() == 2);
        let format = match y {
            Some(_) if padded => Some("dd/mm/yyyy"),
            Some(_) => Some("d/m/yyyy"),
            None => p.format,
        };
        return (p.value, format);
    }
    if (text.contains(',') || text.contains('.')) && number_shape(text).is_some() {
        match conv.decimal_comma {
            // 1.250 is 1250 or 1.25: don't guess.
            None if decimal_evidence(text).is_none() => return keep(),
            None => {}
            Some(false) => {}
            Some(true) => {
                // Swap the separators and read it the en-US way: grouping and percent keep their formats.
                let swapped: String = text
                    .chars()
                    .map(|c| match c {
                        ',' => '.',
                        '.' => ',',
                        c => c,
                    })
                    .collect();
                let p = parse_input(&swapped, sys);
                return if matches!(p.value, Value::Number(_)) { (p.value, p.format) } else { keep() };
            }
        }
    } else if conv.decimal_comma == Some(true) && (text.contains(',') || text.contains('.')) {
        // `1.234,56 EUR`, `$1.50` in a comma-decimal page: not a value in its convention.
        return keep();
    }
    let p = parse_input(text, sys);
    (p.value, p.format)
}

/// Opens an HTML page as a one-sheet workbook: its tables, one after the other, from A1. Spanned
/// cells become merged cells.
pub fn read_html(bytes: &[u8]) -> Result<(Workbook, ReadReport), IoError> {
    let text = crate::csv::decode_text(bytes);
    let rows = table_rows(&text, false);
    let mut report = ReadReport::default();
    if rows.iter().all(|r| r.cells.is_empty()) {
        return Err(IoError::Format("the HTML page has no table".into()));
    }
    let mut wb = Workbook::new();
    let sys = wb.date_system;
    let conv = convention(&rows, document_lang(&text).as_deref());
    let mut sheet = Sheet::new("Sheet1");
    let mut styles = std::mem::take(&mut wb.styles);
    let mut cache: Vec<((Option<&'static str>, bool), StyleId)> = vec![];
    // For each column, the last row a `rowspan` from a row above covers, plus one (0: free).
    let mut busy_until: Vec<u32> = Vec::new();
    let mut steps: u64 = 0;
    let mut rowspans = true;
    for (r, row) in rows.iter().enumerate() {
        let Ok(r) = u32::try_from(r) else { break };
        if r >= MAX_ROWS {
            report.warn(format!("Rows past {MAX_ROWS} were not imported."));
            break;
        }
        if let Some(h) = row.height {
            sheet.rows.entry(r).or_default().size = Some(h);
        }
        let busy = |busy_until: &[u32], col: u32| busy_until.get(col as usize).is_some_and(|&until| until > r);
        let mut col = 0u32;
        for c in &row.cells {
            while busy(&busy_until, col) {
                col += 1;
                steps += 1;
            }
            if rowspans && steps > MAX_SPAN_STEPS {
                // Too costly to follow: forget the row spans (and add no more merges, so none overlap).
                rowspans = false;
                busy_until = Vec::new();
                report.warn("Some row spans were read as single rows: the page has too many of them.");
            }
            if col >= MAX_COLS {
                report.warn(format!("Columns past {MAX_COLS} were not imported."));
                break;
            }
            // A column span stops at a column a cell above still covers (cells never overlap).
            let mut last_col = col;
            while last_col - col + 1 < c.colspan && last_col + 1 < MAX_COLS && !busy(&busy_until, last_col + 1) {
                last_col += 1;
                steps += 1;
            }
            let last_row = r.saturating_add(c.rowspan - 1).min(MAX_ROWS - 1);
            if last_row > r && rowspans {
                steps += u64::from(last_col - col + 1);
                let need = last_col as usize + 1;
                if busy_until.len() < need {
                    busy_until.resize(need, 0);
                }
                for until in busy_until.iter_mut().take(need).skip(col as usize) {
                    *until = last_row + 1;
                }
            }
            if rowspans && (last_row > r || last_col > col) {
                sheet.merges.push(RangeRef::new(CellRef::new(r, col), CellRef::new(last_row, last_col)));
            }
            if c.colspan == 1
                && let Some(w) = c.width
            {
                sheet.cols.entry(col).or_default().size.get_or_insert(w);
            }
            let (value, format) = match c.num {
                _ if c.force_text => (Value::Text(Arc::from(c.text.as_str())), None),
                Some(n) => (Value::number(n), None),
                None if c.text.is_empty() => (Value::Empty, None),
                None => cell_value(&c.text, conv, sys),
            };
            let key = (format, c.bold);
            let style = match cache.iter().find(|(k, _)| *k == key) {
                Some((_, id)) => *id,
                None => {
                    let id = styles.derive(StyleId::DEFAULT, |s| {
                        if let Some(code) = format {
                            s.num_fmt = NumFmt::new(code);
                        }
                        s.font.bold = c.bold;
                    });
                    cache.push((key, id));
                    id
                }
            };
            if value != Value::Empty || style != StyleId::DEFAULT {
                sheet.cells.set(CellRef::new(r, col), Cell { value, formula: None, style });
            }
            col = last_col.saturating_add(1);
        }
    }
    wb.styles = styles;
    wb.sheets = vec![Arc::new(sheet)];
    Ok((wb, report))
}
