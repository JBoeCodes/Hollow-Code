//! H5a tests: both readers on sanitized fixtures (`src-tauri/fixtures/
//! session_import/`, hand-written, no real history), the walker's link and
//! budget rules, staging, commit, dedupe, exclusions, destination, cancel,
//! the 8 MB cap, Continue in Hollow and agent search. Every folder is a temp
//! dir; nothing here reads `$HOME`.

use super::*;
use crate::session_store as store;
use serde_json::Value;
use std::sync::atomic::AtomicBool;

const CLAUDE: &str = include_str!("../../fixtures/session_import/claude_code_session.jsonl");
const CODEX: &str = include_str!("../../fixtures/session_import/codex_rollout.jsonl");
const UNKNOWN: &str = include_str!("../../fixtures/session_import/unknown_layout.jsonl");
const CLAUDE_ID: &str = "11111111-2222-4333-8444-555555555555";
const CODEX_ID: &str = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
const PROJECT: &str = "/Users/sample/Projects/demo";

#[test]
fn pi_inspection_commit_and_dedupe_preserve_sources_and_read_only_history() {
    let rig = Rig::new();
    let root = rig.tmp.path().join("pi-sessions");
    let project = root.join("--demo--");
    std::fs::create_dir_all(&project).unwrap();
    let path = project.join("example_pi-sample.jsonl");
    let text = include_str!("../../fixtures/session_import/pi_session.jsonl");
    std::fs::write(&path, text).unwrap();
    let m = rig.inspect(Source::Pi, &root);
    assert_eq!(m.counts.new, 1);
    assert_eq!(m.conversations[0].other_branches, 1);
    assert_eq!(m.conversations[0].external_id, "pi:pi-sample");
    let result = rig.commit(&m, &CommitRequest::default());
    assert_eq!(result.imported, 1);
    assert_eq!(
        rig.count("SELECT COUNT(*) FROM sessions WHERE agent = 'imported'"),
        1
    );
    assert_eq!(
        rig.count("SELECT COUNT(*) FROM stints WHERE harness_session_id IS NOT NULL"),
        0
    );
    let again = rig.inspect(Source::Pi, &root);
    assert_eq!(again.counts.already_imported, 1);
    assert_eq!(
        rig.count("SELECT COUNT(*) FROM imports WHERE source = 'pi'"),
        1
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), text);
    assert_eq!(Source::parse("pi"), Some(Source::Pi));
    assert_eq!(
        default_root(Source::Pi, Path::new("/example"), None),
        Some(PathBuf::from("/example/.pi/agent/sessions"))
    );
}

/// A fixed time: tests never read the clock for what they compare.
fn at() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-18T09:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

struct Rig {
    tmp: tempfile::TempDir,
    vault: PathBuf,
    staging: Staging,
    projects: Vec<(String, PathBuf)>,
}

impl Rig {
    fn new() -> Rig {
        let tmp = tempfile::tempdir().unwrap();
        // The session store is keyed by the vault folder's name: one per test.
        let vault = tmp.path().join(tmp.path().file_name().unwrap());
        std::fs::create_dir_all(&vault).unwrap();
        let staging = Staging::new(tmp.path().join("app-support").join("Imports"));
        Rig {
            tmp,
            vault,
            staging,
            projects: Vec::new(),
        }
    }

    fn cx(&self) -> Context<'_> {
        Context {
            vault: &self.vault,
            staging: &self.staging,
            projects: &self.projects,
        }
    }

    /// A Claude Code projects folder holding `files` (`(id, text)`).
    fn claude_root(&self, files: &[(&str, &str)]) -> PathBuf {
        let root = self.tmp.path().join("home/.claude/projects");
        let dir = root.join("-Users-sample-Projects-demo");
        std::fs::create_dir_all(&dir).unwrap();
        for (id, text) in files {
            std::fs::write(dir.join(format!("{id}.jsonl")), text).unwrap();
        }
        root
    }

    fn codex_root(&self) -> PathBuf {
        let root = self.tmp.path().join("home/.codex/sessions");
        let dir = root.join("2026/10/02");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("rollout-2026-10-02T10-00-00-{CODEX_ID}.jsonl")),
            CODEX,
        )
        .unwrap();
        root
    }

    fn inspect(&self, source: Source, root: &Path) -> Manifest {
        let id = staging::mint_id();
        inspect(
            &self.cx(),
            &id,
            source,
            root,
            &walker::Budget::default(),
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    fn commit(&self, m: &Manifest, request: &CommitRequest) -> CommitResult {
        commit(&self.cx(), &m.import_id, request, at()).unwrap()
    }

    fn count(&self, sql: &str) -> i64 {
        store::with_read(&self.vault, |c| {
            c.query_row(sql, [], |r| r.get(0))
                .map_err(|e| e.to_string())
        })
        .unwrap()
    }
}

/// The Claude fixture with its session id replaced.
fn claude_as(id: &str) -> String {
    CLAUDE.replace(CLAUDE_ID, id)
}

fn turns_of(vault: &Path, key: &str) -> Vec<(i64, i64, String, String)> {
    store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT id, ord, asked FROM turns WHERE session_key = ?1 ORDER BY ord")
            .unwrap();
        let rows = stmt
            .query_map(params![key], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .map(|r| {
                let (id, ord, asked) = r.unwrap();
                (
                    id,
                    ord,
                    asked,
                    store::transcript_text(conn, key, ord).unwrap(),
                )
            })
            .collect();
        Ok(rows)
    })
    .unwrap()
}

// ── readers ───────────────────────────────────────────────────────────

#[test]
fn claude_code_reader_reads_the_fixture() {
    let rig = Rig::new();
    let root = rig.claude_root(&[(CLAUDE_ID, CLAUDE)]);
    let path = root
        .join("-Users-sample-Projects-demo")
        .join(format!("{CLAUDE_ID}.jsonl"));
    let p = claude_code::read(&path, model::CONVERSATION_MAX_BYTES).unwrap();
    assert!(p.recognized);
    assert_eq!(p.id.as_deref(), Some(CLAUDE_ID));
    assert_eq!(p.cwd.as_deref(), Some(PROJECT));
    assert_eq!(p.display_title(), "Fix the parser test");
    assert_eq!(p.created_at.as_deref(), Some("2026-10-01T09:00:00.000Z"));
    assert_eq!(p.last_at.as_deref(), Some("2026-10-01T09:05:04.000Z"));
    // Sidechain lines are a count, never turns.
    assert_eq!(p.sidechain, 2);
    assert_eq!(p.turns.len(), 2);
    assert_eq!(p.turn_hashes.len(), 2);
    assert_eq!(p.messages, 4);
    let first = &p.turns[0];
    assert_eq!(first.asked, "Why does the parser test fail?");
    assert_eq!(
        first.reply,
        "Let me read the test.\n\nThe test expects a trailing newline."
    );
    assert_eq!(first.tools.len(), 1);
    assert_eq!(first.tools[0].name, "Read");
    assert!(first.tools[0]
        .output
        .as_deref()
        .unwrap()
        .contains("parses a line"));
    // Images are listed by name only (N28).
    assert_eq!(p.turns[1].files, vec!["image 1".to_string()]);
    assert!(p.turns[1]
        .asked_text()
        .ends_with("[Attached in the original session, not copied: image 1]"));
    // One bad line and the last line cut off mid-write: counted, tolerated.
    assert_eq!(p.malformed, 2);
    assert!(p.truncated);
}

#[test]
fn codex_reader_reads_the_fixture() {
    let rig = Rig::new();
    let root = rig.codex_root();
    let path = root
        .join("2026/10/02")
        .join(format!("rollout-2026-10-02T10-00-00-{CODEX_ID}.jsonl"));
    let p = codex::read(&path, model::CONVERSATION_MAX_BYTES).unwrap();
    assert!(p.recognized && !p.subagent);
    assert_eq!(p.id.as_deref(), Some(CODEX_ID));
    assert_eq!(p.cwd.as_deref(), Some("/Users/sample/Projects/demo/app"));
    // Codex's injected environment context is not the person's words.
    assert_eq!(p.turns.len(), 2);
    assert_eq!(p.turns[0].asked, "List the files in src.");
    assert_eq!(p.turns[0].reply, "src has main.rs and lib.rs.");
    assert_eq!(p.turns[0].tools[0].name, "shell");
    assert_eq!(
        p.turns[0].tools[0].output.as_deref(),
        Some("main.rs\nlib.rs\n")
    );
    assert_eq!(p.turns[1].files, vec!["image 1".to_string()]);
    assert_eq!(p.display_title(), "List the files in src.");
    assert_eq!(p.malformed, 1);
    assert!(!p.truncated);
}

