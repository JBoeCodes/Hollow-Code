//! How many chats run at once (Settings, Chat, Chats at once, 2026-10-20).
//!
//! Two app-wide limits the run pump admits every agent chat turn against
//! (`run_pump::QueueFacts::hold`): how many run at once across Hollow, and
//! how many at once in one workspace's own folder. A third, since 2026-11-12,
//! is per conversation: how many of one chat's subagents (the sessions it
//! starts with `start_session`) run at once; further starts wait for one to
//! end (`run_pump::Hold::ParentLimit`). It was a fixed 6 before. Swarm parts
//! keep their own count (`main_agent` `swarm_concurrency`, Settings,
//! Background work → Swarm parts at once): a swarm is one goal fanned out in one
//! call, with its own model and a build-workspace ceiling of two, not a
//! chat's helpers. Every agent turn counts
//! toward the first: workspace chats, a build worktree's turn, the vault
//! lane's one run (a vault-root chat or an automation), swarm parts, durable
//! children and handover successors. `ask_agent` consults keep their own two
//! permits (`run_permit`) and in-process subagents are not turns. A build
//! worktree still runs one turn at a time whatever these say.
//!
//! Saved as `chat-limits.json` in the app-data folder
//! (`agent_settings::app_data_dir`), on the backend because the pump admits
//! turns before the window has loaded. The person changes them in Settings;
//! the COO's route is `settings.set_chat_limits` (`chatsAtOnce`,
//! `chatsPerWorkspace`, `subagentsPerChat`), a confirm action whose card
//! shows old to new (`docs/HOLLOW_APP_COMMAND_MAP.tsv`).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The most chats Settings let run at once across Hollow (60 since
/// 2026-11-16; 25 since 2026-11-07; 10 before). 10 measured comfortable on this Mac once the pump
/// holds new turns under memory pressure (a turn is 0.2 to 0.35 GB; a compile,
/// not a chat, is what fills memory): `outputs/chat-concurrency-2026-10-20.md`.
/// Raising the ceiling never raised what a person gets without asking: the
/// pump allows three bootstrap turns under memory pressure and holds later
/// starts, and the default stays [`DEFAULT_AT_ONCE`].
pub(crate) const MAX_AT_ONCE: u32 = 60;
/// The most chats Settings let run at once in one workspace's own folder
/// (25 since 2026-11-16; 10 since 2026-11-07; 5 before).
pub(crate) const MAX_PER_WORKSPACE: u32 = 25;
/// What a Hollow with no saved limits runs at once across Hollow. Unchanged
/// by the 2026-11-07 ceiling raise: the person sets more themselves.
pub(crate) const DEFAULT_AT_ONCE: u32 = 10;
/// What a Hollow with no saved limits runs at once in one workspace's folder.
pub(crate) const DEFAULT_PER_WORKSPACE: u32 = 5;
/// The most subagents Settings let one chat run at once (each is a whole
/// agent turn that also counts toward the two limits above, so the lower of
/// them binds first on a small Hollow).
pub(crate) const MAX_SUBAGENTS_PER_CHAT: u32 = 20;
/// What a Hollow with no saved limits runs at once from one chat (L1's
/// swarm size, the fixed cap before 2026-11-12).
pub(crate) const DEFAULT_SUBAGENTS_PER_CHAT: u32 = 6;
const _: () = assert!(
    crate::app_sessions::MAX_STARTS_PER_RUN > MAX_SUBAGENTS_PER_CHAT as usize,
    "a run's start guard stays above what one chat may run at once"
);

/// Where the person changes these, as every wait reason names it.
pub(crate) const WHERE: &str = "Settings → Chat → Chats at once";

const FILE: &str = "chat-limits.json";

/// The three limits, as saved and as Settings shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatLimits {
    /// Agent chat turns running at once across Hollow (1 to 60, default 10).
    pub chats_at_once: u32,
    /// Shared turns running at once in one workspace's own folder (1 to 25,
    /// default 5). A build worktree's turn is alone in its folder anyway.
    pub chats_per_workspace: u32,
    /// Subagents of one chat running at once (1 to 20, default 6). A further
    /// `start_session` from that chat waits in the queue for one to end.
    pub subagents_per_chat: u32,
}

impl Default for ChatLimits {
    fn default() -> Self {
        Self {
            chats_at_once: DEFAULT_AT_ONCE,
            chats_per_workspace: DEFAULT_PER_WORKSPACE,
            subagents_per_chat: DEFAULT_SUBAGENTS_PER_CHAT,
        }
    }
}

