//! The From Web background fetch: page downloads run on a worker thread and their results
//! re-enter through the normal engine commands as `prefetchedByUrl` (tables keyed by the URL
//! they came from), so a slow page never blocks the UI (Excel runs its queries in the
//! background too). The web build has no threads: its fetches stay on the synchronous path.

use serde_json::{Value as Json, json};

use crate::SheetApp;

/// One in-flight fetch: the URLs to download and the command its results feed.
pub struct PendingFetch {
    urls: Vec<String>,
    /// The command the results feed (`data.webPreview`, `data.webRefresh`).
    pub(crate) command: (String, Json),
    handle: Option<std::thread::JoinHandle<Json>>,
}

/// The fetched tables of one page, as the `prefetchedByUrl` value holds them.
fn table_json(t: &gridcraft_engine::webquery::WebTable) -> Json {
    json!({"name": t.name, "rows": t.rows})
}

impl SheetApp {
    /// Starts fetching `urls` in the background (a no-op retry if the same fetch is already
    /// running); [`poll_fetch`](Self::poll_fetch) runs `command` with the results once they
    /// are in.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn start_fetch(&mut self, urls: &[&str], command: (&str, Json)) {
        if self.fetch.as_ref().is_some_and(|f| f.urls == urls.iter().map(|s| s.to_string()).collect::<Vec<_>>()) {
            return;
        }
        let source = match self.session.web_source.clone() {
            Some(s) => s,
            None => {
                self.message = Some(("From Web".into(), "Web page import is not available in this build.".into()));
                return;
            }
        };
        let urls: Vec<String> = urls.iter().map(|s| s.to_string()).collect();
        let fetch_urls = urls.clone();
        let handle = std::thread::spawn(move || {
            // Each page lands under its own URL; a failed page records its error instead.
            let mut map = serde_json::Map::new();
            for url in &fetch_urls {
                let req = gridcraft_engine::webquery::WebRequest {
                    url: url.clone(),
                    max_rows: gridcraft_engine::webquery::DEFAULT_MAX_ROWS,
                    max_cols: gridcraft_engine::webquery::MAX_COLS,
                    max_bytes: gridcraft_engine::webquery::DEFAULT_MAX_BYTES,
                };
                let entry = match source(&req) {
                    Ok(tables) => json!({"tables": tables.iter().map(table_json).collect::<Vec<_>>()}),
                    Err(e) => json!({"error": e}),
                };
                map.insert(url.clone(), entry);
            }
            Json::Object(map)
        });
        self.fetch = Some(PendingFetch { urls, command: (command.0.to_string(), command.1), handle: Some(handle) });
    }

    /// True while a fetch is still downloading, for a spinner and repaints.
    pub fn fetching(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.fetch.as_ref().is_some_and(|f| f.handle.as_ref().is_some_and(|h| !h.is_finished()))
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }

    /// Applies a finished fetch: the downloaded pages go to `command` as `prefetchedByUrl` and
    /// both the raw map and the command's result come back to the caller.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn poll_fetch(&mut self) -> Option<(Json, Result<Json, String>)> {
        if !self.fetch.as_ref().is_some_and(|f| f.handle.as_ref().is_none_or(|h| h.is_finished())) {
            return None;
        }
        let fetch = self.fetch.take()?;
        let pages = fetch.handle.and_then(|h| h.join().ok());
        let (id, mut params) = fetch.command;
        match pages {
            Some(pages) => {
                params.as_object_mut().map(|o| o.insert("prefetchedByUrl".into(), pages.clone()));
                Some((pages, self.run(&id, params)))
            }
            None => {
                self.message = Some(("From Web".into(), "The page fetch was cancelled.".into()));
                Some((Json::Null, Ok(Json::Null)))
            }
        }
    }
}
