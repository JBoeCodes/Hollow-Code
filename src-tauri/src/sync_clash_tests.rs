//! Tests for the clash cards of the context sync engine (CX6c): temp
//! folders, local bare repositories as remotes, no network.

use super::clash::{self, ClashChoice};
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

    fn remote(&self, name: &str) -> PathBuf {
        let path = self.base.join(format!("{name}.git"));
        let out = Command::new("git")
            .args(["init", "--quiet", "--bare", "--initial-branch=main"])
            .arg(&path)
            .env_remove("GIT_DIR")
            .output()
            .unwrap();
        assert!(out.status.success());
        path
    }

    fn config(kind: RootKind, remote: &Path) -> RootConfig {
        let id = match kind {
            RootKind::Vault => VAULT,
            _ => PERSONAL,
        };
        RootConfig {
            id: id.into(),
            kind,
            // The vault root takes a remote only with V6; tests give it one
            // below, as V6 will.
            remote: (kind == RootKind::Personal).then(|| remote.to_str().unwrap().to_string()),
            branch: "main".into(),
            enabled: true,
        }
    }

    /// One Mac's root of `kind`, syncing to `remote`.
    fn mac(&self, name: &str, kind: RootKind, remote: &Path) -> Root {
        let vault = self.base.join(name).join("vault");
        fs::create_dir_all(&vault).unwrap();
        let sync = self.base.join(name).join("app").join(SYNC_DIR);
        let mut root = Root::from_config(&Lab::config(kind, remote), &sync, &vault).unwrap();
        root.remote = Some(remote.to_str().unwrap().to_string());
        root
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
    let ctx = CycleCtx {
        machine: machine.into(),
        actor: "test".into(),
        allow_deletions: false,
    };
    let mut alerts = Vec::new();
    let outcome = cycle(root, &ctx, &mut |a| alerts.push(a));
    (outcome, alerts)
}

