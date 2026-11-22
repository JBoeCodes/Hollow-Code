//! The Plaud connector: sign-in, sign-out and a health probe through the
//! `plaud` CLI (`@plaud-ai/cli`, tokens at `~/.plaud/tokens.json`).
//!
//! Recordings stay in Plaud. Until 2026-09-26 this module also pulled each
//! recording into `vault/recordings/*.md` on a timer and filled matching
//! `calendar/*.md` meeting notes; that sync was removed with the vault's
//! meeting and recording mirrors (see docs/BUILD_LOG.md). Agents reach
//! recordings through the Plaud connector shared in Settings, Connectors.

use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;

/// Per-CLI-call wall clock. The CLI has its own internal 30s default; this
/// is the outer backstop so a wedged process can't hang.
pub(crate) const CLI_TIMEOUT: Duration = Duration::from_secs(90);
/// `plaud login` opens a browser and blocks on the callback — needs room.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(180);
/// Marker the UI keys on to route the user to the Connect button.
const NEEDS_AUTH: &str = "Plaud not authenticated — run `plaud login`";

// ── Binary + auth ────────────────────────────────────────────────────────

/// Locate the `plaud` CLI. A macOS .app launched from Finder doesn't inherit
/// the shell PATH, so probe the common npm-global / homebrew locations first.
pub fn find_plaud_binary() -> PathBuf {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".local/bin/plaud"));
    }
    candidates.push(PathBuf::from("/opt/homebrew/bin/plaud"));
    candidates.push(PathBuf::from("/usr/local/bin/plaud"));
    candidates.push(PathBuf::from("/usr/bin/plaud"));
    candidates
        .into_iter()
        .find(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("plaud"))
}

/// PATH for the spawned CLI. The `plaud` binary is an npm package whose
/// shebang is `#!/usr/bin/env node` — a Finder-launched .app inherits the
/// bare system PATH (`/usr/bin:/bin:…`), so the kernel finds `plaud` but
/// `env` can't find `node` and the spawn dies with exit 127. Append the
/// usual node homes (the same dirs `find_plaud_binary` probes) after the
/// inherited PATH: gap-filling only, never shadowing an existing node.
fn augmented_path() -> std::ffi::OsString {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&current).collect();
    let mut extras: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        extras.push(home.join(".local/bin"));
    }
    extras.push(PathBuf::from("/opt/homebrew/bin"));
    extras.push(PathBuf::from("/usr/local/bin"));
    for d in extras {
        if d.is_dir() && !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    std::env::join_paths(dirs).unwrap_or(current)
}

/// Cheap, no-network connectivity check: the CLI persists its OAuth tokens
/// to `~/.plaud/tokens.json` once signed in.
pub fn is_connected() -> bool {
    dirs::home_dir()
        .map(|h| h.join(".plaud/tokens.json").exists())
        .unwrap_or(false)
}

/// Run `plaud login`, which opens the browser and blocks on the OAuth
/// callback. Only invoked from an explicit user action (never a background sync).
pub async fn login() -> Result<(), String> {
    // One Plaud browser sign-in at a time: plaud-mcp's uses the same port
    // (`plaud_mcp_auth::SignInSlot`, S8 review).
    let _slot = crate::plaud_mcp_auth::SignInSlot::take()?;
    // Another account's recordings must not be reused.
    let done = run_cli(&["login"], LOGIN_TIMEOUT).await.map(|_| ());
    crate::recordings::forget_recent();
    done
}

/// Health probe: `plaud me` (exit 0 means the stored token is accepted).
pub async fn ping() -> Result<bool, String> {
    run_cli(&["me"], CLI_TIMEOUT).await.map(|_| true)
}

/// Sign out: `plaud logout` revokes the stored OAuth tokens.
pub async fn logout() -> Result<(), String> {
    // Another account's recordings must not be reused.
    let done = run_cli(&["logout"], CLI_TIMEOUT).await.map(|_| ());
    crate::recordings::forget_recent();
    done
}

// ── Process plumbing ─────────────────────────────────────────────────────

/// Spawn the `plaud` CLI with `args`, capture stdout, and map the CLI's exit
/// codes to error strings. Exit 2 (auth) is surfaced with the `NEEDS_AUTH`
/// marker so the UI can route the user to the Connect button.
pub(crate) async fn run_cli(args: &[&str], to: Duration) -> Result<String, String> {
    let bin = find_plaud_binary();
    let mut cmd = Command::new(&bin);
    cmd.args(args).env("PATH", augmented_path());

    // Own process group, bounded, and settled before returning, so a hung
    // CLI (or a helper it started) cannot outlive the timeout.
    let output = crate::process_group::run_to_completion(&mut cmd, None, to, None)
        .await
        .map_err(|e| format!("spawn plaud (is @plaud-ai/cli installed?): {e}"))?;
    if output.timed_out {
        return Err(format!(
            "plaud {} timed out after {}s",
            args.join(" "),
            to.as_secs()
        ));
    }
    if let Some(e) = output.wait_error {
        return Err(format!("plaud {}: {e}", args.join(" ")));
    }
    let Some(status) = output.status else {
        return Err(format!("plaud {}: exit status unavailable", args.join(" ")));
    };

    if status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }

    let code = status.code().unwrap_or(-1);
    let stderr = output.stderr.trim();
    let low = stderr.to_lowercase();
    if code == 2 || low.contains("not authenticated") || low.contains("auth_failed") {
        Err(format!("{NEEDS_AUTH}. {stderr}"))
    } else {
        Err(format!("plaud {} exited {code}: {stderr}", args.join(" ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn augmented_path_keeps_inherited_entries_and_fills_node_homes() {
        let path = augmented_path();
        let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
        // Inherited PATH entries survive, in order, ahead of the extras.
        let inherited = std::env::var_os("PATH").unwrap_or_default();
        let inherited: Vec<PathBuf> = std::env::split_paths(&inherited).collect();
        assert_eq!(&dirs[..inherited.len()], &inherited[..]);
        // Every candidate node home that exists on disk is present exactly once.
        for d in ["/opt/homebrew/bin", "/usr/local/bin"] {
            let d = PathBuf::from(d);
            if d.is_dir() {
                assert_eq!(dirs.iter().filter(|p| **p == d).count(), 1, "{d:?}");
            }
        }
    }
}
