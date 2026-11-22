//! The `automated_history` unit tests (split out of `automated_history.rs` by
//! B13, 2026-10-16, no behaviour change). Attached from `automated_history.rs`
//! as `#[path] mod tests`, so `super::` is still `crate::automated_history`.

use super::*;
use tempfile::TempDir;

fn vault() -> TempDir {
    TempDir::new().unwrap()
}

fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

#[test]
fn diff_line_counts_match_a_line_diff() {
    assert_eq!(diff_line_counts("", "a\nb\n"), (2, 0));
    assert_eq!(diff_line_counts("a\nb\n", ""), (0, 2));
    assert_eq!(diff_line_counts("a\nb\nc\n", "a\nb\nc\n"), (0, 0));
    // One line changed in the middle: one added, one removed.
    assert_eq!(diff_line_counts("a\nb\nc\n", "a\nB\nc\n"), (1, 1));
    // An insertion and a deletion apart from each other.
    assert_eq!(diff_line_counts("a\nb\nc\nd\n", "a\nx\nb\nd\n"), (1, 1));
    // A moved line is one removal and one addition (exact LCS).
    assert_eq!(diff_line_counts("a\nb\nc\n", "b\nc\na\n"), (1, 1));
}

// Review of merged plan slice 4: a full-vault baseline writes one object
// per file at run start, so objects take a plain fsync and share one
// flush of the objects directory, which lands before the manifest that
// names them is written (with its own full sync).
#[test]
fn baseline_objects_are_flushed_once_before_the_manifest() {
    let dir = vault();
    for name in ["a", "b", "c"] {
        write(dir.path().join(format!("notes/{name}.md")), name);
    }
    let objects = objects_root(dir.path());
    let runs = runs_root(dir.path());
    let (handle, steps) =
        crate::vault::capture_sync_trace(|| begin_run(dir.path(), "chat", None).unwrap());
    let under = |step: &str, kind: &str, root: &Path| {
        step.strip_prefix(kind)
            .map(|p| Path::new(p).parent() == Some(root))
            .unwrap_or(false)
    };
    let object_syncs: Vec<usize> = (0..steps.len())
        .filter(|&i| under(&steps[i], "fsync-file ", &objects))
        .collect();
    assert_eq!(object_syncs.len(), 3, "one fsync per object: {steps:?}");
    assert!(
        !steps.iter().any(|s| under(s, "sync-file ", &objects)),
        "objects never take a full sync each: {steps:?}"
    );
    let flush = format!("sync-dir {}", objects.display());
    let flushes: Vec<usize> = (0..steps.len()).filter(|&i| steps[i] == flush).collect();
    assert_eq!(flushes.len(), 1, "one objects flush: {steps:?}");
    let manifest_sync = (0..steps.len())
        .find(|&i| under(&steps[i], "sync-file ", &runs))
        .expect("the manifest takes a full sync");
    let last_object_rename = (0..steps.len())
        .rfind(|&i| under(&steps[i], "rename ", &objects))
        .unwrap();
    assert!(
        *object_syncs.last().unwrap() < last_object_rename
            && last_object_rename < flushes[0]
            && flushes[0] < manifest_sync,
        "objects, then their flush, then the manifest: {steps:?}"
    );
    finish_run(dir.path(), handle, "success").unwrap();
}

#[test]
fn line_stats_reads_history_objects_and_refuses_binary() {
    let dir = vault();
    let before = b"one\ntwo\n";
    let after = b"one\n2\nthree\n";
    let (hb, ha) = (hash_bytes(before), hash_bytes(after));
    write_object(dir.path(), &hb, before).unwrap();
    write_object(dir.path(), &ha, after).unwrap();
    assert_eq!(line_stats(dir.path(), Some(&hb), Some(&ha)), Some((2, 1)));
    assert_eq!(line_stats(dir.path(), None, Some(&ha)), Some((3, 0)));
    assert_eq!(line_stats(dir.path(), Some(&hb), None), Some((0, 2)));
    let binary = [0xffu8, 0xfe, 0x00];
    let hx = hash_bytes(&binary);
    write_object(dir.path(), &hx, &binary).unwrap();
    assert_eq!(line_stats(dir.path(), Some(&hb), Some(&hx)), None);
    assert_eq!(
        line_stats(dir.path(), Some(&"0".repeat(64)), Some(&ha)),
        None
    );
}

fn mixed_run(dir: &TempDir) -> HistoryRun {
    write(dir.path().join("notes/modified.md"), "before modified");
    write(dir.path().join("notes/deleted.md"), "before deleted");

    let handle = begin_run(dir.path(), "workflow", Some("mixed-run")).unwrap();
    write(dir.path().join("notes/modified.md"), "after modified");
    write(dir.path().join("notes/created.md"), "after created");
    fs::remove_file(dir.path().join("notes/deleted.md")).unwrap();
    finish_run(dir.path(), handle, "success").unwrap()
}

fn assert_mixed_after_state(dir: &TempDir) {
    assert_eq!(
        fs::read_to_string(dir.path().join("notes/created.md")).unwrap(),
        "after created"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("notes/modified.md")).unwrap(),
        "after modified"
    );
    assert!(!dir.path().join("notes/deleted.md").exists());
}

fn assert_mixed_before_state(dir: &TempDir) {
    assert!(!dir.path().join("notes/created.md").exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("notes/modified.md")).unwrap(),
        "before modified"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("notes/deleted.md")).unwrap(),
        "before deleted"
    );
}

#[test]
fn records_create_modify_delete_and_reads_versions() {
    let dir = vault();
    write(dir.path().join("notes/modify.md"), "before");
    write(dir.path().join("notes/delete.md"), "gone later");

    let handle = begin_run(dir.path(), "chat", Some("session-1")).unwrap();
    write(dir.path().join("notes/modify.md"), "after");
    write(dir.path().join("notes/create.md"), "new");
    fs::remove_file(dir.path().join("notes/delete.md")).unwrap();
    let run = finish_run(dir.path(), handle, "success").unwrap();

    assert_eq!(run.status, "success");
    assert_eq!(run.changes.len(), 3);
    let kinds: BTreeMap<_, _> = run
        .changes
        .iter()
        .map(|c| (c.vault_path.as_str(), c.change_type.as_str()))
        .collect();
    assert_eq!(kinds.get("notes/create.md"), Some(&"create"));
    assert_eq!(kinds.get("notes/delete.md"), Some(&"delete"));
    assert_eq!(kinds.get("notes/modify.md"), Some(&"modify"));
    assert_eq!(
        read_version(dir.path(), &run.id, "notes/modify.md", "before")
            .unwrap()
            .as_deref(),
        Some("before")
    );
    assert_eq!(
        read_version(dir.path(), &run.id, "notes/modify.md", "after")
            .unwrap()
            .as_deref(),
        Some("after")
    );
    assert_eq!(
        read_version(dir.path(), &run.id, "notes/create.md", "before").unwrap(),
        None
    );
    assert_eq!(
        read_version(dir.path(), &run.id, "notes/delete.md", "after").unwrap(),
        None
    );
}

#[test]
fn baseline_paths_lists_what_existed_when_the_active_run_began() {
    let dir = vault();
    write(dir.path().join("notes/existing.md"), "before");
    write(dir.path().join(".state.json"), "{}");
    let handle = begin_run(dir.path(), "ingest", None).unwrap();
    write(dir.path().join("notes/created.md"), "during");
    let paths = baseline_paths(dir.path(), &handle).unwrap();
    assert!(paths.contains("notes/existing.md"));
    assert!(
        !paths.contains("notes/created.md"),
        "created after the run began"
    );
    assert!(
        !paths.contains(".state.json"),
        "hidden files are never tracked"
    );
    let finished = HistoryRunHandle {
        id: handle.id.clone(),
        baseline_ms: None,
    };
    finish_run(dir.path(), handle, "success").unwrap();
    assert!(
        baseline_paths(dir.path(), &finished).is_err(),
        "a finished run keeps no baseline"
    );
}

#[test]
fn records_notion_mirror_metadata_from_both_versions() {
    let dir = vault();
    let path = dir.path().join("tasks/mirror.md");
    write(
        &path,
        "---\nsource: notion\nnotion_id: page-before\n---\n\nBefore\n",
    );
    write(dir.path().join("notes/local.md"), "local before");

    let handle = begin_run(dir.path(), "chat", None).unwrap();
    write(
        &path,
        "---\nsource: notion\nnotion_id: page-after\n---\n\nAfter\n",
    );
    write(dir.path().join("notes/local.md"), "local after");
    let run = finish_run(dir.path(), handle, "success").unwrap();

    let mirror = run
        .changes
        .iter()
        .find(|change| change.vault_path == "tasks/mirror.md")
        .unwrap();
    assert!(mirror.notion_mirror);
    assert_eq!(mirror.before_notion_id.as_deref(), Some("page-before"));
    assert_eq!(mirror.after_notion_id.as_deref(), Some("page-after"));

    let local = run
        .changes
        .iter()
        .find(|change| change.vault_path == "notes/local.md")
        .unwrap();
    assert!(!local.notion_mirror);
    assert!(local.before_notion_id.is_none());
    assert!(local.after_notion_id.is_none());
}

/// Hidden paths and folders are never History's. An oversized file the
/// run rewrote used to vanish from the diff too; since merged plan slice
/// 4 it is a change History can't restore, so the run isn't fully
/// recoverable. A link the run left alone is no change.
#[test]
fn ignores_hidden_paths_and_records_what_it_cannot_capture() {
    let dir = vault();
    write(dir.path().join("notes/kept.md"), "a");
    write(dir.path().join(".obsidian/private.md"), "secret");
    write(dir.path().join("notes/.hidden.md"), "secret");
    write(
        dir.path().join("notes/large.bin"),
        vec![b'x'; MAX_ELIGIBLE_FILE_BYTES as usize + 1],
    );
    fs::create_dir_all(dir.path().join("notes/folder.md")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        dir.path().join("notes/kept.md"),
        dir.path().join("notes/link.md"),
    )
    .unwrap();

    let handle = begin_run(dir.path(), "workflow", None).unwrap();
    write(dir.path().join("notes/kept.md"), "b");
    write(dir.path().join(".obsidian/private.md"), "changed");
    write(dir.path().join("notes/.hidden.md"), "changed");
    write(
        dir.path().join("notes/large.bin"),
        vec![b'y'; MAX_ELIGIBLE_FILE_BYTES as usize + 2],
    );
    let run = finish_run(dir.path(), handle, "success").unwrap();
    let paths: Vec<&str> = run.changes.iter().map(|c| c.vault_path.as_str()).collect();
    assert_eq!(paths, ["notes/kept.md", "notes/large.bin"]);
    assert!(run.changes[0].recoverable());
    let large = &run.changes[1];
    assert_eq!(large.change_type, "modify");
    let too_large = Coverage::Uncaptured {
        reason: Some(UncapturedReason::TooLarge),
    };
    assert_eq!(large.coverage_before(), too_large);
    assert_eq!(large.coverage_after(), too_large);
    assert!(!run.fully_recoverable());
}

#[test]
fn binary_versions_restore_but_do_not_coerce_to_text() {
    let dir = vault();
    write(dir.path().join("notes/blob.dat"), [0xff, 0x00, 0x80]);
    let handle = begin_run(dir.path(), "chat", None).unwrap();
    write(dir.path().join("notes/blob.dat"), [0x01, 0x02]);
    let run = finish_run(dir.path(), handle, "success").unwrap();
    assert!(!run.changes[0].text);
    assert!(
        read_version(dir.path(), &run.id, "notes/blob.dat", "before")
            .unwrap_err()
            .contains("binary")
    );
    restore_change(dir.path(), &run.id, "notes/blob.dat", false).unwrap();
    assert_eq!(
        fs::read(dir.path().join("notes/blob.dat")).unwrap(),
        [0xff, 0x00, 0x80]
    );
}

#[test]
fn restore_modify_is_cas_guarded_and_force_is_explicit() {
    let dir = vault();
    let path = dir.path().join("notes/n.md");
    write(&path, "one");
    let handle = begin_run(dir.path(), "chat", None).unwrap();
    write(&path, "two");
    let run = finish_run(dir.path(), handle, "success").unwrap();
    write(&path, "external three");

    let err = restore_change(dir.path(), &run.id, "notes/n.md", false).unwrap_err();
    assert!(err.contains("conflict"));
    assert_eq!(fs::read_to_string(&path).unwrap(), "external three");

    let restore = restore_change(dir.path(), &run.id, "notes/n.md", true).unwrap();
    assert_eq!(restore.source, "historyRestore");
    assert_eq!(restore.source_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(fs::read_to_string(&path).unwrap(), "one");
    assert_eq!(restore.changes.len(), 1);
    let original = get_run(dir.path(), &run.id).unwrap();
    assert_eq!(
        original.changes[0].restored_by_run_id.as_deref(),
        Some(restore.id.as_str())
    );
}

#[test]
fn single_restore_rechecks_after_recovery_baseline_before_writing() {
    let dir = vault();
    let path = dir.path().join("notes/n.md");
    write(&path, "before");
    let handle = begin_run(dir.path(), "chat", None).unwrap();
    write(&path, "recorded after");
    let run = finish_run(dir.path(), handle, "success").unwrap();

    *RESTORE_APPLY_MUTATION
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = Some((
        dir.path().canonicalize().unwrap(),
        "notes/n.md".into(),
        b"racing edit".to_vec(),
    ));

    let error = restore_change(dir.path(), &run.id, "notes/n.md", false).unwrap_err();

    assert!(error.contains("changed after restore preflight"));
    assert_eq!(fs::read_to_string(path).unwrap(), "racing edit");
    let audit = read_all_manifests(dir.path())
        .unwrap()
        .into_iter()
        .find(|manifest| {
            manifest.run.source == "historyRestore"
                && manifest.run.source_id.as_deref() == Some(run.id.as_str())
        })
        .expect("the rejected restore remains auditable");
    assert_eq!(audit.run.status, "error");
}

#[test]
fn restore_create_moves_file_to_trash_and_restore_delete_recreates() {
    let dir = vault();
    let handle = begin_run(dir.path(), "workflow", None).unwrap();
    write(dir.path().join("notes/created.md"), "created");
    let created_run = finish_run(dir.path(), handle, "success").unwrap();
    restore_change(dir.path(), &created_run.id, "notes/created.md", false).unwrap();
    assert!(!dir.path().join("notes/created.md").exists());
    assert!(fs::read_dir(dir.path().join(".trash"))
        .unwrap()
        .next()
        .is_some());

    write(dir.path().join("notes/deleted.md"), "bring me back");
    let handle = begin_run(dir.path(), "workflow", None).unwrap();
    fs::remove_file(dir.path().join("notes/deleted.md")).unwrap();
    let deleted_run = finish_run(dir.path(), handle, "success").unwrap();
    restore_change(dir.path(), &deleted_run.id, "notes/deleted.md", false).unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("notes/deleted.md")).unwrap(),
        "bring me back"
    );
}

#[test]
fn restore_run_reverses_mixed_changes_and_annotates_every_source_change() {
    let dir = vault();
    let source = mixed_run(&dir);

    let restored = restore_run(dir.path(), &source.id, false).unwrap();

    assert_mixed_before_state(&dir);
    assert_eq!(restored.source, "historyRestore");
    assert_eq!(restored.source_id.as_deref(), Some(source.id.as_str()));
    assert_eq!(restored.status, "success");
    let inverse_kinds: BTreeMap<_, _> = restored
        .changes
        .iter()
        .map(|change| (change.vault_path.as_str(), change.change_type.as_str()))
        .collect();
    assert_eq!(inverse_kinds.get("notes/created.md"), Some(&"delete"));
    assert_eq!(inverse_kinds.get("notes/deleted.md"), Some(&"create"));
    assert_eq!(inverse_kinds.get("notes/modified.md"), Some(&"modify"));
    assert!(fs::read_dir(dir.path().join(".trash"))
        .unwrap()
        .next()
        .is_some());

    let annotated = get_run(dir.path(), &source.id).unwrap();
    assert_eq!(annotated.changes.len(), 3);
    assert!(annotated.changes.iter().all(|change| {
        change.restored_at.is_some()
            && change.restored_by_run_id.as_deref() == Some(restored.id.as_str())
    }));
    let timestamps: HashSet<_> = annotated
        .changes
        .iter()
        .filter_map(|change| change.restored_at.as_deref())
        .collect();
    assert_eq!(timestamps.len(), 1, "one audit stamp covers the whole run");
}

#[test]
fn restore_run_preflights_every_path_before_any_write_on_conflict() {
    let dir = vault();
    let source = mixed_run(&dir);
    write(dir.path().join("notes/modified.md"), "external edit");
    let manifest_count = read_all_manifests(dir.path()).unwrap().len();

    let error = restore_run(dir.path(), &source.id, false).unwrap_err();

    assert!(error.contains("conflict"));
    assert_eq!(
        fs::read_to_string(dir.path().join("notes/created.md")).unwrap(),
        "after created"
    );
    assert!(!dir.path().join("notes/deleted.md").exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("notes/modified.md")).unwrap(),
        "external edit"
    );
    assert_eq!(
        read_all_manifests(dir.path()).unwrap().len(),
        manifest_count,
        "preflight failure must not create a restore audit run"
    );
    assert!(!dir.path().join(".trash").exists());
}

#[test]
fn restore_run_loads_every_before_object_before_mutation() {
    let dir = vault();
    let source = mixed_run(&dir);
    let modified = source
        .changes
        .iter()
        .find(|change| change.vault_path == "notes/modified.md")
        .unwrap();
    let hash = modified.before_hash.as_deref().unwrap();
    write(object_path(dir.path(), hash).unwrap(), "corrupt");
    let manifest_count = read_all_manifests(dir.path()).unwrap().len();

    let error = restore_run(dir.path(), &source.id, true).unwrap_err();

    assert!(error.contains("history object"));
    assert_mixed_after_state(&dir);
    assert_eq!(
        read_all_manifests(dir.path()).unwrap().len(),
        manifest_count,
        "object preflight failure must not create a restore audit run"
    );
    assert!(!dir.path().join(".trash").exists());
}

#[test]
fn restore_run_force_bypasses_hash_conflicts() {
    let dir = vault();
    let source = mixed_run(&dir);
    write(dir.path().join("notes/modified.md"), "external edit");

    assert!(restore_run(dir.path(), &source.id, false)
        .unwrap_err()
        .contains("conflict"));
    let restored = restore_run(dir.path(), &source.id, true).unwrap();

    assert_mixed_before_state(&dir);
    assert_eq!(restored.source_id.as_deref(), Some(source.id.as_str()));
}

