//! Move Hollow home: pure-function pins and real-folder fixture tests.
//!
//! Every fixture lives in its own `tempfile::TempDir` (canonical path), with
//! a fake `$HOME`, fake app support and app data folders, a vault in a
//! `Documents/projects` folder (R2) and a Projects folder whose name holds a
//! space and a curly apostrophe (R1), `~/Hollow/Vault` and
//! `~/Hollow/Projects` as symlinks, a Sessions database with Hollow's real
//! schema in WAL mode, a git repository with linked worktrees, Claude, Codex
//! and launchd files. Each test asserts its roots are under its temp folder
//! before doing anything. Only the iCloud flag reader and the system probes
//! are stubbed ([`Fake`]); `git` runs for real.

use crate::move_home::{self, MoveKind, Options, Plan, Probes, Roots, RunHooks, SF_DATALESS};
use crate::move_home_plan as mp;
use crate::move_home_rekey as rekey;
use crate::move_home_run as mr;
use rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

// ── Pure functions ────────────────────────────────────────────────────────

#[test]
fn move_home_stem_and_id_pins() {
    assert_eq!(
        rekey::stem_for(Path::new("/Users/dev/Documents/projects/hollow-vault")),
        "hollow-vault-07f17664ad977f68"
    );
    assert_eq!(
        rekey::stem_for(Path::new("/Users/dev/Hollow/Vault")),
        "Vault-8362d56ddf96b4b1"
    );
    assert_eq!(
        rekey::workspace_id(Path::new(
            "/Users/dev/Documents/Documents - Alex\u{2019}s MacBook Pro/projects/3dprint"
        )),
        "local-9bc7a1b9279e2a7ca4b60d3d"
    );
    assert!(rekey::is_local_id("local-9bc7a1b9279e2a7ca4b60d3d"));
    assert!(!rekey::is_local_id("local-96f37e0d827164a95c07c42"));
    assert!(!rekey::is_local_id("local-96F37E0D827164A95C07C42A"));
}

#[test]
fn move_home_stem_matches_session_store_on_a_real_folder() {
    let tmp = tempfile::TempDir::new().unwrap();
    let real = std::fs::canonicalize(tmp.path()).unwrap();
    let vault = real.join("My Vault");
    std::fs::create_dir(&vault).unwrap();
    assert_eq!(
        crate::session_store::vault_file_stem(&vault),
        rekey::stem_for(&vault)
    );
    assert!(rekey::stem_for(&vault).starts_with("My-Vault-"));
    // The id formula is directory_connections' (sha256 of the path string).
    assert_eq!(rekey::workspace_id(&vault).len(), 30);
}

#[test]
fn move_home_claude_dir_encoding() {
    assert_eq!(
        rekey::claude_dir_name(
            "/Users/dev/Documents/Documents - Alex\u{2019}s MacBook Pro/projects"
        ),
        "-Users-dev-Documents-Documents---Alex-s-MacBook-Pro-projects"
    );
    assert_eq!(
        rekey::claude_dir_name("/Users/dev/Documents/projects/hollow-vault/workspaces/quick-chats"),
        "-Users-dev-Documents-projects-hollow-vault-workspaces-quick-chats"
    );
    assert_eq!(
        rekey::claude_dir_name("/a/bench.nosync/x_y"),
        "-a-bench-nosync-x-y"
    );
    // An astral character is two UTF-16 units, as Claude's replace sees it.
    assert_eq!(rekey::claude_dir_name("/a/\u{1F600}"), "-a---");
}

fn maps(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    rekey::sorted_mappings(
        &pairs
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect::<Vec<_>>(),
    )
}

#[test]
fn move_home_rewrite_paths_is_segment_aware_longest_first_single_pass() {
    let m = maps(&[
        ("/h/Docs/projects/hollow-vault", "/h/Hollow/Vault"),
        ("/h/R1", "/h/Hollow/Projects"),
        ("/h/R1/personal", "/h/Hollow/Projects/Personal"),
    ]);
    let r = |t: &str| rekey::rewrite_paths(t, &m).0;
    assert_eq!(r("/h/Docs/projects/hollow-vault"), "/h/Hollow/Vault");
    assert_eq!(
        r("/h/Docs/projects/hollow-vault/workspaces/x"),
        "/h/Hollow/Vault/workspaces/x"
    );
    assert_eq!(
        r("/h/Docs/projects/hollow-vault2/x"),
        "/h/Docs/projects/hollow-vault2/x"
    );
    assert_eq!(
        r("/h/Docs/projects/hollow-vault.bak"),
        "/h/Docs/projects/hollow-vault.bak"
    );
    assert_eq!(
        r("/h/Docs/projects/hollow-vault-old"),
        "/h/Docs/projects/hollow-vault-old"
    );
    assert_eq!(
        r("/x/h/Docs/projects/hollow-vault"),
        "/x/h/Docs/projects/hollow-vault"
    );
    assert_eq!(
        r("Work at /h/R1/3dprint. Original files stay."),
        "Work at /h/Hollow/Projects/3dprint. Original files stay."
    );
    assert_eq!(
        r("cwd: /h/R1/personal/runescape"),
        "cwd: /h/Hollow/Projects/Personal/runescape"
    );
    assert_eq!(
        r("{\"p\":\"/h/R1\",\"q\":\"/h/R1/a\"}"),
        "{\"p\":\"/h/Hollow/Projects\",\"q\":\"/h/Hollow/Projects/a\"}"
    );
    assert_eq!(
        r("`/h/R1`, (/h/R1)"),
        "`/h/Hollow/Projects`, (/h/Hollow/Projects)"
    );
    assert_eq!(r("/h/R1x /h/R12"), "/h/R1x /h/R12");
    // Output is never rescanned: a chain a->b, b->c rewrites a to b only, and
    // a mapping into its own subfolder doesn't grow.
    let chain = maps(&[("/a", "/b"), ("/b", "/c")]);
    assert_eq!(rekey::rewrite_paths("/a, /b", &chain).0, "/b, /c");
    let grow = maps(&[("/a", "/a/b")]);
    assert_eq!(rekey::rewrite_paths("/a/x", &grow).0, "/a/b/x");
    assert_eq!(rekey::rewrite_paths("/a/x", &grow).1, 1);
    // Unicode survives, and the curly apostrophe path matches exactly.
    let curly = maps(&[("/h/Docs - J\u{2019}s/projects", "/h/Hollow/Projects")]);
    assert_eq!(
        rekey::rewrite_paths("caf\u{e9} /h/Docs - J\u{2019}s/projects/a", &curly).0,
        "caf\u{e9} /h/Hollow/Projects/a"
    );
}

#[test]
fn move_home_rewrite_ids_exact_tokens_only() {
    let ids: BTreeMap<String, String> = [(
        "local-9bc7a1b9279e2a7ca4b60d3d".to_string(),
        "local-000000000000000000000001".to_string(),
    )]
    .into_iter()
    .collect();
    let (t, n) = rekey::rewrite_ids(
        "a local-96f37e0d827164a95c07c42a b /w/local-96f37e0d827164a95c07c42a/x xlocal-96f37e0d827164a95c07c42a local-96f37e0d827164a95c07c42ab",
        &ids,
    );
    assert_eq!(n, 2);
    assert_eq!(
        t,
        "a local-000000000000000000000001 b /w/local-000000000000000000000001/x xlocal-96f37e0d827164a95c07c42a local-96f37e0d827164a95c07c42ab"
    );
}

#[test]
fn move_home_json_rewrite_keeps_every_untouched_byte() {
    let text = "{\n  \"vault\" :\"/h/v\",\n\t\"list\": [\"/h/v/a\", 1, \"\\u00e9/h/v\"],\n  \"/h/v\": {\"k\": \"keep\"}\n}\n";
    let m = maps(&[("/h/v", "/h/Hollow/Vault")]);
    let (out, n) = rekey::rewrite_json_paths(text, &m).unwrap();
    assert_eq!(n, 3);
    assert_eq!(
        out,
        "{\n  \"vault\" :\"/h/Hollow/Vault\",\n\t\"list\": [\"/h/Hollow/Vault/a\", 1, \"\\u00e9/h/v\"],\n  \"/h/Hollow/Vault\": {\"k\": \"keep\"}\n}\n"
    );
    // A store: a value equal to the old stem becomes the new one; a longer
    // string containing it does not.
    let store = "{\"vaults\":{\"old-0123456789abcdef\":{\"repo\":\"/h/v/r\"}},\"x\":\"old-0123456789abcdef-suffix\"}";
    let (out, _) = rekey::rewrite_store_json(
        store,
        "old-0123456789abcdef",
        "Vault-fedcba9876543210",
        &m,
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(
        out,
        "{\"vaults\":{\"Vault-fedcba9876543210\":{\"repo\":\"/h/Hollow/Vault/r\"}},\"x\":\"old-0123456789abcdef-suffix\"}"
    );
}

#[test]
fn move_home_claude_json_and_codex_rewrite_keys_only() {
    let m = maps(&[("/h/R1", "/h/Hollow/Projects")]);
    let text = r#"{"projects":{"/h/R1/a":{"allowedTools":[],"note":"/h/R1/a"},"/h/R1/b":{},"/h/Hollow/Projects/b":{"kept":true}},"other":"/h/R1/a"}"#;
    let (out, n, taken) = rekey::rewrite_claude_json(text, &m).unwrap();
    assert_eq!(n, 1);
    assert_eq!(taken, vec!["/h/R1/b".to_string()]);
    assert_eq!(
        out,
        r#"{"projects":{"/h/Hollow/Projects/a":{"allowedTools":[],"note":"/h/R1/a"},"/h/R1/b":{},"/h/Hollow/Projects/b":{"kept":true}},"other":"/h/R1/a"}"#
    );
    let toml = "model = \"x\"\nsource = \"/h/R1/a\"\n[projects.\"/h/R1/a\"]\ntrust_level = \"trusted\"\n\n  [projects.\"/h/R1/b\"]\n[projects.\"/h/Hollow/Projects/b\"]\n";
    let (out, n, taken) = rekey::rewrite_codex_headers(toml, &m);
    assert_eq!(n, 1);
    assert_eq!(taken, vec!["/h/R1/b".to_string()]);
    assert_eq!(
        out,
        "model = \"x\"\nsource = \"/h/R1/a\"\n[projects.\"/h/Hollow/Projects/a\"]\ntrust_level = \"trusted\"\n\n  [projects.\"/h/R1/b\"]\n[projects.\"/h/Hollow/Projects/b\"]\n"
    );
    assert!(toml::from_str::<toml::Value>(&out).is_ok());
}

#[test]
fn move_home_home_forms_and_moved_names() {
    let m = maps(&[("/u/me/Documents/projects/v", "/u/me/Hollow/Vault")]);
    let forms = rekey::home_forms(&m, Path::new("/u/me"));
    let (t, n) = rekey::rewrite_paths("DEST=\"$HOME/Documents/projects/v/out\" ~/Documents/projects/v/ ${HOME}/Documents/projects/v", &forms);
    assert_eq!(n, 3);
    assert_eq!(
        t,
        "DEST=\"$HOME/Hollow/Vault/out\" ~/Hollow/Vault/ ${HOME}/Hollow/Vault"
    );
    assert_eq!(rekey::moved_name("MEMORY.md", 1), "MEMORY.moved-1.md");
    assert_eq!(rekey::moved_name("README", 2), "README.moved-2");
    assert_eq!(rekey::moved_name(".env", 1), ".env.moved-1");
}

#[test]
fn move_home_real_flags_reader_on_a_normal_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let f = tmp.path().join("plain.txt");
    std::fs::write(&f, "x").unwrap();
    let meta = std::fs::symlink_metadata(&f).unwrap();
    let flags = move_home::SystemProbes.flags(&f, &meta);
    assert_eq!(flags & SF_DATALESS, 0);
}

// ── Fixture ───────────────────────────────────────────────────────────────

#[derive(Default)]
struct Fake {
    dataless: BTreeSet<PathBuf>,
    /// Folders that count as inside a File Provider (iCloud) domain.
    fp: Vec<PathBuf>,
    calls: Mutex<Vec<String>>,
    launchd_fails: bool,
    agent_clis: Vec<String>,
}

impl Probes for Fake {
    fn flags(&self, path: &Path, _meta: &std::fs::Metadata) -> u32 {
        if self.dataless.contains(path) {
            SF_DATALESS
        } else {
            0
        }
    }
    fn processes(&self, _roots: &[PathBuf]) -> Vec<String> {
        Vec::new()
    }
    fn file_provider_domain(&self, path: &Path) -> Option<String> {
        self.fp
            .iter()
            .any(|r| path.starts_with(r))
            .then(|| "com.apple.CloudDocs.iCloudDriveFileProvider/test".to_string())
    }
    fn time_machine(&self, _path: &Path) -> move_home::BackupStatus {
        move_home::BackupStatus {
            covered: Some(true),
            detail: "test".into(),
        }
    }
    fn launchd_probe(&self, dir: &Path, _scratch: &Path) -> Result<Option<String>, String> {
        if self.launchd_fails {
            return Err("Operation not permitted".into());
        }
        std::fs::read_dir(dir).map_err(|e| e.to_string())?;
        Ok(Some("listed".into()))
    }
    fn agent_clis(&self, _not_under: Option<u32>) -> Vec<String> {
        self.agent_clis.clone()
    }
    fn bootout(&self, label: &str) -> Result<(), String> {
        self.calls.lock().unwrap().push(format!("bootout {label}"));
        Ok(())
    }
    fn bootstrap(&self, plist: &Path) -> Result<(), String> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("bootstrap {}", plist.display()));
        Ok(())
    }
}

struct Fx {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    roots: Roots,
    r1: PathBuf,
    r2: PathBuf,
    vault: PathBuf,
    hollow: PathBuf,
    stem: String,
}

fn w(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn sym(target: &Path, link: &Path) {
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(target, link).unwrap();
}

fn git(dir: &Path, home: &Path, args: &[&str]) {
    let mut cmd = std::process::Command::new("git");
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(k);
        }
    }
    let out = cmd
        .env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
            "-C",
        ])
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn id(p: &Path) -> String {
    rekey::workspace_id(p)
}

