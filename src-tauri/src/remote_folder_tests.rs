//! Remote sessions, Phase 3: a workspace folder's snapshot and the three-way
//! apply of what came back, without a runner (the runner's result is made
//! here as a second snapshot). The round trip through a stand-in runner is
//! in `remote_session_tests.rs`.

use super::*;

/// C5 review: a chat's attachments folder (its `.gitignore` is `/*`) is no
/// cache, so it travels with a turn; a tool's cache (`*` alone) still stays
/// here.
#[test]
fn a_chats_attachments_folder_travels_and_a_tools_cache_stays() {
    use base64::{engine::general_purpose::STANDARD, Engine};

    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path();
    let vault = tempfile::tempdir().unwrap();
    let body = STANDARD.encode(b"name,email\n");
    crate::chat::save_attachment(
        &body,
        "text/csv",
        Some("customers.csv"),
        folder,
        vault.path(),
    )
    .unwrap();
    let cache = folder.join(".pytest_cache");
    std::fs::create_dir(&cache).unwrap();
    std::fs::write(cache.join(".gitignore"), "*\n").unwrap();
    assert!(!self_ignoring(&folder.join(crate::chat::ATTACHMENTS_DIR)));
    assert!(self_ignoring(&cache));
    assert_eq!(left_here(folder).caches, [".pytest_cache/"]);
}

struct Fixture {
    _dir: tempfile::TempDir,
    git_dir: PathBuf,
    here: PathBuf,
    there: PathBuf,
}

impl Fixture {
    /// A folder here and its copy "on the runner", both holding `files`.
    fn new(files: &[(&str, &str)]) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path().join("private.git");
        std::fs::create_dir(&git_dir).unwrap();
        git_text(&git_dir, &["init", "-q", "--bare", "."], &[]).unwrap();
        let here = dir.path().join("here");
        let there = dir.path().join("there");
        for folder in [&here, &there] {
            for (name, text) in files {
                let path = folder.join(name);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, text).unwrap();
            }
        }
        Fixture {
            _dir: dir,
            git_dir,
            here,
            there,
        }
    }
    fn base(&self) -> String {
        snapshot(&self.git_dir, &self.here, "aaaa").unwrap().0
    }
    fn result(&self) -> String {
        snapshot(&self.git_dir, &self.there, "bbbb").unwrap().0
    }
    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.here.join(name)).unwrap()
    }
}

/// A file changed only there comes here; one changed only here stays; a new
/// file arrives; a removed one goes; one changed the same way both sides is
/// left as it is.
#[test]
fn one_sided_changes_come_back_and_this_macs_stay() {
    let f = Fixture::new(&[
        ("PLAN.md", "# Plan\n- one\n"),
        ("notes/a.md", "a\n"),
        ("gone.md", "bye\n"),
        ("same.md", "x\n"),
    ]);
    let base = f.base();
    std::fs::write(f.there.join("PLAN.md"), "# Plan\n- one\n- two\n").unwrap();
    std::fs::write(f.there.join("outputs-new.md"), "made there\n").unwrap();
    std::fs::remove_file(f.there.join("gone.md")).unwrap();
    std::fs::write(f.there.join("same.md"), "y\n").unwrap();
    let result = f.result();
    std::fs::write(f.here.join("notes/a.md"), "a, edited here\n").unwrap();
    std::fs::write(f.here.join("same.md"), "y\n").unwrap();
    let back = apply(&f.git_dir, &f.here, &base, &result).unwrap();
    assert_eq!(f.read("PLAN.md"), "# Plan\n- one\n- two\n");
    assert_eq!(f.read("outputs-new.md"), "made there\n");
    assert!(!f.here.join("gone.md").exists());
    assert_eq!(f.read("notes/a.md"), "a, edited here\n");
    assert_eq!(f.read("same.md"), "y\n");
    let mut changed = back.changed.clone();
    changed.sort();
    assert_eq!(changed, vec!["PLAN.md", "gone.md", "outputs-new.md"]);
    assert!(back.copies.is_empty() && back.merged.is_empty(), "{back:?}");
}

/// Both sides changed a text file: edits that don't overlap are merged;
/// ones that do keep this Mac's version and put the runner's beside it as a
/// conflicted copy. A binary file changed on both sides is kept twice too.
#[test]
fn both_sides_changed_merges_or_keeps_both() {
    let f = Fixture::new(&[
        (
            "WORKSPACE.md",
            "# Brief\n\nintro\n\n## Decisions\n- a\n\n## History\n- h\n",
        ),
        ("PLAN.md", "line one\n"),
        ("image.bin", "\u{0}\u{1}\u{2}"),
    ]);
    let base = f.base();
    std::fs::write(
        f.there.join("WORKSPACE.md"),
        "# Brief\n\nintro\n\n## Decisions\n- a\n- b from the runner\n\n## History\n- h\n",
    )
    .unwrap();
    std::fs::write(f.there.join("PLAN.md"), "line one, the runner's\n").unwrap();
    std::fs::write(f.there.join("image.bin"), [0u8, 9, 9, 0xff]).unwrap();
    let result = f.result();
    std::fs::write(
        f.here.join("WORKSPACE.md"),
        "# Brief\n\nintro\n\n## Decisions\n- a\n\n## History\n- h\n- another chat's line\n",
    )
    .unwrap();
    std::fs::write(f.here.join("PLAN.md"), "line one, this Mac's\n").unwrap();
    std::fs::write(f.here.join("image.bin"), [0u8, 7, 0xfe]).unwrap();
    let back = apply(&f.git_dir, &f.here, &base, &result).unwrap();
    assert_eq!(
        f.read("WORKSPACE.md"),
        "# Brief\n\nintro\n\n## Decisions\n- a\n- b from the runner\n\n## History\n- h\n- another chat's line\n"
    );
    assert_eq!(back.merged, vec!["WORKSPACE.md"]);
    assert_eq!(f.read("PLAN.md"), "line one, this Mac's\n");
    assert_eq!(back.copies.len(), 2, "{back:?}");
    let plan_copy = back
        .copies
        .iter()
        .find(|c| c.starts_with("PLAN (conflicted copy"))
        .expect("a conflicted copy of PLAN.md");
    assert_eq!(
        std::fs::read_to_string(f.here.join(plan_copy)).unwrap(),
        "line one, the runner's\n"
    );
    assert_eq!(
        std::fs::read(f.here.join("image.bin")).unwrap(),
        [0u8, 7, 0xfe]
    );
    // Waiting on you finds the copies as it finds any conflict copy.
    let names: Vec<String> = std::fs::read_dir(&f.here)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names
            .iter()
            .any(|n| n.starts_with("image (conflicted copy")),
        "{names:?}"
    );
}