#[test]
fn a_whole_run_restore_is_itself_reversible() {
    let dir = vault();
    let source = mixed_run(&dir);
    let first_restore = restore_run(dir.path(), &source.id, false).unwrap();
    assert_mixed_before_state(&dir);

    let second_restore = restore_run(dir.path(), &first_restore.id, false).unwrap();

    assert_mixed_after_state(&dir);
    assert_eq!(
        second_restore.source_id.as_deref(),
        Some(first_restore.id.as_str())
    );
    let annotated_first_restore = get_run(dir.path(), &first_restore.id).unwrap();
    assert!(annotated_first_restore.changes.iter().all(|change| {
        change.restored_by_run_id.as_deref() == Some(second_restore.id.as_str())
            && change.restored_at.is_some()
    }));
}

#[test]
fn restore_run_rejects_unfinished_and_malformed_runs_without_writes() {
    let unfinished_dir = vault();
    write(unfinished_dir.path().join("notes/a.md"), "before");
    let unfinished = begin_run(unfinished_dir.path(), "workflow", None).unwrap();
    write(unfinished_dir.path().join("notes/a.md"), "after");
    let error = restore_run(unfinished_dir.path(), &unfinished.id, true).unwrap_err();
    assert!(error.contains("unfinished"));
    assert_eq!(
        fs::read_to_string(unfinished_dir.path().join("notes/a.md")).unwrap(),
        "after"
    );
    assert_eq!(read_all_manifests(unfinished_dir.path()).unwrap().len(), 1);

    let malformed_dir = vault();
    let source = mixed_run(&malformed_dir);
    let mut manifest = read_manifest(malformed_dir.path(), &source.id).unwrap();
    manifest.run.changes[0].vault_path = "../escape.md".into();
    write_manifest(malformed_dir.path(), &manifest).unwrap();
    let manifest_count = read_all_manifests(malformed_dir.path()).unwrap().len();
    let error = restore_run(malformed_dir.path(), &source.id, true).unwrap_err();
    assert!(error.contains("path escape"));
    assert_mixed_after_state(&malformed_dir);
    assert_eq!(
        read_all_manifests(malformed_dir.path()).unwrap().len(),
        manifest_count
    );

    let overlap_dir = vault();
    let source = mixed_run(&overlap_dir);
    let mut manifest = read_manifest(overlap_dir.path(), &source.id).unwrap();
    manifest.run.changes[0].vault_path = "notes".into();
    manifest.run.changes[1].vault_path = "notes/child.md".into();
    write_manifest(overlap_dir.path(), &manifest).unwrap();
    let error = restore_run(overlap_dir.path(), &source.id, true).unwrap_err();
    assert!(error.contains("overlapping paths"));
    assert_mixed_after_state(&overlap_dir);
}

#[cfg(unix)]
#[test]
fn restore_run_force_still_rejects_symlink_paths() {
    let dir = vault();
    let source = mixed_run(&dir);
    let target = dir.path().join("notes/modified.md");
    let other = dir.path().join("notes/other.md");
    write(&other, "do not touch");
    fs::remove_file(&target).unwrap();
    std::os::unix::fs::symlink(&other, &target).unwrap();
    let manifest_count = read_all_manifests(dir.path()).unwrap().len();

    let error = restore_run(dir.path(), &source.id, true).unwrap_err();

    assert!(error.contains("symlink"));
    assert_eq!(fs::read_to_string(&other).unwrap(), "do not touch");
    assert_eq!(
        fs::read_to_string(dir.path().join("notes/created.md")).unwrap(),
        "after created"
    );
    assert!(!dir.path().join("notes/deleted.md").exists());
    assert_eq!(
        read_all_manifests(dir.path()).unwrap().len(),
        manifest_count
    );
    assert!(!dir.path().join(".trash").exists());
}

#[test]
fn restore_run_rolls_back_applied_paths_and_finalizes_error_audit() {
    let dir = vault();
    let source = mixed_run(&dir);
    *RESTORE_APPLY_FAILURE
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = Some((
        dir.path().canonicalize().unwrap(),
        "notes/deleted.md".into(),
    ));

    let error = restore_run(dir.path(), &source.id, false).unwrap_err();
    *RESTORE_APPLY_FAILURE
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = None;

    assert!(error.contains("injected restore application failure"));
    assert_mixed_after_state(&dir);
    let manifests = read_all_manifests(dir.path()).unwrap();
    let audit = manifests
        .iter()
        .find(|manifest| {
            manifest.run.source == "historyRestore"
                && manifest.run.source_id.as_deref() == Some(source.id.as_str())
        })
        .expect("failed restore should retain an audit run");
    assert_eq!(audit.run.status, "error");
    assert!(audit.run.finished_at.is_some());
    assert!(
        audit.run.changes.is_empty(),
        "rollback restored the baseline"
    );
    let original = get_run(dir.path(), &source.id).unwrap();
    assert!(original
        .changes
        .iter()
        .all(|change| change.restored_at.is_none() && change.restored_by_run_id.is_none()));
}

#[test]
fn listing_is_newest_first_filtered_and_path_safe() {
    let dir = vault();
    write(dir.path().join("notes/a.md"), "0");
    let one = begin_run(dir.path(), "chat", Some("one")).unwrap();
    write(dir.path().join("notes/a.md"), "1");
    finish_run(dir.path(), one, "success").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let two = begin_run(dir.path(), "chat", Some("two")).unwrap();
    write(dir.path().join("notes/b.md"), "b");
    finish_run(dir.path(), two, "success").unwrap();

    let runs = list_runs(dir.path(), 1).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].source_id.as_deref(), Some("two"));
    let note = list_note_history(dir.path(), "notes/a.md", 10).unwrap();
    assert_eq!(note.len(), 1);
    assert_eq!(note[0].source_id.as_deref(), Some("one"));
    assert!(list_note_history(dir.path(), "../escape", 10).is_err());
    assert!(list_note_history(dir.path(), ".", 10).is_err());
    assert!(get_run(dir.path(), "../../escape").is_err());
}

#[cfg(unix)]
#[test]
fn restore_rejects_a_target_replaced_by_an_internal_symlink() {
    let dir = vault();
    let target = dir.path().join("notes/a.md");
    let other = dir.path().join("notes/other.md");
    write(&target, "before");
    write(&other, "do not touch");
    let handle = begin_run(dir.path(), "chat", None).unwrap();
    write(&target, "after");
    let run = finish_run(dir.path(), handle, "success").unwrap();

    fs::remove_file(&target).unwrap();
    std::os::unix::fs::symlink(&other, &target).unwrap();
    let error = restore_change(dir.path(), &run.id, "notes/a.md", true).unwrap_err();
    assert!(error.contains("symlink"));
    assert_eq!(fs::read_to_string(&other).unwrap(), "do not touch");
}

#[test]
fn reconcile_finalizes_interrupted_runs() {
    let dir = vault();
    write(dir.path().join("notes/a.md"), "before");
    let handle = begin_run(dir.path(), "heartbeat", None).unwrap();
    write(dir.path().join("notes/a.md"), "after crash");
    assert_eq!(reconcile_interrupted(dir.path()).unwrap(), 1);
    let run = get_run(dir.path(), &handle.id).unwrap();
    assert_eq!(run.status, "interrupted");
    assert!(run.finished_at.is_some());
    assert_eq!(run.changes.len(), 1);
    assert_eq!(reconcile_interrupted(dir.path()).unwrap(), 0);
}

#[test]
fn content_objects_are_deduplicated_and_status_reports_usage() {
    let dir = vault();
    write(dir.path().join("notes/a.md"), "same");
    write(dir.path().join("notes/b.md"), "same");
    let handle = begin_run(dir.path(), "chat", None).unwrap();
    let active = storage_status(dir.path()).unwrap();
    assert_eq!(active.active_run_count, 1);
    assert_eq!(active.object_count, 1, "same bytes share one object");
    write(dir.path().join("notes/a.md"), "changed");
    finish_run(dir.path(), handle, "success").unwrap();
    let status = storage_status(dir.path()).unwrap();
    assert_eq!(status.run_count, 1);
    assert_eq!(status.active_run_count, 0);
    assert!(status.total_bytes >= status.object_bytes);
    assert_eq!(status.max_runs, MAX_RUNS);
}

// Night ST6 (2026-10-14): iCloud made a conflict copy of the lease on
// almost every run. The lease is `active-run.nosync` (iCloud Drive skips
// `.nosync` names); a pre-ST6 `active-run` from an older build is honoured
// while its process holds the lease lock and cleared once it's free;
// leftover conflict copies go at startup and are never read as a lease.
#[test]
fn the_lease_is_never_synced_and_a_legacy_lease_is_honoured_only_while_held() {
    let dir = vault();
    let v = dir.path();
    write(v.join("notes/a.md"), "a");
    let run = begin_run(v, "chat", Some("first")).unwrap();
    assert!(history_root(v).join("active-run.nosync").is_file());
    assert!(!history_root(v).join("active-run").exists());
    finish_run(v, run, "success").unwrap();
    assert!(!history_root(v).join("active-run.nosync").exists());

    // An older build is mid-run: its lease lock is held.
    write(
        history_root(v).join("active-run"),
        "workflow-scheduler-legacy\n",
    );
    let lock = crate::lease_lock::hold(v).expect("lease lock");
    let refused = begin_run(v, "chat", Some("second")).unwrap_err();
    assert!(refused.starts_with("Hollow is busy"), "got {refused}");
    assert!(history_root(v).join("active-run").exists());
    // It ended without removing its lease: the lock is free, so it's cleared.
    drop(lock);
    let run = begin_run(v, "chat", Some("second")).unwrap();
    assert!(!history_root(v).join("active-run").exists());
    finish_run(v, run, "success").unwrap();

    // An older build that wrote its lease but hasn't taken the lock yet:
    // its pid is alive, so the lease is busy, not cleared (ST6 review).
    let mut older = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let record = format!(
        "{{\"owner\":\"workflow-scheduler-older\",\"pid\":{}}}\n",
        older.id()
    );
    write(history_root(v).join("active-run"), &record);
    let refused = begin_run(v, "chat", Some("third")).unwrap_err();
    assert!(refused.starts_with("Hollow is busy"), "got {refused}");
    // While it holds the lock too, startup spares it as the live owner.
    let lock = crate::lease_lock::hold(v).expect("lease lock");
    assert_eq!(
        foreign_live_owner(v).as_deref(),
        Some("workflow-scheduler-older")
    );
    drop(lock);
    older.kill().unwrap();
    older.wait().unwrap();
    let run = begin_run(v, "chat", Some("third")).unwrap();
    assert!(
        !history_root(v).join("active-run").exists(),
        "its owner exited"
    );
    finish_run(v, run, "success").unwrap();
}

#[test]
fn icloud_conflict_copies_of_the_lease_are_ignored_and_tidied_at_startup() {
    let dir = vault();
    let v = dir.path();
    write(v.join("notes/a.md"), "a");
    for name in ["active-run 2", "active-run 71", "active-run 3.nosync"] {
        write(history_root(v).join(name), "run-ghost\n");
    }
    write(history_root(v).join("active-run notes.md"), "not a copy");
    // Not a lease: nothing holds the vault.
    assert!(read_lease_record(v).is_none());
    let run = begin_run(v, "chat", Some("first")).unwrap();
    finish_run(v, run, "success").unwrap();
    reconcile_interrupted(v).unwrap();
    for name in ["active-run 2", "active-run 71", "active-run 3.nosync"] {
        assert!(!history_root(v).join(name).exists(), "{name}");
    }
    assert!(history_root(v).join("active-run notes.md").exists());
}

#[test]
fn a_second_full_vault_run_is_rejected_until_the_lease_is_released() {
    let dir = vault();
    write(dir.path().join("notes/a.md"), "before");
    let first = begin_run(dir.path(), "chat", Some("first")).unwrap();

    let error = begin_run(dir.path(), "workflow", Some("second")).unwrap_err();

    assert!(error.starts_with("Hollow is busy"), "got {error}");
    assert!(error.contains("a chat turn"), "got {error}");
    assert_eq!(storage_status(dir.path()).unwrap().active_run_count, 1);
    write(dir.path().join("notes/a.md"), "after");
    finish_run(dir.path(), first, "success").unwrap();

    let second = begin_run(dir.path(), "workflow", Some("second")).unwrap();
    finish_run(dir.path(), second, "success").unwrap();
}

// Wave 2 review nit 6 (closed as unreachable): the enforced automations,
// workflows and the heartbeat, each need the one full-vault lease, so two
// of them are never live at once and only one enforced Claude dollar hold
// exists in a process (budgets' `grant_spend`). If vault-lane automations
// ever run side by side, this fails and the dollar split needs revisiting.
#[test]
fn two_enforced_automations_never_hold_the_vault_lease_at_once() {
    let dir = vault();
    let workflow = begin_run(dir.path(), "workflow", Some("brief")).unwrap();
    let error = begin_run(dir.path(), "heartbeat", None).unwrap_err();
    assert!(error.starts_with("Hollow is busy"), "got {error}");
    let error = begin_run(dir.path(), "workflow", Some("other")).unwrap_err();
    assert!(error.starts_with("Hollow is busy"), "got {error}");
    finish_run(dir.path(), workflow, "success").unwrap();

    let beat = begin_run(dir.path(), "heartbeat", None).unwrap();
    let error = begin_run(dir.path(), "workflow", Some("brief")).unwrap_err();
    assert!(error.starts_with("Hollow is busy"), "got {error}");
    finish_run(dir.path(), beat, "success").unwrap();
}

/// `expected` must block admission, the advisory read must name the same
/// holder, and the refusal must write nothing (no new manifest).
fn assert_blocked_by(dir: &TempDir, expected: &LeaseHolder) {
    let manifests_before = read_all_manifests(dir.path()).unwrap().len();
    let error = try_begin_run(dir.path(), "chat", None).unwrap_err();
    assert_eq!(error, BeginRunError::Busy(expected.clone()));
    assert_eq!(error.to_string(), expected.busy_message());
    assert_eq!(lease_holder(dir.path()).as_ref(), Some(expected));
    assert_eq!(
        read_all_manifests(dir.path()).unwrap().len(),
        manifests_before,
        "a refused admission must not create a manifest"
    );
}

#[test]
fn try_begin_run_names_every_lease_holder() {
    let dir = vault();
    write(dir.path().join("notes/a.md"), "a");
    assert_eq!(lease_holder(dir.path()), None);
    assert!(
        !history_root(dir.path()).exists(),
        "naming a holder must never create the history tree"
    );

    // An agent run, named from its manifest.
    let run = begin_run(dir.path(), "workflow", Some("close-my-day")).unwrap();
    let holder = lease_holder(dir.path()).expect("the workflow holds the lease");
    match &holder {
        LeaseHolder::Run {
            id,
            source,
            source_id,
            started_at,
        } => {
            assert_eq!(id, &run.id);
            assert_eq!(source.as_deref(), Some("workflow"));
            assert_eq!(source_id.as_deref(), Some("close-my-day"));
            assert!(started_at.is_some());
        }
        other => panic!("expected a run holder, got {other:?}"),
    }
    assert!(holder.describe().contains("workflow `close-my-day`"));
    assert!(holder.is_workflow("close-my-day"));
    assert!(!holder.is_workflow("another-workflow"));
    assert_eq!(holder.run_id(), Some(run.id.as_str()));
    assert_blocked_by(&dir, &holder);
    let finished = finish_run(dir.path(), run, "success").unwrap();
    assert_eq!(lease_holder(dir.path()), None);

    // Background writers, recognized by owner prefix before any lookup.
    let notion = background_write_lease(dir.path()).unwrap();
    assert_blocked_by(&dir, &LeaseHolder::TrackerSync);
    drop(notion);
    let scheduler = scheduler_write_lease(dir.path()).unwrap();
    assert_blocked_by(&dir, &LeaseHolder::Scheduler);
    assert!(LeaseHolder::Scheduler.is_scheduler());
    assert_eq!(
        scheduler_write_lease(dir.path()).unwrap_err(),
        BeginRunError::Busy(LeaseHolder::Scheduler),
        "bookkeeping leases exclude each other too"
    );
    drop(scheduler);
    assert_eq!(lease_holder(dir.path()), None);

    // A history clear, then an owner this build cannot classify.
    acquire_lease_file(dir.path(), HISTORY_CLEAR_OWNER).unwrap();
    assert_blocked_by(&dir, &LeaseHolder::HistoryClear);
    release_run_lease(dir.path(), HISTORY_CLEAR_OWNER).unwrap();
    acquire_lease_file(dir.path(), "foreign owner").unwrap();
    assert_blocked_by(&dir, &LeaseHolder::Unknown);
    assert!(LeaseHolder::Unknown
        .busy_message()
        .contains("restart Hollow"));
    release_run_lease(dir.path(), "foreign owner").unwrap();

    // A run id whose manifest is not written yet is a run that is starting.
    let starting = new_run_id();
    acquire_lease_file(dir.path(), &starting).unwrap();
    let expected = LeaseHolder::Run {
        id: starting.clone(),
        source: None,
        source_id: None,
        started_at: None,
    };
    assert_blocked_by(&dir, &expected);
    assert!(expected.describe().contains("starting"));
    release_run_lease(dir.path(), &starting).unwrap();

    // A finished run whose lease was never released is stale, and the
    // refusal says how to recover instead of naming it as live.
    acquire_lease_file(dir.path(), &finished.id).unwrap();
    let stale = LeaseHolder::Stale {
        id: finished.id.clone(),
    };
    assert_blocked_by(&dir, &stale);
    assert!(stale.busy_message().contains("restart Hollow"));
    release_run_lease(dir.path(), &finished.id).unwrap();

    // With every holder gone, admission works again.
    let again = try_begin_run(dir.path(), "chat", None).unwrap();
    finish_run(dir.path(), again, "success").unwrap();
    assert_eq!(lease_holder(dir.path()), None);
}