#[test]
fn an_unknown_layout_says_it_is_not_supported() {
    let rig = Rig::new();
    let root = rig.claude_root(&[("not-a-session", UNKNOWN)]);
    let m = rig.inspect(Source::ClaudeCode, &root);
    assert_eq!(m.unsupported.as_deref(), Some(UNSUPPORTED));
    assert_eq!(m.conversations[0].status, Status::Unsupported);
    assert!(!m.conversations[0].selected);
    let r = rig.commit(
        &m,
        &CommitRequest {
            ids: Some(vec![m.conversations[0].id.clone()]),
            ..CommitRequest::default()
        },
    );
    assert_eq!((r.imported, r.skipped), (0, 1));
    // A Codex file with no session meta is no rollout either.
    let dir = rig.tmp.path().join("codex-odd/2026/10/02");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("rollout-x.jsonl"), UNKNOWN).unwrap();
    let m = rig.inspect(Source::Codex, &rig.tmp.path().join("codex-odd"));
    assert_eq!(m.unsupported.as_deref(), Some(UNSUPPORTED));
    // An empty folder says so instead.
    let empty = rig.tmp.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let m = rig.inspect(Source::ClaudeCode, &empty);
    assert!(m.unsupported.is_none());
    assert_eq!(m.note.as_deref(), Some("No conversations in this folder."));
}

// ── commit and dedupe ─────────────────────────────────────────────────

#[test]
fn commit_writes_the_conversation_and_a_reimport_is_a_no_op() {
    let rig = Rig::new();
    let root = rig.claude_root(&[(CLAUDE_ID, CLAUDE)]);
    let m = rig.inspect(Source::ClaudeCode, &root);
    assert_eq!(m.conversations.len(), 1);
    let c = &m.conversations[0];
    assert_eq!(c.status, Status::New);
    assert!(c.selected);
    assert_eq!(c.title, "Fix the parser test");
    assert_eq!(c.message_count, 4);
    assert_eq!(c.project_folder.as_deref(), Some(PROJECT));
    assert_eq!(c.destination, crate::workspaces::QUICK_CHATS_ID);
    assert_eq!(
        (c.sidechain_lines, c.malformed_lines, c.truncated),
        (2, 2, true)
    );
    // Nothing is in the session store before the commit.
    assert_eq!(rig.count("SELECT COUNT(*) FROM sessions"), 0);

    let r = rig.commit(&m, &CommitRequest::default());
    assert_eq!(r.imported, 1);
    assert!(r.failed.is_empty() && r.staging_deleted);
    assert!(!rig.staging.dir(&m.import_id).unwrap().exists());
    let key = r.session_keys[0].clone();
    let summary = store::session_by_key(&rig.vault, &key).unwrap().unwrap();
    assert_eq!(summary.title.as_deref(), Some("Fix the parser test"));
    assert_eq!(summary.agent, "claude");
    assert!(summary.adopted);
    assert_eq!(summary.turn_count, 2);
    assert_eq!(summary.resumable, "none");
    assert!(summary.created_at.starts_with("2026-10-01T09:00:00"));
    let quick =
        crate::workspaces::session_dir_for(&rig.vault, crate::workspaces::QUICK_CHATS_ID).unwrap();
    assert_eq!(summary.cwd, quick.to_string_lossy());
    let imported = summary.imported.expect("provenance");
    assert_eq!(imported.label, "Claude Code");
    assert_eq!(imported.external_id, format!("claude-code:{CLAUDE_ID}"));
    assert_eq!(imported.version, 1);
    assert_eq!(
        imported.original_last_at.as_deref(),
        Some("2026-10-01T09:05:04.000Z")
    );
    assert!(imported.note.is_none());
    // The normalized payload: replies and tool calls, labelled, not rerun.
    let conversation = store::read_conversation(&rig.vault, &key).unwrap();
    assert_eq!(
        conversation
            .turns
            .iter()
            .filter(|t| t.role == "user")
            .count(),
        2
    );
    let reply = conversation
        .turns
        .iter()
        .find(|t| t.role == "assistant")
        .unwrap();
    assert!(reply.text.ends_with("The test expects a trailing newline."));
    assert_eq!(reply.tools[0].name, "Read");
    assert!(reply.tools[0]
        .result
        .as_deref()
        .unwrap()
        .starts_with(NOT_RERUN));

    // Inspecting again finds it imported and picks nothing; committing is a no-op.
    let again = rig.inspect(Source::ClaudeCode, &root);
    assert_eq!(again.conversations[0].status, Status::AlreadyImported);
    assert_eq!(
        again.conversations[0].existing_key.as_deref(),
        Some(key.as_str())
    );
    assert!(!again.conversations[0].selected);
    let r = rig.commit(
        &again,
        &CommitRequest {
            ids: Some(vec![again.conversations[0].id.clone()]),
            ..CommitRequest::default()
        },
    );
    assert_eq!((r.imported, r.already_imported), (0, 1));
    assert_eq!(rig.count("SELECT COUNT(*) FROM sessions"), 1);
    assert_eq!(rig.count("SELECT COUNT(*) FROM imports"), 1);
}

#[test]
fn an_extended_source_updates_only_its_changed_turns() {
    let rig = Rig::new();
    let root = rig.claude_root(&[(CLAUDE_ID, CLAUDE)]);
    let m = rig.inspect(Source::ClaudeCode, &root);
    let key = rig.commit(&m, &CommitRequest::default()).session_keys[0].clone();
    let before = turns_of(&rig.vault, &key);
    assert_eq!(before.len(), 2);
    // The session went on in Claude Code: one more exchange.
    let more = format!(
        "{}\n{}\n{}\n",
        CLAUDE.trim_end().rsplit_once('\n').unwrap().0,
        format_args!(
            r#"{{"type":"user","uuid":"u-10","sessionId":"{CLAUDE_ID}","cwd":"{PROJECT}","timestamp":"2026-10-02T08:00:00.000Z","message":{{"role":"user","content":"Now run the whole suite."}}}}"#
        ),
        format_args!(
            r#"{{"type":"assistant","uuid":"u-11","sessionId":"{CLAUDE_ID}","cwd":"{PROJECT}","timestamp":"2026-10-02T08:01:00.000Z","message":{{"role":"assistant","content":[{{"type":"text","text":"All 40 tests pass."}}]}}}}"#
        ),
    );
    let file = root
        .join("-Users-sample-Projects-demo")
        .join(format!("{CLAUDE_ID}.jsonl"));
    std::fs::write(&file, more).unwrap();
    let again = rig.inspect(Source::ClaudeCode, &root);
    assert_eq!(again.conversations[0].status, Status::Updated);
    assert!(again.conversations[0].selected);
    let r = rig.commit(&again, &CommitRequest::default());
    assert_eq!((r.updated, r.imported), (1, 0));
    assert_eq!(r.session_keys, vec![key.clone()]);
    let after = turns_of(&rig.vault, &key);
    assert_eq!(after.len(), 3);
    // The unchanged source turns keep their rows; only the new one is written.
    assert_eq!(after[0].0, before[0].0);
    assert_eq!(after[1].0, before[1].0);
    assert_eq!(after[2].2, "Now run the whole suite.");
    assert_eq!(after[2].3, "All 40 tests pass.");
    assert_eq!(rig.count("SELECT COUNT(*) FROM sessions"), 1);
    let summary = store::session_by_key(&rig.vault, &key).unwrap().unwrap();
    assert_eq!(summary.turn_count, 3);
    assert!(summary.last_activity_at.starts_with("2026-10-02T08:01:00"));
    let note = summary.imported.unwrap().note.unwrap();
    assert!(note.starts_with("Updated from Claude Code"), "{note}");
}

#[test]
fn a_source_that_does_not_extend_becomes_version_2() {
    let rig = Rig::new();
    let root = rig.claude_root(&[(CLAUDE_ID, CLAUDE)]);
    let m = rig.inspect(Source::ClaudeCode, &root);
    let first = rig.commit(&m, &CommitRequest::default()).session_keys[0].clone();
    let file = root
        .join("-Users-sample-Projects-demo")
        .join(format!("{CLAUDE_ID}.jsonl"));
    std::fs::write(
        &file,
        CLAUDE.replace("Why does the parser test fail?", "Rewrite the parser."),
    )
    .unwrap();
    let again = rig.inspect(Source::ClaudeCode, &root);
    assert_eq!(again.conversations[0].status, Status::NewVersion);
    assert_eq!(again.conversations[0].version, Some(2));
    let r = rig.commit(&again, &CommitRequest::default());
    assert_eq!(r.new_versions, 1);
    let second = r.session_keys[0].clone();
    assert_ne!(second, first);
    let v2 = store::session_by_key(&rig.vault, &second).unwrap().unwrap();
    assert_eq!(v2.title.as_deref(), Some("Fix the parser test (Version 2)"));
    let p = v2.imported.unwrap();
    assert_eq!(
        (p.version, p.external_id.as_str()),
        (2, &*format!("claude-code:{CLAUDE_ID}#v2"))
    );
    // The first copy is untouched.
    assert_eq!(
        turns_of(&rig.vault, &first)[0].2,
        "Why does the parser test fail?"
    );
    // Importing the same rewrite again is a no-op against version 2.
    let third = rig.inspect(Source::ClaudeCode, &root);
    assert_eq!(third.conversations[0].status, Status::AlreadyImported);
}

