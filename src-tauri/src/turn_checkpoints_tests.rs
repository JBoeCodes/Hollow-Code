//! CP1 tests against real git repositories with a linked worktree.

use super::*;
use std::fs;

struct Rig {
    _tmp: tempfile::TempDir,
    vault: PathBuf,
    main: PathBuf,
    wt: PathBuf,
}

fn sh(dir: &Path, args: &[&str]) -> String {
    run(git(dir)
        .args(["-c", "user.name=T", "-c", "user.email=t@example.com"])
        .args(args))
    .unwrap_or_else(|e| panic!("git {args:?}: {e}"))
}

impl Rig {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        // The store is keyed by the vault folder's name: one per test.
        let vault = tmp.path().join(tmp.path().file_name().unwrap());
        let main = tmp.path().join("repo");
        let wt = tmp.path().join("wt");
        fs::create_dir_all(&vault).unwrap();
        fs::create_dir_all(&main).unwrap();
        sh(&main, &["init", "-q", "-b", "main"]);
        fs::write(main.join("a.txt"), "good\n").unwrap();
        fs::write(main.join(".gitignore"), "ignored/\n").unwrap();
        sh(&main, &["add", "-A"]);
        sh(&main, &["commit", "-q", "-m", "base"]);
        sh(
            &main,
            &["worktree", "add", "-q", "-b", "feat", wt.to_str().unwrap()],
        );
        Rig {
            _tmp: tmp,
            vault,
            main,
            wt,
        }
    }

    fn turn(&self, key: &str, cwd: &Path, run_id: &str) -> i64 {
        crate::session_store::start_turn(
            &self.vault,
            key,
            "claude",
            &cwd.to_string_lossy(),
            run_id,
            "do it",
        )
        .unwrap()
    }
}

/// A chat key of its own per test (the restore lock is process-wide).
fn fresh_key() -> String {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    format!("s-20261118-{:08x}", 0xc0de_0000u32 + n)
}

#[test]
fn a_planted_bad_turn_is_undone_and_the_history_says_so() {
    let rig = Rig::new();
    let key = fresh_key();
    let key = key.as_str();
    let ord = rig.turn(key, &rig.wt, "run-1");
    record(&rig.vault, key, ord, "before", &AtomicBool::new(false)).unwrap();
    // The bad turn: an edit, a new file, a commit, an ignored build output.
    fs::write(rig.wt.join("a.txt"), "broken\n").unwrap();
    fs::write(rig.wt.join("new.txt"), "junk\n").unwrap();
    fs::write(rig.wt.join("c.txt"), "committed\n").unwrap();
    sh(&rig.wt, &["add", "c.txt"]);
    sh(&rig.wt, &["commit", "-q", "-m", "agent"]);
    fs::create_dir_all(rig.wt.join("ignored")).unwrap();
    fs::write(rig.wt.join("ignored/out.bin"), "build\n").unwrap();
    record(&rig.vault, key, ord, "after", &AtomicBool::new(false)).unwrap();
    crate::session_store::finish_turn(&rig.vault, key, ord, "success").unwrap();

    let cps = checkpoints(&rig.vault, key).unwrap();
    assert!(cps.isolated, "{cps:?}");
    assert_eq!(cps.turns.len(), 1);
    assert_eq!(cps.turns[0].files, vec!["a.txt", "c.txt", "new.txt"]);
    assert_eq!(cps.turns[0].run_id.as_deref(), Some("run-1"));
    let head_before = sh(&rig.wt, &["rev-parse", "HEAD^"]);

    let done = restore(&rig.vault, key, ord, &|| false).unwrap();
    assert_eq!(fs::read_to_string(rig.wt.join("a.txt")).unwrap(), "good\n");
    assert!(!rig.wt.join("new.txt").exists());
    assert!(!rig.wt.join("c.txt").exists());
    assert_eq!(sh(&rig.wt, &["rev-parse", "HEAD"]), head_before);
    assert_eq!(done.branch_reset_to.as_deref(), Some(head_before.as_str()));
    // Ignored files are never touched.
    assert!(rig.wt.join("ignored/out.bin").exists());
    // The state before the restore is kept, the agent's commit included.
    let kept = sh(&rig.wt, &["show", &format!("{}:new.txt", done.safety_ref)]);
    assert_eq!(kept, "junk");
    sh(&rig.wt, &["show", &format!("{}^1:c.txt", done.safety_ref)]);
    // The worktree is clean against the restored branch.
    assert_eq!(sh(&rig.wt, &["status", "--porcelain"]), "");
    // The next fresh session's history leaves the undone turn out.
    let next = rig.turn(key, &rig.wt, "run-2");
    let history = crate::session_store::conversation_entries(&rig.vault, key, next).unwrap();
    assert!(history.is_empty(), "{history:?}");
    assert!(
        done.text.contains("Restored the files to before turn 1"),
        "{}",
        done.text
    );
}

