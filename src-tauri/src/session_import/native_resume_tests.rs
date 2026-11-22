//! H8 tests: sanitized fixtures and temp folders only. No real CLI is
//! started (the probe is injected) and no path under `$HOME` is read.

use super::native_resume::*;
use super::*;
use crate::session_store as store;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as AO};

const CLAUDE: &str = include_str!("../../fixtures/session_import/claude_code_session.jsonl");
const CODEX: &str = include_str!("../../fixtures/session_import/codex_rollout.jsonl");
const ID: &str = "11111111-2222-4333-8444-555555555555";
const PROJECT: &str = "/Users/sample/Projects/demo";
/// `claude --help` as listed by 2.1.326 (the lines that matter, sanitized).
const HELP: &str = "Options:\n  --fork-session                        When resuming, create a new session ID\n                                        instead of reusing the original (use\n                                        with --resume or --continue)\n  -r, --resume [value]                  Resume a conversation by session ID or\n                                        name, or pick from a list\n  --session-id <uuid>                   Use a specific session ID\n";

fn at() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-19T09:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn probed(help: &str) -> Probed {
    Probed {
        version: "2.1.326 (Claude Code)\n".into(),
        help: help.into(),
    }
}

fn capable() -> Result<Capability, String> {
    Ok(parse_help(&probed(HELP)))
}

struct Rig {
    tmp: tempfile::TempDir,
    vault: PathBuf,
    root: PathBuf,
    /// The source session's folder, a real folder in the temp dir.
    cwd: String,
}

impl Rig {
    fn new() -> Rig {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join(tmp.path().file_name().unwrap());
        std::fs::create_dir_all(&vault).unwrap();
        let project = tmp.path().join("work/demo");
        std::fs::create_dir_all(&project).unwrap();
        let cwd = project.to_string_lossy().into_owned();
        let root = tmp.path().join("claude/projects");
        std::fs::create_dir_all(root.join(project_slug(&cwd))).unwrap();
        Rig {
            tmp,
            vault,
            root,
            cwd,
        }
    }

    fn text(&self) -> String {
        CLAUDE.replace(PROJECT, &self.cwd)
    }

    fn file(&self) -> PathBuf {
        self.root
            .join(project_slug(&self.cwd))
            .join(format!("{ID}.jsonl"))
    }