#[test]
fn hollow_own_sessions_are_never_imported() {
    let rig = Rig::new();
    // A Hollow chat whose Claude session is the fixture's, and one on Codex.
    let cwd = rig.vault.to_string_lossy().into_owned();
    store::start_turn(
        &rig.vault,
        "s-20261118-own00001",
        "claude",
        &cwd,
        "r1",
        "hi",
    )
    .unwrap();
    store::record_harness_id(&rig.vault, "s-20261118-own00001", CLAUDE_ID).unwrap();
    store::start_turn(&rig.vault, "s-20261118-own00002", "codex", &cwd, "r2", "hi").unwrap();
    store::record_harness_id(&rig.vault, "s-20261118-own00002", CODEX_ID).unwrap();
    for (source, root) in [
        (Source::ClaudeCode, rig.claude_root(&[(CLAUDE_ID, CLAUDE)])),
        (Source::Codex, rig.codex_root()),
    ] {
        let m = rig.inspect(source, &root);
        assert_eq!(m.conversations[0].status, Status::HollowOwn, "{source:?}");
        assert!(!m.conversations[0].selected);
        assert_eq!(m.counts.hollow_own, 1);
        // Even picked by hand, it is skipped.
        let r = rig.commit(
            &m,
            &CommitRequest {
                ids: Some(vec![m.conversations[0].id.clone()]),
                ..CommitRequest::default()
            },
        );
        assert_eq!((r.imported, r.skipped), (0, 1));
    }
    assert_eq!(rig.count("SELECT COUNT(*) FROM imports"), 0);
}

#[test]
fn a_crash_between_conversations_leaves_no_half_row() {
    let rig = Rig::new();
    let other = "22222222-2222-4333-8444-555555555555";
    let root = rig.claude_root(&[(CLAUDE_ID, CLAUDE), (other, &claude_as(other))]);
    let m = rig.inspect(Source::ClaudeCode, &root);
    assert_eq!(m.counts.new, 2);
    let doomed = format!("-Users-sample-Projects-demo/{other}.jsonl");
    let hook = |stage: &str, entry: &str| {
        if stage == "turns" && entry == doomed {
            Err("simulated crash".to_string())
        } else {
            Ok(())
        }
    };
    let r = commit_with(
        &rig.cx(),
        &m.import_id,
        &CommitRequest::default(),
        at(),
        &hook,
    )
    .unwrap();
    assert_eq!(r.imported, 1);
    assert_eq!(r.failed.len(), 1);
    assert_eq!(r.failed[0].id, doomed);
    // The failed conversation left nothing: no session, turns, transcripts or record.
    assert_eq!(rig.count("SELECT COUNT(*) FROM sessions"), 1);
    assert_eq!(rig.count("SELECT COUNT(*) FROM imports"), 1);
    assert_eq!(rig.count("SELECT COUNT(*) FROM turns"), 2);
    assert_eq!(rig.count("SELECT COUNT(*) FROM transcripts"), 2);
    // Staging stays for Retry failed, with only the failed one picked.
    let kept = manifest(&rig.staging, &m.import_id).unwrap();
    assert_eq!(kept.state, "committed");
    let picked: Vec<&str> = kept
        .conversations
        .iter()
        .filter(|c| c.selected)
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(picked, vec![doomed.as_str()]);
    let retry = rig.commit(&kept, &CommitRequest::default());
    assert_eq!((retry.imported, retry.failed.len()), (1, 0));
    assert!(retry.staging_deleted);
    assert_eq!(rig.count("SELECT COUNT(*) FROM imports"), 2);
}

#[test]
fn cancel_discards_the_staged_manifest() {
    let rig = Rig::new();
    let root = rig.claude_root(&[(CLAUDE_ID, CLAUDE)]);
    // A pass cancelled while it runs stages nothing.
    let id = staging::mint_id();
    let cancel_flag = begin_job(&id, Source::ClaudeCode);
    cancel_flag.store(true, Ordering::Relaxed);
    let result = inspect(
        &rig.cx(),
        &id,
        Source::ClaudeCode,
        &root,
        &walker::Budget::default(),
        &cancel_flag,
    );
    end_job(&id, &result);
    assert_eq!(result.unwrap_err(), CANCELLED);
    assert!(!rig.staging.dir(&id).unwrap().exists());
    assert_eq!(
        job_status(&rig.staging, &id).unwrap().unwrap().state,
        "cancelled"
    );
    // A ready one is deleted by Cancel.
    let m = rig.inspect(Source::ClaudeCode, &root);
    assert!(rig
        .staging
        .dir(&m.import_id)
        .unwrap()
        .join(MANIFEST)
        .exists());
    assert!(cancel(&rig.staging, &m.import_id).unwrap());
    assert!(!rig.staging.dir(&m.import_id).unwrap().exists());
    assert!(manifest(&rig.staging, &m.import_id).is_err());
    assert!(!cancel(&rig.staging, &m.import_id).unwrap());
    assert_eq!(rig.count("SELECT COUNT(*) FROM sessions"), 0);
    // An id that isn't one is refused before it names a folder.
    assert!(cancel(&rig.staging, "../outside").is_err());
}

#[test]
fn staging_lives_outside_the_vault_and_old_imports_are_swept() {
    let rig = Rig::new();
    let root = rig.claude_root(&[(CLAUDE_ID, CLAUDE)]);
    let m = rig.inspect(Source::ClaudeCode, &root);
    let dir = rig.staging.dir(&m.import_id).unwrap();
    assert!(dir.join(MANIFEST).is_file());
    assert!(!dir.starts_with(&rig.vault));
    // The app's own staging root is app support's, never a vault's.
    let default = staging::default_root().unwrap();
    assert!(!default.starts_with(&rig.vault));
    // Swept after seven days; kept before.
    let now = std::time::SystemTime::now();
    assert!(staging::sweep(&rig.staging, now).is_empty());
    let later = now + std::time::Duration::from_secs(8 * 24 * 60 * 60);
    assert_eq!(
        staging::sweep(&rig.staging, later),
        vec![m.import_id.clone()]
    );
    assert!(!dir.exists());
}

#[test]
fn the_eight_mb_cap_shortens_the_oldest_turns() {
    let rig = Rig::new();
    let id = "33333333-2222-4333-8444-555555555555";
    let asked = "q".repeat(60_000);
    let reply = "a".repeat(60_000);
    let mut text = String::new();
    for i in 0..80 {
        text.push_str(&format!(
            r#"{{"type":"user","sessionId":"{id}","cwd":"{PROJECT}","timestamp":"2026-10-03T09:{:02}:00.000Z","message":{{"role":"user","content":"{i} {asked}"}}}}"#,
            i % 60
        ));
        text.push('\n');
        text.push_str(&format!(
            r#"{{"type":"assistant","sessionId":"{id}","cwd":"{PROJECT}","timestamp":"2026-10-03T09:{:02}:30.000Z","message":{{"role":"assistant","content":[{{"type":"text","text":"{reply}"}}]}}}}"#,
            i % 60
        ));
        text.push('\n');
    }
    let root = rig.claude_root(&[(id, &text)]);
    let p = claude_code::read(
        &root
            .join("-Users-sample-Projects-demo")
            .join(format!("{id}.jsonl")),
        model::CONVERSATION_MAX_BYTES,
    )
    .unwrap();
    assert!(p.dropped_turns > 0);
    assert_eq!(p.turn_hashes.len(), 80);
    assert_eq!(p.turns.len() + p.dropped_turns, 80);
    let kept: usize = p.turns.iter().map(model::ParsedTurn::weight).sum();
    assert!(kept <= model::CONVERSATION_MAX_BYTES);
    // The newest turn is kept.
    assert!(p.turns.last().unwrap().asked.starts_with("79 "));
    let m = rig.inspect(Source::ClaudeCode, &root);
    assert_eq!(m.conversations[0].shortened_turns, p.dropped_turns);
    let key = rig.commit(&m, &CommitRequest::default()).session_keys[0].clone();
    let turns = turns_of(&rig.vault, &key);
    assert_eq!(turns.len(), p.turns.len() + 1);
    assert!(
        turns[0].3.starts_with("Shortened: the oldest"),
        "{}",
        turns[0].3
    );
}

// ── walker ────────────────────────────────────────────────────────────