#[test]
fn a_restore_in_a_shared_checkout_is_refused_before_any_file_changes() {
    let rig = Rig::new();
    let key = fresh_key();
    let key = key.as_str();
    // The repository's main checkout.
    let ord = rig.turn(key, &rig.main, "run-1");
    record(&rig.vault, key, ord, "before", &AtomicBool::new(false)).unwrap();
    fs::write(rig.main.join("a.txt"), "edited\n").unwrap();
    let err = restore(&rig.vault, key, ord, &|| false).unwrap_err();
    assert!(err.contains("main checkout"), "{err}");
    assert_eq!(
        fs::read_to_string(rig.main.join("a.txt")).unwrap(),
        "edited\n"
    );
    assert_eq!(sh(&rig.main, &["for-each-ref", REF_ROOT]), "");
    assert!(!checkpoints(&rig.vault, key).unwrap().isolated);

    // A worktree another chat also works in.
    let other = "s-20261118-cp1bbbbb";
    let mine = "s-20261118-cp1ccccc";
    let ord = rig.turn(mine, &rig.wt, "run-2");
    record(&rig.vault, mine, ord, "before", &AtomicBool::new(false)).unwrap();
    rig.turn(other, &rig.wt, "run-3");
    fs::write(rig.wt.join("a.txt"), "theirs\n").unwrap();
    let err = restore(&rig.vault, mine, ord, &|| false).unwrap_err();
    assert!(err.contains("Another chat"), "{err}");
    assert_eq!(
        fs::read_to_string(rig.wt.join("a.txt")).unwrap(),
        "theirs\n"
    );
}

#[test]
fn a_running_turn_or_a_missing_checkpoint_is_refused() {
    let rig = Rig::new();
    let key = fresh_key();
    let key = key.as_str();
    let ord = rig.turn(key, &rig.wt, "run-1");
    fs::write(rig.wt.join("a.txt"), "x\n").unwrap();
    let err = restore(&rig.vault, key, ord, &|| true).unwrap_err();
    assert!(err.contains("running"), "{err}");
    let err = restore(&rig.vault, key, ord, &|| false).unwrap_err();
    assert!(err.contains("No checkpoint"), "{err}");
    assert_eq!(fs::read_to_string(rig.wt.join("a.txt")).unwrap(), "x\n");
    // An after checkpoint is never recorded without its before one.
    record(&rig.vault, key, ord, "after", &AtomicBool::new(false)).unwrap();
    assert_eq!(sh(&rig.wt, &["for-each-ref", REF_ROOT]), "");
}

#[test]
fn a_merge_during_the_turn_is_listed_apart() {
    let rig = Rig::new();
    let key = fresh_key();
    let key = key.as_str();
    sh(&rig.main, &["checkout", "-q", "-b", "side"]);
    fs::write(rig.main.join("m.txt"), "from main\n").unwrap();
    sh(&rig.main, &["add", "m.txt"]);
    sh(&rig.main, &["commit", "-q", "-m", "side"]);
    sh(&rig.main, &["checkout", "-q", "main"]);
    let ord = rig.turn(key, &rig.wt, "run-1");
    record(&rig.vault, key, ord, "before", &AtomicBool::new(false)).unwrap();
    fs::write(rig.wt.join("mine.txt"), "agent\n").unwrap();
    sh(
        &rig.wt,
        &["merge", "-q", "--no-ff", "-m", "merge side", "side"],
    );
    record(&rig.vault, key, ord, "after", &AtomicBool::new(false)).unwrap();
    let cps = checkpoints(&rig.vault, key).unwrap();
    assert_eq!(cps.turns[0].files, vec!["mine.txt"]);
    assert_eq!(cps.turns[0].merged_in, vec!["m.txt"]);
}