/// Sessions DB with Hollow's real schema in WAL mode, left with frames in
/// its `-wal` (copied while a writer was open, as after a crash).
fn make_sessions_db(db: &Path, fill: impl FnOnce(&Connection)) {
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    let stage = db.parent().unwrap().join("stage.tmp");
    std::fs::create_dir_all(&stage).unwrap();
    let live = stage.join("s.db");
    let conn = Connection::open(&live).unwrap();
    conn.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(()))
        .unwrap();
    conn.execute_batch("PRAGMA wal_autocheckpoint=0;").unwrap();
    crate::session_store::apply_schema(&conn).unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS external_sessions(id TEXT PRIMARY KEY, cwd TEXT, path TEXT, transcript_path TEXT); \
         CREATE TABLE IF NOT EXISTS external_export_files(id INTEGER PRIMARY KEY, path TEXT); \
         CREATE TABLE IF NOT EXISTS work_origin_allocation_pins(id INTEGER PRIMARY KEY, source_root TEXT);",
    )
    .unwrap();
    fill(&conn);
    for suffix in ["", "-wal", "-shm"] {
        let from = mp::sidecar(&live, suffix);
        if from.exists() {
            std::fs::copy(&from, mp::sidecar(db, suffix)).unwrap();
        }
    }
    drop(conn);
    std::fs::remove_dir_all(&stage).unwrap();
    assert!(
        mp::sidecar(db, "-wal").metadata().unwrap().len() > 0,
        "the fixture keeps a WAL"
    );
}

impl Fx {
    fn new() -> Fx {
        Fx::build(false)
    }

    /// `vault_home`: the vault is already the real `<home>/Vault` (only the
    /// Projects folder moves, so the store stem stays).
    fn build(vault_home: bool) -> Fx {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let home = root.join("home");
        let r1 = home.join("Documents/Documents - Alex\u{2019}s MacBook Pro/projects");
        let r2 = home.join("Documents/projects");
        let vault = if vault_home {
            home.join("Hollow/Vault")
        } else {
            r2.join("hollow-vault")
        };
        let hollow = home.join("Hollow");
        let support = home.join("Library/Application Support/Hollow");
        let app_data = home.join("Library/Application Support/com.hollow.app");
        let config = home.join(".config/Hollow/config.json");
        for p in [
            &home, &r1, &r2, &vault, &hollow, &support, &app_data, &config,
        ] {
            assert!(
                p.starts_with(&root),
                "{} is outside the temp root",
                p.display()
            );
        }
        let stem = rekey::stem_for(&vault);

        // R1 projects.
        let p3d = r1.join("3dprint");
        w(&p3d.join("model.scad"), "cube();\n");
        w(&r1.join("personal/runescape/notes.md"), "gp\n");
        w(&r1.join("dynamic-pricing/old.txt"), "older copy\n");
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        std::fs::File::options()
            .write(true)
            .open(r1.join("dynamic-pricing/old.txt"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        // R1's stale vault copy (a Hollow vault: workspaces + AGENTS.md).
        w(&r1.join("hollow-vault/AGENTS.md"), "stale\n");
        w(
            &r1.join("hollow-vault/workspaces/main/WORKSPACE.md"),
            "stale\n",
        );
        // A git repo in R1 with a worktree outside it and one inside.
        let app = r1.join("app");
        w(&app.join("README.md"), "app\n");
        git(&app, &home, &["init", "-q", "-b", "main"]);
        git(&app, &home, &["add", "README.md"]);
        git(&app, &home, &["commit", "-q", "-m", "init"]);
        let wt1 = hollow.join("Worktrees/app/wt1");
        std::fs::create_dir_all(wt1.parent().unwrap()).unwrap();
        git(
            &app,
            &home,
            &["worktree", "add", "-q", "-b", "wt1", wt1.to_str().unwrap()],
        );
        let wt2 = app.join(".claude/worktrees/wt2");
        std::fs::create_dir_all(wt2.parent().unwrap()).unwrap();
        git(
            &app,
            &home,
            &["worktree", "add", "-q", "-b", "wt2", wt2.to_str().unwrap()],
        );

        // R2: the vault and folders beside it.
        let id3d = id(&p3d);
        let idapp = id(&app);
        w(&vault.join("AGENTS.md"), "vault\n");
        w(
            &vault.join("README.md"),
            &format!(
                "- Code lives in {} and {}.\n",
                r1.display(),
                hollow.join("Repositories").display()
            ),
        );
        w(
            &vault.join("workspaces").join(&id3d).join("WORKSPACE.md"),
            &format!(
                "# 3dprint\n\nWork with the existing project at {}. Original files stay there.\n",
                p3d.display()
            ),
        );
        w(
            &vault.join("workspaces").join(&idapp).join("WORKSPACE.md"),
            &format!(
                "# app\n\nWork with the existing project at {}. Original files stay there.\n",
                app.display()
            ),
        );
        w(
            &vault.join("workspaces/main/WORKSPACE.md"),
            &format!(
                "# Main\n\nSee [3dprint](../{id3d}/WORKSPACE.md) and {}.bak and {}2.\nAlso {} 2/notes and {} copy.\n",
                vault.display(),
                vault.display(),
                r1.display(),
                vault.display()
            ),
        );
        w(
            &vault.join("workspaces/local-0123456789abcdef01234567/WORKSPACE.md"),
            "orphan\n",
        );
        w(
            &vault.join("workflows/herald.md"),
            &format!(
                "---\ncwd: {}\n---\nRun in {}.\n",
                r2.join("morning-paper").display(),
                vault.join("workspaces/main").display()
            ),
        );
        w(&r2.join("morning-paper/press.py"), "print(1)\n");
        w(&r2.join("dynamic-pricing/poc/app.py"), "newer copy\n");
        w(&r2.join("portal-wip/x.txt"), "wip\n");

        // ~/Hollow: links to the two roots, real Repositories and Worktrees.
        if !vault_home {
            sym(&vault, &hollow.join("Vault"));
        }
        // A Finder-style copy beside the Projects folder: `projects 2`.
        let r1_copy = PathBuf::from(format!("{} 2", r1.display()));
        w(&r1_copy.join("notes/x.txt"), "copy\n");
        sym(&r1, &hollow.join("Projects"));
        std::fs::create_dir_all(hollow.join("Repositories/other")).unwrap();

        // Config (0600, with a secret-looking key that must survive).
        w(
            &config,
            &serde_json::to_string_pretty(&serde_json::json!({
                "vaultPath": vault, "projectsPath": r1, "hollowHome": hollow,
                "claudeProjectsPath": home.join(".claude/projects"), "notionToken": "keep-me"
            }))
            .unwrap(),
        );
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600)).unwrap();
        }

        // Stores keyed by the stem.
        let claude_proj = home.join(".claude/projects");
        let enc_3d = rekey::claude_dir_name(&p3d.to_string_lossy());
        let transcript = claude_proj.join(&enc_3d).join("s2.jsonl");
        let ws3d = vault.join("workspaces").join(&id3d);
        make_sessions_db(&support.join("Sessions").join(format!("{stem}.db")), |c| {
            c.execute("INSERT INTO sessions(key, agent, cwd, created_at, last_activity_at) VALUES ('k1','claude',?1,'t','t')", [ws3d.to_string_lossy()]).unwrap();
            c.execute("INSERT INTO sessions(key, agent, cwd, created_at, last_activity_at) VALUES ('k2','claude',?1,'t','t')", [wt1.to_string_lossy()]).unwrap();
            c.execute(
                "INSERT INTO turns(session_key, ord, started_at, asked) VALUES ('k1', 1, 't', ?1)",
                [format!("history mentions {}", p3d.display())],
            )
            .unwrap();
            c.execute("INSERT INTO external_sessions(id, cwd, path, transcript_path) VALUES ('e1', ?1, ?2, ?2)", [p3d.to_string_lossy(), transcript.to_string_lossy()]).unwrap();
            c.execute(
                "INSERT INTO external_export_files(path) VALUES (?1)",
                [r1.join("personal/runescape").to_string_lossy()],
            )
            .unwrap();
            c.execute(
                "INSERT INTO work_origin_allocation_pins(source_root) VALUES (?1)",
                [r1.to_string_lossy()],
            )
            .unwrap();
            c.execute(
                "INSERT INTO interactions(id, session_key, turn_ord, run_id, agent, tool_use_id, source, mode, request_json, target_path, target_abs, status, created_at) \
                 VALUES ('i1','k1',1,'r','claude','t','s','m',?1,?2,?2,'pending','t')",
                [format!("{{\"path\":\"{}\"}}", p3d.join("model.scad").display()), p3d.join("model.scad").to_string_lossy().into_owned()],
            )
            .unwrap();
            c.execute(
                "INSERT INTO run_queue(source, source_id, label, payload_json, status, priority, requested_at, detail) VALUES ('chat','k1','old',?1,'done',1,'t',?2)",
                [format!("{{\"cwd\":\"{}\",\"workspace\":\"{id3d}\"}}", ws3d.display()), format!("ran in {}", vault.display())],
            )
            .unwrap();
            c.execute("INSERT INTO derived_state(session_key, kind, revision, through_ord, payload, updated_at) VALUES ('k1','x',1,1,?1,'t')", [format!("{{\"cwd\":\"{}\"}}", ws3d.display())]).unwrap();
            c.execute("INSERT INTO work_links(session_key, workspace_id, linked_by, at) VALUES ('k1', ?1, 'me', 't')", [&id3d]).unwrap();
            c.execute(
                "INSERT INTO runbook_runs(source, name, workspace, day, steps_hash, status, mode, copy_path, started_at) VALUES ('s','n',?1,'d','h','done','m',?2,'t')",
                [id3d.clone(), vault.join("workspaces").join(&id3d).join("run.md").to_string_lossy().into_owned()],
            )
            .unwrap();
        });
        w(
            &support.join("Sessions").join(format!("{stem}.pump.lock")),
            "",
        );
        w(
            &support.join("Sessions/other-0123456789abcdef.db"),
            "another vault",
        );
        w(
            &support.join("MainAgent").join(format!("{stem}.json")),
            &serde_json::json!({"focus": id3d, "cwd": ws3d}).to_string(),
        );
        w(
            &support
                .join("MainAgent")
                .join(format!("{stem}-swarms.json")),
            &serde_json::json!({"dir": vault.join("workspaces/main")}).to_string(),
        );
        w(
            &support
                .join("WorkflowPermissions")
                .join(format!("{stem}.json")),
            &serde_json::to_string_pretty(
                &serde_json::json!({"vault": vault, "stamps": [{"cwd": r2.join("morning-paper")}]}),
            )
            .unwrap(),
        );
        w(
            &support.join("Directories").join(format!("{stem}.json")),
            &serde_json::json!({"version": 1, "projectsPath": r1, "vaultPath": vault, "projects": [
                {"id": id3d, "path": p3d, "title": "3dprint"},
                {"id": idapp, "path": app, "title": "app"},
                {"id": id(&r1.join("personal")), "path": r1.join("personal"), "title": "personal"},
            ]})
            .to_string(),
        );
        w(&support.join("Leases").join(format!("{stem}.lock")), "");
        w(&support.join("Bakeoffs").join(&stem).join("r.json"), "{}");
        w(
            &app_data.join("build-workspaces.json"),
            &serde_json::to_string_pretty(&serde_json::json!({"vaults": {stem.clone(): {"ws": {"repo": app, "worktree": wt1, "branch": "wt1"}}}})).unwrap(),
        );
        w(
            &app_data.join("workspace-runs-on.json"),
            &serde_json::json!({stem.clone(): {id3d.clone(): "mac-mini"}}).to_string(),
        );
        w(
            &app_data.join("curated-canvases.json"),
            &serde_json::json!({"canvases": [{"vault": vault}]}).to_string(),
        );
        w(
            &app_data.join("instance.json"),
            &serde_json::json!({"vaultPath": vault}).to_string(),
        );
        w(
            &app_data.join("memory-usage").join(format!("{stem}.json")),
            "{}",
        );
        w(
            &app_data
                .join("RemoteFolders")
                .join(format!("{stem}.git"))
                .join("HEAD"),
            "ref: refs/heads/main\n",
        );

        // Claude Code project folders.
        let enc_ws = rekey::claude_dir_name(&ws3d.to_string_lossy());
        w(
            &claude_proj.join(&enc_ws).join("s1.jsonl"),
            &format!("{{\"type\":\"user\",\"cwd\":\"{}\"}}\n", ws3d.display()),
        );
        w(
            &claude_proj.join(&enc_ws).join("memory/MEMORY.md"),
            "ws memory\n",
        );
        w(&transcript, &format!("{{\"cwd\":\"{}\"}}\n", p3d.display()));
        w(
            &claude_proj.join(&enc_3d).join("memory/MEMORY.md"),
            "old memory\n",
        );
        let enc_new_3d = rekey::claude_dir_name(&hollow.join("Projects/3dprint").to_string_lossy());
        w(
            &claude_proj.join(&enc_new_3d).join("memory/MEMORY.md"),
            "new memory\n",
        );
        w(
            &claude_proj
                .join(rekey::claude_dir_name(
                    &r1.join("personal/runescape").to_string_lossy(),
                ))
                .join("memory/MEMORY.md"),
            "rs\n",
        );
        w(
            &claude_proj.join("-elsewhere").join("x.jsonl"),
            "{\"cwd\":\"/elsewhere\"}\n",
        );
        // Memory for `projects 2`, no transcript: its name starts with the
        // Projects folder's encoding, but no folder under it fits.
        w(
            &claude_proj
                .join(rekey::claude_dir_name(&format!("{} 2", r1.display())))
                .join("memory/MEMORY.md"),
            "copy memory\n",
        );
        // A vault workspace's folder with memory but no transcript: its name
        // is exactly one moved folder's encoding.
        let ws_app = vault.join("workspaces").join(&idapp);
        w(
            &claude_proj
                .join(rekey::claude_dir_name(&ws_app.to_string_lossy()))
                .join("memory/MEMORY.md"),
            "app ws\n",
        );
        w(
            &home.join(".claude.json"),
            &serde_json::to_string_pretty(&serde_json::json!({"projects": {p3d.to_string_lossy(): {"trust": true}, vault.to_string_lossy(): {}, "/gone/dead": {}}, "note": p3d})).unwrap(),
        );
        w(
            &home.join(".codex/config.toml"),
            &format!("model = \"x\"\n[projects.\"{}\"]\ntrust_level = \"trusted\"\n[projects.\"{}\"]\ntrust_level = \"trusted\"\n", p3d.display(), vault.display()),
        );
        w(
            &home.join(".gemini/projects.json"),
            &serde_json::json!({"projects": {p3d.to_string_lossy(): "3dprint"}}).to_string(),
        );
        w(
            &home.join("Library/Application Support/obsidian/obsidian.json"),
            &serde_json::json!({"vaults": {"abc": {"path": vault, "open": true}}}).to_string(),
        );