/// Changed here and removed there: kept here. Removed here and changed
/// there: the runner's version comes back as a conflicted copy, so neither
/// side's decision is lost silently.
#[test]
fn a_removal_against_an_edit_keeps_the_edit() {
    let f = Fixture::new(&[("a.md", "a\n"), ("b.md", "b\n")]);
    let base = f.base();
    std::fs::remove_file(f.there.join("a.md")).unwrap();
    std::fs::write(f.there.join("b.md"), "b, the runner's\n").unwrap();
    let result = f.result();
    std::fs::write(f.here.join("a.md"), "a, edited here\n").unwrap();
    std::fs::remove_file(f.here.join("b.md")).unwrap();
    let back = apply(&f.git_dir, &f.here, &base, &result).unwrap();
    assert_eq!(f.read("a.md"), "a, edited here\n");
    assert_eq!(back.kept_here, vec!["a.md"]);
    assert!(!f.here.join("b.md").exists());
    assert_eq!(back.copies.len(), 1, "{back:?}");
    assert_eq!(
        std::fs::read_to_string(f.here.join(&back.copies[0])).unwrap(),
        "b, the runner's\n"
    );
}

/// Nothing from the runner is written through a symlink here, and a symlink
/// it made isn't brought back; junk and files over the size cap aren't sent.
#[test]
fn symlinks_and_junk_never_cross() {
    let f = Fixture::new(&[("docs/a.md", "a\n")]);
    let base = f.base();
    std::fs::write(f.there.join("docs/a.md"), "a, the runner's\n").unwrap();
    std::os::unix::fs::symlink("/etc/hosts", f.there.join("link")).unwrap();
    std::fs::write(f.there.join(".DS_Store"), "junk").unwrap();
    let result = f.result();
    // Here, `docs` became a symlink to a folder outside.
    let outside = f.here.parent().unwrap().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("a.md"), "a\n").unwrap();
    std::fs::remove_dir_all(f.here.join("docs")).unwrap();
    std::os::unix::fs::symlink(&outside, f.here.join("docs")).unwrap();
    let back = apply(&f.git_dir, &f.here, &base, &result).unwrap();
    assert_eq!(
        std::fs::read_to_string(outside.join("a.md")).unwrap(),
        "a\n"
    );
    assert!(f.here.join("link").symlink_metadata().is_err());
    assert!(!f.here.join(".DS_Store").exists());
    let mut skipped = back.skipped.clone();
    skipped.sort();
    assert_eq!(skipped, vec!["docs/a.md", "link"]);
}

/// Paths that would leave the folder or reach a `.git` are refused.
#[test]
fn unsafe_paths_are_refused() {
    for bad in [
        "",
        "/etc/x",
        "../x",
        "a/../../x",
        ".git/config",
        "a/.GIT/x",
        "./x",
    ] {
        assert!(safe_rel(bad).is_none(), "{bad}");
    }
    assert_eq!(
        safe_rel("outputs/a b.md"),
        Some(PathBuf::from("outputs/a b.md"))
    );
}

/// A file over the cap stays here: not in the snapshot, so the runner
/// never sees it and its absence there removes nothing here.
#[test]
fn big_files_stay_here() {
    let f = Fixture::new(&[("small.md", "s\n")]);
    let big = f.here.join("outputs/render.mov");
    std::fs::create_dir_all(big.parent().unwrap()).unwrap();
    let file = std::fs::File::create(&big).unwrap();
    file.set_len(MAX_FILE_BYTES + 1).unwrap();
    let base = f.base();
    let listed = git_text(
        &f.git_dir,
        &["ls-tree", "-r", "--name-only", &base],
        &[("GIT_DIR", f.git_dir.as_os_str())],
    )
    .unwrap();
    assert_eq!(listed, "small.md");
    let result = f.result();
    let back = apply(&f.git_dir, &f.here, &base, &result).unwrap();
    assert!(back.is_empty(), "{back:?}");
    assert!(big.exists());
}