impl ChatLimits {
    /// Limits a person could have meant, or why not (Settings' save).
    pub(crate) fn validated(self) -> Result<Self, String> {
        if !(1..=MAX_AT_ONCE).contains(&self.chats_at_once) {
            return Err(format!("Run 1 to {MAX_AT_ONCE} chats at once."));
        }
        if !(1..=MAX_PER_WORKSPACE).contains(&self.chats_per_workspace) {
            return Err(format!(
                "Run 1 to {MAX_PER_WORKSPACE} chats at once in one workspace."
            ));
        }
        if !(1..=MAX_SUBAGENTS_PER_CHAT).contains(&self.subagents_per_chat) {
            return Err(format!(
                "Run 1 to {MAX_SUBAGENTS_PER_CHAT} subagents at once from one chat."
            ));
        }
        Ok(self)
    }

    /// Into range, whatever a hand-edited file says (reading never fails:
    /// the pump must always have limits).
    pub(crate) fn clamped(self) -> Self {
        Self {
            chats_at_once: self.chats_at_once.clamp(1, MAX_AT_ONCE),
            chats_per_workspace: self.chats_per_workspace.clamp(1, MAX_PER_WORKSPACE),
            subagents_per_chat: self.subagents_per_chat.clamp(1, MAX_SUBAGENTS_PER_CHAT),
        }
    }
}

fn path() -> Result<PathBuf, String> {
    Ok(crate::agent_settings::app_data_dir()?.join(FILE))
}

/// The limits saved at `path`, clamped; the defaults when there is no file
/// or Hollow can't read it.
fn read_at(path: &Path) -> ChatLimits {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<ChatLimits>(&bytes).ok())
        .unwrap_or_default()
        .clamped()
}

fn write_at(path: &Path, limits: &ChatLimits) -> Result<(), String> {
    let bytes =
        serde_json::to_vec_pretty(limits).map_err(|e| format!("Encode chat limits: {e}"))?;
    crate::vault::atomic_write(path, &bytes).map_err(|e| format!("Save chat limits: {e}"))
}

#[cfg(test)]
thread_local! {
    static TEST_LIMITS: std::cell::Cell<Option<ChatLimits>> = const { std::cell::Cell::new(None) };
}

/// Make [`current`] return `limits` on this thread (tests only; the default
/// is [`ChatLimits::default`], so a saved file never leaks into the suite).
#[cfg(test)]
pub(crate) fn set_test_limits(limits: ChatLimits) {
    TEST_LIMITS.with(|cell| cell.set(Some(limits)));
}

/// The limits in force now, read afresh (a small file, read once per pump
/// step and once per queue list). In tests, what [`set_test_limits`] set
/// on this thread, else the defaults.
pub(crate) fn current() -> ChatLimits {
    #[cfg(test)]
    {
        TEST_LIMITS.with(|cell| cell.get()).unwrap_or_default()
    }
    #[cfg(not(test))]
    {
        path().map(|p| read_at(&p)).unwrap_or_default()
    }
}

/// Settings, Chat, the chat limits.
#[tauri::command]
pub fn get_chat_limits() -> ChatLimits {
    current()
}

/// Save the chat limits (Settings). The pump reads them at its next step,
/// so a lowered limit holds new turns and never stops one that is running.
#[tauri::command]
pub fn set_chat_limits(limits: ChatLimits) -> Result<ChatLimits, String> {
    let limits = limits.validated()?;
    write_at(&path()?, &limits)?;
    crate::run_queue::wake();
    Ok(limits)
}

/// `base` with the limits a call names (a limit left out keeps its value),
/// validated as Settings validates a save.
pub(crate) fn merged(
    base: ChatLimits,
    chats_at_once: Option<u32>,
    chats_per_workspace: Option<u32>,
    subagents_per_chat: Option<u32>,
) -> Result<ChatLimits, String> {
    ChatLimits {
        chats_at_once: chats_at_once.unwrap_or(base.chats_at_once),
        chats_per_workspace: chats_per_workspace.unwrap_or(base.chats_per_workspace),
        subagents_per_chat: subagents_per_chat.unwrap_or(base.subagents_per_chat),
    }
    .validated()
}

