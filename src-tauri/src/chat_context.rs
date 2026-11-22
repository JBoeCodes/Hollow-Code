//! Where a chat turn runs (split out of `chat.rs` by ST11, 2026-10-14, no
//! behaviour change): a conversation whose working directory is gone falls
//! back to the vault, and an unsafe one is refused. `chat.rs` re-exports
//! [`preflight_chat_context`] and [`CHAT_CONTEXT_RECOVERED_ERROR`], so
//! callers keep their `chat::` paths.

use std::path::{Path, PathBuf};

#[derive(Debug)]
struct ResolvedChatContext {
    cwd: PathBuf,
    session_id: Option<String>,
    recovered_from: Option<PathBuf>,
}

/// A persisted conversation can outlive its working directory (most often a
/// temporary Claude worktree that was cleaned up after the branch landed).
/// `Command::spawn` reports that as ENOENT, the same error used for a missing
/// executable, which used to produce the misleading `spawn claude` failure.
/// Resolve the safe fallback without spawning. Claude sessions are keyed by
/// cwd, so carrying the stale session id into the vault would only replace one
/// opaque failure with another. The frontend preflights this before composing
/// a prompt and requires an explicit resend after resetting the conversation.
fn resolve_chat_context(
    cwd: PathBuf,
    vault: &Path,
    session_id: Option<String>,
) -> Result<ResolvedChatContext, String> {
    if cwd.is_dir() {
        // Paperclip PR 19: an existing folder that is a system or shared root
        // (`/`, `/tmp`, bare `~`, …) is refused with its own prefix, never
        // `CHAT_CONTEXT_RECOVERED` (which resets the conversation to the
        // vault). Covers the preflight, the send and the pump's admission.
        if let Some(reason) = crate::agent::unsafe_working_dir(&cwd, Some(vault)) {
            return Err(crate::agent::unsafe_cwd_error(&reason));
        }
        return Ok(ResolvedChatContext {
            cwd,
            session_id,
            recovered_from: None,
        });
    }

    if !vault.is_dir() {
        return Err(format!(
            "chat working directory is unavailable ({}) and the vault fallback is unavailable ({})",
            cwd.display(),
            vault.display()
        ));
    }

    Ok(ResolvedChatContext {
        recovered_from: Some(cwd),
        cwd: vault.to_path_buf(),
        session_id: None,
    })
}

/// Cheap IPC preflight used before the composer consumes a prompt or writes an
/// attachment. `Some(path)` means the requested cwd disappeared and the UI
/// must reset to the vault; `None` means the existing conversation is safe to
/// keep. The send path repeats the check to close the preflight/spawn race.
pub(crate) fn preflight_chat_context(
    cwd: PathBuf,
    vault: &Path,
) -> Result<Option<PathBuf>, String> {
    Ok(resolve_chat_context(cwd, vault, None)?.recovered_from)
}

pub(crate) const CHAT_CONTEXT_RECOVERED_ERROR: &str = "CHAT_CONTEXT_RECOVERED";

pub(crate) fn prepare_chat_send_context(
    cwd: PathBuf,
    vault: &Path,
    session_id: Option<String>,
) -> Result<(PathBuf, Option<String>), String> {
    let context = resolve_chat_context(cwd, vault, session_id)?;
    if let Some(previous_cwd) = context.recovered_from {
        return Err(format!(
            "{CHAT_CONTEXT_RECOVERED_ERROR}: {}",
            previous_cwd.display()
        ));
    }
    Ok((context.cwd, context.session_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_chat_cwd_preflight_selects_vault_and_drops_stale_session() {
        let vault = tempfile::tempdir().unwrap();
        let missing = vault.path().join("deleted-worktree");

        let resolved =
            resolve_chat_context(missing.clone(), vault.path(), Some("stale-session".into()))
                .unwrap();

        assert_eq!(resolved.cwd, vault.path());
        assert_eq!(resolved.recovered_from.as_deref(), Some(missing.as_path()));
        assert!(resolved.session_id.is_none());
    }

    #[test]
    fn missing_chat_cwd_aborts_send_instead_of_using_fallback() {
        let vault = tempfile::tempdir().unwrap();
        let missing = vault.path().join("deleted-worktree");

        let error =
            prepare_chat_send_context(missing.clone(), vault.path(), Some("stale-session".into()))
                .unwrap_err();

        assert_eq!(
            error,
            format!("{CHAT_CONTEXT_RECOVERED_ERROR}: {}", missing.display())
        );
    }

    #[test]
    fn existing_chat_cwd_preserves_session() {
        let vault = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();

        let resolved = resolve_chat_context(
            project.path().to_path_buf(),
            vault.path(),
            Some("live-session".into()),
        )
        .unwrap();

        assert_eq!(resolved.cwd, project.path());
        assert!(resolved.recovered_from.is_none());
        assert_eq!(resolved.session_id.as_deref(), Some("live-session"));
    }

    #[test]
    fn missing_chat_cwd_reports_when_vault_is_also_missing() {
        let base = tempfile::tempdir().unwrap();
        let error = resolve_chat_context(
            base.path().join("missing-cwd"),
            &base.path().join("missing-vault"),
            Some("stale-session".into()),
        )
        .unwrap_err();

        assert!(error.contains("working directory is unavailable"));
        assert!(error.contains("vault fallback is unavailable"));
    }
}
