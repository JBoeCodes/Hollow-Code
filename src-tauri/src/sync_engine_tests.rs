//! Tests for the context sync engine (CX6a): temp folders, local bare
//! repositories as remotes, no network.

use super::*;

/// A plain git command in `dir`, which must succeed; its trimmed stdout.
fn sh_git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

struct Lab {
    _dir: tempfile::TempDir,
    base: PathBuf,
}

impl Lab {
    fn new() -> Lab {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        Lab { _dir: dir, base }
    }

    /// A bare remote with branch `main`.
    fn remote(&self, name: &str) -> PathBuf {
        let path = self.base.join(format!("{name}.git"));
        sh_git(
            &self.base,
            &[
                "init",
                "--quiet",
                "--bare",
                "--initial-branch=main",
                path.to_str().unwrap(),
            ],
        );
        path
    }

    /// One Mac: a vault and an app data sync folder, syncing to `remote`.
    fn mac(&self, name: &str, remote: &Path) -> Root {
        let vault = self.base.join(name).join("vault");
        fs::create_dir_all(&vault).unwrap();
        let config = RootConfig {
            id: PERSONAL.into(),
            kind: RootKind::Personal,
            remote: Some(remote.to_str().unwrap().to_string()),
            branch: "main".into(),
            enabled: true,
        };
        let sync = self.base.join(name).join("app").join(SYNC_DIR);
        Root::from_config(&config, &sync, &vault).unwrap()
    }
}

fn write(root: &Root, rel: &str, text: &str) {
    let path = root.work_tree.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn read(root: &Root, rel: &str) -> String {
    fs::read_to_string(root.work_tree.join(rel)).unwrap()
}

fn sync(root: &Root, machine: &str) -> (Outcome, Vec<Alert>) {
    sync_with(root, machine, false)
}

fn sync_with(root: &Root, machine: &str, allow_deletions: bool) -> (Outcome, Vec<Alert>) {
    let ctx = CycleCtx {
        machine: machine.into(),
        actor: "test".into(),
        allow_deletions,
    };
    let mut alerts = Vec::new();
    let outcome = cycle(root, &ctx, &mut |a| alerts.push(a));
    (outcome, alerts)
}

/// Files in `rev` of `root`'s git directory.
fn tree_files(root: &Root, rev: &str) -> Vec<String> {
    let out = git_ok(root, &["ls-tree", "-r", "--name-only", rev], false).unwrap();
    out.text().lines().map(str::to_string).collect()
}

fn remote_files(remote: &Path) -> Vec<String> {
    let out = Command::new("git")
        .arg("--git-dir")
        .arg(remote)
        .args(["ls-tree", "-r", "--name-only", "main"])
        .output()
        .unwrap();
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect()
}

/// A key-shaped value built at run time, so none sits in the source.
fn planted_key() -> String {
    ["sk-", "ant-api03-", &"Q7x".repeat(10)].concat()
}

/// Every file under the vault, with no conflict marker in any of them.
fn assert_no_markers(root: &Root) {
    let mut stack = vec![root.work_tree.clone()];
    while let Some(dir) = stack.pop() {
        for e in fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(text) = fs::read_to_string(&p) {
                assert!(
                    !text.contains("<<<<<<<") && !text.contains(">>>>>>>"),
                    "conflict markers in {}",
                    p.display()
                );
            }
        }
    }
}

#[test]
fn a_planted_secret_blocks_the_commit_and_leaves_everything_alone() {
    let lab = Lab::new();
    let remote = lab.remote("personal");
    let mac = lab.mac("a", &remote);
    write(&mac, "me/Identity.md", "# Identity\nplain\n");
    let secret = format!("note\nKEY {}\n", planted_key());
    write(&mac, "memory/keys.md", &secret);

    let (outcome, alerts) = sync(&mac, "mac-a");
    match &outcome {
        Outcome::Blocked { file, rules } => {
            assert_eq!(file, "memory/keys.md");
            assert!(rules.contains(&"secret.anthropic".to_string()), "{rules:?}");
        }
        other => panic!("{other:?}"),
    }
    // Nothing committed, not even the clean file; nothing pushed; the
    // files are as they were; the index holds nothing staged.
    assert!(rev(&mac, "HEAD").is_none());
    assert!(remote_files(&remote).is_empty());
    assert_eq!(read(&mac, "memory/keys.md"), secret);
    assert_eq!(read(&mac, "me/Identity.md"), "# Identity\nplain\n");
    assert!(git_ok(&mac, &["ls-files"], true).unwrap().text().is_empty());
    // One Waiting on you item names the file and rule, never the value.
    assert_eq!(alerts.len(), 1);
    assert!(alerts[0].title.contains("memory/keys.md"));
    assert!(alerts[0].body.contains("secret.anthropic"));
    assert!(!alerts[0].body.contains("Q7x"));
    let status = status_of(&mac);
    assert_eq!(status.state, SyncState::Blocked);
    assert!(status.text.contains("memory/keys.md"), "{}", status.text);

    // The same block again raises nothing new.
    let (again, alerts) = sync(&mac, "mac-a");
    assert!(matches!(again, Outcome::Blocked { .. }));
    assert!(alerts.is_empty());

    // Fixed: both files go.
    write(
        &mac,
        "memory/keys.md",
        "note\nthe key lives in the Keychain\n",
    );
    let (outcome, _) = sync(&mac, "mac-a");
    assert_eq!(outcome, Outcome::Synced { committed: true });
    assert_eq!(remote_files(&remote), ["me/Identity.md", "memory/keys.md"]);
    assert_eq!(status_of(&mac).state, SyncState::Idle);
}