#[test]
fn lease_holder_descriptions_are_single_line_and_bounded() {
    fn run_holder(source: &str, source_id: Option<&str>) -> LeaseHolder {
        LeaseHolder::Run {
            id: "run-1".into(),
            source: Some(source.into()),
            source_id: source_id.map(str::to_string),
            started_at: None,
        }
    }
    assert_eq!(
        run_holder("chat", Some("session")).describe(),
        "a chat turn"
    );
    assert_eq!(
        run_holder("heartbeat", Some("first beat")).describe(),
        "the heartbeat"
    );
    assert_eq!(run_holder("ingest", None).describe(), "an import");
    assert_eq!(
        run_holder("historyRestore", Some("run-0")).describe(),
        "a history restore"
    );
    assert_eq!(run_holder("workflow", None).describe(), "a workflow run");
    assert_eq!(
        run_holder("workflow", Some("<>\n")).describe(),
        "a workflow run",
        "nothing printable left falls back to the generic name"
    );
    assert_eq!(
        run_holder("custom", None).describe(),
        "an automated `custom` run"
    );

    let hostile = run_holder("workflow", Some("close\nmy — day <!-- x -->")).describe();
    assert!(hostile.starts_with("workflow `"), "got {hostile}");
    for forbidden in ["\n", "—", "<", ">"] {
        assert!(!hostile.contains(forbidden), "{forbidden:?} in {hostile}");
    }
    let long_slug = "a".repeat(500);
    let long = run_holder("workflow", Some(long_slug.as_str())).describe();
    assert_eq!(
        long.chars().count(),
        MAX_HOLDER_LABEL_CHARS + "workflow ``".len()
    );

    for holder in [
        LeaseHolder::TrackerSync,
        LeaseHolder::Scheduler,
        LeaseHolder::HistoryClear,
        LeaseHolder::Unknown,
        LeaseHolder::Stale { id: "run-2".into() },
        run_holder("chat", None),
    ] {
        let message = holder.busy_message();
        assert!(message.starts_with("Hollow is busy: "), "got {message}");
        assert!(message.contains(&holder.describe()), "got {message}");
        assert!(!message.contains('\n'), "got {message}");
    }
    assert!(!LeaseHolder::TrackerSync.busy_message().contains("restart"));
    assert_eq!(LeaseHolder::TrackerSync.run_id(), None);
}

/// Review B·F01 (merged plan slice 1). A file over 2 MiB has no object,
/// so a run that shrinks it used to record a `create`, and restore then
/// moved the user's only copy to the trash. Growing a file past 2 MiB
/// recorded a `delete`, and restore overwrote it. Both are now `modify`
/// with the uncaptured side flagged, and restore leaves the file alone.
#[test]
fn restore_never_touches_a_file_history_did_not_capture() {
    let big = MAX_ELIGIBLE_FILE_BYTES as usize + 1;
    let dir = vault();
    write(dir.path().join("notes/shrinks.bin"), vec![b'x'; big]);
    write(dir.path().join("notes/grows.md"), "small");
    write(dir.path().join("notes/plain.md"), "one");
    let handle = begin_run(dir.path(), "chat", None).unwrap();
    write(dir.path().join("notes/shrinks.bin"), "now small");
    write(dir.path().join("notes/grows.md"), vec![b'y'; big]);
    write(dir.path().join("notes/plain.md"), "two");
    let run = finish_run(dir.path(), handle, "success").unwrap();

    let change = |path: &str| {
        run.changes
            .iter()
            .find(|c| c.vault_path == path)
            .unwrap()
            .clone()
    };
    let shrinks = change("notes/shrinks.bin");
    assert_eq!(shrinks.change_type, "modify", "never create");
    assert!(shrinks.before_uncaptured && !shrinks.after_uncaptured);
    assert!(shrinks.before_hash.is_none() && shrinks.after_hash.is_some());
    let grows = change("notes/grows.md");
    assert_eq!(grows.change_type, "modify", "never delete");
    assert!(grows.after_uncaptured && !grows.before_uncaptured);

    for path in ["notes/shrinks.bin", "notes/grows.md"] {
        for force in [false, true] {
            let error = restore_change(dir.path(), &run.id, path, force).unwrap_err();
            assert!(error.contains("didn't capture"), "{path}: {error}");
        }
    }
    let error = read_version(dir.path(), &run.id, "notes/shrinks.bin", "before").unwrap_err();
    assert!(
        error.contains("didn't capture"),
        "not 'did not exist': {error}"
    );
    // A whole-run restore plans nothing while any side is uncaptured.
    assert!(restore_run(dir.path(), &run.id, true)
        .unwrap_err()
        .contains("didn't capture"));
    assert_eq!(
        fs::read(dir.path().join("notes/shrinks.bin")).unwrap(),
        b"now small"
    );
    assert_eq!(
        fs::read(dir.path().join("notes/grows.md")).unwrap().len(),
        big
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("notes/plain.md")).unwrap(),
        "two"
    );
    // A captured file in the same run still restores on its own.
    restore_change(dir.path(), &run.id, "notes/plain.md", false).unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("notes/plain.md")).unwrap(),
        "one"
    );
    // The finished manifest keeps no uncaptured list, like its baseline.
    assert!(read_manifest(dir.path(), &run.id)
        .unwrap()
        .uncaptured
        .is_empty());
}

fn snap(path: &str, body: &str) -> SnapshotEntry {
    SnapshotEntry {
        vault_path: path.into(),
        hash: hash_bytes(body.as_bytes()),
        size: body.len() as u64,
        // Not text, so the diff reads no object for Notion provenance.
        text: false,
    }
}

fn gap(path: &str, reason: UncapturedReason, size: u64) -> UncapturedEntry {
    UncapturedEntry {
        vault_path: path.into(),
        reason: Some(reason),
        size: Some(size),
        modified_ns: Some(1),
    }
}

/// Merged plan slice 4 (review B·F01): every side of a change is
/// absent, captured, uncaptured with its reason, or unknown, and the
/// type follows what existed. Only absent and captured sides restore.
#[test]
fn diff_classifies_each_coverage_state() {
    let dir = vault();
    let before = [
        snap("gone.md", "a"),
        snap("locked/x.md", "b"),
        snap("unread.md", "c"),
    ];
    let after = [snap("new.md", "d")];
    let before_gaps = [
        gap("grew.bin", UncapturedReason::TooLarge, 3_000_000),
        gap("same.bin", UncapturedReason::TooLarge, 3_000_000),
    ];
    let after_gaps = [
        gap("big.bin", UncapturedReason::TooLarge, 3_000_000),
        gap("grew.bin", UncapturedReason::TooLarge, 3_000_001),
        gap("link.md", UncapturedReason::Excluded, 8),
        gap("same.bin", UncapturedReason::TooLarge, 3_000_000),
        gap("unread.md", UncapturedReason::Unreadable, 1),
    ];
    let unlisted = ["locked".to_string()];
    let changes = diff_snapshots(
        dir.path(),
        &before,
        &after,
        Gaps {
            uncaptured: &before_gaps,
            unlisted: &[],
        },
        Gaps {
            uncaptured: &after_gaps,
            unlisted: &unlisted,
        },
    )
    .unwrap();

    let uncaptured = |reason| Coverage::Uncaptured {
        reason: Some(reason),
    };
    let got: Vec<(&str, &str, Coverage, Coverage)> = changes
        .iter()
        .map(|c| {
            (
                c.vault_path.as_str(),
                c.change_type.as_str(),
                c.coverage_before(),
                c.coverage_after(),
            )
        })
        .collect();
    use Coverage::{Absent, Captured, Unknown};
    use UncapturedReason::{Excluded, TooLarge, Unreadable};
    assert_eq!(
        got,
        vec![
            ("big.bin", "create", Absent, uncaptured(TooLarge)),
            ("gone.md", "delete", Captured, Absent),
            (
                "grew.bin",
                "modify",
                uncaptured(TooLarge),
                uncaptured(TooLarge),
            ),
            ("link.md", "create", Absent, uncaptured(Excluded)),
            ("locked/x.md", "modify", Captured, Unknown),
            ("new.md", "create", Absent, Captured),
            ("unread.md", "modify", Captured, uncaptured(Unreadable)),
        ],
        "same.bin kept its size and time, so it is no change"
    );
    for change in &changes {
        let restorable = matches!(change.vault_path.as_str(), "gone.md" | "new.md");
        assert_eq!(change.recoverable(), restorable, "{}", change.vault_path);
        // The slice 1 flags still say "restore refuses" to older readers.
        assert_eq!(
            change.before_uncaptured || change.after_uncaptured,
            !restorable,
            "{}",
            change.vault_path
        );
    }
}

/// Records written before slice 4 store no coverage; it is derived from
/// their hashes and slice 1 flags. A record that contradicts itself
/// never reads as restorable. Slice 1's active manifests listed bare
/// paths, which still read.
#[test]
fn coverage_of_older_and_inconsistent_records() {
    let h = Some("a".repeat(64));
    let derived = [
        (None, None, false, Coverage::Absent),
        (None, h.clone(), false, Coverage::Captured),
        (None, None, true, Coverage::Uncaptured { reason: None }),
        (None, h.clone(), true, Coverage::Unknown),
        (Some(Coverage::Captured), None, false, Coverage::Unknown),
        (Some(Coverage::Absent), h.clone(), false, Coverage::Unknown),
        (Some(Coverage::Captured), h.clone(), true, Coverage::Unknown),
        (Some(Coverage::Unknown), None, false, Coverage::Unknown),
    ];
    for (stored, hash, flagged, want) in derived {
        assert_eq!(
            side_coverage(stored, hash.as_deref(), flagged),
            want,
            "{stored:?} {hash:?} {flagged}"
        );
    }
    let manifest: RunManifest = serde_json::from_value(serde_json::json!({
        "run": {
            "id": "r", "source": "chat", "sourceId": null,
            "startedAt": "2026-10-01T00:00:00Z", "finishedAt": null,
            "status": "running"
        },
        "uncaptured": ["notes/big.bin"]
    }))
    .unwrap();
    assert_eq!(manifest.uncaptured[0].vault_path, "notes/big.bin");
    assert_eq!(manifest.uncaptured[0].reason, None);
    let wire = serde_json::to_value(Coverage::Uncaptured {
        reason: Some(UncapturedReason::TooLarge),
    })
    .unwrap();
    assert_eq!(
        wire,
        serde_json::json!({"state": "uncaptured", "reason": "tooLarge"})
    );
}

/// Merged plan slice 4: before any restore, the preview lists every file
/// a restore can't return. A run that creates an oversized file used to
/// leave it out of History entirely, so nothing warned that restoring
/// the run would not undo it.
#[test]
fn restore_preview_lists_a_file_history_did_not_capture() {
    let big = MAX_ELIGIBLE_FILE_BYTES as usize + 1;
    let dir = vault();
    write(dir.path().join("notes/plain.md"), "one");
    let handle = begin_run(dir.path(), "chat", None).unwrap();
    write(dir.path().join("notes/plain.md"), "changed");
    write(dir.path().join("notes/export.bin"), vec![b'x'; big]);
    write(dir.path().join("notes/also.bin"), vec![b'y'; big]);
    let run = finish_run(dir.path(), handle, "success").unwrap();

    let preview = restore_preview(&run.changes);
    let too_large = Coverage::Uncaptured {
        reason: Some(UncapturedReason::TooLarge),
    };
    assert_eq!(
        preview,
        vec![
            Unrestorable {
                vault_path: "notes/also.bin".into(),
                side: "after",
                coverage: too_large,
            },
            Unrestorable {
                vault_path: "notes/export.bin".into(),
                side: "after",
                coverage: too_large,
            },
        ]
    );
    // The refusal names every listed file, and nothing is written.
    let error = restore_run(dir.path(), &run.id, true).unwrap_err();
    assert!(error.contains("2 files"), "{error}");
    for path in ["notes/also.bin", "notes/export.bin"] {
        assert!(error.contains(path), "{path}: {error}");
        assert!(error.contains("over 2 MiB"), "{error}");
    }
    assert_eq!(
        fs::read_to_string(dir.path().join("notes/plain.md")).unwrap(),
        "changed"
    );
    assert_eq!(
        fs::read(dir.path().join("notes/export.bin")).unwrap().len(),
        big
    );
    // The captured file restores on its own.
    restore_change(dir.path(), &run.id, "notes/plain.md", false).unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("notes/plain.md")).unwrap(),
        "one"
    );
}

/// Decision 5: a run with any uncaptured file never claims to be fully
/// recoverable. Rewriting an oversized file used to record nothing, so
/// the run below claimed it.
#[test]
fn a_run_with_an_uncaptured_file_is_not_fully_recoverable() {
    let big = MAX_ELIGIBLE_FILE_BYTES as usize + 1;
    let dir = vault();
    write(dir.path().join("notes/a.md"), "one");
    write(dir.path().join("notes/data.bin"), vec![b'x'; big]);

    let handle = begin_run(dir.path(), "chat", None).unwrap();
    write(dir.path().join("notes/a.md"), "two");
    let clean = finish_run(dir.path(), handle, "success").unwrap();
    assert!(clean.fully_recoverable(), "{:?}", clean.changes);

    let handle = begin_run(dir.path(), "chat", None).unwrap();
    write(dir.path().join("notes/a.md"), "three");
    write(dir.path().join("notes/data.bin"), vec![b'y'; big + 10]);
    let run = finish_run(dir.path(), handle, "success").unwrap();
    assert!(!run.fully_recoverable());
    let data = run
        .changes
        .iter()
        .find(|c| c.vault_path == "notes/data.bin")
        .expect("the rewritten oversized file is a change");
    assert!(!data.recoverable());
    assert!(data.before_uncaptured && data.after_uncaptured);
    // Its stored coverage survives a reload from disk.
    let reread = get_run(dir.path(), &run.id).unwrap();
    assert!(!reread.fully_recoverable());
}

/// The real walk names each reason: a link is excluded, an unreadable
/// file is unreadable, and a folder it can't list makes what was below
/// it unknown. Permission checks are skipped where they don't bite
/// (running as root).
#[cfg(unix)]
#[test]
fn the_snapshot_names_why_it_holds_no_copy() {
    use std::os::unix::fs::PermissionsExt;
    let dir = vault();
    let v = dir.path();
    write(v.join("notes/target.md"), "t");
    write(v.join("notes/secret.md"), "s");
    write(v.join("locked/inside.md"), "i");
    std::os::unix::fs::symlink("target.md", v.join("notes/link.md")).unwrap();
    let chmod = |path: &Path, mode: u32| {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap()
    };
    chmod(&v.join("notes/secret.md"), 0o000);
    chmod(&v.join("locked"), 0o000);
    let bites = fs::read(v.join("notes/secret.md")).is_err();
    let coverage = snapshot_coverage(v, None);
    chmod(&v.join("notes/secret.md"), 0o644);
    chmod(&v.join("locked"), 0o755);
    let coverage = coverage.unwrap();

    let reason = |path: &str| {
        coverage
            .uncaptured
            .iter()
            .find(|e| e.vault_path == path)
            .and_then(|e| e.reason)
    };
    assert_eq!(reason("notes/link.md"), Some(UncapturedReason::Excluded));
    if bites {
        assert_eq!(
            reason("notes/secret.md"),
            Some(UncapturedReason::Unreadable)
        );
        assert_eq!(coverage.unlisted, ["locked"]);
        assert!(Gaps::of(&coverage).unlisted_above("locked/inside.md"));
    }
}

/// Review B·F01 (merged plan slice 1). Object GC keeps only what the
/// readable manifests reference, so a manifest that won't parse used to
/// lose its objects and its restore. Retention now prunes nothing until
/// every run record reads.
#[test]
fn retention_prunes_nothing_while_a_run_record_is_unreadable() {
    let dir = vault();
    write(dir.path().join("notes/a.md"), "0");
    let mut ids = Vec::new();
    for i in 0..4 {
        let handle = begin_run_inner(dir.path(), "chat", None).unwrap();
        write(dir.path().join("notes/a.md"), format!("{i}-payload"));
        ids.push(
            finish_run_inner(dir.path(), &handle.id, "success", FlagMode::None)
                .unwrap()
                .run
                .id,
        );
    }
    let corrupt = manifest_path(dir.path(), &ids[1]).unwrap();
    let saved = fs::read(&corrupt).unwrap();
    fs::write(&corrupt, b"{ not json").unwrap();
    let objects = || fs::read_dir(objects_root(dir.path())).unwrap().count();
    let before = objects();

    enforce_retention(dir.path(), 1, 90, 1).unwrap();
    assert_eq!(objects(), before, "no object collected");
    assert_eq!(
        read_all_manifests(dir.path()).unwrap().len(),
        3,
        "no readable run pruned"
    );
    assert!(corrupt.exists());

    // Once it reads again, retention resumes and the repaired run restores.
    fs::write(&corrupt, saved).unwrap();
    enforce_retention(dir.path(), 10, 90, u64::MAX).unwrap();
    restore_run(dir.path(), &ids[1], true).unwrap();
    enforce_retention(dir.path(), 1, 90, u64::MAX).unwrap();
    assert!(
        read_all_manifests(dir.path()).unwrap().len() <= 2,
        "pruning works again"
    );
}

/// Review of #1383: a `runs/` entry that can't be read was skipped, so
/// object GC could collect the versions of a record it never saw. It now
/// counts as unreadable and retention prunes and collects nothing, while
/// readers of the manifests still get every one that reads.
#[test]
fn retention_prunes_nothing_while_a_run_entry_cannot_be_read() {
    let dir = vault();
    write(dir.path().join("notes/a.md"), "0");
    for i in 0..4 {
        let handle = begin_run_inner(dir.path(), "chat", None).unwrap();
        write(dir.path().join("notes/a.md"), format!("{i}-payload"));
        finish_run_inner(dir.path(), &handle.id, "success", FlagMode::None).unwrap();
    }
    let objects = || fs::read_dir(objects_root(dir.path())).unwrap().count();
    let before = objects();

    SCAN_FAULT_ENTRY.with(|f| f.set(true));
    assert_eq!(
        read_all_manifests(dir.path()).unwrap().len(),
        4,
        "readers still see every readable record"
    );
    enforce_retention(dir.path(), 1, 90, 1).unwrap();
    SCAN_FAULT_ENTRY.with(|f| f.set(false));
    assert_eq!(objects(), before, "no object collected");
    assert_eq!(
        read_all_manifests(dir.path()).unwrap().len(),
        4,
        "no run pruned"
    );

    // Once every entry reads, retention resumes.
    enforce_retention(dir.path(), 1, 90, u64::MAX).unwrap();
    assert_eq!(read_all_manifests(dir.path()).unwrap().len(), 1);
    assert!(objects() < before, "unreferenced objects collected");
}

