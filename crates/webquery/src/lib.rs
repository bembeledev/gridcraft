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
    let mut bytes: Vec<u8> = Vec::new();
    let _ = resp.into_body().into_reader().take(req.max_bytes as u64).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