#[test]
fn the_walker_never_follows_a_link_and_keeps_to_its_budgets() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(root.join("a/b")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.jsonl"), "{}\n").unwrap();
    std::fs::write(root.join("a/one.jsonl"), "{}\n").unwrap();
    std::fs::write(root.join("a/b/two.jsonl"), "{}\n{}\n").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("a/linked-dir")).unwrap();
    std::os::unix::fs::symlink(outside.join("secret.jsonl"), root.join("a/linked.jsonl")).unwrap();
    let never = AtomicBool::new(false);
    let all = |_: &walker::Entry| true;
    let w = walker::walk(&root, &walker::Budget::default(), &never, all).unwrap();
    let rels: Vec<String> = w.entries.iter().map(walker::Entry::rel_path).collect();
    assert_eq!(rels, vec!["a/one.jsonl", "a/b/two.jsonl"]);
    assert_eq!(w.symlinks, 2);
    assert!(w.entries.iter().all(|e| e.path.starts_with(&root)));
    assert!(w.stopped.is_none());
    // A root that is itself a link is refused.
    let link_root = tmp.path().join("link-root");
    std::os::unix::fs::symlink(&root, &link_root).unwrap();
    assert!(walker::walk(&link_root, &walker::Budget::default(), &never, all).is_err());
    // Each cap stops the walk and says which.
    let cap = |b: walker::Budget| walker::walk(&root, &b, &never, all).unwrap();
    let base = walker::Budget::default();
    let w = cap(walker::Budget {
        max_files: 1,
        ..base
    });
    assert_eq!((w.entries.len(), w.stopped), (1, Some(walker::Stop::Files)));
    let w = cap(walker::Budget {
        max_bytes: 4,
        ..base
    });
    assert_eq!((w.entries.len(), w.stopped), (1, Some(walker::Stop::Bytes)));
    let w = cap(walker::Budget {
        max_visited: 1,
        ..base
    });
    assert_eq!(w.stopped, Some(walker::Stop::Visited));
    let w = cap(walker::Budget {
        max_time: std::time::Duration::ZERO,
        ..base
    });
    assert_eq!((w.entries.len(), w.stopped), (0, Some(walker::Stop::Time)));
    let w = cap(walker::Budget {
        max_depth: 1,
        ..base
    });
    assert_eq!((w.entries.len(), w.too_deep), (1, 1));
    // A consumer's filter decides what is kept (CX19 brings its own).
    let w = walker::walk(&root, &base, &never, |e| e.name() == "two.jsonl").unwrap();
    assert_eq!(w.entries.len(), 1);
    assert_eq!(w.entries[0].kind, walker::Kind::JsonLines);
}

// ── destination ───────────────────────────────────────────────────────

#[test]
fn destination_is_the_connected_project_by_cwd_else_quick_chats() {
    let projects = vec![
        ("local-demo".to_string(), PathBuf::from(PROJECT)),
        (
            "local-app".to_string(),
            PathBuf::from("/Users/sample/Projects/demo/app"),
        ),
        (
            "local-gone".to_string(),
            PathBuf::from("/Users/sample/Projects/gone"),
        ),
    ];
    let has = |id: &str| id != "local-gone";
    let quick = crate::workspaces::QUICK_CHATS_ID;
    assert_eq!(destination_for(Some(PROJECT), &projects, has), "local-demo");
    assert_eq!(
        destination_for(Some("/Users/sample/Projects/demo/src"), &projects, has),
        "local-demo"
    );
    // The deepest project wins.
    assert_eq!(
        destination_for(Some("/Users/sample/Projects/demo/app/x"), &projects, has),
        "local-app"
    );
    // A prefix that isn't a whole folder name is no match.
    assert_eq!(
        destination_for(Some("/Users/sample/Projects/demolition"), &projects, has),
        quick
    );
    // A cwd that climbs out with `..` is no match for the folder it starts with.
    assert_eq!(
        destination_for(
            Some("/Users/sample/Projects/demo/../../elsewhere"),
            &projects,
            has
        ),
        quick
    );
    // A project with no workspace, and no cwd at all: Quick chats.
    assert_eq!(
        destination_for(Some("/Users/sample/Projects/gone/a"), &projects, has),
        quick
    );
    assert_eq!(destination_for(None, &projects, has), quick);

    // Through inspect and commit, with a workspace for the project.
    let mut rig = Rig::new();
    crate::workspaces::ensure_named(&rig.vault, "local-demo", "Demo", "The demo project.").unwrap();
    rig.projects = vec![("local-demo".to_string(), PathBuf::from(PROJECT))];
    let root = rig.claude_root(&[(CLAUDE_ID, CLAUDE)]);
    let m = rig.inspect(Source::ClaudeCode, &root);
    assert_eq!(m.conversations[0].destination, "local-demo");
    let key = rig.commit(&m, &CommitRequest::default()).session_keys[0].clone();
    let dir = crate::workspaces::session_dir_for(&rig.vault, "local-demo").unwrap();
    let summary = store::session_by_key(&rig.vault, &key).unwrap().unwrap();
    assert_eq!(summary.cwd, dir.to_string_lossy());
    // The batch override puts every conversation in the workspace picked.
    let m = rig.inspect(Source::Codex, &rig.codex_root());
    assert_eq!(m.conversations[0].destination, "local-demo");
    let r = rig.commit(
        &m,
        &CommitRequest {
            destination: Some(quick.to_string()),
            ..CommitRequest::default()
        },
    );
    let summary = store::session_by_key(&rig.vault, &r.session_keys[0])
        .unwrap()
        .unwrap();
    let quick_dir = crate::workspaces::session_dir_for(&rig.vault, quick).unwrap();
    assert_eq!(summary.cwd, quick_dir.to_string_lossy());
    assert_eq!(summary.agent, "codex");
    // The COO's workspace is refused as a destination.
    let m = rig.inspect(
        Source::ClaudeCode,
        &rig.claude_root(&[(
            "44444444-2222-4333-8444-555555555555",
            &claude_as("44444444-2222-4333-8444-555555555555"),
        )]),
    );
    let err = commit(
        &rig.cx(),
        &m.import_id,
        &CommitRequest {
            destination: Some(crate::workspaces::MAIN_AGENT_ID.to_string()),
            ..CommitRequest::default()
        },
        at(),
    )
    .unwrap_err();
    assert!(err.contains("COO"), "{err}");
}

#[test]
fn default_roots_come_from_the_home_folder_and_codex_home() {
    let home = Path::new("/tmp/fake-home");
    assert_eq!(
        default_root(Source::ClaudeCode, home, None).unwrap(),
        PathBuf::from("/tmp/fake-home/.claude/projects")
    );
    assert_eq!(
        default_root(Source::Codex, home, None).unwrap(),
        PathBuf::from("/tmp/fake-home/.codex/sessions")
    );
    assert_eq!(
        default_root(Source::Codex, home, Some(Path::new("/tmp/codex-home"))).unwrap(),
        PathBuf::from("/tmp/codex-home/sessions")
    );
    // An export source has no folder: the person picks the file.
    assert_eq!(default_root(Source::ChatGpt, home, None), None);
}

// ── read-only record, Continue in Hollow, search ──────────────────────

#[test]
fn continue_in_hollow_hands_the_imported_turns_over_as_quoted_history() {
    let rig = Rig::new();
    let root = rig.claude_root(&[(CLAUDE_ID, CLAUDE)]);
    let m = rig.inspect(Source::ClaudeCode, &root);
    let key = rig.commit(&m, &CommitRequest::default()).session_keys[0].clone();
    // The imported record is read-only.
    let cwd = rig.vault.to_string_lossy().into_owned();
    let err = store::start_turn(&rig.vault, &key, "claude", &cwd, "run-x", "more").unwrap_err();
    assert_eq!(err, READ_ONLY);
    // Continue: the person's agent, model, workspace and mode.
    let request = ContinueRequest {
        agent: "codex".into(),
        model: Some("gpt-5".into()),
        workspace_id: None,
        mode: Some("plan".into()),
        effort: Some("high".into()),
    };
    let next = continue_imported(&rig.vault, &key, &request, at()).unwrap();
    assert_ne!(next.key, key);
    assert_eq!(next.parent_key, key);
    assert_eq!((next.agent.as_str(), next.mode.as_str()), ("codex", "plan"));
    assert_eq!(next.workspace_id, crate::workspaces::QUICK_CHATS_ID);
    assert_eq!(next.title, "Fix the parser test (continued)");
    let summary = store::session_by_key(&rig.vault, &next.key)
        .unwrap()
        .unwrap();
    assert_eq!(summary.parent_key.as_deref(), Some(key.as_str()));
    assert_eq!(summary.fork_kind.as_deref(), Some(FORK_KIND));
    assert!(summary.imported.is_none() && !summary.adopted);
    assert_eq!(
        continued_import(&rig.vault, &next.key),
        Some((key.clone(), "Claude Code".to_string()))
    );
    assert!(continued_import(&rig.vault, &key).is_none());
    // Its first turn's fresh session carries the imported turns, marked.
    let ord =
        store::start_turn(&rig.vault, &next.key, "codex", &next.cwd, "run-1", "Go on").unwrap();
    let body = crate::context_compiler::compile_turn(
        &rig.vault,
        &next.key,
        &[],
        ord,
        crate::context_compiler::Entry::Composer,
        crate::context_compiler::Session::New,
        false,
        crate::task_state::Reader::AppTools,
        "Go on",
    )
    .body;
    assert!(body.starts_with("<conversation-context>"), "{body}");
    assert!(body.contains("Why does the parser test fail?"), "{body}");
    assert!(body.contains(&handoff_agent("Claude Code")), "{body}");
    assert!(body.ends_with("Go on"));
    // The imported record is unchanged.
    assert_eq!(turns_of(&rig.vault, &key).len(), 2);
    // Refusals: an unknown agent, a chat that wasn't imported, the COO.
    let bad = ContinueRequest {
        agent: "nope".into(),
        ..ContinueRequest::default()
    };
    assert!(continue_imported(&rig.vault, &key, &bad, at()).is_err());
    let ok = ContinueRequest {
        agent: "claude".into(),
        ..ContinueRequest::default()
    };
    assert!(continue_imported(&rig.vault, &next.key, &ok, at()).is_err());
    let coo = ContinueRequest {
        workspace_id: Some(crate::workspaces::MAIN_AGENT_ID.into()),
        ..ok
    };
    assert!(continue_imported(&rig.vault, &key, &coo, at()).is_err());
}

