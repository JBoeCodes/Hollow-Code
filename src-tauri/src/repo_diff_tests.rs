//! Tests for `repo_diff` (Paperclip PR 16). Every test drives real `git` in
//! temp directories with isolated config (`fixture_git`); no provider turn.
//! Kept beside the module so its own source lint (nothing is ever staged)
//! can name the banned commands.

use super::*;
use crate::automated_history::{self, HistoryRun};
use crate::evidence::{EvidenceCollector, ObservedChange};
use std::fs;

fn fixture_git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new(&*GIT_BINARY)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn init_repo(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    fixture_git(dir, &["init", "-q", "--template="]);
}

fn commit_all(dir: &Path, message: &str) {
    fixture_git(dir, &["add", "-A"]);
    fixture_git(dir, &["commit", "-q", "-m", message]);
}

fn put(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn vault_dir() -> tempfile::TempDir {
    tempfile::Builder::new().prefix("vault-").tempdir().unwrap()
}

fn work_dir(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new().prefix(prefix).tempdir().unwrap()
}

fn spawns() -> usize {
    GIT_SPAWNS.with(|count| count.get())
}

async fn select(vault: &Path, cwd: &Path) -> Vec<Target> {
    select_targets(
        vault,
        cwd,
        Instant::now() + Duration::from_secs(60),
        None,
        &LIMITS,
    )
    .await
}

async fn armed(vault: &Path, cwd: &Path, limits: Limits) -> RepoBaseline {
    let mut baseline = begin_with(vault, cwd, None, limits).await;
    baseline.arm();
    baseline
}

fn observed_changes(run: &HistoryRun) -> Vec<ObservedChange> {
    run.changes
        .iter()
        .map(|change| ObservedChange {
            path: change.vault_path.clone(),
            change_type: change.change_type.clone(),
            ..Default::default()
        })
        .collect()
}

fn row<'a>(repo: &'a RepoChanges, path: &str) -> &'a RepoFileChange {
    repo.files
        .iter()
        .find(|file| file.path == path)
        .unwrap_or_else(|| panic!("no row for {path}: {:?}", repo.files))
}

fn sidecar_path(vault: &Path, run_id: &str) -> PathBuf {
    vault.join(format!(".hollow-history/runs/{run_id}.repo.json"))
}

/// Acceptance: a vault that is itself a git repository never gets a repo
/// diff for a vault-cwd run (and no git runs at all), while vault history
/// still records the note.
#[tokio::test]
async fn vault_git_repo_run_records_no_repo_diff() {
    let dir = vault_dir();
    let v = dir.path();
    put(v, "notes/a.md", "one\n");
    init_repo(v);
    commit_all(v, "init");

    let handle = automated_history::try_begin_run(v, "chat", None).unwrap();
    let before = spawns();
    let baseline = begin(v, v, None).await;
    assert!(baseline.is_empty());
    assert_eq!(spawns(), before, "no git runs for a vault-root cwd");

    let mut collector = EvidenceCollector::new("chat", None, &handle.id);
    collector.set_repo_baseline(baseline);
    collector.arm_repo_baseline();
    put(v, "notes/a.md", "two\n");
    let repo = finish_for_run(v, &handle.id, collector.take_repo_baseline()).await;
    assert!(repo.is_empty());
    collector.set_repo_changes(repo);
    let run = automated_history::finish_run(v, handle, "success").unwrap();
    assert!(!sidecar_path(v, &run.id).exists());
    let receipt = collector
        .finish(v, "success", observed_changes(&run))
        .unwrap();
    assert!(
        receipt.files.iter().any(|f| f.path == "notes/a.md"),
        "{:?}",
        receipt.files
    );
    assert!(receipt.repo_changes.is_empty());
}

#[tokio::test]
async fn vault_repo_and_ancestors_are_never_targets() {
    let dir = vault_dir();
    let v = dir.path();
    put(v, "workspaces/x/WORKSPACE.md", "x\n");
    init_repo(v);
    commit_all(v, "init");
    assert!(select(v, &v.join("workspaces/x")).await.is_empty());

    let outer = work_dir("outer-");
    let o = outer.path();
    put(o, "Vault/n.md", "n\n");
    put(o, "sibling/s.md", "s\n");
    init_repo(o);
    commit_all(o, "init");
    let vault = o.join("Vault");
    assert!(select(&vault, &vault).await.is_empty());
    assert!(select(&vault, &o.join("sibling")).await.is_empty());
}

