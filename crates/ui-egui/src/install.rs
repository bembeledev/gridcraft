//! Install as an app (the browser build). A browser that can install the web app (a PWA: its own
//! window and launcher icon, working offline) offers it to the page; while it does, the File menu
//! has Install app…, which shows the browser's own install prompt. An installed app, or the app
//! running installed, gets no offer, so the item is gone. The host keeps the offer
//! (`Services::install_app`); desktop builds have none.
//!
//! Safari installs web apps too (Add to Home Screen on iPhone and iPad, Add to Dock on a Mac) but
//! offers no prompt a page can show. There the item opens a window with the steps.

use egui::RichText;

use crate::SheetApp;
use crate::theme::Tokens;

/// How the browser installs the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offer {
    /// Its own install prompt (`InstallApp::prompt`).
    Prompt,
    /// No prompt a page can show: GridCraft shows these steps (Safari).
    Steps(Steps),
}

/// Safari's steps for installing a web app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Steps {
    /// iPhone and iPad: Share ▸ Add to Home Screen.
    HomeScreen,
    /// A Mac (Safari 17 or later): File ▸ Add to Dock.
    Dock,
}

impl Steps {
    /// The steps' text, in order, and the one that says what follows.
    pub fn messages(self) -> (&'static [&'static str], &'static str) {
        match self {
            Steps::HomeScreen => (
                &["Tap the Share button in the browser toolbar.", "Scroll down and tap Add to Home Screen.", "Tap Add in the top-right corner."],
                "GridCraft will appear on your Home Screen as an app and work offline.",
            ),
            Steps::Dock => (
                &["Open the File menu in Safari.", "Choose Add to Dock…", "Click Add."],
                "GridCraft will open from your Dock in its own window and work offline.",
            ),
        }
    }
}

/// The browser's offer to install the app, kept by the host.
pub struct InstallApp {
    /// What the browser offers now; None: nothing (installed, or it can't install).
    pub offer: Box<dyn Fn() -> Option<Offer>>,
    /// Show the browser's install prompt (`Offer::Prompt`). Browsers only show it right after a click.
    pub prompt: Box<dyn Fn()>,
}

/// Safari's steps for this browser, from its `navigator.userAgent`, `navigator.vendor` and
/// `navigator.maxTouchPoints`; None: not Safari, or a Safari that can't install web apps.
pub fn safari_steps(user_agent: &str, vendor: &str, touch_points: i32) -> Option<Steps> {
    let other = ["CriOS/", "FxiOS/", "EdgiOS/", "OPiOS/", "OPT/"];
    let safari = vendor.starts_with("Apple") && user_agent.contains("Safari/") && !other.iter().any(|o| user_agent.contains(o));
    if !safari {
        return None;
    }
    let version = user_agent.split("Version/").nth(1)?;
    // iPadOS Safari presents itself as a Mac; only its touch screen tells them apart.
    let mobile = ["iPhone", "iPad", "iPod"].iter().any(|d| user_agent.contains(d));
    if mobile || (user_agent.contains("Macintosh") && touch_points > 1) {
        return Some(Steps::HomeScreen);
    }
    let major: u32 = version.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
    (user_agent.contains("Macintosh") && major >= 17).then_some(Steps::Dock)
}

/// What the browser offers now (None: nothing, so File has no Install app… item).
pub fn offered(app: &SheetApp) -> Option<Offer> {
    app.services.install_app.as_ref().and_then(|i| (i.offer)())
}

/// File ▸ Install app…: the browser's own prompt (browsers only show it right after a click), or
/// Safari's steps.
pub fn start(app: &mut SheetApp, offer: Offer) {
    match offer {
        Offer::Prompt => {
            if let Some(install) = &app.services.install_app {
                (install.prompt)();
            }
        }
        Offer::Steps(steps) => app.install_steps = Some(steps),
    }
}

/// The window with Safari's steps, while open.
pub fn show(app: &mut SheetApp, ctx: &egui::Context) {
    let Some(steps) = app.install_steps else { return };
    let t = Tokens::get(ctx);
    let lang = app.ui.language;
    let (lines, after) = steps.messages();
    let mut open = true;
    let mut done = false;
    egui::Window::new(lang.tr("Install GridCraft"))
        .id(egui::Id::new("gc_install_steps"))
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        .default_width(380.0)
        .show(ctx, |ui| {
            ui.label(lang.tr("To install GridCraft on this device:"));
            ui.add_space(6.0);
            for (i, text) in lines.iter().enumerate() {
                ui.label(format!("{}. {}", i + 1, lang.tr(text)));
            }
            ui.add_space(6.0);
            ui.add(egui::Label::new(RichText::new(lang.tr(after)).color(t.text_dim)).wrap());
            ui.add_space(10.0);
            if ui.button(lang.tr("Done")).clicked() {
                done = true;
            }
        });
    if !open || done {
        app.install_steps = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safari_steps_by_device() {
        const APPLE: &str = "Apple Computer, Inc.";
        let iphone =
            "Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Mobile/15E148 Safari/604.1";
        let mac =
            |v: &str| format!("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/{v} Safari/605.1.15");
        assert_eq!(safari_steps(iphone, APPLE, 5), Some(Steps::HomeScreen));
        assert_eq!(safari_steps(&mac("18.5"), APPLE, 5), Some(Steps::HomeScreen), "an iPad presents itself as a Mac");
        assert_eq!(safari_steps(&mac("18.5"), APPLE, 0), Some(Steps::Dock));
        assert_eq!(safari_steps(&mac("17.0"), APPLE, 0), Some(Steps::Dock));
        assert_eq!(safari_steps(&mac("16.6"), APPLE, 0), None, "Add to Dock came with Safari 17");
        let chrome_ios = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/138.0.7204.156 Mobile/15E148 Safari/604.1";
        assert_eq!(safari_steps(chrome_ios, APPLE, 5), None, "Chrome on iOS isn't Safari");
        let chrome = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";
        assert_eq!(safari_steps(chrome, "Google Inc.", 0), None, "Chrome prompts instead");
        let firefox = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:144.0) Gecko/20100101 Firefox/144.0";
        assert_eq!(safari_steps(firefox, "", 0), None);
        assert_eq!(safari_steps("", APPLE, 0), None, "hostile input is just not Safari");
        assert_eq!(safari_steps("Safari/ Version/99999999999999999999 Macintosh", APPLE, 0), None);
    }
}