/// Runs on for a vault workspace lives in app data, per vault; the Main
/// agent can't move.
#[test]
fn a_workspaces_runs_on_is_kept_outside_the_vault() {
    let vault = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(vault.path().join("workspaces/w1")).unwrap();
    std::fs::create_dir_all(vault.path().join("workspaces/main")).unwrap();
    assert_eq!(runs_on_for(vault.path(), "w1"), None);
    assert_eq!(
        set_runs_on(vault.path(), "w1", Some("runner")).unwrap(),
        Some("runner".into())
    );
    assert_eq!(runs_on_for(vault.path(), "w1").as_deref(), Some("runner"));
    assert_eq!(
        set_runs_on(vault.path(), "w1", Some("default")).unwrap(),
        None
    );
    assert_eq!(runs_on_for(vault.path(), "w1"), None);
    assert!(set_runs_on(vault.path(), "main", Some("mac-mini")).is_err());
    std::fs::create_dir_all(vault.path().join("workspaces/quick-chats")).unwrap();
    assert!(set_runs_on(vault.path(), "quick-chats", Some("mac-mini")).is_err());
    assert!(set_runs_on(vault.path(), "nope", Some("mac-mini")).is_err());
    assert!(set_runs_on(vault.path(), "w1", Some("moon")).is_err());
    assert!(std::fs::read_dir(vault.path().join("workspaces/w1"))
        .unwrap()
        .next()
        .is_none());
}

/// M6 (PER-20): a per-workspace Runs on file saved before the runner's
/// neutral name says `mac-mini`. It reads as `runner`, reading leaves the
/// file as it is, and only a choice the person makes writes `runner`.
#[test]
fn an_old_mac_mini_runs_on_file_reads_as_the_runner_and_is_rewritten_only_on_a_save() {
    let vault = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(vault.path().join("workspaces/w-old")).unwrap();
    let stem = crate::session_store::vault_file_stem(vault.path());
    // The old shape, `{vault: {id: "mac-mini"}}`, as an earlier Hollow wrote it.
    {
        let _held = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut store = load().unwrap();
        store
            .entry(stem.clone())
            .or_default()
            .insert("w-old".into(), "mac-mini".into());
        let text = serde_json::to_string_pretty(&store).unwrap();
        crate::vault::atomic_write_private(&store_path().unwrap(), text.as_bytes()).unwrap();
    }
    let stored = || {
        load()
            .unwrap()
            .get(&stem)
            .and_then(|m| m.get("w-old").cloned())
    };
    assert_eq!(stored().as_deref(), Some("mac-mini"));
    for _ in 0..2 {
        assert_eq!(
            runs_on_for(vault.path(), "w-old").as_deref(),
            Some("runner")
        );
    }
    assert_eq!(
        stored().as_deref(),
        Some("mac-mini"),
        "reading never rewrites it"
    );
    assert_eq!(
        stored_runs_on(vault.path(), "w-old").as_deref(),
        Some("mac-mini")
    );
    // The phone's failed-send undo puts the old value back as it was.
    set_runs_on(vault.path(), "w-old", Some("this-mac")).unwrap();
    assert_eq!(
        restore_runs_on(vault.path(), "w-old", Some("mac-mini")).unwrap(),
        Some("runner".into())
    );
    assert_eq!(stored().as_deref(), Some("mac-mini"));
    // Either name saves as `runner`.
    assert_eq!(
        set_runs_on(vault.path(), "w-old", Some(" mac-mini ")).unwrap(),
        Some("runner".into())
    );
    assert_eq!(stored().as_deref(), Some("runner"));
    assert_eq!(
        set_runs_on(vault.path(), "w-old", Some("this-mac")).unwrap(),
        Some("this-mac".into())
    );
    assert_eq!(stored().as_deref(), Some("this-mac"));
    assert_eq!(set_runs_on(vault.path(), "w-old", None).unwrap(), None);
    assert_eq!(stored(), None);
}

/// Only a vault workspace's own folder moves: never the Main agent's, Quick
/// chats', a subfolder or the vault itself; and a workspace with no choice
/// of its own stays here (the Settings default isn't consulted for it: the
/// Settings file is shared by this test process, so it isn't set here).
#[test]
fn only_a_workspaces_own_folder_moves_and_only_on_its_own_choice() {
    let vault = tempfile::tempdir().unwrap();
    let brief = |id: &str, kind: &str| {
        let dir = vault.path().join("workspaces").join(id);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(
            dir.join("WORKSPACE.md"),
            format!("---\nhollow_workspace: 1\n{kind}title: {id}\nstatus: active\n---\n\n# {id}\n"),
        )
        .unwrap();
        dir
    };
    let w1 = brief("w1", "");
    let main = brief("main", "kind: main\n");
    let quick = brief("quick-chats", "kind: quick\n");
    assert!(matches!(folder_for(vault.path(), &w1), Some((_, true))));
    assert_eq!(folder_for(vault.path(), &main), None);
    assert_eq!(folder_for(vault.path(), &quick), None);
    assert_eq!(folder_for(vault.path(), &w1.join("sub")), None);
    assert_eq!(folder_for(vault.path(), vault.path()), None);
    use crate::agent::AgentKind;
    assert_eq!(
        crate::remote_session::runs_on(
            vault.path(),
            &w1,
            AgentKind::Claude,
            crate::remote_session::PlaceReads::LIVE
        ),
        None,
        "no choice of its own: stays here"
    );
    assert_eq!(
        crate::remote_session::runs_on(
            vault.path(),
            &quick,
            AgentKind::Claude,
            crate::remote_session::PlaceReads::LIVE
        ),
        None
    );
    set_runs_on(vault.path(), "w1", Some("mac-mini")).unwrap();
    assert!(crate::remote_session::runs_on(
        vault.path(),
        &w1,
        AgentKind::Claude,
        crate::remote_session::PlaceReads::LIVE
    )
    .is_some());
    assert_eq!(
        crate::remote_session::runs_on(
            vault.path(),
            &w1,
            AgentKind::Antigravity,
            crate::remote_session::PlaceReads::LIVE
        ),
        None
    );
    set_runs_on(vault.path(), "w1", None).unwrap();
}