#[test]
fn an_existing_commit_stays_when_a_later_change_is_blocked() {
    let lab = Lab::new();
    let remote = lab.remote("personal");
    let mac = lab.mac("a", &remote);
    write(&mac, "me/Identity.md", "one\n");
    assert_eq!(sync(&mac, "mac-a").0, Outcome::Synced { committed: true });
    let head = rev(&mac, "HEAD").unwrap();
    write(&mac, "me/Identity.md", "two\n");
    write(&mac, "learnings/x.csv", "a,b\n1,2\n");
    let (outcome, _) = sync(&mac, "mac-a");
    assert_eq!(
        outcome,
        Outcome::Blocked {
            file: "learnings/x.csv".into(),
            rules: vec!["file.data_type".into()]
        }
    );
    assert_eq!(rev(&mac, "HEAD").unwrap(), head);
    // The index is back at HEAD: the modified file isn't staged.
    let staged = git_ok(&mac, &["diff", "--cached", "--name-only"], true).unwrap();
    assert!(staged.text().is_empty(), "{}", staged.text());
}

#[test]
fn two_writers_merge_disjoint_edits() {
    let lab = Lab::new();
    let remote = lab.remote("personal");
    let a = lab.mac("a", &remote);
    let b = lab.mac("b", &remote);
    let lines: Vec<String> = (1..=10).map(|i| format!("line {i}")).collect();
    write(&a, "me/Identity.md", &(lines.join("\n") + "\n"));
    // B has a file of its own before it ever synced: unrelated histories.
    write(&b, "memory/early.md", "from b\n");
    assert_eq!(sync(&a, "mac-a").0, Outcome::Synced { committed: true });
    assert_eq!(sync(&b, "mac-b").0, Outcome::Synced { committed: true });
    assert_eq!(read(&b, "me/Identity.md"), lines.join("\n") + "\n");
    assert_eq!(sync(&a, "mac-a").0, Outcome::Synced { committed: false });
    assert_eq!(read(&a, "memory/early.md"), "from b\n");

    // Disjoint edits to one file, plus a new file on each side.
    let mut on_a = lines.clone();
    on_a[0] = "line 1 edited on a".into();
    write(&a, "me/Identity.md", &(on_a.join("\n") + "\n"));
    write(&a, "learnings/a.md", "a\n");
    let mut on_b = lines.clone();
    on_b[9] = "line 10 edited on b".into();
    write(&b, "me/Identity.md", &(on_b.join("\n") + "\n"));
    write(&b, "home/mac-b.md", "b\n");

    assert_eq!(sync(&a, "mac-a").0, Outcome::Synced { committed: true });
    assert_eq!(sync(&b, "mac-b").0, Outcome::Synced { committed: true });
    assert_eq!(sync(&a, "mac-a").0, Outcome::Synced { committed: false });
    let mut merged = lines.clone();
    merged[0] = on_a[0].clone();
    merged[9] = on_b[9].clone();
    for mac in [&a, &b] {
        assert_eq!(read(mac, "me/Identity.md"), merged.join("\n") + "\n");
        assert_eq!(read(mac, "learnings/a.md"), "a\n");
        assert_eq!(read(mac, "home/mac-b.md"), "b\n");
        assert_eq!(pending_commits(mac), 0);
        assert_no_markers(mac);
    }
    assert_eq!(
        rev(&a, "HEAD"),
        rev(&b, "HEAD"),
        "both Macs end on the same commit"
    );
    // No history was rewritten: every remote commit is in both.
    let log = git_ok(&a, &["log", "--format=%B", "HEAD"], false)
        .unwrap()
        .text();
    assert!(log.contains("Hollow-Machine: mac-a") && log.contains("Hollow-Machine: mac-b"));
}

#[test]
fn a_same_line_clash_keeps_both_versions_and_no_markers() {
    let lab = Lab::new();
    let remote = lab.remote("personal");
    let a = lab.mac("a", &remote);
    let b = lab.mac("b", &remote);
    write(&a, "me/Identity.md", "name: Ada\n");
    sync(&a, "mac-a");
    sync(&b, "mac-b");
    write(&a, "me/Identity.md", "name: Ada Lovelace\n");
    write(&b, "me/Identity.md", "name: Ada King\n");
    assert_eq!(sync(&a, "mac-a").0, Outcome::Synced { committed: true });

    let (outcome, alerts) = sync(&b, "mac-b");
    let copy = "me/Identity (from mac-a).md".to_string();
    assert_eq!(
        outcome,
        Outcome::Clash {
            copies: vec![copy.clone()]
        }
    );
    assert_eq!(read(&b, "me/Identity.md"), "name: Ada King\n");
    assert_eq!(read(&b, &copy), "name: Ada Lovelace\n");
    assert_no_markers(&b);
    // A clash card (CX6c), not an error notification.
    assert!(alerts.is_empty(), "{alerts:?}");
    let open = clash::open(&b);
    assert_eq!(open.len(), 1);
    assert_eq!(
        (open[0].path.as_str(), open[0].copy.as_str()),
        ("me/Identity.md", copy.as_str())
    );
    let status = status_of(&b);
    assert_eq!(status.clashes, 1);
    assert_eq!(status.state, SyncState::Clash);
    assert_eq!(status.clash_copies, std::slice::from_ref(&copy));
    assert_eq!(status.pending_commits, 0, "the merge was pushed");
    // The vault's conflict-copy list offers it beside its original.
    assert_eq!(
        crate::conflict_copies::original_name("Identity (from mac-a).md").as_deref(),
        Some("Identity.md")
    );

    // A takes the merge in: the same two files on both Macs.
    assert_eq!(sync(&a, "mac-a").0, Outcome::Synced { committed: false });
    assert_eq!(read(&a, "me/Identity.md"), "name: Ada King\n");
    assert_eq!(read(&a, &copy), "name: Ada Lovelace\n");
    assert_no_markers(&a);

    // Once the person keeps one, the clash state clears.
    fs::remove_file(b.work_tree.join(&copy)).unwrap();
    assert_eq!(status_of(&b).state, SyncState::Idle);
}