        // launchd and scripts.
        w(
            &home.join("Library/LaunchAgents/com.test.portal.plist"),
            &format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n<dict>\n<key>Label</key><string>com.test.portal</string>\n<key>WorkingDirectory</key><string>{}</string>\n<key>StandardOutPath</key><string>{}</string>\n</dict>\n</plist>\n",
                r2.join("dynamic-pricing/poc").display(),
                vault.join("workspaces/main/portal.log").display()
            ),
        );
        w(
            &home.join(".local/bin/mini-pull"),
            "#!/bin/sh\nDEST=\"$HOME/Documents/projects/hollow-vault/workspaces/main/outputs\"\n",
        );
        w(
            &home.join(".local/bin/api-token.env"),
            "SECRET=never-read\n",
        );

        let roots = Roots::with(&home, &config, &support, &app_data, "Hollow").unwrap();
        for p in [
            &roots.home,
            &roots.config_path,
            &roots.support,
            &roots.app_data,
            &roots.vault,
            &roots.hollow_home_path(),
        ] {
            assert!(
                p.starts_with(&root),
                "{} is outside the temp root",
                p.display()
            );
        }
        assert!(roots.projects.as_ref().unwrap().starts_with(&root));
        Fx {
            _tmp: tmp,
            root,
            home,
            roots,
            r1,
            r2,
            vault,
            hollow,
            stem,
        }
    }

    fn plan(&self, options: &Options, probes: &dyn Probes) -> Plan {
        mp::plan_with(&self.roots, options, None, probes).unwrap()
    }

    fn backups(&self) -> PathBuf {
        self.hollow.join("Backups")
    }

    /// path -> (kind, mode, content hash or link target), without the
    /// backup folder and SQLite `-shm` files (an index any reader rewrites).
    fn tree(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        let skip = self.backups();
        let mut stack = vec![self.root.clone()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.starts_with(&skip) || p.to_string_lossy().ends_with("-shm") {
                    continue;
                }
                use sha2::Digest;
                use std::os::unix::fs::PermissionsExt;
                let m = std::fs::symlink_metadata(&p).unwrap();
                let mode = m.permissions().mode() & 0o7777;
                let rel = p
                    .strip_prefix(&self.root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                if m.file_type().is_symlink() {
                    out.insert(
                        rel,
                        format!("l {}", std::fs::read_link(&p).unwrap().display()),
                    );
                } else if m.is_dir() {
                    out.insert(rel, format!("d {mode:o}"));
                    stack.push(p);
                } else if let Some(kind) = crate::move_home_icloud::special_kind(&m.file_type()) {
                    // A socket or pipe can't be read: kind, mode and mtime.
                    use std::os::unix::fs::MetadataExt;
                    out.insert(
                        rel,
                        format!("s {kind} {mode:o} {}.{}", m.mtime(), m.mtime_nsec()),
                    );
                } else {
                    let h = sha2::Sha256::digest(std::fs::read(&p).unwrap());
                    out.insert(rel, format!("f {mode:o} {h:x}"));
                }
            }
        }
        out
    }

    fn stage(&self, options: &Options, probes: &dyn Probes) -> PathBuf {
        let plan = self.plan(options, probes);
        assert!(plan.ready, "plan not ready: {:#?}", plan.blockers);
        mr::stage(&self.roots, options, &plan.hash, None, probes)
            .unwrap()
            .plan_file
    }
}

fn diff(a: &BTreeMap<String, String>, b: &BTreeMap<String, String>) -> Vec<String> {
    let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    keys.into_iter()
        .filter(|k| a.get(*k) != b.get(*k))
        .map(|k| format!("{k}: {:?} -> {:?}", a.get(k), b.get(k)))
        .collect()
}