/// A repository nested in the vault is a target; what vault history already
/// recorded is dropped from its rows, so only history-invisible changes
/// (dot-paths here) remain.
#[tokio::test]
async fn nested_vault_repo_reports_only_history_invisible_changes() {
    let dir = vault_dir();
    let v = dir.path();
    let app = v.join("code/app");
    put(&app, "notes.md", "a\n");
    put(&app, ".gitignore", "target\n");
    init_repo(&app);
    commit_all(&app, "init");

    let handle = automated_history::try_begin_run(v, "chat", None).unwrap();
    let mut collector = EvidenceCollector::new("chat", None, &handle.id);
    collector.set_repo_baseline(begin(v, &app, None).await);
    collector.arm_repo_baseline();
    put(&app, "notes.md", "b\n");
    put(&app, ".gitignore", "target\nnode_modules\n");
    let repo = finish_for_run(v, &handle.id, collector.take_repo_baseline()).await;
    assert_eq!(repo.len(), 1);
    assert_eq!(repo[0].vault_prefix.as_deref(), Some("code/app"));
    assert_eq!(repo[0].total_files, 2);
    collector.set_repo_changes(repo);
    let run = automated_history::finish_run(v, handle, "success").unwrap();
    let receipt = collector
        .finish(v, "success", observed_changes(&run))
        .unwrap();
    assert!(
        receipt.files.iter().any(|f| f.path == "code/app/notes.md"),
        "{:?}",
        receipt.files
    );
    let repo = &receipt.repo_changes[0];
    let paths: Vec<&str> = repo.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, vec![".gitignore"]);
    assert_eq!(repo.total_files, 1);
    assert_ne!(receipt.verification_state, "noChanges");
}

/// Acceptance: a linked-project run records a tracked modify, an untracked
/// create, a delete and a commit, stores the sidecar with patches, and its
/// receipt is never `noChanges`.
#[tokio::test]
async fn linked_project_run_records_changes_and_stores_sidecar() {
    let tmp = work_dir("hollow-");
    let projects = tmp.path().join("Projects");
    let vault = tmp.path().join("Vault");
    fs::create_dir_all(&projects).unwrap();
    fs::create_dir_all(&vault).unwrap();
    let projects = projects.canonicalize().unwrap();
    let vault = vault.canonicalize().unwrap();
    let app = projects.join("app");
    put(&app, "README.md", "hello\n");
    put(&app, "old.txt", "bye\n");
    put(&app, "src/lib.rs", "fn a() {}\n");
    init_repo(&app);
    commit_all(&app, "init");
    let connection = crate::directory_connections::connect(
        projects.to_str().unwrap(),
        vault.to_str().unwrap(),
        &tmp.path().join("config.json"),
    )
    .unwrap();
    let project = connection
        .projects
        .iter()
        .find(|p| p.path.ends_with("app"))
        .expect("the app project");
    let cwd = vault.join("workspaces").join(&project.id);
    assert!(cwd.is_dir());

    let handle = automated_history::try_begin_run(&vault, "chat", None).unwrap();
    let baseline = begin(&vault, &cwd, None).await;
    assert_eq!(baseline.targets.len(), 1);
    assert_eq!(baseline.targets[0].kind, "linkedProject");
    let mut collector = EvidenceCollector::new("chat", None, &handle.id);
    collector.set_repo_baseline(baseline);
    collector.arm_repo_baseline();

    put(&app, "README.md", "hello\nworld\n");
    put(&app, "new.txt", "fresh\n");
    fs::remove_file(app.join("old.txt")).unwrap();
    put(&app, "src/lib.rs", "fn b() {}\n");
    fixture_git(&app, &["add", "src/lib.rs"]);
    fixture_git(&app, &["commit", "-q", "-m", "b"]);

    let repo = finish_for_run(&vault, &handle.id, collector.take_repo_baseline()).await;
    assert_eq!(repo.len(), 1);
    let r = &repo[0];
    assert_eq!(r.target, "linkedProject");
    assert_eq!(r.status, "captured", "{:?}", r.error);
    assert_ne!(r.head_before, r.head_after);
    assert_eq!(r.total_files, 4);
    assert_eq!(row(r, "README.md").change_type, "modify");
    assert_eq!(row(r, "README.md").status, " M");
    assert_eq!(row(r, "new.txt").change_type, "create");
    assert!(row(r, "new.txt").untracked);
    assert_eq!(row(r, "old.txt").change_type, "delete");
    assert_eq!(row(r, "src/lib.rs").change_type, "modify");
    assert_eq!(
        row(r, "src/lib.rs").status,
        "M",
        "committed and clean: the raw letter"
    );
    assert!(
        r.files.iter().all(|f| f.patch.is_none()),
        "summaries carry no patch"
    );
    assert_eq!(row(r, "README.md").additions, Some(1));

    let sidecar = read_sidecar(&vault, &handle.id)
        .unwrap()
        .expect("a sidecar");
    let s = &sidecar.repos[0];
    assert!(row(s, "README.md")
        .patch
        .as_deref()
        .unwrap()
        .contains("+world"));
    assert!(row(s, "new.txt")
        .patch
        .as_deref()
        .unwrap()
        .contains("+fresh"));
    assert!(row(s, "old.txt").patch.as_deref().unwrap().contains("-bye"));
    assert!(row(s, "src/lib.rs")
        .patch
        .as_deref()
        .unwrap()
        .contains("+fn b() {}"));

    collector.set_repo_changes(repo);
    let run = automated_history::finish_run(&vault, handle, "success").unwrap();
    let receipt = collector
        .finish(&vault, "success", observed_changes(&run))
        .unwrap();
    assert_eq!(receipt.repo_changes.len(), 1);
    assert_eq!(receipt.verification_state, "unknownOrder");
    assert!(
        read_sidecar(&vault, &run.id).unwrap().is_some(),
        "kept after finish"
    );
}