/// A file added on both sides, or changed on both sides when it isn't
/// Markdown or plain text, is kept twice: a line merge could make it a file
/// that no longer parses.
#[test]
fn added_on_both_sides_or_not_text_is_kept_twice() {
    let f = Fixture::new(&[("data.json", "{\"a\": 1}\n"), ("notes.md", "a\n\nb\n")]);
    let base = f.base();
    std::fs::write(f.there.join("new.json"), "{\"there\": 1}\n").unwrap();
    std::fs::write(f.there.join("data.json"), "{\"a\": 1, \"there\": 2}\n").unwrap();
    std::fs::write(f.there.join("notes.md"), "a\n\nb\nc there\n").unwrap();
    let result = f.result();
    std::fs::write(f.here.join("new.json"), "{\"here\": 1}\n").unwrap();
    std::fs::write(f.here.join("data.json"), "{\"here\": 0, \"a\": 1}\n").unwrap();
    std::fs::write(f.here.join("notes.md"), "a here\n\nb\n").unwrap();
    let back = apply(&f.git_dir, &f.here, &base, &result).unwrap();
    assert_eq!(f.read("new.json"), "{\"here\": 1}\n");
    assert_eq!(f.read("data.json"), "{\"here\": 0, \"a\": 1}\n");
    assert_eq!(back.copies.len(), 2, "{back:?}");
    assert_eq!(back.merged, vec!["notes.md"]);
    assert_eq!(f.read("notes.md"), "a here\n\nb\nc there\n");
}

/// One file that can't be written here doesn't stop the rest, and is named.
#[test]
fn one_file_that_cant_be_written_doesnt_stop_the_rest() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new(&[("locked/a.md", "a\n"), ("b.md", "b\n")]);
    let base = f.base();
    std::fs::write(f.there.join("locked/a.md"), "a, the runner's\n").unwrap();
    std::fs::write(f.there.join("b.md"), "b, the runner's\n").unwrap();
    let result = f.result();
    let locked = f.here.join("locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    let back = apply(&f.git_dir, &f.here, &base, &result);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    let back = back.unwrap();
    assert_eq!(back.changed, vec!["b.md"]);
    assert_eq!(back.failed.len(), 1, "{back:?}");
    assert!(back.failed[0].starts_with("locked/a.md"), "{back:?}");
    assert_eq!(f.read("b.md"), "b, the runner's\n");
}

/// Hollow's own session records never come back from the runner, and a
/// repository inside the folder stays here.
#[test]
fn session_records_and_inner_repositories_stay_here() {
    let f = Fixture::new(&[("sessions/s-1.md", "record\n"), ("PLAN.md", "p\n")]);
    std::fs::create_dir_all(f.here.join("outputs/app/.git")).unwrap();
    std::fs::write(f.here.join("outputs/app/main.rs"), "fn main() {}\n").unwrap();
    let base = f.base();
    let listed = git_text(
        &f.git_dir,
        &["ls-tree", "-r", "--name-only", &base],
        &[("GIT_DIR", f.git_dir.as_os_str())],
    )
    .unwrap();
    assert_eq!(listed, "PLAN.md");
}

/// This Mac's volume doesn't tell `Sessions/` from `sessions/` (review of
/// #1083): a runner's `Sessions/s-9.md` would land in `sessions/`.
#[test]
fn what_stays_here_matches_without_case() {
    let dir = tempfile::tempdir().unwrap();
    for rel in [
        "Sessions/s-9.md",
        "SESSIONS/x.md",
        "a/NODE_MODULES/x.js",
        "a/.Trash/x",
        "a/.ds_store",
        "a/x.HOLLOW-TMP-1",
        "\u{17F}essions/x.md",
        "a/node_module\u{17F}/x.js",
        "a/.ds_\u{17F}tore",
        "se\u{DF}ions/x.md",
        "se\u{1E9E}ions/x.md",
        "a/.ds_\u{FB06}ore",
        "a/.ds_\u{FB05}ore",
        "a/x.hollow-tmp-\u{212A}",
    ] {
        assert!(stays_here(dir.path(), Path::new(rel)), "{rel}");
    }
    assert!(!stays_here(dir.path(), Path::new("notes/sessions.md")));
    assert!(!stays_here(dir.path(), Path::new("outputs/Sessions/x.md")));
}