fn all_options() -> Options {
    Options {
        include_vault_siblings: true,
        personal: vec!["portal-wip".into()],
        launch_agents: vec!["com.test.portal".into()],
        scripts: vec!["mini-pull".into()],
        ..Default::default()
    }
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

// ── Fixture tests ─────────────────────────────────────────────────────────

#[test]
fn move_home_dry_run_reports_and_changes_nothing() {
    let fx = Fx::new();
    let before = fx.tree();
    let mut fake = Fake::default();
    fake.dataless.insert(fx.r1.join("3dprint/model.scad"));
    let plan = fx.plan(&Options::default(), &fake);
    assert_eq!(
        diff(&before, &fx.tree()),
        Vec::<String>::new(),
        "the dry run changed files"
    );
    let kinds: Vec<(MoveKind, String)> =
        plan.moves.iter().map(|m| (m.kind, m.to.clone())).collect();
    let h = fx.hollow.to_string_lossy().into_owned();
    assert_eq!(kinds[0], (MoveKind::Vault, format!("{h}/Vault")));
    assert_eq!(kinds[1], (MoveKind::ProjectsRoot, format!("{h}/Projects")));
    assert!(kinds.contains(&(MoveKind::Personal, format!("{h}/Projects/Personal"))));
    assert!(
        kinds.contains(&(MoveKind::Archive, format!("{h}/Archive/hollow-vault"))),
        "{kinds:?}"
    );
    assert_eq!(
        plan.moves[0].replaces_symlink.as_deref(),
        Some(fx.vault.to_str().unwrap())
    );
    // Folders beside the vault are listed, not moved, and the same-name pair
    // is reported with its winner (newer content beside the vault).
    assert_eq!(plan.beside_vault.len(), 3);
    assert!(!plan.moves.iter().any(|m| m
        .from
        .starts_with(&fx.r2.join("morning-paper").to_string_lossy().into_owned())));
    let same = plan
        .collisions
        .iter()
        .find(|c| c.kind == "sameName")
        .unwrap();
    assert_eq!(
        same.winner.as_deref(),
        Some(fx.r2.join("dynamic-pricing").to_str().unwrap())
    );
    // Stores, Sessions columns, ids, files, repos, agents, dataless.
    assert_eq!(plan.old_stem, fx.stem);
    assert_eq!(plan.new_stem, rekey::stem_for(&fx.hollow.join("Vault")));
    assert!(plan.stores.iter().any(|s| s.what == mp::WHAT_SESSIONS));
    assert!(plan
        .stores
        .iter()
        .any(|s| s.from.ends_with(&format!("{}.pump.lock", fx.stem))));
    assert!(plan
        .stores
        .iter()
        .any(|s| s.from.ends_with(&format!("RemoteFolders/{}.git", fx.stem))));
    assert!(!plan
        .stores
        .iter()
        .any(|s| s.from.contains("other-0123456789abcdef")));
    let rows = |t: &str, c: &str| {
        plan.columns
            .iter()
            .find(|x| x.table == t && x.column == c)
            .map(|x| x.rows)
    };
    assert_eq!(rows("sessions", "cwd"), Some(1));
    assert_eq!(rows("external_sessions", "transcript_path"), Some(1));
    assert_eq!(rows("work_links", "workspace_id"), Some(1));
    assert_eq!(rows("runbook_runs", "workspace"), Some(1));
    assert_eq!(plan.ids.len(), 3);
    assert_eq!(plan.ids.iter().filter(|i| i.has_workspace).count(), 2);
    assert_eq!(
        plan.orphans,
        vec!["local-0123456789abcdef01234567".to_string()]
    );
    let file = |what: &str| plan.files.iter().filter(|f| f.what == what).count();
    assert_eq!(file(mp::FILE_CONFIG), 1);
    assert!(file(mp::FILE_NOTE) >= 4, "{:#?}", plan.files);
    assert!(file(mp::FILE_STORE) >= 6, "{:#?}", plan.files);
    assert_eq!(file(mp::FILE_CLAUDE_JSON), 1);
    assert_eq!(file(mp::FILE_CODEX), 1);
    assert_eq!(file(mp::FILE_LAUNCH_AGENT), 0, "not on the plan");
    assert_eq!(plan.repos.len(), 1);
    assert_eq!(plan.repos[0].worktrees, 2);
    assert_eq!(plan.repos[0].broken, 0);
    assert_eq!(plan.dataless.total, 1);
    assert!(plan
        .agents
        .iter()
        .any(|a| a.agent == "Claude Code projects" && a.merges >= 1 && a.ambiguous.len() == 1));
    assert!(plan
        .launch_agents
        .iter()
        .any(|l| l.label == "com.test.portal" && !l.on_plan));
    assert!(plan.scripts.iter().any(|l| l.label == "mini-pull"));
    assert!(!plan.scripts.iter().any(|l| l.label.contains("token")));
    assert!(plan.ready, "{:#?}", plan.blockers);
    assert!(plan.same_volume);
    assert_eq!(plan.text, move_home::describe(&plan));
    assert!(plan.text.iter().any(|l| l.contains("Plan hash")));
    // Same decisions, same hash.
    assert_eq!(plan.hash, fx.plan(&Options::default(), &fake).hash);
    assert_ne!(plan.hash, fx.plan(&all_options(), &fake).hash);
}

#[test]
fn move_home_full_run_rekeys_everything_then_rolls_back_byte_for_byte() {
    let fx = Fx::new();
    let fake = Fake::default();
    let options = all_options();
    let plan_file = fx.stage(&options, &fake);
    let before = fx.tree();
    let out = move_home::run_with(&plan_file, &fake, None, &RunHooks::default());
    assert!(
        out.is_ok(),
        "{out:?}\n{}",
        read(&plan_file.with_file_name("journal.jsonl"))
    );
    let h = &fx.hollow;
    let p3d_new = h.join("Projects/3dprint");
    // Layout.
    assert!(h.join("Vault/AGENTS.md").is_file());
    assert_eq!(std::fs::read_link(&fx.vault).unwrap(), h.join("Vault"));
    assert_eq!(std::fs::read_link(&fx.r1).unwrap(), h.join("Projects"));
    assert!(!std::fs::symlink_metadata(h.join("Projects"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(h.join("Projects/Personal/runescape/notes.md").is_file());
    assert!(h.join("Projects/Personal/portal-wip/x.txt").is_file());
    assert!(h.join("Archive/hollow-vault/AGENTS.md").is_file());
    assert_eq!(
        read(&h.join("Projects/dynamic-pricing/poc/app.py")),
        "newer copy\n"
    );
    let archived: Vec<String> = std::fs::read_dir(h.join("Archive"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        archived.iter().any(|n| n.starts_with("dynamic-pricing")),
        "{archived:?}"
    );
    assert_eq!(
        std::fs::read_link(fx.r2.join("morning-paper")).unwrap(),
        h.join("Projects/morning-paper")
    );
    // Stores re-keyed; the original Sessions trio is in the backup folder.
    let new_stem = rekey::stem_for(&h.join("Vault"));
    let support = &fx.roots.support;
    let new_db = support.join("Sessions").join(format!("{new_stem}.db"));
    assert!(new_db.is_file());
    assert!(!support
        .join("Sessions")
        .join(format!("{}.db", fx.stem))
        .exists());
    assert!(plan_file
        .parent()
        .unwrap()
        .join("sessions-original")
        .join(format!("{}.db", fx.stem))
        .is_file());
    assert!(support
        .join("MainAgent")
        .join(format!("{new_stem}-swarms.json"))
        .is_file());
    assert!(support.join("Sessions/other-0123456789abcdef.db").is_file());
    let new3d = rekey::workspace_id(&p3d_new);
    let old3d = rekey::workspace_id(&fx.r1.join("3dprint"));
    let conn = mp::open_read_only(&new_db).unwrap();
    let cwd: String = conn
        .query_row("SELECT cwd FROM sessions WHERE key='k1'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        cwd,
        h.join("Vault/workspaces").join(&new3d).to_string_lossy()
    );
    let wt_cwd: String = conn
        .query_row("SELECT cwd FROM sessions WHERE key='k2'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        wt_cwd,
        h.join("Worktrees/app/wt1").to_string_lossy(),
        "worktree paths stay"
    );
    let asked: String = conn
        .query_row("SELECT asked FROM turns", [], |r| r.get(0))
        .unwrap();
    assert!(
        asked.contains(&*fx.r1.to_string_lossy()),
        "history text is left alone"
    );
    let wl: String = conn
        .query_row("SELECT workspace_id FROM work_links", [], |r| r.get(0))
        .unwrap();
    assert_eq!(wl, new3d);
    let payload: String = conn
        .query_row("SELECT payload_json FROM run_queue", [], |r| r.get(0))
        .unwrap();
    assert!(payload.contains(&new3d) && !payload.contains(&old3d));
    let tp: String = conn
        .query_row("SELECT transcript_path FROM external_sessions", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        tp,
        fx.home
            .join(".claude/projects")
            .join(rekey::claude_dir_name(&p3d_new.to_string_lossy()))
            .join("s2.jsonl")
            .to_string_lossy()
    );
    let pin: String = conn
        .query_row(
            "SELECT source_root FROM work_origin_allocation_pins",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pin, h.join("Projects").to_string_lossy());
    drop(conn);
    // JSON stores.
    let bw = read(&fx.roots.app_data.join("build-workspaces.json"));
    assert!(
        bw.contains(&new_stem)
            && !bw.contains(&fx.stem)
            && bw.contains(&*h.join("Projects/app").to_string_lossy())
    );
    let runs_on = read(&fx.roots.app_data.join("workspace-runs-on.json"));
    assert!(runs_on.contains(&new_stem) && runs_on.contains(&new3d));
    // Config: new paths, unknown keys kept, mode kept.
    let cfg: serde_json::Value = serde_json::from_str(&read(&fx.roots.config_path)).unwrap();
    assert_eq!(cfg["vaultPath"], h.join("Vault").to_string_lossy().as_ref());
    assert_eq!(
        cfg["projectsPath"],
        h.join("Projects").to_string_lossy().as_ref()
    );
    assert_eq!(cfg["notionToken"], "keep-me");
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&fx.roots.config_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    // Vault notes and the workspace folder rename.
    let ws = h.join("Vault/workspaces").join(&new3d);
    assert!(read(&ws.join("WORKSPACE.md"))
        .contains(&format!("project at {}. Original", p3d_new.display())));
    assert!(!h.join("Vault/workspaces").join(&old3d).exists());
    let main_ws = read(&h.join("Vault/workspaces/main/WORKSPACE.md"));
    assert!(main_ws.contains(&new3d), "{main_ws}");
    assert!(
        main_ws.contains(&format!("{}.bak", fx.vault.display()))
            && main_ws.contains(&format!("{}2", fx.vault.display())),
        "{main_ws}"
    );
    let herald = read(&h.join("Vault/workflows/herald.md"));
    assert!(
        herald.contains(&format!(
            "cwd: {}",
            h.join("Projects/morning-paper").display()
        )),
        "{herald}"
    );
    // Git: worktrees repaired both ways, nothing pruned.
    let wt1_git = read(&h.join("Worktrees/app/wt1/.git"));
    assert!(
        wt1_git.contains(&*h.join("Projects/app/.git/worktrees").to_string_lossy()),
        "{wt1_git}"
    );
    // Agents.
    let cp = fx.home.join(".claude/projects");
    let enc_new = cp.join(rekey::claude_dir_name(&p3d_new.to_string_lossy()));
    assert_eq!(read(&enc_new.join("memory/MEMORY.md")), "new memory\n");
    assert_eq!(
        read(&enc_new.join("memory/MEMORY.moved-1.md")),
        "old memory\n"
    );
    assert!(enc_new.join("s2.jsonl").is_file());
    let enc_ws = cp.join(rekey::claude_dir_name(&ws.to_string_lossy()));
    assert_eq!(read(&enc_ws.join("memory/MEMORY.md")), "ws memory\n");
    let ws_app = h
        .join("Vault/workspaces")
        .join(rekey::workspace_id(&h.join("Projects/app")));
    assert_eq!(
        read(
            &cp.join(rekey::claude_dir_name(&ws_app.to_string_lossy()))
                .join("memory/MEMORY.md")
        ),
        "app ws\n"
    );
    // No transcript: rebuilt from the folders on disk (one fits).
    assert_eq!(
        read(
            &cp.join(rekey::claude_dir_name(
                &h.join("Projects/Personal/runescape").to_string_lossy()
            ))
            .join("memory/MEMORY.md")
        ),
        "rs\n"
    );
    // `projects 2` is not under the Projects folder: left as it is.
    let copy_dir = cp.join(rekey::claude_dir_name(&format!("{} 2", fx.r1.display())));
    assert_eq!(read(&copy_dir.join("memory/MEMORY.md")), "copy memory\n");
    assert!(
        main_ws.contains(&format!(
            "Also {} 2/notes and {} copy.",
            fx.r1.display(),
            fx.vault.display()
        )),
        "{main_ws}"
    );
    let cj: serde_json::Value = serde_json::from_str(&read(&fx.home.join(".claude.json"))).unwrap();
    assert!(cj["projects"].get(&*p3d_new.to_string_lossy()).is_some());
    assert!(
        cj["projects"].get("/gone/dead").is_some(),
        "dead keys are kept"
    );
    assert_eq!(cj["note"], fx.r1.join("3dprint").to_string_lossy().as_ref());
    assert!(read(&fx.home.join(".codex/config.toml"))
        .contains(&format!("[projects.\"{}\"]", p3d_new.display())));
    // launchd and scripts on the plan.
    assert!(
        read(&fx.home.join("Library/LaunchAgents/com.test.portal.plist"))
            .contains(&*h.join("Projects/dynamic-pricing/poc").to_string_lossy())
    );
    assert!(read(&fx.home.join(".local/bin/mini-pull"))
        .contains("$HOME/Hollow/Vault/workspaces/main/outputs"));
    let calls = fake.calls.lock().unwrap().clone();
    assert_eq!(
        calls.first().map(String::as_str),
        Some("bootout com.test.portal")
    );
    assert!(calls.last().unwrap().starts_with("bootstrap "));
    // Status and verify.
    let st = move_home::status(&fx.roots_after());
    assert_eq!(st.state, "done", "{st:#?}");
    assert!(st.checks.iter().all(|c| c.ok), "{:#?}", st.checks);
    assert!(st.can_roll_back);
    // Idempotent: the same plan again is a no-op; a fresh plan has nothing to do.
    assert_eq!(
        move_home::run_with(&plan_file, &fake, None, &RunHooks::default()).unwrap(),
        "The move already finished."
    );
    let again = mp::plan_with(&fx.roots_after(), &Options::default(), None, &fake).unwrap();
    assert!(again.is_noop(), "{:#?}", again.text);
    assert!(mr::stage(
        &fx.roots_after(),
        &Options::default(),
        &again.hash,
        None,
        &fake
    )
    .is_err());
    // Roll back: the tree is byte for byte what it was.
    let rb = move_home::rollback_with(plan_file.parent().unwrap(), &fake, None);
    assert!(rb.is_ok(), "{rb:?}");
    assert_eq!(diff(&before, &fx.tree()), Vec::<String>::new());
    assert_eq!(move_home::status(&fx.roots).state, "rolledBack");
}

impl Fx {
    /// The roots Hollow reads after the move (config rewritten).
    fn roots_after(&self) -> Roots {
        Roots::with(
            &self.home,
            &self.roots.config_path,
            &self.roots.support,
            &self.roots.app_data,
            "Hollow",
        )
        .unwrap()
    }
}

#[test]
fn move_home_kill_at_every_step_then_rollback_restores_the_tree() {
    let fake = Fake::default();
    let count = {
        let fx = Fx::new();
        mr::op_count(&fx.plan(&all_options(), &fake))
    };
    assert!(count > 20, "{count} steps");
    for k in 0..=count {
        for crash_mid in [false, true] {
            if crash_mid && k == count {
                continue;
            }
            let fx = Fx::new();
            let plan_file = fx.stage(&all_options(), &fake);
            let names = mr::op_names(&read_plan(&plan_file));
            let before = fx.tree();
            let r = move_home::run_with(
                &plan_file,
                &fake,
                None,
                &RunHooks {
                    stop_after: Some(k),
                    crash_mid,
                    ..Default::default()
                },
            );
            if k < count {
                assert_eq!(r, Err("stopped (test)".into()), "k={k}");
            }
            let rb = move_home::rollback_with(plan_file.parent().unwrap(), &fake, None);
            assert!(
                rb.is_ok(),
                "k={k} ({}) crash_mid={crash_mid}: {rb:?}",
                names.get(k).cloned().unwrap_or_default()
            );
            let d = diff(&before, &fx.tree());
            assert!(
                d.is_empty(),
                "k={k} ({}) crash_mid={crash_mid}: {d:#?}",
                names.get(k).cloned().unwrap_or_default()
            );
        }
    }
}

fn read_plan(plan_file: &Path) -> Plan {
    serde_json::from_str(&read(plan_file)).unwrap()
}

#[test]
fn move_home_resume_after_a_kill_finishes_the_move() {
    let fx = Fx::new();
    let fake = Fake::default();
    let plan_file = fx.stage(&all_options(), &fake);
    let n = mr::op_count(&read_plan(&plan_file));
    let r = move_home::run_with(
        &plan_file,
        &fake,
        None,
        &RunHooks {
            stop_after: Some(n / 2),
            crash_mid: true,
            ..Default::default()
        },
    );
    assert!(r.is_err());
    assert_eq!(move_home::status(&fx.roots).state, "running");
    let r = move_home::run_with(&plan_file, &fake, None, &RunHooks::default());
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(move_home::status(&fx.roots_after()).state, "done");
}

#[test]
fn move_home_failing_step_rolls_back_by_itself() {
    let fx = Fx::new();
    let fake = Fake::default();
    let plan_file = fx.stage(&Options::default(), &fake);
    let before = fx.tree();
    // A store target appears between staging and the run.
    let plan = read_plan(&plan_file);
    let late = plan
        .stores
        .iter()
        .find(|s| s.what == mp::WHAT_STORE && s.to.ends_with(".lock"))
        .unwrap();
    w(Path::new(&late.to), "made meanwhile");
    let r = move_home::run_with(&plan_file, &fake, None, &RunHooks::default());
    assert!(
        r.as_ref().is_err_and(|e| e.contains("rolled back")),
        "{r:?}"
    );
    std::fs::remove_file(&late.to).unwrap();
    assert_eq!(diff(&before, &fx.tree()), Vec::<String>::new());
    assert_eq!(move_home::status(&fx.roots).state, "rolledBack");
}

#[test]
fn move_home_stem_collision_and_taken_target_block() {
    let fx = Fx::new();
    let fake = Fake::default();
    let new_stem = rekey::stem_for(&fx.hollow.join("Vault"));
    w(
        &fx.roots
            .support
            .join("MainAgent")
            .join(format!("{new_stem}.json")),
        "{}",
    );
    let plan = fx.plan(&Options::default(), &fake);
    assert!(!plan.ready);
    assert!(plan.collisions.iter().any(|c| c.kind == "stemCollision"));
    let e = mr::stage(&fx.roots, &Options::default(), &plan.hash, None, &fake)
        .err()
        .unwrap();
    assert!(e.contains("can't start"), "{e}");
    assert!(!fx.backups().exists(), "nothing staged");

    let fx = Fx::new();
    std::fs::remove_file(fx.hollow.join("Projects")).unwrap();
    w(&fx.hollow.join("Projects/someone-else/x"), "x");
    let plan = fx.plan(&Options::default(), &fake);
    assert!(!plan.ready);
    assert!(plan.collisions.iter().any(|c| c.kind == "targetExists"));
}

#[test]
fn move_home_running_chats_block_except_the_callers_and_hash_binds() {
    let fx = Fx::new();
    let fake = Fake::default();
    let db = mp::sessions_db(&fx.roots, &fx.stem);
    {
        let c = Connection::open(&db).unwrap();
        c.execute("INSERT INTO run_queue(source, source_id, label, status, priority, requested_at) VALUES ('chat','coo','My chat','running',1,'t')", []).unwrap();
    }
    let plan = fx.plan(&Options::default(), &fake);
    assert!(plan
        .blockers
        .iter()
        .any(|b| b.kind == "runningChat" && b.blocking));
    let mine = mp::plan_with(&fx.roots, &Options::default(), Some("coo"), &fake).unwrap();
    assert!(mine.ready, "{:#?}", mine.blockers);
    let e = mr::stage(
        &fx.roots,
        &Options::default(),
        "not-the-hash",
        Some("coo"),
        &fake,
    )
    .err()
    .unwrap();
    assert!(e.contains("changed since"), "{e}");
    assert!(mr::stage(
        &fx.roots,
        &Options::default(),
        &mine.hash,
        Some("coo"),
        &fake
    )
    .is_ok());
    assert_eq!(move_home::status(&fx.roots).state, "staged");
}

#[test]
fn move_home_rollback_refused_after_new_work() {
    let fx = Fx::new();
    let fake = Fake::default();
    let plan_file = fx.stage(&Options::default(), &fake);
    move_home::run_with(&plan_file, &fake, None, &RunHooks::default()).unwrap();
    let after = fx.roots_after();
    assert!(move_home::status(&after).can_roll_back);
    let new_db = mp::sessions_db(&after, &rekey::stem_for(&fx.hollow.join("Vault")));
    {
        let c = Connection::open(&new_db).unwrap();
        c.execute(
            "INSERT INTO turns(session_key, ord, started_at) VALUES ('k1', 2, 't')",
            [],
        )
        .unwrap();
    }
    let st = move_home::status(&after);
    assert!(!st.can_roll_back);
    let why = st.rollback_refusal.unwrap();
    assert!(
        why.contains("1 new turns") && why.contains("Copy what you need"),
        "{why}"
    );
    assert!(mr::check_rollback(&after, None).is_err());
    assert!(move_home::rollback_with(plan_file.parent().unwrap(), &fake, None).is_err());
    assert!(
        fx.hollow.join("Vault/AGENTS.md").is_file(),
        "nothing was rolled back"
    );
}

#[test]
fn move_home_dataless_files_are_moved_but_never_read() {
    let fx = Fx::new();
    let mut fake = Fake::default();
    let note = fx.vault.join("workflows/herald.md");
    fake.dataless.insert(note.clone());
    let before = read(&note);
    let plan = fx.plan(&Options::default(), &fake);
    assert!(!plan.files.iter().any(|f| f.path == note.to_string_lossy()));
    assert!(plan.dataless.total >= 1);
    assert!(plan
        .blockers
        .iter()
        .any(|b| b.kind == "dataless" && !b.blocking));
    let plan_file = mr::stage(&fx.roots, &Options::default(), &plan.hash, None, &fake)
        .unwrap()
        .plan_file;
    let mut fake2 = Fake::default();
    fake2.dataless.insert(note.clone());
    fake2
        .dataless
        .insert(fx.hollow.join("Vault/workflows/herald.md"));
    move_home::run_with(&plan_file, &fake2, None, &RunHooks::default()).unwrap();
    assert_eq!(
        read(&fx.hollow.join("Vault/workflows/herald.md")),
        before,
        "an evicted file is moved, never rewritten"
    );
}

#[test]
fn move_home_failed_launchd_probe_is_a_failed_check() {
    let fx = Fx::new();
    let fake = Fake {
        launchd_fails: true,
        ..Default::default()
    };
    let plan_file = fx.stage(&Options::default(), &fake);
    let r = move_home::run_with(&plan_file, &fake, None, &RunHooks::default());
    assert!(
        r.as_ref()
            .is_err_and(|e| e.contains("verify found problems")),
        "{r:?}"
    );
    let st = move_home::status(&fx.roots_after());
    assert_eq!(st.state, "failed");
    assert!(st.checks.iter().any(|c| c.name == "launchd" && !c.ok));
    assert!(st.can_roll_back, "a failed check offers rollback");
}

#[test]
fn move_home_fresh_install_has_nothing_to_move() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let home = root.join("u");
    let hollow = home.join("Hollow");
    for d in [
        "Vault/workspaces",
        "Projects",
        "Repositories",
        "Worktrees",
        "Archive",
    ] {
        std::fs::create_dir_all(hollow.join(d)).unwrap();
    }
    let config = home.join(".config/Hollow/config.json");
    w(&config, &serde_json::json!({"vaultPath": hollow.join("Vault"), "projectsPath": hollow.join("Projects"), "hollowHome": hollow}).to_string());
    let support = home.join("Library/Application Support/Hollow");
    let app_data = home.join("Library/Application Support/com.hollow.app");
    std::fs::create_dir_all(&support).unwrap();
    std::fs::create_dir_all(&app_data).unwrap();
    let roots = Roots::with(&home, &config, &support, &app_data, "Hollow").unwrap();
    for p in [&roots.home, &roots.vault, &roots.support, &roots.app_data] {
        assert!(p.starts_with(&root));
    }
    let plan = mp::plan_with(&roots, &Options::default(), None, &Fake::default()).unwrap();
    assert!(plan.is_noop(), "{:#?}", plan.text);
    assert!(plan.moves.is_empty() && plan.mkdirs.is_empty());
    assert!(plan.text.iter().any(|l| l.contains("nothing to do")));
    assert!(mr::stage(
        &roots,
        &Options::default(),
        &plan.hash,
        None,
        &Fake::default()
    )
    .is_err());
    assert_eq!(move_home::status(&roots).state, "none");
}

#[test]
fn move_home_outside_home_is_refused_unless_allowed() {
    let fx = Fx::new();
    let fake = Fake::default();
    let mut roots = fx.roots.clone();
    roots.hollow_home = Some(fx.root.join("elsewhere/Hollow"));
    std::fs::create_dir_all(fx.root.join("elsewhere")).unwrap();
    let plan = mp::plan_with(&roots, &Options::default(), None, &fake).unwrap();
    assert!(plan
        .blockers
        .iter()
        .any(|b| b.kind == "outsideHome" && b.blocking));
    let allowed = mp::plan_with(
        &roots,
        &Options {
            allow_outside_home: true,
            ..Default::default()
        },
        None,
        &fake,
    )
    .unwrap();
    assert!(!allowed.blockers.iter().any(|b| b.kind == "outsideHome"));
}

#[test]
fn move_home_nothing_from_the_developers_home_leaks() {
    let fx = Fx::new();
    let user_home = ["/Users/", "dev"].concat();
    if fx.root.starts_with(&user_home) {
        return; // A TMPDIR under the real home can't prove anything here.
    }
    let fake = Fake::default();
    let plan = fx.plan(&all_options(), &fake);
    let json = serde_json::to_string(&plan).unwrap();
    assert!(!json.contains(&user_home), "the plan names {user_home}");
    let plan_file = mr::stage(&fx.roots, &all_options(), &plan.hash, None, &fake)
        .unwrap()
        .plan_file;
    move_home::run_with(&plan_file, &fake, None, &RunHooks::default()).unwrap();
    let mut stack = vec![fx.root.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            let m = std::fs::symlink_metadata(&p).unwrap();
            if m.file_type().is_symlink() {
                assert!(!std::fs::read_link(&p)
                    .unwrap()
                    .to_string_lossy()
                    .contains(&user_home));
            } else if m.is_dir() {
                stack.push(p);
            } else {
                let bytes = std::fs::read(&p).unwrap();
                assert!(
                    !bytes
                        .windows(user_home.len())
                        .any(|win| win == user_home.as_bytes()),
                    "{} names {user_home}",
                    p.display()
                );
            }
        }
    }
}

// ── Review fixes (2026-11-12) ─────────────────────────────────────────────

/// Item 4: in free text a space never ends a moved path (Finder's
/// `projects 2`, `hollow-vault copy`); whole values match only exactly or
/// with `/` after.
#[test]
fn move_home_a_space_never_ends_a_moved_path() {
    let m = maps(&[
        ("/x/projects", "/h/Projects"),
        ("/x/hollow-vault", "/h/Vault"),
    ]);
    let r = |t: &str| rekey::rewrite_paths(t, &m).0;
    assert_eq!(r("see /x/projects 2/a"), "see /x/projects 2/a");
    assert_eq!(r("/x/hollow-vault copy"), "/x/hollow-vault copy");
    assert_eq!(r("/x/projects:/x/hollow-vault"), "/h/Projects:/h/Vault");
    assert_eq!(r("at /x/projects/a. Next"), "at /h/Projects/a. Next");
    assert_eq!(
        r("/x/projects\n/x/projects/b"),
        "/h/Projects\n/h/Projects/b"
    );
    assert_eq!(rekey::rewrite_whole("/x/projects 2/a", &m), None);
    assert_eq!(rekey::rewrite_whole("/x/projects2", &m), None);
    assert_eq!(
        rekey::rewrite_whole("/x/projects", &m).as_deref(),
        Some("/h/Projects")
    );
    assert_eq!(
        rekey::rewrite_whole("/x/projects/a b", &m).as_deref(),
        Some("/h/Projects/a b")
    );
    assert_eq!(
        rekey::rewrite_json_value("/x/hollow-vault copy/x", &m),
        None
    );
    assert_eq!(
        rekey::rewrite_json_value("cd /x/projects/a && ls", &m).as_deref(),
        Some("cd /h/Projects/a && ls")
    );
    let toml = "[projects.\"/x/projects 2\"]\n[projects.\"/x/projects/a\"]\n";
    assert_eq!(
        rekey::rewrite_codex_headers(toml, &m).0,
        "[projects.\"/x/projects 2\"]\n[projects.\"/h/Projects/a\"]\n"
    );
    assert_eq!(
        rekey::rewrite_git_pointer("gitdir: /x/projects 2/.git/worktrees/a\n", &m).1,
        0
    );
    assert_eq!(
        rekey::rewrite_git_pointer("gitdir: /x/projects/a/.git/worktrees/w\n", &m).0,
        "gitdir: /h/Projects/a/.git/worktrees/w\n"
    );
    // A Sessions path column is a whole value.
    let ids = BTreeMap::new();
    assert_eq!(
        mp::rewrite_value("/x/projects 2/a", mp::ColKind::Path, &m, &ids),
        None
    );
    assert_eq!(
        mp::rewrite_value("/x/projects/a", mp::ColKind::Path, &m, &ids).as_deref(),
        Some("/h/Projects/a")
    );
}

/// Item 1: the vault is already `<home>/Vault` (same stem), the Projects
/// folder moves: workspace folders are renamed and the Sessions database's
/// path and workspace id columns are rewritten in place, and verified.
#[test]
fn move_home_vault_already_home_still_rewrites_sessions() {
    let fx = Fx::build(true);
    let fake = Fake::default();
    let plan = fx.plan(&Options::default(), &fake);
    assert_eq!(plan.old_stem, plan.new_stem);
    assert!(!plan.stores.iter().any(|s| s.what == mp::WHAT_SESSIONS));
    assert!(mr::op_names(&plan).contains(&"sessions".to_string()));
    let plan_file = fx.stage(&Options::default(), &fake);
    let before = fx.tree();
    let r = move_home::run_with(&plan_file, &fake, None, &RunHooks::default());
    assert!(r.is_ok(), "{r:?}");
    let h = &fx.hollow;
    let new3d = rekey::workspace_id(&h.join("Projects/3dprint"));
    let db = mp::sessions_db(&fx.roots, &fx.stem);
    let conn = mp::open_read_only(&db).unwrap();
    let cwd: String = conn
        .query_row("SELECT cwd FROM sessions WHERE key='k1'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        cwd,
        h.join("Vault/workspaces").join(&new3d).to_string_lossy()
    );
    let wl: String = conn
        .query_row("SELECT workspace_id FROM work_links", [], |r| r.get(0))
        .unwrap();
    assert_eq!(wl, new3d);
    drop(conn);
    assert!(h.join("Vault/workspaces").join(&new3d).is_dir());
    let st = move_home::status(&fx.roots_after());
    assert!(
        st.checks.iter().any(|c| c.name == "sessions" && c.ok),
        "{:#?}",
        st.checks
    );
    assert!(st.checks.iter().all(|c| c.ok), "{:#?}", st.checks);
    let rb = move_home::rollback_with(plan_file.parent().unwrap(), &fake, None);
    assert!(rb.is_ok(), "{rb:?}");
    assert_eq!(diff(&before, &fx.tree()), Vec::<String>::new());
}

/// Items 2 and 3: a rollback killed after it marked the steps undone, before
/// it restored files, reads as unfinished (not done), blocks a launch, and
/// finishes on the next try (restore runs again).
#[test]
fn move_home_rollback_killed_before_restore_finishes_on_retry() {
    let fx = Fx::new();
    let fake = Fake::default();
    let plan_file = fx.stage(&all_options(), &fake);
    let dir = plan_file.parent().unwrap().to_path_buf();
    let before = fx.tree();
    move_home::run_with(&plan_file, &fake, None, &RunHooks::default()).unwrap();
    let kill = RunHooks {
        kill_before_restore: true,
        ..Default::default()
    };
    let r = mr::rollback_with_hooks(&dir, &fake, None, &kill);
    assert_eq!(r, Err(mr::KILLED.to_string()));
    let st = move_home::status(&fx.roots);
    assert_eq!(st.state, "running", "an interrupted rollback is unfinished");
    assert!(matches!(
        mr::unfinished(&fx.roots),
        Some(mr::Unfinished::Interrupted { locked: false, .. })
    ));
    assert!(!mr::should_relaunch(&dir));
    let r = move_home::rollback_with(&dir, &fake, None);
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(diff(&before, &fx.tree()), Vec::<String>::new());
    assert_eq!(move_home::status(&fx.roots).state, "rolledBack");
    assert!(mr::should_relaunch(&dir));
}

/// Item 3: a failed undo is a half state: status `failed`, the launch guard
/// stops on it and the helper doesn't open Hollow. A refused rollback
/// changes nothing and the move stays done.
#[test]
fn move_home_half_states_block_launch_and_relaunch() {
    let fx = Fx::new();
    let fake = Fake::default();
    let plan_file = fx.stage(&Options::default(), &fake);
    let dir = plan_file.parent().unwrap().to_path_buf();
    move_home::run_with(&plan_file, &fake, None, &RunHooks::default()).unwrap();
    assert!(mr::should_relaunch(&dir));
    assert_eq!(mr::unfinished(&fx.roots_after()), None);
    // Refused (new work): the move stands.
    let new_db = mp::sessions_db(
        &fx.roots_after(),
        &rekey::stem_for(&fx.hollow.join("Vault")),
    );
    let c = Connection::open(&new_db).unwrap();
    c.execute(
        "INSERT INTO turns(session_key, ord, started_at) VALUES ('k1', 9, 't')",
        [],
    )
    .unwrap();
    assert!(move_home::rollback_with(&dir, &fake, None).is_err());
    assert_eq!(move_home::status(&fx.roots_after()).state, "done");
    assert_eq!(mr::unfinished(&fx.roots_after()), None);
    c.execute("DELETE FROM turns WHERE ord = 9", []).unwrap();
    drop(c);
    std::fs::remove_file(dir.join("verified.json")).unwrap();
    // An undo that fails partway: the old workspace folder is back.
    let old3d = rekey::workspace_id(&fx.r1.join("3dprint"));
    w(
        &fx.hollow.join("Vault/workspaces").join(&old3d).join("x.md"),
        "x",
    );
    let r = move_home::rollback_with(&dir, &fake, None);
    assert!(
        r.as_ref()
            .is_err_and(|e| e.contains("can't take its name back")),
        "{r:?}"
    );
    assert_eq!(move_home::status(&fx.roots).state, "failed");
    assert!(matches!(
        mr::unfinished(&fx.roots),
        Some(mr::Unfinished::Failed { .. })
    ));
    assert!(!mr::should_relaunch(&dir));
}

/// Item 5: `~/.claude.json` rewritten by Claude after the snapshot (before
/// its step) is still rewritten: a changed file is not "already done".
#[test]
fn move_home_agent_settings_written_after_snapshot_are_rewritten() {
    let fx = Fx::new();
    let fake = Fake::default();
    let plan_file = fx.stage(&Options::default(), &fake);
    let plan = read_plan(&plan_file);
    let i = plan
        .files
        .iter()
        .position(|f| f.what == mp::FILE_CLAUDE_JSON)
        .unwrap();
    let at = mr::op_names(&plan)
        .iter()
        .position(|n| *n == format!("file:{i}"))
        .unwrap();
    let r = move_home::run_with(
        &plan_file,
        &fake,
        None,
        &RunHooks {
            stop_after: Some(at),
            ..Default::default()
        },
    );
    assert_eq!(r, Err(mr::KILLED.to_string()));
    let cj_path = fx.home.join(".claude.json");
    let mut cj: serde_json::Value = serde_json::from_str(&read(&cj_path)).unwrap();
    let late = fx.r1.join("app");
    cj["projects"][late.to_string_lossy().as_ref()] = serde_json::json!({"late": true});
    std::fs::write(&cj_path, serde_json::to_string_pretty(&cj).unwrap()).unwrap();
    move_home::run_with(&plan_file, &fake, None, &RunHooks::default()).unwrap();
    let cj: serde_json::Value = serde_json::from_str(&read(&cj_path)).unwrap();
    let new_app = fx.hollow.join("Projects/app");
    assert_eq!(
        cj["projects"][new_app.to_string_lossy().as_ref()]["late"],
        true,
        "{cj:#}"
    );
    assert!(cj["projects"]
        .get(fx.r1.join("3dprint").to_string_lossy().as_ref())
        .is_none());
}

/// Item 5: a Claude or Codex CLI running outside Hollow is a warning in the
/// dry run and stops the run before any change.
#[test]
fn move_home_agent_cli_running_refuses_the_run() {
    let fx = Fx::new();
    let fake = Fake {
        agent_clis: vec!["claude (pid 4242)".into()],
        ..Default::default()
    };
    let plan = fx.plan(&Options::default(), &fake);
    assert!(plan
        .blockers
        .iter()
        .any(|b| b.kind == "agentCli" && !b.blocking && b.detail.contains("pid 4242")));
    let plan_file = fx.stage(&Options::default(), &fake);
    let before = fx.tree();
    let r = move_home::run_with(&plan_file, &fake, None, &RunHooks::default());
    assert!(
        r.as_ref()
            .is_err_and(|e| e.contains("claude (pid 4242)") && e.contains("Nothing was changed")),
        "{r:?}"
    );
    assert_eq!(diff(&before, &fx.tree()), Vec::<String>::new());
    assert!(mr::should_relaunch(plan_file.parent().unwrap()));
    assert!(
        move_home::agent_cli_name("/Users/a/.local/share/claude/versions/2.1.322", "2.1.322")
            .is_some()
    );
    assert_eq!(
        move_home::agent_cli_name("/opt/homebrew/bin/codex", "codex"),
        Some("codex")
    );
    assert_eq!(
        move_home::agent_cli_name(
            "/usr/local/bin/node",
            "node /x/@anthropic-ai/claude-code/cli.js"
        ),
        Some("claude")
    );
    assert_eq!(move_home::agent_cli_name("/bin/zsh", "zsh -c claude"), None);
}

/// Item 6: a file outside the vault changed after the move (an agent added
/// a project) is mapped back new to old on rollback, keeping the change.
#[test]
fn move_home_rollback_maps_back_files_changed_since() {
    let fx = Fx::new();
    let fake = Fake::default();
    let plan_file = fx.stage(&Options::default(), &fake);
    let before = fx.tree();
    move_home::run_with(&plan_file, &fake, None, &RunHooks::default()).unwrap();
    let cj_path = fx.home.join(".claude.json");
    let mut cj: serde_json::Value = serde_json::from_str(&read(&cj_path)).unwrap();
    cj["projects"]["/other/new"] = serde_json::json!({});
    std::fs::write(&cj_path, serde_json::to_string_pretty(&cj).unwrap()).unwrap();
    let r = move_home::rollback_with(plan_file.parent().unwrap(), &fake, None).unwrap();
    assert!(
        r.contains("mapped back") && r.contains(".claude.json"),
        "{r}"
    );
    let cj: serde_json::Value = serde_json::from_str(&read(&cj_path)).unwrap();
    assert!(
        cj["projects"].get("/other/new").is_some(),
        "the later change is kept"
    );
    assert!(cj["projects"]
        .get(fx.r1.join("3dprint").to_string_lossy().as_ref())
        .is_some());
    assert!(cj["projects"]
        .get(
            fx.hollow
                .join("Projects/3dprint")
                .to_string_lossy()
                .as_ref()
        )
        .is_none());
    let rel = cj_path
        .strip_prefix(&fx.root)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let d: Vec<String> = diff(&before, &fx.tree())
        .into_iter()
        .filter(|l| !l.starts_with(&format!("{rel}:")))
        .collect();
    assert_eq!(d, Vec::<String>::new());
}

/// Item 7: killed between moving the original Sessions trio out and moving
/// the rewritten copy in, a resumed run finishes the swap from the copy.
#[test]
fn move_home_killed_mid_sessions_swap_resumes() {
    let fx = Fx::new();
    let fake = Fake::default();
    let plan_file = fx.stage(&Options::default(), &fake);
    let kill = RunHooks {
        kill_in_sessions_swap: true,
        ..Default::default()
    };
    assert_eq!(
        move_home::run_with(&plan_file, &fake, None, &kill),
        Err(mr::KILLED.to_string())
    );
    assert_eq!(move_home::status(&fx.roots).state, "running");
    let r = move_home::run_with(&plan_file, &fake, None, &RunHooks::default());
    assert!(r.is_ok(), "{r:?}");
    let st = move_home::status(&fx.roots_after());
    assert!(st.checks.iter().all(|c| c.ok), "{:#?}", st.checks);
    let new_db = mp::sessions_db(&fx.roots, &rekey::stem_for(&fx.hollow.join("Vault")));
    let n: i64 = mp::open_read_only(&new_db)
        .unwrap()
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 2);
}

/// Item 8: something real at a moved folder's old path stops the rollback
/// before anything is undone, and says what to move aside.
#[test]
fn move_home_rollback_stops_when_the_old_path_is_taken() {
    let fx = Fx::new();
    let fake = Fake::default();
    let plan_file = fx.stage(&Options::default(), &fake);
    move_home::run_with(&plan_file, &fake, None, &RunHooks::default()).unwrap();
    std::fs::remove_file(&fx.vault).unwrap();
    w(&fx.vault.join("new.txt"), "made after the move");
    let after = fx.tree();
    let r = move_home::rollback_with(plan_file.parent().unwrap(), &fake, None);
    assert!(
        r.as_ref()
            .is_err_and(|e| e.contains("in the way") && e.contains("Nothing was rolled back")),
        "{r:?}"
    );
    assert_eq!(
        diff(&after, &fx.tree()),
        Vec::<String>::new(),
        "nothing was touched"
    );
    assert_eq!(move_home::status(&fx.roots_after()).state, "done");
    assert_eq!(mr::unfinished(&fx.roots_after()), None);
}

/// Item 9: a Claude folder with no transcript is renamed only when exactly
/// one folder on disk fits its name; `projects 2` (beside the Projects
/// folder, not in it) is reported and left.
#[test]
fn move_home_claude_dirs_without_a_transcript_are_rebuilt_from_disk() {
    let fx = Fx::new();
    let plan = fx.plan(&Options::default(), &Fake::default());
    let cp = fx.home.join(".claude/projects");
    let rs = cp.join(rekey::claude_dir_name(
        &fx.r1.join("personal/runescape").to_string_lossy(),
    ));
    let copy = cp.join(rekey::claude_dir_name(&format!("{} 2", fx.r1.display())));
    assert!(plan.stores.iter().any(|s| s.from == rs.to_string_lossy()
        && s.to
            == cp
                .join(rekey::claude_dir_name(
                    &fx.hollow
                        .join("Projects/Personal/runescape")
                        .to_string_lossy()
                ))
                .to_string_lossy()));
    assert!(!plan
        .stores
        .iter()
        .any(|s| s.from.starts_with(&*copy.to_string_lossy())));
    let agent = plan
        .agents
        .iter()
        .find(|a| a.agent == "Claude Code projects")
        .unwrap();
    assert!(
        agent
            .ambiguous
            .iter()
            .any(|a| a.contains("no folder on disk fits")),
        "{:#?}",
        agent.ambiguous
    );
}

/// Item 10: reading a database with a `-wal` and no `-shm` creates nothing
/// beside it and still sees the WAL's rows.
#[test]
fn move_home_read_only_open_never_creates_shm() {
    let fx = Fx::new();
    let db = mp::sessions_db(&fx.roots, &fx.stem);
    std::fs::remove_file(mp::sidecar(&db, "-shm")).unwrap();
    let wal = std::fs::read(mp::sidecar(&db, "-wal")).unwrap();
    let main = std::fs::read(&db).unwrap();
    let conn = mp::open_read_only(&db).unwrap();
    let n: i64 = conn
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 2, "the WAL's rows are read");
    drop(conn);
    assert!(!mp::sidecar(&db, "-shm").exists(), "no -shm was created");
    assert_eq!(std::fs::read(mp::sidecar(&db, "-wal")).unwrap(), wal);
    assert_eq!(std::fs::read(&db).unwrap(), main);
    let _ = mp::plan_with(&fx.roots, &Options::default(), None, &Fake::default()).unwrap();
    assert!(
        !mp::sidecar(&db, "-shm").exists(),
        "the dry run created no -shm"
    );
}

/// Item 11: an evicted folder is counted as evicted and never listed.
#[test]
fn move_home_evicted_folder_is_counted_never_listed() {
    let fx = Fx::new();
    let mut fake = Fake::default();
    fake.dataless.insert(fx.r1.join("personal/runescape"));
    let plan = fx.plan(&Options::default(), &fake);
    let personal = plan
        .moves
        .iter()
        .find(|m| m.kind == MoveKind::Personal)
        .unwrap();
    assert_eq!(personal.files, 0, "its file was not listed");
    assert_eq!(personal.dataless, 1);
    assert_eq!(plan.dataless.total, 1);
    assert!(plan.dataless.roots[0].sample[0].ends_with("personal/runescape"));
}

// ── Delta review fixes ────────────────────────────────────────────────────

/// A: rebuilding a Claude folder's path never lists an iCloud-evicted
/// folder (reported instead), and a child name must match whole (`-fo` is
/// not the start of `-foo-bar`), so an evicted `fo` is never reached.
#[test]
fn move_home_claude_dir_decode_never_lists_an_evicted_folder() {
    let fx = Fx::new();
    let cp = fx.home.join(".claude/projects");
    w(&fx.r1.join("foo-bar/a.txt"), "a");
    std::fs::create_dir_all(fx.r1.join("fo")).unwrap();
    let foo = cp.join(rekey::claude_dir_name(
        &fx.r1.join("foo-bar").to_string_lossy(),
    ));
    w(&foo.join("memory/MEMORY.md"), "m");
    let mut fake = Fake::default();
    fake.dataless.insert(fx.r1.join("personal"));
    fake.dataless.insert(fx.r1.join("fo"));
    let plan = fx.plan(&Options::default(), &fake);
    let rs = cp.join(rekey::claude_dir_name(
        &fx.r1.join("personal/runescape").to_string_lossy(),
    ));
    assert!(!plan.stores.iter().any(|s| s.from == rs.to_string_lossy()));
    let agent = plan
        .agents
        .iter()
        .find(|a| a.agent == "Claude Code projects")
        .unwrap();
    assert!(
        agent.ambiguous.iter().any(|a| a.contains("iCloud-evicted")),
        "{:#?}",
        agent.ambiguous
    );
    assert!(
        plan.stores.iter().any(|s| s.from == foo.to_string_lossy()),
        "foo-bar is found without listing fo"
    );
}

/// B: a rollback that failed partway (after restoring vault notes) is
/// retried without being refused by its own restores.
#[test]
fn move_home_retried_rollback_is_not_refused_by_its_own_restores() {
    let fx = Fx::new();
    let fake = Fake::default();
    let plan_file = fx.stage(&Options::default(), &fake);
    let dir = plan_file.parent().unwrap().to_path_buf();
    let before = fx.tree();
    move_home::run_with(&plan_file, &fake, None, &RunHooks::default()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    // A store's old name is taken again: that undo fails, the rest goes on.
    let stray = fx
        .roots
        .support
        .join("MainAgent")
        .join(format!("{}.json", fx.stem));
    w(&stray, "stray");
    assert!(move_home::rollback_with(&dir, &fake, None).is_err());
    std::fs::remove_file(&stray).unwrap();
    let r = move_home::rollback_with(&dir, &fake, None);
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(diff(&before, &fx.tree()), Vec::<String>::new());
}

/// C: a file step killed after its write landed journals its size before
/// the write from the copy, so verify's byte count holds on resume.
#[test]
fn move_home_resumed_file_step_keeps_its_size_before() {
    let fx = Fx::new();
    let fake = Fake::default();
    let plan_file = fx.stage(&Options::default(), &fake);
    let plan = read_plan(&plan_file);
    let note = fx
        .vault
        .join("workspaces")
        .join(rekey::workspace_id(&fx.r1.join("3dprint")))
        .join("WORKSPACE.md");
    let i = plan
        .files
        .iter()
        .position(|f| f.path == note.to_string_lossy())
        .unwrap();
    let kill = RunHooks {
        kill_after_write: Some(i),
        ..Default::default()
    };
    assert_eq!(
        move_home::run_with(&plan_file, &fake, None, &kill),
        Err(mr::KILLED.to_string())
    );
    let r = move_home::run_with(&plan_file, &fake, None, &RunHooks::default());
    assert!(r.is_ok(), "{r:?}");
    let st = move_home::status(&fx.roots_after());
    assert!(st.checks.iter().all(|c| c.ok), "{:#?}", st.checks);
}

/// D: with the stem unchanged, killed after only the original main file
/// moved out: a resume moves the original `-wal` out before the copy goes in,
/// and a rollback puts the originals back untouched.
#[test]
fn move_home_same_stem_swap_killed_after_the_first_original() {
    let fake = Fake::default();
    let kill = RunHooks {
        kill_after_first_original: true,
        ..Default::default()
    };
    let fx = Fx::build(true);
    let plan_file = fx.stage(&Options::default(), &fake);
    assert_eq!(
        move_home::run_with(&plan_file, &fake, None, &kill),
        Err(mr::KILLED.to_string())
    );
    let db = mp::sessions_db(&fx.roots, &fx.stem);
    assert!(
        mp::sidecar(&db, "-wal").exists(),
        "the original -wal is still in place"
    );
    move_home::run_with(&plan_file, &fake, None, &RunHooks::default()).unwrap();
    assert!(
        !mp::sidecar(&db, "-wal").exists(),
        "no original -wal beside the copy"
    );
    assert!(plan_file
        .parent()
        .unwrap()
        .join("sessions-original")
        .join(format!("{}.db-wal", fx.stem))
        .exists());
    let cwd: String = mp::open_read_only(&db)
        .unwrap()
        .query_row("SELECT cwd FROM sessions WHERE key='k1'", [], |r| r.get(0))
        .unwrap();
    let new3d = rekey::workspace_id(&fx.hollow.join("Projects/3dprint"));
    assert!(cwd.ends_with(&new3d), "{cwd}");
    assert!(move_home::status(&fx.roots_after())
        .checks
        .iter()
        .all(|c| c.ok));

    let fx = Fx::build(true);
    let plan_file = fx.stage(&Options::default(), &fake);
    let before = fx.tree();
    assert_eq!(
        move_home::run_with(&plan_file, &fake, None, &kill),
        Err(mr::KILLED.to_string())
    );
    let r = move_home::rollback_with(plan_file.parent().unwrap(), &fake, None);
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(diff(&before, &fx.tree()), Vec::<String>::new());
}

/// E: a failed run stopped before its automatic rollback began is half done:
/// the launch rolls it back first and the helper doesn't open Hollow.
#[test]
fn move_home_failed_run_stopped_before_its_rollback_is_unfinished() {
    let fx = Fx::new();
    let fake = Fake::default();
    let plan_file = fx.stage(&Options::default(), &fake);
    let dir = plan_file.parent().unwrap().to_path_buf();
    let before = fx.tree();
    let plan = read_plan(&plan_file);
    let late = plan
        .stores
        .iter()
        .find(|s| s.what == mp::WHAT_STORE && s.to.ends_with(".lock"))
        .unwrap();
    w(Path::new(&late.to), "made meanwhile");
    let kill = RunHooks {
        kill_before_auto_rollback: true,
        ..Default::default()
    };
    assert_eq!(
        move_home::run_with(&plan_file, &fake, None, &kill),
        Err(mr::KILLED.to_string())
    );
    assert!(mr::half_state(&dir).is_some());
    assert!(!mr::should_relaunch(&dir));
    assert!(matches!(
        mr::unfinished(&fx.roots),
        Some(mr::Unfinished::Interrupted { locked: false, .. })
    ));
    let r = move_home::rollback_with(&dir, &fake, None);
    assert!(r.is_ok(), "{r:?}");
    std::fs::remove_file(&late.to).unwrap();
    assert_eq!(diff(&before, &fx.tree()), Vec::<String>::new());
    assert!(mr::should_relaunch(&dir));
}

/// F: `:`, `<` and `&` end a path in free text; a JSON path value with a
/// space in it is free text, not a whole value.
#[test]
fn move_home_free_text_boundaries_and_json_values_with_spaces() {
    let m = maps(&[("/x/projects", "/h/Projects")]);
    assert_eq!(
        rekey::rewrite_paths(
            "PATH=/x/projects:/bin <string>/x/projects</string> cd /x/projects&&ls",
            &m
        )
        .0,
        "PATH=/h/Projects:/bin <string>/h/Projects</string> cd /h/Projects&&ls"
    );
    assert_eq!(
        rekey::rewrite_json_value("/usr/bin/x --dir /x/projects/a", &m).as_deref(),
        Some("/usr/bin/x --dir /h/Projects/a")
    );
    assert_eq!(rekey::rewrite_json_value("/x/projects 2/a", &m), None);
    assert_eq!(
        rekey::rewrite_json_value("/x/projects/a", &m).as_deref(),
        Some("/h/Projects/a")
    );
}

/// The read-only dry run against this Mac's real config, for a person to
/// check by hand: `cargo test --lib move_home_real_dry_run -- --ignored
/// --nocapture`. It reads only: no write, no download of evicted files.
#[test]
#[ignore = "reads this Mac's real Hollow config and folders (read-only); run by hand: cargo test --lib move_home_real_dry_run -- --ignored --nocapture"]
fn move_home_real_dry_run() {
    let home = dirs::home_dir().unwrap();
    let config = home.join(".config/Hollow/config.json");
    let support = home.join("Library/Application Support/Hollow");
    let app_data = home.join("Library/Application Support/com.hollow.app");
    let roots = Roots::with(&home, &config, &support, &app_data, "Hollow").unwrap();
    let options = Options {
        budget_secs: Some(90),
        ..Default::default()
    };
    let plan = mp::plan_with(&roots, &options, None, &move_home::SystemProbes).unwrap();
    for l in &plan.text {
        println!("{l}");
    }
}

/// Second delta review: a rollback killed after the copy's sidecars went
/// aside but before its main file did resumes and restores the original
/// trio byte for byte (sidecars first, so a main file present means its
/// sidecars are still there).
#[test]
fn move_home_rollback_killed_between_copy_sidecars_and_main_resumes() {
    let fake = Fake::default();
    let fx = Fx::build(true);
    let plan_file = fx.stage(&Options::default(), &fake);
    let dir = plan_file.parent().unwrap().to_path_buf();
    let before = fx.tree();
    move_home::run_with(&plan_file, &fake, None, &RunHooks::default()).unwrap();
    // The copy as Hollow left it: its own `-wal` and `-shm` beside it.
    let db = mp::sessions_db(&fx.roots, &fx.stem);
    std::fs::write(mp::sidecar(&db, "-wal"), b"the copy's wal").unwrap();
    std::fs::write(mp::sidecar(&db, "-shm"), b"the copy's shm").unwrap();
    let kill = RunHooks {
        kill_rollback_before_copy_main: true,
        ..Default::default()
    };
    assert_eq!(
        mr::rollback_with_hooks(&dir, &fake, None, &kill),
        Err(mr::KILLED.to_string())
    );
    let r = move_home::rollback_with(&dir, &fake, None);
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(diff(&before, &fx.tree()), Vec::<String>::new());
}

/// Final review: a rollback killed after the original `-wal`/`-shm` came back
/// but before the original main file did, or stopped by a conflict, resumes
/// on retry and restores the original trio byte for byte, never leaving the
/// original WAL behind in the backup.
#[test]
fn move_home_rollback_killed_while_restoring_the_original_trio_resumes() {
    let fake = Fake::default();
    let fx = Fx::build(true);
    let db = mp::sessions_db(&fx.roots, &fx.stem);
    assert!(
        mp::sidecar(&db, "-wal").exists(),
        "the fixture has an original -wal"
    );
    let plan_file = fx.stage(&Options::default(), &fake);
    let dir = plan_file.parent().unwrap().to_path_buf();
    let before = fx.tree();
    move_home::run_with(&plan_file, &fake, None, &RunHooks::default()).unwrap();
    let kill = RunHooks {
        kill_rollback_before_original_main: true,
        ..Default::default()
    };
    assert_eq!(
        mr::rollback_with_hooks(&dir, &fake, None, &kill),
        Err(mr::KILLED.to_string())
    );
    let r = move_home::rollback_with(&dir, &fake, None);
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(diff(&before, &fx.tree()), Vec::<String>::new());
    assert!(
        !dir.join("sessions-original")
            .join(format!("{}.db-wal", fx.stem))
            .exists(),
        "the original -wal came back"
    );
}

// ── Folders in iCloud (File Provider): copy, compare, retire last ─────────

fn fake_icloud(fx: &Fx) -> Fake {
    Fake {
        fp: vec![fx.home.join("Documents")],
        ..Default::default()
    }
}

#[test]
fn move_home_icloud_folders_are_planned_as_copies_nested_ones_as_renames() {
    let fx = Fx::new();
    let fake = fake_icloud(&fx);
    let plan = fx.plan(&all_options(), &fake);
    assert!(plan.ready, "{:#?}", plan.blockers);
    let copies: Vec<&move_home::PlannedMove> =
        plan.moves.iter().filter(|m| m.method == "copy").collect();
    assert!(copies.iter().any(|m| m.kind == MoveKind::Vault));
    assert!(copies.iter().any(|m| m.kind == MoveKind::ProjectsRoot));
    for (i, m) in plan.moves.iter().enumerate() {
        let nested = plan.moves[..i]
            .iter()
            .any(|o| m.from.starts_with(&format!("{}/", o.from)));
        assert_eq!(m.method == "copy", !nested, "{}", m.from);
    }
    assert_eq!(plan.dataless.copy_folders.len(), copies.len());
    let text = mp::describe(&plan).join("\n");
    assert!(text.contains("copied out of iCloud"), "{text}");
    assert!(text.contains("Folders in iCloud, moved by copy"), "{text}");
    // Without a File Provider domain everything stays a rename.
    let plain = fx.plan(&all_options(), &Fake::default());
    assert!(plain.moves.iter().all(|m| m.method == "rename"));
    assert!(plain.dataless.copy_folders.is_empty());
}

#[test]
fn move_home_icloud_run_copies_keeps_originals_until_the_end_and_rolls_back() {
    let fx = Fx::new();
    let fake = fake_icloud(&fx);
    let options = all_options();
    let plan_file = fx.stage(&options, &fake);
    let before = fx.tree();
    let out = move_home::run_with(&plan_file, &fake, None, &RunHooks::default());
    let journal = read(&plan_file.with_file_name("journal.jsonl"));
    assert!(out.is_ok(), "{out:?}\n{journal}");
    let h = &fx.hollow;
    assert!(h.join("Vault/AGENTS.md").is_file());
    assert!(h.join("Projects/Personal/runescape/notes.md").is_file());
    // The originals are gone and links stay at the old paths.
    assert_eq!(std::fs::read_link(&fx.vault).unwrap(), h.join("Vault"));
    assert_eq!(std::fs::read_link(&fx.r1).unwrap(), h.join("Projects"));
    let plan = read_plan(&plan_file);
    let names = mr::op_names(&plan);
    let retire = names.iter().position(|n| n.starts_with("retire:")).unwrap();
    let verify = names.iter().position(|n| n == "verify").unwrap();
    assert!(retire > verify, "{names:?}");
    assert!(
        journal.contains("\"step\":\"retire:0\",\"state\":\"done\""),
        "{journal}"
    );
    assert!(
        journal.contains("\\\"copied\\\":true"),
        "the copy is journaled with what was compared: {journal}"
    );
    // No leftover temporary copy.
    for m in &plan.moves {
        assert!(!mp::sidecar(Path::new(&m.to), ".move-home-tmp").exists());
    }
    let rb = move_home::rollback_with(plan_file.parent().unwrap(), &fake, None);
    assert!(rb.is_ok(), "{rb:?}");
    let d = diff(&before, &fx.tree());
    assert!(d.is_empty(), "{d:#?}");
}

#[test]
fn move_home_icloud_kill_at_every_step_then_rollback_restores_the_tree() {
    let fake_for = |fx: &Fx| fake_icloud(fx);
    let count = {
        let fx = Fx::new();
        mr::op_count(&fx.plan(&all_options(), &fake_for(&fx)))
    };
    for k in 0..=count {
        for crash_mid in [false, true] {
            if crash_mid && k == count {
                continue;
            }
            let fx = Fx::new();
            let fake = fake_for(&fx);
            let plan_file = fx.stage(&all_options(), &fake);
            let names = mr::op_names(&read_plan(&plan_file));
            let before = fx.tree();
            let r = move_home::run_with(
                &plan_file,
                &fake,
                None,
                &RunHooks {
                    stop_after: Some(k),
                    crash_mid,
                    ..Default::default()
                },
            );
            if k < count {
                assert_eq!(r, Err("stopped (test)".into()), "k={k}");
            }
            let rb = move_home::rollback_with(plan_file.parent().unwrap(), &fake, None);
            let step = names.get(k).cloned().unwrap_or_default();
            assert!(rb.is_ok(), "k={k} ({step}) crash_mid={crash_mid}: {rb:?}");
            let d = diff(&before, &fx.tree());
            assert!(d.is_empty(), "k={k} ({step}) crash_mid={crash_mid}: {d:#?}");
        }
    }
}

#[test]
fn move_home_icloud_killed_after_the_copy_or_inside_the_retire_rolls_back() {
    for hooks in [
        RunHooks {
            kill_after_copy_rename: true,
            ..Default::default()
        },
        RunHooks {
            kill_in_retire: true,
            ..Default::default()
        },
    ] {
        let fx = Fx::new();
        let fake = fake_icloud(&fx);
        let plan_file = fx.stage(&all_options(), &fake);
        let before = fx.tree();
        let r = move_home::run_with(&plan_file, &fake, None, &hooks);
        assert_eq!(r, Err(mr::KILLED.to_string()), "{hooks:?}");
        if hooks.kill_in_retire {
            // Part of the original is gone: the half state blocks a launch.
            assert!(mr::half_state(plan_file.parent().unwrap()).is_some());
        }
        let rb = move_home::rollback_with(plan_file.parent().unwrap(), &fake, None);
        assert!(rb.is_ok(), "{hooks:?}: {rb:?}");
        let d = diff(&before, &fx.tree());
        assert!(d.is_empty(), "{hooks:?}: {d:#?}");
    }
}

#[test]
fn move_home_icloud_failed_checks_keep_the_originals() {
    let fx = Fx::new();
    let mut fake = fake_icloud(&fx);
    fake.launchd_fails = true;
    let plan_file = fx.stage(&Options::default(), &fake);
    let before = fx.tree();
    let r = move_home::run_with(&plan_file, &fake, None, &RunHooks::default());
    assert!(
        r.as_ref()
            .is_err_and(|e| e.contains("verify found problems")),
        "{r:?}"
    );
    // The original vault is still a real folder beside the copy.
    assert!(std::fs::symlink_metadata(&fx.vault).unwrap().is_dir());
    assert!(!std::fs::symlink_metadata(&fx.vault)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(fx.hollow.join("Vault/AGENTS.md").is_file());
    let journal = read(&plan_file.with_file_name("journal.jsonl"));
    assert!(journal.contains("the checks failed"), "{journal}");
    let rb = move_home::rollback_with(plan_file.parent().unwrap(), &fake, None);
    assert!(rb.is_ok(), "{rb:?}");
    assert_eq!(diff(&before, &fx.tree()), Vec::<String>::new());
}

#[test]
fn move_home_icloud_evicted_files_block_until_downloaded_and_the_probe_agrees() {
    let fx = Fx::new();
    let mut fake = fake_icloud(&fx);
    let note = fx.vault.join("workflows/herald.md");
    fake.dataless.insert(note.clone());
    let plan = fx.plan(&Options::default(), &fake);
    assert!(!plan.ready);
    let b = plan
        .blockers
        .iter()
        .find(|b| b.kind == "dataless" && b.blocking)
        .unwrap_or_else(|| panic!("{:#?}", plan.blockers));
    assert!(b.detail.contains("move-home --download"), "{}", b.detail);
    assert!(plan.dataless.to_download >= 1);
    assert!(plan.dataless.download_secs >= 1);
    assert!(mp::describe(&plan).join("\n").contains("Download first"));
    // Evicted between staging and the run: the probe refuses before any change.
    let clean = fake_icloud(&fx);
    let plan_file = fx.stage(&Options::default(), &clean);
    let before = fx.tree();
    let r = move_home::run_with(&plan_file, &fake, None, &RunHooks::default());
    assert!(r.as_ref().is_err_and(|e| e.contains("--download")), "{r:?}");
    assert_eq!(diff(&before, &fx.tree()), Vec::<String>::new());
}

#[test]
fn move_home_icloud_copy_pieces() {
    use crate::move_home_icloud as ic;
    let tmp = tempfile::TempDir::new().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let src = root.join("src");
    w(&src.join("a/b/c.txt"), "hello");
    w(&src.join("top.txt"), "top");
    sym(Path::new("top.txt"), &src.join("link"));
    std::fs::set_permissions(
        src.join("top.txt"),
        std::os::unix::fs::PermissionsExt::from_mode(0o640),
    )
    .unwrap();
    let dst = root.join("dst");
    assert!(ic::clone_tree(&src, &dst).is_ok());
    let st = ic::compare_trees(&src, &dst, false).unwrap();
    assert_eq!((st.files, st.links), (2, 1));
    assert!(
        ic::clone_tree(&src, &dst).is_err(),
        "an existing target is refused"
    );
    // A changed byte count, a missing entry and an extra entry are caught.
    let other = root.join("other");
    assert!(ic::clone_tree(&src, &other).is_ok());
    std::fs::write(other.join("a/b/c.txt"), "hello!").unwrap();
    assert!(ic::compare_trees(&src, &other, false).is_err());
    ic::remove_tree(&other).unwrap();
    std::fs::remove_file(dst.join("top.txt")).unwrap();
    assert!(ic::compare_trees(&src, &dst, false).is_err());
    assert!(
        ic::compare_trees(&dst, &src, true).is_ok(),
        "a partly removed copy is a subset"
    );
    w(&dst.join("extra.txt"), "x");
    assert!(ic::compare_trees(&dst, &src, true).is_err());
    // Removal copes with read-only folders.
    std::fs::set_permissions(
        dst.join("a"),
        std::os::unix::fs::PermissionsExt::from_mode(0o500),
    )
    .unwrap();
    assert!(ic::remove_tree(&dst).is_ok());
    assert!(!dst.exists());
    // The probe leaves nothing behind.
    let dest_dir = root.join("dest-dir");
    std::fs::create_dir(&dest_dir).unwrap();
    assert!(ic::clone_probe(&src, &dest_dir, "t").is_ok());
    assert_eq!(std::fs::read_dir(&dest_dir).unwrap().count(), 0);
    assert!(!src.join(".hollow-move-probe-t").exists());
    // Estimates and text.
    assert!(ic::download_secs(150_100, 0) > ic::download_secs(1, 0));
    assert_eq!(ic::download_secs(12, 0), 1);
    assert_eq!(ic::duration_text(40), "40 s");
    assert_eq!(ic::duration_text(600), "10 min");
    assert_eq!(ic::duration_text(3 * 3600 + 5 * 60), "3 h 5 min");
}

#[test]
fn move_home_icloud_download_reads_evicted_files_and_counts_them() {
    use crate::move_home_icloud as ic;
    let tmp = tempfile::TempDir::new().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    w(&root.join("a/one.txt"), "12345");
    w(&root.join("a/b/two.txt"), "1234567890");
    w(&root.join("three.txt"), "local");
    let mut fake = Fake::default();
    fake.dataless.insert(root.join("a/one.txt"));
    fake.dataless.insert(root.join("a/b/two.txt"));
    let ev = ic::count_evicted(&root, &fake);
    assert_eq!((ev.files, ev.bytes, ev.dirs), (2, 15, 0));
    let stop = std::sync::atomic::AtomicBool::new(false);
    let lines = std::sync::atomic::AtomicUsize::new(0);
    let d = ic::download_evicted(std::slice::from_ref(&root), &fake, 4, &stop, &|_| {
        lines.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    });
    assert_eq!((d.files, d.bytes, d.failed.len()), (2, 15, 0));
    assert!(
        lines.load(std::sync::atomic::Ordering::Relaxed) >= 1,
        "a final progress line"
    );
    // A stop flag ends the pass before it reads anything.
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let d = ic::download_evicted(&[root], &fake, 4, &stop, &|_| {});
    assert!(d.stopped);
    assert_eq!(d.files, 0);
}

/// Real iCloud behaviour, run by hand on a Mac with Desktop & Documents in
/// iCloud: `HOLLOW_ICLOUD_TEST_SRC` is a scratch folder inside `~/Documents`
/// (a tree of files is made in it), `HOLLOW_ICLOUD_TEST_DEST` a folder
/// outside iCloud on the same volume. Clones the tree out, compares, probes,
/// removes the original, and prints the timings.
#[test]
#[ignore = "needs a real iCloud folder: HOLLOW_ICLOUD_TEST_SRC and HOLLOW_ICLOUD_TEST_DEST"]
fn move_home_icloud_real_folder() {
    use crate::move_home_icloud as ic;
    let (Ok(src), Ok(dest)) = (
        std::env::var("HOLLOW_ICLOUD_TEST_SRC"),
        std::env::var("HOLLOW_ICLOUD_TEST_DEST"),
    ) else {
        panic!("set HOLLOW_ICLOUD_TEST_SRC and HOLLOW_ICLOUD_TEST_DEST");
    };
    let (src, dest) = (PathBuf::from(src), PathBuf::from(dest));
    let n: usize = std::env::var("HOLLOW_ICLOUD_TEST_FILES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30_000);
    let probes = move_home::SystemProbes;
    let domain = probes.file_provider_domain(&src);
    println!(
        "source domain: {domain:?}; dest domain: {:?}",
        probes.file_provider_domain(&dest)
    );
    assert!(
        domain.is_some(),
        "the source is not in a File Provider domain"
    );
    assert!(probes.file_provider_domain(&dest).is_none());
    let tree = src.join("tree");
    for i in 0..n {
        let p = tree.join(format!("d{}/e{}/f{i}.txt", i % 50, (i / 50) % 20));
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, vec![b'x'; 2048 + (i % 7) * 512]).unwrap();
    }
    sym(Path::new("../x"), &tree.join("d0/lnk"));
    let t = std::time::Instant::now();
    let ev = ic::count_evicted(&tree, &probes);
    println!("count_evicted: {:?} in {:?}", ev, t.elapsed());
    assert_eq!(ev.files + ev.dirs, 0);
    let t = std::time::Instant::now();
    ic::clone_probe(&src, &dest, "real").unwrap();
    println!("clone_probe: {:?}", t.elapsed());
    let copy = dest.join("tree-copy");
    let t = std::time::Instant::now();
    let how = ic::clone_tree(&tree, &copy).unwrap();
    println!("clone_tree ({how}): {:?}", t.elapsed());
    let t = std::time::Instant::now();
    let st = ic::compare_trees(&tree, &copy, false).unwrap();
    println!("compare_trees: {st:?} in {:?}", t.elapsed());
    assert_eq!(st.files, n as u64);
    let t = std::time::Instant::now();
    ic::remove_tree(&tree).unwrap();
    println!("remove_tree (original in iCloud): {:?}", t.elapsed());
    // A rollback rebuilds the original by cloning the copy back into the
    // domain, straight to its path.
    let t = std::time::Instant::now();
    let how = ic::clone_tree(&copy, &tree).unwrap();
    println!("clone_back ({how}) into iCloud: {:?}", t.elapsed());
    let t = std::time::Instant::now();
    let st = ic::compare_trees(&copy, &tree, false).unwrap();
    println!("compare_back: {st:?} in {:?}", t.elapsed());
    ic::remove_tree(&tree).unwrap();
    // The copy is whole and readable after the original is gone.
    let again = ic::count_evicted(&copy, &probes);
    assert_eq!(again.files + again.dirs, 0);
    assert_eq!(
        std::fs::read(copy.join("d1/e0/f1.txt")).unwrap().len(),
        2048 + 512
    );
    ic::remove_tree(&copy).unwrap();
}

#[test]
fn move_home_icloud_rollback_killed_while_removing_the_copy_finishes_on_retry() {
    let fx = Fx::new();
    let fake = fake_icloud(&fx);
    let plan_file = fx.stage(&all_options(), &fake);
    let dir = plan_file.parent().unwrap().to_path_buf();
    let before = fx.tree();
    move_home::run_with(&plan_file, &fake, None, &RunHooks::default()).unwrap();
    let kill = RunHooks {
        kill_in_rollback_copy: true,
        ..Default::default()
    };
    assert_eq!(
        mr::rollback_with_hooks(&dir, &fake, None, &kill),
        Err(mr::KILLED.to_string())
    );
    // Both the rebuilt original and the copy exist: the retry must not
    // delete the rebuilt original and clone a half-removed copy over it.
    let plan = read_plan(&plan_file);
    let both = plan.moves.iter().filter(|m| m.method == "copy").any(|m| {
        let real_dir = |p: &str| {
            std::fs::symlink_metadata(p).is_ok_and(|x| x.is_dir() && !x.file_type().is_symlink())
        };
        real_dir(&m.from) && real_dir(&m.to)
    });
    assert!(both, "the kill left a rebuilt original and its copy");
    let r = move_home::rollback_with(&dir, &fake, None);
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(diff(&before, &fx.tree()), Vec::<String>::new());
}

#[test]
fn move_home_icloud_rollback_never_deletes_something_else_at_the_old_path() {
    let fx = Fx::new();
    let fake = fake_icloud(&fx);
    let plan_file = fx.stage(&Options::default(), &fake);
    let dir = plan_file.parent().unwrap().to_path_buf();
    let before = fx.tree();
    move_home::run_with(&plan_file, &fake, None, &RunHooks::default()).unwrap();
    // The link at the old path becomes a real folder with other content
    // (another Mac's iCloud change, a person, a script).
    std::fs::remove_file(&fx.vault).unwrap();
    w(&fx.vault.join("mine.txt"), "not part of the move");
    let r = move_home::rollback_with(&dir, &fake, None);
    assert!(r.as_ref().is_err_and(|e| e.contains("in the way")), "{r:?}");
    assert_eq!(read(&fx.vault.join("mine.txt")), "not part of the move");
    assert!(
        fx.hollow.join("Vault/AGENTS.md").is_file(),
        "the copy stays until the path is free"
    );
    std::fs::remove_dir_all(&fx.vault).unwrap();
    let r = move_home::rollback_with(&dir, &fake, None);
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(diff(&before, &fx.tree()), Vec::<String>::new());
}

#[test]
fn move_home_icloud_original_written_after_the_copy_is_kept_with_a_warning() {
    let fx = Fx::new();
    let fake = fake_icloud(&fx);
    let plan_file = fx.stage(&Options::default(), &fake);
    let dir = plan_file.parent().unwrap().to_path_buf();
    let names = mr::op_names(&read_plan(&plan_file));
    let retire = names.iter().position(|n| n.starts_with("retire:")).unwrap();
    let r = move_home::run_with(
        &plan_file,
        &fake,
        None,
        &RunHooks {
            stop_after: Some(retire),
            ..Default::default()
        },
    );
    assert_eq!(r, Err(mr::KILLED.to_string()));
    // Something writes to the old path after the copy was made.
    std::thread::sleep(std::time::Duration::from_millis(50));
    w(&fx.vault.join("late-note.md"), "written to the old path");
    let r = move_home::run_with(&plan_file, &fake, None, &RunHooks::default());
    let msg = r.unwrap_or_else(|e| panic!("a kept original is a warning, not a failure: {e}"));
    assert!(
        msg.contains("Warning") && msg.contains("late-note.md"),
        "{msg}"
    );
    assert_eq!(
        read(&fx.vault.join("late-note.md")),
        "written to the old path"
    );
    assert!(!std::fs::symlink_metadata(&fx.vault)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(fx.hollow.join("Vault/AGENTS.md").is_file());
    assert_eq!(move_home::status(&fx.roots_after()).state, "done");
    // Rolling back removes the copy and keeps the later note.
    let r = move_home::rollback_with(&dir, &fake, None);
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(
        read(&fx.vault.join("late-note.md")),
        "written to the old path"
    );
}

#[test]
fn move_home_icloud_a_failed_retire_is_a_warning_and_a_rollback_rebuilds_the_original() {
    let fx = Fx::new();
    let fake = fake_icloud(&fx);
    let plan_file = fx.stage(&Options::default(), &fake);
    let dir = plan_file.parent().unwrap().to_path_buf();
    let before = fx.tree();
    let names = mr::op_names(&read_plan(&plan_file));
    let retire = names.iter().position(|n| n.starts_with("retire:")).unwrap();
    let r = move_home::run_with(
        &plan_file,
        &fake,
        None,
        &RunHooks {
            stop_after: Some(retire),
            ..Default::default()
        },
    );
    assert_eq!(r, Err(mr::KILLED.to_string()));
    // A file the removal can't delete (an immutable flag, as iCloud or another
    // tool may set).
    let locked = fx.vault.join("AGENTS.md");
    assert!(std::process::Command::new("chflags")
        .args(["uchg"])
        .arg(&locked)
        .status()
        .unwrap()
        .success());
    let r = move_home::run_with(&plan_file, &fake, None, &RunHooks::default());
    let unlock = || {
        let _ = std::process::Command::new("chflags")
            .args(["-R", "nouchg"])
            .arg(&fx.root)
            .status();
    };
    let msg = match r {
        Ok(m) => m,
        Err(e) => {
            unlock();
            panic!("a failed retire must not roll the verified move back: {e}");
        }
    };
    assert!(msg.contains("Warning"), "{msg}");
    assert!(fx.hollow.join("Vault/AGENTS.md").is_file());
    // The rollback can't delete the leftover while it is locked, and says so
    // without touching the copy.
    let r = move_home::rollback_with(&dir, &fake, None);
    assert!(r.is_err(), "{r:?}");
    assert!(fx.hollow.join("Vault/AGENTS.md").is_file());
    unlock();
    let r = move_home::rollback_with(&dir, &fake, None);
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(diff(&before, &fx.tree()), Vec::<String>::new());
}

#[test]
fn move_home_icloud_a_copy_folder_inside_another_copy_folder_is_blocked() {
    let m = |from: &str, method: &str| move_home::PlannedMove {
        from: from.into(),
        to: format!("/h/{from}"),
        kind: MoveKind::Project,
        bytes: 0,
        files: 0,
        dataless: 0,
        dataless_bytes: 0,
        method: method.into(),
        replaces_symlink: None,
        compat_symlink: true,
        why: String::new(),
    };
    let both = [m("/d/R1/vault", "copy"), m("/d/R1", "copy")];
    let why = mp::copy_layout_conflict(&both).unwrap();
    assert!(why.contains("/d/R1/vault is inside /d/R1"), "{why}");
    assert!(mp::copy_layout_conflict(&[m("/d/R1/vault", "rename"), m("/d/R1", "copy")]).is_none());
    assert!(mp::copy_layout_conflict(&[m("/d/R1", "copy"), m("/d/R2", "copy")]).is_none());
    assert!(mp::copy_layout_conflict(&[m("/d/R1", "copy"), m("/d/R1x", "copy")]).is_none());
}

/// Run until just before the retire steps, so a test can change the folders
/// at the old paths first.
fn stop_before_retire(fx: &Fx, plan_file: &Path, fake: &Fake) {
    let names = mr::op_names(&read_plan(plan_file));
    let retire = names.iter().position(|n| n.starts_with("retire:")).unwrap();
    let r = move_home::run_with(
        plan_file,
        fake,
        None,
        &RunHooks {
            stop_after: Some(retire),
            ..Default::default()
        },
    );
    assert_eq!(r, Err(mr::KILLED.to_string()));
    let _ = fx;
}

#[test]
fn move_home_icloud_a_kept_original_means_a_rollback_parks_later_work_instead_of_deleting_it() {
    let fx = Fx::new();
    let fake = fake_icloud(&fx);
    let plan_file = fx.stage(&Options::default(), &fake);
    let dir = plan_file.parent().unwrap().to_path_buf();
    stop_before_retire(&fx, &plan_file, &fake);
    std::thread::sleep(std::time::Duration::from_millis(50));
    w(&fx.r1.join("late.md"), "written to the old Projects folder");
    let msg = move_home::run_with(&plan_file, &fake, None, &RunHooks::default()).unwrap();
    assert!(msg.contains("Warning") && msg.contains("late.md"), "{msg}");
    // Hollow runs on the copy for a while.
    std::thread::sleep(std::time::Duration::from_millis(50));
    let work = fx.hollow.join("Projects/3dprint/new-work.txt");
    w(&work, "written after the move");
    let r = move_home::rollback_with(&dir, &fake, None);
    let out = r.unwrap_or_else(|e| panic!("a rollback must not be refused for this: {e}"));
    assert!(out.contains("parked"), "{out}");
    // The later work survives in the backup folder; the old original (kept
    // in iCloud) is back in place with its own late note.
    let parked: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("kept-copy-"))
        })
        .collect();
    assert_eq!(parked.len(), 1, "{parked:?}");
    assert_eq!(
        read(&parked[0].join("3dprint/new-work.txt")),
        "written after the move"
    );
    assert_eq!(
        read(&fx.r1.join("late.md")),
        "written to the old Projects folder"
    );
    assert!(
        !fx.hollow.join("Projects/3dprint").exists()
            || std::fs::symlink_metadata(fx.hollow.join("Projects"))
                .unwrap()
                .file_type()
                .is_symlink()
    );
}

#[test]
fn move_home_icloud_the_original_must_hold_exactly_what_was_copied() {
    // A file that arrives with an old modification time (iCloud delivering
    // another Mac's file, a mv) and a folder that can't be listed both keep
    // the original.
    for case in ["old-mtime", "unlistable"] {
        let fx = Fx::new();
        let fake = fake_icloud(&fx);
        let plan_file = fx.stage(&Options::default(), &fake);
        stop_before_retire(&fx, &plan_file, &fake);
        let hidden = fx.vault.join("workflows");
        if case == "old-mtime" {
            let f = fx.vault.join("from-another-mac.md");
            w(&f, "arrived late");
            let old = std::time::SystemTime::now() - std::time::Duration::from_secs(86_400);
            std::fs::File::options()
                .write(true)
                .open(&f)
                .unwrap()
                .set_modified(old)
                .unwrap();
        } else {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hidden, std::fs::Permissions::from_mode(0o000)).unwrap();
        }
        let r = move_home::run_with(&plan_file, &fake, None, &RunHooks::default());
        if case == "unlistable" {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hidden, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let msg = r.unwrap_or_else(|e| panic!("{case}: {e}"));
        assert!(
            msg.contains("Warning") && msg.contains("kept in iCloud"),
            "{case}: {msg}"
        );
        assert!(
            !std::fs::symlink_metadata(&fx.vault)
                .unwrap()
                .file_type()
                .is_symlink(),
            "{case}: the original stays"
        );
    }
}

// The 11-13 fixes (workspace aliases, special files): the five checks.
#[path = "move_home_fix_tests.rs"]
mod fix_tests;