fn remote_text(remote: &Path, rel: &str) -> Option<String> {
    let out = Command::new("git")
        .arg("--git-dir")
        .arg(remote)
        .args(["show", &format!("main:{rel}")])
        .output()
        .unwrap();
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Two Macs of `kind` that changed the same line of `rel`: B holds the
/// clash (its version at `rel`, A's beside it). Returns (A, B, remote, copy).
fn clash_on(lab: &Lab, kind: RootKind, rel: &str) -> (Root, Root, PathBuf, String) {
    let remote = lab.remote("r");
    let a = lab.mac("a", kind, &remote);
    let b = lab.mac("b", kind, &remote);
    write(&a, rel, "title: one\nshared line\n");
    sync(&a, "mac-a");
    sync(&b, "mac-b");
    write(&a, rel, "title: from a\nshared line\n");
    write(&b, rel, "title: from b\nshared line\n");
    assert_eq!(sync(&a, "mac-a").0, Outcome::Synced { committed: true });
    let (outcome, alerts) = sync(&b, "mac-b");
    let copy = copy_name(rel, "mac-a", &|_| false);
    assert_eq!(
        outcome,
        Outcome::Clash {
            copies: vec![copy.clone()]
        }
    );
    // No error notification: the clash is a card of its own.
    assert!(alerts.is_empty(), "{alerts:?}");
    (a, b, remote, copy)
}

#[test]
fn a_planted_clash_on_the_personal_root_makes_one_card() {
    let lab = Lab::new();
    let (_a, b, _r, copy) = clash_on(&lab, RootKind::Personal, "me/Identity.md");
    let open = clash::open(&b);
    assert_eq!(open.len(), 1);
    let rec = &open[0];
    assert_eq!(rec.path, "me/Identity.md");
    assert_eq!(rec.copy, copy);
    assert_eq!(
        (rec.mine_machine.as_str(), rec.theirs_machine.as_str()),
        ("mac-b", "mac-a")
    );
    assert!(rec.replaceable && !rec.editing);
    assert_eq!(rec.mine_hash, clash::hash(b"title: from b\nshared line\n"));
    assert_eq!(
        rec.theirs_hash,
        clash::hash(b"title: from a\nshared line\n")
    );

    // The person's card: both versions' first lines and what differs.
    let card = clash::card(&b, rec, true);
    assert_eq!(card.root, "personal");
    assert_eq!(
        card.mine.as_ref().unwrap().lines,
        ["title: from b", "shared line"]
    );
    assert_eq!(
        card.theirs.as_ref().unwrap().lines,
        ["title: from a", "shared line"]
    );
    assert_eq!((card.only_mine, card.only_theirs), (1, 1));
    assert!(!card.changed);
    // The agents' read: paths only, no line of text.
    let bare = clash::card(&b, rec, false);
    assert!(bare.mine.is_none() && bare.theirs.is_none());
    assert_eq!(status_of(&b).clashes, 1);
}

#[test]
fn a_planted_clash_on_the_vault_root_makes_one_card() {
    let lab = Lab::new();
    let (_a, b, _r, copy) = clash_on(&lab, RootKind::Vault, "library/plan.md");
    let open = clash::open(&b);
    assert_eq!(open.len(), 1);
    assert_eq!(
        (open[0].path.as_str(), open[0].copy.as_str()),
        ("library/plan.md", copy.as_str())
    );
    assert_eq!(clash::card(&b, &open[0], true).root, "vault");
    // Keep mine works the same on this root.
    let done = clash::resolve(&b, "library/plan.md", &copy, ClashChoice::KeepMine).unwrap();
    assert!(!done.open);
    assert!(!b.work_tree.join(&copy).exists());
    assert!(clash::open(&b).is_empty());
}

#[test]
fn keep_mine_trashes_the_copy_and_the_next_cycle_commits_it() {
    let lab = Lab::new();
    let (a, b, remote, copy) = clash_on(&lab, RootKind::Personal, "memory/x.md");
    let done = clash::resolve(&b, "memory/x.md", &copy, ClashChoice::KeepMine).unwrap();
    assert_eq!(done.path, "memory/x.md");
    assert!(!b.work_tree.join(&copy).exists());
    assert_eq!(read(&b, "memory/x.md"), "title: from b\nshared line\n");
    // Nothing deleted outright: the copy is in the vault's trash.
    let trashed = fs::read_dir(b.work_tree.join(".trash")).unwrap().count();
    assert_eq!(trashed, 1);
    assert!(clash::open(&b).is_empty());
    assert_eq!(status_of(&b).state, SyncState::Idle);
    // An ordinary local change: committed, pushed and taken in by A.
    assert_eq!(sync(&b, "mac-b").0, Outcome::Synced { committed: true });
    assert!(remote_text(&remote, &copy).is_none());
    assert_eq!(sync(&a, "mac-a").0, Outcome::Synced { committed: false });
    assert!(!a.work_tree.join(&copy).exists());
}

#[test]
fn keep_theirs_takes_the_copy_and_refuses_a_changed_file() {
    let lab = Lab::new();
    let (_a, b, remote, copy) = clash_on(&lab, RootKind::Personal, "learnings/l.md");

    // The file changed after the clash: refused, nothing moved.
    write(&b, "learnings/l.md", "edited since\n");
    let err = clash::resolve(&b, "learnings/l.md", &copy, ClashChoice::KeepTheirs).unwrap_err();
    assert!(err.contains("changed since the clash"), "{err}");
    assert_eq!(read(&b, "learnings/l.md"), "edited since\n");
    assert!(b.work_tree.join(&copy).exists());
    assert_eq!(clash::open(&b).len(), 1);
    assert!(clash::card(&b, &clash::open(&b)[0], true).changed);

    // Back as recorded: the copy takes the name, the old text goes to trash.
    write(&b, "learnings/l.md", "title: from b\nshared line\n");
    clash::resolve(&b, "learnings/l.md", &copy, ClashChoice::KeepTheirs).unwrap();
    assert_eq!(read(&b, "learnings/l.md"), "title: from a\nshared line\n");
    assert!(!b.work_tree.join(&copy).exists());
    assert!(clash::open(&b).is_empty());
    assert_eq!(sync(&b, "mac-b").0, Outcome::Synced { committed: true });
    assert_eq!(
        remote_text(&remote, "learnings/l.md").as_deref(),
        Some("title: from a\nshared line\n")
    );
}

#[test]
fn keep_both_and_edit_then_done_leave_both_files() {
    let lab = Lab::new();
    let (_a, b, _r, copy) = clash_on(&lab, RootKind::Personal, "me/a.md");
    clash::resolve(&b, "me/a.md", &copy, ClashChoice::KeepBoth).unwrap();
    assert!(b.work_tree.join(&copy).exists());
    assert_eq!(read(&b, "me/a.md"), "title: from b\nshared line\n");
    assert!(clash::open(&b).is_empty());
    // The kept copy no longer reads as a clash.
    assert_eq!(status_of(&b).state, SyncState::Idle);
    assert_eq!(sync(&b, "mac-b").0, Outcome::Synced { committed: false });

    let lab = Lab::new();
    let (_a, b, _r, copy) = clash_on(&lab, RootKind::Personal, "me/a.md");
    let editing = clash::resolve(&b, "me/a.md", &copy, ClashChoice::Edit).unwrap();
    assert!(editing.open);
    assert_eq!(editing.path, "me/a.md");
    assert!(clash::open(&b)[0].editing);
    clash::resolve(&b, "me/a.md", &copy, ClashChoice::Done).unwrap();
    assert!(clash::open(&b).is_empty());
    assert!(b.work_tree.join(&copy).exists());

    // A copy removed by hand closes its card too.
    let lab = Lab::new();
    let (_a, b, _r, copy) = clash_on(&lab, RootKind::Personal, "me/a.md");
    fs::remove_file(b.work_tree.join(&copy)).unwrap();
    assert!(clash::open(&b).is_empty());
    sync(&b, "mac-b");
    assert!(load_state(&b).clashes.is_empty(), "the record is dropped");
}

#[test]
fn paths_roots_and_choices_outside_the_card_are_refused() {
    let lab = Lab::new();
    let (_a, b, _r, copy) = clash_on(&lab, RootKind::Personal, "me/a.md");
    for bad in [
        "../a.md",
        "me/../../a.md",
        "/etc/passwd",
        "",
        "me/./a.md",
        "notes/a.md",
    ] {
        assert!(
            clash::resolve(&b, bad, &copy, ClashChoice::KeepMine).is_err(),
            "{bad}"
        );
    }
    // A copy name is not a card's file.
    assert!(clash::resolve(&b, &copy, &copy, ClashChoice::KeepMine).is_err());
    // The command's root id and choice are checked before any file.
    let vault = b.work_tree.clone();
    assert!(resolve_clash_in(&vault, "../personal", "me/a.md", &copy, "keep_mine").is_err());
    assert!(resolve_clash_in(&vault, "personal", "me/a.md", &copy, "delete_all").is_err());
    assert!(b.work_tree.join(&copy).exists());
    assert_eq!(clash::open(&b).len(), 1);

    // A symlink where the copy was is never followed.
    let outside = lab.base.join("outside.md");
    fs::write(&outside, "keep me\n").unwrap();
    fs::remove_file(b.work_tree.join(&copy)).unwrap();
    std::os::unix::fs::symlink(&outside, b.work_tree.join(&copy)).unwrap();
    assert!(clash::resolve(&b, "me/a.md", &copy, ClashChoice::KeepTheirs).is_err());
    assert_eq!(fs::read_to_string(&outside).unwrap(), "keep me\n");
    assert_eq!(read(&b, "me/a.md"), "title: from b\nshared line\n");
}

#[test]
fn a_clash_survives_a_restart() {
    let lab = Lab::new();
    let (_a, b, remote, copy) = clash_on(&lab, RootKind::Personal, "me/a.md");
    drop(b);
    // A new process: the same config read again, nothing held in memory.
    let again = lab.mac("b", RootKind::Personal, &remote);
    let open = clash::open(&again);
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].copy, copy);
    assert_eq!(status_of(&again).clashes, 1);
}