#[test]
fn retention_enforces_count_age_and_storage_then_garbage_collects() {
    let dir = vault();
    write(dir.path().join("notes/a.md"), "0");
    let mut ids = Vec::new();
    for i in 0..5 {
        let handle = begin_run_inner(dir.path(), "chat", None).unwrap();
        write(dir.path().join("notes/a.md"), format!("{i}-payload"));
        let run = finish_run_inner(dir.path(), &handle.id, "success", FlagMode::None)
            .unwrap()
            .run;
        ids.push(run.id);
    }
    enforce_retention(dir.path(), 3, 90, u64::MAX).unwrap();
    assert_eq!(read_all_manifests(dir.path()).unwrap().len(), 3);
    assert!(get_run(dir.path(), &ids[0]).is_err(), "oldest run pruned");

    // Age-prune a retained manifest by moving its timestamps beyond the
    // private retention helper's one-day cutoff.
    let mut old = read_manifest(dir.path(), &ids[4]).unwrap();
    old.run.started_at = "2020-01-01T00:00:00Z".into();
    old.run.finished_at = Some("2020-01-01T00:00:01Z".into());
    write_manifest(dir.path(), &old).unwrap();
    enforce_retention(dir.path(), 10, 1, u64::MAX).unwrap();
    assert!(get_run(dir.path(), &ids[4]).is_err());

    // Tiny storage ceiling removes completed runs and their unreferenced
    // objects. It may retain zero completed runs, which is preferable to
    // breaking an active baseline.
    enforce_retention(dir.path(), 10, 90, 1).unwrap();
    let remaining = read_all_manifests(dir.path()).unwrap();
    assert!(remaining.iter().all(|m| m.run.finished_at.is_none()) || remaining.is_empty());
    let refs = referenced_hashes(&remaining);
    let object_names: HashSet<String> = fs::read_dir(objects_root(dir.path()))
        .unwrap()
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .collect();
    assert_eq!(object_names, refs);
}

/// One finished run with a repository sidecar of `sidecar` bytes
/// (Paperclip PR 16), stored while its lease is held.
fn run_with_sidecar(dir: &TempDir, n: usize, sidecar: Option<&[u8]>) -> String {
    let handle = begin_run_inner(dir.path(), "chat", None).unwrap();
    write(dir.path().join("notes/a.md"), format!("{n}-payload"));
    if let Some(bytes) = sidecar {
        store_repo_sidecar(dir.path(), &handle.id, Some(bytes)).unwrap();
    }
    finish_run_inner(dir.path(), &handle.id, "success", FlagMode::None)
        .unwrap()
        .run
        .id
}

fn sidecar_exists(dir: &TempDir, id: &str) -> bool {
    repo_sidecar_path(dir.path(), id).unwrap().exists()
}

#[test]
fn repo_sidecar_is_pruned_with_its_run_and_orphans_swept() {
    let dir = vault();
    write(dir.path().join("notes/a.md"), "0");
    let ids: Vec<String> = (0..4)
        .map(|n| run_with_sidecar(&dir, n, Some(b"{\"repos\":[]}")))
        .collect();
    assert!(ids.iter().all(|id| sidecar_exists(&dir, id)));
    let orphan = runs_root(dir.path()).join("run-x.repo.json");
    write(&orphan, "{}");
    // Not a sidecar name (its id fails validation): left alone.
    let stranger = runs_root(dir.path()).join("notes.v2.repo.json");
    write(&stranger, "{}");
    let plain = runs_root(dir.path()).join("notes.repo.json");
    write(&plain, "{}");

    enforce_retention(dir.path(), 2, 90, u64::MAX).unwrap();
    assert!(!dir
        .path()
        .join(format!(".hollow-history/runs/{}.json", ids[0]))
        .exists());
    assert!(!sidecar_exists(&dir, &ids[0]), "pruned with its manifest");
    assert!(!sidecar_exists(&dir, &ids[1]));
    assert!(sidecar_exists(&dir, &ids[2]) && sidecar_exists(&dir, &ids[3]));
    assert!(!orphan.exists(), "an orphan sidecar is swept");
    assert!(stranger.exists());
    assert!(plain.exists(), "not shaped like a run id: left alone");
}

#[test]
fn repo_sidecar_counts_against_the_storage_ceiling() {
    let dir = vault();
    write(dir.path().join("notes/a.md"), "0");
    let first = run_with_sidecar(&dir, 1, None);
    let second = run_with_sidecar(&dir, 2, None);
    let manifests = read_all_manifests(dir.path()).unwrap();
    let refs = referenced_hashes(&manifests);
    let fits = object_bytes_for_refs(dir.path(), &refs)
        + manifests.iter().map(serialized_manifest_size).sum::<u64>();

    enforce_retention(dir.path(), 10, 90, fits).unwrap();
    assert_eq!(
        read_all_manifests(dir.path()).unwrap().len(),
        2,
        "no sidecar: nothing pruned"
    );

    let handle_bytes = vec![b'x'; 4096];
    // Planted directly: the ceiling counts whatever is there.
    write(
        repo_sidecar_path(dir.path(), &second).unwrap(),
        &handle_bytes,
    );
    enforce_retention(dir.path(), 10, 90, fits).unwrap();
    let left: Vec<String> = read_all_manifests(dir.path())
        .unwrap()
        .into_iter()
        .map(|m| m.run.id)
        .collect();
    assert!(
        !left.contains(&first),
        "the oldest finished run is pruned first"
    );
    assert!(left.contains(&second) || left.is_empty());
}

#[test]
fn repo_sidecar_is_cleared_and_never_read_as_a_manifest() {
    let dir = vault();
    write(dir.path().join("notes/a.md"), "0");
    let id = run_with_sidecar(&dir, 1, Some(b"{\"schemaVersion\":1}"));
    let runs = list_runs(dir.path(), 50).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, id);
    let status = storage_status(dir.path()).unwrap();
    assert_eq!(status.run_count, 1);
    let sidecar = fs::metadata(repo_sidecar_path(dir.path(), &id).unwrap())
        .unwrap()
        .len();
    assert!(status.total_bytes >= sidecar);
    clear_history(dir.path()).unwrap();
    assert!(!sidecar_exists(&dir, &id));
}

#[test]
fn store_repo_sidecar_requires_the_runs_own_lease() {
    let dir = vault();
    write(dir.path().join("notes/a.md"), "0");
    // No lease at all.
    assert!(store_repo_sidecar(dir.path(), "run-none", Some(b"{}")).is_err());

    let handle = begin_run_inner(dir.path(), "chat", None).unwrap();
    // Someone else's run id.
    let err = store_repo_sidecar(dir.path(), "run-foreign", Some(b"{}")).unwrap_err();
    assert!(err.contains("no longer names this run"), "{err}");
    assert!(!sidecar_exists(&dir, "run-foreign"));
    // An invalid id is refused before anything else.
    assert!(store_repo_sidecar(dir.path(), "../x", Some(b"{}")).is_err());

    // A symlink planted at the path is replaced, never followed.
    let outside = TempDir::new().unwrap();
    let target = outside.path().join("target.json");
    write(&target, "untouched");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, repo_sidecar_path(dir.path(), &handle.id).unwrap())
        .unwrap();
    #[cfg(unix)]
    assert!(
        read_repo_sidecar_bytes(dir.path(), &handle.id).is_err(),
        "reads refuse a symlink"
    );
    store_repo_sidecar(dir.path(), &handle.id, Some(b"{\"ok\":true}")).unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), "untouched");
    let path = repo_sidecar_path(dir.path(), &handle.id).unwrap();
    assert!(!fs::symlink_metadata(&path)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        read_repo_sidecar_bytes(dir.path(), &handle.id)
            .unwrap()
            .unwrap(),
        b"{\"ok\":true}"
    );

    // Over the cap: refused on write, refused on read.
    let big = vec![b' '; (MAX_REPO_SIDECAR_BYTES + 1) as usize];
    assert!(store_repo_sidecar(dir.path(), &handle.id, Some(&big)).is_err());
    write(&path, &big);
    assert!(read_repo_sidecar_bytes(dir.path(), &handle.id).is_err());

    // `None` removes whatever was planted there.
    store_repo_sidecar(dir.path(), &handle.id, None).unwrap();
    assert!(!path.exists());
    assert_eq!(
        read_repo_sidecar_bytes(dir.path(), &handle.id).unwrap(),
        None
    );
    finish_run_inner(dir.path(), &handle.id, "success", FlagMode::None).unwrap();
    // The lease is gone now: the finished run can no longer write.
    assert!(store_repo_sidecar(dir.path(), &handle.id, Some(b"{}")).is_err());
}

#[test]
fn corrupt_object_is_detected_and_repaired_by_a_later_snapshot() {
    let dir = vault();
    write(dir.path().join("notes/a.md"), "valid");
    let first = begin_run(dir.path(), "chat", None).unwrap();
    let manifest = read_manifest(dir.path(), &first.id).unwrap();
    let hash = manifest.baseline[0].hash.clone();
    write(object_path(dir.path(), &hash).unwrap(), "corrupt");
    assert!(read_object(dir.path(), &hash)
        .unwrap_err()
        .contains("SHA-256"));

    // A later snapshot in the same leased run sees the expected hash and
    // atomically repairs the object using the source file's known-good
    // bytes. A second full-vault run is intentionally rejected until this
    // one finalizes.
    snapshot(dir.path()).unwrap();
    assert_eq!(read_object(dir.path(), &hash).unwrap(), b"valid");
    finish_run(dir.path(), first, "success").unwrap();
}

// ── Snapshot stat cache (OB09) ─────────────────────────────────────────────

/// Moves this thread's snapshot clock ahead while alive. ctime can't be set
/// back, so this is how a test makes a file it just wrote settled.
struct ClockAhead;

impl ClockAhead {
    fn by(secs: u64) -> Self {
        let ahead = std::time::Duration::from_secs(secs);
        SNAPSHOT_CLOCK_AHEAD.with(|clock| clock.set(ahead));
        ClockAhead
    }
}

impl Drop for ClockAhead {
    fn drop(&mut self) {
        SNAPSHOT_CLOCK_AHEAD.with(|clock| clock.set(std::time::Duration::ZERO));
    }
}

/// `scope`'s snapshot and the file and object reads it took.
fn counted_snapshot(vault: &Path, scope: Option<&str>) -> (Vec<SnapshotEntry>, usize) {
    CONTENT_READS.with(|reads| reads.set(0));
    let entries = snapshot_under(vault, scope).unwrap();
    (entries, CONTENT_READS.with(|reads| reads.get()))
}

fn set_mtime(path: &Path, mtime: SystemTime) {
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(mtime).unwrap();
}

fn a_minute_ago() -> SystemTime {
    SystemTime::now() - std::time::Duration::from_secs(60)
}

/// The hash `entries` hold for `path`.
fn hash_at(entries: &[SnapshotEntry], path: &str) -> String {
    let entry = entries.iter().find(|entry| entry.vault_path == path);
    entry.unwrap().hash.clone()
}

fn cache_keys(vault: &Path) -> Vec<String> {
    let mut keys: Vec<String> = load_stat_cache(vault).into_keys().collect();
    keys.sort();
    keys
}

/// OB09: a second snapshot of an unchanged folder reads no file and
/// captures exactly what the first one did.
#[test]
fn a_second_snapshot_of_an_unchanged_scope_reads_no_file() {
    let dir = vault();
    let v = dir.path();
    let old = a_minute_ago();
    for i in 0..1000 {
        let path = v.join(format!("workspaces/w/{}/f{i}.md", i % 10));
        write(&path, format!("file {i}"));
        set_mtime(&path, old);
    }
    let _clock = ClockAhead::by(60);
    let (first, reads) = counted_snapshot(v, Some("workspaces/w"));
    assert_eq!(first.len(), 1000);
    assert_eq!(reads, 1000, "each file once; no object existed");
    let (second, reads) = counted_snapshot(v, Some("workspaces/w"));
    assert_eq!(reads, 0, "the stat cache vouched for every file");
    assert_eq!(second, first);
}

/// OB09: a rewrite to the same size that puts the old mtime back is still
/// captured: inside the racy window (always read), when vouched for (ctime
/// moved) and when replaced by a rename (a new inode).
#[test]
fn a_same_size_rewrite_with_the_old_mtime_is_still_captured() {
    let dir = vault();
    let v = dir.path();
    let path = v.join("notes/a.md");
    // The real clock: written this second, so never vouched for.
    write(&path, "one!");
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    counted_snapshot(v, None);
    write(&path, "two!");
    set_mtime(&path, mtime);
    let (after, reads) = counted_snapshot(v, None);
    assert_eq!(reads, 1);
    assert_eq!(hash_at(&after, "notes/a.md"), hash_bytes(b"two!"));

    // Settled and vouched for, then rewritten in place with the cached
    // size and mtime: only ctime tells.
    let _clock = ClockAhead::by(60);
    let old = a_minute_ago();
    write(&path, "three");
    set_mtime(&path, old);
    counted_snapshot(v, None);
    assert_eq!(counted_snapshot(v, None).1, 0, "vouched for");
    write(&path, "four!");
    set_mtime(&path, old);
    let (after, reads) = counted_snapshot(v, None);
    assert_eq!(reads, 1, "ctime moved");
    assert_eq!(hash_at(&after, "notes/a.md"), hash_bytes(b"four!"));

    // Replaced by a rename (from a hidden temp file, never captured).
    assert_eq!(counted_snapshot(v, None).1, 0, "vouched for again");
    let temp = v.join("notes/.a.md.tmp");
    write(&temp, "five!");
    set_mtime(&temp, old);
    fs::rename(&temp, &path).unwrap();
    let (after, reads) = counted_snapshot(v, None);
    assert_eq!(reads, 1, "a new inode");
    assert_eq!(hash_at(&after, "notes/a.md"), hash_bytes(b"five!"));
}

/// OB09: a cache that is garbage, another version, or not sealed by this
/// process (a forged entry) is ignored: the snapshot reads the file and is
/// right, and the cache is rebuilt.
#[test]
fn a_corrupt_or_forged_stat_cache_is_ignored_and_rebuilt() {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    let dir = vault();
    let v = dir.path();
    let path = v.join("notes/a.md");
    write(&path, "one!");
    set_mtime(&path, a_minute_ago());
    let _clock = ClockAhead::by(60);
    let (first, _) = counted_snapshot(v, None);
    let cache = stat_cache_path(v);
    assert!(cache.is_file());

    // Unvouched, the file is read and its existing object re-verified (as
    // before OB09): two counted reads.
    fs::write(&cache, b"\0 not a cache").unwrap();
    let (after, reads) = counted_snapshot(v, None);
    assert_eq!((after, reads), (first.clone(), 2));
    assert_eq!(counted_snapshot(v, None).1, 0, "rebuilt");

    // Another version, though sealed with this process's key.
    let mut files = load_stat_cache(v);
    let newer = StatCache {
        version: STAT_CACHE_VERSION + 1,
        files: files.clone(),
    };
    let body = serde_json::to_vec(&newer).unwrap();
    fs::write(&cache, seal_stat_cache(&body)).unwrap();
    assert_eq!(counted_snapshot(v, None).1, 2, "another version");

    // The file's new stat with its old hash: what an agent would forge to
    // hide an edit. Under this process's seal it would be believed; under
    // any other it is ignored.
    let mtime = fs::metadata(&path).unwrap().modified().unwrap();
    write(&path, "two!");
    set_mtime(&path, mtime);
    let meta = fs::symlink_metadata(&path).unwrap();
    files.get_mut("notes/a.md").unwrap().0 = file_stat(&meta).unwrap();
    let forged = StatCache {
        version: STAT_CACHE_VERSION,
        files,
    };
    let body = serde_json::to_vec(&forged).unwrap();
    assert!(open_stat_cache(&seal_stat_cache(&body)).is_some());
    let other = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, b"not this process");
    let tag = ring::hmac::sign(&other, &body);
    let mut unsealed = BASE64.encode(tag.as_ref()).into_bytes();
    unsealed.push(b'\n');
    unsealed.extend_from_slice(&body);
    fs::write(&cache, unsealed).unwrap();
    let (after, reads) = counted_snapshot(v, None);
    assert_eq!(reads, 1);
    assert_eq!(hash_at(&after, "notes/a.md"), hash_bytes(b"two!"));
}

/// OB09: when retention has taken the object a cached entry names, the file
/// is read and its object written again; a damaged one is repaired.
#[test]
fn a_vouched_file_whose_object_is_gone_is_written_again() {
    let dir = vault();
    let v = dir.path();
    let path = v.join("notes/a.md");
    write(&path, "kept");
    set_mtime(&path, a_minute_ago());
    let _clock = ClockAhead::by(60);
    let (first, _) = counted_snapshot(v, None);
    assert_eq!(counted_snapshot(v, None).1, 0, "vouched for");
    let object = object_path(v, &first[0].hash).unwrap();
    fs::remove_file(&object).unwrap();
    let (after, reads) = counted_snapshot(v, None);
    assert_eq!((after, reads), (first.clone(), 1));
    assert_eq!(read_object(v, &first[0].hash).unwrap(), b"kept");
    // Another size: the file is read, and so is the object it replaces.
    fs::write(&object, "kep").unwrap();
    assert_eq!(counted_snapshot(v, None).1, 2, "damaged object");
    assert_eq!(read_object(v, &first[0].hash).unwrap(), b"kept");
}

/// OB09: a scoped snapshot replaces only its folder's entries (by path
/// component, so `workspaces/ab` is outside `workspaces/a`) and drops the
/// ones it no longer captures; the rest stay vouched for.
#[test]
fn a_scoped_snapshot_keeps_the_stat_cache_outside_its_folder() {
    let dir = vault();
    let v = dir.path();
    let old = a_minute_ago();
    let files = [
        "notes/z.md",
        "workspaces/a/big.md",
        "workspaces/a/gone.md",
        "workspaces/a/x.md",
        "workspaces/ab/w.md",
        "workspaces/b/y.md",
    ];
    for rel in files {
        write(v.join(rel), rel);
        set_mtime(&v.join(rel), old);
    }
    let _clock = ClockAhead::by(60);
    counted_snapshot(v, None);
    assert_eq!(cache_keys(v), files);

    fs::remove_file(v.join("workspaces/a/gone.md")).unwrap();
    let big = vec![b'x'; MAX_ELIGIBLE_FILE_BYTES as usize + 1];
    write(v.join("workspaces/a/big.md"), big);
    // x.md is vouched for; big.md is now too large to read.
    assert_eq!(counted_snapshot(v, Some("workspaces/a")).1, 0);
    let kept = [
        "notes/z.md",
        "workspaces/a/x.md",
        "workspaces/ab/w.md",
        "workspaces/b/y.md",
    ];
    assert_eq!(cache_keys(v), kept);
    assert_eq!(counted_snapshot(v, None).1, 0, "outside kept");
}