#[test]
fn staged_work_is_unstaged_and_the_last_turn_counts_even_without_checkpoints() {
    let rig = Rig::new();
    let key = fresh_key();
    let key = key.as_str();
    let ord = rig.turn(key, &rig.wt, "run-1");
    record(&rig.vault, key, ord, "before", &AtomicBool::new(false)).unwrap();
    fs::write(rig.wt.join("a.txt"), "staged edit\n").unwrap();
    fs::write(rig.wt.join("new.txt"), "staged\n").unwrap();
    sh(&rig.wt, &["add", "-A"]);
    crate::session_store::finish_turn(&rig.vault, key, ord, "success").unwrap();
    // A later turn whose checkpoints were never recorded.
    let later = rig.turn(key, &rig.wt, "run-2");
    crate::session_store::finish_turn(&rig.vault, key, later, "success").unwrap();
    let done = restore(&rig.vault, key, ord, &|| false).unwrap();
    assert_eq!(done.through, later);
    assert_eq!(sh(&rig.wt, &["status", "--porcelain"]), "");
    assert_eq!(sh(&rig.wt, &["diff", "--cached", "--name-only"]), "");
    // A second restore keeps its own safety ref beside the first.
    let again = restore(&rig.vault, key, ord, &|| false).unwrap();
    assert_ne!(again.safety_ref, done.safety_ref);
    sh(&rig.wt, &["rev-parse", "--verify", &done.safety_ref]);
    // Undone turns drop out of the list, so Restore is not offered on them.
    assert!(checkpoints(&rig.vault, key).unwrap().turns.is_empty());
}

#[test]
fn files_outside_the_safety_snapshot_or_another_branch_are_refused_untouched() {
    let rig = Rig::new();
    let key = fresh_key();
    let key = key.as_str();
    fs::write(rig.wt.join("notes.txt"), "old\n").unwrap();
    let ord = rig.turn(key, &rig.wt, "run-1");
    record(&rig.vault, key, ord, "before", &AtomicBool::new(false)).unwrap();
    fs::write(rig.wt.join(".gitignore"), "ignored/\nnotes.txt\n").unwrap();
    fs::write(rig.wt.join("notes.txt"), "new and unsaved\n").unwrap();
    let err = restore(&rig.vault, key, ord, &|| false).unwrap_err();
    assert!(err.contains("ignored by git now"), "{err}");
    assert_eq!(
        fs::read_to_string(rig.wt.join("notes.txt")).unwrap(),
        "new and unsaved\n"
    );
    assert_eq!(
        sh(
            &rig.wt,
            &["for-each-ref", &format!("{REF_ROOT}/{key}/restore-*")]
        ),
        ""
    );

    sh(&rig.wt, &["checkout", "-q", "-b", "other"]);
    let err = restore(&rig.vault, key, ord, &|| false).unwrap_err();
    assert!(err.contains("switch back first"), "{err}");
    assert_eq!(sh(&rig.wt, &["symbolic-ref", "HEAD"]), "refs/heads/other");
}

#[test]
fn a_chat_in_a_subfolder_shares_the_worktree_and_a_running_restore_blocks() {
    let rig = Rig::new();
    let key = fresh_key();
    let key = key.as_str();
    let ord = rig.turn(key, &rig.wt, "run-1");
    record(&rig.vault, key, ord, "before", &AtomicBool::new(false)).unwrap();
    fs::create_dir_all(rig.wt.join("src")).unwrap();
    rig.turn("s-20261118-cp1ddddd", &rig.wt.join("src"), "run-2");
    let err = restore(&rig.vault, key, ord, &|| false).unwrap_err();
    assert!(err.contains("Another chat"), "{err}");

    let other = "s-20261118-cp1eeeee";
    let _held = {
        RESTORING.lock().unwrap().insert(other.to_string());
        Restoring(other.to_string())
    };
    assert!(restoring(other));
    let err = restore(&rig.vault, other, 1, &|| false).unwrap_err();
    assert!(err.contains("already running"), "{err}");
}

#[test]
fn an_abandoned_before_checkpoint_is_not_written() {
    let rig = Rig::new();
    let key = fresh_key();
    let key = key.as_str();
    let ord = rig.turn(key, &rig.wt, "run-1");
    assert!(record(&rig.vault, key, ord, "before", &AtomicBool::new(true)).is_err());
    assert_eq!(sh(&rig.wt, &["for-each-ref", REF_ROOT]), "");
}