fn write_manifest(vault: &Path, projects: &[(&str, PathBuf)]) {
    let projects: Vec<serde_json::Value> = projects
        .iter()
        .map(|(id, path)| serde_json::json!({ "id": id, "path": path.to_string_lossy(), "title": "p" }))
        .collect();
    let manifest = crate::directory_connections::manifest_path(vault);
    fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    fs::write(
        manifest,
        serde_json::json!({ "version": 1, "projects": projects }).to_string(),
    )
    .unwrap();
}

#[tokio::test]
async fn non_repo_project_outside_cwd_and_symlinked_cwd() {
    let dir = vault_dir();
    let v = dir.path();
    let plain_id = format!("local-{}", "a".repeat(24));
    let linked_id = format!("local-{}", "b".repeat(24));

    // A plain folder under a repo-backed Projects root is not a target.
    let outer = work_dir("projects-");
    let o = outer.path();
    put(o, "plain/a.txt", "a\n");
    init_repo(o);
    commit_all(o, "init");
    let workspace = v.join("workspaces").join(&plain_id);
    fs::create_dir_all(&workspace).unwrap();
    write_manifest(v, &[(plain_id.as_str(), o.join("plain"))]);
    assert!(select(v, &workspace).await.is_empty());

    // A repository outside the vault is a `cwd` target with its prefix.
    let other_dir = work_dir("repo-");
    let other = other_dir.path().join("app");
    put(&other, "src/lib.rs", "fn a() {}\n");
    init_repo(&other);
    commit_all(&other, "init");
    let targets = select(v, &other.join("src")).await;
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].kind, "cwd");
    assert_eq!(targets[0].cwd_in_repo.as_deref(), Some("src/"));

    // A workspace folder symlinked into its linked project: one target.
    #[cfg(unix)]
    {
        write_manifest(
            v,
            &[
                (plain_id.as_str(), o.join("plain")),
                (linked_id.as_str(), other.canonicalize().unwrap()),
            ],
        );
        let linked = v.join("workspaces").join(&linked_id);
        std::os::unix::fs::symlink(&other, &linked).unwrap();
        let targets = select(v, &linked).await;
        assert_eq!(targets.len(), 1, "deduplicated by canonical root");

        // A cwd given through a symlink keeps that spelling as an alias.
        let link = other_dir.path().join("link");
        std::os::unix::fs::symlink(&other, &link).unwrap();
        let targets = select(v, &link.join("src")).await;
        assert_eq!(targets.len(), 1);
        assert!(
            targets[0].alias_roots.contains(&lossy(&link)),
            "{:?}",
            targets[0].alias_roots
        );
        assert!(targets[0]
            .alias_roots
            .contains(&lossy(&other.canonicalize().unwrap())));
    }
}

