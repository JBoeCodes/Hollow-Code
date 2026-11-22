//! Tests for the vault root of the context sync engine (CX6b): temp folders,
//! no remote, no network.

use super::vault::{self, MANIFEST, MAX_BYTES};
use super::*;

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

    /// The vault root of one Mac, on.
    fn vault_root(&self) -> Root {
        let vault = self.base.join("vault");
        fs::create_dir_all(&vault).unwrap();
        let config = RootConfig {
            id: VAULT.into(),
            kind: RootKind::Vault,
            remote: None,
            branch: "main".into(),
            enabled: true,
        };
        Root::from_config(&config, &self.base.join("app").join(SYNC_DIR), &vault).unwrap()
    }
}

fn put(root: &Root, rel: &str, bytes: &[u8]) {
    let path = root.work_tree.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn sync(root: &Root) -> (Outcome, Vec<Alert>) {
    sync_with(root, false)
}

fn sync_with(root: &Root, allow_deletions: bool) -> (Outcome, Vec<Alert>) {
    let ctx = CycleCtx {
        machine: "mac-a".into(),
        actor: "test".into(),
        allow_deletions,
    };
    let mut alerts = Vec::new();
    let outcome = cycle(root, &ctx, &mut |a| alerts.push(a));
    (outcome, alerts)
}

fn head_files(root: &Root) -> Vec<String> {
    let out = git_ok(root, &["ls-tree", "-r", "--name-only", "HEAD"], false).unwrap();
    out.text().lines().map(str::to_string).collect()
}

/// A key-shaped value built at run time, so none sits in the source.
fn planted_key() -> String {
    ["sk-", "ant-api03-", &"Q7x".repeat(10)].concat()
}

/// A text file of `n` bytes.
fn text_of(n: usize) -> Vec<u8> {
    let line = b"plain words in a long note\n";
    line.iter().copied().cycle().take(n).collect()
}

/// Every file under `dir` with its bytes.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            let kind = e.file_type().unwrap();
            if kind.is_dir() {
                stack.push(p);
            } else if kind.is_file() {
                out.insert(p.clone(), fs::read(&p).unwrap());
            }
        }
    }
    out
}

/// A vault with one file of each kind the root must keep or leave out.
fn plant(root: &Root) {
    put(root, "AGENTS.md", b"# Vault\n");
    put(root, "library/work/brand.md", b"# Brand\n");
    put(root, "workspaces/w/WORKSPACE.md", b"# W\n");
    put(root, "workspaces/w/sessions/s-1.md", b"a chat\n");
    put(root, "workspaces/w/canvas.json", b"{\"a\":1}\n");
    // Out: the personal root's folders.
    put(root, "me/Identity.md", b"me\n");
    put(root, "memory/m.md", b"memory\n");
    put(root, "learnings/l.md", b"learning\n");
    put(root, "home/mac-a.md", b"home\n");
    // Out: tasks, history and per-Mac state.
    put(root, "tasks/follow-up.md", b"---\ntitle: x\n---\n");
    put(root, ".hollow-history/runs/r.json", b"{}\n");
    put(root, ".notifications.jsonl", b"{}\n");
    put(root, ".review-queue.json", b"{}\n");
    put(root, "workspaces/w/.computer-actions.jsonl", b"{}\n");
    // Out: big, binary, data and key files, dependencies, media.
    put(
        root,
        "workspaces/w/outputs/big.md",
        &text_of(6 * 1024 * 1024),
    );
    put(
        root,
        "workspaces/w/outputs/shot.png",
        b"\x89PNG\r\n\x1a\n\0\0",
    );
    put(
        root,
        "workspaces/w/outputs/blob.md",
        b"looks like text\0but is not",
    );
    put(root, "workspaces/w/outputs/rows.csv", b"a,b\n1,2\n");
    put(root, "workspaces/w/.env", b"TOKEN=x\n");
    put(
        root,
        "workspaces/w/node_modules/p/index.js",
        b"module.exports = 1;\n",
    );
    put(root, "workspaces/w/outputs/screenshots/a.md", b"caption\n");
}

const KEPT: [&str; 5] = [
    "AGENTS.md",
    "library/work/brand.md",
    "workspaces/w/WORKSPACE.md",
    "workspaces/w/canvas.json",
    "workspaces/w/sessions/s-1.md",
];