/// Forces this thread's answer to "does the vault's file system keep a
/// real ctime?" while alive.
struct RealCtime;

impl RealCtime {
    fn forced(real: bool) -> Self {
        REAL_CTIME_OVERRIDE.with(|forced| forced.set(Some(real)));
        RealCtime
    }
}

impl Drop for RealCtime {
    fn drop(&mut self) {
        REAL_CTIME_OVERRIDE.with(|forced| forced.set(None));
    }
}

/// Lowers this thread's stat cache size cap while alive.
struct CacheCap;

impl CacheCap {
    fn at(bytes: u64) -> Self {
        STAT_CACHE_CAP.with(|cap| cap.set(Some(bytes)));
        CacheCap
    }
}

impl Drop for CacheCap {
    fn drop(&mut self) {
        STAT_CACHE_CAP.with(|cap| cap.set(None));
    }
}

/// A settled file at `rel` holding `bytes`.
fn settled_file(vault: &Path, rel: &str, bytes: &str) {
    let path = vault.join(rel);
    write(&path, bytes);
    set_mtime(&path, a_minute_ago());
}

/// OB09 review: an object rewritten in place to the same size (same inode,
/// same length) no longer passes for intact. The entry holds the object's
/// own stat, so the file is read and the object repaired, as before OB09.
#[test]
fn a_same_size_damaged_object_is_repaired_although_its_file_is_vouched_for() {
    use std::io::Write as _;
    let dir = vault();
    let v = dir.path();
    settled_file(v, "notes/a.md", "kept");
    let _clock = ClockAhead::by(60);
    let (first, _) = counted_snapshot(v, None);
    assert_eq!(counted_snapshot(v, None).1, 0, "vouched for");
    let hash = first[0].hash.clone();
    let object = object_path(v, &hash).unwrap();
    let mut file = fs::File::options().write(true).open(&object).unwrap();
    file.write_all(b"junk").unwrap();
    drop(file);
    assert!(read_object(v, &hash).is_err(), "damaged in place");
    let (after, reads) = counted_snapshot(v, None);
    assert_eq!(reads, 2, "the file read, the damaged object read");
    assert_eq!(after, first);
    assert_eq!(read_object(v, &hash).unwrap(), b"kept");
    assert_eq!(hash_bytes(&fs::read(&object).unwrap()), hash);
    assert_eq!(counted_snapshot(v, None).1, 0, "vouched for again");
}

/// OB09 review: the cache vouches only on file systems known to keep a
/// real ctime and inode; names are `statfs`'s (`f_fstypename` on macOS).
#[test]
fn only_file_systems_with_a_real_ctime_are_trusted() {
    for name in ["apfs", "hfs", "ext4", "xfs", "btrfs"] {
        assert!(keeps_real_ctime(name), "{name}");
    }
    let untrusted = ["msdos", "exfat", "smbfs", "nfs", "afpfs", "webdav"];
    let unknown = ["macfuse", "osxfuse", "fuse", "ntfs", "tmpfs", ""];
    for name in untrusted.into_iter().chain(unknown).chain(["APFS"]) {
        assert!(!keeps_real_ctime(name), "{name}");
    }
    #[cfg(target_os = "linux")]
    {
        assert_eq!(linux_file_system_name(0xEF53), Some("ext4"));
        assert_eq!(linux_file_system_name(0x5846_5342), Some("xfs"));
        assert_eq!(linux_file_system_name(0x9123_683E), Some("btrfs"));
        // NFS, SMB2, FUSE, FAT.
        for magic in [0x6969, 0xFE53_4D42, 0x6573_5546, 0x4D44] {
            assert_eq!(linux_file_system_name(magic), None, "{magic:#x}");
        }
    }
    // The other OB09 tests rely on the test's temp folder being one.
    let dir = vault();
    let name = file_system_name(dir.path());
    let trusted = name.as_deref().is_some_and(keeps_real_ctime);
    assert!(trusted, "{name:?}");
}

/// OB09 review: on a file system without a real ctime (FAT, exFAT, a
/// network share) every snapshot reads every file and keeps no cache.
#[test]
fn without_a_real_ctime_every_file_is_read_and_no_cache_is_kept() {
    let dir = vault();
    let v = dir.path();
    settled_file(v, "notes/a.md", "kept");
    let _clock = ClockAhead::by(60);
    let forced = RealCtime::forced(false);
    let (first, reads) = counted_snapshot(v, None);
    assert_eq!(reads, 1);
    let (second, reads) = counted_snapshot(v, None);
    assert_eq!(reads, 2, "the file read again, its object re-verified");
    assert_eq!(second, first);
    assert!(!stat_cache_path(v).exists(), "no cache written");
    drop(forced);
    let _forced = RealCtime::forced(true);
    assert_eq!(counted_snapshot(v, None).1, 2, "a cache built");
    assert_eq!(counted_snapshot(v, None).1, 0, "and used");
}

/// OB09 review: a body over the cap is not saved (every load would ignore
/// it) and the old file is removed; a file over the cap is not loaded.
#[test]
fn an_over_cap_stat_cache_is_neither_saved_nor_loaded() {
    let dir = vault();
    let v = dir.path();
    settled_file(v, "notes/a.md", "kept");
    let _clock = ClockAhead::by(60);
    counted_snapshot(v, None);
    let size = fs::metadata(stat_cache_path(v)).unwrap().len();
    {
        let _cap = CacheCap::at(size - 1);
        assert!(load_stat_cache(v).is_empty(), "a byte over the cap");
    }
    assert_eq!(load_stat_cache(v).len(), 1);

    // A second entry grows the body past the cap: a.md is still vouched
    // for from the old file, which is then removed rather than left stale.
    settled_file(v, "notes/b.md", "more");
    let _cap = CacheCap::at(size);
    let (after, reads) = counted_snapshot(v, None);
    assert_eq!((after.len(), reads), (2, 1));
    assert!(!stat_cache_path(v).exists(), "not saved; old one gone");
}

/// OB09 review: a FIFO or a link in the cache's place is refused on the
/// open handle; a FIFO with no writer doesn't block the snapshot.
#[cfg(unix)]
#[test]
fn a_fifo_or_link_in_place_of_the_stat_cache_is_ignored() {
    let dir = vault();
    let v = dir.path();
    settled_file(v, "notes/a.md", "kept");
    let _clock = ClockAhead::by(60);
    counted_snapshot(v, None);
    let cache = stat_cache_path(v);
    let sealed = fs::read(&cache).unwrap();
    fs::remove_file(&cache).unwrap();
    let fifo = std::ffi::CString::new(cache.to_string_lossy().as_bytes()).unwrap();
    // SAFETY: a NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o644) }, 0);
    assert!(load_stat_cache(v).is_empty(), "a FIFO");

    fs::remove_file(&cache).unwrap();
    let copy = v.join("notes/sealed-copy");
    fs::write(&copy, &sealed).unwrap();
    std::os::unix::fs::symlink(&copy, &cache).unwrap();
    assert!(load_stat_cache(v).is_empty(), "a link to a sealed copy");

    fs::remove_file(&cache).unwrap();
    fs::rename(&copy, &cache).unwrap();
    assert_eq!(load_stat_cache(v).len(), 1, "same bytes, plain file");
}

/// OB09 review: two settled files of one size and mtime that swap names
/// are both read (each path now has the other's inode, and a new ctime).
#[test]
fn two_settled_files_that_swap_names_are_both_read() {
    let dir = vault();
    let v = dir.path();
    settled_file(v, "notes/a.md", "AAAA");
    settled_file(v, "notes/b.md", "BBBB");
    let _clock = ClockAhead::by(60);
    counted_snapshot(v, None);
    assert_eq!(counted_snapshot(v, None).1, 0, "vouched for");
    let (a, b) = (v.join("notes/a.md"), v.join("notes/b.md"));
    let temp = v.join("notes/.swap.tmp");
    fs::rename(&a, &temp).unwrap();
    fs::rename(&b, &a).unwrap();
    fs::rename(&temp, &b).unwrap();
    let (after, reads) = counted_snapshot(v, None);
    assert_eq!(reads, 4, "both files read, both objects re-verified");
    assert_eq!(hash_at(&after, "notes/a.md"), hash_bytes(b"BBBB"));
    assert_eq!(hash_at(&after, "notes/b.md"), hash_bytes(b"AAAA"));
}

/// OB09 review: settled needs both timestamps outside the racy window; an
/// old mtime (set back by `touch -m`) with a fresh ctime is not settled.
#[test]
fn a_file_is_settled_only_when_its_mtime_and_ctime_are_both_old() {
    let threshold: i64 = 1_000 * 1_000_000_000;
    let (old, fresh) = (threshold - 1, threshold);
    let settled = |mtime, ctime| FileStat(4, mtime, ctime, 1, 1).settled(threshold);
    assert!(settled(old, old));
    assert!(!settled(old, fresh), "fresh ctime");
    assert!(!settled(fresh, old), "fresh mtime");
}

#[test]
fn clear_history_removes_only_hidden_history_tree() {
    let dir = vault();
    write(dir.path().join("notes/keep.md"), "keep");
    let handle = begin_run(dir.path(), "chat", None).unwrap();
    finish_run(dir.path(), handle, "success").unwrap();
    clear_history(dir.path()).unwrap();
    assert!(!history_root(dir.path()).exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("notes/keep.md")).unwrap(),
        "keep"
    );
    assert!(list_runs(dir.path(), 10).unwrap().is_empty());
    clear_history(dir.path()).unwrap(); // idempotent
}

#[test]
fn clear_history_rejects_an_active_run_without_damaging_its_baseline() {
    let dir = vault();
    write(dir.path().join("notes/keep.md"), "before");
    let handle = begin_run(dir.path(), "chat", None).unwrap();
    write(dir.path().join("notes/keep.md"), "after");

    let error = clear_history(dir.path()).unwrap_err();

    assert!(error.contains("active"));
    assert!(history_root(dir.path()).exists());
    let run = finish_run(dir.path(), handle, "success").unwrap();
    assert_eq!(run.changes.len(), 1);
    clear_history(dir.path()).unwrap();
    assert!(!history_root(dir.path()).exists());
}

#[cfg(unix)]
#[test]
fn read_apis_reject_a_symlinked_history_tree_and_clear_only_unlinks_it() {
    let dir = vault();
    let external = vault();
    write(external.path().join("runs/sentinel"), "outside");
    write(
        external.path().join(ACTIVE_RUN_FILE),
        "tracker-sync-outside\n",
    );
    std::os::unix::fs::symlink(external.path(), history_root(dir.path())).unwrap();

    assert!(list_runs(dir.path(), 10).unwrap_err().contains("symlink"));
    assert_eq!(
        lease_holder(dir.path()),
        None,
        "the advisory read never follows a replaced history tree"
    );
    clear_history(dir.path()).unwrap();
    assert!(!history_root(dir.path()).exists());
    assert_eq!(
        fs::read_to_string(external.path().join("runs/sentinel")).unwrap(),
        "outside"
    );
}

#[test]
fn wire_types_serialize_camel_case() {
    let change = HistoryChange {
        vault_path: "notes/a.md".into(),
        change_type: "modify".into(),
        before_hash: Some("a".repeat(64)),
        after_hash: Some("b".repeat(64)),
        before_size: Some(1),
        after_size: Some(2),
        text: true,
        notion_mirror: false,
        before_notion_id: None,
        after_notion_id: None,
        before_uncaptured: false,
        after_uncaptured: false,
        before_coverage: None,
        after_coverage: None,
        restored_at: None,
        restored_by_run_id: None,
        also_changed_by: Vec::new(),
        unattributed: false,
        unverified: false,
        hollow_write: false,
    };
    let value = serde_json::to_value(change).unwrap();
    assert!(
        value.get("beforeUncaptured").is_none(),
        "false flags stay off the wire"
    );
    assert!(value.get("vaultPath").is_some());
    assert!(value.get("changeType").is_some());
    assert!(value.get("beforeHash").is_some());
    assert!(value.get("restoredByRunId").is_none());
}

// ── The step spine's restore to a step (master plan F6) ─────────────

#[test]
fn restore_to_versions_is_exact_and_a_refusal_changes_nothing() {
    let dir = vault();
    let v = dir.path();
    write(v.join("notes/a.md"), "v0");
    let handle = begin_run(v, "chat", Some("s-1")).unwrap();
    // Step 2 wrote v1 (the spine stores it), later steps wrote v2 and a
    // new file.
    write(v.join("notes/a.md"), "v1");
    let v1 = store_step_object(v, &handle.id, b"v1").unwrap();
    write(v.join("notes/a.md"), "v2");
    write(v.join("notes/new.md"), "n");
    let run = finish_run(v, handle, "success").unwrap();
    // Retention after other runs keeps the pinned step object.
    let other = begin_run(v, "chat", Some("s-2")).unwrap();
    finish_run(v, other, "success").unwrap();
    assert!(object_path(v, &v1).unwrap().exists(), "pinned by its run");
    assert!(
        store_step_object(v, &run.id, b"late").is_err(),
        "a finished run pins nothing"
    );
    let wanted = vec![
        WantedVersion {
            vault_path: "notes/a.md".into(),
            hash: Some(v1.clone()),
            size: Some(2),
        },
        WantedVersion {
            vault_path: "notes/new.md".into(),
            hash: None,
            size: None,
        },
    ];
    // Someone edited a file since: refused, and nothing changed.
    write(v.join("notes/new.md"), "edited since");
    let err = restore_to_versions(v, &run.id, &wanted).unwrap_err();
    assert!(err.contains("no longer matches"), "{err}");
    assert_eq!(fs::read_to_string(v.join("notes/a.md")).unwrap(), "v2");
    assert_eq!(
        fs::read_to_string(v.join("notes/new.md")).unwrap(),
        "edited since"
    );
    // A version History doesn't hold: refused, nothing changed.
    write(v.join("notes/new.md"), "n");
    let missing = vec![WantedVersion {
        vault_path: "notes/a.md".into(),
        hash: Some("0".repeat(64)),
        size: Some(2),
    }];
    assert!(restore_to_versions(v, &run.id, &missing).is_err());
    assert_eq!(fs::read_to_string(v.join("notes/a.md")).unwrap(), "v2");
    // A path the run never recorded: History can't vouch for it.
    let unknown = vec![WantedVersion {
        vault_path: "notes/other.md".into(),
        hash: None,
        size: None,
    }];
    assert!(restore_to_versions(v, &run.id, &unknown)
        .unwrap_err()
        .contains("can't vouch"));
    // Exact: a.md is v1 again and the file created later is gone.
    let restore = restore_to_versions(v, &run.id, &wanted).unwrap().unwrap();
    assert_eq!(fs::read_to_string(v.join("notes/a.md")).unwrap(), "v1");
    assert!(!v.join("notes/new.md").exists());
    // Already there: nothing to do, no History run.
    assert!(
        restore_to_versions(
            v,
            &run.id,
            &[WantedVersion {
                vault_path: "notes/a.md".into(),
                hash: get_run(v, &run.id)
                    .unwrap()
                    .changes
                    .iter()
                    .find(|c| c.vault_path == "notes/a.md")
                    .unwrap()
                    .after_hash
                    .clone(),
                size: Some(2),
            }]
        )
        .is_err(),
        "a.md is v1 now, not the run's after-version: refused"
    );
    // The restore is a History run of its own, so it can be undone.
    restore_run(v, &restore.id, false).unwrap();
    assert_eq!(fs::read_to_string(v.join("notes/a.md")).unwrap(), "v2");
    assert_eq!(fs::read_to_string(v.join("notes/new.md")).unwrap(), "n");
}

// ── Protected-change flags (Paperclip PR 5) ─────────────────────────

#[test]
fn a_tracked_run_persists_its_flags_and_a_second_finish_is_not_new() {
    let dir = vault();
    let v = dir.path();
    write(v.join("notes/a.md"), "a");
    let handle = try_begin_run(v, "chat", Some("session-1")).unwrap();
    // A direct dot-state write is visible only to a tracked finish.
    write(v.join(".review-queue.json"), "{}");
    let first = {
        let _guard = lock_history();
        finish_run_inner(v, &handle.id, "success", FlagMode::Tracked).unwrap()
    };
    assert!(first.newly);
    assert_eq!(
        first.origin,
        Some(("chat".to_string(), Some("session-1".to_string()))),
        "notifications use the source the run began with"
    );
    assert_eq!(first.run.flags.len(), 1, "{:?}", first.run.flags);
    assert_eq!(first.run.flags[0].kind, "stateFile");
    assert_eq!(get_run(v, &handle.id).unwrap().flags, first.run.flags);

    let again = {
        let _guard = lock_history();
        finish_run_inner(v, &handle.id, "success", FlagMode::Tracked).unwrap()
    };
    assert!(!again.newly, "an idempotent re-finish never notifies again");
    assert_eq!(again.run.flags, first.run.flags);

    // Unflagged runs keep the old manifest shape.
    let quiet = begin_run(v, "chat", None).unwrap();
    let quiet = finish_run(v, quiet, "success").unwrap();
    assert!(quiet.flags.is_empty());
    let value = serde_json::to_value(&quiet).unwrap();
    assert!(
        value.get("flags").is_none(),
        "empty flags are not serialized"
    );
}

#[test]
fn restores_are_never_tracked_or_flagged() {
    let dir = vault();
    let v = dir.path();
    write(v.join("me/Guardrails.md"), "- Never delete notes.\n");
    let handle = begin_run(v, "chat", None).unwrap();
    write(v.join("me/Guardrails.md"), "- Anything goes.\n");
    let run = finish_run(v, handle, "success").unwrap();
    assert!(
        run.flags.iter().any(|f| f.kind == "operatorProfile"),
        "{:?}",
        run.flags
    );

    let restore = restore_change(v, &run.id, "me/Guardrails.md", false).unwrap();
    assert_eq!(restore.source, "historyRestore");
    assert!(restore.flags.is_empty(), "{:?}", restore.flags);
    assert_eq!(
        fs::read_to_string(v.join("me/Guardrails.md")).unwrap(),
        "- Never delete notes.\n"
    );
}