#[tokio::test]
async fn preexisting_dirty_files_are_excluded_unless_changed() {
    let dir = vault_dir();
    let work = work_dir("repo-");
    let r = work.path().join("r");
    put(&r, "a.txt", "a\n");
    put(&r, "b.txt", "b\n");
    put(&r, "c.txt", "c\n");
    init_repo(&r);
    commit_all(&r, "init");
    put(&r, "a.txt", "a dirty\n");
    put(&r, "b.txt", "b dirty\n");
    put(&r, "c.txt", "c dirty\n");

    let baseline = armed(dir.path(), &r, LIMITS).await;
    put(&r, "b.txt", "b dirty and more\n");
    put(&r, "c.txt", "c\n");
    let repo = capture_after(baseline).await.remove(0);
    assert_eq!(repo.status, "captured", "{:?}", repo.error);
    assert!(
        repo.files.iter().all(|f| f.path != "a.txt"),
        "untouched dirty file: {:?}",
        repo.files
    );
    assert!(row(&repo, "b.txt").dirty_before);
    assert_eq!(row(&repo, "b.txt").status, " M");
    assert_eq!(row(&repo, "c.txt").status, "clean");
    assert!(row(&repo, "c.txt").dirty_before);
}

fn lines(n: usize, text: &str) -> String {
    (0..n).map(|i| format!("{text} {i}\n")).collect()
}

#[tokio::test]
async fn caps_bound_files_patches_and_total() {
    let dir = vault_dir();
    let v = dir.path();

    // More changed paths than listed rows.
    let work = work_dir("repo-");
    let r = work.path().join("many");
    put(&r, "seed.txt", "seed\n");
    init_repo(&r);
    commit_all(&r, "init");
    let baseline = armed(
        v,
        &r,
        Limits {
            max_files: 3,
            ..LIMITS
        },
    )
    .await;
    for n in 0..5 {
        put(&r, &format!("n{n}.txt"), "x\n");
    }
    let repo = capture_after(baseline).await.remove(0);
    assert_eq!(repo.files.len(), 3);
    assert_eq!(repo.total_files, 5);
    assert!(repo.truncated);

    // An oversized file gets no patch.
    let r = work.path().join("big");
    put(&r, "big.txt", "small\n");
    init_repo(&r);
    commit_all(&r, "init");
    let baseline = armed(
        v,
        &r,
        Limits {
            max_file_bytes: 64,
            ..LIMITS
        },
    )
    .await;
    put(&r, "big.txt", &lines(20, "large line"));
    let repo = capture_after(baseline).await.remove(0);
    assert!(row(&repo, "big.txt").oversized);
    assert!(row(&repo, "big.txt").patch.is_none());

    // A capped patch stream: the partial section is kept, marked, and the
    // repository is still captured with no error; a later file gets none.
    let r = work.path().join("stream");
    put(&r, "a.txt", "a\n");
    put(&r, "b.txt", "b\n");
    init_repo(&r);
    commit_all(&r, "init");
    let small = Limits {
        max_total_patch_bytes: 1000,
        stream_slack: 0,
        max_patch_bytes: 1000,
        ..LIMITS
    };
    let baseline = armed(v, &r, small).await;
    put(&r, "a.txt", &lines(200, "a changed"));
    put(&r, "b.txt", "b changed\n");
    let repo = capture_after(baseline).await.remove(0);
    assert_eq!(repo.status, "captured");
    assert!(repo.error.is_none());
    let a = row(&repo, "a.txt");
    assert!(a.patch.is_some());
    assert!(a.patch_truncated);
    assert!(a.patch.as_deref().unwrap().len() <= 1000);
    assert!(row(&repo, "b.txt").patch.is_none());
    assert!(repo.truncated);

    // The budget runs out after status: every row stays, without patches.
    let r = work.path().join("late");
    put(&r, "a.txt", "a\n");
    init_repo(&r);
    commit_all(&r, "init");
    let baseline = armed(v, &r, LIMITS).await;
    put(&r, "a.txt", "a changed\n");
    put(&r, "new.txt", "new\n");
    EXPIRE_BEFORE_PATCHES.with(|flag| flag.set(true));
    let repo = capture_after(baseline).await.remove(0);
    EXPIRE_BEFORE_PATCHES.with(|flag| flag.set(false));
    assert_eq!(repo.status, "captured");
    assert_eq!(repo.files.len(), 2);
    assert!(repo.files.iter().all(|f| f.patch.is_none()));
    assert!(repo.truncated);

    // Multibyte text: every cut lands on a character boundary.
    let r = work.path().join("utf8");
    put(&r, "é.txt", "é\n");
    init_repo(&r);
    commit_all(&r, "init");
    let baseline = armed(
        v,
        &r,
        Limits {
            max_patch_bytes: 101,
            ..LIMITS
        },
    )
    .await;
    put(&r, "é.txt", &"é".repeat(300));
    let repo = capture_after(baseline).await.remove(0);
    let patch = row(&repo, "é.txt").patch.clone().expect("a patch");
    assert!(patch.len() <= 101);
    assert!(row(&repo, "é.txt").patch_truncated);
    assert_eq!(truncate_char_boundary("ééé", 3), "é");
    assert_eq!(cap_patch("é".repeat(10), 5), ("éé".to_string(), true));
    assert_eq!(cap_error(&"é".repeat(400)).chars().count(), MAX_ERROR_CHARS);
}