#[test]
fn the_first_commit_is_text_only_and_local_only() {
    let lab = Lab::new();
    let root = lab.vault_root();
    plant(&root);
    let before = snapshot(&root.work_tree);

    let (outcome, alerts) = sync(&root);
    assert_eq!(outcome, Outcome::LocalOnly { committed: true });
    assert!(alerts.is_empty(), "{alerts:?}");
    // The planted 6 MB file, the binary ones, tasks/, history, state, the
    // personal folders and the key and data files all stayed out.
    assert_eq!(head_files(&root), KEPT);
    // Nothing in the vault changed and no ignore file was written into it.
    assert_eq!(snapshot(&root.work_tree), before);
    assert!(!root.work_tree.join(".gitignore").exists());
    // The generated ignore sits in the git directory in app data.
    let exclude = fs::read_to_string(root.git_dir.join("info/exclude")).unwrap();
    assert_eq!(exclude, vault::exclude_text(&MANIFEST));
    assert!(!root.git_dir.starts_with(&root.work_tree));

    // No remote activity: no remote configured, no remote refs, no fetch.
    let url = git_run(
        &root,
        &["config", "--get-regexp", "^remote\\."],
        false,
        None,
        &[],
        LOCAL_TIMEOUT,
    )
    .unwrap();
    assert_ne!(url.code, 0, "a remote was configured: {}", url.text());
    let refs = git_ok(&root, &["for-each-ref", "refs/remotes"], false).unwrap();
    assert!(refs.text().is_empty());
    assert!(!root.git_dir.join("FETCH_HEAD").exists());

    let status = status_of(&root);
    assert_eq!(status.state, SyncState::LocalOnly);
    assert_eq!(status.pending_commits, 0);
    assert_eq!(status.unsynced_files, 0);
    assert!(status.text.starts_with("Local only"), "{}", status.text);
    assert_eq!(
        serde_json::to_value(SyncState::LocalOnly).unwrap(),
        "localOnly"
    );

    // A new note counts as not committed yet, then goes in.
    put(&root, "workspaces/w/PLAN.md", b"- [ ] one\n");
    assert_eq!(status_of(&root).unsynced_files, 1);
    assert_eq!(sync(&root).0, Outcome::LocalOnly { committed: true });
    assert!(head_files(&root).contains(&"workspaces/w/PLAN.md".to_string()));
    assert_eq!(sync(&root).0, Outcome::LocalOnly { committed: false });
}

#[test]
fn a_planted_secret_stays_out_named_while_the_rest_commits() {
    let lab = Lab::new();
    let root = lab.vault_root();
    plant(&root);
    let secret = format!("notes\nKEY {}\n", planted_key());
    put(&root, "workspaces/w/outputs/notes.md", secret.as_bytes());

    let (outcome, alerts) = sync(&root);
    assert_eq!(outcome, Outcome::LocalOnly { committed: true });
    // The secret's file is neither committed nor staged; the rest is; the
    // file is as it was.
    assert_eq!(head_files(&root), KEPT);
    let index = git_ok(&root, &["ls-files"], false).unwrap().text();
    assert!(!index.contains("notes.md"), "{index}");
    assert_eq!(
        fs::read_to_string(root.work_tree.join("workspaces/w/outputs/notes.md")).unwrap(),
        secret
    );
    // One Waiting on you item names file and rule, never the value.
    assert_eq!(alerts.len(), 1);
    assert!(
        alerts[0]
            .body
            .contains("workspaces/w/outputs/notes.md (holds secret.anthropic)"),
        "{}",
        alerts[0].body
    );
    assert!(!alerts[0].body.contains("Q7x"));
    // Again: still out, nothing new raised.
    let (again, alerts) = sync(&root);
    assert_eq!(again, Outcome::LocalOnly { committed: false });
    assert!(alerts.is_empty());
    let status = status_of(&root);
    assert_eq!(status.state, SyncState::Blocked);
    assert!(status.text.contains("notes.md"), "{}", status.text);

    // Fixed: it goes in, still local only.
    put(
        &root,
        "workspaces/w/outputs/notes.md",
        b"notes\nthe key is in the Keychain\n",
    );
    assert_eq!(sync(&root).0, Outcome::LocalOnly { committed: true });
    assert!(head_files(&root).contains(&"workspaces/w/outputs/notes.md".to_string()));
    assert_eq!(status_of(&root).state, SyncState::LocalOnly);
}