/// What the snapshot leaves out isn't taken from the runner either: a
/// result that adds a session record, junk, or a file inside a repository
/// that stays here is skipped.
#[test]
fn what_stays_here_is_never_taken_from_the_runner() {
    let f = Fixture::new(&[("PLAN.md", "p\n")]);
    std::fs::create_dir_all(f.here.join("outputs/app/.git")).unwrap();
    let base = f.base();
    std::fs::create_dir_all(f.there.join("sessions")).unwrap();
    std::fs::write(f.there.join("sessions/s-9.md"), "planted\n").unwrap();
    std::fs::create_dir_all(f.there.join("outputs/app")).unwrap();
    std::fs::write(f.there.join("outputs/app/evil.rs"), "x\n").unwrap();
    // Made as the runner would if it ignored the excludes.
    let index = f.git_dir.join("raw-index");
    let env = [
        ("GIT_DIR", f.git_dir.as_os_str()),
        ("GIT_WORK_TREE", f.there.as_os_str()),
        ("GIT_INDEX_FILE", index.as_os_str()),
    ];
    git_text(&f.there, &["add", "-A", "-f", "."], &env).unwrap();
    let tree = git_text(&f.there, &["write-tree"], &env).unwrap();
    let mut cenv: Vec<(&str, &OsStr)> = env.to_vec();
    for (k, v) in IDENTITY {
        cenv.push((k, OsStr::new(v)));
    }
    let result = git_text(
        &f.there,
        &["commit-tree", &tree, "-p", &base, "-m", "r"],
        &cenv,
    )
    .unwrap();
    let back = apply(&f.git_dir, &f.here, &base, &result).unwrap();
    assert!(!f.here.join("sessions/s-9.md").exists());
    assert!(!f.here.join("outputs/app/evil.rs").exists());
    let mut skipped = back.skipped.clone();
    skipped.sort();
    assert_eq!(skipped, vec!["outputs/app/evil.rs", "sessions/s-9.md"]);
}

/// Phase 3 leftover (2026-11-02): every snapshot sent is remembered, the
/// newest 16 per folder, and only those can be a kept result's base;
/// another folder's snapshot or a made-up commit can't. The last snapshot
/// sent before they were remembered (`snap`) counts as sent, so a result
/// kept on it then still comes back.
#[test]
fn snapshots_sent_are_remembered() {
    let f = Fixture::new(&[("PLAN.md", "plan\n")]);
    let dir_env = [("GIT_DIR", f.git_dir.as_os_str())];
    // A snapshot from before: only `snap` was kept.
    let older = snapshot(&f.git_dir, &f.here, "eeee").unwrap().0;
    git_text(
        &f.git_dir,
        &["update-ref", "-d", &sent_ref("eeee", &older)],
        &dir_env,
    )
    .unwrap();
    assert!(!was_sent(&f.git_dir, "eeee", &older));
    std::fs::write(f.here.join("PLAN.md"), "plan again\n").unwrap();
    let newer = snapshot(&f.git_dir, &f.here, "eeee").unwrap().0;
    assert!(
        was_sent(&f.git_dir, "eeee", &older),
        "the one before counts"
    );
    assert!(was_sent(&f.git_dir, "eeee", &newer));
    std::fs::write(f.here.join("PLAN.md"), "plan\n").unwrap();
    let first = snapshot(&f.git_dir, &f.here, "cccc").unwrap().0;
    assert!(was_sent(&f.git_dir, "cccc", &first));
    assert!(!was_sent(&f.git_dir, "dddd", &first), "another folder's");
    assert!(!was_sent(&f.git_dir, "cccc", &"0".repeat(40)));
    assert!(!was_sent(&f.git_dir, "cccc", "HEAD"));
    let mut sent = vec![first.clone()];
    for i in 0..SENT_KEPT + 2 {
        std::fs::write(f.here.join("PLAN.md"), format!("plan {i}\n")).unwrap();
        sent.push(snapshot(&f.git_dir, &f.here, "cccc").unwrap().0);
    }
    let remembered = sent
        .iter()
        .filter(|c| was_sent(&f.git_dir, "cccc", c))
        .count();
    // Which go is by commit time, in whole seconds (these are all made in
    // one), so only how many is checked here.
    assert_eq!(remembered, SENT_KEPT);
}

/// Two snapshots of an unchanged folder are two commits even inside one clock
/// second (overnight 11-07, lane D3). A commit with no parent and the same
/// message was one commit for one tree and one second, so a kept result on the
/// first turn's snapshot looked like one made on the second turn's own, which
/// is never applied three ways (the flake in
/// `a_workspace_folder_turn_runs_there_and_its_changes_come_back`).
#[test]
fn two_snapshots_of_an_unchanged_folder_are_two_commits() {
    let f = Fixture::new(&[("PLAN.md", "plan\n")]);
    let first = snapshot(&f.git_dir, &f.here, "ffff").unwrap().0;
    let second = snapshot(&f.git_dir, &f.here, "ffff").unwrap().0;
    assert_ne!(first, second, "one id for two snapshots");
    let dir_env = [("GIT_DIR", f.git_dir.as_os_str())];
    let tree_of = |commit: &str| {
        git_text(
            &f.git_dir,
            &["rev-parse", &format!("{commit}^{{tree}}")],
            &dir_env,
        )
        .unwrap()
    };
    assert_eq!(tree_of(&first), tree_of(&second), "the same files");
    assert!(
        was_sent(&f.git_dir, "ffff", &first),
        "the first is remembered"
    );
    assert!(was_sent(&f.git_dir, "ffff", &second));
}

/// The files in a snapshot commit, NUL-safe.
fn tree_files(git_dir: &Path, commit: &str) -> Vec<String> {
    let raw = git_raw(
        git_dir,
        &["ls-tree", "-r", "-z", "--name-only", commit],
        &[("GIT_DIR", git_dir.as_os_str())],
    )
    .unwrap();
    raw.split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect()
}