#[test]
fn an_open_clash_never_stops_other_files_syncing() {
    let lab = Lab::new();
    let (a, b, remote, _copy) = clash_on(&lab, RootKind::Personal, "me/a.md");
    write(&b, "memory/other.md", "new on b\n");
    assert_eq!(sync(&b, "mac-b").0, Outcome::Synced { committed: true });
    assert_eq!(
        remote_text(&remote, "memory/other.md").as_deref(),
        Some("new on b\n")
    );
    write(&a, "home/a.md", "new on a\n");
    assert_eq!(sync(&a, "mac-a").0, Outcome::Synced { committed: true });
    assert_eq!(sync(&b, "mac-b").0, Outcome::Synced { committed: false });
    assert_eq!(read(&b, "home/a.md"), "new on a\n");
    // Still open, still one card.
    assert_eq!(clash::open(&b).len(), 1);
    assert_eq!(status_of(&b).state, SyncState::Clash);
}

/// What the vault's conflict copies item would list, leaving out `root`'s
/// clash copies as Waiting on you does.
fn copies_item(root: &Root) -> Option<(String, String)> {
    let skip: BTreeSet<String> = clash::skip_copies(root).into_iter().collect();
    crate::conflict_copies::waiting(&root.work_tree, &skip)
}

#[test]
fn a_kept_copy_is_never_offered_again_as_a_conflict_copy() {
    // Keep both.
    let lab = Lab::new();
    let (_a, b, _r, copy) = clash_on(&lab, RootKind::Personal, "me/a.md");
    // Open: the card holds it, the conflict copies item doesn't.
    assert!(copies_item(&b).is_none());
    assert!(crate::conflict_copies::waiting(&b.work_tree, &BTreeSet::new()).is_some());
    clash::resolve(&b, "me/a.md", &copy, ClashChoice::KeepBoth).unwrap();
    assert!(b.work_tree.join(&copy).exists());
    assert!(copies_item(&b).is_none(), "kept both: not offered again");
    sync(&b, "mac-b");
    assert!(copies_item(&b).is_none(), "a cycle keeps the mark");

    // Edit, then Done.
    let lab = Lab::new();
    let (_a, b, _r, copy) = clash_on(&lab, RootKind::Personal, "me/a.md");
    clash::resolve(&b, "me/a.md", &copy, ClashChoice::Edit).unwrap();
    assert!(copies_item(&b).is_none());
    clash::resolve(&b, "me/a.md", &copy, ClashChoice::Done).unwrap();
    assert!(copies_item(&b).is_none(), "done: not offered again");
    // Once the copy is gone the mark goes too.
    fs::remove_file(b.work_tree.join(&copy)).unwrap();
    sync(&b, "mac-b");
    assert!(load_state(&b).resolved_copies.is_empty());
}