#[test]
fn the_off_switch_leaves_everything_alone() {
    let lab = Lab::new();
    let mut root = lab.vault_root();
    plant(&root);
    let before = snapshot(&root.work_tree);
    root.enabled = false;
    assert_eq!(sync(&root).0, Outcome::Off);
    assert!(!root.git_dir.exists());
    assert!(!root.sync_dir.exists());
    assert_eq!(snapshot(&root.work_tree), before);
    assert_eq!(status_of(&root).state, SyncState::Off);

    // On, commit, off again: the git directory and vault stay as they are.
    root.enabled = true;
    assert_eq!(sync(&root).0, Outcome::LocalOnly { committed: true });
    let head = rev(&root, "HEAD").unwrap();
    put(&root, "workspaces/w/WORKSPACE.md", b"# W changed\n");
    let before = snapshot(&root.work_tree);
    let git_before = snapshot(&root.git_dir);
    root.enabled = false;
    assert_eq!(sync(&root).0, Outcome::Off);
    assert_eq!(rev(&root, "HEAD").unwrap(), head);
    assert_eq!(snapshot(&root.work_tree), before);
    assert_eq!(snapshot(&root.git_dir), git_before);
}

#[test]
fn a_changed_manifest_regenerates_the_ignore_and_touches_no_vault_file() {
    let lab = Lab::new();
    let root = lab.vault_root();
    plant(&root);
    put(&root, "workspaces/w/.gitignore", b"# mine\ndrafts/\n");
    put(&root, "workspaces/w/drafts/d.md", b"draft\n");
    assert_eq!(sync(&root).0, Outcome::LocalOnly { committed: true });
    // The vault's own .gitignore is honoured, and kept as it is.
    let files = head_files(&root);
    assert!(!files.contains(&"workspaces/w/drafts/d.md".to_string()));
    assert!(files.contains(&"workspaces/w/.gitignore".to_string()));

    // An older manifest's ignore is replaced at the next cycle.
    let exclude = root.git_dir.join("info/exclude");
    let older = vault::Manifest {
        version: 0,
        top_dirs: &[],
        any_dirs: &[],
        binary_exts: &["png"],
        key_file_globs: &[],
    };
    assert!(vault::ensure_exclude(&root, &older).unwrap());
    assert!(fs::read_to_string(&exclude)
        .unwrap()
        .contains("manifest v0"));
    let before = snapshot(&root.work_tree);
    assert_eq!(sync(&root).0, Outcome::LocalOnly { committed: false });
    assert_eq!(
        fs::read_to_string(&exclude).unwrap(),
        vault::exclude_text(&MANIFEST)
    );
    assert_eq!(snapshot(&root.work_tree), before);
    // Unchanged manifest: nothing rewritten.
    assert!(!vault::ensure_exclude(&root, &MANIFEST).unwrap());

    // A new manifest that leaves a tracked folder out: the next cycle stages
    // nothing there, and the committed copy stays (V6 decides untracking).
    let newer = vault::Manifest {
        version: 2,
        top_dirs: &["tasks", "library"],
        ..MANIFEST
    };
    assert!(!vault::path_allowed_by(&newer, "library/work/brand.md"));
    assert!(vault::exclude_text(&newer).contains("/library/\n"));
}