/// Overnight R3: the folder's own `.gitignore` files don't decide what goes
/// (the vault isn't a git repository; they are the person's, for some
/// other tool). What one names goes like any file; what stays here is
/// Hollow's call alone, whatever a `.gitignore` says: secret-looking names,
/// junk, a repository inside the folder, a file over the cap. Each kind is
/// listed for the start line.
#[test]
fn gitignored_files_go_and_secret_looking_ones_stay() {
    let f = Fixture::new(&[
        ("PLAN.md", "plan\n"),
        (
            ".gitignore",
            "build/\nnotes-private.md\n*.log\n.env\n*.pem\nx*\n",
        ),
        ("build/out.txt", "built\n"),
        ("build/sub/deep.txt", "deep\n"),
        ("build/node_modules/j.js", "junk\n"),
        ("build/.DS_Store", "junk\n"),
        ("notes-private.md", "mine\n"),
        ("run.log", "log\n"),
        (".env", "TOKEN=here\n"),
        ("secrets.pem", "pem\n"),
        ("keys/id_ed25519", "key\n"),
        // Added literally: `x*` must not take `x.hollow-tmp-1` with it.
        ("x*", "a file named x*\n"),
        ("x.hollow-tmp-1", "junk\n"),
        ("x.pem", "pem\n"),
        ("a\nsecret.txt", "a newline in its name\n"),
        ("build/app/a.rs", "in a repository\n"),
        ("sessions/s-1.md", "Hollow's own\n"),
    ]);
    std::fs::create_dir_all(f.here.join("build/app/.git")).unwrap();
    let big = std::fs::File::create(f.here.join("build/huge.bin")).unwrap();
    big.set_len(MAX_FILE_BYTES + 1).unwrap();
    let (base, left) = snapshot(&f.git_dir, &f.here, "aaaa").unwrap();
    assert_eq!(
        tree_files(&f.git_dir, &base),
        vec![
            ".gitignore",
            "PLAN.md",
            "build/out.txt",
            "build/sub/deep.txt",
            "notes-private.md",
            "run.log",
            "x*",
        ]
    );
    assert_eq!(
        left.secret,
        vec![
            ".env",
            "a\nsecret.txt",
            "keys/id_ed25519",
            "secrets.pem",
            "x.pem"
        ]
    );
    assert_eq!(left.big, vec!["build/huge.bin"]);
    assert_eq!(left.repos, vec!["build/app/"]);
    // Next turn: an ignored file sent before is changed, another removed,
    // one is new; the index follows all three.
    std::fs::write(f.here.join("build/out.txt"), "rebuilt\n").unwrap();
    std::fs::remove_file(f.here.join("run.log")).unwrap();
    std::fs::write(f.here.join("build/sub/new.txt"), "new\n").unwrap();
    let (next, _) = snapshot(&f.git_dir, &f.here, "aaaa").unwrap();
    let files = tree_files(&f.git_dir, &next);
    assert!(
        files.contains(&"build/sub/new.txt".to_string()),
        "{files:?}"
    );
    assert!(!files.contains(&"run.log".to_string()), "{files:?}");
    let out = git_text(
        &f.git_dir,
        &["show", &format!("{next}:build/out.txt")],
        &[("GIT_DIR", f.git_dir.as_os_str())],
    )
    .unwrap();
    assert_eq!(out, "rebuilt");
    // Nothing stayed: nothing to name.
    assert_eq!(LeftHere::default().notice(), None);
}

/// A secret-looking file an earlier snapshot sent (before overnight R3)
/// leaves the index at the next one, so the runner's copy no longer has it.
#[test]
fn a_secret_looking_file_sent_before_leaves_the_index() {
    let f = Fixture::new(&[("PLAN.md", "plan\n"), ("client_secret.json", "{}\n")]);
    let index = f.git_dir.join("hollow-index-aaaa");
    let env = [
        ("GIT_DIR", f.git_dir.as_os_str()),
        ("GIT_WORK_TREE", f.here.as_os_str()),
        ("GIT_INDEX_FILE", index.as_os_str()),
    ];
    // As an earlier snapshot took it.
    git_text(&f.here, &["add", "-A", "."], &env).unwrap();
    let (base, left) = snapshot(&f.git_dir, &f.here, "aaaa").unwrap();
    assert_eq!(tree_files(&f.git_dir, &base), vec!["PLAN.md"]);
    assert_eq!(left.secret, vec!["client_secret.json"]);
}

/// The start line names the first few of each kind and counts the rest.
#[test]
fn what_stayed_is_named_for_the_start_line() {
    let left = LeftHere {
        secret: (0..7).map(|i| format!("k{i}.key")).collect(),
        big: vec!["render.mov".into()],
        caches: vec![".venv/".into()],
        ..LeftHere::default()
    };
    assert_eq!(
        left.notice().unwrap(),
        serde_json::json!({
            "secret": { "count": 7, "first": ["k0.key", "k1.key", "k2.key", "k3.key", "k4.key"] },
            "big": { "count": 1, "first": ["render.mov"] },
            "repos": { "count": 0, "first": [] },
            "budget": { "count": 0, "first": [] },
        })
    );
    // Caches are junk: not named.
    let only_caches = LeftHere {
        caches: vec![".venv/".into()],
        ..LeftHere::default()
    };
    assert_eq!(only_caches.notice(), None);
}

