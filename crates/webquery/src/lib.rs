//! The web table source for Data ▸ Get Data ▸ From Web.
//!
//! [`provider`] returns the [`WebSource`] a native host installs in
//! `Session::web_source`. It fetches one page, extracts its HTML tables and
//! reduces each to a [`WebTable`]. Native only: the engine and the web build
//! never link it.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::sync::Arc;

use gridcraft_engine::webquery::{WebFetch, WebTable};

/// The web source with this build's fetcher.
pub fn provider() -> WebFetch {
    Arc::new(fetch)
}

/// Runs `req`: fetch the URL, extract the tables the page holds.
pub fn fetch(req: &gridcraft_engine::webquery::WebRequest) -> Result<Vec<WebTable>, String> {
    log::info!("data.fromWeb: {} (max {} bytes)", redact(&req.url), req.max_bytes);
    #[cfg(target_arch = "wasm32")]
    {
        let _ = req;
        Err("Web page import is not available in this build.".into())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let html = http_get(req)?;
        Ok(gridcraft_engine::webquery::extract_tables(&html, req.max_rows, req.max_cols))
    }
}

/// The URL's origin (scheme and host), for logs and messages: userinfo and the path — which
/// can hold a secret segment — never reach the log.
pub fn redact(url: &str) -> String {
    let url = url.trim_start();
    let Some((scheme, rest)) = url.split_once("://") else {
        return "…".into();
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let authority = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    format!("{scheme}://{authority}")
}

/// One bounded GET: https only, no redirects, no cookies; size- and time-capped.
#[cfg(not(target_arch = "wasm32"))]
fn http_get(req: &gridcraft_engine::webquery::WebRequest) -> Result<String, String> {
    use std::io::Read;

    if !req.url.trim_start().starts_with("https://") {
        return Err("Only https:// addresses can be imported.".into());
    }
    // Userinfo (`https://user:pass@host/`) would be sent as basic auth: check it here too, so
    // the provider never forwards an embedded credential even if a caller skipped validation.
    let authority = req.url.trim_start().trim_start_matches("https://").split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return Err("Web addresses with user and password (`user:pass@host`) can't be imported: the secret would be saved with the workbook.".into());
    }
    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder().max_redirects(0).timeout_global(Some(std::time::Duration::from_secs(30))).build(),
    );
    let resp = agent
        .get(&req.url)
        .header("user-agent", concat!("GridCraft/", env!("CARGO_PKG_VERSION")))
        .header("accept", "text/html,application/xhtml+xml")
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(code) => format!("The page answered {code} and could not be imported."),
            other => other.to_string(),
        })?;
    let content_type = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    if !content_type.is_empty() && !content_type.contains("html") && !content_type.contains("xml") && !content_type.contains("text") {
        return Err(format!("The address serves {content_type}, not a web page."));
    }
    // Read up to the cap, then check for one more byte: a truncated page can look complete
    // (a whole table before the cutoff) and import wrong data in good conscience.
    // The page's own charset (windows-1252 pages are common): without it, every non-ASCII byte
    // would come back as a replacement character. ureq's own `charset()` only matches the exact
    // lowercase `charset=` and keeps the quotes, so parse the header here — case-insensitive,
    // quotes stripped. Decode only after the cap and the overflow check, so `max_bytes` still
    // bounds the bytes read, and fall back to UTF-8.
    let encoding = charset_from(&content_type).and_then(|c| encoding_rs::Encoding::for_label(c.as_bytes())).unwrap_or(encoding_rs::UTF_8);
    let mut reader = resp.into_body().into_reader();
    let mut bytes: Vec<u8> = Vec::new();
    let _ = reader.by_ref().take(req.max_bytes as u64).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let mut extra = [0u8; 1];
    if reader.read(&mut extra).map_err(|e| e.to_string())? != 0 {
        return Err(format!("The page exceeds the {} byte limit.", req.max_bytes));
    }
    Ok(encoding.decode(&bytes).0.into_owned())
}

/// The `Content-Type` header's `charset` value, if it names one: parameter names match
/// case-insensitively and a quoted value loses its quotes, both of which ureq's own
/// `Body::charset()` leaves as-is (exact lowercase `charset=`, quotes kept), so a quoted or
/// capitalised label would silently fall back to UTF-8.
fn charset_from(content_type: &str) -> Option<String> {
    content_type
        .split(';')
        .filter_map(|p| p.trim().split_once('='))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("charset"))
        .map(|(_, value)| value.trim().trim_matches('"').trim_matches('\'').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// Parameter names match case-insensitively and quotes are stripped, where ureq's own
    /// lookup would miss both and fall back to UTF-8.
    fn the_charset_parameter_is_parsed_case_insensitively_and_unquoted() {
        assert_eq!(charset_from("text/html; charset=windows-1252").as_deref(), Some("windows-1252"));
        assert_eq!(charset_from("text/html; Charset=ISO-8859-1").as_deref(), Some("ISO-8859-1"));
        assert_eq!(charset_from("text/html; charset=\"windows-1252\"").as_deref(), Some("windows-1252"));
        assert_eq!(charset_from("text/html; charset='utf-8'").as_deref(), Some("utf-8"));
        assert_eq!(charset_from("text/html").as_deref(), None);
        assert_eq!(charset_from("text/html; boundary=none").as_deref(), None);
    }

    #[test]
    /// A quoted windows-1252 label resolves to that encoding, not the UTF-8 fallback.
    fn a_quoted_charset_resolves_to_its_encoding() {
        let c = charset_from("text/html; charset=\"windows-1252\"").unwrap();
        assert_eq!(encoding_rs::Encoding::for_label(c.as_bytes()), Some(encoding_rs::WINDOWS_1252));
    }
}