#[test]
fn a_tracked_file_that_grows_too_big_or_turns_binary_keeps_its_last_version() {
    let lab = Lab::new();
    let root = lab.vault_root();
    put(&root, "library/a.md", b"small\n");
    put(&root, "library/b.md", b"text\n");
    put(&root, "library/c.md", b"keep\n");
    assert_eq!(sync(&root).0, Outcome::LocalOnly { committed: true });
    let head = rev(&root, "HEAD").unwrap();

    put(&root, "library/a.md", &text_of(MAX_BYTES as usize + 1));
    put(&root, "library/b.md", b"now\0binary");
    put(&root, "library/c.md", b"keep, edited\n");
    let (outcome, alerts) = sync(&root);
    assert_eq!(outcome, Outcome::LocalOnly { committed: true });
    let new_head = rev(&root, "HEAD").unwrap();
    assert_ne!(new_head, head);
    let show = |p: &str| {
        git_ok(&root, &["show", &format!("HEAD:{p}")], false)
            .unwrap()
            .text()
    };
    assert_eq!(show("library/a.md"), "small");
    assert_eq!(show("library/b.md"), "text");
    assert_eq!(show("library/c.md"), "keep, edited");
    // Both are named once, and the root reads as needing a look.
    assert_eq!(alerts.len(), 1);
    assert!(
        alerts[0].body.contains("library/a.md (larger than 1 MB)"),
        "{}",
        alerts[0].body
    );
    assert!(
        alerts[0].body.contains("library/b.md (not a text file)"),
        "{}",
        alerts[0].body
    );
    assert_eq!(status_of(&root).state, SyncState::Blocked);
    // The grown file was never hashed: no object in the git directory is
    // over the ceiling.
    let objects = git_ok(
        &root,
        &[
            "cat-file",
            "--batch-all-objects",
            "--batch-check=%(objectsize)",
        ],
        false,
    )
    .unwrap()
    .text();
    assert!(
        objects
            .lines()
            .all(|n| n.parse::<u64>().unwrap() <= MAX_BYTES),
        "{objects}"
    );
}

#[test]
fn case_variants_of_the_personal_folders_stay_out() {
    let lab = Lab::new();
    let root = lab.vault_root();
    put(&root, "library/a.md", b"a\n");
    put(&root, "Me/Identity.md", b"me\n");
    put(&root, "MEMORY/m.md", b"m\n");
    put(&root, "Home/mac.md", b"h\n");
    assert_eq!(sync(&root).0, Outcome::LocalOnly { committed: true });
    assert_eq!(head_files(&root), ["library/a.md"]);
}

#[test]
fn symlinks_and_nested_repositories_stay_out() {
    let lab = Lab::new();
    let root = lab.vault_root();
    put(&root, "library/a.md", b"a\n");
    put(&root, "outside.md", b"outside\n");
    std::os::unix::fs::symlink(
        root.work_tree.join("outside.md"),
        root.work_tree.join("library/link.md"),
    )
    .unwrap();
    let nested = root.work_tree.join("workspaces/w/repo");
    fs::create_dir_all(&nested).unwrap();
    Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&nested)
        .output()
        .unwrap();
    put(&root, "workspaces/w/repo/README.md", b"nested\n");

    let (outcome, alerts) = sync(&root);
    assert_eq!(outcome, Outcome::LocalOnly { committed: true });
    assert_eq!(head_files(&root), ["library/a.md", "outside.md"]);
    assert_eq!(alerts.len(), 1);
    assert!(
        alerts[0].body.contains("library/link.md (a symlink)"),
        "{}",
        alerts[0].body
    );
}

