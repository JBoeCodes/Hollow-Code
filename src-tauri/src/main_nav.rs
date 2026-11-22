//! Main-window navigation guard (2026-10-02, review A P2). A plain web link
//! in a note or in rendered markdown would otherwise navigate the main
//! window's webview away from Hollow: the app is gone, and the page that
//! loads in its place sits where the app's IPC used to be. Only
//! `ChatMessage` intercepted such links; `linkGuard.ts` now catches the rest
//! in the frontend and opens them, and this guard is the backstop that holds
//! whatever the frontend misses.
//!
//! The guard is a plugin `on_navigation` hook, so it sees every webview; it
//! acts on the `main` webview only. The Preview pane's browser tabs
//! (`browser-*`, child webviews of the main window) keep their own policy in
//! `browser_pane::classify`, and the `md-preview-*` and overlay windows are
//! left alone.
//!
//! wry asks for every frame, not just the top one, and passes only the URL:
//! the hook can't tell a click from a script setting `location`. So it never
//! opens anything itself. Web and mail links are blocked here like every
//! other foreign URL (review finding: opening them externally let an
//! Interactive artifact script open a tab in the user's real browser once a
//! second without a click). Clicked links are opened by the frontend:
//! `App.tsx` for the main document, the `sandbox-doc.js` nav bridge for
//! artifact frames. The policy admits what the app's own iframes load:
//! sandboxed artifact documents (`artifact:`) and `about:blank` /
//! `about:srcdoc`.

use crate::browser_pane::Verdict;
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime, Url};

const MAIN_LABEL: &str = "main";

/// Whether this platform serves the app's custom protocols as
/// `http(s)://<scheme>.localhost` (Windows, Android) rather than
/// `<scheme>://localhost` (macOS, Linux). Mirrors Tauri's `is_local_url`.
const LOCALHOST_FORM: bool = cfg!(any(windows, target_os = "android"));

/// Custom protocols the main window's own pages and frames load from.
const APP_SCHEMES: [&str; 2] = ["tauri", "artifact"];

/// What the main window may navigate to. `dev` is the dev-server origin
/// (only in dev). Anything that is not the app itself is denied. A web or
/// mail link comes back as `External` so the log can say the frontend should
/// have opened it; the hook blocks it all the same.
pub fn classify(url: &Url, dev: Option<&Url>) -> Verdict {
    classify_for(url, dev, LOCALHOST_FORM)
}

fn classify_for(url: &Url, dev: Option<&Url>, localhost_form: bool) -> Verdict {
    let host = url.host_str().unwrap_or("");
    match url.scheme() {
        s if APP_SCHEMES.contains(&s) => {
            if !localhost_form && host == "localhost" {
                Verdict::Allow
            } else {
                Verdict::Deny
            }
        }
        "http" | "https" => {
            if dev.is_some_and(|d| d.origin() == url.origin()) {
                return Verdict::Allow;
            }
            if let Some(scheme) = host.strip_suffix(".localhost") {
                // The Windows form of the app's protocols, or another Tauri
                // host (ipc, asset) a page must never become.
                return if localhost_form && APP_SCHEMES.contains(&scheme) {
                    Verdict::Allow
                } else {
                    Verdict::Deny
                };
            }
            Verdict::External
        }
        "about" if matches!(url.path(), "blank" | "srcdoc") => Verdict::Allow,
        "mailto" => Verdict::External,
        _ => Verdict::Deny,
    }
}

pub fn plugin<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("main-nav-guard")
        .on_navigation(|webview, url| {
            if webview.label() != MAIN_LABEL {
                return true;
            }
            let app = webview.app_handle();
            let dev = if tauri::is_dev() { app.config().build.dev_url.clone() } else { None };
            let verdict = classify(url, dev.as_ref());
            // A video URL's path is its token (media.rs): never log it.
            let shown = if url.scheme() == "hollow-media" || url.host_str() == Some("hollow-media.localhost") {
                "a hollow-media URL".to_string()
            } else {
                url.to_string()
            };
            match verdict {
                Verdict::Allow => {}
                // Never opened from here: the hook can't tell a click from a
                // script, and clicks are opened by the frontend already.
                Verdict::External | Verdict::Ask => eprintln!("main-nav-guard: blocked the main window from loading {shown} (link clicks open from the frontend)"),
                Verdict::Deny => eprintln!("main-nav-guard: blocked the main window from loading {shown}"),
            }
            loads(verdict)
        })
        .build()
}