#[test]
fn a_clash_is_recorded_even_when_the_push_fails() {
    use std::os::unix::fs::PermissionsExt;
    let lab = Lab::new();
    let remote = lab.remote("r");
    let a = lab.mac("a", RootKind::Personal, &remote);
    let b = lab.mac("b", RootKind::Personal, &remote);
    write(&a, "me/a.md", "title: one\nshared line\n");
    sync(&a, "mac-a");
    sync(&b, "mac-b");
    write(&a, "me/a.md", "title: from a\nshared line\n");
    write(&b, "me/a.md", "title: from b\nshared line\n");
    sync(&a, "mac-a");
    // The remote refuses every push now.
    let hook = remote.join("hooks").join("pre-receive");
    fs::create_dir_all(hook.parent().unwrap()).unwrap();
    fs::write(&hook, "#!/bin/sh\necho refused >&2\nexit 1\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let (outcome, _) = sync(&b, "mac-b");
    assert!(
        !matches!(outcome, Outcome::Synced { .. } | Outcome::Clash { .. }),
        "{outcome:?}"
    );
    // The merge was written: its card is saved, read back from disk.
    assert_eq!(load_state(&b).clashes.len(), 1);
    assert_eq!(clash::open(&b).len(), 1);
    // The remote takes pushes again: the next cycle pushes, never doubles it.
    fs::remove_file(&hook).unwrap();
    assert!(matches!(sync(&b, "mac-b").0, Outcome::Synced { .. }));
    assert!(matches!(sync(&b, "mac-b").0, Outcome::Synced { .. }));
    assert_eq!(load_state(&b).clashes.len(), 1);
}

#[test]
fn an_edited_copy_is_never_trashed_or_promoted() {
    let lab = Lab::new();
    let (_a, b, _r, copy) = clash_on(&lab, RootKind::Personal, "me/a.md");
    write(&b, &copy, "edited copy\n");
    for choice in [ClashChoice::KeepMine, ClashChoice::KeepTheirs] {
        let err = clash::resolve(&b, "me/a.md", &copy, choice).unwrap_err();
        assert!(
            err.contains("changed since the clash") && err.contains(&copy),
            "{err}"
        );
    }
    assert_eq!(read(&b, &copy), "edited copy\n");
    assert_eq!(read(&b, "me/a.md"), "title: from b\nshared line\n");
    assert!(!b.work_tree.join(".trash").exists());
    assert_eq!(clash::open(&b).len(), 1);
    // A record is found by its file and its copy together.
    let other = "me/a (from mac-c).md";
    write(&b, other, "x\n");
    let err = clash::resolve(&b, "me/a.md", other, ClashChoice::KeepBoth).unwrap_err();
    assert!(err.contains("no open clash"), "{err}");
}

#[test]
fn the_agents_list_carries_no_line_of_any_file_and_resolving_is_the_persons() {
    let lab = Lab::new();
    let (_a, b, _r, _copy) = clash_on(&lab, RootKind::Personal, "me/a.md");
    let roots = [b.clone()];
    let agents = serde_json::to_string(&clash_list_of(&roots, false)).unwrap();
    assert!(agents.contains("me/a.md"), "{agents}");
    for text in ["title:", "shared line"] {
        assert!(!agents.contains(text), "{agents}");
    }
    let person = serde_json::to_string(&clash_list_of(&roots, true)).unwrap();
    assert!(person.contains("shared line"));
    // Choosing is user-only, with its reason, and never an action.
    assert!(crate::app_actions::USER_ONLY.contains(&"resolve_sync_clash"));
    assert!(crate::app_actions::COO_EXCEPTIONS
        .iter()
        .any(|(c, _)| *c == "resolve_sync_clash"));
}