#[test]
fn a_missing_folder_or_mass_removal_is_never_committed() {
    let lab = Lab::new();
    let root = lab.vault_root();
    for i in 0..4 {
        put(&root, &format!("library/n{i}.md"), b"n\n");
    }
    put(&root, "workspaces/w/WORKSPACE.md", b"w\n");
    assert_eq!(sync(&root).0, Outcome::LocalOnly { committed: true });
    let head = rev(&root, "HEAD").unwrap();

    let moved = lab.base.join("library-away");
    fs::rename(root.work_tree.join("library"), &moved).unwrap();
    match sync(&root).0 {
        Outcome::Blocked { file, rules } => {
            assert_eq!(file, "library/");
            assert!(rules[0].contains("missing"), "{rules:?}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(rev(&root, "HEAD").unwrap(), head);

    fs::rename(&moved, root.work_tree.join("library")).unwrap();
    for i in 0..3 {
        fs::remove_file(root.work_tree.join(format!("library/n{i}.md"))).unwrap();
    }
    assert!(matches!(sync(&root).0, Outcome::Blocked { .. }));
    assert_eq!(rev(&root, "HEAD").unwrap(), head);

    // A folder deleted on purpose: the person's allow_deletions lets that
    // one cycle past both guards, and the next cycle is clean.
    fs::remove_dir_all(root.work_tree.join("library")).unwrap();
    assert!(matches!(sync(&root).0, Outcome::Blocked { .. }));
    assert_eq!(
        sync_with(&root, true).0,
        Outcome::LocalOnly { committed: true }
    );
    assert_eq!(head_files(&root), ["workspaces/w/WORKSPACE.md"]);
    assert_eq!(sync(&root).0, Outcome::LocalOnly { committed: false });

    // A folder the manifest leaves out is never guarded.
    let tracked = [
        "tasks/x.md".to_string(),
        "Me/x.md".into(),
        "library/a.md".into(),
    ];
    assert_eq!(vault::tracked_folders(&tracked), ["library"]);
}

#[test]
fn the_vault_root_takes_no_remote_and_is_set_up_explicitly() {
    let lab = Lab::new();
    let sync_dir = lab.base.join("app").join(SYNC_DIR);
    let config = RootConfig {
        id: VAULT.into(),
        kind: RootKind::Vault,
        remote: Some("git@github.com:owner/vault.git".into()),
        branch: "main".into(),
        enabled: true,
    };
    assert!(Root::from_config(&config, &sync_dir, &lab.base).is_err());

    // Settings: the personal root stays as it was; the vault root is saved
    // as kind vault with no remote, and off turns it off.
    let settings = lab.base.join("app").join(SETTINGS_FILE);
    update_root_at(&settings, PERSONAL, |r| {
        r.remote = Some("git@github.com:owner/personal.git".into());
        r.enabled = true;
        Ok(())
    })
    .unwrap();
    assert!(config_at(&settings).root(VAULT).is_none());
    let saved = configure_vault_at(&settings, true).unwrap();
    assert_eq!(saved.kind, RootKind::Vault);
    assert!(saved.enabled && saved.remote.is_none());
    let read = config_at(&settings);
    assert_eq!(read.roots.len(), 2);
    assert!(read.root(PERSONAL).unwrap().remote.is_some());
    assert!(!configure_vault_at(&settings, false).unwrap().enabled);

    // A remote written into the file by hand is refused, not used.
    update_root_at(&settings, VAULT, |r| {
        r.remote = Some("git@github.com:owner/vault.git".into());
        Ok(())
    })
    .unwrap();
    assert!(configure_vault_at(&settings, true).is_err());
}

#[test]
fn path_rules_follow_the_manifest_and_the_cx1_rules() {
    for ok in [
        "AGENTS.md",
        "library/work/brand.md",
        "workspaces/w/outputs/report.md",
        "workspaces/w/canvas.json",
        "workflows/daily.md",
        "workspaces/w/.env.example",
    ] {
        assert!(vault::path_allowed(ok), "{ok}");
    }
    for out in [
        "me/Identity.md",
        "memory/x.md",
        "learnings/x.md",
        "home/mac.md",
        "Me/Identity.md",
        "LEARNINGS/x.md",
        "tasks/x.md",
        "Tasks/x.md",
        ".hollow-history/runs/r.json",
        ".trash/x.md",
        ".notifications.jsonl",
        "workspaces/w/log.jsonl",
        "workspaces/w/node_modules/a.js",
        "workspaces/w/outputs/a.PNG",
        "workspaces/w/outputs/a.pdf",
        "workspaces/w/outputs/a.sqlite",
        "workspaces/w/server.pem",
        "workspaces/w/.env",
        "workspaces/w/.DS_Store",
        "workspaces/w/.git/config",
        "workspaces/w/../x.md",
    ] {
        assert!(!vault::path_allowed(out), "{out}");
    }
    // The generated ignore lists CX1's data types, read from its rule.
    let data = scan_rules::data_type_extensions();
    assert!(
        data.iter().any(|e| e == "csv") && data.iter().any(|e| e == "pdf"),
        "{data:?}"
    );
    let text = vault::exclude_text(&MANIFEST);
    for e in &data {
        assert!(text.contains(&format!("\n*.{e}\n")), "{e}");
    }
    for d in PERSONAL_ALLOW {
        assert!(text.contains(&format!("\n/{d}/\n")), "{d}");
    }
    assert!(text.contains("\n/tasks/\n") && text.contains("\n/.*\n"));
}