/// A result made as the runner would if it took everything (`add -A -f`):
/// what `there` holds, on top of `base`.
fn raw_result(f: &Fixture, base: &str, message: &str) -> String {
    let index = f.git_dir.join("raw-index");
    let _ = std::fs::remove_file(&index);
    let env = [
        ("GIT_DIR", f.git_dir.as_os_str()),
        ("GIT_WORK_TREE", f.there.as_os_str()),
        ("GIT_INDEX_FILE", index.as_os_str()),
    ];
    git_text(&f.there, &["add", "-A", "-f", "."], &env).unwrap();
    let tree = git_text(&f.there, &["write-tree"], &env).unwrap();
    let mut cenv: Vec<(&str, &OsStr)> = env.to_vec();
    for (k, v) in IDENTITY {
        cenv.push((k, OsStr::new(v)));
    }
    git_text(
        &f.there,
        &["commit-tree", &tree, "-p", base, "-m", message],
        &cenv,
    )
    .unwrap()
}

/// Overnight R3 review: on the way back only names dangerous to create in
/// a vault are refused (key, credential and tool-settings files), named in
/// `refused`; a deliverable that merely looks secret (`token-samples.json`)
/// comes back the usual way, and one clashing with a file that stayed here
/// becomes a conflicted copy. A file over the cap there stays there
/// (`too_big`), never read here. Names compare as APFS compares them.
#[test]
fn dangerous_names_are_never_taken_from_the_runner() {
    let f = Fixture::new(&[("PLAN.md", "p\n"), (".gitignore", "build/\n")]);
    std::fs::write(f.here.join(".env"), "TOKEN=here\n").unwrap();
    std::fs::write(f.here.join("password-notes.md"), "mine\n").unwrap();
    let base = f.base();
    for dir in ["build", "config", ".ssh"] {
        std::fs::create_dir_all(f.there.join(dir)).unwrap();
    }
    for (name, text) in [
        ("build/new.txt", "made there\n"),
        (".env", "TOKEN=there\n"),
        (".ENVRC", "export X=1\n"),
        ("config/server.pem", "pem\n"),
        (".ssh/config", "Host x\n"),
        ("id_ecdsa", "key\n"),
        ("token-samples.json", "[]\n"),
        ("password-notes.md", "the runner's\n"),
    ] {
        std::fs::write(f.there.join(name), text).unwrap();
    }
    let huge = std::fs::File::create(f.there.join("build/huge.bin")).unwrap();
    huge.set_len(MAX_FILE_BYTES + 1).unwrap();
    let result = raw_result(&f, &base, "r");
    let back = apply(&f.git_dir, &f.here, &base, &result).unwrap();
    assert_eq!(f.read("build/new.txt"), "made there\n");
    assert_eq!(f.read("token-samples.json"), "[]\n");
    assert_eq!(f.read(".env"), "TOKEN=here\n");
    assert_eq!(f.read("password-notes.md"), "mine\n");
    for gone in [
        ".ENVRC",
        "config/server.pem",
        ".ssh/config",
        "id_ecdsa",
        "build/huge.bin",
    ] {
        assert!(!f.here.join(gone).exists(), "{gone}");
    }
    let mut changed = back.changed.clone();
    changed.sort();
    assert_eq!(changed, vec!["build/new.txt", "token-samples.json"]);
    let mut refused = back.refused.clone();
    refused.sort();
    assert_eq!(
        refused,
        vec![
            ".ENVRC",
            ".env",
            ".ssh/config",
            "config/server.pem",
            "id_ecdsa"
        ]
    );
    assert_eq!(back.copies.len(), 1, "{back:?}");
    assert!(back.copies[0].starts_with("password-notes (conflicted copy"));
    assert_eq!(back.too_big.first, vec!["build/huge.bin"]);
    assert!(back.skipped.is_empty(), "{back:?}");
    for name in [
        "id_rsa",
        "ID_ED25519",
        ".ENV.local",
        "Server.KEY",
        ".npmrc",
        ".kube",
        "AuthKey_X.p8",
    ] {
        assert!(never_taken_back(name), "{name}");
    }
    for name in [
        "token-samples.json",
        "password-reset-preview.html",
        "manage-credentials-1.png",
        "entra-client-secret-request.md",
        "PLAN.md",
    ] {
        assert!(!never_taken_back(name), "{name}");
    }
}

/// What a broad rule calls secret-looking never goes (overnight R3 review
/// added key files and token names `secret_name` doesn't list).
#[test]
fn secret_looking_covers_keys_and_tokens() {
    for name in [
        "id_rsa",
        "ID_ED25519",
        "id_ecdsa",
        "id_dsa.pub",
        ".ENV.local",
        "Server.KEY",
        "\u{17F}ecret",
        "AuthKey_ABC123.p8",
        "release.jks",
        "putty.ppk",
        "vault.kdbx",
        "serviceAccountKey.json",
        "github_token",
        "api-token.txt",
        "token-samples.json",
    ] {
        assert!(secret_looking(name), "{name}");
    }
    for name in [".gitignore", "PLAN.md", "notes.md", "keys"] {
        assert!(!secret_looking(name), "{name}");
    }
}