#[test]
fn the_off_switch_leaves_files_alone() {
    let lab = Lab::new();
    let remote = lab.remote("personal");
    let mut mac = lab.mac("a", &remote);
    write(&mac, "me/Identity.md", "one\n");
    mac.enabled = false;
    let before = fs::metadata(mac.work_tree.join("me/Identity.md")).unwrap();
    assert_eq!(sync(&mac, "mac-a").0, Outcome::Off);
    assert!(
        !mac.sync_dir.exists(),
        "no git directory, lock or state made"
    );
    let after = fs::metadata(mac.work_tree.join("me/Identity.md")).unwrap();
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    assert_eq!(read(&mac, "me/Identity.md"), "one\n");
    assert_eq!(status_of(&mac).state, SyncState::Off);
    assert_eq!(status_of(&mac).text, "Sync is off");

    // On, one sync; off again, a change stays local and nothing moves.
    mac.enabled = true;
    assert_eq!(sync(&mac, "mac-a").0, Outcome::Synced { committed: true });
    let head = rev(&mac, "HEAD");
    let pushed = remote_files(&remote);
    mac.enabled = false;
    write(&mac, "me/Identity.md", "two\n");
    write(&mac, "memory/new.md", "new\n");
    let state_before = fs::read(mac.state_path()).unwrap();
    assert_eq!(sync(&mac, "mac-a").0, Outcome::Off);
    assert_eq!(rev(&mac, "HEAD"), head);
    assert_eq!(remote_files(&remote), pushed);
    assert_eq!(read(&mac, "me/Identity.md"), "two\n");
    assert_eq!(read(&mac, "memory/new.md"), "new\n");
    assert_eq!(fs::read(mac.state_path()).unwrap(), state_before);
    assert!(
        mac.git_dir.join("HEAD").is_file(),
        "the git directory is kept"
    );
}

#[test]
fn offline_commits_queue_and_go_when_the_remote_is_back() {
    let lab = Lab::new();
    let missing = lab.base.join("later.git");
    let mac = lab.mac("a", &missing);
    write(&mac, "me/Identity.md", "one\n");
    assert!(matches!(sync(&mac, "mac-a").0, Outcome::Offline { .. }));
    let status = status_of(&mac);
    assert_eq!(status.pending_commits, 1);
    assert_eq!(status.text, "1 change not synced");
    assert!(status.last_error.is_some());
    write(&mac, "memory/two.md", "two\n");
    assert!(matches!(sync(&mac, "mac-a").0, Outcome::Offline { .. }));
    let status = status_of(&mac);
    assert_eq!(
        (status.state, status.pending_commits),
        (SyncState::Offline, 2)
    );
    assert_eq!(status.text, "2 changes not synced");

    let remote = lab.remote("later");
    assert_eq!(remote, missing);
    assert_eq!(sync(&mac, "mac-a").0, Outcome::Synced { committed: false });
    let status = status_of(&mac);
    assert_eq!((status.state, status.pending_commits), (SyncState::Idle, 0));
    assert_eq!(status.text, "Up to date");
    assert_eq!(remote_files(&remote), ["me/Identity.md", "memory/two.md"]);
}

#[test]
fn only_the_allow_list_is_tracked_or_written() {
    let lab = Lab::new();
    let remote = lab.remote("personal");
    let mac = lab.mac("a", &remote);
    write(&mac, "me/Identity.md", "me\n");
    write(&mac, "me/.DS_Store", "finder\n");
    write(&mac, "notes/private.md", "not synced\n");
    write(&mac, "STANDING_ORDERS.md", "not synced\n");
    write(&mac, "memeo/x.md", "a prefix of me, not me\n");
    // The vault's own repo ignores the synced folders; that can't hide
    // them from this sync.
    write(&mac, ".gitignore", "me/\nmemory/\n");
    assert_eq!(sync(&mac, "mac-a").0, Outcome::Synced { committed: true });
    assert_eq!(tree_files(&mac, "HEAD"), ["me/Identity.md"]);

    // The remote has files outside the folders (the repo's README, its
    // templates) and one inside: only the inside one reaches the vault.
    let clone = lab.base.join("plain");
    sh_git(
        &lab.base,
        &["clone", "--quiet", remote.to_str().unwrap(), "plain"],
    );
    fs::create_dir_all(clone.join("home")).unwrap();
    fs::write(clone.join("README.md"), "personal context\n").unwrap();
    fs::write(clone.join("home/mac-a.md"), "the mac\n").unwrap();
    sh_git(&clone, &["add", "-A"]);
    sh_git(&clone, &["commit", "--quiet", "-m", "readme"]);
    sh_git(&clone, &["push", "--quiet", "origin", "HEAD:main"]);
    write(&mac, "memory/x.md", "x\n");
    assert_eq!(sync(&mac, "mac-a").0, Outcome::Synced { committed: true });
    assert!(!mac.work_tree.join("README.md").exists());
    assert_eq!(read(&mac, "home/mac-a.md"), "the mac\n");
    assert_eq!(read(&mac, "STANDING_ORDERS.md"), "not synced\n");
    // The remote keeps its README: the sync never removes what it doesn't
    // track.
    assert_eq!(
        remote_files(&remote),
        [
            "README.md",
            "home/mac-a.md",
            "me/Identity.md",
            "memory/x.md"
        ]
    );
    // A second local change leaves the README alone too.
    write(&mac, "me/Identity.md", "me again\n");
    assert_eq!(sync(&mac, "mac-a").0, Outcome::Synced { committed: true });
    assert!(remote_files(&remote).contains(&"README.md".to_string()));
}

