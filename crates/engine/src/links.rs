//! Which hyperlink targets may be opened. A cell's link comes from the document (often a file
//! somebody sent), so it is hostile input: only `http`, `https` and `mailto` ever leave the
//! workbook for the system; `file:`, `smb:`, UNC paths, custom URL handlers, `javascript:` and
//! the like are refused. Everything without a scheme stays an in-workbook reference.

use gridcraft_core::{CellRef, RangeRef};
use gridcraft_formula::Expr;
use gridcraft_model::{Hyperlink, Workbook};

/// What a link target is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkKind {
    /// A cell, range, name or table of the workbook (`Sheet2!A1`, `Total`).
    Internal(String),
    /// An address to hand to the system, trimmed: `http`, `https` or `mailto` only.
    External(String),
    /// Refused, with the reason to show.
    Refused(String),
}

const ALLOWED: [&str; 3] = ["http", "https", "mailto"];

/// Classifies a link target. Never panics; the single place that decides what may be opened.
pub fn classify(target: &str) -> LinkKind {
    let t = target.trim();
    if t.chars().any(char::is_control) {
        return LinkKind::Refused("The link contains control characters.".into());
    }
    if t.starts_with("\\\\") || t.starts_with("//") {
        return LinkKind::Refused("Links to network locations aren't opened.".into());
    }
    if RangeRef::parse(t).is_some() {
        return LinkKind::Internal(t.to_string());
    }
    let Some((scheme, rest)) = t.split_once(':') else { return LinkKind::Internal(t.to_string()) };
    let scheme_like = scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !scheme_like {
        return LinkKind::Internal(t.to_string());
    }
    let lower = scheme.to_ascii_lowercase();
    if ALLOWED.contains(&lower.as_str()) && !rest.trim().is_empty() {
        LinkKind::External(t.to_string())
    } else {
        LinkKind::Refused(format!("Links of type \"{lower}:\" aren't opened. Only http, https and mailto links are allowed."))
    }
}

/// The link of a cell: the one stored with it, or, for a cell whose formula is `HYPERLINK(target, …)`,
/// the target that formula currently evaluates to. A scheme-less web address (`google.com`) is
/// opened over `https`, as Excel does.
pub fn cell_link(wb: &Workbook, sheet: usize, at: CellRef) -> Option<Hyperlink> {
    let sh = wb.sheet(sheet)?;
    if let Some(h) = sh.hyperlinks.get(&at) {
        return Some(h.clone());
    }
    let expr = sh.cell(at)?.formula.as_ref()?.expr()?;
    let Expr::Call(name, args) = &expr else { return None };
    if name != "HYPERLINK" {
        return None;
    }
    let value = gridcraft_calc::evaluate_expr(wb, sheet, at, args.first()?);
    let target = value.display().trim().to_string();
    if target.is_empty() || matches!(value, gridcraft_core::Value::Error(_)) {
        return None;
    }
    let target = if looks_like_web_address(&target) && wb.name(&target, sheet).is_none() { format!("https://{target}") } else { target };
    let external = !matches!(classify(&target), LinkKind::Internal(_));
    Some(Hyperlink { target, tooltip: None, external })
}

fn looks_like_web_address(t: &str) -> bool {
    if t.contains(|c: char| c.is_whitespace() || matches!(c, '\\' | '!' | '#' | ':')) {
        return false;
    }
    let host = t.split('/').next().unwrap_or(t);
    let Some((_, tld)) = host.rsplit_once('.') else { return false };
    tld.len() >= 2
        && tld.chars().all(|c| c.is_ascii_alphabetic())
        && !["xlsx", "xlsm", "xlsb", "xls", "csv", "txt", "pdf", "doc", "docx"].contains(&tld.to_ascii_lowercase().as_str())
        && RangeRef::parse(t).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_https_and_mailto_leave_the_workbook() {
        for ok in ["http://a.example/x", "HTTPS://A.example", "  https://a.example  ", "MailTo:a@b.example"] {
            assert!(matches!(classify(ok), LinkKind::External(_)), "{ok}");
        }
        assert_eq!(classify(" https://a.example "), LinkKind::External("https://a.example".into()));
        for bad in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "file:///etc/passwd",
            "smb://host/share",
            "ms-excel:ofe|u|http://x",
            "\\\\host\\share\\x.exe",
            "//host/share",
            "C:\\Windows\\calc.exe",
            "\tjavascript:1",
            "java\nscript:1",
            "\u{1}http://a.example",
            "http:",
            "mailto:",
            "x-custom://a",
        ] {
            assert!(matches!(classify(bad), LinkKind::Refused(_)), "{bad:?} -> {:?}", classify(bad));
        }
    }

    #[test]
    fn references_stay_internal() {
        for r in ["Sheet2!A1", "'My Sheet'!B2:C3", "A1:B2", "#Sheet1!A1", "Total", "", "   "] {
            assert!(matches!(classify(r), LinkKind::Internal(_)), "{r:?} -> {:?}", classify(r));
        }
    }
}