/// `settings.set_chat_limits`' card: old to new, refused when invalid or
/// unchanged.
pub(crate) fn chat_limits_card(
    _cx: &crate::app_actions::Cx,
    a: &crate::app_actions::Args,
) -> Result<Option<crate::app_actions::Card>, String> {
    let get = |key: &str| a.0.get(key).and_then(|v| v.as_u64()).map(|n| n as u32);
    let old = current();
    let new = merged(
        old,
        get("chatsAtOnce"),
        get("chatsPerWorkspace"),
        get("subagentsPerChat"),
    )?;
    if new == old {
        return Err("Those are already the chat limits. Nothing changed.".into());
    }
    let mut card =
        crate::app_actions::Card::new("Hollow app tools: change how many chats run at once");
    if new.chats_at_once != old.chats_at_once {
        card = card.line(format!(
            "Chats at once across Hollow: {} → {}",
            old.chats_at_once, new.chats_at_once
        ));
    }
    if new.chats_per_workspace != old.chats_per_workspace {
        card = card.line(format!(
            "Chats at once in one workspace: {} → {}",
            old.chats_per_workspace, new.chats_per_workspace
        ));
    }
    if new.subagents_per_chat != old.subagents_per_chat {
        card = card.line(format!(
            "Subagents at once from one chat: {} → {}",
            old.subagents_per_chat, new.subagents_per_chat
        ));
    }
    if new.chats_at_once > old.chats_at_once
        || new.chats_per_workspace > old.chats_per_workspace
        || new.subagents_per_chat > old.subagents_per_chat
    {
        card = card.line("More chats at once use more memory and spend faster.");
    }
    Ok(Some(card))
}