// ── Owned leases and release wake-ups (Paperclip PR 7) ──

/// Arm a `lease_released` waiter before the release, so a wake-up that
/// lands first is kept as a stored permit rather than lost.
async fn released_within_a_second(release: impl FnOnce()) {
    let waiter = lease_released().notified();
    tokio::pin!(waiter);
    waiter.as_mut().enable();
    release();
    tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
        .await
        .expect("a lease release wakes a waiting pump within a second");
}

#[tokio::test]
async fn releasing_the_lease_wakes_a_waiting_pump() {
    let dir = vault();
    let v = dir.path();
    write(v.join("notes/a.md"), "a");

    // A run's finish releases its lease and notifies.
    let handle = try_begin_run(v, "chat", None).unwrap();
    forget_release_notices(v);
    released_within_a_second(|| {
        finish_run(v, handle, "success").unwrap();
    })
    .await;
    assert!(
        release_notified(v),
        "finish_run's release notified for this vault"
    );
    assert!(lease_holder(v).is_none());

    // Dropping the scheduler's bookkeeping lease notifies; taking it never does.
    forget_release_notices(v);
    let bookkeeping = scheduler_write_lease(v).unwrap();
    assert!(!release_notified(v), "acquiring a lease never notifies");
    released_within_a_second(|| drop(bookkeeping)).await;
    assert!(release_notified(v));

    // A successful history clear: its lease goes with the tree.
    forget_release_notices(v);
    clear_history(v).unwrap();
    assert!(release_notified(v));

    // Startup's orphan-lease cleanup.
    forget_release_notices(v);
    ensure_history_dirs(v).unwrap();
    fs::write(active_run_path(v), "run-orphaned\n").unwrap();
    reconcile_interrupted(v).unwrap();
    assert!(!active_run_path(v).exists());
    assert!(release_notified(v));

    // A lease that belongs to someone else is never released or notified.
    forget_release_notices(v);
    fs::write(active_run_path(v), "run-foreign\n").unwrap();
    release_run_lease(v, "run-mine").unwrap();
    assert!(active_run_path(v).exists());
    assert!(!release_notified(v));
}

/// Paperclip PR 15c: admission measures its baseline, a handle without
/// the field still parses, and the lease's admission clock is monotonic.
#[test]
fn begin_run_records_the_baseline_duration() {
    let dir = vault();
    let v = dir.path();
    write(v.join("notes/a.md"), "before");
    let handle = begin_run(v, "ingest", None).unwrap();
    assert!(handle.baseline_ms.is_some(), "the snapshot is timed");
    finish_run(v, handle, "success").unwrap();

    let legacy: HistoryRunHandle = serde_json::from_str(r#"{"id":"run-legacy"}"#).unwrap();
    assert_eq!(legacy.baseline_ms, None);
    assert_eq!(
        serde_json::to_value(&legacy).unwrap(),
        serde_json::json!({"id": "run-legacy"})
    );

    let lease = try_begin_owned(v, "heartbeat", None).unwrap();
    assert!(lease.baseline_ms().is_some());
    let first = lease.admitted_elapsed_ms();
    std::thread::sleep(std::time::Duration::from_millis(15));
    let second = lease.admitted_elapsed_ms();
    assert!(second >= first + 10, "monotonic: {first} → {second}");
    lease.finish("success").unwrap();
}

#[test]
fn a_dropped_run_lease_finalizes_its_run_as_cancelled() {
    let dir = vault();
    let v = dir.path();
    write(v.join("notes/a.md"), "before");

    let lease = try_begin_owned(v, "heartbeat", None).unwrap();
    let id = lease.id().to_string();
    assert!(id.starts_with("run-"), "{id}");
    assert_eq!(lease.vault(), v);
    assert!(
        crate::run_guard::is_tracking(v),
        "an owned lease is tracked like any other admission"
    );
    match try_begin_owned(v, "workflow", Some("other")) {
        Err(BeginRunError::Busy(holder)) => assert_eq!(holder.run_id(), Some(id.as_str())),
        other => panic!("the lease is exclusive: {other:?}"),
    }

    write(v.join("notes/a.md"), "after");
    drop(lease);

    assert!(lease_holder(v).is_none(), "the lease file is gone");
    assert!(!active_run_path(v).exists());
    assert!(
        !crate::run_guard::is_tracking(v),
        "the tracked entry was taken at finish, not leaked"
    );
    let run = list_runs(v, 10)
        .unwrap()
        .into_iter()
        .find(|run| run.id == id)
        .expect("the dropped run was finalized");
    assert_eq!(run.status, "cancelled");
    assert!(run.finished_at.is_some());
    assert_eq!(run.changes.len(), 1, "{:?}", run.changes);
}

#[test]
fn lease_is_checks_the_live_lease_file_and_finish_consumes_the_lease() {
    let dir = vault();
    let v = dir.path();
    let elsewhere = vault();

    let lease = try_begin_owned(v, "ingest", None).unwrap();
    assert!(lease_is(v, &lease));
    assert!(
        !lease_is(elsewhere.path(), &lease),
        "a lease never vouches for another vault"
    );
    let path = active_run_path(v);
    let original = fs::read_to_string(&path).unwrap();
    fs::write(&path, "run-forged\n").unwrap();
    assert!(
        !lease_is(v, &lease),
        "a rewritten lease file no longer names this run"
    );
    fs::write(&path, &original).unwrap();
    assert!(lease_is(v, &lease));

    let id = lease.id().to_string();
    let run = lease.finish("success").unwrap();
    assert_eq!(run.id, id);
    assert_eq!(run.status, "success");
    assert!(lease_holder(v).is_none());
}

// ── Lease liveness (Paperclip PR 18) ──

/// The pid of a child that has already exited and been reaped.
#[cfg(unix)]
fn exited_pid() -> i32 {
    let mut child = std::process::Command::new("/usr/bin/true")
        .spawn()
        .expect("spawn /usr/bin/true");
    let pid = child.id() as i32;
    child.wait().expect("reap it");
    pid
}

/// A live `/bin/sleep 60`, killed and reaped however the test ends.
#[cfg(unix)]
struct Sleeper(std::process::Child);

#[cfg(unix)]
impl Sleeper {
    fn spawn() -> Self {
        Sleeper(
            std::process::Command::new("/bin/sleep")
                .arg("60")
                .spawn()
                .expect("spawn /bin/sleep 60"),
        )
    }

    fn pid(&self) -> i32 {
        self.0.id() as i32
    }
}

#[cfg(unix)]
impl Drop for Sleeper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Write a new-format lease record by hand, as another process would.
fn write_lease(vault: &Path, owner: &str, pid: i32, started_at: Option<u64>) {
    ensure_history_dirs(vault).unwrap();
    let record = LeaseRecord {
        owner: owner.to_string(),
        pid: Some(pid),
        started_at,
    };
    fs::write(
        active_run_path(vault),
        format!("{}\n", serde_json::to_string(&record).unwrap()),
    )
    .unwrap();
}

/// Start a real run, then drop everything this process knows about it
/// (its tracking and its held lease), leaving exactly what a run begun
/// by ANOTHER process would leave on disk.
fn foreign_run(vault: &Path, source: &str) -> String {
    let handle = try_begin_run(vault, source, None).unwrap();
    assert!(crate::run_guard::take_tracking(&handle.id).is_some());
    forget_held(vault, &handle.id);
    handle.id
}

/// A running manifest written directly (a second orphan run).
fn orphan_manifest(vault: &Path) -> String {
    let id = new_run_id();
    write_manifest(
        vault,
        &RunManifest {
            run: HistoryRun {
                id: id.clone(),
                source: "workflow".into(),
                source_id: Some("orphan".into()),
                started_at: Utc::now().to_rfc3339(),
                finished_at: None,
                status: "running".into(),
                changes: Vec::new(),
                flags: Vec::new(),
                scope: None,
                overlaps: Vec::new(),
            },
            baseline: Vec::new(),
            uncaptured: Vec::new(),
            unlisted: Vec::new(),
        },
    )
    .unwrap();
    id
}

fn held_for(vault: &Path, owner: &str) -> bool {
    HELD_OWNERS
        .lock()
        .unwrap()
        .contains_key(&(canonical_vault(vault), owner.to_string()))
}

fn held_any(vault: &Path) -> bool {
    let canonical = canonical_vault(vault);
    HELD_OWNERS
        .lock()
        .unwrap()
        .keys()
        .any(|(held_vault, _)| *held_vault == canonical)
}

/// Let this thread act as the GUI (a registered notification app) for
/// the rest of the test.
fn as_gui() {
    TEST_GUI_RECLAIM.with(|cell| cell.set(true));
}

fn usage_row(key: &str, surface: &str, run_id: Option<&str>) -> crate::session_store::UsageRow {
    let mut row = crate::session_store::tests::usage_row_fixture(
        key,
        &crate::session_store::db_time(Utc::now()),
        "running",
    );
    row.surface = surface.into();
    row.run_id = run_id.map(str::to_string);
    row
}

fn usage_outcome(vault: &Path, key: &str) -> String {
    crate::session_store::usage_rows_since(vault, "")
        .unwrap()
        .into_iter()
        .find(|row| row.run_key == key)
        .unwrap_or_else(|| panic!("usage row {key}"))
        .outcome
}

#[test]
fn lease_file_records_owner_pid_and_process_start_and_release_accepts_both_forms() {
    let dir = vault();
    let v = dir.path();
    write(v.join("notes/a.md"), "a");

    let handle = try_begin_run(v, "chat", None).unwrap();
    let id = handle.id.clone();
    let text = fs::read_to_string(active_run_path(v)).unwrap();
    assert!(
        text.ends_with('\n') && text.lines().count() == 1,
        "{text:?}"
    );
    let value: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
    assert_eq!(value["owner"], serde_json::json!(id));
    assert_eq!(value["pid"], serde_json::json!(std::process::id()));
    let record = read_lease_record(v).unwrap();
    assert_eq!(record.owner, id);
    assert_eq!(record.pid, Some(own_pid()));
    assert_eq!(record.started_at, crate::proc_info::own_start_micros());
    assert_eq!(read_lease_owner(v).as_deref(), Some(id.as_str()));
    assert!(held_for(v, &id), "the holder keeps the lease lock");
    assert_eq!(crate::lease_lock::probe(v), LockProbe::Held);
    finish_run(v, handle, "success").unwrap();
    assert!(!active_run_path(v).exists());
    assert!(!held_for(v, &id));
    assert_eq!(
        crate::lease_lock::probe(v),
        LockProbe::Free,
        "the lock went with the lease"
    );

    // A long owner is bounded, and the record stays under the size cap.
    let long = "o".repeat(400);
    acquire_lease_file(v, &long).unwrap();
    let bytes = fs::metadata(active_run_path(v)).unwrap().len();
    assert!(bytes < MAX_LEASE_BYTES, "{bytes} bytes");
    let bounded_owner = "o".repeat(MAX_LEASE_OWNER_CHARS);
    assert_eq!(read_lease_owner(v).as_deref(), Some(bounded_owner.as_str()));
    release_run_lease(v, &bounded_owner).unwrap();
    assert!(!active_run_path(v).exists());

    // Legacy bare-owner leases still parse and release.
    for owner in ["run-x", "tracker-sync-x"] {
        ensure_history_dirs(v).unwrap();
        fs::write(active_run_path(v), format!("{owner}\n")).unwrap();
        let record = read_lease_record(v).unwrap();
        assert_eq!((record.owner.as_str(), record.pid), (owner, None));
        release_run_lease(v, "someone-else").unwrap();
        assert!(
            active_run_path(v).exists(),
            "a mismatched owner is never released"
        );
        release_run_lease(v, owner).unwrap();
        assert!(!active_run_path(v).exists(), "{owner}");
    }
    // Malformed JSON reads as an unknown holder.
    ensure_history_dirs(v).unwrap();
    fs::write(active_run_path(v), "{\"owner\": 7}\n").unwrap();
    assert_eq!(read_lease_record(v), Some(LeaseRecord::unreadable()));
    assert_eq!(lease_holder(v), Some(LeaseHolder::Unknown));
    fs::remove_file(active_run_path(v)).unwrap();

    // A history clear's new-format record.
    acquire_lease_file(v, HISTORY_CLEAR_OWNER).unwrap();
    assert_eq!(read_lease_record(v).unwrap().pid, Some(own_pid()));
    assert_eq!(lease_holder(v), Some(LeaseHolder::HistoryClear));
    release_run_lease(v, HISTORY_CLEAR_OWNER).unwrap();
    assert!(!active_run_path(v).exists());

    // The background guards leave nothing behind.
    drop(background_write_lease(v).unwrap());
    drop(scheduler_write_lease(v).unwrap());
    assert!(!active_run_path(v).exists());
    assert!(!held_any(v), "no held lease outlives its file");
    assert_eq!(crate::lease_lock::probe(v), LockProbe::Free);
}

#[test]
fn owner_exited_truth_table() {
    let record = |pid: Option<i32>, started_at: Option<u64>| LeaseRecord {
        owner: "run-x".into(),
        pid,
        started_at,
    };
    let dead = |_: i32| Some(false);
    let alive = |_: i32| Some(true);
    let eperm = |_: i32| None;
    let start_100 = |_: i32| Some(100u64);
    let start_unknown = |_: i32| None;
    fn check(
        rec: &LeaseRecord,
        holds: bool,
        lock: LockProbe,
        alive: &dyn Fn(i32) -> Option<bool>,
        start_of: &dyn Fn(i32) -> Option<u64>,
    ) -> bool {
        owner_exited(&ExitCtx {
            record: rec,
            own_pid: 50,
            in_process_holds: holds,
            lock,
            alive,
            start_of,
        })
    }
    let exited = record(Some(4242), Some(100));
    // Reclaimable: gone, or alive with another start (pid reuse).
    assert!(check(&exited, false, LockProbe::Free, &dead, &start_100));
    assert!(check(
        &record(Some(4242), Some(99)),
        false,
        LockProbe::Free,
        &alive,
        &start_100
    ));
    assert!(check(
        &record(Some(4242), None),
        false,
        LockProbe::Free,
        &dead,
        &start_unknown
    ));
    // Never reclaimable.
    assert!(
        !check(&exited, true, LockProbe::Free, &dead, &start_100),
        "held in process"
    );
    assert!(
        !check(&exited, false, LockProbe::Held, &dead, &start_100),
        "lock held"
    );
    assert!(
        !check(&exited, false, LockProbe::Unknown, &dead, &start_100),
        "lock unknown"
    );
    assert!(
        !check(
            &record(None, None),
            false,
            LockProbe::Free,
            &dead,
            &start_100
        ),
        "legacy"
    );
    assert!(
        !check(
            &record(Some(50), Some(1)),
            false,
            LockProbe::Free,
            &dead,
            &start_100
        ),
        "own pid"
    );
    assert!(
        !check(
            &record(Some(1), Some(1)),
            false,
            LockProbe::Free,
            &dead,
            &start_100
        ),
        "launchd"
    );
    assert!(!check(
        &record(Some(0), Some(1)),
        false,
        LockProbe::Free,
        &dead,
        &start_100
    ));
    assert!(
        !check(&exited, false, LockProbe::Free, &alive, &start_100),
        "same start"
    );
    assert!(
        !check(&exited, false, LockProbe::Free, &alive, &start_unknown),
        "unknown start"
    );
    assert!(
        !check(
            &record(Some(4242), None),
            false,
            LockProbe::Free,
            &alive,
            &start_100
        ),
        "unrecorded start"
    );
    assert!(
        !check(&exited, false, LockProbe::Free, &eperm, &start_unknown),
        "EPERM"
    );
}

/// The blocker a review found: the lease file is agent-writable, so a pid
/// alone must never let this process reclaim (and finalize, consuming its
/// tracking) its OWN live run.
#[cfg(unix)]
#[test]
fn a_spoofed_dead_pid_on_our_own_live_run_is_never_reclaimed() {
    as_gui();
    let dir = vault();
    let v = dir.path();
    write(v.join("notes/a.md"), "a");
    let handle = try_begin_run(v, "workflow", Some("spoofed")).unwrap();
    let id = handle.id.clone();
    write_lease(v, &id, exited_pid(), Some(1));

    assert!(lease_holder(v).is_some(), "still held");
    assert!(dead_lease_holder_after(v, std::time::Duration::from_millis(10)).is_none());
    match try_begin_run(v, "chat", None) {
        Err(BeginRunError::Busy(holder)) => assert_eq!(holder.run_id(), Some(id.as_str())),
        other => panic!("the live run keeps the vault: {other:?}"),
    }
    assert!(
        run_is_active(&get_run(v, &id).unwrap()),
        "its manifest stays running"
    );
    assert!(
        crate::run_guard::is_tracking(v),
        "its tracking was not consumed"
    );
    let run = finish_run(v, handle, "success").unwrap();
    assert_eq!(run.status, "success");
    assert!(!crate::run_guard::is_tracking(v));
    assert!(lease_holder(v).is_none());

    // An untracked background lease held here is protected the same way.
    let notion = background_write_lease(v).unwrap();
    let owner = read_lease_owner(v).unwrap();
    write_lease(v, &owner, exited_pid(), Some(1));
    assert_eq!(lease_holder(v), Some(LeaseHolder::TrackerSync));
    assert_eq!(
        try_begin_run(v, "chat", None).unwrap_err(),
        BeginRunError::Busy(LeaseHolder::TrackerSync)
    );
    drop(notion);
    assert!(lease_holder(v).is_none());
    assert!(!held_any(v));
}

#[cfg(unix)]
#[test]
fn a_foreign_exited_owner_with_a_running_manifest_is_reclaimed_then_begin_run_succeeds() {
    as_gui();
    let dir = vault();
    let v = dir.path();
    let other = vault();
    write(v.join("notes/a.md"), "a");
    let a = foreign_run(v, "workflow");
    write(v.join("notes/a.md"), "changed by the dead run");
    write_lease(v, &a, exited_pid(), Some(1));
    crate::session_store::upsert_usage(v, &usage_row("a-main", "workflow", Some(a.as_str())))
        .unwrap();
    crate::session_store::upsert_usage(v, &usage_row("a-delegate", "delegate", Some(a.as_str())))
        .unwrap();
    crate::session_store::upsert_usage(v, &usage_row("unrelated", "chat", None)).unwrap();
    // A notice queued for another vault must survive this vault's drain.
    push_reclaim_notices(vec![ReclaimNotice {
        vault: canonical_vault(other.path()),
        finalized: None,
        pid: 7,
        run_id: None,
        reclaimed: false,
    }]);

    assert_eq!(lease_holder(v), None, "an exited owner reads as free");
    let handle = try_begin_run(v, "chat", None).expect("the admission reclaims the lease");
    let reclaimed = get_run(v, &a).unwrap();
    assert_eq!(reclaimed.status, "interrupted");
    assert!(reclaimed.finished_at.is_some());
    assert_eq!(reclaimed.changes.len(), 1, "its change set was kept");
    let health: Vec<_> = crate::health::recent(v)
        .into_iter()
        .filter(|entry| entry.subsystem == "history")
        .collect();
    assert_eq!(health.len(), 1, "{health:?}");
    assert!(health[0].detail.contains("exited"), "{}", health[0].detail);
    assert_eq!(usage_outcome(v, "a-main"), "interrupted");
    assert_eq!(usage_outcome(v, "a-delegate"), "interrupted");
    assert_eq!(
        usage_outcome(v, "unrelated"),
        "running",
        "only the dead run's rows"
    );
    let canonical_other = canonical_vault(other.path());
    assert!(
        RECLAIM_NOTICES
            .lock()
            .unwrap()
            .iter()
            .any(|notice| notice.vault == canonical_other),
        "another vault's notice is untouched"
    );
    drain_reclaim_notices(other.path());
    finish_run(v, handle, "success").unwrap();
}

#[cfg(unix)]
#[test]
fn a_held_lease_lock_or_matching_live_start_keeps_the_lease() {
    as_gui();
    let dir = vault();
    let v = dir.path();
    write(v.join("notes/a.md"), "a");
    let a = foreign_run(v, "workflow");

    // Another process holds the lease lock: the dead pid is not enough.
    write_lease(v, &a, exited_pid(), Some(1));
    let lock = crate::lease_lock::hold(v).expect("the test plays the other process");
    assert!(lease_holder(v).is_some());
    assert!(matches!(
        try_begin_run(v, "chat", None),
        Err(BeginRunError::Busy(_))
    ));
    assert!(run_is_active(&get_run(v, &a).unwrap()));
    drop(lock);

    // A live owner started exactly when recorded keeps the lease.
    let sleeper = Sleeper::spawn();
    let start = crate::proc_info::process_start_micros(sleeper.pid()).expect("a live start");
    write_lease(v, &a, sleeper.pid(), Some(start));
    assert!(lease_holder(v).is_some());
    assert!(matches!(
        try_begin_run(v, "chat", None),
        Err(BeginRunError::Busy(_))
    ));
    assert!(run_is_active(&get_run(v, &a).unwrap()));

    // The same pid with another start is a reused pid: reclaimed.
    write_lease(v, &a, sleeper.pid(), Some(start + 1));
    assert!(lease_holder(v).is_none());
    let handle = try_begin_run(v, "chat", None).expect("reclaimed");
    assert_eq!(get_run(v, &a).unwrap().status, "interrupted");
    finish_run(v, handle, "success").unwrap();
    drop(sleeper);

    // A legacy bare-owner lease is never reclaimed.
    ensure_history_dirs(v).unwrap();
    fs::write(active_run_path(v), "tracker-sync-legacy\n").unwrap();
    assert_eq!(lease_holder(v), Some(LeaseHolder::TrackerSync));
    assert_eq!(
        try_begin_run(v, "chat", None).unwrap_err(),
        BeginRunError::Busy(LeaseHolder::TrackerSync)
    );
    fs::remove_file(active_run_path(v)).unwrap();

    // PR 7b's dead-holder read: a legacy stale lease stays dead (restart
    // hint), an exited owner's stale lease reads as free.
    let finished = begin_run(v, "chat", None).unwrap();
    let finished = finish_run(v, finished, "success").unwrap();
    ensure_history_dirs(v).unwrap();
    fs::write(active_run_path(v), format!("{}\n", finished.id)).unwrap();
    assert!(matches!(
        dead_lease_holder_after(v, std::time::Duration::from_millis(10)),
        Some(LeaseHolder::Stale { .. })
    ));
    write_lease(v, &finished.id, exited_pid(), Some(1));
    assert_eq!(
        dead_lease_holder_after(v, std::time::Duration::from_millis(10)),
        None
    );
    assert_eq!(lease_holder(v), None);
    let again = try_begin_run(v, "chat", None).expect("the stale lease is reclaimed");
    finish_run(v, again, "success").unwrap();
}

#[cfg(unix)]
#[test]
fn exited_background_leases_are_reclaimed_and_headless_skips_manifested_runs() {
    let dir = vault();
    let v = dir.path();
    write(v.join("notes/a.md"), "a");
    for owner in [
        "tracker-sync-dead",
        "workflow-scheduler-dead",
        HISTORY_CLEAR_OWNER,
    ] {
        write_lease(v, owner, exited_pid(), Some(1));
        assert_eq!(lease_holder(v), None, "{owner}");
        let handle = try_begin_run(v, "chat", None).unwrap_or_else(|e| panic!("{owner}: {e}"));
        finish_run(v, handle, "success").unwrap();
    }
    write_lease(v, HISTORY_CLEAR_OWNER, exited_pid(), Some(1));
    clear_history(v).expect("a dead history clear's lease is reclaimed");
    assert!(!history_root(v).exists());

    // No notification app (the headless CLI, and unit tests by default):
    // a dead owner WITH a running manifest is left for the GUI.
    assert!(!may_finalize_reclaimed_runs());
    let a = foreign_run(v, "workflow");
    write_lease(v, &a, exited_pid(), Some(1));
    assert!(
        lease_holder(v).is_some(),
        "advisory reads agree with admission"
    );
    match background_write_lease(v) {
        Err(BeginRunError::Busy(holder)) => assert_eq!(holder.run_id(), Some(a.as_str())),
        other => panic!("the headless process must not reclaim it: {other:?}"),
    }
    assert!(
        run_is_active(&get_run(v, &a).unwrap()),
        "the manifest stays running"
    );
}

/// Two processes reclaiming the same dead lease must not both win: the
/// reclaim takes the lease lock before it touches the file, so the one
/// that finds the lock taken (or the lease changed) backs off and never
/// unlinks the other's fresh lease (Paperclip PR 18 review).
#[cfg(unix)]
#[test]
fn a_concurrent_reclaimer_makes_this_reclaim_back_off_without_unlinking() {
    fn set_hook(hook: Option<Box<dyn FnMut(&Path)>>) {
        TEST_BEFORE_RECLAIM_HOLD.with(|cell| *cell.borrow_mut() = hook);
    }
    let dir = vault();
    let v = dir.path();
    write(v.join("notes/a.md"), "a");

    // Case 1: the rival holds the lease lock (it is mid-reclaim).
    write_lease(v, "tracker-sync-dead", exited_pid(), Some(1));
    let rival: std::rc::Rc<std::cell::RefCell<Option<HeldLock>>> = Default::default();
    let slot = rival.clone();
    set_hook(Some(Box::new(move |vault: &Path| {
        *slot.borrow_mut() =
            Some(crate::lease_lock::hold(vault).expect("the rival takes the lock"));
    })));
    match try_begin_run(v, "chat", None) {
        Err(BeginRunError::Busy(LeaseHolder::TrackerSync)) => {}
        other => panic!("this reclaim must back off: {other:?}"),
    }
    set_hook(None);
    assert_eq!(
        read_lease_owner(v).as_deref(),
        Some("tracker-sync-dead"),
        "nothing was unlinked"
    );
    assert!(!held_any(v), "this process holds nothing");
    drop(rival.borrow_mut().take());

    // Case 2: the rival already replaced the lease (its own new record,
    // no lock of ours to see): the lease changed, so it is left alone.
    let replacement = exited_pid();
    set_hook(Some(Box::new(move |vault: &Path| {
        write_lease(vault, "tracker-sync-rival", replacement, Some(2));
    })));
    match try_begin_run(v, "chat", None) {
        Err(BeginRunError::Busy(LeaseHolder::TrackerSync)) => {}
        other => panic!("a changed lease is never reclaimed in the same pass: {other:?}"),
    }
    set_hook(None);
    assert_eq!(
        read_lease_owner(v).as_deref(),
        Some("tracker-sync-rival"),
        "the rival's lease survives"
    );
    assert!(!held_any(v));
    assert_eq!(
        crate::lease_lock::probe(v),
        LockProbe::Free,
        "the backed-off reclaim let the lock go"
    );

    // Uncontended, the reclaim succeeds and the new lease keeps the very
    // lock the reclaim took.
    let handle = try_begin_run(v, "chat", None).expect("the dead rival's lease is reclaimed");
    assert_eq!(crate::lease_lock::probe(v), LockProbe::Held);
    assert!(held_for(v, &handle.id));
    finish_run(v, handle, "success").unwrap();
    assert_eq!(crate::lease_lock::probe(v), LockProbe::Free);
}

#[test]
fn startup_reconcile_spares_only_a_lock_holding_foreign_owner() {
    let dir = vault();
    let v = dir.path();
    write(v.join("notes/a.md"), "a");
    let a = foreign_run(v, "workflow");
    let b = orphan_manifest(v);
    // Any pid but ours; the lease lock is what proves the owner is live.
    let foreign_pid = if own_pid() == 2 { 3 } else { 2 };
    write_lease(v, &a, foreign_pid, Some(1));
    for (key, surface, run) in [
        ("a-main", "workflow", Some(a.as_str())),
        ("a-delegate", "delegate", Some(a.as_str())),
        ("b-main", "workflow", Some(b.as_str())),
        ("no-run", "oneshot", None),
    ] {
        crate::session_store::upsert_usage(v, &usage_row(key, surface, run)).unwrap();
    }

    // Case 1: another process holds the lease lock.
    let lock = crate::lease_lock::hold(v).expect("the test plays the other process");
    assert_eq!(foreign_live_owner(v).as_deref(), Some(a.as_str()));
    assert_eq!(reconcile_interrupted(v).unwrap(), 1, "only the orphan");
    assert_eq!(get_run(v, &b).unwrap().status, "interrupted");
    assert!(
        run_is_active(&get_run(v, &a).unwrap()),
        "the live foreign run is spared"
    );
    assert!(active_run_path(v).exists(), "and so is its lease");
    crate::usage_ledger::reconcile_interrupted(v, Some(a.as_str())).unwrap();
    assert_eq!(usage_outcome(v, "a-main"), "running");
    assert_eq!(usage_outcome(v, "a-delegate"), "running");
    assert_eq!(usage_outcome(v, "b-main"), "interrupted");
    assert_eq!(usage_outcome(v, "no-run"), "interrupted");
    drop(lock);

    // Case 2: the lock is free (a spoofed live pid): the old behaviour.
    assert_eq!(foreign_live_owner(v), None);
    assert_eq!(reconcile_interrupted(v).unwrap(), 1);
    assert_eq!(get_run(v, &a).unwrap().status, "interrupted");
    assert!(!active_run_path(v).exists(), "the lease was cleared");
    crate::usage_ledger::reconcile_interrupted(v, None).unwrap();
    assert_eq!(usage_outcome(v, "a-main"), "interrupted");

    // Reclaim finalizes exactly the dead owner's run, never everything.
    let source = include_str!("automated_history.rs");
    let start = source.find(concat!("fn ", "reclaim_locked(")).unwrap();
    let end = start + source[start..].find("\n}\n").unwrap();
    assert!(!source[start..end].contains(concat!("reconcile_", "interrupted(")));
}

#[test]
fn with_live_run_serializes_against_finish_run() {
    use std::sync::{Arc, Barrier};
    let dir = vault();
    let v = dir.path().to_path_buf();
    write(v.join("notes/seed.md"), "seed");
    let handle = try_begin_run(&v, "chat", Some("s-20260926-0000abcd")).unwrap();
    let run_id = handle.id.clone();
    let barrier = Arc::new(Barrier::new(2));

    let writer = {
        let (v, run_id, barrier) = (v.clone(), run_id.clone(), barrier.clone());
        std::thread::spawn(move || {
            set_before_live_write_hook(move || {
                // The finishing thread starts now; it must wait for this
                // write rather than diff and release the lease under it.
                barrier.wait();
                std::thread::sleep(std::time::Duration::from_millis(150));
            });
            with_live_run(&v, &run_id, &["chat"], || {
                fs::write(v.join("notes/agent-task.md"), "written inside the run")
                    .map_err(|e| e.to_string())
            })
        })
    };
    barrier.wait();
    let run = finish_run(&v, handle, "success").unwrap();
    writer.join().unwrap().unwrap();
    assert!(
        run.changes
            .iter()
            .any(|c| c.vault_path == "notes/agent-task.md" && c.change_type == "create"),
        "a write inside with_live_run belongs to the run it named: {:?}",
        run.changes
    );

    // Once the run has finished, nothing more is written on its behalf.
    let refused = with_live_run(&v, &run_id, &["chat"], || -> Result<(), String> {
        fs::write(v.join("notes/late.md"), "late").map_err(|e| e.to_string())
    });
    assert_eq!(refused.unwrap_err(), LIVE_RUN_FINISHED_ERROR);
    assert!(!v.join("notes/late.md").exists());
}

#[test]
fn with_live_run_refuses_every_holder_but_the_named_run() {
    let dir = vault();
    let v = dir.path();
    write(v.join("notes/seed.md"), "seed");
    let refused = |result: Result<(), String>| {
        assert_eq!(result.unwrap_err(), LIVE_RUN_FINISHED_ERROR);
    };

    refused(with_live_run(v, "run-nobody", &["chat"], || Ok(())));

    let handle = try_begin_run(v, "workflow", Some("brief")).unwrap();
    let id = handle.id.clone();
    refused(with_live_run(v, &id, &["chat"], || Ok(())));
    refused(with_live_run(v, "run-someone-else", &["workflow"], || {
        Ok(())
    }));
    refused(with_live_run(v, "", &["workflow"], || Ok(())));
    assert_eq!(
        with_live_run(v, &id, &["chat", "workflow"], || Ok(7)).unwrap(),
        7
    );
    assert_eq!(
        with_live_run(v, &id, &["workflow"], || Err::<(), String>("nope".into())).unwrap_err(),
        "nope",
        "the closure's own error passes through"
    );
    finish_run(v, handle, "success").unwrap();

    // A lease left naming the finalized run is stale.
    fs::write(active_run_path(v), format!("{id}\n")).unwrap();
    refused(with_live_run(v, &id, &["workflow"], || Ok(())));
    // A run still snapshotting its baseline (no manifest yet) has no
    // source to match.
    fs::write(active_run_path(v), "run-starting-1\n").unwrap();
    assert!(matches!(
        lease_holder(v),
        Some(LeaseHolder::Run { source: None, .. })
    ));
    refused(with_live_run(
        v,
        "run-starting-1",
        &["chat", "workflow"],
        || Ok(()),
    ));
    fs::remove_file(active_run_path(v)).unwrap();

    // Background holders never authorize a run's writes.
    let bookkeeping = scheduler_write_lease(v).unwrap();
    refused(with_live_run(v, &id, &["workflow"], || Ok(())));
    drop(bookkeeping);
    assert!(lease_holder(v).is_none());
}

fn captured_map(entries: &[SnapshotEntry]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|entry| (entry.vault_path.clone(), entry.hash.clone()))
        .collect()
}