    /// Writes the session file, imports it and returns the chat's key.
    fn import(&self) -> String {
        std::fs::write(self.file(), self.text()).unwrap();
        let staging = staging::Staging::new(self.tmp.path().join("app/Imports"));
        let projects: Vec<(String, PathBuf)> = Vec::new();
        let cx = Context {
            vault: &self.vault,
            staging: &staging,
            projects: &projects,
        };
        let id = staging::mint_id();
        inspect(
            &cx,
            &id,
            Source::ClaudeCode,
            &self.root,
            &walker::Budget::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
        commit(&cx, &id, &CommitRequest::default(), at())
            .unwrap()
            .session_keys[0]
            .clone()
    }
}

#[test]
fn the_help_text_decides_the_capability() {
    let c = parse_help(&probed(HELP));
    assert_eq!(c.version.as_deref(), Some("2.1.326 (Claude Code)"));
    assert!(c.resume && c.fork_session && c.supports_fork_resume());
    // Missing the fork flag: not capable. A flag only named in prose does not count.
    let old = parse_help(&probed(
        "  -r, --resume [value]   Resume\n  --continue  Use --fork-session someday\n",
    ));
    assert!(old.resume && !old.fork_session && !old.supports_fork_resume());
    // A longer flag that merely starts with the name is not the flag.
    let near = parse_help(&probed(
        "  --resume-session-at <id>  x\n  --fork-session-x  y\n",
    ));
    assert!(!near.resume && !near.fork_session);
    assert!(!parse_help(&probed("")).supports_fork_resume());
}

#[test]
fn the_probe_runs_once_per_key_and_failures_are_not_kept() {
    let cache = CapabilityCache::new();
    let calls = AtomicUsize::new(0);
    let probe = || {
        calls.fetch_add(1, AO::SeqCst);
        Ok(probed(HELP))
    };
    assert!(cache.get_or_probe("bin@1", probe).unwrap().fork_session);
    assert!(cache.get_or_probe("bin@1", probe).unwrap().fork_session);
    assert_eq!(calls.load(AO::SeqCst), 1);
    // A new binary (another key) probes again.
    cache.get_or_probe("bin@2", probe).unwrap();
    assert_eq!(calls.load(AO::SeqCst), 2);
    // A failed probe is returned and not kept.
    let fresh = CapabilityCache::new();
    assert!(fresh.get_or_probe("x", || Err("no claude".into())).is_err());
    assert!(fresh.get_or_probe("x", || Ok(probed(HELP))).is_ok());
}

#[test]
fn fork_arguments_are_built_only_for_a_safe_id() {
    assert_eq!(
        fork_args(ID).unwrap(),
        vec!["--resume", ID, "--fork-session"]
    );
    for bad in ["", "--dangerously-skip-permissions", "a b", "x/../y", "-r"] {
        assert!(fork_args(bad).is_none(), "{bad:?}");
    }
}

#[test]
fn the_slug_and_the_lookup_find_only_a_plain_file() {
    assert_eq!(project_slug("/Users/a b/x.y"), "-Users-a-b-x-y");
    let rig = Rig::new();
    assert!(locate(&rig.root, &rig.cwd, ID).is_none());
    std::fs::write(rig.file(), rig.text()).unwrap();
    assert_eq!(locate(&rig.root, &rig.cwd, ID), Some(rig.file()));
    // A cwd whose slug differs still finds it one folder below the root.
    assert_eq!(locate(&rig.root, "/elsewhere", ID), Some(rig.file()));
    // A link is never followed.
    std::fs::remove_file(rig.file()).unwrap();
    let real = rig.tmp.path().join("real.jsonl");
    std::fs::write(&real, rig.text()).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&real, rig.file()).unwrap();
        assert!(locate(&rig.root, &rig.cwd, ID).is_none());
    }
}

#[test]
fn the_head_must_name_the_same_id_and_folder() {
    let rig = Rig::new();
    std::fs::write(rig.file(), rig.text()).unwrap();
    assert!(head_matches(&rig.file(), ID, &rig.cwd));
    assert!(!head_matches(&rig.file(), "other-id", &rig.cwd));
    assert!(!head_matches(&rig.file(), ID, "/some/other/folder"));
    std::fs::write(rig.file(), "not json\n{\"type\":\"summary\"}\n").unwrap();
    assert!(!head_matches(&rig.file(), ID, &rig.cwd));
}