/// Whether the hook lets the main window load a URL with this verdict. Only
/// the app's own origins and frames load; web and mail links are blocked like
/// the rest.
fn loads(verdict: Verdict) -> bool {
    verdict == Verdict::Allow
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn the_app_and_its_frames_load_and_web_links_are_told_apart() {
        let dev = u("http://localhost:1420");
        // macOS / Linux form.
        for s in [
            "tauri://localhost/",
            "tauri://localhost/index.html?x=1",
            "artifact://localhost/abc",
            "about:blank",
            "about:srcdoc",
        ] {
            assert_eq!(classify_for(&u(s), None, false), Verdict::Allow, "{s}");
        }
        assert_eq!(
            classify_for(&u("http://localhost:1420/"), Some(&dev), false),
            Verdict::Allow,
            "dev server in dev"
        );
        for s in [
            "https://example.com/a",
            "http://example.com",
            "http://localhost:5173/",
            "mailto:a@b.c",
        ] {
            assert_eq!(
                classify_for(&u(s), Some(&dev), false),
                Verdict::External,
                "{s}"
            );
        }
        assert_eq!(
            classify_for(&u("http://localhost:1420/"), None, false),
            Verdict::External,
            "the dev origin is only the app in dev"
        );
    }

    #[test]
    fn everything_else_is_blocked() {
        let dev = u("http://localhost:1420");
        for s in [
            "tauri://evil.example/",
            "artifact://example.com/x",
            "ipc://localhost/x",
            "asset://localhost/x",
            "file:///etc/passwd",
            "data:text/html,hi",
            "blob:https://example.com/1",
            "javascript:alert(1)",
            "about:config",
            "tel:123",
            "obsidian://open?vault=x",
            // The Windows form is not the app on macOS.
            "http://tauri.localhost/",
            "https://artifact.localhost/x",
            "http://ipc.localhost/",
            "http://asset.localhost/x",
        ] {
            assert_eq!(classify_for(&u(s), Some(&dev), false), Verdict::Deny, "{s}");
        }
    }

    #[test]
    fn the_windows_form_is_the_app_only_on_windows() {
        for s in [
            "http://tauri.localhost/",
            "https://tauri.localhost/index.html",
            "http://artifact.localhost/abc",
        ] {
            assert_eq!(classify_for(&u(s), None, true), Verdict::Allow, "{s}");
        }
        for s in [
            "http://ipc.localhost/",
            "http://asset.localhost/x",
            "tauri://localhost/",
            "artifact://localhost/abc",
        ] {
            assert_eq!(classify_for(&u(s), None, true), Verdict::Deny, "{s}");
        }
        assert_eq!(
            classify_for(&u("https://example.com/"), None, true),
            Verdict::External
        );
    }

    #[test]
    fn web_and_mail_links_never_load_in_the_main_window() {
        // The hook can't tell a click from a script setting `location`, so a
        // web link is never loaded and never opened from Rust (the frontend
        // opens clicked links). Review finding: an artifact script looping on
        // `location = 'https://…'` must open nothing.
        let dev = u("http://localhost:1420");
        for form in [false, true] {
            for s in [
                "https://example.com/a",
                "http://example.com/?leak=1",
                "http://localhost:5173/",
                "mailto:a@b.c",
            ] {
                assert!(
                    !loads(classify_for(&u(s), Some(&dev), form)),
                    "{s} (localhost form {form})"
                );
            }
        }
        assert!(loads(classify_for(&u("tauri://localhost/"), None, false)));
        assert!(loads(classify_for(
            &u("http://tauri.localhost/"),
            None,
            true
        )));
    }
}