/// Overhaul K1: a snapshot that is handed only the paths that changed
/// matches a full walk, and each stored object is the file's bytes.
#[test]
fn an_incremental_snapshot_matches_the_vault_byte_for_byte() {
    let dir = vault();
    let v = dir.path();
    let old = a_minute_ago();
    for (path, bytes) in [
        ("notes/a.md", "alpha"),
        ("notes/b.md", "beta"),
        ("other/c.md", "gamma"),
    ] {
        let path = v.join(path);
        write(&path, bytes);
        set_mtime(&path, old);
    }
    let _clock = ClockAhead::by(60);
    let first = snapshot_coverage(v, None).unwrap();
    assert_eq!(first.captured.len(), 3);

    write(v.join("notes/a.md"), "ALPHA");
    set_mtime(&v.join("notes/a.md"), old);
    fs::remove_file(v.join("notes/b.md")).unwrap();
    write(v.join("notes/d.md"), "delta");
    set_mtime(&v.join("notes/d.md"), old);
    // Disk changed, and the change list does not name it. The incremental
    // snapshot must keep the tip's bytes for this file.
    write(v.join("other/c.md"), "GAMMA");
    set_mtime(&v.join("other/c.md"), old);

    CONTENT_READS.with(|reads| reads.set(0));
    let inc = super::incremental::apply_changes(
        v,
        first,
        &[
            "notes/a.md".into(),
            "notes/b.md".into(),
            "notes/d.md".into(),
        ],
    )
    .unwrap();
    let reads = CONTENT_READS.with(|reads| reads.get());
    assert!(reads < 8, "unchanged files were read too: {reads}");
    assert!(
        !inc.captured
            .iter()
            .any(|entry| entry.vault_path == "notes/b.md"),
        "a deleted file leaves the snapshot"
    );
    let by_path = captured_map(&inc.captured);
    assert_eq!(
        read_object(v, by_path.get("notes/a.md").unwrap()).unwrap(),
        b"ALPHA"
    );
    assert_eq!(
        read_object(v, by_path.get("notes/d.md").unwrap()).unwrap(),
        b"delta"
    );
    assert_eq!(
        read_object(v, by_path.get("other/c.md").unwrap()).unwrap(),
        b"gamma",
        "a path the change list omitted stays the tip's bytes"
    );

    // Put c back so the listed changes are the whole diff, then the
    // incremental coverage matches a full walk.
    write(v.join("other/c.md"), "gamma");
    set_mtime(&v.join("other/c.md"), old);
    let again = super::incremental::apply_changes(
        v,
        snapshot_coverage(v, None).unwrap(),
        &["other/c.md".into()],
    )
    .unwrap();
    // The full walk above already saw gamma. Applying c again is a no-op
    // against that walk, and it equals the incremental tree of a, d and c.
    assert_eq!(captured_map(&again.captured), captured_map(&inc.captured));
    for entry in &inc.captured {
        let disk = fs::read(v.join(&entry.vault_path)).unwrap();
        assert_eq!(
            read_object(v, &entry.hash).unwrap(),
            disk,
            "{}",
            entry.vault_path
        );
        assert_eq!(hash_bytes(&disk), entry.hash);
    }

    assert!(super::incremental::save_tip(v, &inc));
    assert!(super::incremental::tip_hashes(v).contains(by_path.get("notes/a.md").unwrap()));
    let tip_file = history_root(v).join("snapshot-tip.nosync");
    let older = fs::read(&tip_file).unwrap();
    assert!(super::incremental::save_tip(v, &inc));
    fs::write(&tip_file, &older).unwrap();
    assert!(
        super::incremental::tip_hashes(v).is_empty(),
        "an earlier tip from this process must not open"
    );

    // A publish that fails the size cap removes the tip and does not reuse
    // its generation. Copying the first sealed tip back must not open,
    // before the next publish or after it.
    {
        let _cap = CacheCap::at(1);
        assert!(
            !super::incremental::save_tip(v, &inc),
            "a tip over the cap is not published"
        );
    }
    fs::write(&tip_file, &older).unwrap();
    assert!(
        super::incremental::tip_hashes(v).is_empty(),
        "a restored tip does not open while nothing is published"
    );
    assert!(super::incremental::save_tip(v, &inc));
    fs::write(&tip_file, &older).unwrap();
    assert!(
        super::incremental::tip_hashes(v).is_empty(),
        "generation 1 must not open after a failed publish"
    );
}