#[test]
fn patch_stream_split_unquotes_paths() {
    let stream = concat!(
        "diff --git a/my file.txt b/my file.txt\n",
        "index 1..2 100644\n",
        "--- a/my file.txt\n",
        "+++ b/my file.txt\n",
        "@@ -1 +1 @@\n",
        "-a\n",
        "+b\n",
        "diff --git \"a/sp ace\\tx\" \"b/sp ace\\tx\"\n",
        "@@ -1 +1 @@\n",
        "+tab\n",
        "diff --git \"a/caf\\303\\251.txt\" \"b/caf\\303\\251.txt\"\n",
        "@@ -1 +1 @@\n",
        "+diff --git inside a line is not a header\n",
        "+",
    );
    let sections = split_patch_stream(stream.as_bytes(), 1 << 20);
    let paths: Vec<&str> = sections.iter().map(|(path, _, _)| path.as_str()).collect();
    assert_eq!(paths, vec!["my file.txt", "sp ace\tx", "café.txt"]);
    assert!(sections[0].1.starts_with("diff --git a/my file.txt"));
    assert!(sections[0].1.ends_with("+b\n"));
    assert!(sections.iter().all(|(_, _, capped)| !capped));

    let capped = split_patch_stream(stream.as_bytes(), 20);
    assert!(capped.iter().all(|(_, text, _)| text.len() <= 20));
    assert!(capped[0].2);
    assert_eq!(parse_diff_header("diff --git a/x b/y"), None);
    assert_eq!(
        parse_diff_header("diff --git a/a b/c b/a b/c"),
        Some("a b/c".into())
    );
}

fn sha256_of(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}

#[tokio::test]
async fn capture_never_rewrites_the_index() {
    let dir = vault_dir();
    let work = work_dir("repo-");
    let r = work.path().join("r");
    put(&r, "a.txt", "a\n");
    put(&r, "b.txt", "b\n");
    init_repo(&r);
    commit_all(&r, "init");
    // Stat-dirty, content-clean: a refreshing status would rewrite the index.
    std::thread::sleep(Duration::from_millis(20));
    put(&r, "a.txt", "a\n");
    let index = r.join(".git/index");
    let before = sha256_of(&index);
    let baseline = armed(dir.path(), &r, LIMITS).await;
    put(&r, "b.txt", "b changed\n");
    let repo = capture_after(baseline).await.remove(0);
    assert_eq!(repo.status, "captured", "{:?}", repo.error);
    assert_eq!(sha256_of(&index), before, "the index was rewritten");

    // Two files dirty at the pre-run snapshot and reverted by the run without
    // git (an Edit tool or an IDE): both become `clean` tracked rows, so the
    // batched `git diff <base> -- c.txt d.txt` sees two stat-dirty,
    // content-clean paths, which makes git refresh (and rewrite) the index
    // unless diff.autoRefreshIndex is off. Optional-locks don't cover it.
    put(&r, "c.txt", "c\n");
    put(&r, "d.txt", "d\n");
    commit_all(&r, "more");
    std::thread::sleep(Duration::from_millis(1100));
    put(&r, "c.txt", "c dirty\n");
    put(&r, "d.txt", "d dirty\n");
    let before = sha256_of(&index);
    let baseline = armed(dir.path(), &r, LIMITS).await;
    std::thread::sleep(Duration::from_millis(1100));
    put(&r, "c.txt", "c\n");
    put(&r, "d.txt", "d\n");
    let repo = capture_after(baseline).await.remove(0);
    assert_eq!(repo.status, "captured", "{:?}", repo.error);
    for path in ["c.txt", "d.txt"] {
        let reverted = row(&repo, path);
        assert_eq!(reverted.status, "clean", "{reverted:?}");
        assert!(!reverted.untracked && reverted.dirty_before, "{reverted:?}");
    }
    assert_eq!(sha256_of(&index), before, "git diff refreshed the index");

    // A held index.lock does not fail the capture.
    fs::write(r.join(".git/index.lock"), "").unwrap();
    let baseline = armed(dir.path(), &r, LIMITS).await;
    put(&r, "b.txt", "b changed again\n");
    let repo = capture_after(baseline).await.remove(0);
    assert_eq!(repo.status, "captured", "{:?}", repo.error);
    assert!(r.join(".git/index.lock").exists());

    let cmd = git_command(&r, Some(&r.join(".git")), &["evil".to_string()]);
    let std_cmd = cmd.as_std();
    let args: Vec<String> = std_cmd
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert_eq!(args[0], "--no-optional-locks");
    assert!(args.contains(&"--no-lazy-fetch".to_string()));
    let envs: Vec<(String, Option<String>)> = std_cmd
        .get_envs()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.map(|value| value.to_string_lossy().into_owned()),
            )
        })
        .collect();
    assert!(envs.contains(&("GIT_OPTIONAL_LOCKS".into(), Some("0".into()))));
    assert!(envs.contains(&("GIT_NO_LAZY_FETCH".into(), Some("1".into()))));
    let ours = |key: &str| {
        matches!(
            key,
            "GIT_OPTIONAL_LOCKS"
                | "GIT_TERMINAL_PROMPT"
                | "GIT_LITERAL_PATHSPECS"
                | "GIT_NO_LAZY_FETCH"
                | "GIT_PAGER"
                | "GIT_CONFIG_COUNT"
                | "GIT_CONFIG_NOSYSTEM"
                | "GIT_CONFIG_GLOBAL"
        ) || key.starts_with("GIT_CONFIG_KEY_")
            || key.starts_with("GIT_CONFIG_VALUE_")
    };
    for (key, value) in &envs {
        if key.starts_with("GIT_") && value.is_some() {
            assert!(ours(key), "unexpected {key}");
        }
    }
    for (key, _) in std::env::vars_os() {
        let key = key.to_string_lossy().into_owned();
        if key.starts_with("GIT_") && !ours(&key) {
            assert!(envs.contains(&(key.clone(), None)), "inherited {key}");
        }
    }
    assert!(
        envs.iter()
            .any(|(key, value)| key.starts_with("GIT_CONFIG_KEY_")
                && value.as_deref() == Some("filter.evil.clean")),
        "the filter driver is disabled at command scope"
    );
}