#[test]
fn symlinks_are_left_out_and_the_rest_still_syncs() {
    let lab = Lab::new();
    let remote = lab.remote("personal");
    let mac = lab.mac("a", &remote);
    fs::write(lab.base.join("outside.md"), "outside\n").unwrap();
    write(&mac, "me/Identity.md", "me\n");
    std::os::unix::fs::symlink(
        lab.base.join("outside.md"),
        mac.work_tree.join("me/link.md"),
    )
    .unwrap();
    // The symlink is left out and named; the plain file still syncs.
    let (outcome, alerts) = sync(&mac, "mac-a");
    assert_eq!(outcome, Outcome::Synced { committed: true });
    assert_eq!(remote_files(&remote), ["me/Identity.md"]);
    assert_eq!(alerts.len(), 1);
    assert!(alerts[0].body.contains("me/link.md (a symlink)"));
    let status = status_of(&mac);
    assert_eq!(status.state, SyncState::Blocked);
    assert!(status.text.contains("me/link.md"), "{}", status.text);
    // Same again: no second item.
    assert!(sync(&mac, "mac-a").1.is_empty());
    fs::remove_file(mac.work_tree.join("me/link.md")).unwrap();

    // A synced folder that is itself a symlink.
    fs::create_dir_all(lab.base.join("elsewhere")).unwrap();
    std::os::unix::fs::symlink(lab.base.join("elsewhere"), mac.work_tree.join("memory")).unwrap();
    let (outcome, _) = sync(&mac, "mac-a");
    assert_eq!(
        outcome,
        Outcome::Blocked {
            file: "memory/".into(),
            rules: vec!["a symlink".into()]
        }
    );
    fs::remove_file(mac.work_tree.join("memory")).unwrap();
    assert_eq!(sync(&mac, "mac-a").0, Outcome::Synced { committed: false });
    assert_eq!(status_of(&mac).state, SyncState::Idle);

    // A symlink pushed to the remote is never written into the vault, and a
    // local commit still goes out past it.
    let clone = lab.base.join("plain");
    sh_git(
        &lab.base,
        &["clone", "--quiet", remote.to_str().unwrap(), "plain"],
    );
    std::os::unix::fs::symlink("/etc/hosts", clone.join("me/hosts.md")).unwrap();
    sh_git(&clone, &["add", "-A"]);
    sh_git(&clone, &["commit", "--quiet", "-m", "link"]);
    sh_git(&clone, &["push", "--quiet", "origin", "HEAD:main"]);
    write(&mac, "memory/local.md", "local\n");
    let (outcome, alerts) = sync(&mac, "mac-a");
    assert_eq!(outcome, Outcome::Synced { committed: true });
    assert!(alerts[0].body.contains("me/hosts.md"), "{alerts:?}");
    assert!(fs::symlink_metadata(mac.work_tree.join("me/hosts.md")).is_err());
    assert!(remote_files(&remote).contains(&"memory/local.md".to_string()));
    // Its absence from the vault is never staged as a removal.
    write(&mac, "memory/local.md", "local again\n");
    assert_eq!(sync(&mac, "mac-a").0, Outcome::Synced { committed: true });
    assert!(remote_files(&remote).contains(&"me/hosts.md".to_string()));
}

#[test]
fn a_held_lock_skips_the_cycle() {
    let lab = Lab::new();
    let remote = lab.remote("personal");
    let mac = lab.mac("a", &remote);
    write(&mac, "me/Identity.md", "me\n");
    let held = take_lock(&mac).unwrap();
    assert!(lock_held(&mac));
    assert_eq!(sync(&mac, "mac-a").0, Outcome::Busy);
    assert_eq!(status_of(&mac).state, SyncState::Syncing);
    assert!(!mac.git_dir.exists());
    drop(held);
    assert!(!lock_held(&mac));
    assert_eq!(sync(&mac, "mac-a").0, Outcome::Synced { committed: true });
}

#[test]
fn stale_git_locks_are_cleared_and_fresh_ones_kept() {
    let lab = Lab::new();
    let gd = lab.base.join("x.git");
    fs::create_dir_all(gd.join("refs/heads")).unwrap();
    fs::write(gd.join("index.lock"), "").unwrap();
    fs::write(gd.join("refs/heads/main.lock"), "").unwrap();
    clear_stale_git_locks(&gd, Duration::from_secs(3600));
    assert!(gd.join("index.lock").exists(), "a fresh lock stays");
    clear_stale_git_locks(&gd, Duration::ZERO);
    assert!(!gd.join("index.lock").exists());
    assert!(!gd.join("refs/heads/main.lock").exists());
}