#[test]
fn agent_search_finds_imports_unless_kept_out() {
    use crate::session_store::MessageText;
    let rig = Rig::new();
    let root = rig.claude_root(&[(CLAUDE_ID, CLAUDE)]);
    let m = rig.inspect(Source::ClaudeCode, &root);
    let r = rig.commit(
        &m,
        &CommitRequest {
            keep_out_of_search: true,
            ..CommitRequest::default()
        },
    );
    let key = r.session_keys[0].clone();
    let search = |text| {
        crate::workspaces::search_sessions_in(
            &rig.vault,
            "trailing newline",
            None,
            None,
            text,
            None,
        )
        .unwrap()
        .items
    };
    // The person's own search still finds it, with its provenance.
    let mine = search(MessageText::AsWritten);
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].session.key, key);
    assert!(
        mine[0]
            .session
            .imported
            .as_ref()
            .unwrap()
            .keep_out_of_search
    );
    // The agent's search doesn't, until the switch is turned off.
    assert!(search(MessageText::Redacted).is_empty());
    assert_eq!(
        set_keep_out_of_search(&rig.vault, &m.import_id, false).unwrap(),
        1
    );
    let agents = search(MessageText::Redacted);
    assert_eq!(agents.len(), 1);
    assert_eq!(
        agents[0].session.imported.as_ref().unwrap().label,
        "Claude Code"
    );
    let b = batches(&rig.vault).unwrap();
    assert_eq!(
        (b.len(), b[0].conversations, b[0].keep_out_of_search),
        (1, 1, false)
    );
}

#[test]
fn preview_shows_a_bounded_slice() {
    let rig = Rig::new();
    let root = rig.claude_root(&[(CLAUDE_ID, CLAUDE)]);
    let m = rig.inspect(Source::ClaudeCode, &root);
    let p = preview(&rig.staging, &m.import_id, &m.conversations[0].id).unwrap();
    assert_eq!(p.total_turns, 2);
    assert_eq!(p.turns[0].asked, "Why does the parser test fail?");
    assert_eq!(p.turns[0].tools, vec!["Read".to_string()]);
    assert!(preview(&rig.staging, &m.import_id, "../../etc/passwd").is_err());
}

// ── H5b: ChatGPT export ───────────────────────────────────────────────

const CHATGPT: &str = include_str!("../../fixtures/session_import/chatgpt_conversations.json");
const GPT_1: &str = "conv-sample-0001";
const GPT_2: &str = "conv-sample-0002";

impl Rig {
    /// A ChatGPT export zip holding `conversations.json` and two
    /// attachments, built here.
    fn chatgpt_zip(&self, name: &str, extra: &[(&str, &[u8])]) -> PathBuf {
        let path = self.tmp.path().join(name);
        let mut entries: Vec<(&str, &[u8])> = vec![
            ("conversations.json", CHATGPT.as_bytes()),
            ("file-sample-2-garden-plan.pdf", b"%PDF sample"),
            ("user.json", b"{\"id\":\"user-sample\"}"),
        ];
        entries.extend_from_slice(extra);
        archive::tests::build_zip(&path, &entries);
        path
    }

    fn try_inspect(&self, source: Source, root: &Path) -> Result<Manifest, String> {
        let id = staging::mint_id();
        inspect(
            &self.cx(),
            &id,
            source,
            root,
            &walker::Budget::default(),
            &AtomicBool::new(false),
        )
    }
}

fn gpt_value() -> Vec<Value> {
    serde_json::from_str(CHATGPT).unwrap()
}

#[test]
fn chatgpt_reader_follows_current_node_and_names_what_it_skips() {
    let convs = gpt_value();
    let p = chatgpt::parse(&convs[0], model::CONVERSATION_MAX_BYTES);
    assert!(p.recognized);
    assert_eq!(p.id.as_deref(), Some(GPT_1));
    assert_eq!(p.display_title(), "Plan a garden");
    assert_eq!(p.created_at.as_deref(), Some("2023-11-14T22:13:20+00:00"));
    assert_eq!(p.last_at.as_deref(), Some("2023-11-14T22:14:20.500+00:00"));
    // The edited-prompt branch is counted, not imported.
    assert_eq!(p.other_branches, 1);
    // Hidden system node and Custom Instructions context are not turns.
    assert_eq!(p.turns.len(), 2);
    let first = &p.turns[0];
    assert_eq!(first.asked, "How do I start a herb garden?");
    assert_eq!(first.reply, "Start with a sunny spot and good drainage.");
    // The image and the named upload are listed by name only (N28).
    assert_eq!(
        first.files,
        vec!["garden-plan.pdf".to_string(), "image 2".to_string()]
    );
    let second = &p.turns[1];
    assert_eq!(second.asked, "Which herbs?");
    assert_eq!(second.reply, "Basil and mint, two packets each.");
    assert!(!second.reply.contains("shade"));
    // The code call and its output are a labelled excerpt.
    assert_eq!(second.tools.len(), 1);
    assert_eq!(second.tools[0].name, "python");
    assert_eq!(second.tools[0].output.as_deref(), Some("2"));
    assert!(second.tools[0].input.to_string().contains("print(1 + 1)"));
    assert_eq!(p.malformed, 0);

    // No current_node: the newest leaf stands in; null parts and an unknown
    // content type become a label, never a panic.
    let q = chatgpt::parse(&convs[1], model::CONVERSATION_MAX_BYTES);
    assert!(q.recognized);
    assert_eq!(q.display_title(), "Hello there");
    assert_eq!(q.turns.len(), 1);
    assert_eq!(
        q.turns[0].reply,
        "[hologram content, not imported]\n\nHi! How can I help?"
    );
    assert_eq!(q.malformed, 1);

    // Anything else is unrecognized, not a failure.
    for odd in [&convs[2], &convs[3], &convs[4]] {
        assert!(!chatgpt::parse(odd, 1_000).recognized);
    }
}

#[test]
fn a_loop_in_a_damaged_tree_stops() {
    let conv: Value = serde_json::json!({
        "id": "loop-1", "current_node": "a",
        "mapping": {
            "a": {"parent": "b", "children": [], "message": {"author": {"role": "user"}, "content": {"content_type": "text", "parts": ["one"]}}},
            "b": {"parent": "a", "children": ["a"], "message": {"author": {"role": "assistant"}, "content": {"content_type": "text", "parts": ["two"]}}},
            "c": "not a node",
        }
    });
    let p = chatgpt::parse(&conv, 1_000_000);
    // It ends (each node is read once) instead of spinning.
    assert!(p.recognized);
    assert_eq!(p.turns.len(), 2);
}

#[test]
fn a_chatgpt_zip_inspects_stages_and_imports() {
    let rig = Rig::new();
    let zip = rig.chatgpt_zip("export.zip", &[]);
    let m = rig.inspect(Source::ChatGpt, &zip);
    assert_eq!(m.state, "ready");
    assert_eq!(m.source_label, "ChatGPT");
    assert_eq!(m.counts.new, 2);
    assert_eq!(m.conversations.len(), 2);
    let export = m.export.clone().unwrap();
    assert_eq!(export.name, "export.zip");
    assert_eq!(export.shards, 1);
    assert_eq!(export.malformed, 3);
    // Attachments and other files are listed by name, never copied.
    assert_eq!(
        export.other_files,
        vec![
            "file-sample-2-garden-plan.pdf".to_string(),
            "user.json".to_string()
        ]
    );
    let staged: Vec<String> = std::fs::read_dir(rig.staging.dir(&m.import_id).unwrap())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        staged.iter().all(|n| n == "conversations" || n == MANIFEST),
        "{staged:?}"
    );
    let one = m
        .conversations
        .iter()
        .find(|c| c.external_id == format!("chatgpt:{GPT_1}"))
        .unwrap();
    assert_eq!(one.other_branches, 1);
    assert_eq!(one.destination, crate::workspaces::QUICK_CHATS_ID);
    assert!(one.selected && one.status == Status::New);
    assert_eq!(one.message_count, 4);

    // Preview carries the branch count.
    let preview = preview(&rig.staging, &m.import_id, &one.id).unwrap();
    assert_eq!(preview.other_branches, 1);
    assert_eq!(preview.total_turns, 2);

    let done = rig.commit(&m, &CommitRequest::default());
    assert_eq!((done.imported, done.failed.len()), (2, 0));
    assert!(done.staging_deleted);
    assert_eq!(
        rig.count("SELECT COUNT(*) FROM sessions WHERE agent = 'imported' AND adopted = 1"),
        2
    );
    assert_eq!(
        rig.count(&format!(
            "SELECT COUNT(*) FROM imports WHERE source = 'chatgpt' AND external_id = 'chatgpt:{GPT_2}' \
             AND root LIKE 'export:%'"
        )),
        1
    );
    let key = done.session_keys[0].clone();
    let prov = store::with_read(&rig.vault, |c| Ok(provenance(c, &key)))
        .unwrap()
        .unwrap();
    assert_eq!(prov.label, "ChatGPT");
    assert_eq!(prov.source, "chatgpt");
    // Landed in Quick chats; the imported record is read-only.
    assert!(is_imported_in(&rig, &key));

    // Importing the same export again is a no-op.
    let again = rig.inspect(Source::ChatGpt, &zip);
    assert_eq!(again.counts.already_imported, 2);
    assert!(again.conversations.iter().all(|c| !c.selected));
    let result = rig.commit(&again, &CommitRequest::default());
    assert_eq!(result.imported, 0);
    assert_eq!(
        rig.count("SELECT COUNT(*) FROM sessions WHERE agent = 'imported'"),
        2
    );
}

