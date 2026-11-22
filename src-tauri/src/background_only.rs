//! Hollow's helper processes stay out of the Dock (PER-34, 2026-11-16).
//!
//! Every hidden subcommand (`vault-mcp`, `app-mcp`, `reaper`, `serve` and the
//! rest of [`crate::launch::COMMANDS`]) runs from the app's own binary inside
//! `Hollow.app`, whose `Info.plist` has no `LSUIElement`: the GUI needs a Dock
//! icon. So a helper that touches AppKit checks in with Launch Services as a
//! **Foreground** app, and the Dock shows a second Hollow until it exits.
//!
//! What touched AppKit on 2026-11-15 (10:11 PM): a Codex chat's
//! `search_vault` call, which reads connected-project and source PDFs
//! through PDFKit (`doc_text`). PDFKit brings up AppKit, and that checks the
//! process in. Claude chats' helpers that night hadn't read a PDF, so they
//! never checked in at all; the agent was not the cause, the PDF was. Any
//! helper can reach PDFKit (`read_source` on a PDF does it directly), and
//! more AppKit users may come, so the fix is not per call site.
//!
//! The fix: [`enter`] runs first in `main.rs`, before any subcommand starts.
//! For a subcommand it makes the process's `NSApplication` and sets its
//! activation policy to **Prohibited**, so Launch Services lists it as
//! `BackgroundOnly` and the Dock never shows it, whatever it touches later.
//! A plain app launch is left alone.
//!
//! Only where there is a window server to talk to: a process in a session
//! without graphics access (`serve` as a LaunchDaemon, an `ssh` session such
//! as the Mac mini runner's) skips it, because AppKit can't connect there and
//! nothing could show in a Dock anyway. Off macOS it does nothing.

/// Whether the first argument names a hidden subcommand (one of
/// [`crate::launch::COMMANDS`], the list a test keeps equal to `main.rs`'s
/// arms), as opposed to a plain app launch.
pub(crate) fn is_command(first: Option<&str>) -> bool {
    first.is_some_and(|arg| crate::launch::COMMANDS.iter().any(|(name, _)| *name == arg))
}

/// Make this process background-only when it runs a subcommand (see the
/// module doc). Returns whether the policy was set; `false` for a plain
/// launch, a session without a window server, or off the main thread.
/// Never panics: a helper must start whatever this finds.
pub fn enter(first: Option<&str>) -> bool {
    is_command(first) && platform::prohibit_activation()
}

#[cfg(target_os = "macos")]
mod platform {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    #[link(name = "Security", kind = "framework")]
    extern "C" {
        /// `Security/AuthSession.h`: the caller's audit session and its
        /// attribute bits. Asks securityd only; no window server connection.
        fn SessionGetInfo(session: u32, session_id: *mut u32, attributes: *mut u32) -> i32;
    }

    /// `callerSecuritySession`: the calling process's own session.
    const CALLER_SECURITY_SESSION: u32 = u32::MAX;
    /// `sessionHasGraphicAccess`: the session can reach the window server.
    const SESSION_HAS_GRAPHIC_ACCESS: u32 = 0x0010;

    /// Whether this process's session has a window server (a logged-in
    /// console user), so AppKit can connect.
    pub(super) fn has_window_server() -> bool {
        let mut id = 0u32;
        let mut attributes = 0u32;
        // SAFETY: both out-pointers are valid, aligned u32s for the call.
        let status = unsafe { SessionGetInfo(CALLER_SECURITY_SESSION, &mut id, &mut attributes) };
        status == 0 && attributes & SESSION_HAS_GRAPHIC_ACCESS != 0
    }

    pub(super) fn prohibit_activation() -> bool {
        // `main.rs` calls this on the main thread, before any other thread.
        // Checked first, so the off-main-thread refusal holds (and is
        // tested) whatever session the tests run in.
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        if !has_window_server() {
            return false;
        }
        NSApplication::sharedApplication(mtm)
            .setActivationPolicy(NSApplicationActivationPolicy::Prohibited)
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    pub(super) fn has_window_server() -> bool {
        false
    }

    pub(super) fn prohibit_activation() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_subcommand_goes_background_and_a_plain_launch_does_not() {
        for (name, _) in crate::launch::COMMANDS {
            assert!(is_command(Some(name)), "{name}");
        }
        // The helpers PER-34 names, by name, so a rename can't drop one.
        for name in [
            "app-mcp",
            "vault-mcp",
            "code-context-mcp",
            "tasks-mcp",
            "questions-mcp",
            "team-mcp",
            "computer",
            "computer-mcp",
            "remote-mcp",
            "reaper",
            "serve",
        ] {
            assert!(is_command(Some(name)), "{name}");
        }
        for launch in [
            None,
            Some(""),
            Some("--hidden"),
            Some("-psn_0_123"),
            Some("--version"),
        ] {
            assert!(!is_command(launch), "{launch:?}");
            assert!(!enter(launch), "{launch:?}");
        }
    }

    /// Off the main thread `enter` refuses quietly: never a panic, never an
    /// `NSApplication` made on the wrong thread. The thread check comes
    /// before the session check, so this holds in a GUI session and over
    /// ssh alike. (Its own thread, because `--test-threads=1` runs tests on
    /// the main one.)
    #[test]
    fn off_the_main_thread_it_does_nothing() {
        let entered = std::thread::spawn(|| {
            let _ = platform::has_window_server();
            enter(Some("vault-mcp"))
        })
        .join()
        .expect("enter never panics");
        assert!(!entered);
    }

    /// `main.rs` makes a subcommand background-only before it dispatches
    /// one, so nothing a helper runs can check in as a Foreground app first.
    #[test]
    fn main_enters_background_only_before_any_subcommand_runs() {
        let main = include_str!("main.rs");
        let enter = main
            .find("hollow_lib::enter_background_only(")
            .expect("main.rs calls hollow_lib::enter_background_only");
        let first_arm = main
            .find("=> return hollow_lib::")
            .expect("main.rs dispatches");
        assert!(
            enter < first_arm,
            "enter_background_only must run before the match"
        );
        assert!(
            crate::source_pins::contains(
                main,
                "hollow_lib::enter_background_only(command.as_deref());"
            ),
            "main.rs passes the first argument"
        );
    }
}