#[test]
fn remotes_with_secrets_or_odd_transports_are_refused() {
    for ok in [
        "https://github.com/owner/personal-context.git",
        "ssh://git@github.com/owner/repo.git",
        "git@github.com:owner/personal-context.git",
        "/Users/someone/remote.git",
        "file:///tmp/remote.git",
    ] {
        assert!(valid_remote(ok).is_ok(), "{ok}");
    }
    for bad in [
        "",
        "https://user:token@github.com/owner/repo.git",
        "ext::sh -c touch% /tmp/x",
        "-uhelp",
        "https://github.com/a b",
        "http://github.com/owner/repo.git",
        "relative/path",
        "https:///nohost",
    ] {
        assert!(valid_remote(bad).is_err(), "{bad}");
    }
}

#[test]
fn copy_names_follow_the_conflict_copy_form() {
    let none = |_: &str| false;
    assert_eq!(
        copy_name("me/Identity.md", "mac-a", &none),
        "me/Identity (from mac-a).md"
    );
    assert_eq!(
        copy_name("memory/README", "mac-a", &none),
        "memory/README (from mac-a)"
    );
    assert_eq!(
        copy_name("me/.hidden", "mac-a", &none),
        "me/.hidden (from mac-a)"
    );
    let first_taken = |c: &str| c == "me/a (from mac-a).md";
    assert_eq!(
        copy_name("me/a.md", "mac-a", &first_taken),
        "me/a (from mac-a 2).md"
    );
    for name in ["a (from mac-a).md", "a (from mac-a 2).md"] {
        assert_eq!(
            crate::conflict_copies::original_name(name).as_deref(),
            Some("a.md"),
            "{name}"
        );
    }
}

#[test]
fn settings_save_refuses_a_broken_file_and_keeps_other_roots() {
    let lab = Lab::new();
    let path = lab.base.join("context-sync.json");
    fs::write(&path, "{not json").unwrap();
    assert!(update_root_at(&path, PERSONAL, |_| Ok(())).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "{not json");
    assert_eq!(config_at(&path), SyncConfig::default());

    fs::remove_file(&path).unwrap();
    let saved = update_root_at(&path, PERSONAL, |r| {
        r.remote = Some("git@github.com:o/r.git".into());
        r.enabled = true;
        Ok(())
    })
    .unwrap();
    assert!(saved.enabled);
    update_root_at(&path, "vault", |r| {
        r.kind = RootKind::Vault;
        Ok(())
    })
    .unwrap();
    let config = config_at(&path);
    assert_eq!(config.roots.len(), 2);
    assert_eq!(config.root(PERSONAL), Some(&saved));
    // The vault root builds (CX6b) with no remote; the org root waits (V6).
    let vault = config.root("vault").unwrap();
    assert!(Root::from_config(vault, &lab.base, &lab.base).is_ok());
    let mut org = vault.clone();
    org.kind = RootKind::Org;
    assert!(Root::from_config(&org, &lab.base, &lab.base).is_err());
    assert!(!fs::read_to_string(&path).unwrap().contains("token"));
}

