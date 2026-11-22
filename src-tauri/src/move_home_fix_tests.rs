//! Move Hollow home, the 11-13 fixes: the five checks asked for before a
//! third try (a child of `move_home_tests`, so it shares its fixture).
//!
//! 1. Chats, workspace history and audit records are reachable under the new
//!    ids after a move (`audit_log` keeps its rows; `workspace_aliases`).
//! 2. Two same-named folders (`dynamic-pricing-and-competitive-intelligence`
//!    in both roots) keep separate identities and the right contents.
//! 3. Git worktrees in moved projects still resolve.
//! 4. A failure after the re-key starts restores the folders and a
//!    byte-identical Sessions database.
//! 5. Special files: a known throwaway one is left out and the source is
//!    untouched; any other stops the move.
//!
//! The moves here are the iCloud kind (copy, compare, retire last), the way
//! the real move runs, unless a test says otherwise.

use super::*;
use crate::audit::{AuditEntry, AuditFilter};
use std::os::unix::fs::MetadataExt;

const DPCI: &str = "dynamic-pricing-and-competitive-intelligence";

fn fixture_db(fx: &Fx) -> PathBuf {
    fx.roots
        .support
        .join("Sessions")
        .join(format!("{}.db", fx.stem))
}

/// Add rows to the fixture's Sessions database in one transaction.
fn with_db(fx: &Fx, f: impl FnOnce(&Connection)) {
    let mut conn = Connection::open(fixture_db(fx)).unwrap();
    let tx = conn.transaction().unwrap();
    f(&tx);
    tx.commit().unwrap();
}

/// One audit row through Hollow's own writer (hash chain and head).
fn audit_row(conn: &Connection, session: &str, workspace: &str) -> AuditEntry {
    crate::audit::append(
        conn,
        AuditEntry {
            kind: crate::audit::kind::RUN_STARTED.into(),
            session_key: Some(session.into()),
            workspace_id: Some(workspace.into()),
            asked_by: Some("person".into()),
            agent: Some("claude".into()),
            ..Default::default()
        },
    )
    .unwrap()
    .unwrap()
}

fn usage_row(conn: &Connection, run_key: &str, workspace: &str, tokens: i64) {
    let at = crate::session_store::db_time(chrono::Utc::now());
    conn.execute(
        "INSERT INTO usage_ledger(run_key, at, started_at, surface, billing, outcome, spawned, \
         basis, cost_status, input_tokens, output_tokens, workspace_id) \
         VALUES (?1, ?2, ?2, 'chat', 'subscription', 'ok', 1, 'reported', 'known', ?3, 0, ?4)",
        rusqlite::params![run_key, at, tokens, workspace],
    )
    .unwrap();
}

fn chat(conn: &Connection, key: &str, cwd: &Path) {
    conn.execute(
        "INSERT INTO sessions(key, agent, cwd, created_at, last_activity_at) \
         VALUES (?1, 'claude', ?2, 't', 't')",
        rusqlite::params![key, cwd.to_string_lossy()],
    )
    .unwrap();
}

/// The moved Sessions database, copied (one consistent file) to where
/// Hollow's readers look for the new vault's store, so the real readers run
/// on it. Removed on drop.
struct Installed {
    vault: PathBuf,
    db: PathBuf,
}

impl Drop for Installed {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(mp::sidecar(&self.db, suffix));
        }
    }
}

fn install_for_readers(fx: &Fx) -> Installed {
    let vault = fx.hollow.join("Vault");
    let moved = fx
        .roots
        .support
        .join("Sessions")
        .join(format!("{}.db", rekey::stem_for(&vault)));
    let db = crate::session_store::db_path_for(&vault).unwrap();
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(mp::sidecar(&db, suffix));
    }
    let c = mp::open_read_only(&moved).unwrap();
    c.execute("VACUUM INTO ?1", [db.to_string_lossy()]).unwrap();
    Installed { vault, db }
}

/// A brief Hollow lists as a workspace, naming its project.
fn brief(title: &str, project: &Path, marker: &str) -> String {
    format!(
        "---\nhollow_workspace: 1\ntitle: \"{title}\"\nstatus: active\n---\n\n# {title}\n\nWork with the existing project at {}. Original files stay there.\n\n{marker}\n",
        project.display()
    )
}