/// What the runner left out of its result and named in its message comes
/// here as `too_big` and `past_budget`, the first few and how many.
#[test]
fn what_the_runner_left_there_is_named() {
    let f = Fixture::new(&[("PLAN.md", "p\n")]);
    let base = f.base();
    let result = raw_result(
        &f,
        &base,
        "hollow-folder: the turn on the runner\n\nleft: big 1\nleft-path: big render.mov\nleft: budget 2400\nleft-path: budget out/1\nleft-path: budget out/2\n",
    );
    let back = apply(&f.git_dir, &f.here, &base, &result).unwrap();
    assert_eq!(
        back.too_big,
        Named {
            count: 1,
            first: vec!["render.mov".into()]
        }
    );
    assert_eq!(
        back.past_budget,
        Named {
            count: 2400,
            first: vec!["out/1".into(), "out/2".into()]
        }
    );
    assert!(!back.is_empty());
    assert_eq!(
        back.not_brought_json()["pastBudget"],
        serde_json::json!({ "count": 2400, "first": ["out/1", "out/2"] })
    );
}

/// Overnight R3 review: folders that ignore themselves (a venv, a tool's
/// cache) are junk, and a `.gitignore` negation brings nothing that stays
/// back in: the index is checked after the adds.
#[test]
fn caches_stay_and_a_negation_brings_nothing_back_in() {
    let f = Fixture::new(&[
        ("PLAN.md", "p\n"),
        (
            ".gitignore",
            "!node_modules/\n!/sessions/\n!render.mov\n!.env\n",
        ),
        ("node_modules/j.js", "junk\n"),
        ("sessions/s-1.md", "Hollow's own\n"),
        (".env", "TOKEN=here\n"),
        (".venv/.gitignore", "# Created by venv; see the docs\n*\n"),
        (".venv/lib/x.py", "x\n"),
        (
            ".pytest_cache/CACHEDIR.TAG",
            "Signature: 8a477f597d28d172789f06886806bc55\n",
        ),
        (".pytest_cache/v/x", "x\n"),
        ("old-venv/pyvenv.cfg", "home = /usr/bin\n"),
        ("old-venv/bin/python", "x\n"),
        ("notes/.gitignore", "*.tmp\n"),
        ("notes/a.tmp", "goes: not only `*`\n"),
    ]);
    let big = std::fs::File::create(f.here.join("render.mov")).unwrap();
    big.set_len(MAX_FILE_BYTES + 1).unwrap();
    let (base, left) = snapshot(&f.git_dir, &f.here, "aaaa").unwrap();
    assert_eq!(
        tree_files(&f.git_dir, &base),
        vec![".gitignore", "PLAN.md", "notes/.gitignore", "notes/a.tmp"]
    );
    assert_eq!(left.caches, vec![".pytest_cache/", ".venv/", "old-venv/"]);
    assert_eq!(left.big, vec!["render.mov"]);
    // Nothing of a cache here is written into from the runner either.
    std::fs::create_dir_all(f.there.join(".venv/lib")).unwrap();
    std::fs::write(f.there.join(".venv/lib/y.py"), "y\n").unwrap();
    let result = raw_result(&f, &base, "r");
    let back = apply(&f.git_dir, &f.here, &base, &result).unwrap();
    assert!(!f.here.join(".venv/lib/y.py").exists());
    assert!(
        back.skipped.contains(&".venv/lib/y.py".to_string()),
        "{back:?}"
    );
}

/// What only a `.gitignore` names goes within [`GITIGNORED_MAX_FILES`],
/// those sent before counted, in path order; the rest stays and is named.
#[test]
fn the_budget_bounds_what_only_a_gitignore_names() {
    let f = Fixture::new(&[("PLAN.md", "p\n"), (".gitignore", "out/\n")]);
    std::fs::create_dir_all(f.here.join("out")).unwrap();
    for i in 0..GITIGNORED_MAX_FILES + 7 {
        std::fs::write(f.here.join(format!("out/f{i:05}")), "x").unwrap();
    }
    let (base, left) = snapshot(&f.git_dir, &f.here, "aaaa").unwrap();
    assert_eq!(
        tree_files(&f.git_dir, &base).len(),
        GITIGNORED_MAX_FILES + 2
    );
    assert_eq!(left.budget.count, 7);
    assert_eq!(
        left.budget.first[0],
        format!("out/f{:05}", GITIGNORED_MAX_FILES)
    );
    // Next turn: those sent count first, so the same ones stay out.
    let (next, left) = snapshot(&f.git_dir, &f.here, "aaaa").unwrap();
    assert_eq!(tree_files(&f.git_dir, &next), tree_files(&f.git_dir, &base));
    assert_eq!(left.budget.count, 7);
}

/// `update-index --add --remove --stdin`, which adds what only a
/// `.gitignore` names, passes over a listed file that went meanwhile (a long
/// `add -f` pathspec list failed the whole add, exit 128).
#[test]
fn a_listed_file_that_went_meanwhile_doesnt_stop_the_rest() {
    let f = Fixture::new(&[("a.txt", "a\n"), ("b.txt", "b\n")]);
    let index = f.git_dir.join("race-index");
    let env = [
        ("GIT_DIR", f.git_dir.as_os_str()),
        ("GIT_WORK_TREE", f.here.as_os_str()),
        ("GIT_INDEX_FILE", index.as_os_str()),
    ];
    git_input(
        &f.here,
        &["update-index", "--add", "--remove", "-z", "--stdin"],
        &env,
        b"a.txt\0gone.txt\0b.txt\0",
    )
    .unwrap();
    let listed = git_text(&f.here, &["ls-files"], &env).unwrap();
    assert_eq!(listed, "a.txt\nb.txt");
}