/// `settings.set_chat_limits`: save the merged limits (after the card).
pub(crate) fn agent_set_limits(
    chats_at_once: Option<u32>,
    chats_per_workspace: Option<u32>,
    subagents_per_chat: Option<u32>,
) -> Result<ChatLimits, String> {
    let limits = merged(
        current(),
        chats_at_once,
        chats_per_workspace,
        subagents_per_chat,
    )?;
    write_at(&path()?, &limits)?;
    crate::run_queue::wake();
    Ok(limits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_ten_and_five_and_a_save_outside_the_range_is_refused() {
        // The ceiling is 60 and 25 (2026-11-16), but the defaults a person
        // gets without asking stay 10 and 5.
        assert_eq!(
            ChatLimits::default(),
            ChatLimits {
                chats_at_once: 10,
                chats_per_workspace: 5,
                ..ChatLimits::default()
            }
        );
        let ok = ChatLimits {
            chats_at_once: 3,
            chats_per_workspace: 2,
            ..Default::default()
        };
        assert_eq!(ok.validated(), Ok(ok));
        for (at_once, per) in [(0, 2), (61, 2), (3, 0), (3, 26)] {
            let bad = ChatLimits {
                chats_at_once: at_once,
                chats_per_workspace: per,
                ..Default::default()
            };
            assert!(bad.validated().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn subagents_per_chat_defaults_to_six_takes_one_to_twenty_and_reads_back_clamped() {
        // 2026-11-12: the fixed per-parent cap of 6 is a setting now.
        assert_eq!(ChatLimits::default().subagents_per_chat, 6);
        for n in [1, 2, 6, 10, 15, 20] {
            let ok = ChatLimits {
                subagents_per_chat: n,
                ..ChatLimits::default()
            };
            assert_eq!(ok.validated(), Ok(ok), "{n}");
        }
        for n in [0, 21] {
            let bad = ChatLimits {
                subagents_per_chat: n,
                ..ChatLimits::default()
            };
            assert_eq!(
                bad.validated(),
                Err("Run 1 to 20 subagents at once from one chat.".to_string()),
                "{n}"
            );
        }
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        // A file saved before the setting existed reads as the default.
        std::fs::write(&file, r#"{"chatsAtOnce": 12, "chatsPerWorkspace": 3}"#).unwrap();
        assert_eq!(read_at(&file).subagents_per_chat, 6);
        std::fs::write(&file, r#"{"subagentsPerChat": 40}"#).unwrap();
        assert_eq!(read_at(&file).subagents_per_chat, 20);
        std::fs::write(&file, r#"{"subagentsPerChat": 0}"#).unwrap();
        assert_eq!(read_at(&file).subagents_per_chat, 1);
        let saved = ChatLimits {
            subagents_per_chat: 2,
            ..ChatLimits::default()
        };
        write_at(&file, &saved).unwrap();
        assert_eq!(read_at(&file), saved);
        // The wire name the window and the COO use.
        assert!(std::fs::read_to_string(&file)
            .unwrap()
            .contains("\"subagentsPerChat\": 2"));
    }

    #[test]
    fn the_ceilings_are_sixty_twenty_five_and_twenty_and_both_ends_save() {
        // The old ceilings (10 and 5) are inside the range now, and each
        // new ceiling is the last accepted value.
        for (at_once, per) in [(1, 1), (10, 5), (11, 6), (25, 10), (60, 25)] {
            let ok = ChatLimits {
                chats_at_once: at_once,
                chats_per_workspace: per,
                ..Default::default()
            };
            assert_eq!(ok.validated(), Ok(ok), "{ok:?}");
        }
        let over_at_once = ChatLimits {
            chats_at_once: 61,
            chats_per_workspace: 5,
            ..Default::default()
        };
        assert_eq!(
            over_at_once.validated(),
            Err("Run 1 to 60 chats at once.".to_string())
        );
        let over_per_workspace = ChatLimits {
            chats_at_once: 10,
            chats_per_workspace: 26,
            ..Default::default()
        };
        assert_eq!(
            over_per_workspace.validated(),
            Err("Run 1 to 25 chats at once in one workspace.".to_string())
        );
        // The defaults sit inside the range and are not the ceilings.
        let defaults = ChatLimits::default();
        assert_eq!(defaults.validated(), Ok(defaults));
        assert!(defaults.chats_at_once < MAX_AT_ONCE);
        assert!(defaults.chats_per_workspace < MAX_PER_WORKSPACE);
    }

    #[test]
    fn a_saved_file_reads_back_clamped_and_a_broken_one_reads_as_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        assert_eq!(read_at(&file), ChatLimits::default(), "no file");
        let saved = ChatLimits {
            chats_at_once: 4,
            chats_per_workspace: 2,
            ..Default::default()
        };
        write_at(&file, &saved).unwrap();
        assert_eq!(read_at(&file), saved);
        std::fs::write(&file, r#"{"chatsAtOnce": 90, "chatsPerWorkspace": 0}"#).unwrap();
        assert_eq!(
            read_at(&file),
            ChatLimits {
                chats_at_once: 60,
                chats_per_workspace: 1,
                ..ChatLimits::default()
            },
            "a hand-edited file is clamped"
        );
        let top = ChatLimits {
            chats_at_once: 60,
            chats_per_workspace: 25,
            ..Default::default()
        };
        write_at(&file, &top).unwrap();
        assert_eq!(read_at(&file), top, "the new ceilings read back unclamped");
        std::fs::write(&file, r#"{"chatsAtOnce": 12, "chatsPerWorkspace": 40}"#).unwrap();
        assert_eq!(
            read_at(&file),
            ChatLimits {
                chats_at_once: 12,
                chats_per_workspace: 25,
                ..ChatLimits::default()
            },
            "above the old 10 is kept, above the new ceiling is clamped"
        );
        std::fs::write(&file, r#"{"chatsAtOnce": 7}"#).unwrap();
        assert_eq!(read_at(&file).chats_per_workspace, 5, "a missing field");
        std::fs::write(&file, "{not json").unwrap();
        assert_eq!(read_at(&file), ChatLimits::default());
    }

    #[test]
    fn current_reads_this_threads_limits_in_tests() {
        assert_eq!(current(), ChatLimits::default());
        let lowered = ChatLimits {
            chats_at_once: 3,
            chats_per_workspace: 2,
            ..Default::default()
        };
        set_test_limits(lowered);
        assert_eq!(current(), lowered);
        assert_eq!(get_chat_limits(), lowered);
    }

    #[test]
    fn a_call_changes_only_the_limit_it_names_and_is_validated() {
        let base = ChatLimits::default();
        let next = merged(base, Some(20), None, None).unwrap();
        assert_eq!(next.chats_at_once, 20);
        assert_eq!(next.chats_per_workspace, base.chats_per_workspace);
        assert_eq!(merged(base, None, None, None), Ok(base));
        assert!(merged(base, Some(61), None, None).is_err());
        assert!(merged(base, None, Some(0), None).is_err());
        assert!(merged(base, Some(0), Some(3), None).is_err());
        assert!(merged(base, None, None, Some(21)).is_err());
        let only_subagents = merged(base, None, None, Some(3)).unwrap();
        assert_eq!(only_subagents.subagents_per_chat, 3);
        assert_eq!(only_subagents.chats_at_once, base.chats_at_once);
        assert_eq!(only_subagents.chats_per_workspace, base.chats_per_workspace);
    }
}