fn is_imported_in(rig: &Rig, key: &str) -> bool {
    store::with_read(&rig.vault, |c| Ok(is_imported(c, key))).unwrap()
}

#[test]
fn an_extracted_export_folder_reads_like_the_zip() {
    let rig = Rig::new();
    let folder = rig.tmp.path().join("chatgpt-export");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("conversations.json"), CHATGPT).unwrap();
    std::fs::write(folder.join("file-sample-2-garden-plan.pdf"), b"%PDF").unwrap();
    let m = rig.inspect(Source::ChatGpt, &folder);
    assert_eq!(m.counts.new, 2);
    assert_eq!(m.export.unwrap().other_files.len(), 1);
}

#[test]
fn sharded_exports_merge_and_a_repeated_id_counts_once() {
    let rig = Rig::new();
    let convs = gpt_value();
    let shard = |items: &[&Value]| serde_json::to_vec(&items).unwrap();
    let (a, b) = (shard(&[&convs[0]]), shard(&[&convs[1], &convs[0]]));
    let path = rig.tmp.path().join("sharded.zip");
    archive::tests::build_zip(
        &path,
        &[
            ("conversations-000.json", &a),
            ("conversations-001.json", &b),
            ("conversations-notes.json", b"[]"),
        ],
    );
    let m = rig.inspect(Source::ChatGpt, &path);
    let e = m.export.unwrap();
    assert_eq!((e.shards, e.duplicates), (2, 1));
    assert_eq!(m.conversations.len(), 2);
    assert_eq!(e.other_files, vec!["conversations-notes.json".to_string()]);
}

#[test]
fn a_shard_that_breaks_keeps_what_came_before() {
    let rig = Rig::new();
    let one = serde_json::to_string(&gpt_value()[0]).unwrap();
    let broken = format!("[{one}, {{\"id\": \"cut-off\", \"mapping\": ");
    let path = rig.tmp.path().join("broken.zip");
    archive::tests::build_zip(&path, &[("conversations.json", broken.as_bytes())]);
    let m = rig.inspect(Source::ChatGpt, &path);
    assert_eq!(m.conversations.len(), 1);
    assert_eq!(
        m.export.unwrap().damaged,
        vec!["conversations.json".to_string()]
    );
}

#[test]
fn an_export_without_conversations_says_it_is_not_supported() {
    let rig = Rig::new();
    let path = rig.tmp.path().join("other.zip");
    archive::tests::build_zip(&path, &[("user.json", b"{}"), ("chat.html", b"<html>")]);
    let m = rig.inspect(Source::ChatGpt, &path);
    assert_eq!(m.unsupported.as_deref(), Some(UNSUPPORTED));
    assert!(m.conversations.is_empty());
    // A conversations file in another shape is unsupported too.
    let odd = rig.tmp.path().join("odd.zip");
    archive::tests::build_zip(&odd, &[("conversations.json", b"{\"not\": \"a list\"}")]);
    let m = rig.inspect(Source::ChatGpt, &odd);
    assert!(m.conversations.is_empty());
    assert_eq!(m.export.unwrap().damaged.len(), 1);
}