#[test]
fn an_intact_claude_import_is_offered_and_continues_without_writing_the_source() {
    let rig = Rig::new();
    let key = rig.import();
    let before = std::fs::read(rig.file()).unwrap();
    let modified = std::fs::metadata(rig.file()).unwrap().modified().unwrap();
    let offer = native_offer_with(&rig.vault, &key, &capable()).unwrap();
    assert!(offer.available, "{:?}", offer.reason);
    assert_eq!(offer.cwd.as_deref(), Some(rig.cwd.as_str()));
    assert_eq!(offer.cli_version.as_deref(), Some("2.1.326 (Claude Code)"));

    let request = NativeRequest {
        model: Some("sonnet".into()),
        mode: Some("plan".into()),
        effort: None,
    };
    let next = continue_native_with(&rig.vault, &key, &request, &capable(), at()).unwrap();
    assert_eq!(next.resume_session_id, ID);
    assert_eq!(next.chat.parent_key, key);
    assert_eq!(next.chat.agent, "claude");
    assert_eq!(next.chat.cwd, rig.cwd);
    assert_eq!(next.chat.mode, "plan");
    // The chat runs in the source folder and keeps fork_kind import, so a
    // failed resume falls back to the handoff bundle.
    let (cwd, kind): (String, String) = store::with_read(&rig.vault, |c| {
        c.query_row(
            "SELECT cwd, fork_kind FROM sessions WHERE key = ?1",
            [&next.chat.key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())
    })
    .unwrap();
    assert_eq!((cwd.as_str(), kind.as_str()), (rig.cwd.as_str(), "import"));
    assert!(continued_import(&rig.vault, &next.chat.key).is_some());
    // Only the source id forks; the fork's own id (after turn 1) resumes plainly.
    assert!(fork_pending(&rig.vault, &next.chat.key, ID));
    assert!(!fork_pending(
        &rig.vault,
        &next.chat.key,
        "the-forks-own-id"
    ));
    assert!(!fork_pending(&rig.vault, &key, ID));
    assert!(!fork_pending(&rig.vault, "../escape", ID));
    // The source file is byte for byte and time for time as it was.
    assert_eq!(std::fs::read(rig.file()).unwrap(), before);
    assert_eq!(
        std::fs::metadata(rig.file()).unwrap().modified().unwrap(),
        modified
    );
}

#[test]
fn every_failed_check_falls_back_to_the_handoff() {
    let rig = Rig::new();
    let key = rig.import();
    let refused = |cap: Result<Capability, String>| {
        let offer = native_offer_with(&rig.vault, &key, &cap).unwrap();
        assert!(!offer.available);
        let why = offer.reason.unwrap();
        let err = continue_native_with(&rig.vault, &key, &NativeRequest::default(), &cap, at())
            .unwrap_err();
        assert_eq!(err, why);
        why
    };
    // The CLI can't fork, or can't be probed.
    assert!(refused(Ok(parse_help(&probed("  -r, --resume [value]  x\n")))).contains("fork"));
    assert!(refused(Err("not installed".into())).contains("not installed"));
    // The source file moved, changed session, or its folder is gone.
    let original = std::fs::read_to_string(rig.file()).unwrap();
    std::fs::write(rig.file(), original.replace(&rig.cwd, "/gone/elsewhere")).unwrap();
    assert!(refused(capable()).contains("same session"));
    std::fs::remove_file(rig.file()).unwrap();
    assert!(refused(capable()).contains("isn't where"));
    std::fs::write(rig.file(), &original).unwrap();
    assert!(
        native_offer_with(&rig.vault, &key, &capable())
            .unwrap()
            .available
    );
    std::fs::remove_dir_all(&rig.cwd).unwrap();
    assert!(refused(capable()).contains("no longer exists"));
    // Nothing was created by any refusal, and the handoff still works.
    let natives: i64 = store::with_read(&rig.vault, |c| {
        c.query_row("SELECT COUNT(*) FROM import_native", [], |r| r.get(0))
            .map_err(|e| e.to_string())
    })
    .unwrap();
    assert_eq!(natives, 0);
    let handoff = ContinueRequest {
        agent: "claude".into(),
        ..Default::default()
    };
    assert!(continue_imported(&rig.vault, &key, &handoff, at()).is_ok());
}

#[test]
fn a_codex_import_and_a_hollow_chat_are_never_offered_natively() {
    let rig = Rig::new();
    let codex_root = rig.tmp.path().join("codex/sessions/2026/10/02");
    std::fs::create_dir_all(&codex_root).unwrap();
    std::fs::write(
        codex_root.join("rollout-2026-10-02T10-00-00-aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee.jsonl"),
        CODEX,
    )
    .unwrap();
    let staging = staging::Staging::new(rig.tmp.path().join("app/Imports"));
    let projects: Vec<(String, PathBuf)> = Vec::new();
    let cx = Context {
        vault: &rig.vault,
        staging: &staging,
        projects: &projects,
    };
    let id = staging::mint_id();
    inspect(
        &cx,
        &id,
        Source::Codex,
        &rig.tmp.path().join("codex/sessions"),
        &walker::Budget::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let key = commit(&cx, &id, &CommitRequest::default(), at())
        .unwrap()
        .session_keys[0]
        .clone();
    let offer = native_offer_with(&rig.vault, &key, &capable()).unwrap();
    assert!(!offer.available);
    assert!(offer.reason.unwrap().contains("Claude Code"));
    // A chat that was never imported is an error, not an offer.
    assert!(native_offer_with(&rig.vault, "s-nothing", &capable()).is_err());
}