/// Add connected projects to the fixture's Directories store.
fn connect(fx: &Fx, projects: &[(&Path, &str)]) {
    let path = fx
        .roots
        .support
        .join("Directories")
        .join(format!("{}.json", fx.stem));
    let mut v: serde_json::Value = serde_json::from_str(&read(&path)).unwrap();
    let list = v["projects"].as_array_mut().unwrap();
    for (p, title) in projects {
        list.push(serde_json::json!({"id": id(p), "path": p, "title": title}));
    }
    std::fs::write(&path, v.to_string()).unwrap();
}

fn sha(path: &Path) -> Option<String> {
    use sha2::Digest;
    std::fs::read(path)
        .ok()
        .map(|b| format!("{:x}", sha2::Sha256::digest(b)))
}

/// sha256 of the Sessions database's main file, `-wal` and `-shm`.
fn db_hashes(db: &Path) -> Vec<(String, Option<String>)> {
    ["", "-wal", "-shm"]
        .iter()
        .map(|s| (s.to_string(), sha(&mp::sidecar(db, s))))
        .collect()
}

fn git_out(dir: &Path, home: &Path, args: &[&str]) -> (bool, String) {
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
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

/// A stale Unix socket at `at`, as a crashed daemon leaves it. Bound at a
/// short path (macOS caps a socket path at 104 bytes) and renamed in.
fn stale_socket(at: &Path) {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let short = PathBuf::from(format!("/tmp/hmf-{}-{n}.sock", std::process::id()));
    let _ = std::fs::remove_file(&short);
    drop(std::os::unix::net::UnixListener::bind(&short).unwrap());
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::rename(&short, at).unwrap();
    assert!(std::fs::symlink_metadata(at)
        .unwrap()
        .file_type()
        .is_socket_kind());
}

trait SocketKind {
    fn is_socket_kind(&self) -> bool;
}
impl SocketKind for std::fs::FileType {
    fn is_socket_kind(&self) -> bool {
        use std::os::unix::fs::FileTypeExt;
        self.is_socket()
    }
}

fn fifo(at: &Path) {
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    let c = std::ffi::CString::new(at.to_string_lossy().as_bytes()).unwrap();
    // SAFETY: a valid NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o644) }, 0);
}

/// Every entry under `root`: (kind, mode, size, mtime in ns), links not
/// followed. The before/after listing for the special-file checks.
fn listing(root: &Path) -> BTreeMap<String, (String, u32, u64, i128)> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            let m = std::fs::symlink_metadata(&p).unwrap();
            let ft = m.file_type();
            let kind = if ft.is_symlink() {
                "link".to_string()
            } else if ft.is_dir() {
                stack.push(p.clone());
                "dir".to_string()
            } else if let Some(k) = crate::move_home_icloud::special_kind(&ft) {
                k.to_string()
            } else {
                "file".to_string()
            };
            // Folder times are left out: the probe clones a small tree
            // through each source and removes it, which touches the folder.
            let mtime = if ft.is_dir() {
                0
            } else {
                m.mtime() as i128 * 1_000_000_000 + m.mtime_nsec() as i128
            };
            out.insert(
                p.strip_prefix(root).unwrap().to_string_lossy().into_owned(),
                (kind, m.mode() & 0o7777, m.len(), mtime),
            );
        }
    }
    out
}

// ── Check 1 ───────────────────────────────────────────────────────────────