#[test]
fn a_hostile_zip_is_refused_and_stages_nothing() {
    let rig = Rig::new();
    let outside = rig.tmp.path().join("evil.json");
    let zip = rig.chatgpt_zip("evil.zip", &[("../evil.json", b"[]")]);
    let id = staging::mint_id();
    let err = inspect(
        &rig.cx(),
        &id,
        Source::ChatGpt,
        &zip,
        &walker::Budget::default(),
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(err.contains("outside its folder"), "{err}");
    assert!(!rig.staging.dir(&id).unwrap().exists());
    assert!(!outside.exists());
    // A bomb, a corrupt file and a missing path are refused the same way.
    let zeros = vec![0u8; 20 * 1024 * 1024];
    let bomb = rig.tmp.path().join("bomb.zip");
    archive::tests::build_zip(&bomb, &[("conversations.json", &zeros)]);
    assert!(rig
        .try_inspect(Source::ChatGpt, &bomb)
        .unwrap_err()
        .contains("expands far more"));
    let junk = rig.tmp.path().join("junk.zip");
    std::fs::write(&junk, b"not a zip").unwrap();
    assert!(rig
        .try_inspect(Source::ChatGpt, &junk)
        .unwrap_err()
        .contains("can't be read"));
    assert!(rig
        .try_inspect(Source::ChatGpt, &rig.tmp.path().join("gone.zip"))
        .is_err());
}

#[test]
fn a_nested_archive_attachment_is_listed_and_the_export_still_reads() {
    let rig = Rig::new();
    let zip = rig.chatgpt_zip("nested.zip", &[("file-upload.zip", b"PK")]);
    let m = rig.inspect(Source::ChatGpt, &zip);
    assert_eq!(m.counts.new, 2);
    assert_eq!(m.export.unwrap().nested_skipped, 1);
}

#[test]
fn a_cancelled_export_pass_stages_nothing() {
    let rig = Rig::new();
    let zip = rig.chatgpt_zip("export.zip", &[]);
    let id = staging::mint_id();
    let flag = AtomicBool::new(true);
    let r = inspect(
        &rig.cx(),
        &id,
        Source::ChatGpt,
        &zip,
        &walker::Budget::default(),
        &flag,
    );
    assert_eq!(r.unwrap_err(), CANCELLED);
    assert!(!rig.staging.dir(&id).unwrap().exists());
}

#[test]
fn chatgpt_destination_is_quick_chats_even_with_a_connected_project() {
    let mut rig = Rig::new();
    rig.projects = vec![("demo".into(), PathBuf::from(PROJECT))];
    let zip = rig.chatgpt_zip("export.zip", &[]);
    let m = rig.inspect(Source::ChatGpt, &zip);
    assert!(m
        .conversations
        .iter()
        .all(|c| c.destination == crate::workspaces::QUICK_CHATS_ID && c.project_folder.is_none()));
}

// ── H5b: Gemini (Google Takeout) ──────────────────────────────────────

const GEMINI_JSON: &str = "Takeout/My Activity/Gemini Apps/MyActivity.json";

/// One sanitized Takeout record.
fn gem(time: &str, prompt: &str, html: &str) -> Value {
    serde_json::json!({
        "header": "Gemini Apps",
        "title": format!("Prompted {prompt}"),
        "titleUrl": "https://gemini.google.com/app/sample",
        "time": time,
        "products": ["Gemini Apps"],
        "safeHtmlItem": [{ "html": html }],
        "activityControls": ["Gemini Apps Activity"],
    })
}

/// The sample activity: three chats' worth in a shuffled file, noise
/// included. Chat A is 10:00, 10:10, 10:40 (each within 30 minutes of the
/// one before, 30 exactly joins); chat B starts at 11:20 (40 minutes on).
fn gemini_records() -> Vec<Value> {
    let mut with_file = gem(
        "2026-10-01T10:10:00.000Z",
        "second one",
        "<p>Second answer</p>",
    );
    with_file["subtitles"] = serde_json::json!([
        { "name": "Attached 1 file." },
        { "name": "- garden-plan.png" }
    ]);
    vec![
        gem(
            "2026-10-01T11:20:00.000Z",
            "a new day's question",
            "<p>Fresh &amp; clean</p>",
        ),
        gem(
            "2026-10-01T10:00:00.000Z",
            "first question",
            "<p>First <b>answer</b></p><script>steal()</script><ul><li>a</li><li>b</li></ul>",
        ),
        with_file,
        gem("2026-10-01T10:40:00.000Z", "third", "<p>Third</p>"),
        // The same record twice (two exports merged in one folder).
        gem("2026-10-01T10:40:00.000Z", "third", "<p>Third</p>"),
        // Not prompts: ignored, not malformed.
        serde_json::json!({"header": "Gemini Apps", "title": "Used Gemini Apps", "time": "2026-10-01T10:05:00Z"}),
        serde_json::json!({"header": "Search", "title": "Searched for cats", "time": "2026-10-01T10:05:00Z"}),
        // Malformed: no time, a bad time, not an object, no title.
        serde_json::json!({"header": "Gemini Apps", "title": "Prompted lost"}),
        serde_json::json!({"header": "Gemini Apps", "title": "Prompted x", "time": "yesterday"}),
        serde_json::json!("just a string"),
        serde_json::json!({"header": "Gemini Apps", "time": "2026-10-01T10:05:00Z"}),
    ]
}

fn takeout_bytes(records: &[Value]) -> Vec<u8> {
    serde_json::to_vec(records).unwrap()
}

impl Rig {
    fn gemini_zip(&self, name: &str, records: &[Value], extra: &[(&str, &[u8])]) -> PathBuf {
        let path = self.tmp.path().join(name);
        let json = takeout_bytes(records);
        let mut entries: Vec<(&str, &[u8])> = vec![
            (GEMINI_JSON, &json),
            ("Takeout/My Activity/Gemini Apps/attached.png", b"png"),
            ("Takeout/archive_browser.html", b"<html>index</html>"),
        ];
        entries.extend_from_slice(extra);
        archive::tests::build_zip(&path, &entries);
        path
    }
}

#[test]
fn gemini_activity_is_grouped_by_the_thirty_minute_rule_and_imports() {
    let rig = Rig::new();
    let zip = rig.gemini_zip("takeout.zip", &gemini_records(), &[]);
    let m = rig.inspect(Source::Gemini, &zip);
    assert_eq!(m.state, "ready");
    assert_eq!(m.source_label, "Gemini");
    assert_eq!(m.conversations.len(), 2);
    assert_eq!(m.counts.new, 2);
    let export = m.export.clone().unwrap();
    assert_eq!(
        (export.shards, export.malformed, export.duplicates),
        (1, 4, 1)
    );
    assert!(export.damaged.is_empty());
    // Attachments and the rest are listed by name, never copied.
    assert_eq!(
        export.other_files,
        vec![
            "Takeout/My Activity/Gemini Apps/attached.png".to_string(),
            "Takeout/archive_browser.html".to_string()
        ]
    );
    // Newest first; every id is a hash, never record text.
    let a = m.conversations.iter().find(|c| c.turn_count == 3).unwrap();
    let b = m.conversations.iter().find(|c| c.turn_count == 1).unwrap();
    for c in [a, b] {
        let hash = c.external_id.strip_prefix("gemini:").unwrap();
        assert_eq!(hash.len(), 32);
        assert!(hash.chars().all(|ch| ch.is_ascii_hexdigit()), "{hash}");
        assert_eq!(c.destination, crate::workspaces::QUICK_CHATS_ID);
        assert_eq!(c.project_folder, None);
        assert_eq!(c.grouping.as_deref(), Some("Grouped by time"));
        assert!(c.selected && c.status == Status::New);
    }
    assert_eq!(a.title, "first question");
    assert_eq!(a.created_at.as_deref(), Some("2026-10-01T10:00:00.000Z"));
    assert_eq!(a.last_at.as_deref(), Some("2026-10-01T10:40:00.000Z"));
    assert_eq!(a.message_count, 6);
    assert_eq!(b.title, "a new day's question");
    for name in std::fs::read_dir(rig.staging.dir(&m.import_id).unwrap().join("conversations"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
    {
        assert!(name.ends_with(".json") && name.len() == 29, "{name}");
    }
    let preview = preview(&rig.staging, &m.import_id, &a.id).unwrap();
    assert_eq!(preview.grouping.as_deref(), Some("Grouped by time"));
    assert_eq!(preview.turns[0].asked, "first question");
    assert_eq!(preview.turns[0].reply, "First answer\n- a\n- b");
    assert_eq!(
        preview.turns[1].asked,
        "second one\n\n[Attached in the original session, not copied: garden-plan.png]"
    );

    let done = rig.commit(&m, &CommitRequest::default());
    assert_eq!((done.imported, done.failed.len()), (2, 0));
    let key = store::with_read(&rig.vault, |c| {
        c.query_row(
            "SELECT session_key FROM imports WHERE external_id = ?1",
            params![a.external_id],
            |r| r.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())
    })
    .unwrap();
    let turns = turns_of(&rig.vault, &key);
    // Turn 1 is the label, then the three prompts in time order.
    assert_eq!(turns.len(), 4);
    assert!(turns[0]
        .3
        .starts_with("Grouped by time: Gemini has no conversation ids"));
    assert_eq!(turns[1].2, "first question");
    assert_eq!(turns[3].2, "third");
    assert!(is_imported_in(&rig, &key));

    // The same export again is a no-op.
    let again = rig.inspect(Source::Gemini, &zip);
    assert_eq!(again.counts.already_imported, 2);
    assert_eq!(rig.commit(&again, &CommitRequest::default()).imported, 0);
    assert_eq!(
        rig.count("SELECT COUNT(*) FROM sessions WHERE agent = 'imported'"),
        2
    );
}

#[test]
fn a_later_takeout_extends_a_chat_in_place_and_starts_new_ones() {
    let rig = Rig::new();
    let first = rig.gemini_zip("one.zip", &gemini_records(), &[]);
    let m = rig.inspect(Source::Gemini, &first);
    rig.commit(&m, &CommitRequest::default());
    let before = rig.count("SELECT COUNT(*) FROM sessions WHERE agent = 'imported'");
    // A newer export: chat A gained a prompt 20 minutes after its last one.
    let mut more = gemini_records();
    more.push(gem("2026-10-01T11:00:00.000Z", "fourth", "<p>Fourth</p>"));
    // 11:00 is 20 minutes after 10:40 and 20 before 11:20: the new prompt
    // joins A, and 11:20 is within 30 minutes of 11:00, so B joins too.
    let second = rig.gemini_zip("two.zip", &more, &[]);
    let m2 = rig.inspect(Source::Gemini, &second);
    // One chat now (11:20 chains on): its first record is still 10:00, so it
    // is chat A's id, an update of the imported chat.
    assert_eq!(m2.conversations.len(), 1);
    let c = &m2.conversations[0];
    assert_eq!(c.status, Status::Updated);
    let done = rig.commit(&m2, &CommitRequest::default());
    assert_eq!((done.updated, done.imported), (1, 0));
    let key = done.session_keys[0].clone();
    let turns = turns_of(&rig.vault, &key);
    assert_eq!(turns.len(), 6);
    assert_eq!(turns[0].3.split(':').next(), Some("Grouped by time"));
    assert_eq!(
        turns.iter().map(|t| t.2.as_str()).collect::<Vec<_>>(),
        vec![
            "",
            "first question",
            "second one\n\n[Attached in the original session, not copied: garden-plan.png]",
            "third",
            "fourth",
            "a new day's question"
        ]
    );
    // Chat B's old copy stays as it was; only A's session was updated.
    assert_eq!(
        rig.count("SELECT COUNT(*) FROM sessions WHERE agent = 'imported'"),
        before
    );
}

#[test]
fn an_extracted_takeout_folder_reads_like_the_zip() {
    let rig = Rig::new();
    let folder = rig.tmp.path().join("Takeout-folder");
    let dir = folder.join("Takeout/My Activity/Gemini Apps");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("MyActivity.json"),
        takeout_bytes(&gemini_records()),
    )
    .unwrap();
    let m = rig.inspect(Source::Gemini, &folder);
    assert_eq!(m.conversations.len(), 2);
    // Picking the Gemini Apps folder itself works too.
    let m = rig.inspect(Source::Gemini, &dir);
    assert_eq!(m.conversations.len(), 2);
}

#[test]
fn an_html_only_takeout_or_another_layout_says_it_is_not_supported() {
    let rig = Rig::new();
    let path = rig.tmp.path().join("html.zip");
    archive::tests::build_zip(
        &path,
        &[(
            "Takeout/My Activity/Gemini Apps/MyActivity.html",
            b"<html><div class=\"content-cell\">Prompted hi<br>Oct 1, 2026, 10:00:00 AM EDT</div></html>",
        )],
    );
    let m = rig.inspect(Source::Gemini, &path);
    assert_eq!(m.unsupported.as_deref(), Some(UNSUPPORTED));
    assert!(m.conversations.is_empty());
    assert_eq!(m.export.unwrap().other_files.len(), 1);
    // Another product's activity is not Gemini's.
    let other = rig.tmp.path().join("search.zip");
    archive::tests::build_zip(
        &other,
        &[("Takeout/My Activity/Search/MyActivity.json", b"[]")],
    );
    assert_eq!(
        rig.inspect(Source::Gemini, &other).unsupported.as_deref(),
        Some(UNSUPPORTED)
    );
    // A file that is not an array is damaged, with nothing staged.
    let odd = rig.tmp.path().join("odd.zip");
    archive::tests::build_zip(&odd, &[(GEMINI_JSON, b"{\"not\": \"a list\"}")]);
    let m = rig.inspect(Source::Gemini, &odd);
    assert!(m.conversations.is_empty());
    assert_eq!(m.export.unwrap().damaged.len(), 1);
}

#[test]
fn a_hostile_takeout_zip_is_refused_and_stages_nothing() {
    let rig = Rig::new();
    let zip = rig.gemini_zip("evil.zip", &gemini_records(), &[("../evil.json", b"[]")]);
    let id = staging::mint_id();
    let r = inspect(
        &rig.cx(),
        &id,
        Source::Gemini,
        &zip,
        &walker::Budget::default(),
        &AtomicBool::new(false),
    );
    assert!(r.is_err());
    assert!(!rig.staging.dir(&id).unwrap().exists());
}

#[test]
fn a_cancelled_gemini_pass_stages_nothing() {
    let rig = Rig::new();
    let zip = rig.gemini_zip("takeout.zip", &gemini_records(), &[]);
    let id = staging::mint_id();
    let r = inspect(
        &rig.cx(),
        &id,
        Source::Gemini,
        &zip,
        &walker::Budget::default(),
        &AtomicBool::new(true),
    );
    assert_eq!(r.unwrap_err(), CANCELLED);
    assert!(!rig.staging.dir(&id).unwrap().exists());
}

#[test]
fn gemini_html_in_a_response_is_text_in_the_stored_chat() {
    let rig = Rig::new();
    let records = vec![gem(
        "2026-10-02T08:00:00Z",
        "<img src=x onerror=alert(1)> what",
        "<p>Safe &lt;b&gt;text&lt;/b&gt;</p><script>evil()</script><iframe src=\"https://x\"></iframe>",
    )];
    let zip = rig.gemini_zip("t.zip", &records, &[]);
    let m = rig.inspect(Source::Gemini, &zip);
    let done = rig.commit(&m, &CommitRequest::default());
    let turns = turns_of(&rig.vault, &done.session_keys[0]);
    // The prompt is the person's own text, kept as typed; the response lost
    // its tags and scripts, and an escaped tag stays literal text.
    assert_eq!(turns[1].2, "<img src=x onerror=alert(1)> what");
    assert_eq!(turns[1].3, "Safe <b>text</b>");
}

// ── claude.ai export (H5b) ────────────────────────────────────────────

const CLAUDEAI: &str = include_str!("../../fixtures/session_import/claudeai_conversations.json");
const CL_1: &str = "11111111-aaaa-4bbb-8ccc-000000000001";
const CL_2: &str = "11111111-aaaa-4bbb-8ccc-000000000002";

impl Rig {
    /// A claude.ai export zip: conversations plus the account and project
    /// files that must never be read, built here.
    fn claudeai_zip(&self, name: &str, extra: &[(&str, &[u8])]) -> PathBuf {
        let path = self.tmp.path().join(name);
        let mut entries: Vec<(&str, &[u8])> = vec![
            ("conversations.json", CLAUDEAI.as_bytes()),
            (
                "projects.json",
                b"[{\"uuid\":\"p1\",\"name\":\"SECRET-PROJECT\"}]",
            ),
            (
                "users.json",
                b"[{\"full_name\":\"SECRET-NAME\",\"email_address\":\"a@b.test\"}]",
            ),
        ];
        entries.extend_from_slice(extra);
        archive::tests::build_zip(&path, &entries);
        path
    }
}

fn claude_value() -> Vec<Value> {
    serde_json::from_str(CLAUDEAI).unwrap()
}

#[test]
fn claudeai_reader_uses_blocks_and_names_files_only() {
    let convs = claude_value();
    let p = claudeai::parse(&convs[0], model::CONVERSATION_MAX_BYTES);
    assert!(p.recognized);
    assert_eq!(p.id.as_deref(), Some(CL_1));
    assert_eq!(p.display_title(), "Sourdough starter");
    assert_eq!(p.created_at.as_deref(), Some("2025-03-01T10:00:00+00:00"));
    assert_eq!(p.last_at.as_deref(), Some("2025-03-01T10:05:00+00:00"));
    assert_eq!(p.turns.len(), 2);
    let first = &p.turns[0];
    assert_eq!(first.asked, "How do I feed a starter?");
    assert_eq!(
        first.files,
        vec!["feeding-notes.txt".to_string(), "starter.png".to_string()]
    );
    // Thinking is skipped; an unknown block is a label; the tool call and
    // its result are an excerpt.
    assert_eq!(
        first.reply,
        "Equal parts flour and water.\n[hologram content, not imported]"
    );
    assert_eq!(first.tools.len(), 1);
    assert_eq!(first.tools[0].name, "web_search");
    assert_eq!(first.tools[0].output.as_deref(), Some("1:1:1 is common"));
    let second = &p.turns[1];
    assert_eq!(second.asked, "How often?");
    assert_eq!(
        second.reply,
        "Daily at room temperature, or weekly in the fridge."
    );
    // The non-object message is counted, not imported.
    assert_eq!(p.malformed, 1);
    let all = format!("{:?}", p.turns);
    assert!(!all.contains("SECRET-ATTACHMENT-BODY") && !all.contains("PRIVATE-REASONING"));

    // No block list, no title, a bad time: plain text, display title from
    // the first prompt, the time falls back to none.
    let q = claudeai::parse(&convs[1], model::CONVERSATION_MAX_BYTES);
    assert!(q.recognized);
    assert_eq!(q.display_title(), "Hello there");
    assert_eq!(q.turns.len(), 1);
    assert_eq!(q.turns[0].reply, "Hi! How can I help?");
    assert_eq!(q.created_at, None);

    // No chat_messages, a bare number or string: unrecognized, no panic.
    for odd in [&convs[2], &convs[4], &convs[5]] {
        assert!(!claudeai::parse(odd, 1_000).recognized);
    }
}

#[test]
fn a_claudeai_zip_inspects_stages_and_imports_without_profile_files() {
    let rig = Rig::new();
    let zip = rig.claudeai_zip("claude-export.zip", &[]);
    let m = rig.inspect(Source::ClaudeAi, &zip);
    assert_eq!(m.state, "ready");
    assert_eq!(m.source_label, "claude.ai");
    assert_eq!(m.counts.new, 2);
    let export = m.export.clone().unwrap();
    assert_eq!(export.shards, 1);
    // The element with no uuid and the two that aren't conversations.
    assert_eq!(export.malformed, 3);
    // projects.json and users.json are listed by name, never read.
    assert_eq!(
        export.other_files,
        vec!["projects.json".to_string(), "users.json".to_string()]
    );
    // Staging files are named by a hash, never the raw uuid.
    let staged = rig.staging.dir(&m.import_id).unwrap().join("conversations");
    let names: Vec<String> = std::fs::read_dir(&staged)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 3);
    assert!(names.iter().all(|n| !n.contains("aaaa-4bbb")), "{names:?}");
    let one = m
        .conversations
        .iter()
        .find(|c| c.external_id == format!("claudeai:{CL_1}"))
        .unwrap();
    assert_eq!(one.destination, crate::workspaces::QUICK_CHATS_ID);
    assert_eq!(one.title, "Sourdough starter");
    assert!(one.selected && one.status == Status::New);

    let done = rig.commit(&m, &CommitRequest::default());
    assert_eq!((done.imported, done.failed.len()), (2, 0));
    assert_eq!(
        rig.count(&format!(
            "SELECT COUNT(*) FROM imports WHERE source = 'claudeai' AND external_id = 'claudeai:{CL_2}' \
             AND root LIKE 'export:%'"
        )),
        1
    );
    let key = done.session_keys[0].clone();
    let prov = store::with_read(&rig.vault, |c| Ok(provenance(c, &key)))
        .unwrap()
        .unwrap();
    assert_eq!(prov.label, "claude.ai");
    assert_eq!(prov.source, "claudeai");
    assert!(is_imported_in(&rig, &key));

    // A second look dedupes on claudeai:<uuid>.
    let again = rig.inspect(Source::ClaudeAi, &zip);
    assert_eq!(again.counts.already_imported, 2);
    assert_eq!(rig.commit(&again, &CommitRequest::default()).imported, 0);
}

#[test]
fn a_claudeai_folder_a_hostile_zip_and_an_unsupported_export() {
    let rig = Rig::new();
    let folder = rig.tmp.path().join("claude-export");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("conversations.json"), CLAUDEAI).unwrap();
    std::fs::write(folder.join("users.json"), b"[]").unwrap();
    let m = rig.inspect(Source::ClaudeAi, &folder);
    assert_eq!(m.counts.new, 2);
    assert_eq!(
        m.export.unwrap().other_files,
        vec!["users.json".to_string()]
    );

    let evil = rig.claudeai_zip("evil.zip", &[("../evil.json", b"[]")]);
    assert!(rig.try_inspect(Source::ClaudeAi, &evil).is_err());

    let none = rig.tmp.path().join("none.zip");
    archive::tests::build_zip(&none, &[("users.json", b"[]")]);
    let m = rig.inspect(Source::ClaudeAi, &none);
    assert_eq!(m.unsupported.as_deref(), Some(UNSUPPORTED));
    assert!(m.conversations.is_empty());
}

#[test]
fn claudeai_shard_names() {
    assert!(claudeai::is_shard("conversations.json"));
    assert!(claudeai::is_shard("data/Conversations.json"));
    assert!(!claudeai::is_shard("a/b/conversations.json"));
    assert!(!claudeai::is_shard("projects.json"));
    assert!(!claudeai::is_shard("conversations-000.json"));
}