/// A repository with `a.txt` committed, and the marker-writing script.
fn hostile_repo(root: &Path, name: &str) -> PathBuf {
    let repo = root.join(name);
    put(&repo, "a.txt", "original\n");
    init_repo(&repo);
    commit_all(&repo, "init");
    repo
}

fn evil_script(root: &Path, marker: &Path) -> PathBuf {
    let script = root.join("evil.sh");
    fs::write(
        &script,
        format!("#!/bin/sh\n: > '{}'\ncat\n", marker.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    }
    script
}

async fn capture_modified(vault: &Path, repo: &Path, marker: &Path) -> RepoChanges {
    let _ = fs::remove_file(marker);
    let baseline = armed(vault, repo, LIMITS).await;
    put(repo, "a.txt", "changed by the run\n");
    let captured = capture_after(baseline).await.remove(0);
    assert!(
        !marker.exists(),
        "a repo-planted program ran in {}",
        repo.display()
    );
    captured
}

#[cfg(unix)]
#[tokio::test]
async fn hostile_repo_config_is_never_executed() {
    let dir = vault_dir();
    let v = dir.path();
    let work = work_dir("hostile-");
    let w = work.path();
    let marker = w.join("marker");
    let script = evil_script(w, &marker);
    let script = script.to_str().unwrap();

    // fsmonitor.
    let repo = hostile_repo(w, "fsmonitor");
    fixture_git(&repo, &["config", "core.fsmonitor", script]);
    let captured = capture_modified(v, &repo, &marker).await;
    assert_eq!(captured.status, "captured", "{:?}", captured.error);
    assert!(row(&captured, "a.txt").patch.is_some());

    // Clean/process/smudge filters.
    let repo = hostile_repo(w, "filter");
    put(&repo, ".gitattributes", "* filter=evil\n");
    commit_all(&repo, "attributes");
    for key in [
        "filter.evil.clean",
        "filter.evil.process",
        "filter.evil.smudge",
    ] {
        fixture_git(&repo, &["config", key, script]);
    }
    let captured = capture_modified(v, &repo, &marker).await;
    assert!(row(&captured, "a.txt").patch.is_some());

    // textconv and external diff drivers.
    let repo = hostile_repo(w, "textconv");
    put(&repo, ".gitattributes", "* diff=evil\n");
    commit_all(&repo, "attributes");
    fixture_git(&repo, &["config", "diff.evil.textconv", script]);
    fixture_git(&repo, &["config", "diff.evil.command", script]);
    capture_modified(v, &repo, &marker).await;

    // A driver name containing `=` (what `-c` would mis-split).
    let repo = hostile_repo(w, "equals");
    put(&repo, ".gitattributes", "* filter=x=y\n");
    commit_all(&repo, "attributes");
    let config = repo.join(".git/config");
    let mut text = fs::read_to_string(&config).unwrap();
    text.push_str(&format!(
        "[filter \"x=y\"]\n\tclean = {script}\n\tprocess = {script}\n"
    ));
    fs::write(&config, text).unwrap();
    capture_modified(v, &repo, &marker).await;

    // A partial clone whose missing blob would be fetched from an `ext::`
    // remote: lazy fetch is refused, so the row comes back without a patch.
    let repo = hostile_repo(w, "promisor");
    let blob = fixture_git(&repo, &["rev-parse", "HEAD:a.txt"])
        .trim()
        .to_string();
    for (key, value) in [
        ("core.repositoryformatversion", "1".to_string()),
        ("extensions.partialClone", "origin".to_string()),
        (
            "remote.origin.url",
            format!("ext::sh -c touch% {}", marker.display()),
        ),
        ("remote.origin.promisor", "true".to_string()),
        ("protocol.ext.allow", "always".to_string()),
    ] {
        fixture_git(&repo, &["config", key, &value]);
    }
    fs::remove_file(repo.join(".git/objects").join(&blob[..2]).join(&blob[2..])).unwrap();
    let captured = capture_modified(v, &repo, &marker).await;
    if captured.status == "captured" {
        assert!(row(&captured, "a.txt").patch.is_none());
    }

    // A submodule whose own config defines a filter, with the superproject
    // asking for recursive submodule diffs and the run committing a gitlink
    // move: the gitlink row is listed without a patch.
    let sub_src = w.join("subsrc");
    put(&sub_src, "s.txt", "s\n");
    init_repo(&sub_src);
    commit_all(&sub_src, "init");
    let sup = hostile_repo(w, "super");
    fixture_git(
        &sup,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            sub_src.to_str().unwrap(),
            "sub",
        ],
    );
    commit_all(&sup, "add sub");
    let sub = sup.join("sub");
    put(&sub, "s.txt", "s v2\n");
    commit_all(&sub, "v2");
    fixture_git(&sup, &["config", "diff.submodule", "diff"]);
    let _ = fs::remove_file(&marker);
    let baseline = armed(v, &sup, LIMITS).await;
    fixture_git(&sup, &["add", "sub"]);
    fixture_git(&sup, &["commit", "-q", "-m", "move sub"]);
    put(&sub, ".gitattributes", "* filter=evil\n");
    fixture_git(&sub, &["config", "filter.evil.clean", script]);
    put(&sub, "s.txt", "s v2\n");
    let _ = fs::remove_file(&marker);
    let captured = capture_after(baseline).await.remove(0);
    assert!(!marker.exists(), "a submodule's filter ran");
    let gitlink = row(&captured, "sub");
    assert!(gitlink.gitlink);
    assert!(gitlink.patch.is_none());

    // A post-run `core.worktree` edit: the pinned work tree is still read.
    let repo = hostile_repo(w, "worktree");
    let _ = fs::remove_file(&marker);
    let baseline = armed(v, &repo, LIMITS).await;
    fixture_git(
        &repo,
        &[
            "config",
            "core.worktree",
            w.join("elsewhere").to_str().unwrap(),
        ],
    );
    put(&repo, "a.txt", "changed\n");
    let captured = capture_after(baseline).await.remove(0);
    assert_eq!(captured.status, "captured", "{:?}", captured.error);
    assert!(row(&captured, "a.txt")
        .patch
        .as_deref()
        .unwrap()
        .contains("+changed"));
}