/// Times a full walk against a one-file rescan on a clone of this repo.
/// The gate skips it. The numbers belong in the build log, not in an assert.
#[test]
#[ignore = "cloned-tree timing; cargo test --lib -- --ignored incremental_snapshot_timing_on_a_cloned_tree --nocapture"]
fn incremental_snapshot_timing_on_a_cloned_tree() {
    let dir = vault();
    let v = dir.path();
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .unwrap();
    let status = std::process::Command::new("rsync")
        .args([
            "-a",
            "--exclude",
            ".git",
            "--exclude",
            "node_modules",
            "--exclude",
            "target",
            "--exclude",
            "dist",
            "--exclude",
            ".hollow-history",
        ])
        .arg(format!("{}/", repo.display()))
        .arg(format!("{}/", v.display()))
        .status()
        .unwrap();
    assert!(status.success(), "rsync of the clone failed");
    let _clock = ClockAhead::by(60);
    let cold_at = std::time::Instant::now();
    let first = snapshot_coverage(v, None).unwrap();
    let cold = cold_at.elapsed();
    assert!(
        first.captured.len() > 100,
        "the clone had too few files to time: {}",
        first.captured.len()
    );
    let warm_at = std::time::Instant::now();
    let warm = snapshot_coverage(v, None).unwrap();
    let warm_took = warm_at.elapsed();
    let sample = first
        .captured
        .iter()
        .find(|entry| entry.text && entry.size < 100_000)
        .expect("a small text file");
    let path = v.join(&sample.vault_path);
    fs::write(&path, b"changed-for-timing\n").unwrap();
    set_mtime(&path, a_minute_ago());
    CONTENT_READS.with(|reads| reads.set(0));
    let inc_at = std::time::Instant::now();
    let inc = super::incremental::apply_changes(v, warm, std::slice::from_ref(&sample.vault_path))
        .unwrap();
    let inc_took = inc_at.elapsed();
    let reads = CONTENT_READS.with(|reads| reads.get());
    eprintln!(
        "history-timing files={} cold_ms={} warm_ms={} one_path_ms={} content_reads={}",
        first.captured.len(),
        cold.as_millis(),
        warm_took.as_millis(),
        inc_took.as_millis(),
        reads
    );
    assert!(
        reads < first.captured.len() / 10,
        "one path read too much of the clone: {reads} of {}",
        first.captured.len()
    );
    let hash = captured_map(&inc.captured)
        .get(&sample.vault_path)
        .expect("changed file")
        .clone();
    assert_eq!(read_object(v, &hash).unwrap(), b"changed-for-timing\n");
}

// A link opened from Linear leaves one finished, change-free History row
// without taking the vault lease, so it works while a run is live.
#[test]
fn record_event_leaves_a_finished_row_and_never_takes_the_lease() {
    let dir = vault();
    write(dir.path().join("notes/a.md"), "a");
    let live = begin_run(dir.path(), "chat", Some("s-1")).unwrap();
    let row = record_event(dir.path(), "Linear link", Some("ENG-307")).unwrap();
    assert_eq!(
        (row.source.as_str(), row.source_id.as_deref()),
        ("Linear link", Some("ENG-307"))
    );
    assert_eq!(row.status, "success");
    assert!(row.finished_at.is_some() && row.changes.is_empty());
    let runs = list_runs(dir.path(), 10).unwrap();
    assert!(runs.iter().any(|r| r.id == row.id && !run_is_active(r)));
    assert!(
        runs.iter().any(|r| r.id == live.id && run_is_active(r)),
        "the live run is untouched"
    );
    finish_run(dir.path(), live, "success").unwrap();
}

// Event rows keep their own budget: a burst of link opens never pushes a
// real run out of the run count.
#[test]
fn event_rows_cannot_push_real_runs_out_of_retention() {
    let dir = vault();
    write(dir.path().join("notes/a.md"), "0");
    let mut real = Vec::new();
    for i in 0..3 {
        let handle = begin_run_inner(dir.path(), "chat", None).unwrap();
        write(dir.path().join("notes/a.md"), format!("{i}-payload"));
        real.push(
            finish_run_inner(dir.path(), &handle.id, "success", FlagMode::None)
                .unwrap()
                .run
                .id,
        );
    }
    for _ in 0..(MAX_EVENT_ROWS + 20) {
        record_event(dir.path(), crate::linear_link::SOURCE, Some("ENG-1")).unwrap();
    }
    // A budget of 3 real runs: all three stay, the events keep their 100 newest.
    enforce_retention(dir.path(), 3, 90, u64::MAX).unwrap();
    let left = read_all_manifests(dir.path()).unwrap();
    for id in &real {
        assert!(
            left.iter().any(|m| &m.run.id == id),
            "real run {id} survived"
        );
    }
    assert_eq!(
        left.iter().filter(|m| is_event_row(m)).count(),
        MAX_EVENT_ROWS
    );
    assert_eq!(left.len(), 3 + MAX_EVENT_ROWS);
}

/// What another Hollow process holding the object store lock looks like from
/// here: a second open file description of the same lock file.
fn other_process_objects_lock(v: &Path, shared: bool) -> crate::flock::HeldLock {
    let file = crate::lease_lock::open_objects_lock(v).unwrap();
    if shared {
        crate::flock::HeldLock::try_take_shared(file).unwrap()
    } else {
        crate::flock::HeldLock::try_take(file).unwrap()
    }
}

/// Q-R9 (2026-11-13): another process (the headless MCP host) stores a
/// run's objects before the run record that names them exists. A cleanup in
/// that gap kept only what records name and deleted them, and a later
/// restore failed. Cleanup now waits for the object store lock and skips the
/// round while that process holds it.
#[test]
fn object_cleanup_keeps_what_another_process_stored_before_its_run_record() {
    let dir = vault();
    let v = dir.path();
    write(v.join("notes/a.md"), "0");
    let handle = begin_run_inner(v, "chat", None).unwrap();
    write(v.join("notes/a.md"), "1");
    finish_run_inner(v, &handle.id, "success", FlagMode::None).unwrap();

    // The other process is between its object write and its record.
    let in_flight = other_process_objects_lock(v, true);
    let bytes = b"stored by another process, no record yet";
    let hash = hash_bytes(bytes);
    write_object(v, &hash, bytes).unwrap();
    enforce_retention(v, MAX_RUNS, MAX_AGE_DAYS, MAX_STORAGE_BYTES).unwrap();
    assert_eq!(
        read_object(v, &hash).unwrap(),
        bytes,
        "cleanup left the in-flight object alone"
    );
    assert!(
        clear_history(v).is_err(),
        "a clear refuses while a write is in flight"
    );
    assert_eq!(read_object(v, &hash).unwrap(), bytes);
    assert!(get_run(v, &handle.id).is_ok(), "nothing was cleared");

    // Once it lets go, an object no record names is garbage again.
    drop(in_flight);
    enforce_retention(v, MAX_RUNS, MAX_AGE_DAYS, MAX_STORAGE_BYTES).unwrap();
    assert!(
        read_object(v, &hash).is_err(),
        "unreferenced object collected"
    );
    assert!(get_run(v, &handle.id).is_ok(), "the finished run kept");
    restore_run(v, &handle.id, true).unwrap();
}

/// Q-R9: a write waits while another process's cleanup holds the lock
/// exclusively, then holds it shared itself.
#[test]
fn a_history_write_waits_for_another_process_object_cleanup() {
    let dir = vault();
    let v = dir.path().to_path_buf();
    let cleanup = other_process_objects_lock(&v, false);
    let started = Instant::now();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        drop(cleanup);
    });
    let held = objects_in_flight(&v).expect("the write holds the lock shared");
    let waited = started.elapsed();
    release.join().unwrap();
    assert!(
        waited >= std::time::Duration::from_millis(140),
        "the write waited for the cleanup: {waited:?}"
    );
    // Shared: a second write beside it takes the lock too, and a cleanup
    // does not.
    let _second = objects_in_flight(&v).expect("shared beside it");
    let file = crate::lease_lock::open_objects_lock(&v).unwrap();
    assert!(crate::flock::HeldLock::try_take(file).is_err());
    drop(held);
}

/// Q-R9: the incremental snapshot reused the tip's hash for every file the
/// file-system events didn't name, without checking the object was still
/// stored. Another process's cleanup doesn't keep this process's tip, so a
/// run record could name a deleted object. A missing object is now read
/// and stored again.
#[test]
fn incremental_snapshot_stores_again_an_object_another_process_cleaned_up() {
    let dir = vault();
    let v = dir.path();
    write(v.join("notes/a.md"), "alpha");
    write(v.join("notes/b.md"), "beta");
    let tip = snapshot_coverage(v, None).unwrap();
    let a = captured_map(&tip.captured)["notes/a.md"].clone();
    let b = captured_map(&tip.captured)["notes/b.md"].clone();
    fs::remove_file(object_path(v, &a).unwrap()).unwrap();

    let next = super::incremental::apply_changes(v, tip, &[]).unwrap();
    let by_path = captured_map(&next.captured);
    assert_eq!(by_path["notes/a.md"], a);
    assert_eq!(
        read_object(v, &a).unwrap(),
        b"alpha",
        "the missing object is stored again"
    );
    assert_eq!(by_path["notes/b.md"], b);
    assert_eq!(read_object(v, &b).unwrap(), b"beta");
}

/// Review of #1383: a retention cleanup stops deleting before a waiting
/// write would refuse, so a cleanup alone never makes a write refuse.
#[test]
fn cleanup_delete_window_is_shorter_than_a_writers_wait() {
    assert!(OBJECTS_CLEANUP_DELETE_WINDOW < OBJECTS_SHARED_WAIT);
}

/// Review of #1383: a cleanup that paused after collecting its references
/// and resumed past its delete window unlinks nothing; a fresh one does.
#[test]
fn a_cleanup_past_its_delete_window_deletes_nothing() {
    let dir = vault();
    let v = dir.path();
    let bytes = b"reused by a write that went ahead";
    let hash = hash_bytes(bytes);
    write_object(v, &hash, bytes).unwrap();

    let mut paused = objects_cleanup_lock(v).expect("free");
    paused.taken = Instant::now()
        .checked_sub(OBJECTS_CLEANUP_DELETE_WINDOW)
        .expect("uptime longer than the window");
    assert!(!paused.may_delete());
    gc_objects(v, &HashSet::new(), Some(&paused)).unwrap();
    assert_eq!(read_object(v, &hash).unwrap(), bytes, "nothing deleted");
    drop(paused);

    let fresh = objects_cleanup_lock(v).expect("free");
    gc_objects(v, &HashSet::new(), Some(&fresh)).unwrap();
    assert!(
        read_object(v, &hash).is_err(),
        "a fresh cleanup collects it"
    );
}

fn object_names(v: &Path) -> BTreeSet<String> {
    fs::read_dir(objects_root(v))
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Every object a readable run record names is stored.
fn assert_records_restorable(v: &Path) {
    let objects = object_names(v);
    for manifest in read_all_manifests(v).unwrap() {
        for hash in referenced_hashes(std::slice::from_ref(&manifest)) {
            assert!(
                objects.contains(&hash),
                "run {} names missing object {hash}",
                manifest.run.id
            );
        }
    }
}

/// Review round 2 of #1383: a write never stores or reuses an object
/// without the object store lock. One that can't get it refuses, stores
/// nothing, and its run goes without History's coverage: an admission
/// reports History busy (the caller defers the run), a finish fails before
/// its record is written, a step pin isn't stored.
#[test]
fn a_write_without_the_objects_lock_refuses_and_stores_nothing() {
    let dir = vault();
    let v = dir.path();
    write(v.join("notes/a.md"), "before");
    let live = begin_run(v, "chat", Some("s-1")).unwrap();
    let stored = object_names(v);
    OBJECTS_SHARED_WAIT_TEST.with(|w| w.set(Some(std::time::Duration::from_millis(50))));
    let cleanup = other_process_objects_lock(v, false);

    // A finish refuses before its after-versions are stored or recorded.
    write(v.join("notes/a.md"), "after, never stored");
    assert!(store_step_object(v, &live.id, b"a step version").is_err());
    let finished = finish_run_inner(v, &live.id, "success", FlagMode::None);
    assert!(finished.is_err(), "the finish refused");
    assert_eq!(object_names(v), stored, "nothing stored");
    assert_records_restorable(v);

    drop(cleanup);
    OBJECTS_SHARED_WAIT_TEST.with(|w| w.set(None));
    let run = finish_run_inner(v, &live.id, "success", FlagMode::None)
        .unwrap()
        .run;
    assert_eq!(run.changes.len(), 1, "finishes once the lock is free");
    assert_records_restorable(v);

    // An admission reports History busy, writes no record and frees its
    // lease, so the run queue or chat defers it.
    let cleanup = other_process_objects_lock(v, false);
    let records = read_all_manifests(v).unwrap().len();
    let stored = object_names(v);
    match begin_run_inner(v, "chat", None) {
        Err(BeginRunError::Busy(LeaseHolder::HistoryClear)) => {}
        other => panic!("expected History busy, got {other:?}"),
    }
    assert_eq!(read_all_manifests(v).unwrap().len(), records, "no record");
    assert_eq!(object_names(v), stored, "no baseline stored");
    drop(cleanup);
    let handle = begin_run_inner(v, "chat", None).expect("the lease was freed");
    finish_run_inner(v, &handle.id, "success", FlagMode::None).unwrap();
}

/// Review round 2 of #1383: Clear removes every run record before any
/// object, so a clear that fails midway never leaves a record whose
/// versions are gone; the orphaned objects are collected later.
#[test]
fn clear_removes_records_before_objects() {
    let dir = vault();
    let v = dir.path();
    write(v.join("notes/a.md"), "0");
    for i in 0..3 {
        let handle = begin_run_inner(v, "chat", None).unwrap();
        write(v.join("notes/a.md"), format!("{i}"));
        finish_run_inner(v, &handle.id, "success", FlagMode::None).unwrap();
    }
    let stored = object_names(v);
    assert!(!stored.is_empty());

    CLEAR_FAULT_BEFORE_OBJECTS.with(|f| f.set(true));
    let failed = clear_history(v);
    CLEAR_FAULT_BEFORE_OBJECTS.with(|f| f.set(false));
    assert!(failed.is_err(), "the injected failure stops the clear");
    assert!(
        read_all_manifests(v).unwrap_or_default().is_empty(),
        "records went first"
    );
    assert_eq!(object_names(v), stored, "no object removed yet");
    assert_records_restorable(v);

    // The lease was released: the next clear finishes, and runs start again.
    clear_history(v).unwrap();
    assert!(object_names(v).is_empty());
    let handle = begin_run_inner(v, "chat", None).unwrap();
    finish_run_inner(v, &handle.id, "success", FlagMode::None).unwrap();
}

/// Review round 3 of #1383: beside a live scoped run, a `runs/` that can
/// be read by path but not listed leaves the finished runs' records in
/// place. Clear then refuses to collect objects instead of reading the
/// folder as empty, so no record loses its versions. Skipped where the
/// permission doesn't bite (running as root).
#[cfg(unix)]
#[test]
fn clear_keeps_objects_when_runs_cannot_be_listed() {
    use std::os::unix::fs::PermissionsExt;
    let dir = vault();
    let v = dir.path();
    write(v.join("workspaces/wb/WORKSPACE.md"), "brief");
    write(v.join("tasks/a.md"), "task");
    let finished = try_begin_owned(v, "chat", None).unwrap();
    write(v.join("tasks/a.md"), "changed");
    let finished = finished.finish("success").unwrap();
    let chat = try_begin_scoped_shared(v, "workspaces/wb", "chat", None).unwrap();
    let stored = object_names(v);
    assert!(!stored.is_empty());

    let runs = history_root(v).join(RUNS_DIR);
    let chmod = |path: &Path, mode: u32| {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap()
    };
    // After Clear's checks (which list runs/), so only the removal pass
    // and the record check meet the unlistable folder.
    let hooked = runs.clone();
    set_before_clear_delete_hook(move || {
        fs::set_permissions(&hooked, fs::Permissions::from_mode(0o100)).unwrap();
    });
    let bites = {
        chmod(&runs, 0o100);
        let bites = fs::read_dir(&runs).is_err();
        chmod(&runs, 0o755);
        bites
    };
    let cleared = clear_history(v);
    chmod(&runs, 0o755);
    if !bites {
        return;
    }
    let refused = cleared.unwrap_err();
    assert!(
        refused.contains("couldn't list its run records"),
        "{refused}"
    );
    assert_eq!(object_names(v), stored, "no object collected");
    assert!(
        get_run(v, &finished.id).is_ok(),
        "the record is still there"
    );
    assert_records_restorable(v);
    assert!(lease_holder(v).is_none(), "the clear let the vault go");

    // Once runs/ lists again, Clear finishes and the live run goes on.
    clear_history(v).unwrap();
    assert!(get_run(v, &finished.id).is_err(), "finished record cleared");
    write(v.join("workspaces/wb/WORKSPACE.md"), "brief, by b");
    chat.finish("success").unwrap();
}