#[test]
fn move_home_fix_chats_history_and_audit_reachable_under_new_ids() {
    let fx = Fx::new();
    let fake = fake_icloud(&fx);
    let p3d_old = fx.r1.join("3dprint");
    let old3d = id(&p3d_old);
    let new3d = id(&fx.hollow.join("Projects/3dprint"));
    let ws_old = fx.vault.join("workspaces").join(&old3d);
    // A History & recovery run changed the workspace's brief and a note
    // before the move.
    w(&ws_old.join("notes.md"), "notes v1\n");
    let handle = crate::automated_history::begin_run(&fx.vault, "chat", Some("k1")).unwrap();
    w(&ws_old.join("notes.md"), "notes v2\n");
    let brief_text = format!(
        "{}\nA chat added this line.\n",
        read(&ws_old.join("WORKSPACE.md"))
    );
    w(&ws_old.join("WORKSPACE.md"), &brief_text);
    let hrun = crate::automated_history::finish_run(&fx.vault, handle, "success").unwrap();
    let old_note = format!("workspaces/{old3d}/WORKSPACE.md");
    assert!(
        hrun.changes.iter().any(|c| c.vault_path == old_note),
        "{:?}",
        hrun.changes
    );
    // Audit rows (append-only, hash-chained) and usage under the old id.
    let mut audit_ids = Vec::new();
    with_db(&fx, |c| {
        audit_ids.push(audit_row(c, "k1", &old3d).id);
        audit_ids.push(audit_row(c, "k1", &old3d).id);
        audit_row(c, "other", "main");
        usage_row(c, "run-3d", &old3d, 1234);
    });
    let plan_file = fx.stage(&all_options(), &fake);
    let plan = read_plan(&plan_file);
    assert!(
        plan.notes
            .iter()
            .any(|n| n.contains("audit_log.workspace_id: 2 rows keep their old workspace ids")),
        "{:#?}",
        plan.notes
    );
    let out = move_home::run_with(&plan_file, &fake, None, &RunHooks::default());
    assert!(
        out.is_ok(),
        "{out:?}\n{}",
        read(&plan_file.with_file_name("journal.jsonl"))
    );
    let st = move_home::status(&fx.roots_after());
    assert_eq!(st.state, "done", "{st:#?}");
    let check = st
        .checks
        .iter()
        .find(|c| c.name == "workspace aliases")
        .unwrap();
    assert!(check.ok, "{check:?}");

    let inst = install_for_readers(&fx);
    let v = &inst.vault;
    // Audit: the rows are unchanged (old id, chain intact) and found by the
    // new id and by the old one.
    for asked in [&new3d, &old3d] {
        let rows = crate::audit::query(
            v,
            &AuditFilter {
                workspace: Some(asked.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        let mut got: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
        got.sort();
        let mut want = audit_ids.clone();
        want.sort();
        assert_eq!(got, want, "asked {asked}");
        assert!(rows
            .iter()
            .all(|r| r.workspace_id.as_deref() == Some(old3d.as_str())));
    }
    let action = crate::audit::read_action(
        v,
        AuditFilter {
            workspace: Some(new3d.clone()),
            ..Default::default()
        },
        true,
    )
    .unwrap();
    assert_eq!(action["verify"]["ok"], true, "{action}");
    assert_eq!(action["entries"][0]["currentWorkspaceId"], new3d.as_str());
    assert_eq!(action["entries"][0]["workspaceId"], old3d.as_str());
    // Usage: the run ledger, the CFO's project budget and Settings spend.
    let rows = crate::run_ledger::rows_between(v, "2000-01-01", "2100-01-01").unwrap();
    let r = rows.iter().find(|r| r.run_key == "run-3d").unwrap();
    assert_eq!(r.workspace, new3d);
    let tokens = crate::session_store::with_read(v, |c| {
        crate::cfo::budgets::project_tokens(
            c,
            &new3d,
            chrono::Utc::now() - chrono::Duration::hours(5),
        )
    })
    .unwrap();
    assert_eq!(tokens, 1234);
    // Chats: the conversation's folder is the workspace's new folder.
    let conn = mp::open_read_only(&inst.db).unwrap();
    let cwd: String = conn
        .query_row("SELECT cwd FROM sessions WHERE key='k1'", [], |r| r.get(0))
        .unwrap();
    let ws_new = v.join("workspaces").join(&new3d);
    assert_eq!(cwd, ws_new.to_string_lossy());
    assert!(ws_new.join("WORKSPACE.md").is_file());
    assert!(read(&ws_new.join("WORKSPACE.md")).contains("A chat added this line."));
    // Workspace history: the run is listed under the new path and its
    // versions read.
    let new_note = format!("workspaces/{new3d}/WORKSPACE.md");
    let listed = crate::automated_history::list_note_history(v, &new_note, 10).unwrap();
    assert!(listed.iter().any(|r| r.id == hrun.id), "{listed:?}");
    let after = crate::automated_history::read_version(v, &hrun.id, &new_note, "after")
        .unwrap()
        .unwrap();
    assert!(after.contains("A chat added this line."));
    let shown = crate::automated_history::get_run(v, &hrun.id).unwrap();
    assert!(
        shown.changes.iter().any(|c| c.vault_path == new_note),
        "{:?}",
        shown.changes
    );
    // A scoped run's folder maps with its paths, so the overlap recheck
    // (`under_scope`) still matches them.
    let mut scoped = hrun.clone();
    scoped.scope = Some(format!("workspaces/{old3d}"));
    let aliases = crate::workspace_aliases::Aliases::read(v);
    let scoped = crate::automated_history::current_paths(v, &aliases, scoped);
    assert_eq!(scoped.scope, Some(format!("workspaces/{new3d}")));
    assert!(scoped.changes.iter().any(|c| c.vault_path == new_note));
    // Restoring a change puts the file back where it is now, never under the
    // old id. (The brief itself is refused unforced: the move rewrote its
    // project line, so it no longer matches the run's after-version.)
    let new_notes = format!("workspaces/{new3d}/notes.md");
    crate::automated_history::restore_change(v, &hrun.id, &new_notes, false).unwrap();
    assert_eq!(read(&ws_new.join("notes.md")), "notes v1\n");
    let refused = crate::automated_history::restore_change(v, &hrun.id, &new_note, false);
    assert!(
        refused
            .as_ref()
            .is_err_and(|e| e.contains("no longer matches")),
        "{refused:?}"
    );
    assert!(!v.join("workspaces").join(&old3d).exists());
    // Control: without the aliases the same audit read finds nothing, which
    // is what re-keying without them would leave.
    drop(conn);
    Connection::open(&inst.db)
        .unwrap()
        .execute_batch("DROP TABLE workspace_aliases")
        .unwrap();
    let rows = crate::audit::query(
        v,
        &AuditFilter {
            workspace: Some(new3d.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(rows.is_empty(), "{rows:?}");
}

// ── Check 2 ───────────────────────────────────────────────────────────────

#[test]
fn move_home_fix_same_named_folders_keep_separate_identities() {
    let fx = Fx::new();
    let fake = fake_icloud(&fx);
    let r1d = fx.r1.join(DPCI);
    let r2d = fx.r2.join(DPCI);
    w(&r1d.join("README.md"), "R1: the older copy\n");
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
    std::fs::File::options()
        .write(true)
        .open(r1d.join("README.md"))
        .unwrap()
        .set_modified(old)
        .unwrap();
    w(&r2d.join("app.py"), "R2: the newer copy\n");
    connect(&fx, &[(&r1d, "DPCI (R1)"), (&r2d, "DPCI (R2)")]);
    let (id1, id2) = (id(&r1d), id(&r2d));
    let ws = fx.vault.join("workspaces");
    w(
        &ws.join(&id1).join("WORKSPACE.md"),
        &brief("DPCI R1", &r1d, "MARK-R1"),
    );
    w(
        &ws.join(&id2).join("WORKSPACE.md"),
        &brief("DPCI R2", &r2d, "MARK-R2"),
    );
    let (mut a1, mut a2) = (String::new(), String::new());
    with_db(&fx, |c| {
        chat(c, "chat-r1", &ws.join(&id1));
        chat(c, "chat-r2", &ws.join(&id2));
        a1 = audit_row(c, "chat-r1", &id1).id;
        a2 = audit_row(c, "chat-r2", &id2).id;
    });
    let plan_file = fx.stage(&all_options(), &fake);
    let plan = read_plan(&plan_file);
    let to_of = |from: &Path| {
        plan.moves
            .iter()
            .find(|m| Path::new(&m.from) == from)
            .map(|m| PathBuf::from(&m.to))
            .unwrap()
    };
    let (t1, t2) = (to_of(&r1d), to_of(&r2d));
    assert!(
        t1.starts_with(fx.hollow.join("Archive")),
        "{}",
        t1.display()
    );
    assert_eq!(t2, fx.hollow.join("Projects").join(DPCI));
    let (n1, n2) = (id(&t1), id(&t2));
    assert_ne!(n1, n2);
    let pairs: BTreeMap<String, String> = plan
        .ids
        .iter()
        .map(|i| (i.old.clone(), i.new.clone()))
        .collect();
    assert_eq!(pairs.get(&id1), Some(&n1));
    assert_eq!(pairs.get(&id2), Some(&n2));
    let out = move_home::run_with(&plan_file, &fake, None, &RunHooks::default());
    assert!(
        out.is_ok(),
        "{out:?}\n{}",
        read(&plan_file.with_file_name("journal.jsonl"))
    );
    // Contents: each folder where it belongs.
    assert_eq!(read(&t1.join("README.md")), "R1: the older copy\n");
    assert!(!t1.join("app.py").exists());
    assert_eq!(read(&t2.join("app.py")), "R2: the newer copy\n");
    assert!(!t2.join("README.md").exists());
    // Workspaces: each brief under its own new id, naming its own folder.
    let nws = fx.hollow.join("Vault/workspaces");
    let b1 = read(&nws.join(&n1).join("WORKSPACE.md"));
    let b2 = read(&nws.join(&n2).join("WORKSPACE.md"));
    assert!(
        b1.contains("MARK-R1") && b1.contains(&*t1.to_string_lossy()),
        "{b1}"
    );
    assert!(
        b2.contains("MARK-R2") && b2.contains(&*t2.to_string_lossy()),
        "{b2}"
    );
    assert!(!nws.join(&id1).exists() && !nws.join(&id2).exists());
    let inst = install_for_readers(&fx);
    let homes = crate::workspaces::SessionHomes::read(&inst.vault).unwrap();
    let conn = mp::open_read_only(&inst.db).unwrap();
    let cwd_of = |k: &str| -> String {
        conn.query_row("SELECT cwd FROM sessions WHERE key=?1", [k], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(
        homes.workspace_for(&cwd_of("chat-r1")).as_deref(),
        Some(n1.as_str())
    );
    assert_eq!(
        homes.workspace_for(&cwd_of("chat-r2")).as_deref(),
        Some(n2.as_str())
    );
    // Audit: each old row shows under its own workspace only.
    let ids_for = |w: &str| -> Vec<String> {
        crate::audit::query(
            &inst.vault,
            &AuditFilter {
                workspace: Some(w.into()),
                ..Default::default()
            },
        )
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect()
    };
    assert_eq!(ids_for(&n1), vec![a1.clone()]);
    assert_eq!(ids_for(&n2), vec![a2.clone()]);
    drop(conn);
}

// ── Check 3 ───────────────────────────────────────────────────────────────

fn assert_worktrees_resolve(fx: &Fx) {
    let app = fx.hollow.join("Projects/app");
    let (ok, list) = git_out(&app, &fx.home, &["worktree", "list", "--porcelain"]);
    assert!(ok, "{list}");
    let wt1 = fx.hollow.join("Worktrees/app/wt1");
    let wt2 = app.join(".claude/worktrees/wt2");
    for p in [&app, &wt1, &wt2] {
        assert!(
            list.contains(&format!("worktree {}\n", p.display())),
            "{} missing:\n{list}",
            p.display()
        );
    }
    assert!(!list.contains("prunable"), "{list}");
    for (wt, branch) in [(&wt1, "wt1"), (&wt2, "wt2")] {
        let (ok, st) = git_out(wt, &fx.home, &["status", "--porcelain=v1", "-b"]);
        assert!(ok, "git status in {}: {st}", wt.display());
        assert!(st.starts_with(&format!("## {branch}")), "{st}");
        let (ok, top) = git_out(wt, &fx.home, &["rev-parse", "--show-toplevel"]);
        assert!(ok && top.trim() == wt.to_string_lossy(), "{top}");
    }
}

#[test]
fn move_home_fix_git_worktrees_resolve_after_a_copy_move_and_a_rename_move() {
    for icloud in [true, false] {
        let fx = Fx::new();
        let fake = if icloud {
            fake_icloud(&fx)
        } else {
            Fake::default()
        };
        let plan_file = fx.stage(&all_options(), &fake);
        let out = move_home::run_with(&plan_file, &fake, None, &RunHooks::default());
        assert!(out.is_ok(), "icloud {icloud}: {out:?}");
        assert_worktrees_resolve(&fx);
    }
}

// ── Check 4 ───────────────────────────────────────────────────────────────

#[test]
fn move_home_fix_failure_after_the_rekey_starts_restores_folders_and_db() {
    // Inside the re-key (run 2's place), and after the swapped-in copy (the
    // next step and the checks), for copy and rename moves.
    let cases: Vec<(&str, bool)> = vec![
        ("in-rekey", true),
        ("in-rekey", false),
        ("after-swap", true),
        ("after-swap", false),
        ("verify", true),
    ];
    for (case, icloud) in cases {
        let fx = Fx::new();
        let fake = if icloud {
            fake_icloud(&fx)
        } else {
            Fake::default()
        };
        with_db(&fx, |c| {
            audit_row(c, "k1", &id(&fx.r1.join("3dprint")));
        });
        let plan_file = fx.stage(&all_options(), &fake);
        let names = mr::op_names(&read_plan(&plan_file));
        let db = fixture_db(&fx);
        let hashes = db_hashes(&db);
        let before = fx.tree();
        let hooks = match case {
            "in-rekey" => RunHooks {
                fail_in_sessions_rekey: true,
                ..Default::default()
            },
            "after-swap" => {
                let i = names.iter().position(|n| n == "sessions").unwrap();
                RunHooks {
                    fail_step: Some(names[i + 1].clone()),
                    ..Default::default()
                }
            }
            _ => RunHooks {
                fail_step: Some("verify".into()),
                ..Default::default()
            },
        };
        let r = move_home::run_with(&plan_file, &fake, None, &hooks);
        let journal = read(&plan_file.with_file_name("journal.jsonl"));
        assert!(
            r.as_ref().is_err_and(|e| e.contains("rolled back")),
            "{case} {icloud}: {r:?}\n{journal}"
        );
        if case != "in-rekey" {
            assert!(
                journal.contains("\"step\":\"sessions\",\"state\":\"done\""),
                "{case}: the swap happened first\n{journal}"
            );
        }
        assert_eq!(db_hashes(&db), hashes, "{case} {icloud}: Sessions trio");
        let d = diff(&before, &fx.tree());
        assert!(d.is_empty(), "{case} {icloud}: {d:#?}");
        assert_eq!(move_home::status(&fx.roots).state, "rolledBack");
    }
}

// ── Check 5 ───────────────────────────────────────────────────────────────

#[test]
fn move_home_fix_throwaway_socket_is_left_out_and_the_source_untouched() {
    let fx = Fx::new();
    let fake = fake_icloud(&fx);
    let sock = fx.r1.join("app/.git/fsmonitor--daemon.ipc");
    let sock2 = fx.vault.join(".runtime/fsmonitor--daemon.ipc");
    stale_socket(&sock);
    stale_socket(&sock2);
    let plan = fx.plan(&all_options(), &fake);
    assert!(plan.ready, "{:#?}", plan.blockers);
    assert!(
        plan.notes
            .iter()
            .any(|n| n.contains("throwaway runtime file") && n.contains(&*sock.to_string_lossy())),
        "{:#?}",
        plan.notes
    );
    let src_before = listing(&fx.r1);
    let r2_before = listing(&fx.r2);
    let before = fx.tree();
    let plan_file = fx.stage(&all_options(), &fake);
    stop_before_retire(&fx, &plan_file, &fake);
    // Copied, compared and in place; the sources are exactly as they were,
    // sockets and mtimes included.
    let journal = read(&plan_file.with_file_name("journal.jsonl"));
    assert!(journal.contains("\\\"throwawaySpecial\\\":1"), "{journal}");
    assert_eq!(listing(&fx.r1), src_before, "R1 source changed");
    assert_eq!(listing(&fx.r2), r2_before, "R2 source changed");
    assert!(std::fs::symlink_metadata(&sock).is_ok());
    let copy_git = fx.hollow.join("Projects/app/.git");
    assert!(copy_git.is_dir());
    assert!(std::fs::symlink_metadata(copy_git.join("fsmonitor--daemon.ipc")).is_err());
    assert!(fx.hollow.join("Vault/.runtime").is_dir());
    assert!(
        std::fs::symlink_metadata(fx.hollow.join("Vault/.runtime/fsmonitor--daemon.ipc")).is_err()
    );
    // Finish: the retire takes the originals (socket and all) as planned.
    let out = move_home::run_with(&plan_file, &fake, None, &RunHooks::default());
    assert!(out.is_ok(), "{out:?}");
    assert_eq!(move_home::status(&fx.roots_after()).state, "done");
    // Roll back after the retire: the originals are rebuilt from the copies,
    // which never held the throwaway sockets, so only those differ.
    let rb = move_home::rollback_with(plan_file.parent().unwrap(), &fake, None);
    assert!(rb.is_ok(), "{rb:?}");
    let d = diff(&before, &fx.tree());
    let rel = |p: &Path| {
        p.strip_prefix(&fx.root)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    };
    assert_eq!(
        d.iter()
            .map(|l| l.split(':').next().unwrap().to_string())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([rel(&sock), rel(&sock2)]),
        "only the throwaway sockets differ after a rollback that rebuilt the originals: {d:#?}"
    );
}

#[test]
fn move_home_fix_rollback_before_the_retire_keeps_throwaway_sockets() {
    let fx = Fx::new();
    let fake = fake_icloud(&fx);
    stale_socket(&fx.r1.join("app/.git/fsmonitor--daemon.ipc"));
    let before = fx.tree();
    let plan_file = fx.stage(&all_options(), &fake);
    stop_before_retire(&fx, &plan_file, &fake);
    let rb = move_home::rollback_with(plan_file.parent().unwrap(), &fake, None);
    assert!(rb.is_ok(), "{rb:?}");
    let d = diff(&before, &fx.tree());
    assert!(d.is_empty(), "{d:#?}");
}

#[test]
fn move_home_fix_unexpected_special_file_stops_the_move() {
    // In the plan: a blocker naming the path.
    for (name, make) in [
        ("pipe", fifo as fn(&Path)),
        ("app.sock", stale_socket as fn(&Path)),
    ] {
        let fx = Fx::new();
        let fake = fake_icloud(&fx);
        let p = fx.r2.join("morning-paper").join(name);
        make(&p);
        let plan = fx.plan(&all_options(), &fake);
        assert!(!plan.ready);
        let b = plan
            .blockers
            .iter()
            .find(|b| b.kind == "specialFile")
            .unwrap_or_else(|| panic!("{:#?}", plan.blockers));
        assert!(b.detail.contains(&*p.to_string_lossy()), "{}", b.detail);
    }
    // At the run: one appeared after the check. The probe stops before any
    // change, naming it; the source and everything else are untouched.
    let fx = Fx::new();
    let fake = fake_icloud(&fx);
    let plan_file = fx.stage(&all_options(), &fake);
    let p = fx.r1.join("app/build.fifo");
    fifo(&p);
    let src_before = listing(&fx.r1);
    let before = fx.tree();
    let r = move_home::run_with(&plan_file, &fake, None, &RunHooks::default());
    let e = r.unwrap_err();
    assert!(
        e.contains(&*p.to_string_lossy()) && e.contains("pipe"),
        "{e}"
    );
    assert_eq!(listing(&fx.r1), src_before);
    assert!(diff(&before, &fx.tree()).is_empty());
    // `~/Hollow/Projects` is still the link to R1: nothing was copied.
    assert!(std::fs::symlink_metadata(fx.hollow.join("Projects"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(!mp::sidecar(&fx.hollow.join("Projects"), ".move-home-tmp").exists());
}

#[test]
fn move_home_fix_special_files_scan_and_copy_pieces() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let src = root.join("src");
    w(&src.join("a.txt"), "a");
    stale_socket(&src.join(".git/fsmonitor--daemon.ipc"));
    let before = listing(&src);
    let found = crate::move_home_icloud::special_files(&src).unwrap();
    assert_eq!(
        found.throwaway,
        vec![src.join(".git/fsmonitor--daemon.ipc")]
    );
    assert!(found.unexpected.is_empty() && found.refusal().is_none());
    let dst = root.join("dst");
    crate::move_home_icloud::clone_tree(&src, &dst).unwrap();
    assert_eq!(
        crate::move_home_icloud::drop_throwaway_specials(&dst).unwrap(),
        1
    );
    // Compared without it, both ways; the source still has it.
    let st = crate::move_home_icloud::compare_trees(&src, &dst, false).unwrap();
    assert_eq!(st.files, 1);
    assert_eq!(listing(&src), before);
    fifo(&src.join("p"));
    let found = crate::move_home_icloud::special_files(&src).unwrap();
    let why = found.refusal().unwrap();
    assert!(
        why.contains(&*src.join("p").to_string_lossy()) && why.contains("pipe"),
        "{why}"
    );
}

// ── Rehearsal on a real-shaped database (by hand, scratch copy only) ─────

/// Run the Sessions re-key, exactly as the move's step does it (append-only
/// tables left out, aliases recorded in the same transaction), on a scratch
/// copy of a real Sessions database with a real plan, then read the audit
/// record back through the aliases. Run by hand:
/// `HOLLOW_REKEY_DB=<scratch copy> HOLLOW_REKEY_PLAN=<plan.json> cargo test
/// --lib move_home_fix_rekey_on_a_scratch_copy -- --ignored --nocapture`.
/// It refuses a database under the live app data folder.
#[test]
#[ignore = "re-keys a scratch copy of a real Sessions database; run by hand with HOLLOW_REKEY_DB and HOLLOW_REKEY_PLAN"]
fn move_home_fix_rekey_on_a_scratch_copy() {
    let db = PathBuf::from(std::env::var("HOLLOW_REKEY_DB").expect("HOLLOW_REKEY_DB"));
    let plan = read_plan(Path::new(
        &std::env::var("HOLLOW_REKEY_PLAN").expect("HOLLOW_REKEY_PLAN"),
    ));
    if let Some(data) = dirs::data_dir() {
        assert!(
            !db.starts_with(data.join("Hollow")),
            "never the live store: {}",
            db.display()
        );
    }
    let maps = rekey::sorted_mappings(&plan.mappings);
    let ids: BTreeMap<String, String> = plan
        .ids
        .iter()
        .map(|i| (i.old.clone(), i.new.clone()))
        .collect();
    let started = std::time::Instant::now();
    let mut conn = Connection::open(&db).unwrap();
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .unwrap();
    let counts_before = mr::table_counts(&conn).unwrap();
    let audit_before = crate::audit::verify_in(&conn).unwrap();
    let kept = mp::kept_id_columns(&conn);
    let mut old_audit: BTreeMap<String, i64> = BTreeMap::new();
    for old in ids.keys() {
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM audit_log WHERE workspace_id = ?1",
                [old],
                |r| r.get(0),
            )
            .unwrap();
        if n > 0 {
            old_audit.insert(old.clone(), n);
        }
    }
    let (cols, missing) = mp::rewrite_columns(&conn);
    assert!(!cols.iter().any(|(t, _, _)| t == "audit_log"));
    let tx = conn.transaction().unwrap();
    let mut rewritten = BTreeMap::new();
    for (table, column, kind) in &cols {
        let n = mr::rewrite_column(&tx, table, column, *kind, &maps, &ids)
            .unwrap_or_else(|e| panic!("{table}.{column}: {e}"));
        if n > 0 {
            rewritten.insert(format!("{table}.{column}"), n);
        }
    }
    let aliases = crate::workspace_aliases::record(&tx, &ids, "rehearsal", "now").unwrap();
    tx.commit().unwrap();
    let check: String = conn
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(check, "ok");
    let audit_after = crate::audit::verify_in(&conn).unwrap();
    assert_eq!(
        (audit_before.ok, audit_before.rows, &audit_before.first_bad),
        (audit_after.ok, audit_after.rows, &audit_after.first_bad)
    );
    let mut counts_after = mr::table_counts(&conn).unwrap();
    assert_eq!(
        counts_after.remove("workspace_aliases").unwrap_or(0),
        counts_before.get("workspace_aliases").copied().unwrap_or(0) + aliases
    );
    let mut before = counts_before.clone();
    before.remove("workspace_aliases");
    assert_eq!(before, counts_after);
    // Every old id's audit rows are found by its new id.
    let a = crate::workspace_aliases::Aliases::load(&conn);
    for (old, n) in &old_audit {
        let new = &ids[old];
        let want = a.ids_for(new);
        let sql = format!(
            "SELECT count(*) FROM audit_log WHERE workspace_id IN ({})",
            crate::workspace_aliases::placeholders(want.len())
        );
        let got: i64 = conn
            .query_row(&sql, rusqlite::params_from_iter(want.iter()), |r| r.get(0))
            .unwrap();
        assert!(got >= *n, "{old} -> {new}: {got} < {n}");
    }
    println!(
        "rehearsal: {} ms; {} ids ({} recorded as aliases); kept append-only id columns {:?}; audit rows under old ids {:?}; audit chain ok={} rows={}; rows rewritten {:?}; columns not in this database {:?}",
        started.elapsed().as_millis(),
        ids.len(),
        aliases,
        kept,
        old_audit,
        audit_after.ok,
        audit_after.rows,
        rewritten,
        missing
    );
}