#[test]
fn a_staged_hunk_is_kept_and_a_file_where_a_folder_goes_back_is_refused() {
    let rig = Rig::new();
    let key = fresh_key();
    let key = key.as_str();
    fs::create_dir_all(rig.wt.join("build")).unwrap();
    fs::write(rig.wt.join("build/out.txt"), "tracked\n").unwrap();
    sh(&rig.wt, &["add", "build/out.txt"]);
    sh(&rig.wt, &["commit", "-q", "-m", "build"]);
    let ord = rig.turn(key, &rig.wt, "run-1");
    record(&rig.vault, key, ord, "before", &AtomicBool::new(false)).unwrap();
    fs::write(rig.wt.join("a.txt"), "staged version\n").unwrap();
    sh(&rig.wt, &["add", "a.txt"]);
    fs::write(rig.wt.join("a.txt"), "disk version\n").unwrap();
    let done = restore(&rig.vault, key, ord, &|| false).unwrap();
    let staged = sh(
        &rig.wt,
        &[
            "cat-file",
            "-p",
            &format!("{}-index:a.txt", done.safety_ref),
        ],
    );
    assert_eq!(staged, "staged version");

    let ord = rig.turn(key, &rig.wt, "run-2");
    record(&rig.vault, key, ord, "before", &AtomicBool::new(false)).unwrap();
    fs::remove_dir_all(rig.wt.join("build")).unwrap();
    fs::write(rig.wt.join(".gitignore"), "ignored/\nbuild\n").unwrap();
    fs::write(rig.wt.join("build"), "an ignored file\n").unwrap();
    let err = restore(&rig.vault, key, ord, &|| false).unwrap_err();
    assert!(err.contains("not a folder now"), "{err}");
    assert_eq!(
        fs::read_to_string(rig.wt.join("build")).unwrap(),
        "an ignored file\n"
    );
}

/// CH1: a branch of a chat in its isolated worktree keeps the chosen turn's
/// after checkpoint as `refs/hollow/branches/<branch>` and lists the paths
/// the original changed since; no file is touched.
#[test]
fn a_branch_keeps_the_branch_points_checkpoint_and_lists_later_changes() {
    let rig = Rig::new();
    let key = fresh_key();
    let key = key.as_str();
    let ord = rig.turn(key, &rig.wt, "run-1");
    record(&rig.vault, key, ord, "before", &AtomicBool::new(false)).unwrap();
    fs::write(rig.wt.join("a.txt"), "turn one\n").unwrap();
    record(&rig.vault, key, ord, "after", &AtomicBool::new(false)).unwrap();
    crate::session_store::finish_turn(&rig.vault, key, ord, "success").unwrap();
    let at_one = rev(&rig.wt, &ref_name(key, ord, "after")).unwrap();
    // A later turn of the original changes a file and adds one.
    let two = rig.turn(key, &rig.wt, "run-2");
    fs::write(rig.wt.join("a.txt"), "turn two\n").unwrap();
    fs::write(rig.wt.join("b.txt"), "new\n").unwrap();
    crate::session_store::finish_turn(&rig.vault, key, two, "success").unwrap();

    let b = crate::chat_branch::branch(&rig.vault, key, "run-1", fixed_time()).unwrap();
    assert_eq!(b.through, ord);
    assert_eq!(b.checkpoint.as_deref(), Some(at_one.as_str()));
    let name = format!("{BRANCH_REF_ROOT}/{}", b.key);
    assert_eq!(b.ref_name.as_deref(), Some(name.as_str()));
    assert_eq!(rev(&rig.wt, &name).as_deref(), Some(at_one.as_str()));
    assert_eq!(b.changed_since, vec!["a.txt", "b.txt"]);
    // Nothing in the worktree moved.
    assert_eq!(
        fs::read_to_string(rig.wt.join("a.txt")).unwrap(),
        "turn two\n"
    );
    assert!(rig.wt.join("b.txt").exists());
    // The ref is create-only.
    assert!(keep_for_branch(&rig.wt, &b.key, &at_one).is_err());
    // Two chats in one worktree: neither is isolated from now on.
    assert!(!checkpoints(&rig.vault, key).unwrap().isolated);
    let info = crate::chat_branch::info(&rig.vault, &b.key)
        .unwrap()
        .unwrap();
    assert_eq!(info.checkpoint.as_deref(), Some(at_one.as_str()));
}

/// A fixed time: tests never read the clock.
fn fixed_time() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339("2026-11-18T09:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc)
}