#[test]
fn a_missing_synced_folder_is_never_a_mass_deletion() {
    let lab = Lab::new();
    let remote = lab.remote("personal");
    let mac = lab.mac("a", &remote);
    for i in 0..5 {
        write(&mac, &format!("memory/f{i}.md"), "fact\n");
    }
    write(&mac, "me/Identity.md", "me\n");
    assert_eq!(sync(&mac, "mac-a").0, Outcome::Synced { committed: true });
    let head = rev(&mac, "HEAD");

    // The folder is gone (evicted, unmounted, a move half done).
    fs::rename(mac.work_tree.join("memory"), lab.base.join("away")).unwrap();
    let (outcome, alerts) = sync(&mac, "mac-a");
    assert_eq!(
        outcome,
        Outcome::Blocked {
            file: "memory/".into(),
            rules: vec!["missing from the vault (nothing was deleted)".into()]
        }
    );
    assert_eq!(alerts.len(), 1);
    assert_eq!(rev(&mac, "HEAD"), head);
    assert_eq!(remote_files(&remote).len(), 6);
    fs::rename(lab.base.join("away"), mac.work_tree.join("memory")).unwrap();
    assert_eq!(sync(&mac, "mac-a").0, Outcome::Synced { committed: false });

    // More than half the files gone at once is refused too.
    for i in 0..4 {
        fs::remove_file(mac.work_tree.join(format!("memory/f{i}.md"))).unwrap();
    }
    let (outcome, _) = sync(&mac, "mac-a");
    match &outcome {
        Outcome::Blocked { file, rules } => {
            assert_eq!(file, "4 files");
            assert!(rules[0].contains("4 of 6"), "{rules:?}");
            assert!(rules[0].contains("allow removals"), "the way out is named");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(rev(&mac, "HEAD"), head);
    assert_eq!(status_of(&mac).state, SyncState::Blocked);
    // The person confirms once: that cycle removes them.
    assert_eq!(
        sync_with(&mac, "mac-a", true).0,
        Outcome::Synced { committed: true }
    );
    assert_eq!(remote_files(&remote).len(), 2);
    // The confirm was for one cycle only.
    fs::remove_file(mac.work_tree.join("me/Identity.md")).unwrap();
    fs::remove_file(mac.work_tree.join("memory/f4.md")).unwrap();
    assert!(matches!(sync(&mac, "mac-a").0, Outcome::Blocked { .. }));
}

#[test]
fn removals_from_the_remote_pass_the_same_guard() {
    let lab = Lab::new();
    let remote = lab.remote("personal");
    let a = lab.mac("a", &remote);
    let b = lab.mac("b", &remote);
    for i in 0..6 {
        write(&a, &format!("memory/f{i}.md"), "fact\n");
    }
    sync(&a, "mac-a");
    sync(&b, "mac-b");
    for i in 0..4 {
        fs::remove_file(a.work_tree.join(format!("memory/f{i}.md"))).unwrap();
    }
    assert_eq!(
        sync_with(&a, "mac-a", true).0,
        Outcome::Synced { committed: true }
    );
    let head = rev(&b, "HEAD");
    let (outcome, alerts) = sync(&b, "mac-b");
    match &outcome {
        Outcome::Blocked { file, rules } => {
            assert_eq!(file, "4 files");
            assert!(rules[0].contains("the remote removes 4 of 6"), "{rules:?}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(alerts.len(), 1);
    assert_eq!(rev(&b, "HEAD"), head);
    assert!(
        b.work_tree.join("memory/f0.md").exists(),
        "nothing removed here"
    );
    assert_eq!(
        sync_with(&b, "mac-b", true).0,
        Outcome::Synced { committed: false }
    );
    assert!(!b.work_tree.join("memory/f0.md").exists());
    assert!(b.work_tree.join("memory/f5.md").exists());
}

#[test]
fn a_secret_from_the_remote_is_withheld_and_the_rest_still_syncs() {
    let lab = Lab::new();
    let remote = lab.remote("personal");
    let a = lab.mac("a", &remote);
    let b = lab.mac("b", &remote);
    write(&a, "me/Identity.md", "me\n");
    sync(&a, "mac-a");
    sync(&b, "mac-b");
    // Someone pushes a key past the scan (a plain clone, no Hollow).
    let clone = lab.base.join("plain");
    sh_git(
        &lab.base,
        &["clone", "--quiet", remote.to_str().unwrap(), "plain"],
    );
    fs::write(clone.join("me/keys.md"), format!("KEY {}\n", planted_key())).unwrap();
    sh_git(&clone, &["add", "-A"]);
    sh_git(&clone, &["commit", "--quiet", "-m", "keys"]);
    sh_git(&clone, &["push", "--quiet", "origin", "HEAD:main"]);
    write(&b, "memory/b.md", "b\n");
    let head = rev(&b, "HEAD");
    let (outcome, alerts) = sync(&b, "mac-b");
    assert_eq!(outcome, Outcome::Synced { committed: true });
    assert_eq!(alerts.len(), 1);
    assert!(alerts[0].body.contains("me/keys.md"));
    assert!(alerts[0]
        .body
        .contains("remove it on the remote or on the Mac that pushed it"));
    assert!(!alerts[0].body.contains("Q7x"));
    assert!(
        !b.work_tree.join("me/keys.md").exists(),
        "never written here"
    );
    assert_ne!(rev(&b, "HEAD"), head);
    assert!(
        remote_files(&remote).contains(&"memory/b.md".to_string()),
        "pushed"
    );
    assert_eq!(status_of(&b).pending_commits, 0);
    assert_eq!(status_of(&b).unsynced_files, 0);
    // Its absence here is never pushed as a removal.
    write(&b, "memory/b.md", "b2\n");
    sync(&b, "mac-b");
    assert!(remote_files(&remote).contains(&"me/keys.md".to_string()));
    // Fixed on the remote: the clean version arrives.
    fs::write(clone.join("me/keys.md"), "the key lives in the Keychain\n").unwrap();
    sh_git(
        &clone,
        &["pull", "--quiet", "--no-rebase", "origin", "main"],
    );
    fs::write(clone.join("me/keys.md"), "the key lives in the Keychain\n").unwrap();
    sh_git(&clone, &["commit", "--quiet", "-am", "fix"]);
    sh_git(&clone, &["push", "--quiet", "origin", "HEAD:main"]);
    assert_eq!(sync(&b, "mac-b").0, Outcome::Synced { committed: false });
    assert_eq!(read(&b, "me/keys.md"), "the key lives in the Keychain\n");
    assert_eq!(status_of(&b).state, SyncState::Idle);
}

#[test]
fn a_file_and_folder_clash_keeps_both() {
    let lab = Lab::new();
    let remote = lab.remote("personal");
    let a = lab.mac("a", &remote);
    let b = lab.mac("b", &remote);
    write(&a, "memory/base.md", "base\n");
    sync(&a, "mac-a");
    sync(&b, "mac-b");
    write(&a, "memory/x/y.md", "a folder\n");
    write(&b, "memory/x", "b file\n");
    assert_eq!(sync(&a, "mac-a").0, Outcome::Synced { committed: true });
    let (outcome, _) = sync(&b, "mac-b");
    assert!(matches!(&outcome, Outcome::Clash { .. }), "{outcome:?}");
    assert_eq!(read(&b, "memory/x/y.md"), "a folder\n");
    // B's file is kept under another name in the merge, and B's own copy
    // wasn't removed from the remote's view of history.
    let tree = tree_files(&b, "HEAD");
    let kept: Vec<&String> = tree
        .iter()
        .filter(|p| p.starts_with("memory/x") && *p != "memory/x/y.md")
        .collect();
    assert_eq!(kept.len(), 1, "{tree:?}");
    let blob_text = git_ok(&b, &["show", &format!("HEAD:{}", kept[0])], false)
        .unwrap()
        .text();
    assert_eq!(blob_text, "b file");
    assert_no_markers(&b);
    assert_eq!(sync(&a, "mac-a").0, Outcome::Synced { committed: false });
    assert_eq!(read(&a, "memory/x/y.md"), "a folder\n");
}

#[test]
fn a_rewritten_remote_history_is_named() {
    let lab = Lab::new();
    let remote = lab.remote("personal");
    let mac = lab.mac("a", &remote);
    write(&mac, "me/Identity.md", "one\n");
    sync(&mac, "mac-a");
    write(&mac, "me/Identity.md", "two\n");
    sync(&mac, "mac-a");
    // Someone force-pushes a different history.
    let clone = lab.base.join("plain");
    sh_git(&lab.base, &["init", "--quiet", "-b", "main", "plain"]);
    fs::create_dir_all(clone.join("me")).unwrap();
    fs::write(clone.join("me/Other.md"), "other\n").unwrap();
    sh_git(&clone, &["add", "-A"]);
    sh_git(&clone, &["commit", "--quiet", "-m", "rewrite"]);
    sh_git(
        &clone,
        &[
            "push",
            "--quiet",
            "--force",
            remote.to_str().unwrap(),
            "HEAD:main",
        ],
    );
    let (outcome, alerts) = sync(&mac, "mac-a");
    assert_eq!(outcome, Outcome::Synced { committed: false });
    assert!(
        alerts.iter().any(|a| a.title.contains("rewritten")),
        "{alerts:?}"
    );
    // Nothing lost: this Mac's file is pushed back beside the new one.
    assert_eq!(remote_files(&remote), ["me/Identity.md", "me/Other.md"]);
    assert_eq!(read(&mac, "me/Identity.md"), "two\n");
}

#[test]
fn an_empty_first_cycle_commits_nothing_and_a_fresh_orphan_lock_is_cleared() {
    let lab = Lab::new();
    let remote = lab.remote("personal");
    let mac = lab.mac("a", &remote);
    fs::create_dir_all(mac.work_tree.join("me")).unwrap();
    assert_eq!(sync(&mac, "mac-a").0, Outcome::Synced { committed: false });
    assert!(rev(&mac, "HEAD").is_none());
    // A git that died left index.lock a moment ago; no git runs now.
    fs::write(mac.git_dir.join("index.lock"), "").unwrap();
    write(&mac, "me/Identity.md", "me\n");
    assert_eq!(sync(&mac, "mac-a").0, Outcome::Synced { committed: true });
    assert!(!mac.git_dir.join("index.lock").exists());
}

#[test]
fn the_off_switch_stops_a_cycle_before_it_writes() {
    let lab = Lab::new();
    let remote = lab.remote("personal");
    let a = lab.mac("a", &remote);
    let mut b = lab.mac("b", &remote);
    write(&a, "me/Identity.md", "me\n");
    sync(&a, "mac-a");
    // B started on, but its settings now say off.
    let settings = lab.base.join("b-settings.json");
    update_root_at(&settings, PERSONAL, |r| {
        r.remote = Some(remote.to_str().unwrap().into());
        r.enabled = false;
        Ok(())
    })
    .unwrap();
    b.settings_file = Some(settings);
    assert_eq!(sync(&b, "mac-b").0, Outcome::Off);
    assert!(!b.work_tree.join("me/Identity.md").exists());
    assert!(rev(&b, "HEAD").is_none());
}

#[test]
fn a_file_changed_after_the_check_is_never_overwritten() {
    let lab = Lab::new();
    let remote = lab.remote("personal");
    let a = lab.mac("a", &remote);
    let b = lab.mac("b", &remote);
    write(&a, "me/Identity.md", "one\n");
    sync(&a, "mac-a");
    sync(&b, "mac-b");
    write(&a, "me/Identity.md", "from a\n");
    sync(&a, "mac-a");
    // An agent writes the file between the check and the write.
    let target = b.work_tree.join("me/Identity.md");
    BEFORE_WRITE.with(|h| {
        *h.borrow_mut() = Some(Box::new(move |_: &str| {
            fs::write(&target, "agent edit\n").unwrap();
        }))
    });
    let (outcome, _) = sync(&b, "mac-b");
    BEFORE_WRITE.with(|h| *h.borrow_mut() = None);
    assert!(matches!(outcome, Outcome::Retry { .. }), "{outcome:?}");
    assert_eq!(read(&b, "me/Identity.md"), "agent edit\n");
    // The next cycle commits the edit and merges it like any clash.
    assert!(matches!(sync(&b, "mac-b").0, Outcome::Clash { .. }));
    assert_eq!(read(&b, "me/Identity.md"), "agent edit\n");

    let mac = lab.mac("c", &remote);
    write(&mac, "me/Identity.md", "one\n");
    assert!(holds(&mac, "me/Identity.md", Some(b"one\n")).is_ok());
    write(&mac, "me/Identity.md", "edited\n");
    assert!(holds(&mac, "me/Identity.md", Some(b"one\n")).is_err());
    assert!(holds(&mac, "me/New.md", None).is_ok());
    write(&mac, "me/New.md", "x\n");
    assert!(holds(&mac, "me/New.md", None).is_err());
}

#[test]
fn unsafe_names_modes_and_sizes_are_refused() {
    let lab = Lab::new();
    let remote = lab.remote("personal");
    let mac = lab.mac("a", &remote);
    for path in [
        "me/.git/config",
        "me/.GIT/hooks/x",
        "memory/a/.Git",
        "me/../x",
        "me//x",
    ] {
        assert!(refuse(&mac, path, "100644", Some(1)).is_some(), "{path}");
    }
    assert!(refuse(&mac, "me/ok.md", "100644", Some(1)).is_none());
    assert!(refuse(&mac, "me/big.md", "100644", Some(OUT_CAP as u64 + 1)).is_some());
    assert!(refuse(&mac, "me/l.md", "120000", Some(1)).is_some());
    assert!(GIT_CONFIG.contains(&"transfer.fsckObjects=true"));
    assert!(GIT_CONFIG.contains(&"fetch.fsckObjects=true"));
}

#[test]
fn credentials_never_show_in_a_remote_or_an_error() {
    assert!(valid_remote("https://ghp_TOKEN@github.com/o/r.git").is_err());
    assert!(valid_remote("https://user@github.com/o/r.git").is_err());
    assert!(valid_remote("ssh://git@github.com/o/r.git").is_ok());
    assert_eq!(
        redact_urls("fatal: could not read https://abc123@github.com/o/r.git: denied"),
        "fatal: could not read https://***@github.com/o/r.git: denied"
    );
    assert_eq!(redact_urls("ssh://git@host/x"), "ssh://git@host/x");
    assert_eq!(
        redact_urls("https://h/x?api_key=abc&client_secret=def&refresh_token=ghi&q=1"),
        "https://h/x?api_key=***&client_secret=***&refresh_token=***&q=1"
    );
}

#[test]
fn working_changes_count_as_not_synced() {
    let lab = Lab::new();
    let remote = lab.remote("personal");
    let mac = lab.mac("a", &remote);
    write(&mac, "me/Identity.md", "one\n");
    sync(&mac, "mac-a");
    assert_eq!(status_of(&mac).text, "Up to date");
    write(&mac, "me/Identity.md", "two\n");
    write(&mac, "memory/new.md", "new\n");
    write(&mac, "me/.DS_Store", "finder\n");
    write(&mac, "memory/sub/.DS_Store", "finder\n");
    let status = status_of(&mac);
    assert_eq!((status.pending_commits, status.unsynced_files), (0, 2));
    assert_eq!(status.text, "2 changes not synced");
}

#[test]
fn the_off_switch_stops_writes_mid_apply() {
    let lab = Lab::new();
    let remote = lab.remote("personal");
    let a = lab.mac("a", &remote);
    let mut b = lab.mac("b", &remote);
    write(&a, "me/one.md", "1\n");
    write(&a, "me/two.md", "2\n");
    sync(&a, "mac-a");
    let settings = lab.base.join("b-settings.json");
    let on = |enabled: bool| {
        update_root_at(&settings, PERSONAL, |r| {
            r.remote = Some("/x.git".into());
            r.enabled = enabled;
            Ok(())
        })
        .unwrap();
    };
    on(true);
    b.settings_file = Some(settings.clone());
    // The person turns sync off after the first file is written.
    let mut seen = 0;
    let flip = settings.clone();
    BEFORE_WRITE.with(|h| {
        *h.borrow_mut() = Some(Box::new(move |_: &str| {
            seen += 1;
            if seen == 2 {
                update_root_at(&flip, PERSONAL, |r| {
                    r.enabled = false;
                    Ok(())
                })
                .unwrap();
            }
        }))
    });
    let (outcome, _) = sync(&b, "mac-b");
    BEFORE_WRITE.with(|h| *h.borrow_mut() = None);
    assert_eq!(outcome, Outcome::Off);
    assert!(b.work_tree.join("me/one.md").exists());
    assert!(
        !b.work_tree.join("me/two.md").exists(),
        "no write after off"
    );
    assert!(rev(&b, "HEAD").is_none(), "HEAD never moved");
}

#[test]
fn a_folder_that_became_a_file_and_back() {
    let lab = Lab::new();
    let remote = lab.remote("personal");
    let a = lab.mac("a", &remote);
    let b = lab.mac("b", &remote);
    write(&a, "memory/base.md", "base\n");
    write(&a, "memory/x/y.md", "in a folder\n");
    sync(&a, "mac-a");
    sync(&b, "mac-b");
    write(&b, "memory/x/.DS_Store", "finder\n");
    // Folder to file on A.
    fs::remove_dir_all(a.work_tree.join("memory/x")).unwrap();
    write(&a, "memory/x", "now a file\n");
    assert_eq!(sync(&a, "mac-a").0, Outcome::Synced { committed: true });
    assert_eq!(sync(&b, "mac-b").0, Outcome::Synced { committed: false });
    assert_eq!(read(&b, "memory/x"), "now a file\n");
    // And back to a folder.
    fs::remove_file(a.work_tree.join("memory/x")).unwrap();
    write(&a, "memory/x/z.md", "a folder again\n");
    assert_eq!(sync(&a, "mac-a").0, Outcome::Synced { committed: true });
    assert_eq!(sync(&b, "mac-b").0, Outcome::Synced { committed: false });
    assert_eq!(read(&b, "memory/x/z.md"), "a folder again\n");
}

#[test]
fn a_clashing_file_that_quotes_a_marker_still_merges() {
    let lab = Lab::new();
    let remote = lab.remote("personal");
    let a = lab.mac("a", &remote);
    let b = lab.mac("b", &remote);
    write(&a, "learnings/git.md", "base\n");
    sync(&a, "mac-a");
    sync(&b, "mac-b");
    write(
        &a,
        "learnings/git.md",
        "a conflict looks like\n<<<<<<< HEAD\n",
    );
    write(&b, "learnings/git.md", "b says\n>>>>>>> theirs\n");
    sync(&a, "mac-a");
    let (outcome, _) = sync(&b, "mac-b");
    assert!(matches!(outcome, Outcome::Clash { .. }), "{outcome:?}");
    assert_eq!(read(&b, "learnings/git.md"), "b says\n>>>>>>> theirs\n");
    assert_eq!(
        read(&b, "learnings/git (from mac-a).md"),
        "a conflict looks like\n<<<<<<< HEAD\n"
    );
}