#[cfg(unix)]
#[tokio::test]
async fn special_files_symlinks_and_secrets_are_never_read() {
    let dir = vault_dir();
    let v = dir.path();
    let work = work_dir("repo-");
    let outside = work.path().join("outside.txt");
    fs::write(&outside, "TOPSECRET-OUTSIDE").unwrap();
    let r = work.path().join("r");
    put(&r, "f.txt", "fifo soon\n");
    put(&r, "g.txt", "plain\n");
    init_repo(&r);
    commit_all(&r, "init");
    // The capture still honours the user's XDG excludes file
    // (`~/.config/git/ignore`, read even with GIT_CONFIG_GLOBAL=/dev/null),
    // which on some machines ignores `.env.local`. `info/exclude` outranks it,
    // so this negation keeps the fixture independent of the host.
    put(&r, ".git/info/exclude", "!.env.local\n!.envrc\n!.kube/\n");
    let listed_before = fixture_git(&r, &["ls-files", "--stage"]);

    let handle = automated_history::try_begin_run(v, "chat", None).unwrap();
    let mut baseline = begin(v, &r, None).await;
    baseline.arm();

    fs::remove_file(r.join("f.txt")).unwrap();
    let fifo = std::ffi::CString::new(r.join("f.txt").to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o644) }, 0);
    std::os::unix::fs::symlink(&outside, r.join("link")).unwrap();
    put(&r, "credentials.json", "{\"key\":\"CREDSECRET\"}\n");
    put(&r, "id_rsa", "KEYSECRET\n");
    put(&r, ".env.local", "TOKEN=ENVSECRET\n");
    // Dotfile secret stores redact has no pattern for (direnv exports, a
    // kubeconfig under `.kube/`).
    put(&r, ".envrc", "export AWS_SECRET_ACCESS_KEY=ENVRCSECRET\n");
    put(&r, ".kube/config", "token: KUBESECRET\n");
    let key = format!("sk-ant-{}", "A".repeat(32));
    // PR 17's patterns (GitHub token, DSN password) reach the sidecar too.
    let gh = format!("ghp_{}", "Z".repeat(36));
    put(
        &r,
        "g.txt",
        &format!(
            "plain\nkey: {key}\ngh: {gh}\ndb: postgres://admin:DSNSECRET@db.example.com/app\n"
        ),
    );

    let started = Instant::now();
    let repo = finish_for_run(v, &handle.id, Some(baseline)).await;
    assert!(
        started.elapsed() < LIMITS.post_budget,
        "{:?}",
        started.elapsed()
    );
    let r0 = &repo[0];
    assert_eq!(r0.status, "captured", "{:?}", r0.error);
    assert_eq!(row(r0, "f.txt").status, "special");
    for secret in [
        "credentials.json",
        "id_rsa",
        ".env.local",
        ".envrc",
        ".kube/config",
    ] {
        assert!(row(r0, secret).withheld, "{secret}");
    }

    let bytes = automated_history::read_repo_sidecar_bytes(v, &handle.id)
        .unwrap()
        .unwrap();
    let text = String::from_utf8(bytes).unwrap();
    for leaked in [
        "CREDSECRET",
        "KEYSECRET",
        "ENVSECRET",
        "ENVRCSECRET",
        "KUBESECRET",
        "TOPSECRET-OUTSIDE",
        "DSNSECRET",
        key.as_str(),
        gh.as_str(),
    ] {
        assert!(!text.contains(leaked), "{leaked} reached the sidecar");
    }
    let sidecar = read_sidecar(v, &handle.id).unwrap().unwrap();
    let link = row(&sidecar.repos[0], "link");
    assert!(
        link.patch.as_deref().unwrap().contains("outside.txt"),
        "{:?}",
        link.patch
    );
    assert!(row(&sidecar.repos[0], "g.txt")
        .patch
        .as_deref()
        .unwrap()
        .contains("[api-key]"));

    automated_history::finish_run(v, handle, "success").unwrap();
    assert_eq!(
        fixture_git(&r, &["ls-files", "--stage"]),
        listed_before,
        "the index changed"
    );
    let _ = fs::remove_file(r.join("f.txt"));

    let source = include_str!("repo_diff.rs");
    for banned in [
        concat!("add ", "-N"),
        concat!("intent", "-to-add"),
        concat!("update", "-index"),
    ] {
        assert!(
            !source.contains(banned),
            "repo_diff.rs must never stage: {banned}"
        );
    }
}

#[test]
fn parsers_read_z_records() {
    let status = parse_status_z(b" M a.txt\0?? new file.txt\0D  gone.txt\0");
    assert_eq!(
        status,
        vec![
            (" M".to_string(), "a.txt".to_string()),
            ("??".to_string(), "new file.txt".to_string()),
            ("D ".to_string(), "gone.txt".to_string()),
        ]
    );
    let raw = parse_raw_z(b":100644 100644 aaaa bbbb M\0a.txt\0:000000 160000 0000 cccc A\0sub\0");
    assert_eq!(raw["a.txt"].status, "M");
    assert!(raw["sub"].gitlink());
    let numstat = parse_numstat_z(b"3\t1\ta.txt\0-\t-\timg.png\0");
    assert_eq!(numstat["a.txt"], (Some(3), Some(1), false));
    assert_eq!(numstat["img.png"], (None, None, true));
    assert_eq!(
        parse_filter_names(b"filter.lfs.clean\0filter.x=y.process\0filter.a.b.smudge\0"),
        vec!["a.b", "lfs", "x=y"]
    );
}
