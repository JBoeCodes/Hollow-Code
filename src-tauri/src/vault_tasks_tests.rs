//! The `vault_tasks` unit tests (split out of `vault_tasks.rs` by S05,
//! 2026-10-16, no behaviour change). Attached from `vault_tasks.rs` as
//! `#[path] mod tests`, so `super::` is still `crate::vault_tasks`.

use super::*;

// Overnight 10-11/12 P5: listing reads a task file again only when its
// stamp moved, one test per rule.
#[test]
fn listing_reads_each_task_once_until_its_stamp_moves() {
    let reads = || TASK_READS.with(|n| n.get());
    let vault = tempfile::tempdir().unwrap();
    let dir = vault.path().join("tasks");
    fs::create_dir_all(&dir).unwrap();
    let task = |title: &str| format!("---\ntitle: \"{title}\"\nstatus: todo\n---\n\nNotes.\n");
    fs::write(dir.join("a.md"), task("Alpha")).unwrap();
    fs::write(dir.join("b.md"), task("Beta")).unwrap();
    fs::write(dir.join("c.md"), "no frontmatter, not a task\n").unwrap();
    let titles = || {
        let mut t: Vec<String> = list_parsed(vault.path())
            .into_iter()
            .map(|t| t.title)
            .collect();
        t.sort();
        t
    };
    assert_eq!(titles(), ["Alpha", "Beta"]);
    let before = reads();
    assert_eq!(titles(), ["Alpha", "Beta"]);
    assert_eq!(
        reads(),
        before,
        "unchanged files (a non-task too) come from the cache"
    );
    // An edit that changes the length.
    fs::write(dir.join("a.md"), task("Alpha two")).unwrap();
    assert_eq!(titles(), ["Alpha two", "Beta"]);
    assert_eq!(reads(), before + 1, "only the edited file is read again");
    // Same length, mtime put back: only the ctime moved.
    let file = dir.join("b.md");
    let mtime = fs::metadata(&file).unwrap().modified().unwrap();
    let handle = fs::OpenOptions::new().write(true).open(&file).unwrap();
    std::os::unix::fs::FileExt::write_all_at(&handle, task("Gamm").as_bytes(), 0).unwrap();
    handle
        .set_times(fs::FileTimes::new().set_modified(mtime))
        .unwrap();
    drop(handle);
    assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), mtime);
    assert_eq!(titles(), ["Alpha two", "Gamm"]);
    // Replaced by a rename (an atomic save): a new inode.
    let tmp = dir.join(".a.md.tmp");
    fs::write(&tmp, task("Alpha thr")).unwrap();
    fs::rename(&tmp, dir.join("a.md")).unwrap();
    assert_eq!(titles(), ["Alpha thr", "Gamm"]);
    // A file that becomes a task, and one deleted.
    fs::write(dir.join("c.md"), task("Charlie")).unwrap();
    fs::remove_file(dir.join("b.md")).unwrap();
    assert_eq!(titles(), ["Alpha thr", "Charlie"]);
}
use tempfile::TempDir;

fn vault() -> TempDir {
    TempDir::new().unwrap()
}

#[test]
fn slugify_simple() {
    assert_eq!(slugify("Hello World"), "hello-world");
    assert_eq!(slugify("Hello, World!"), "hello-world");
    assert_eq!(slugify(""), "task");
}

#[test]
fn parse_round_trip() {
    let fm = TaskFrontmatter {
        title: "Write the PR".into(),
        status: "In progress".into(),
        priority: Some("High".into()),
        due: Some("2026-06-01".into()),
        effort: Some("M".into()),
        project: Some("Refactor X".into()),
        created: Some("2026-05-27T00:00:00Z".into()),
        last_synced_at: Some("2026-05-27T10:00:00Z".into()),
        ..Default::default()
    };
    let rendered = render(&fm, "Some long description here.");
    let (map, body) = split_frontmatter(&rendered);
    assert_eq!(map.get("title"), Some(&"Write the PR".to_string()));
    assert_eq!(map.get("status"), Some(&"In progress".to_string()));
    assert_eq!(map.get("priority"), Some(&"High".to_string()));
    assert_eq!(map.get("due"), Some(&"2026-06-01".to_string()));
    assert!(body.contains("Some long description here."));
    // Notion mirror metadata is omitted unless a connector explicitly sets it.
    assert!(!rendered.contains("notion"));
}

#[test]
fn notion_metadata_round_trips_when_present() {
    let dir = vault();
    let rel = upsert(
        dir.path(),
        TaskFrontmatter {
            title: "Synced".into(),
            status: "Not started".into(),
            source: Some("notion".into()),
            notion_id: Some("abc123".into()),
            notion_url: Some("https://notion.so/abc123".into()),
            ..Default::default()
        },
        Some("body"),
    )
    .unwrap();
    assert_eq!(notion_id(dir.path(), &rel).as_deref(), Some("abc123"));
    let t = get(dir.path(), &rel).unwrap();
    assert_eq!(t.title, "Synced");
}

#[test]
fn upsert_creates_in_tasks_dir() {
    let dir = vault();
    let fm = TaskFrontmatter {
        title: "First Task".into(),
        status: "Not started".into(),
        ..Default::default()
    };
    let rel = upsert(dir.path(), fm, Some("body")).unwrap();
    assert_eq!(rel, "tasks/first-task.md");
    assert!(dir.path().join(&rel).exists());
}

#[test]
fn upsert_collision_picks_next_filename() {
    let dir = vault();
    let fm = TaskFrontmatter {
        title: "Follow up".into(),
        status: "Not started".into(),
        ..Default::default()
    };
    let r1 = upsert(dir.path(), fm.clone(), Some("a")).unwrap();
    assert_eq!(r1, "tasks/follow-up.md");
    let r2 = upsert(dir.path(), fm, Some("b")).unwrap();
    // Every create is a new file — second call collides, gets -2 suffix.
    assert_eq!(r2, "tasks/follow-up-2.md");
}

#[test]
fn concurrent_task_creates_preserve_every_unique_body() {
    let dir = vault();
    let barrier = std::sync::Barrier::new(8);
    let paths = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|index| {
                let root = dir.path();
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    upsert(
                        root,
                        TaskFrontmatter {
                            title: "Shared title".into(),
                            status: "Not started".into(),
                            ..Default::default()
                        },
                        Some(&format!("Unique body {index}")),
                    )
                    .unwrap()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    let unique: std::collections::HashSet<_> = paths.iter().collect();
    assert_eq!(unique.len(), 8);
    let bodies: std::collections::HashSet<_> = paths
        .iter()
        .map(|rel| {
            let content = std::fs::read_to_string(dir.path().join(rel)).unwrap();
            split_frontmatter(&content).1.trim().to_string()
        })
        .collect();
    assert_eq!(
        bodies,
        (0..8).map(|index| format!("Unique body {index}")).collect()
    );
}

#[test]
fn undo_delete_preserves_a_replacement_at_the_same_path_and_original_in_trash() {
    let dir = vault();
    let fm = TaskFrontmatter {
        title: "Shared task".into(),
        ..Default::default()
    };
    let rel = upsert(dir.path(), fm.clone(), Some("Original task body")).unwrap();
    let original = std::fs::read_to_string(dir.path().join(&rel)).unwrap();
    delete(dir.path(), &rel).unwrap();
    assert_eq!(
        upsert(dir.path(), fm, Some("Replacement body")).unwrap(),
        rel
    );
    let replacement = std::fs::read_to_string(dir.path().join(&rel)).unwrap();
    let error = restore(dir.path(), &rel, &original).unwrap_err();
    assert!(error.contains("another task now exists"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join(&rel)).unwrap(),
        replacement
    );
    let trash = std::fs::read_dir(dir.path().join(".trash"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(std::fs::read_to_string(trash).unwrap(), original);
}

#[test]
fn undo_delete_restores_original_bytes_when_path_remains_free() {
    let dir = vault();
    let rel = upsert(
        dir.path(),
        TaskFrontmatter {
            title: "Restore me".into(),
            ..Default::default()
        },
        Some("Unchanged original"),
    )
    .unwrap();
    let original = std::fs::read_to_string(dir.path().join(&rel)).unwrap();
    delete(dir.path(), &rel).unwrap();
    restore(dir.path(), &rel, &original).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join(&rel)).unwrap(),
        original
    );
}

#[test]
fn terminal_status_is_case_and_vocabulary_tolerant() {
    // The app writes "Delivered"; the embedded chat (or a hand edit) might
    // write "Done"/"Complete" in any casing. All count as complete.
    for s in [
        "Delivered",
        "delivered",
        "Done",
        "DONE",
        "complete",
        "Completed",
    ] {
        assert!(is_terminal_status(s), "{s} should be terminal");
    }
    for s in ["Not started", "In progress", "", "blocked", "doing"] {
        assert!(!is_terminal_status(s), "{s} should not be terminal");
    }
}

#[test]
fn set_status_writes_frontmatter() {
    let dir = vault();
    let rel = upsert(
        dir.path(),
        TaskFrontmatter {
            title: "Do thing".into(),
            status: "Not started".into(),
            ..Default::default()
        },
        None,
    )
    .unwrap();
    let after = set_status(dir.path(), &rel, "Done").unwrap();
    let content = std::fs::read_to_string(dir.path().join(&rel)).unwrap();
    assert_eq!(after, content_hash(&content));
    assert!(content.contains("status: Done"));
    assert!(content.contains("status/done"));
}

// Data safety 0.3 — port of hollow-mac DataSafetyTests.taskMutateNoLostLabels:
// concurrent TaskRepository.mutate appends never lose a label (TOCTOU closed
// by vault::MUTATE_LOCK inside vault_tasks::mutate).
#[test]
fn concurrent_mutate_never_loses_a_label() {
    use std::sync::Arc;
    use std::thread;
    let dir = vault();
    let v: Arc<std::path::PathBuf> = Arc::new(dir.path().to_path_buf());
    let rel = upsert(
        &v,
        TaskFrontmatter {
            title: "Race".into(),
            status: "Not started".into(),
            ..Default::default()
        },
        Some(""),
    )
    .unwrap();
    let rel = Arc::new(rel);
    let n = 50;
    let mut handles = Vec::new();
    for i in 0..n {
        let v = v.clone();
        let rel = rel.clone();
        handles.push(thread::spawn(move || {
            let tag = format!("tag-a{i:02}");
            let _ = mutate(&v, &rel, move |fm| fm.labels.push(tag));
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let content = std::fs::read_to_string(v.join(&*rel))
        .unwrap()
        .to_lowercase();
    for i in 0..n {
        assert!(
            content.contains(&format!("tag-a{i:02}")),
            "lost label tag-a{i:02}"
        );
    }
}

#[test]
fn set_priority_to_none_clears() {
    let dir = vault();
    let rel = upsert(
        dir.path(),
        TaskFrontmatter {
            title: "Pri".into(),
            status: "Not started".into(),
            priority: Some("High".into()),
            ..Default::default()
        },
        None,
    )
    .unwrap();
    set_priority(dir.path(), &rel, None).unwrap();
    let content = std::fs::read_to_string(dir.path().join(&rel)).unwrap();
    assert!(!content.contains("priority: High"));
}

#[test]
fn find_by_url_rejects_path_escape() {
    let dir = vault();
    // A crafted vault: URL must not resolve outside the vault, even
    // when the target exists on disk.
    assert_eq!(find_by_url(dir.path(), "vault:../outside.md"), None);
    assert_eq!(find_by_url(dir.path(), "vault:../../etc/hosts"), None);
    assert_eq!(find_by_url(dir.path(), "vault:/etc/hosts"), None);
}

#[test]
fn mutate_rejects_path_escape() {
    let dir = vault();
    // set_status routes through mutate, which now safe_joins the read.
    assert!(set_status(dir.path(), "../evil.md", "Done").is_err());
    assert!(set_status(dir.path(), "/etc/passwd", "Done").is_err());
}

#[test]
fn list_returns_parsed_tasks() {
    let dir = vault();
    upsert(
        dir.path(),
        TaskFrontmatter {
            title: "One".into(),
            status: "Not started".into(),
            ..Default::default()
        },
        None,
    )
    .unwrap();
    upsert(
        dir.path(),
        TaskFrontmatter {
            title: "Two".into(),
            status: "In progress".into(),
            priority: Some("High".into()),
            due: Some("2026-06-01".into()),
            ..Default::default()
        },
        None,
    )
    .unwrap();
    let tasks = list(dir.path());
    assert_eq!(tasks.len(), 2);
    let two = tasks.iter().find(|t| t.title == "Two").unwrap();
    assert_eq!(two.status, "In progress");
    // Legacy "High" normalizes to P1 on read (Todoist-style P1..P4).
    assert_eq!(two.priority.as_deref(), Some("P1"));
    assert_eq!(two.due.as_deref(), Some("2026-06-01"));
}

#[test]
fn task_gets_vault_url() {
    let dir = vault();
    upsert(
        dir.path(),
        TaskFrontmatter {
            title: "Plain task".into(),
            status: "Not started".into(),
            ..Default::default()
        },
        None,
    )
    .unwrap();
    let tasks = list(dir.path());
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].url.starts_with("vault:tasks/"));
}

#[test]
fn normalize_priority_maps_legacy_and_p_levels() {
    assert_eq!(normalize_priority("High").as_deref(), Some("P1"));
    assert_eq!(normalize_priority("medium").as_deref(), Some("P2"));
    assert_eq!(normalize_priority("LOW").as_deref(), Some("P3"));
    assert_eq!(normalize_priority("p1").as_deref(), Some("P1"));
    assert_eq!(normalize_priority("P4").as_deref(), Some("P4"));
    assert_eq!(normalize_priority(""), None);
    assert_eq!(normalize_priority("bogus"), None);
}

#[test]
fn subtasks_round_trip_through_body() {
    let subs = vec![
        SubTask {
            text: "buy milk".into(),
            done: false,
        },
        SubTask {
            text: "email boss".into(),
            done: true,
        },
    ];
    let body = compose_body("Some prose.", &subs);
    assert!(body.contains("Some prose."));
    assert!(body.contains("- [ ] buy milk"));
    assert!(body.contains("- [x] email boss"));
    let parsed = parse_subtasks(&body);
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].text, "buy milk");
    assert!(!parsed[0].done);
    assert!(parsed[1].done);
    assert_eq!(strip_subtasks(&body), "Some prose.");
}

#[test]
fn parse_reads_labels_section_time_and_subtasks() {
    let dir = vault();
    let fm = TaskFrontmatter {
        title: "Rich".into(),
        status: "Not started".into(),
        priority: Some("P1".into()),
        due: Some("2026-06-15".into()),
        due_time: Some("14:30".into()),
        duration_min: Some(90),
        section: Some("Social".into()),
        labels: vec!["ui".into(), "customer".into()],
        recurrence: Some("every weekday".into()),
        ..Default::default()
    };
    let rel = upsert(dir.path(), fm, Some("Prose.\n\n- [ ] one\n- [x] two")).unwrap();
    let content = std::fs::read_to_string(dir.path().join(&rel)).unwrap();
    assert!(content.contains("labels: ui, customer"));
    assert!(content.contains("- label/ui"));
    assert!(content.contains("- label/customer"));

    let tasks = list(dir.path());
    let t = tasks.iter().find(|t| t.title == "Rich").unwrap();
    assert_eq!(t.priority.as_deref(), Some("P1"));
    assert_eq!(t.due_time.as_deref(), Some("14:30"));
    assert_eq!(t.duration_min, Some(90));
    assert_eq!(t.section.as_deref(), Some("Social"));
    assert_eq!(t.labels, vec!["ui".to_string(), "customer".to_string()]);
    assert_eq!(t.recurrence.as_deref(), Some("every weekday"));
    assert_eq!(t.subtasks.len(), 2);
    assert_eq!(t.subtasks[1].text, "two");
    assert!(t.subtasks[1].done);
}

#[test]
fn migrate_priorities_maps_legacy() {
    let dir = vault();
    let legacy = "---\ntitle: Legacy\nstatus: Not started\npriority: High\n---\n\nbody\n";
    crate::vault::write_note(dir.path(), "tasks/legacy.md", legacy).unwrap();
    assert_eq!(migrate_priorities(dir.path()).unwrap(), 1);
    let content = std::fs::read_to_string(dir.path().join("tasks/legacy.md")).unwrap();
    assert!(content.contains("priority: P1"));
    assert!(content.contains("priority/p1"));
    // Idempotent: a second run rewrites nothing.
    assert_eq!(migrate_priorities(dir.path()).unwrap(), 0);
}

#[test]
fn apply_update_sets_fields_and_subtasks() {
    let dir = vault();
    let rel = upsert(
        dir.path(),
        TaskFrontmatter {
            title: "Edit me".into(),
            status: "Not started".into(),
            ..Default::default()
        },
        Some("Original prose."),
    )
    .unwrap();
    let input = UpdateTaskInput {
        url: format!("vault:{rel}"),
        priority: Some("p2".into()),
        labels: Some(vec!["ui".into(), "ui".into()]), // dup collapses
        due: Some("2026-07-01".into()),
        section: Some("Backend".into()),
        description: Some("New prose.".into()),
        subtasks: Some(vec![SubTask {
            text: "step 1".into(),
            done: false,
        }]),
        ..Default::default()
    };
    apply_update(dir.path(), &rel, &input).unwrap();
    let tasks = list(dir.path());
    let t = &tasks[0];
    assert_eq!(t.priority.as_deref(), Some("P2"));
    assert_eq!(t.labels, vec!["ui".to_string()]);
    assert_eq!(t.due.as_deref(), Some("2026-07-01"));
    assert_eq!(t.section.as_deref(), Some("Backend"));
    assert_eq!(t.description, "New prose.");
    assert_eq!(t.subtasks.len(), 1);
    assert_eq!(t.subtasks[0].text, "step 1");
}

#[test]
fn full_iso_due_survives_an_unrelated_edit() {
    // Notion-backed tasks store `due` as a full RFC3339 stamp. Editing any
    // other field must leave it intact — the editor used to send back an
    // empty `due` because a date input can't hold this shape.
    let dir = vault();
    let iso = "2026-07-23T17:00:00.000-04:00";
    let file = format!("---\ntitle: Ship it\nstatus: Not started\ndue: \"{iso}\"\n---\n\nbody\n");
    crate::vault::write_note(dir.path(), "tasks/ship-it.md", &file).unwrap();
    assert_eq!(list(dir.path())[0].due.as_deref(), Some(iso));

    let input = UpdateTaskInput {
        url: "vault:tasks/ship-it.md".into(),
        priority: Some("p1".into()),
        ..Default::default()
    };
    apply_update(dir.path(), "tasks/ship-it.md", &input).unwrap();
    let t = &list(dir.path())[0];
    assert_eq!(t.priority.as_deref(), Some("P1"));
    assert_eq!(t.due.as_deref(), Some(iso));
}

#[test]
fn changing_project_drops_the_stale_project_url() {
    let dir = vault();
    let rel = upsert(
        dir.path(),
        TaskFrontmatter {
            title: "Re-file me".into(),
            status: "Not started".into(),
            project: Some("Old project".into()),
            project_url: Some("vault:projects/active/Old project.md".into()),
            ..Default::default()
        },
        None,
    )
    .unwrap();
    apply_update(
        dir.path(),
        &rel,
        &UpdateTaskInput {
            url: format!("vault:{rel}"),
            project: Some("New project".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let t = &list(dir.path())[0];
    assert_eq!(t.project_name.as_deref(), Some("New project"));
    assert_eq!(t.project_url, None);
}

#[test]
fn resaving_the_same_project_keeps_its_url() {
    let dir = vault();
    let url = "vault:projects/active/Same project.md";
    let rel = upsert(
        dir.path(),
        TaskFrontmatter {
            title: "Leave my link alone".into(),
            status: "Not started".into(),
            project: Some("Same project".into()),
            project_url: Some(url.into()),
            ..Default::default()
        },
        None,
    )
    .unwrap();
    apply_update(
        dir.path(),
        &rel,
        &UpdateTaskInput {
            url: format!("vault:{rel}"),
            project: Some("Same project".into()),
            priority: Some("p3".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let t = &list(dir.path())[0];
    assert_eq!(t.priority.as_deref(), Some("P3"));
    assert_eq!(t.project_url.as_deref(), Some(url));
}

/// Every key the Paperclip PR 3 acceptance criteria name, plus a legacy
/// priority spelling, a tags block and a body with a checklist in the
/// middle. No `notion_id`, so migrate treats it as a local file.
const FULL: &str = r#"---
title: "Full task"
status: Not started
priority: High
due: "2026-07-23T17:00:00.000-04:00"
due_end: "2026-07-24T09:00:00.000-04:00"
due_time_zone: America/New_York
previous_due: 2026-06-30
description: "Remote summary: kept"
notion_project_ids: "[\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"]"
aliases:
  - First alias
  - Second alias
custom_key: keep me
tags:
  - tasks
  - status/not-started
  - priority/high
---

Intro prose.

- [ ] first step
- [x] second step

Closing prose.
"#;
const FULL_REL: &str = "tasks/full-task.md";

fn write(dir: &Path, rel: &str, content: &str) {
    crate::vault::write_note(dir, rel, content).unwrap();
}

fn read(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap()
}

fn url(rel: &str) -> String {
    format!("vault:{rel}")
}

/// (frontmatter block, everything from the closing fence on).
fn split_fence(content: &str) -> (&str, &str) {
    let end = content[4..].find("\n---").unwrap() + 4;
    (&content[4..end], &content[end..])
}

/// Every line of FULL's frontmatter is still present in `after`, except
/// the regenerated tags block and lines starting with a `changed` prefix.
fn assert_full_lines_kept(after: &str, changed: &[&str]) {
    let (old_block, _) = split_fence(FULL);
    let (new_block, _) = split_fence(after);
    let new_lines: Vec<&str> = new_block.lines().collect();
    let mut in_tags = false;
    for line in old_block.lines() {
        if line == "tags:" {
            in_tags = true;
            continue;
        }
        if in_tags && line.starts_with("  - ") {
            continue;
        }
        in_tags = false;
        if changed.iter().any(|prefix| line.starts_with(prefix)) {
            continue;
        }
        assert!(
            new_lines.contains(&line),
            "lost frontmatter line {line:?} in:\n{after}"
        );
    }
    assert!(
        new_block.contains("aliases:\n  - First alias\n  - Second alias\ncustom_key: keep me"),
        "list-valued key changed shape:\n{after}"
    );
}

#[test]
fn update_task_preserves_unmodelled_frontmatter_and_body() {
    let dir = vault();
    write(dir.path(), FULL_REL, FULL);
    let written = apply_update(
        dir.path(),
        FULL_REL,
        &UpdateTaskInput {
            url: url(FULL_REL),
            priority: Some("p2".into()),
            labels: Some(vec!["ui".into()]),
            section: Some("Backend".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let after = read(dir.path(), FULL_REL);
    assert_eq!(written, content_hash(&after));
    assert_full_lines_kept(&after, &["priority:"]);
    assert!(after.contains("\npriority: P2\n"));
    assert!(after.contains("  - label/ui\n"));
    assert_eq!(split_fence(&after).1, split_fence(FULL).1);
    let t = get(dir.path(), FULL_REL).unwrap();
    assert_eq!(t.priority.as_deref(), Some("P2"));
    assert_eq!(t.labels, vec!["ui".to_string()]);
    assert_eq!(t.section.as_deref(), Some("Backend"));
    assert_eq!(t.description, "Remote summary: kept");
    assert_eq!(t.due.as_deref(), Some("2026-07-23T17:00:00.000-04:00"));
    assert_eq!(t.subtasks.len(), 2);
}

#[test]
fn set_status_and_set_priority_preserve_unmodelled_frontmatter() {
    let dir = vault();
    write(dir.path(), FULL_REL, FULL);
    set_status(dir.path(), FULL_REL, "In progress").unwrap();
    set_priority(dir.path(), FULL_REL, Some("P3")).unwrap();
    let after = read(dir.path(), FULL_REL);
    assert_full_lines_kept(&after, &["status:", "priority:"]);
    assert!(after.contains("\nstatus: In progress\n"));
    assert!(after.contains("\npriority: P3\n"));
    assert!(after.contains("  - status/in-progress\n"));
    assert!(after.contains("  - priority/p3\n"));
    assert_eq!(split_fence(&after).1, split_fence(FULL).1);
}

#[test]
fn recurrence_roll_forward_preserves_unmodelled_frontmatter_and_drops_stale_range() {
    let dir = vault();
    write(dir.path(), FULL_REL, FULL);
    // The roll-forward patch (`set_status_with_recurrence`'s write) as a
    // bare mutate; the shared writer is covered below.
    mutate(dir.path(), FULL_REL, |fm| {
        fm.due = Some("2026-08-01".into());
        fm.status = "Not started".to_string();
    })
    .unwrap();
    let after = read(dir.path(), FULL_REL);
    assert_full_lines_kept(&after, &["due:", "due_end:", "due_time_zone:"]);
    assert!(after.contains("\ndue: 2026-08-01\n"));
    assert!(!after.contains("due_end:"));
    assert!(!after.contains("due_time_zone:"));
    assert_eq!(split_fence(&after).1, split_fence(FULL).1);
}

#[test]
fn changing_due_drops_the_stale_range_metadata_but_other_edits_keep_it() {
    let dir = vault();
    write(dir.path(), FULL_REL, FULL);
    let end = "due_end: \"2026-07-24T09:00:00.000-04:00\"";
    let zone = "due_time_zone: America/New_York";
    apply_update(
        dir.path(),
        FULL_REL,
        &UpdateTaskInput {
            url: url(FULL_REL),
            labels: Some(vec!["later".into()]),
            ..Default::default()
        },
    )
    .unwrap();
    let after = read(dir.path(), FULL_REL);
    assert!(after.lines().any(|l| l == end), "{after}");
    assert!(after.lines().any(|l| l == zone), "{after}");
    apply_update(
        dir.path(),
        FULL_REL,
        &UpdateTaskInput {
            url: url(FULL_REL),
            due: Some("2026-08-01".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let after = read(dir.path(), FULL_REL);
    assert!(!after.contains("due_end:"), "{after}");
    assert!(!after.contains("due_time_zone:"), "{after}");
    assert_eq!(
        get(dir.path(), FULL_REL).unwrap().due.as_deref(),
        Some("2026-08-01")
    );
}

#[test]
fn migrate_priorities_preserves_frontmatter_body_and_unknown_priorities() {
    let dir = vault();
    write(dir.path(), FULL_REL, FULL);
    let odd =
        "---\ntitle: Odd\nstatus: Not started\npriority: Critical\n---\n\n- [ ] a\nprose after\n";
    write(dir.path(), "tasks/odd.md", odd);
    assert_eq!(migrate_priorities(dir.path()).unwrap(), 1);
    let expected = FULL.replacen("priority: High", "priority: P1", 1).replacen(
        "  - priority/high",
        "  - priority/p1",
        1,
    );
    assert_eq!(read(dir.path(), FULL_REL), expected);
    assert_eq!(read(dir.path(), "tasks/odd.md"), odd);
    assert_eq!(migrate_priorities(dir.path()).unwrap(), 0);
    assert_eq!(read(dir.path(), FULL_REL), expected);
}

#[test]
fn migrate_priorities_skips_notion_linked_and_already_canonical_files() {
    let dir = vault();
    let linked = FULL.replacen(
        "custom_key: keep me",
        "custom_key: keep me\nnotion_id: 0123456789abcdef0123456789abcdef",
        1,
    );
    write(dir.path(), "tasks/linked.md", &linked);
    let canonical = "---\ntitle: Canonical\nstatus: Not started\npriority: P2\n---\n\nBody.\n";
    write(dir.path(), "tasks/canonical.md", canonical);
    assert_eq!(migrate_priorities(dir.path()).unwrap(), 0);
    assert_eq!(read(dir.path(), "tasks/linked.md"), linked);
    assert_eq!(read(dir.path(), "tasks/canonical.md"), canonical);
}

#[test]
fn description_edit_targets_the_frontmatter_key_when_present() {
    let dir = vault();
    let describe = |rel: &str, text: &str| {
        apply_update(
            dir.path(),
            rel,
            &UpdateTaskInput {
                url: url(rel),
                description: Some(text.into()),
                ..Default::default()
            },
        )
        .unwrap();
    };

    // A non-empty Notion summary key receives the edit; the body is untouched.
    let summary = "---\ntitle: Linked\nstatus: Not started\ndescription: \"Remote summary\"\n---\n\nAuthored body.\n";
    write(dir.path(), "tasks/summary.md", summary);
    describe("tasks/summary.md", "Edited summary");
    let after = read(dir.path(), "tasks/summary.md");
    let (map, _) = split_frontmatter(&after);
    assert_eq!(
        map.get("description").map(String::as_str),
        Some("Edited summary")
    );
    assert_eq!(
        get(dir.path(), "tasks/summary.md").unwrap().description,
        "Edited summary"
    );
    assert_eq!(split_fence(&after).1, split_fence(summary).1);

    // An EMPTY summary key (a blank Notion Summary) still receives it.
    let empty =
        "---\ntitle: Linked\nstatus: Not started\ndescription: \"\"\n---\n\nAuthored body.\n";
    write(dir.path(), "tasks/empty.md", empty);
    describe("tasks/empty.md", "Edited");
    let after = read(dir.path(), "tasks/empty.md");
    let (map, _) = split_frontmatter(&after);
    assert_eq!(map.get("description").map(String::as_str), Some("Edited"));
    assert_eq!(
        get(dir.path(), "tasks/empty.md").unwrap().description,
        "Edited"
    );
    assert_eq!(split_fence(&after).1, split_fence(empty).1);

    // Resending the displayed fallback prose is not an edit.
    write(dir.path(), "tasks/fallback.md", empty);
    describe("tasks/fallback.md", "Authored body.");
    let after = read(dir.path(), "tasks/fallback.md");
    assert!(after.lines().any(|l| l == "description: \"\""), "{after}");
    assert_eq!(split_fence(&after).1, split_fence(empty).1);
    // Without the key, apply_update_sets_fields_and_subtasks covers the body path.
}

#[test]
fn unchanged_description_and_subtasks_leave_the_body_bytes_alone() {
    let dir = vault();
    let file = "---\ntitle: Steps\nstatus: Not started\n---\n\nIntro\n- [ ] step\nOutro\n";
    write(dir.path(), "tasks/steps.md", file);
    let t = get(dir.path(), "tasks/steps.md").unwrap();
    apply_update(
        dir.path(),
        "tasks/steps.md",
        &UpdateTaskInput {
            url: url("tasks/steps.md"),
            priority: Some("P1".into()),
            description: Some(t.description.clone()),
            subtasks: Some(t.subtasks.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let after = read(dir.path(), "tasks/steps.md");
    assert!(after.contains("\npriority: P1\n"));
    assert_eq!(split_fence(&after).1, split_fence(file).1);
}

fn guarded_task(dir: &Path) -> String {
    upsert(
        dir,
        TaskFrontmatter {
            title: "Guarded".into(),
            status: "Not started".into(),
            ..Default::default()
        },
        Some("Body"),
    )
    .unwrap()
}

#[test]
fn stale_expected_hash_returns_conflict_and_writes_nothing() {
    let dir = vault();
    let rel = guarded_task(dir.path());
    let baseline = get(dir.path(), &rel).unwrap().content_hash;
    let external = format!("{}\nEdited in Obsidian.\n", read(dir.path(), &rel));
    std::fs::write(dir.path().join(&rel), &external).unwrap();
    let error = apply_update(
        dir.path(),
        &rel,
        &UpdateTaskInput {
            url: url(&rel),
            priority: Some("P1".into()),
            expected: Some(baseline),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.starts_with(TASK_CONFLICT_ERROR), "{error}");
    assert_eq!(read(dir.path(), &rel), external);
}

#[test]
fn matching_expected_hash_writes_and_returns_the_new_hash() {
    let dir = vault();
    let rel = guarded_task(dir.path());
    let baseline = get(dir.path(), &rel).unwrap().content_hash;
    assert_eq!(baseline, content_hash(&read(dir.path(), &rel)));
    let written = apply_update(
        dir.path(),
        &rel,
        &UpdateTaskInput {
            url: url(&rel),
            priority: Some("P2".into()),
            expected: Some(baseline),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(written, content_hash(&read(dir.path(), &rel)));
    assert_eq!(get(dir.path(), &rel).unwrap().content_hash, written);
    // An empty baseline means "no baseline", not "the file must be empty".
    apply_update(
        dir.path(),
        &rel,
        &UpdateTaskInput {
            url: url(&rel),
            priority: Some("P3".into()),
            expected: Some(String::new()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        get(dir.path(), &rel).unwrap().priority.as_deref(),
        Some("P3")
    );
}

/// Notion two-way sync is gone (2026-11-10): a legacy `notion_project_ids`
/// key is history, so a project change saves locally and keeps the key.
#[test]
fn a_project_change_on_a_file_with_legacy_notion_project_ids_saves_locally() {
    let dir = vault();
    write(dir.path(), FULL_REL, FULL);
    apply_update(
        dir.path(),
        FULL_REL,
        &UpdateTaskInput {
            url: url(FULL_REL),
            project: Some("Other".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let task = get(dir.path(), FULL_REL).unwrap();
    assert_eq!(task.project_name.as_deref(), Some("Other"));
    assert!(read(dir.path(), FULL_REL).contains("notion_project_ids:"));
}

#[test]
fn labels_in_a_yaml_list_are_read_and_survive_unrelated_edits() {
    let dir = vault();
    let rel = "tasks/listed.md";
    let file = "---\ntitle: Listed\nstatus: Not started\nlabels:\n  - ui\n  - backend\ncustom: kept\n---\n\nBody.\n";
    write(dir.path(), rel, file);
    assert_eq!(get(dir.path(), rel).unwrap().labels, ["ui", "backend"]);

    // A priority edit leaves the list byte-for-byte.
    set_priority(dir.path(), rel, Some("P2")).unwrap();
    let after = read(dir.path(), rel);
    assert!(
        after.contains("labels:\n  - ui\n  - backend\ncustom: kept\n"),
        "{after}"
    );
    assert!(after.contains("  - label/backend\n"), "{after}");

    // Adding a label keeps the ones the list held (as the comma scalar).
    apply_update(
        dir.path(),
        rel,
        &UpdateTaskInput {
            url: url(rel),
            labels: Some(vec!["ui".into(), "backend".into(), "api".into()]),
            ..Default::default()
        },
    )
    .unwrap();
    let after = read(dir.path(), rel);
    assert!(
        after.contains("\nlabels: ui, backend, api\ncustom: kept\n"),
        "{after}"
    );
    assert_eq!(
        get(dir.path(), rel).unwrap().labels,
        ["ui", "backend", "api"]
    );
}

#[test]
fn a_newline_in_status_or_labels_cannot_end_the_frontmatter_block() {
    let dir = vault();
    let rel = guarded_task(dir.path());
    set_status(dir.path(), &rel, "x\n---\nk: v").unwrap();
    apply_update(
        dir.path(),
        &rel,
        &UpdateTaskInput {
            url: url(&rel),
            labels: Some(vec!["one\n---".into()]),
            ..Default::default()
        },
    )
    .unwrap();
    let after = read(dir.path(), &rel);
    let (block, rest) = split_fence(&after);
    assert!(!block.lines().any(|l| l.trim() == "---"), "{after}");
    assert!(block.contains("  - status/x-----k:-v\n"), "{after}");
    assert_eq!(rest, "\n---\n\nBody\n");
    let t = get(dir.path(), &rel).unwrap();
    assert_eq!(t.title, "Guarded");
    assert_eq!(t.status, "x\n---\nk: v");
}

#[test]
fn an_edit_an_indented_key_would_shadow_is_refused() {
    let dir = vault();
    let rel = "tasks/shadowed.md";
    let file =
        "---\ntitle: Shadowed\nstatus: Not started\nnotes: |\n  status: waiting on legal\n---\n";
    write(dir.path(), rel, file);
    let error = set_status(dir.path(), rel, "Delivered").unwrap_err();
    assert!(error.contains("`status`"), "{error}");
    assert_eq!(read(dir.path(), rel), file);
}

#[test]
fn render_output_is_unchanged_by_the_field_table_refactor() {
    let fm = TaskFrontmatter {
        title: "Write the PR".into(),
        status: "In progress".into(),
        priority: Some("P1".into()),
        due: Some("2026-06-01".into()),
        due_time: Some("14:30".into()),
        duration_min: Some(90),
        effort: Some("M".into()),
        project: Some("Refactor: X".into()),
        project_url: Some("vault:projects/active/Refactor X.md".into()),
        section: Some("Backend".into()),
        labels: vec!["ui".into(), "customer care".into()],
        recurrence: Some("every weekday".into()),
        blocked_by: Vec::new(),
        created: Some("2026-05-27T00:00:00Z".into()),
        last_synced_at: Some("2026-05-27T10:00:00Z".into()),
        source: Some("notion".into()),
        origin: None,
        fingerprint: None,
        notion_id: Some("abc123".into()),
        notion_url: Some("https://notion.so/abc123".into()),
    };
    let header = "---\ntitle: Write the PR\nstatus: In progress\npriority: P1\ndue: 2026-06-01\ndue_time: \"14:30\"\nduration_min: 90\neffort: M\nproject: \"Refactor: X\"\nproject_url: \"vault:projects/active/Refactor X.md\"\nsection: Backend\nlabels: ui, customer care\nrecurrence: every weekday\ncreated: \"2026-05-27T00:00:00Z\"\nsource: notion\nnotion_id: abc123\nnotion_url: \"https://notion.so/abc123\"\nlast_synced_at: \"2026-05-27T10:00:00Z\"\ntags:\n  - tasks\n  - status/in-progress\n  - priority/p1\n  - label/ui\n  - label/customer-care\n---\n";
    assert_eq!(render(&fm, "Body text."), format!("{header}\nBody text.\n"));
    assert_eq!(render(&fm, "  "), header);
}

// ── Paperclip PR 11 (11a): section-aware bodies, idempotent create ──

fn step(text: &str, done: bool) -> SubTask {
    SubTask {
        text: text.into(),
        done,
    }
}

fn task_file(title: &str, body: &str) -> String {
    format!("---\ntitle: {title}\nstatus: Not started\n---\n\n{body}")
}

fn body_of(content: &str) -> String {
    split_frontmatter(content).1
}

/// Lines that differ between two texts (position by position, plus any
/// difference in line count).
fn changed_lines(a: &str, b: &str) -> usize {
    let (la, lb): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    la.iter().zip(lb.iter()).filter(|(x, y)| x != y).count() + la.len().abs_diff(lb.len())
}

fn update(dir: &Path, rel: &str, input: UpdateTaskInput) {
    apply_update(
        dir,
        rel,
        &UpdateTaskInput {
            url: url(rel),
            ..input
        },
    )
    .unwrap();
}

const SECTIONED: &str = "Intro prose.\n\n- [ ] real step\n- [x] done step\n\n## Done when\n\n- [ ] criterion one\nThe release is tagged.\n\n## Activity\n\n- [ ] not a step\n- 2026-09-01 Started.\n";

#[test]
fn reserved_sections_are_excluded_from_subtasks_and_description() {
    let pb = parse_body(SECTIONED);
    assert_eq!(
        pb.subtasks,
        vec![step("real step", false), step("done step", true)]
    );
    assert_eq!(pb.prose, "Intro prose.");
    assert_eq!(
        pb.done_when().as_deref(),
        Some("- [ ] criterion one\nThe release is tagged.")
    );
    assert_eq!(
        pb.activity().as_deref(),
        Some("- [ ] not a step\n- 2026-09-01 Started.")
    );

    let dir = vault();
    write(
        dir.path(),
        "tasks/sectioned.md",
        &task_file("Sectioned", SECTIONED),
    );
    let t = get(dir.path(), "tasks/sectioned.md").unwrap();
    assert_eq!(t.subtasks.len(), 2, "{:?}", t.subtasks);
    assert_eq!(t.description, "Intro prose.");
    assert!(!t.description.contains("Done when") && !t.description.contains("Activity"));
    assert_eq!(
        t.done_when.as_deref(),
        Some("- [ ] criterion one\nThe release is tagged.")
    );
    assert_eq!(
        t.activity.as_deref(),
        Some("- [ ] not a step\n- 2026-09-01 Started.")
    );

    // Heading spellings: trailing colon, closing #s, any case.
    for heading in ["## done when:", "##   DONE WHEN ##", "## Done When"] {
        let pb = parse_body(&format!("{heading}\n\n- [ ] criterion\n"));
        assert!(pb.subtasks.is_empty(), "{heading}");
        assert_eq!(
            pb.done_when().as_deref(),
            Some("- [ ] criterion"),
            "{heading}"
        );
    }
    // An H3 does not open a reserved section.
    assert_eq!(
        parse_body("### Activity\n- [ ] still a step\n")
            .subtasks
            .len(),
        1
    );
}

const NONCANONICAL: &str = "## Done when\n\nShipped.\n\n## Steps\n\n* [ ] first\n  - [ ] indented second\n- [x] third\n\n## Notes\n\nTrailing notes.\n";

#[test]
fn toggling_a_step_in_a_noncanonical_body_is_a_one_line_diff() {
    let pb = parse_body(NONCANONICAL);
    assert_eq!(
        pb.subtasks,
        vec![
            step("first", false),
            step("indented second", false),
            step("third", true)
        ]
    );

    // Toggle: exactly that line changes, keeping its `* ` marker.
    let toggled = splice_subtasks(
        &pb,
        &[
            step("first", true),
            step("indented second", false),
            step("third", true),
        ],
    );
    assert_eq!(changed_lines(NONCANONICAL, &toggled), 1, "{toggled}");
    assert!(toggled.contains("\n* [x] first\n"), "{toggled}");

    // Text edit: only that line, indentation and marker kept.
    let edited = splice_subtasks(
        &pb,
        &[
            step("first", false),
            step("second, edited", false),
            step("third", true),
        ],
    );
    assert_eq!(changed_lines(NONCANONICAL, &edited), 1, "{edited}");
    assert!(edited.contains("\n  - [ ] second, edited\n"), "{edited}");

    // Add: inserted right after the last step line.
    let mut four = pb.subtasks.clone();
    four.push(step("fourth", false));
    let added = splice_subtasks(&pb, &four);
    assert_eq!(
        added,
        NONCANONICAL.replacen("- [x] third\n", "- [x] third\n- [ ] fourth\n", 1)
    );

    // Remove the last: only its line goes.
    let removed = splice_subtasks(&pb, &pb.subtasks[..2]);
    assert_eq!(removed, NONCANONICAL.replacen("- [x] third\n", "", 1));

    // No change: byte-identical.
    assert_eq!(splice_subtasks(&pb, &pb.subtasks), NONCANONICAL);

    // Through update_task, a toggle is a one-line body diff.
    let dir = vault();
    let rel = "tasks/noncanonical.md";
    write(dir.path(), rel, &task_file("Noncanonical", NONCANONICAL));
    update(
        dir.path(),
        rel,
        UpdateTaskInput {
            subtasks: Some(vec![
                step("first", false),
                step("indented second", true),
                step("third", true),
            ]),
            ..Default::default()
        },
    );
    let after = body_of(&read(dir.path(), rel));
    assert_eq!(changed_lines(NONCANONICAL, &after), 1, "{after}");
    assert!(after.contains("\n  - [x] indented second\n"), "{after}");

    // CRLF bodies keep CRLF, also for an inserted step.
    let crlf = "Intro\r\n- [ ] a\r\nOutro\r\n";
    let pb = parse_body(crlf);
    assert_eq!(
        splice_subtasks(&pb, &[step("a", true)]),
        "Intro\r\n- [x] a\r\nOutro\r\n"
    );
    assert_eq!(
        splice_subtasks(&pb, &[step("a", false), step("b", false)]),
        "Intro\r\n- [ ] a\r\n- [ ] b\r\nOutro\r\n"
    );

    // A body with no steps gets them after the leading prose, before the
    // first reserved section, separated by blank lines.
    let pb = parse_body("Intro prose.\n## Activity\n\n- log\n");
    assert_eq!(
        splice_subtasks(&pb, &[step("new", false)]),
        "Intro prose.\n\n- [ ] new\n\n## Activity\n\n- log\n"
    );
}

const ROUND_TRIP: &str = "Intro.\n\n- [ ] one\n- [ ] two\n\n## Done when\n\n- [ ] Tests pass\n\n## Activity\n\n- 2026-09-01 created\n\n## Activity\n\n- 2026-09-02 more\n";
const ROUND_TRIP_SECTIONS: &str = "## Done when\n\n- [ ] Tests pass\n\n## Activity\n\n- 2026-09-01 created\n\n## Activity\n\n- 2026-09-02 more\n";

/// Paperclip PR 11 acceptance: a body with `## Activity` and `## Done when`
/// round-trips through update_task, and their checklist lines never
/// become subtasks.
#[test]
fn update_task_round_trips_a_body_with_activity_and_done_when() {
    let dir = vault();
    let rel = "tasks/round-trip.md";
    let file = task_file("Round trip", ROUND_TRIP);
    write(dir.path(), rel, &file);

    // (a) A status change keeps the body byte for byte.
    set_status(dir.path(), rel, "In progress").unwrap();
    assert_eq!(split_fence(&read(dir.path(), rel)).1, split_fence(&file).1);
    update(
        dir.path(),
        rel,
        UpdateTaskInput {
            status: Some("Not started".into()),
            ..Default::default()
        },
    );
    assert_eq!(split_fence(&read(dir.path(), rel)).1, split_fence(&file).1);

    // (b) A step toggle keeps both sections byte for byte.
    update(
        dir.path(),
        rel,
        UpdateTaskInput {
            subtasks: Some(vec![step("one", true), step("two", false)]),
            ..Default::default()
        },
    );
    let after = read(dir.path(), rel);
    assert!(after.ends_with(ROUND_TRIP_SECTIONS), "{after}");
    assert_eq!(changed_lines(ROUND_TRIP, &body_of(&after)), 1);
    let t = get(dir.path(), rel).unwrap();
    assert_eq!(t.subtasks, vec![step("one", true), step("two", false)]);

    // (c) A body-routed description replacement keeps both sections
    // verbatim, including the repeated `## Activity` heading.
    update(
        dir.path(),
        rel,
        UpdateTaskInput {
            description: Some("New intro.".into()),
            ..Default::default()
        },
    );
    let after = read(dir.path(), rel);
    assert_eq!(
        body_of(&after),
        format!("New intro.\n\n- [x] one\n- [ ] two\n\n{ROUND_TRIP_SECTIONS}")
    );
    assert_eq!(after.matches("\n## Activity\n").count(), 2);
    let t = get(dir.path(), rel).unwrap();
    assert_eq!(t.description, "New intro.");
    assert_eq!(t.subtasks.len(), 2);
    assert_eq!(t.done_when.as_deref(), Some("- [ ] Tests pass"));
    assert_eq!(
        t.activity.as_deref(),
        Some("- 2026-09-01 created\n\n- 2026-09-02 more")
    );

    // A description that itself types a reserved heading can't swallow
    // the steps composed after it.
    update(
        dir.path(),
        rel,
        UpdateTaskInput {
            description: Some("Newer intro.\n\n## Activity\n\n- typed".into()),
            ..Default::default()
        },
    );
    let t = get(dir.path(), rel).unwrap();
    assert_eq!(t.description, "Newer intro.");
    assert_eq!(t.subtasks.len(), 2);
    assert!(
        t.activity.as_deref().unwrap().ends_with("- typed"),
        "{:?}",
        t.activity
    );
}

#[test]
fn canonical_compose_matches_compose_body_when_there_are_no_sections() {
    let subs = vec![step("buy milk", false), step("email boss", true)];
    for prose in ["", "Some prose.", "Line one\nLine two", "  padded  "] {
        assert_eq!(
            compose_task_body(prose, &subs, &[]),
            compose_body(prose, &subs)
        );
        assert_eq!(compose_task_body(prose, &[], &[]), compose_body(prose, &[]));
    }
    for canonical in [
        compose_body("Some prose.", &subs),
        compose_body("", &subs),
        compose_body("Only prose.", &[]),
    ] {
        let pb = parse_body(&canonical);
        assert_eq!(
            compose_task_body(&pb.prose, &pb.subtasks, &pb.sections),
            canonical
        );
    }
}

#[test]
fn fenced_headings_and_checklists_are_literal() {
    let body =
        "Intro\n\n```\n## Activity\n- [ ] x\n```\n\n~~~~ text\n- [ ] y\n~~~~\n\n- [ ] real\n";
    let pb = parse_body(body);
    assert!(pb.sections.is_empty());
    assert_eq!(pb.subtasks, vec![step("real", false)]);
    assert!(
        pb.prose.contains("```\n## Activity\n- [ ] x\n```"),
        "{}",
        pb.prose
    );

    let dir = vault();
    let rel = "tasks/fenced.md";
    write(dir.path(), rel, &task_file("Fenced", body));
    update(
        dir.path(),
        rel,
        UpdateTaskInput {
            subtasks: Some(vec![step("real", true)]),
            ..Default::default()
        },
    );
    assert_eq!(
        body_of(&read(dir.path(), rel)),
        body.replacen("- [ ] real", "- [x] real", 1)
    );
}

#[test]
fn an_unclosed_fence_does_not_swallow_steps_or_sections() {
    let body =
        "Intro\n```\n- [ ] one\n- [ ] two\n\n## Done when\n\nShipped\n\n## Activity\n\n- log\n";
    let pb = parse_body(body);
    assert_eq!(pb.subtasks, vec![step("one", false), step("two", false)]);
    assert_eq!(pb.sections.len(), 2);

    let dir = vault();
    let rel = "tasks/unclosed.md";
    write(dir.path(), rel, &task_file("Unclosed", body));
    update(
        dir.path(),
        rel,
        UpdateTaskInput {
            subtasks: Some(vec![step("one", false), step("two", true)]),
            ..Default::default()
        },
    );
    let t = get(dir.path(), rel).unwrap();
    assert_eq!(t.subtasks, vec![step("one", false), step("two", true)]);
    assert_eq!(t.done_when.as_deref(), Some("Shipped"));
    assert_eq!(t.activity.as_deref(), Some("- log"));

    // A description edit composes the stray fence BEFORE the steps; it
    // stays text, so every step and both sections survive a re-read.
    update(
        dir.path(),
        rel,
        UpdateTaskInput {
            description: Some("Changed intro\n```".into()),
            ..Default::default()
        },
    );
    let t = get(dir.path(), rel).unwrap();
    assert_eq!(t.description, "Changed intro\n```");
    assert_eq!(t.subtasks, vec![step("one", false), step("two", true)]);
    assert_eq!(t.done_when.as_deref(), Some("Shipped"));
    assert_eq!(t.activity.as_deref(), Some("- log"));
}

#[test]
fn non_reserved_headings_stay_in_the_description() {
    let pb = parse_body("# Title\n\n## Context\n\nSome context.\n\n- [ ] step\n");
    assert_eq!(pb.prose, "# Title\n\n## Context\n\nSome context.");
    assert_eq!(pb.subtasks, vec![step("step", false)]);
    assert!(pb.sections.is_empty());
}

#[test]
fn append_to_section_splices_in_place() {
    let entry = "- b";
    // Appended after the last line of the existing section.
    assert_eq!(
        append_to_section(
            "Intro\n\n## Activity\n\n- a\n",
            ReservedKind::Activity,
            entry
        ),
        "Intro\n\n## Activity\n\n- a\n- b\n"
    );
    assert_eq!(
        append_to_section(
            "## Activity\n\n- a\n\n## Notes\n\nx\n",
            ReservedKind::Activity,
            entry
        ),
        "## Activity\n\n- a\n- b\n\n## Notes\n\nx\n"
    );
    // The LAST Activity section receives it.
    assert_eq!(
        append_to_section(
            "## Activity\n\n- a\n\n## Activity\n\n- c\n",
            ReservedKind::Activity,
            entry
        ),
        "## Activity\n\n- a\n\n## Activity\n\n- c\n- b\n"
    );
    // A heading-only section.
    assert_eq!(
        append_to_section("## Activity\n", ReservedKind::Activity, entry),
        "## Activity\n\n- b\n"
    );
    // No section: created after the last reserved section...
    assert_eq!(
        append_to_section(
            "Intro\n\n## Done when\n\nShipped\n\n## Notes\n\nx\n",
            ReservedKind::Activity,
            entry
        ),
        "Intro\n\n## Done when\n\nShipped\n\n## Activity\n\n- b\n\n## Notes\n\nx\n"
    );
    // ...or at the end of the body.
    assert_eq!(
        append_to_section("Intro\n", ReservedKind::Activity, entry),
        "Intro\n\n## Activity\n\n- b\n"
    );
    assert_eq!(
        append_to_section("Intro", ReservedKind::Activity, entry),
        "Intro\n\n## Activity\n\n- b\n"
    );
    assert_eq!(
        append_to_section("", ReservedKind::Activity, entry),
        "## Activity\n\n- b\n"
    );
    // CRLF is kept; a would-be heading is escaped so it can't end the section.
    assert_eq!(
        append_to_section(
            "Intro\r\n\r\n## Activity\r\n\r\n- a\r\n",
            ReservedKind::Activity,
            entry
        ),
        "Intro\r\n\r\n## Activity\r\n\r\n- a\r\n- b\r\n"
    );
    let escaped = append_to_section("## Activity\n\n- a\n", ReservedKind::Activity, "## Sneaky");
    assert_eq!(escaped, "## Activity\n\n- a\n\\## Sneaky\n");
    assert_eq!(
        parse_body(&escaped).activity().as_deref(),
        Some("- a\n\\## Sneaky")
    );
    // Steps elsewhere are untouched and entries are never steps.
    let with_steps = append_to_section("- [ ] step\n", ReservedKind::Activity, "- [ ] logged");
    let pb = parse_body(&with_steps);
    assert_eq!(pb.subtasks, vec![step("step", false)]);
    assert_eq!(pb.activity().as_deref(), Some("- [ ] logged"));

    // A fence line in an entry is escaped too, so it can't pair with a
    // later block's fence and hide the steps and sections in between.
    let body = "## Activity\n\n- a\n\n# Next\n\n- [ ] step\n```\ncode\n```\n";
    let fenced = append_to_section(body, ReservedKind::Activity, "```");
    assert_eq!(fenced, body.replacen("- a\n", "- a\n\\```\n", 1));
    let pb = parse_body(&fenced);
    assert_eq!(pb.subtasks, vec![step("step", false)]);
    assert_eq!(pb.activity().as_deref(), Some("- a\n\\```"));
    assert_eq!(
        append_to_section("## Activity\n", ReservedKind::Activity, "  ~~~ info"),
        "## Activity\n\n\\~~~ info\n"
    );
    // A blank body's first line may be the rest of a CRLF frontmatter
    // fence line: the section goes after it, never ahead of it.
    assert_eq!(
        append_to_section("\r\n", ReservedKind::Activity, entry),
        "\r\n## Activity\r\n\r\n- b\r\n"
    );
}

#[test]
fn a_stray_fence_opener_pairs_with_a_later_fence_line() {
    // The narrower guarantee: an opener with no matching fence line
    // after it is text (see an_unclosed_fence_…), but a stray opener
    // still pairs with the next matching fence line, even one meant to
    // open a later block, as it does in Obsidian. Pinned so nothing
    // relies on the stronger claim.
    let body = "Intro\n```\n- [ ] one\n\n## Activity\n\n```\nlog\n```\n";
    let pb = parse_body(body);
    assert!(pb.subtasks.is_empty(), "{:?}", pb.subtasks);
    assert!(pb.sections.is_empty());
    assert!(pb.prose.contains("- [ ] one"), "{}", pb.prose);
}

#[test]
fn deleting_or_inserting_a_middle_step_touches_only_its_line() {
    // Steps under two headings: deleting the first one removes only its
    // line; the later steps stay under their own heading.
    let two = "## Today\n- [ ] a\n## Later\n- [ ] b\n- [ ] c\n";
    let pb = parse_body(two);
    let deleted = splice_subtasks(&pb, &[step("b", false), step("c", false)]);
    assert_eq!(deleted, "## Today\n## Later\n- [ ] b\n- [ ] c\n");
    assert_eq!(two.lines().count() - deleted.lines().count(), 1);

    // Mixed indentation and markers: each surviving line keeps its bytes.
    let pb = parse_body(NONCANONICAL);
    let without_first =
        splice_subtasks(&pb, &[step("indented second", false), step("third", true)]);
    assert_eq!(without_first, NONCANONICAL.replacen("* [ ] first\n", "", 1));
    let without_middle = splice_subtasks(&pb, &[step("first", false), step("third", true)]);
    assert_eq!(
        without_middle,
        NONCANONICAL.replacen("  - [ ] indented second\n", "", 1)
    );

    // An insert in the middle goes right after its preceding step; one
    // at the front goes right before the first step.
    let middle = splice_subtasks(
        &pb,
        &[
            step("first", false),
            step("new", false),
            step("indented second", false),
            step("third", true),
        ],
    );
    assert_eq!(
        middle,
        NONCANONICAL.replacen("* [ ] first\n", "* [ ] first\n- [ ] new\n", 1)
    );
    let front = splice_subtasks(
        &pb,
        &[
            step("zero", false),
            step("first", false),
            step("indented second", false),
            step("third", true),
        ],
    );
    assert_eq!(
        front,
        NONCANONICAL.replacen("* [ ] first\n", "- [ ] zero\n* [ ] first\n", 1)
    );

    // A delete and a toggle together: two lines, nothing moves.
    let both = splice_subtasks(&pb, &[step("indented second", true), step("third", true)]);
    assert_eq!(
        both,
        NONCANONICAL.replacen("* [ ] first\n", "", 1).replacen(
            "  - [ ] indented second",
            "  - [x] indented second",
            1
        )
    );

    // Duplicate texts pair in order.
    let dup = "- [ ] x\n- [ ] x\n- [ ] y\n";
    let pb = parse_body(dup);
    assert_eq!(
        splice_subtasks(&pb, &[step("x", false), step("y", false)]),
        "- [ ] x\n- [ ] y\n"
    );
}

/// Adding the first step to a body that starts with a reserved section
/// must not glue the step onto the closing frontmatter fence: in a CRLF
/// file (or one whose closing `---` has trailing spaces) the parsed
/// body's first line is the rest of that fence line.
#[test]
fn a_first_step_never_joins_the_closing_frontmatter_fence() {
    let dir = vault();
    for (rel, file, fence) in [
        (
            "tasks/crlf.md",
            "---\r\ntitle: Crlf\r\nstatus: Not started\r\n---\r\n\r\n## Done when\r\n\r\nShipped.\r\n",
            "---\r\n",
        ),
        (
            "tasks/spaces.md",
            "---\ntitle: Spaces\nstatus: Not started\n---  \n\n## Done when\n\nShipped.\n",
            "---  \n",
        ),
    ] {
        write(dir.path(), rel, file);
        update(dir.path(), rel, UpdateTaskInput { subtasks: Some(vec![step("step", false)]), ..Default::default() });
        let after = read(dir.path(), rel);
        let nl = if file.contains("\r\n") { "\r\n" } else { "\n" };
        // The closing fence line is intact and still alone on its line.
        assert!(after.contains(&format!("{nl}{fence}")), "{after:?}");
        assert!(!after.contains("---- ["), "{after:?}");
        assert!(after.contains(&format!("{nl}- [ ] step{nl}")), "{after:?}");
        let t = get(dir.path(), rel).unwrap();
        assert_eq!(t.subtasks, vec![step("step", false)], "{after:?}");
        assert_eq!(t.done_when.as_deref(), Some("Shipped."), "{after:?}");
        assert_eq!(after.split(nl).filter(|l| l.trim_end() == "---").count(), 2, "{after:?}");
    }
    // The same at the splice level: a CRLF body that starts with the rest
    // of the fence line gets the step just before the heading.
    let pb = parse_body("\r\n\r\n## Done when\r\n\r\nShipped.\r\n");
    assert_eq!(
        splice_subtasks(&pb, &[step("step", false)]),
        "\r\n\r\n- [ ] step\r\n\r\n## Done when\r\n\r\nShipped.\r\n"
    );
}

fn keyed(title: &str, key: Option<&str>) -> CreateTaskInput {
    CreateTaskInput {
        title: title.into(),
        idempotency_key: key.map(str::to_string),
        ..Default::default()
    }
}

fn task_file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir.join("tasks"))
        .map(|d| {
            d.filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn create_with_a_key_returns_the_existing_task_instead_of_a_twin() {
    let dir = vault();
    let first = create_from_input(
        dir.path(),
        &keyed("Draft the memo", Some("project-draft:x\ndraft the memo")),
    )
    .unwrap();
    assert!(!first.existing);
    assert_eq!(first.rel, "tasks/draft-the-memo.md");
    assert_eq!(
        first.content_hash,
        content_hash(&read(dir.path(), &first.rel))
    );
    assert_eq!(
        (first.title.as_str(), first.status.as_str()),
        ("Draft the memo", "Not started")
    );
    let second = create_from_input(
        dir.path(),
        &keyed("A different title", Some("project-draft:x\ndraft the memo")),
    )
    .unwrap();
    assert!(second.existing);
    assert_eq!(second.rel, first.rel);
    assert_eq!(second.content_hash, first.content_hash);
    assert_eq!(task_file_names(dir.path()), ["draft-the-memo.md"]);

    // Without a key, two creates are two tasks (titles may repeat).
    create_from_input(dir.path(), &keyed("Draft the memo", None)).unwrap();
    assert_eq!(
        task_file_names(dir.path()),
        ["draft-the-memo-2.md", "draft-the-memo.md"]
    );
}

#[test]
fn concurrent_creates_with_the_same_key_make_one_file() {
    let dir = vault();
    let barrier = std::sync::Barrier::new(8);
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|index| {
                let root = dir.path();
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    create_from_input(
                        root,
                        &keyed(&format!("Shared title {index}"), Some("same-key")),
                    )
                    .unwrap()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    let rels: std::collections::HashSet<&str> = results.iter().map(|r| r.rel.as_str()).collect();
    assert_eq!(rels.len(), 1);
    assert_eq!(results.iter().filter(|r| r.existing).count(), 7);
    assert_eq!(task_file_names(dir.path()).len(), 1);
}

#[test]
fn slug_fallback_fingerprint_precedence_and_key_bounds() {
    let dir = vault();
    let fm = |title: &str, fp: Option<String>| TaskFrontmatter {
        title: title.into(),
        status: "Not started".into(),
        fingerprint: fp,
        ..Default::default()
    };
    // Slug fallback finds an un-fingerprinted task; without it a twin is made.
    write(
        dir.path(),
        "tasks/follow-up.md",
        "---\ntitle: Follow up\nstatus: Delivered\n---\n\nOld.\n",
    );
    let fp_a = fingerprint_for_key("key-a");
    let hit = create_idempotent(
        dir.path(),
        fm("Follow up!", fp_a.clone()),
        "",
        true,
        ExistingPolicy::Return,
    )
    .unwrap();
    assert!(hit.existing);
    assert_eq!(
        (hit.rel.as_str(), hit.status.as_str()),
        ("tasks/follow-up.md", "Delivered")
    );
    let twin = create_idempotent(
        dir.path(),
        fm("Follow up!", fp_a.clone()),
        "",
        false,
        ExistingPolicy::Return,
    )
    .unwrap();
    assert!(!twin.existing);
    assert_eq!(twin.rel, "tasks/follow-up-2.md");

    // A fingerprint match (a renamed task) beats a different slug match.
    let fp_b = fingerprint_for_key("key-b");
    write(
        dir.path(),
        "tasks/original.md",
        "---\ntitle: Original\nstatus: Not started\n---\n",
    );
    write(
        dir.path(),
        "tasks/renamed.md",
        &format!(
            "---\ntitle: Renamed thing\nstatus: Not started\nfingerprint: {}\n---\n",
            fp_b.clone().unwrap()
        ),
    );
    let by_fp = create_idempotent(
        dir.path(),
        fm("Original", fp_b.clone()),
        "",
        true,
        ExistingPolicy::Return,
    )
    .unwrap();
    assert_eq!(by_fp.rel, "tasks/renamed.md");
    // RequireSameTitleSlug refuses a reused key for different work...
    let error = create_idempotent(
        dir.path(),
        fm("Original", fp_b.clone()),
        "",
        true,
        ExistingPolicy::RequireSameTitleSlug,
    )
    .unwrap_err();
    assert!(
        error.starts_with(TASK_IDEMPOTENCY_MISMATCH_ERROR),
        "{error}"
    );
    // ...and returns the task when the slug agrees.
    let same = create_idempotent(
        dir.path(),
        fm("Renamed Thing", fp_b),
        "",
        false,
        ExistingPolicy::RequireSameTitleSlug,
    )
    .unwrap();
    assert_eq!(
        (same.rel.as_str(), same.existing),
        ("tasks/renamed.md", true)
    );
    assert_eq!(
        fingerprint_index(dir.path())
            .get(&fingerprint_for_key("key-b").unwrap())
            .map(String::as_str),
        Some("vault:tasks/renamed.md")
    );

    // A blank key means no fingerprint.
    assert_eq!(fingerprint_for_key("   "), None);
    let blank = create_from_input(dir.path(), &keyed("Blank key", Some("  "))).unwrap();
    assert_eq!(get(dir.path(), &blank.rel).unwrap().fingerprint, None);

    // Keys are bounded at the IPC boundary only; the digest is fixed width.
    let before = task_file_names(dir.path());
    let error = create_from_input(dir.path(), &keyed("Too long", Some(&"k".repeat(20 * 1024))))
        .unwrap_err();
    assert!(error.starts_with(TASK_KEY_ERROR), "{error}");
    assert_eq!(task_file_names(dir.path()), before);
    let ok =
        create_from_input(dir.path(), &keyed("Long key", Some(&"k".repeat(2 * 1024)))).unwrap();
    assert_eq!(
        get(dir.path(), &ok.rel)
            .unwrap()
            .fingerprint
            .map(|f| f.len()),
        Some(64)
    );

    // A blank title is refused, not written as an untitled file.
    assert!(create_from_input(dir.path(), &keyed("   ", None)).is_err());
}

#[test]
fn fingerprint_origin_and_blocked_by_round_trip_through_the_pr3_path() {
    let dir = vault();
    let rel = "tasks/keyed.md";
    write(
        dir.path(),
        "tasks/other.md",
        "---\ntitle: Other\nstatus: Not started\n---\n",
    );
    let file = "---\ntitle: Keyed\nstatus: Not started\npriority: High\nblocked_by: \"vault:tasks/other.md\"\norigin: \"meeting:calendar/x.md\"\nfingerprint: ABC123\ncustom: kept\n---\n\nBody.\n\n- [ ] step\n";
    write(dir.path(), rel, file);
    let kept = |label: &str| {
        let after = read(dir.path(), rel);
        for line in [
            "blocked_by: \"vault:tasks/other.md\"",
            "origin: \"meeting:calendar/x.md\"",
            "fingerprint: ABC123",
            "custom: kept",
        ] {
            assert!(
                after.lines().any(|l| l == line),
                "{label}: lost {line:?} in:\n{after}"
            );
        }
    };
    assert_eq!(migrate_priorities(dir.path()).unwrap(), 1);
    kept("migrate_priorities");
    update(
        dir.path(),
        rel,
        UpdateTaskInput {
            title: Some("Keyed, renamed".into()),
            priority: Some("P3".into()),
            subtasks: Some(vec![step("step", true)]),
            ..Default::default()
        },
    );
    kept("update_task");
    set_status(dir.path(), rel, "In progress").unwrap();
    kept("set_status");
    set_priority(dir.path(), rel, Some("P2")).unwrap();
    kept("set_priority");
    let t = get(dir.path(), rel).unwrap();
    assert_eq!(t.origin.as_deref(), Some("meeting:calendar/x.md"));
    assert_eq!(
        t.fingerprint.as_deref(),
        Some("abc123"),
        "compared lowercase"
    );
    assert_eq!(t.title, "Keyed, renamed");
    assert_eq!(t.blocked_by, ["vault:tasks/other.md"]);

    // A YAML flow-style blocked_by is read as a list, not literal text:
    // `[]` is empty (the task stays ready) and quoted items unquote.
    write(
        dir.path(),
        "tasks/flow-empty.md",
        "---\ntitle: Flow empty\nstatus: Not started\nblocked_by: []\n---\n",
    );
    write(
        dir.path(),
        "tasks/flow.md",
        "---\ntitle: Flow\nstatus: Not started\nblocked_by: [vault:tasks/other.md, \"tasks/keyed.md\"]\n---\n",
    );
    let all = list(dir.path());
    let empty = by_url(&all, "tasks/flow-empty.md");
    assert!(empty.blocked_by.is_empty(), "{:?}", empty.blocked_by);
    assert!(empty.ready && empty.blocker_warnings.is_empty());
    let flow = by_url(&all, "tasks/flow.md");
    assert_eq!(
        flow.blocked_by,
        ["vault:tasks/other.md", "vault:tasks/keyed.md"]
    );
    assert!(
        flow.blocker_warnings.is_empty(),
        "{:?}",
        flow.blocker_warnings
    );
    // The same list sent back unchanged keeps the flow line as written.
    update(
        dir.path(),
        "tasks/flow.md",
        UpdateTaskInput {
            blocked_by: Some(vec![
                "vault:tasks/other.md".into(),
                "vault:tasks/keyed.md".into(),
            ]),
            ..Default::default()
        },
    );
    let after = read(dir.path(), "tasks/flow.md");
    assert!(
        after.contains("\nblocked_by: [vault:tasks/other.md, \"tasks/keyed.md\"]\n"),
        "{after}"
    );

    // A YAML-list blocked_by is read, kept byte for byte by unrelated
    // edits, and rewritten as the comma scalar only when it changes.
    let listed = "tasks/listed.md";
    write(
        dir.path(),
        listed,
        "---\ntitle: Listed\nstatus: Not started\nblocked_by:\n  - vault:tasks/other.md\n  - tasks/keyed.md\ncustom: kept\n---\n",
    );
    assert_eq!(
        get(dir.path(), listed).unwrap().blocked_by,
        ["vault:tasks/other.md", "vault:tasks/keyed.md"]
    );
    update(
        dir.path(),
        listed,
        UpdateTaskInput {
            title: Some("Listed, renamed".into()),
            ..Default::default()
        },
    );
    set_status(dir.path(), listed, "In progress").unwrap();
    // The same list sent back unchanged (bare entry included) writes nothing new.
    update(
        dir.path(),
        listed,
        UpdateTaskInput {
            blocked_by: Some(vec![
                "vault:tasks/other.md".into(),
                "vault:tasks/keyed.md".into(),
            ]),
            ..Default::default()
        },
    );
    let after = read(dir.path(), listed);
    assert!(
        after.contains("blocked_by:\n  - vault:tasks/other.md\n  - tasks/keyed.md\ncustom: kept\n"),
        "{after}"
    );
    update(
        dir.path(),
        listed,
        UpdateTaskInput {
            blocked_by: Some(vec!["vault:tasks/other.md".into()]),
            ..Default::default()
        },
    );
    let after = read(dir.path(), listed);
    assert!(
        after.contains("\nblocked_by: \"vault:tasks/other.md\"\ncustom: kept\n"),
        "{after}"
    );
    assert!(!after.contains("  - vault:tasks/other.md"), "{after}");

    // A new keyed create renders both keys once.
    let created = create_from_input(
        dir.path(),
        &CreateTaskInput {
            title: "Fresh".into(),
            idempotency_key: Some("k".into()),
            origin: Some("project:projects/a.md\nsecond line".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let content = read(dir.path(), &created.rel);
    assert!(
        content.contains("\norigin: \"project:projects/a.md second line\"\n"),
        "{content}"
    );
    assert!(
        content.contains(&format!(
            "\nfingerprint: {}\n",
            fingerprint_for_key("k").unwrap()
        )),
        "{content}"
    );
}

#[test]
fn open_checklist_items_skips_checked_and_fenced_lines() {
    let text = "- [ ] one\n- [x] done\n```\n- [ ] fenced\n```\n```\n- [ ] after a stray fence\n## Activity\n- [ ] in a section\n";
    assert_eq!(
        open_checklist_items(text),
        ["one", "after a stray fence", "in a section"]
    );
}

// ── Paperclip PR 12 (12a): shared task writers ───────────────────────

const RECURRING_REL: &str = "tasks/water-plants.md";
const RECURRING: &str = "---\ntitle: Water plants\nstatus: In progress\ndue: 2026-09-20\nrecurrence: every week\nprevious_due: 2026-09-13\ncustom_key: keep me\n---\n\nBody prose.\n\n- [ ] step\n\n## Activity\n\n- 2026-09-20 10:00 you: noted\n";

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 26).unwrap()
}

fn at(hour: u32, minute: u32) -> DateTime<Local> {
    use chrono::TimeZone;
    Local
        .with_ymd_and_hms(2026, 9, 26, hour, minute, 0)
        .unwrap()
}

#[test]
fn set_status_with_recurrence_rolls_a_recurring_task_forward() {
    let dir = vault();
    write(dir.path(), RECURRING_REL, RECURRING);
    let next = crate::quick_add::next_occurrence("every week", today())
        .unwrap()
        .to_string();

    let outcome =
        set_status_with_recurrence(dir.path(), RECURRING_REL, "Done", today(), None).unwrap();
    let after = read(dir.path(), RECURRING_REL);
    assert_eq!(outcome.status, "Not started");
    assert_eq!(outcome.rolled_to.as_deref(), Some(next.as_str()));
    assert_eq!(outcome.content_hash, content_hash(&after));
    assert!(after.contains(&format!("\ndue: {next}\n")), "{after}");
    assert!(after.contains("\nstatus: Not started\n"), "{after}");
    for kept in [
        "recurrence: every week",
        "previous_due: 2026-09-13",
        "custom_key: keep me",
    ] {
        assert!(
            after.lines().any(|l| l == kept),
            "lost {kept:?} in:\n{after}"
        );
    }
    assert_eq!(
        split_fence(&after).1,
        split_fence(RECURRING).1,
        "body bytes kept"
    );
}

#[test]
fn set_status_with_recurrence_completes_a_non_recurring_task() {
    let dir = vault();
    let rel = "tasks/once.md";
    let file = "---\ntitle: Once\nstatus: Not started\ndue: 2026-09-20\n---\n\nBody.\n";
    write(dir.path(), rel, file);

    let outcome = set_status_with_recurrence(dir.path(), rel, "  Done ", today(), None).unwrap();
    assert_eq!((outcome.status.as_str(), outcome.rolled_to), ("Done", None));
    let task = get(dir.path(), rel).unwrap();
    assert_eq!(
        (task.status.as_str(), task.due.as_deref()),
        ("Done", Some("2026-09-20"))
    );

    // A non-terminal status on a recurring task is written as asked.
    write(dir.path(), RECURRING_REL, RECURRING);
    let moved =
        set_status_with_recurrence(dir.path(), RECURRING_REL, "Waiting", today(), None).unwrap();
    assert_eq!((moved.status.as_str(), moved.rolled_to), ("Waiting", None));
    assert_eq!(
        get(dir.path(), RECURRING_REL).unwrap().due.as_deref(),
        Some("2026-09-20")
    );

    // An empty status is refused and writes nothing.
    let before = read(dir.path(), rel);
    assert!(set_status_with_recurrence(dir.path(), rel, "   ", today(), None).is_err());
    assert_eq!(read(dir.path(), rel), before);
}

#[test]
fn set_status_with_recurrence_refuses_a_stale_expected_hash() {
    let dir = vault();
    write(dir.path(), RECURRING_REL, RECURRING);
    let stale = content_hash(RECURRING);
    // The GUI (or Obsidian) edits the task after the caller read it.
    set_priority(dir.path(), RECURRING_REL, Some("P1")).unwrap();
    let edited = read(dir.path(), RECURRING_REL);

    let error = set_status_with_recurrence(
        dir.path(),
        RECURRING_REL,
        "Done",
        today(),
        Some(stale.as_str()),
    )
    .unwrap_err();
    assert!(error.starts_with(TASK_CONFLICT_ERROR), "{error}");
    assert_eq!(read(dir.path(), RECURRING_REL), edited, "nothing written");

    // The current hash goes through; an empty one means no check.
    let fresh = content_hash(&edited);
    let outcome = set_status_with_recurrence(
        dir.path(),
        RECURRING_REL,
        "Done",
        today(),
        Some(fresh.as_str()),
    )
    .unwrap();
    assert!(outcome.rolled_to.is_some());
    set_status_with_recurrence(dir.path(), RECURRING_REL, "In progress", today(), Some(""))
        .unwrap();
}

#[test]
fn set_task_status_uses_the_shared_roll_forward() {
    // The command and PR 12's set_status tool must share one writer: the
    // old command read the recurrence outside the mutate lock.
    let source = include_str!("commands.rs");
    let start = source
        .find("pub async fn set_task_status(")
        .expect("command present");
    let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
    assert!(
        body.contains("vault_tasks::set_status_with_recurrence("),
        "{body}"
    );
    assert!(!body.contains("next_occurrence"), "{body}");
    assert!(!body.contains("vault_tasks::mutate("), "{body}");
}

/// Review F01 (2026-10-13): a bulk priority change, then a newer edit
/// from elsewhere, then the bulk Undo. The Undo carries the hash its
/// apply returned, so it is refused and the newer edit stays on disk.
#[test]
fn priority_undo_with_the_applied_hash_never_overwrites_a_newer_edit() {
    let dir = vault();
    write(dir.path(), RECURRING_REL, RECURRING);
    let previewed = content_hash(RECURRING);

    // Apply checks the previewed baseline and returns the next one.
    let applied =
        set_priority_checked(dir.path(), RECURRING_REL, Some("P1"), Some(&previewed)).unwrap();
    // Another surface (or an outside editor) changes it again.
    set_priority(dir.path(), RECURRING_REL, Some("P3")).unwrap();
    let newer = read(dir.path(), RECURRING_REL);
    assert!(newer.contains("priority: P3"), "{newer}");

    let error = set_priority_checked(dir.path(), RECURRING_REL, None, Some(&applied)).unwrap_err();
    assert!(error.starts_with(TASK_CONFLICT_ERROR), "{error}");
    assert_eq!(
        read(dir.path(), RECURRING_REL),
        newer,
        "the newer edit survives"
    );

    // A stale preview is refused the same way, and the file is untouched.
    let error =
        set_priority_checked(dir.path(), RECURRING_REL, Some("P2"), Some(&previewed)).unwrap_err();
    assert!(error.starts_with(TASK_CONFLICT_ERROR), "{error}");
    assert_eq!(read(dir.path(), RECURRING_REL), newer);

    // With the current hash the Undo goes through; empty means no check.
    let current = content_hash(&newer);
    set_priority_checked(dir.path(), RECURRING_REL, None, Some(&current)).unwrap();
    let cleared = read(dir.path(), RECURRING_REL);
    assert!(!cleared.contains("priority:"), "{cleared}");
    assert!(
        cleared.contains("custom_key: keep me"),
        "unmodelled keys survive"
    );
    set_priority_checked(dir.path(), RECURRING_REL, Some("P4"), Some("")).unwrap();
    assert!(read(dir.path(), RECURRING_REL).contains("priority: P4"));
}

/// Review F01: a bulk status Undo after a newer edit is refused.
#[test]
fn status_undo_with_the_applied_hash_never_overwrites_a_newer_edit() {
    let dir = vault();
    write(
        dir.path(),
        FULL_REL,
        "---\ntitle: Ship it\nstatus: Not started\n---\n\nBody.\n",
    );
    let previewed = content_hash(&read(dir.path(), FULL_REL));
    let applied =
        set_status_with_recurrence(dir.path(), FULL_REL, "Delivered", today(), Some(&previewed))
            .unwrap();
    assert_eq!(applied.status, "Delivered");
    // Someone reopens it to In progress before the Undo.
    set_status(dir.path(), FULL_REL, "In progress").unwrap();
    let newer = read(dir.path(), FULL_REL);

    let error = set_status_with_recurrence(
        dir.path(),
        FULL_REL,
        "Not started",
        today(),
        Some(&applied.content_hash),
    )
    .unwrap_err();
    assert!(error.starts_with(TASK_CONFLICT_ERROR), "{error}");
    assert_eq!(read(dir.path(), FULL_REL), newer, "In progress survives");
}

#[test]
fn task_status_and_priority_commands_pass_the_expected_hash_through() {
    let source = include_str!("commands.rs");
    for (command, writer) in [
        (
            "pub async fn set_task_status(",
            "vault_tasks::set_status_with_recurrence(",
        ),
        (
            "pub async fn set_task_priority(",
            "vault_tasks::set_priority_checked(",
        ),
    ] {
        let start = source.find(command).expect("command present");
        let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
        assert!(
            crate::source_pins::contains(body, "expected: Option<String>"),
            "{body}"
        );
        assert!(crate::source_pins::contains(body, writer), "{body}");
        assert!(
            crate::source_pins::contains(body, "expected.as_deref()"),
            "{body}"
        );
    }
}

#[test]
fn create_from_input_normalizes_like_the_create_command() {
    let dir = vault();
    write(
        dir.path(),
        "projects/Active/Alpha.md",
        "---\ntitle: Alpha\n---\n",
    );
    let created = create_from_input(
        dir.path(),
        &CreateTaskInput {
            title: "  Normalize me ".into(),
            priority: Some("High".into()),
            labels: vec![" a ".into(), "A".into(), "".into(), "b".into()],
            project_url: Some("vault:projects/Active/Alpha.md".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let task = get(dir.path(), &created.rel).unwrap();
    assert_eq!(task.title, "Normalize me");
    assert_eq!(task.status, "Not started");
    assert_eq!(task.priority.as_deref(), Some("P1"));
    assert_eq!(task.labels, ["a", "b"]);
    assert_eq!(task.project_name.as_deref(), Some("Alpha"));
    assert_eq!(
        task.project_url.as_deref(),
        Some("vault:projects/Active/Alpha.md")
    );
    let (fm, _) = split_frontmatter(&read(dir.path(), &created.rel));
    assert!(fm.contains_key("created"));
    assert_eq!(fm.get("created"), fm.get("last_synced_at"));
    // An unresolvable project is refused, not stored as a dangling link.
    let dangling = CreateTaskInput {
        title: "Dangling".into(),
        project_url: Some("vault:notes/x.md".into()),
        ..Default::default()
    };
    assert!(create_from_input(dir.path(), &dangling).is_err());
}

#[test]
fn agent_creates_scope_keys_by_conversation_and_refuse_a_mismatch() {
    let dir = vault();
    let agent = |title: &str, key: Option<&str>, surface: &str, scope: &str| {
        create_agent_task(
            dir.path(),
            &keyed(title, key),
            surface,
            scope,
            "run-20260926-1",
        )
    };

    let first = agent("Draft the memo", Some("1"), "chat", "s-a").unwrap();
    assert!(!first.existing);
    let content = read(dir.path(), &first.rel);
    assert!(
        content.contains("\norigin: \"run:run-20260926-1\"\n")
            || content.contains("\norigin: run:run-20260926-1\n"),
        "{content}"
    );
    let expected_fp = fingerprint_for_key(&scoped_agent_key("chat", "s-a", "1").unwrap()).unwrap();
    assert_eq!(
        get(dir.path(), &first.rel).unwrap().fingerprint.as_deref(),
        Some(expected_fp.as_str())
    );

    // The same generic key in another conversation, or on another
    // surface with the same scope, is new work.
    let other = agent("Book the venue", Some("1"), "chat", "s-b").unwrap();
    assert!(!other.existing);
    let workflow = agent("Draft the memo", Some("1"), "workflow", "s-a").unwrap();
    assert!(!workflow.existing);
    assert_ne!(workflow.rel, first.rel);

    // A retry in the first conversation returns its task, title and status.
    let retry = agent("draft the memo!", Some(" 1 "), "chat", "s-a").unwrap();
    assert!(retry.existing);
    assert_eq!(retry.rel, first.rel);
    assert_eq!(
        (retry.title.as_str(), retry.status.as_str()),
        ("Draft the memo", "Not started")
    );
    assert_eq!(
        retry.content_hash,
        content_hash(&read(dir.path(), &first.rel))
    );

    // The key reused for different work is refused and writes nothing.
    let before = task_file_names(dir.path());
    let error = agent("Something else", Some("1"), "chat", "s-a").unwrap_err();
    assert!(
        error.starts_with(TASK_IDEMPOTENCY_MISMATCH_ERROR),
        "{error}"
    );
    assert!(
        error.contains(&first.rel),
        "names the existing task: {error}"
    );

    // A key is required.
    for key in [None, Some(""), Some("   ")] {
        let error = agent("No key", key, "chat", "s-a").unwrap_err();
        assert!(error.starts_with(TASK_KEY_ERROR), "{error}");
    }
    let huge = "k".repeat(TASK_KEY_MAX_BYTES + 1);
    let error = agent("Huge", Some(huge.as_str()), "chat", "s-a").unwrap_err();
    assert!(error.starts_with(TASK_KEY_ERROR), "{error}");
    assert_eq!(task_file_names(dir.path()), before);

    // Key shape: fixed width, scoped, deterministic.
    let key = scoped_agent_key("chat", "s-a", "1").unwrap();
    assert!(
        key.starts_with("agent:") && key.len() == "agent:".len() + 32,
        "{key}"
    );
    assert_eq!(scoped_agent_key("chat", "s-a", " 1 "), Some(key.clone()));
    assert_ne!(scoped_agent_key("chat", "s-b", "1"), Some(key.clone()));
    assert_ne!(scoped_agent_key("workflow", "s-a", "1"), Some(key));
    assert_eq!(scoped_agent_key("chat", "s-a", "  "), None);
}

#[test]
fn concurrent_agent_creates_with_one_key_create_one_file() {
    let dir = vault();
    let barrier = std::sync::Barrier::new(4);
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let root = dir.path();
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    create_agent_task(root, &keyed("Shared", Some("k")), "chat", "s-a", "run-1")
                        .unwrap()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|r| !r.existing).count(), 1);
    assert_eq!(results.iter().filter(|r| r.existing).count(), 3);
    assert_eq!(task_file_names(dir.path()), ["shared.md"]);
}

const ACTIVITY_REL: &str = "tasks/with-sections.md";
const WITH_SECTIONS: &str = "---\ntitle: With sections\nstatus: Not started\ncustom: x\n---\n\nIntro.\n\n- [ ] one\n\n## Done when\n\nShipped.\n";

#[test]
fn append_activity_adds_timestamped_lines_under_the_activity_section() {
    let dir = vault();
    write(dir.path(), ACTIVITY_REL, WITH_SECTIONS);

    let first = append_activity(
        dir.path(),
        ACTIVITY_REL,
        "agent (chat)",
        "Started the draft.",
        at(14, 5),
        None,
        None,
    )
    .unwrap();
    assert!(!first.duplicate);
    let after = read(dir.path(), ACTIVITY_REL);
    assert_eq!(first.content_hash, content_hash(&after));
    assert_eq!(
        split_fence(&after).0,
        split_fence(WITH_SECTIONS).0,
        "frontmatter bytes kept"
    );
    assert!(
        after.ends_with("## Done when\n\nShipped.\n\n## Activity\n\n- 2026-09-26 14:05 agent (chat): Started the draft.\n"),
        "{after}"
    );

    let second = append_activity(
        dir.path(),
        ACTIVITY_REL,
        "agent (chat)",
        "Sent it.",
        at(15, 30),
        Some(first.content_hash.as_str()),
        None,
    )
    .unwrap();
    let after = read(dir.path(), ACTIVITY_REL);
    assert_eq!(second.content_hash, content_hash(&after));
    assert!(
        after.starts_with(&WITH_SECTIONS[..WITH_SECTIONS.len() - 1]),
        "earlier bytes kept:\n{after}"
    );
    let task = get(dir.path(), ACTIVITY_REL).unwrap();
    assert_eq!(task.subtasks, vec![step("one", false)]);
    assert_eq!(task.description, "Intro.");
    assert_eq!(task.done_when.as_deref(), Some("Shipped."));
    assert_eq!(
        task.activity.as_deref(),
        Some("- 2026-09-26 14:05 agent (chat): Started the draft.\n- 2026-09-26 15:30 agent (chat): Sent it.")
    );

    // A stale baseline is a conflict and writes nothing.
    let error = append_activity(
        dir.path(),
        ACTIVITY_REL,
        "agent",
        "Late.",
        at(16, 0),
        Some(first.content_hash.as_str()),
        None,
    )
    .unwrap_err();
    assert!(error.starts_with(TASK_CONFLICT_ERROR), "{error}");
    assert_eq!(read(dir.path(), ACTIVITY_REL), after);
}

#[test]
fn append_activity_with_a_marker_is_idempotent() {
    let dir = vault();
    write(dir.path(), ACTIVITY_REL, WITH_SECTIONS);
    let marker = activity_marker("chat", "s-a", "progress-1").unwrap();
    assert_eq!(marker.len(), 16);
    assert_eq!(
        activity_marker("chat", "s-a", " progress-1 ").as_deref(),
        Some(marker.as_str())
    );
    assert_ne!(
        activity_marker("chat", "s-b", "progress-1").as_deref(),
        Some(marker.as_str())
    );
    assert_eq!(activity_marker("chat", "s-a", " "), None);

    let first = append_activity(
        dir.path(),
        ACTIVITY_REL,
        "agent (chat)",
        "Halfway.",
        at(9, 0),
        None,
        Some(marker.as_str()),
    )
    .unwrap();
    let written = read(dir.path(), ACTIVITY_REL);
    let again = append_activity(
        dir.path(),
        ACTIVITY_REL,
        "agent (chat)",
        "Halfway.",
        at(9, 1),
        None,
        Some(marker.as_str()),
    )
    .unwrap();
    assert!(!first.duplicate);
    assert!(again.duplicate);
    assert_eq!(again.content_hash, first.content_hash);
    assert_eq!(read(dir.path(), ACTIVITY_REL), written, "bytes unchanged");
    let tag = format!("<!-- hollow-activity: {marker} -->");
    assert_eq!(written.matches(&tag).count(), 1, "{written}");
    assert!(
        written.contains(&format!("agent (chat): Halfway. {tag}\n")),
        "{written}"
    );

    // A retry whose first attempt landed but whose response was lost
    // still carries the pre-write baseline. The marker wins over that
    // (now stale) baseline: a duplicate, not a conflict.
    let before = content_hash(WITH_SECTIONS);
    let retried = append_activity(
        dir.path(),
        ACTIVITY_REL,
        "agent (chat)",
        "Halfway.",
        at(9, 3),
        Some(before.as_str()),
        Some(marker.as_str()),
    )
    .unwrap();
    assert!(retried.duplicate);
    assert_eq!(retried.content_hash, first.content_hash);
    assert_eq!(read(dir.path(), ACTIVITY_REL), written, "bytes unchanged");

    // Another key is another entry.
    let other = activity_marker("chat", "s-a", "progress-2").unwrap();
    assert!(
        !append_activity(
            dir.path(),
            ACTIVITY_REL,
            "agent (chat)",
            "Done.",
            at(9, 2),
            None,
            Some(other.as_str())
        )
        .unwrap()
        .duplicate
    );
}

#[test]
fn append_activity_keeps_a_closing_fence_with_no_newline_after_it() {
    let dir = vault();
    write(dir.path(), "tasks/bare.md", "---\ntitle: A\n---");
    append_activity(
        dir.path(),
        "tasks/bare.md",
        "agent",
        "Hi.",
        at(10, 0),
        None,
        None,
    )
    .unwrap();
    let after = read(dir.path(), "tasks/bare.md");
    assert_eq!(
        after,
        "---\ntitle: A\n---\n## Activity\n\n- 2026-09-26 10:00 agent: Hi.\n"
    );
    assert_eq!(get(dir.path(), "tasks/bare.md").unwrap().title, "A");

    write(dir.path(), "tasks/crlf.md", "---\r\ntitle: B\r\n---");
    append_activity(
        dir.path(),
        "tasks/crlf.md",
        "agent",
        "Hi.",
        at(10, 1),
        None,
        None,
    )
    .unwrap();
    let after = read(dir.path(), "tasks/crlf.md");
    assert!(
        after.starts_with("---\r\ntitle: B\r\n---\r\n## Activity"),
        "{after:?}"
    );
}

#[test]
fn activity_line_escapes_checklists_and_headings() {
    let line = activity_line(
        "agent\n(chat)",
        "- [ ] first\r\n- [ ] x\n* [x] y\n+ [ ] z\n\n## H\n# top\nplain",
        at(8, 7),
        None,
    )
    .unwrap();
    assert_eq!(
        line,
        "- 2026-09-26 08:07 agent (chat): - \\[ ] first\n    - \\[ ] x\n    * \\[x] y\n    + \\[ ] z\n\n    \\## H\n    \\# top\n    plain"
    );

    // In a task, none of it becomes a step and the section stays whole.
    let dir = vault();
    write(dir.path(), ACTIVITY_REL, WITH_SECTIONS);
    append_activity(
        dir.path(),
        ACTIVITY_REL,
        "agent",
        "Notes:\n- [ ] x\n## H\nafter",
        at(8, 8),
        None,
        None,
    )
    .unwrap();
    let task = get(dir.path(), ACTIVITY_REL).unwrap();
    assert_eq!(task.subtasks, vec![step("one", false)]);
    let activity = task.activity.unwrap();
    assert!(activity.ends_with("    \\## H\n    after"), "{activity}");
    assert_eq!(
        parse_subtasks(&read(dir.path(), ACTIVITY_REL)),
        vec![step("one", false)]
    );

    // Bounds.
    assert!(activity_line("agent", "  \n ", at(8, 9), None).is_err());
    assert!(activity_line("agent", &"x".repeat(ACTIVITY_MAX_CHARS + 1), at(8, 9), None).is_err());
    assert!(activity_line(
        "agent",
        &vec!["x"; ACTIVITY_MAX_LINES + 1].join("\n"),
        at(8, 9),
        None
    )
    .is_err());
    assert!(activity_line("agent", "ok", at(8, 9), Some("NOT-HEX")).is_err());
    assert!(activity_line("", "ok", at(8, 9), None)
        .unwrap()
        .starts_with("- 2026-09-26 08:09 agent: ok"));
}

#[test]
fn resolve_task_ref_rejects_paths_outside_tasks() {
    let dir = vault();
    write(dir.path(), "tasks/a.md", "---\ntitle: A\n---\n");
    write(dir.path(), "notes/a.md", "---\ntitle: Note\n---\n");
    std::fs::write(
        dir.path().join("tasks/.hidden.md"),
        "---\ntitle: Hidden\n---\n",
    )
    .unwrap();

    for ok in ["vault:tasks/a.md", "tasks/a.md", "  vault:tasks/a.md "] {
        assert_eq!(
            resolve_task_ref(dir.path(), ok).unwrap(),
            "tasks/a.md",
            "{ok}"
        );
    }
    for bad in [
        "vault:../x.md",
        "vault:notes/a.md",
        "tasks/.hidden.md",
        "tasks/missing.md",
        "tasks/../tasks/a.md",
        "tasks/./a.md",
        "tasks//a.md",
        "tasks/a.txt",
        "/tasks/a.md",
        "tasks\\a.md",
        "",
    ] {
        assert!(
            resolve_task_ref(dir.path(), bad).is_err(),
            "{bad:?} must be refused"
        );
    }

    // A symlink under tasks/ must not aim a task write elsewhere in the
    // vault (me/, watched dot-state), nor may a symlinked subfolder.
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        write(
            dir.path(),
            "me/Guardrails.md",
            "---\ntitle: Guardrails\n---\n",
        );
        symlink(
            dir.path().join("me/Guardrails.md"),
            dir.path().join("tasks/link.md"),
        )
        .unwrap();
        symlink(dir.path().join("notes"), dir.path().join("tasks/sub")).unwrap();
        symlink(
            dir.path().join("tasks/a.md"),
            dir.path().join("tasks/alias.md"),
        )
        .unwrap();
        for bad in ["tasks/link.md", "tasks/sub/a.md", "tasks/alias.md"] {
            let error = resolve_task_ref(dir.path(), bad).unwrap_err();
            assert!(error.contains("link"), "{bad}: {error}");
        }
    }
}

#[test]
fn held_modal_fields_adds_due_time_when_a_held_due_has_a_time() {
    use crate::tracker::ipc::{TrackerLock, TrackerLockKind, TrackerTaskMeta};
    let dir = vault();
    let task_with = |due: &str, fields: Option<&[&str]>| {
        let content = format!("---\ntitle: Linked\nstatus: Not started\ndue: \"{due}\"\n---\n");
        let mut task = parse("tasks/linked.md", &content, dir.path()).unwrap();
        task.tracker = fields.map(|fields| TrackerTaskMeta {
            provider: "linear".into(),
            id: "issue-1".into(),
            key: None,
            url: None,
            status_type: None,
            priority: 0,
            lock: TrackerLock {
                kind: TrackerLockKind::Linked,
                fields: fields.iter().map(|f| f.to_string()).collect(),
            },
        });
        task
    };
    let held = |task: &Task| held_modal_fields(task).into_iter().collect::<Vec<_>>();

    assert_eq!(
        held(&task_with(
            "2026-09-30T15:00:00.000-04:00",
            Some(&["title", "due"][..])
        )),
        ["due", "dueTime", "title"]
    );
    assert_eq!(
        held(&task_with("2026-09-30", Some(&["title", "due"][..]))),
        ["due", "title"]
    );
    // No due held: the time is Hollow's to set.
    assert_eq!(
        held(&task_with(
            "2026-09-30T15:00:00.000-04:00",
            Some(&["project"][..])
        )),
        ["project"]
    );
    assert!(held(&task_with("2026-09-30T15:00", None)).is_empty());
    assert!(held(&task_with("2026-09-30T15:00", None)).is_empty());
    // Unknown names are ignored rather than trusted.
    assert_eq!(
        held(&task_with("2026-09-30", Some(&["bogus", "status"][..]))),
        ["status"]
    );
}

#[test]
fn field_validators_accept_only_canonical_values() {
    assert_eq!(
        valid_due(Some(" 2026-09-30 ".into())).as_deref(),
        Some("2026-09-30")
    );
    assert_eq!(valid_due(Some("next Friday".into())), None);
    assert_eq!(valid_due(Some("2026-02-30".into())), None);
    assert_eq!(valid_priority(Some(" p2 ".into())).as_deref(), Some("P2"));
    assert_eq!(valid_priority(Some("urgent".into())), None);
    assert_eq!(one_line("  a\n  b\tc ", 4), "a b ");
    assert_eq!(valid_clock(" 09:05 ").as_deref(), Some("09:05"));
    assert_eq!(valid_clock("23:59").as_deref(), Some("23:59"));
    for bad in [
        "24:00", "12:60", "4pm", "9:05", "09-05", "0905", "+1:05", "12:5a", "",
    ] {
        assert_eq!(valid_clock(bad), None, "{bad:?}");
    }
}

// ── Paperclip PR 11 (11b): blockers, readiness, cycles ──

/// A task file with a status and an optional `blocked_by` scalar.
fn blocked_task(dir: &Path, rel: &str, title: &str, status: &str, blocked_by: &str) {
    let line = if blocked_by.is_empty() {
        String::new()
    } else {
        format!("blocked_by: \"{blocked_by}\"\n")
    };
    write(
        dir,
        rel,
        &format!("---\ntitle: {title}\nstatus: {status}\n{line}---\n"),
    );
}

fn by_url<'a>(tasks: &'a [Task], rel: &str) -> &'a Task {
    tasks
        .iter()
        .find(|t| t.url == format!("vault:{rel}"))
        .unwrap_or_else(|| panic!("no task {rel}"))
}

fn warning_kinds(task: &Task) -> Vec<&str> {
    task.blocker_warnings
        .iter()
        .map(|w| w.kind.as_str())
        .collect()
}

fn set_blockers(dir: &Path, rel: &str, blockers: &[&str]) -> Result<String, String> {
    apply_update(
        dir,
        rel,
        &UpdateTaskInput {
            url: url(rel),
            blocked_by: Some(blockers.iter().map(|b| b.to_string()).collect()),
            ..Default::default()
        },
    )
}

#[test]
fn list_computes_blocks_ready_and_unresolved_blockers() {
    let dir = vault();
    let d = dir.path();
    blocked_task(d, "tasks/b.md", "B", "In progress", "");
    blocked_task(d, "tasks/a.md", "A", "Not started", "vault:tasks/b.md");
    blocked_task(d, "tasks/d.md", "D", "Done", "");
    blocked_task(d, "tasks/c.md", "C", "Not started", "vault:tasks/d.md");
    blocked_task(d, "tasks/e.md", "E", "Cancelled", "vault:tasks/d.md");
    blocked_task(d, "tasks/f.md", "F", "On hold", "vault:tasks/d.md");
    blocked_task(d, "tasks/g.md", "G", "Not started", "");
    let tasks = list(d);

    let a = by_url(&tasks, "tasks/a.md");
    assert!(!a.ready, "blocked by an open task");
    assert_eq!(a.unresolved_blockers, ["vault:tasks/b.md"]);
    assert!(
        a.blocker_warnings.is_empty(),
        "an open blocker is not a warning"
    );
    assert_eq!(by_url(&tasks, "tasks/b.md").blocks, ["vault:tasks/a.md"]);
    assert!(by_url(&tasks, "tasks/b.md").ready, "no blockers of its own");

    let c = by_url(&tasks, "tasks/c.md");
    assert!(c.ready, "its only blocker is Done");
    assert!(c.unresolved_blockers.is_empty());
    assert_eq!(
        by_url(&tasks, "tasks/d.md").blocks,
        ["vault:tasks/c.md", "vault:tasks/e.md", "vault:tasks/f.md"],
        "sorted"
    );
    assert!(
        !by_url(&tasks, "tasks/d.md").ready,
        "a Done task is not ready"
    );
    assert!(!by_url(&tasks, "tasks/e.md").ready, "own status Cancelled");
    assert!(!by_url(&tasks, "tasks/f.md").ready, "own status On hold");
    assert!(by_url(&tasks, "tasks/g.md").ready, "no blockers, open");

    // `get` parses blocked_by but computes none of the list-only fields.
    let single = get(d, "tasks/a.md").unwrap();
    assert_eq!(single.blocked_by, ["vault:tasks/b.md"]);
    assert!(
        !single.ready
            && single.unresolved_blockers.is_empty()
            && single.blocker_warnings.is_empty()
    );
    assert!(get(d, "tasks/b.md").unwrap().blocks.is_empty());
}

#[test]
fn never_resolving_blockers_warn() {
    let dir = vault();
    let d = dir.path();
    for (rel, status) in [
        ("tasks/c1.md", "Cancelled"),
        ("tasks/c2.md", "canceled"),
        ("tasks/c3.md", "Won\u{2019}t do"),
        ("tasks/c4.md", "WONT DO"),
    ] {
        blocked_task(d, rel, rel, status, "");
    }
    write(
        d,
        "tasks/weekly.md",
        "---\ntitle: Weekly\nstatus: Not started\nrecurrence: every week\n---\n",
    );
    // Archived out of tasks/: the link names nothing now.
    write(
        d,
        "archive/tasks/2026-09-01/old.md",
        "---\ntitle: Old\nstatus: Not started\n---\n",
    );
    blocked_task(
        d,
        "tasks/waiting.md",
        "Waiting",
        "Not started",
        "vault:tasks/c1.md, vault:tasks/c2.md, vault:tasks/c3.md, vault:tasks/c4.md, vault:tasks/old.md, vault:tasks/weekly.md",
    );
    let tasks = list(d);
    let t = by_url(&tasks, "tasks/waiting.md");
    assert_eq!(
        warning_kinds(t),
        [
            "cancelled",
            "cancelled",
            "cancelled",
            "cancelled",
            "missing",
            "recurring"
        ]
    );
    assert_eq!(t.blocker_warnings[4].url, "vault:tasks/old.md");
    assert_eq!(t.unresolved_blockers.len(), 6, "all stay unresolved");
    assert!(!t.ready);

    assert!(is_cancelled_status(" won't  do ") && is_cancelled_status("Won\u{2018}t do"));
    assert!(!is_cancelled_status("Cancel later") && !is_cancelled_status("Done"));
    assert!(
        is_hold_status("Waiting on Sam") && is_hold_status("on-hold") && is_hold_status("Blocked")
    );
    assert!(!is_hold_status("Not started") && !is_hold_status("Holdings review"));
}

#[test]
fn blocker_references_resolve_case_insensitively_on_read() {
    let dir = vault();
    let d = dir.path();
    blocked_task(
        d,
        "tasks/foo.md",
        "Foo",
        "In progress",
        "vault:tasks/Bar.md",
    );
    blocked_task(
        d,
        "tasks/bar.md",
        "Bar",
        "Not started",
        "vault:tasks/Foo.md",
    );
    let tasks = list(d);
    let bar = by_url(&tasks, "tasks/bar.md");
    assert!(
        !warning_kinds(bar).contains(&"missing"),
        "{:?}",
        bar.blocker_warnings
    );
    assert_eq!(
        bar.unresolved_blockers,
        ["vault:tasks/Foo.md"],
        "counts toward readiness"
    );
    assert_eq!(
        by_url(&tasks, "tasks/foo.md").blocks,
        ["vault:tasks/bar.md"]
    );
    // And participates in cycle detection: foo ↔ bar.
    assert_eq!(warning_kinds(bar), ["cycle"]);
    assert_eq!(warning_kinds(by_url(&tasks, "tasks/foo.md")), ["cycle"]);

    // An ambiguous case-insensitive match resolves to nothing.
    let two = vec![
        Task {
            url: "vault:tasks/X.md".into(),
            ..Default::default()
        },
        Task {
            url: "vault:tasks/x.MD".into(),
            ..Default::default()
        },
    ];
    let index = BlockerIndex::new(&two);
    assert_eq!(index.resolve("vault:tasks/X.md"), Some(0), "exact first");
    assert_eq!(index.resolve("vault:tasks/x.md"), None, "ambiguous");
}

/// Paperclip PR 11 acceptance: cycle rejection.
#[test]
fn update_rejects_blocker_cycles_and_writes_nothing() {
    let dir = vault();
    let d = dir.path();
    for (rel, title) in [
        ("tasks/a.md", "Alpha"),
        ("tasks/b.md", "Beta"),
        ("tasks/c.md", "Gamma"),
    ] {
        blocked_task(d, rel, title, "Not started", "");
    }
    set_blockers(d, "tasks/a.md", &["vault:tasks/b.md"]).unwrap();

    // 2-cycle.
    let before = read(d, "tasks/b.md");
    let error = set_blockers(d, "tasks/b.md", &["vault:tasks/a.md"]).unwrap_err();
    assert!(error.starts_with(TASK_CYCLE_ERROR), "{error}");
    assert!(error.contains("Beta → Alpha → Beta"), "{error}");
    assert_eq!(read(d, "tasks/b.md"), before);

    // 3-cycle.
    set_blockers(d, "tasks/b.md", &["vault:tasks/c.md"]).unwrap();
    let before = read(d, "tasks/c.md");
    let error = set_blockers(d, "tasks/c.md", &["vault:tasks/a.md"]).unwrap_err();
    assert!(error.starts_with(TASK_CYCLE_ERROR), "{error}");
    assert!(error.contains("Gamma → Alpha → Beta → Gamma"), "{error}");
    assert_eq!(read(d, "tasks/c.md"), before);

    // Self-block, also next to a valid entry.
    let before = read(d, "tasks/a.md");
    let error =
        set_blockers(d, "tasks/a.md", &["vault:tasks/b.md", "vault:tasks/a.md"]).unwrap_err();
    assert!(error.starts_with(TASK_CYCLE_ERROR), "{error}");
    assert_eq!(read(d, "tasks/a.md"), before);

    // Clearing is always allowed.
    set_blockers(d, "tasks/b.md", &[]).unwrap();
    assert!(!read(d, "tasks/b.md").contains("blocked_by"));
    assert!(get(d, "tasks/b.md").unwrap().blocked_by.is_empty());
}

#[test]
fn create_never_rebinds_a_dangling_blocker_or_closes_a_cycle() {
    let dir = vault();
    let d = dir.path();
    // B names tasks/x.md, which no longer exists.
    blocked_task(d, "tasks/b.md", "B", "Not started", "vault:tasks/x.md");
    let created = create_from_input(
        d,
        &CreateTaskInput {
            title: "X".into(),
            blocked_by: vec!["tasks/b.md".into()],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(created.rel, "tasks/x-2.md", "the dangling name is skipped");
    let tasks = list(d);
    assert_eq!(warning_kinds(by_url(&tasks, "tasks/b.md")), ["missing"]);
    let x = by_url(&tasks, "tasks/x-2.md");
    assert_eq!(x.blocked_by, ["vault:tasks/b.md"], "canonicalized");
    assert!(x.blocker_warnings.is_empty(), "no cycle formed");

    // A new blocker that names no task refuses the create and writes nothing.
    let before = task_file_names(d);
    let error = create_from_input(
        d,
        &CreateTaskInput {
            title: "Y".into(),
            blocked_by: vec!["vault:tasks/nope.md".into()],
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.starts_with(TASK_BLOCKER_ERROR), "{error}");
    assert_eq!(task_file_names(d), before);

    // The create-time check itself: an existing edge pointing at the
    // not-yet-created path closes a loop through the new blockers.
    let tasks = list_parsed(d);
    let error = check_blocker_cycle(
        &tasks,
        "vault:tasks/x.md",
        "New X",
        &["vault:tasks/b.md".to_string()],
        &["vault:tasks/b.md".to_string()],
    )
    .unwrap_err();
    assert!(error.starts_with(TASK_CYCLE_ERROR), "{error}");
    assert!(error.contains("New X → B → New X"), "{error}");
    assert_eq!(task_file_names(d), before, "a check writes nothing");

    // And create_idempotent itself refuses it: with the avoid set off,
    // X would land at tasks/x.md, which B already waits on.
    IGNORE_DANGLING_AVOID.with(|c| c.set(true));
    let result = create_from_input(
        d,
        &CreateTaskInput {
            title: "X".into(),
            blocked_by: vec!["tasks/b.md".into()],
            ..Default::default()
        },
    );
    IGNORE_DANGLING_AVOID.with(|c| c.set(false));
    let error = result.unwrap_err();
    assert!(error.starts_with(TASK_CYCLE_ERROR), "{error}");
    assert!(error.contains("X → B → X"), "{error}");
    assert_eq!(
        task_file_names(d),
        before,
        "a refused create writes nothing"
    );
}

#[test]
fn undo_restore_that_rebinds_into_a_cycle_is_flagged_not_refused() {
    let dir = vault();
    let d = dir.path();
    blocked_task(d, "tasks/c.md", "C", "Not started", "");
    blocked_task(d, "tasks/b.md", "B", "Not started", "vault:tasks/c.md");
    blocked_task(d, "tasks/a.md", "A", "Not started", "vault:tasks/b.md");
    let b_bytes = read(d, "tasks/b.md");
    delete(d, "tasks/b.md").unwrap();
    // With B gone, C may wait on A (A → B is dangling, so no loop yet).
    set_blockers(d, "tasks/c.md", &["vault:tasks/a.md"]).unwrap();
    restore(d, "tasks/b.md", &b_bytes).unwrap();
    assert_eq!(read(d, "tasks/b.md"), b_bytes, "Undo returns the bytes");
    let tasks = list(d);
    for rel in ["tasks/a.md", "tasks/b.md", "tasks/c.md"] {
        assert!(
            warning_kinds(by_url(&tasks, rel)).contains(&"cycle"),
            "{rel}: {:?}",
            by_url(&tasks, rel).blocker_warnings
        );
    }
}

#[test]
fn blocker_validation_checks_only_new_entries_and_canonicalizes() {
    let dir = vault();
    let d = dir.path();
    blocked_task(d, "tasks/a.md", "A", "Not started", "");
    blocked_task(d, "tasks/e.md", "E", "Not started", "");
    blocked_task(
        d,
        "tasks/keeper.md",
        "Keeper",
        "Not started",
        "vault:tasks/gone.md, vault:tasks/a.md",
    );

    let before = read(d, "tasks/e.md");
    for bad in [
        "vault:tasks/nope.md",
        "vault:tasks/a,b.md",
        "vault:tasks/../a.md",
        "vault:projects/p.md",
    ] {
        let error = set_blockers(d, "tasks/e.md", &[bad]).unwrap_err();
        assert!(error.starts_with(TASK_BLOCKER_ERROR), "{bad}: {error}");
    }
    assert_eq!(read(d, "tasks/e.md"), before);

    // A stored broken entry is kept while a valid one is removed.
    set_blockers(d, "tasks/keeper.md", &["vault:tasks/gone.md"]).unwrap();
    assert!(read(d, "tasks/keeper.md").contains("\nblocked_by: \"vault:tasks/gone.md\"\n"));

    // New entries are stored as the task's canonical URL.
    set_blockers(d, "tasks/e.md", &["vault:tasks/A.md"]).unwrap();
    assert_eq!(
        get(d, "tasks/e.md").unwrap().blocked_by,
        ["vault:tasks/a.md"]
    );
    set_blockers(d, "tasks/e.md", &["tasks/a.md", "vault:tasks/a.md"]).unwrap();
    assert_eq!(
        get(d, "tasks/e.md").unwrap().blocked_by,
        ["vault:tasks/a.md"],
        "deduplicated"
    );
    assert!(read(d, "tasks/e.md").contains("\nblocked_by: \"vault:tasks/a.md\"\n"));
}

#[test]
fn externally_authored_cycles_are_flagged_on_read() {
    let dir = vault();
    let d = dir.path();
    blocked_task(d, "tasks/a.md", "A", "Not started", "vault:tasks/b.md");
    blocked_task(d, "tasks/b.md", "B", "Not started", "vault:tasks/a.md");
    blocked_task(
        d,
        "tasks/self.md",
        "Self",
        "Not started",
        "vault:tasks/self.md",
    );
    let tasks = list(d);
    assert_eq!(warning_kinds(by_url(&tasks, "tasks/a.md")), ["cycle"]);
    assert_eq!(warning_kinds(by_url(&tasks, "tasks/b.md")), ["cycle"]);
    assert_eq!(warning_kinds(by_url(&tasks, "tasks/self.md")), ["cycle"]);
    assert!(!by_url(&tasks, "tasks/a.md").ready);

    // An unrelated edit of a task already on a stored loop still saves:
    // only newly added blockers are searched for a loop.
    blocked_task(d, "tasks/c.md", "C", "Not started", "");
    set_blockers(d, "tasks/a.md", &["vault:tasks/b.md", "vault:tasks/c.md"]).unwrap();
}

// S09: atomic task claims.

fn owner_of(dir: &Path, rel: &str) -> Option<String> {
    claim_of(&read(dir, rel)).map(|(owner, _)| owner)
}

#[test]
fn racing_claims_from_one_read_leave_one_winner_and_one_conflict() {
    let dir = vault();
    let d = dir.path();
    blocked_task(d, "tasks/a.md", "A", "Not started", "");
    // Both claimants read the same baseline before either writes.
    let first = content_hash(&read(d, "tasks/a.md"));
    let second = content_hash(&read(d, "tasks/a.md"));
    let (first, second) = (Some(first.as_str()), Some(second.as_str()));
    let won = claim_task(d, "tasks/a.md", "chat:one", Local::now(), first).unwrap();
    let lost = claim_task(d, "tasks/a.md", "chat:two", Local::now(), second).unwrap_err();
    assert!(lost.starts_with(TASK_CONFLICT_ERROR), "{lost}");
    assert!(!won.already);
    assert_eq!(won.owner.as_deref(), Some("chat:one"));
    assert_eq!(won.content_hash, content_hash(&read(d, "tasks/a.md")));
    assert_eq!(owner_of(d, "tasks/a.md").as_deref(), Some("chat:one"));
    assert_eq!(get(d, "tasks/a.md").unwrap().status, CLAIMED_STATUS);
    assert!(read(d, "tasks/a.md").contains("\nclaimed_at: "));
}

#[test]
fn a_claimed_task_refuses_a_second_claim_naming_its_owner() {
    let dir = vault();
    let d = dir.path();
    blocked_task(d, "tasks/a.md", "A", "Not started", "");
    claim_task(d, "tasks/a.md", "chat:one", Local::now(), None).unwrap();
    let before = read(d, "tasks/a.md");
    let refused = claim_task(d, "tasks/a.md", "workflow:x", Local::now(), None).unwrap_err();
    assert!(refused.starts_with(TASK_CLAIMED_ERROR), "{refused}");
    assert!(refused.contains("chat:one"), "{refused}");
    assert_eq!(read(d, "tasks/a.md"), before);
    // The owner's own repeat is reported, not refused, and changes nothing.
    let again = claim_task(d, "tasks/a.md", "chat:one", Local::now(), None).unwrap();
    assert!(again.already);
    assert_eq!(read(d, "tasks/a.md"), before);
    // Only the owner releases.
    let stranger = release_task(d, "tasks/a.md", "chat:two", None).unwrap_err();
    assert!(stranger.contains("chat:one"), "{stranger}");
    assert_eq!(read(d, "tasks/a.md"), before);
    // A finished task isn't open to a claim.
    blocked_task(d, "tasks/done.md", "Done", "Done", "");
    let done = claim_task(d, "tasks/done.md", "chat:one", Local::now(), None).unwrap_err();
    assert!(done.starts_with(TASK_CLAIMED_ERROR), "{done}");
    assert_eq!(owner_of(d, "tasks/done.md"), None);
}

#[test]
fn release_makes_a_task_claimable_again() {
    let dir = vault();
    let d = dir.path();
    blocked_task(d, "tasks/a.md", "A", "Not started", "");
    claim_task(d, "tasks/a.md", "chat:one", Local::now(), None).unwrap();
    assert!(next_claimable(d, list(d)).is_none());
    let released = release_task(d, "tasks/a.md", "chat:one", None).unwrap();
    assert_eq!(released.status, RELEASED_STATUS);
    assert_eq!(released.owner, None);
    let text = read(d, "tasks/a.md");
    assert_eq!(released.content_hash, content_hash(&text));
    assert!(!text.contains("claimed_by"), "{text}");
    assert!(!text.contains("claimed_at"), "{text}");
    let next = next_claimable(d, list(d)).map(|t| t.url);
    assert_eq!(next, Some(url("tasks/a.md")));
    // Releasing twice is refused.
    let twice = release_task(d, "tasks/a.md", "chat:one", None).unwrap_err();
    assert!(twice.starts_with(TASK_CLAIMED_ERROR), "{twice}");
}

#[test]
fn next_claimable_skips_a_task_whose_blocker_is_open() {
    let dir = vault();
    let d = dir.path();
    blocked_task(d, "tasks/blocker.md", "Blocker", "In progress", "");
    blocked_task(
        d,
        "tasks/waits.md",
        "Waits",
        "Not started",
        "vault:tasks/blocker.md",
    );
    assert!(next_claimable(d, list(d)).is_none());
    blocked_task(d, "tasks/blocker.md", "Blocker", "Done", "");
    let next = next_claimable(d, list(d)).map(|t| t.url);
    assert_eq!(next, Some(url("tasks/waits.md")));
}

#[test]
fn next_claimable_prefers_priority_then_the_oldest() {
    let dir = vault();
    let d = dir.path();
    let task = |title: &str, extra: &str| {
        format!("---\ntitle: {title}\nstatus: Not started\n{extra}---\n")
    };
    write(d, "tasks/new.md", &task("New", "created: 2026-10-02\n"));
    write(d, "tasks/old.md", &task("Old", "created: 2026-09-01\n"));
    write(d, "tasks/undated.md", &task("Undated", ""));
    let next = next_claimable(d, list(d)).map(|t| t.url);
    assert_eq!(next, Some(url("tasks/old.md")));
    write(d, "tasks/urgent.md", &task("Urgent", "priority: P1\n"));
    let next = next_claimable(d, list(d)).map(|t| t.url);
    assert_eq!(next, Some(url("tasks/urgent.md")));
}

#[test]
fn finishing_a_claimed_recurring_task_rolls_it_forward_unclaimed() {
    let dir = vault();
    let d = dir.path();
    let rel = "tasks/water.md";
    write(
        d,
        rel,
        "---\ntitle: Water\nstatus: Not started\ndue: 2026-09-20\nrecurrence: every week\n---\n",
    );
    claim_task(d, rel, "chat:one", Local::now(), None).unwrap();
    let outcome = set_status_with_recurrence(d, rel, "Done", today(), None).unwrap();
    assert!(outcome.rolled_to.is_some());
    assert_eq!(outcome.status, "Not started");
    let text = read(d, rel);
    assert!(!text.contains("claimed_by"), "{text}");
    assert!(!text.contains("claimed_at"), "{text}");
    assert_eq!(next_claimable(d, list(d)).map(|t| t.url), Some(url(rel)));
}

#[test]
fn a_claimed_task_set_back_to_not_started_is_claimable_again() {
    let dir = vault();
    let d = dir.path();
    blocked_task(d, "tasks/a.md", "A", "Not started", "");
    claim_task(d, "tasks/a.md", "chat:one", Local::now(), None).unwrap();
    set_status_with_recurrence(d, "tasks/a.md", "Not started", today(), None).unwrap();
    assert_eq!(owner_of(d, "tasks/a.md"), None);
    let next = next_claimable(d, list(d)).map(|t| t.url);
    assert_eq!(next, Some(url("tasks/a.md")));
    // Another session can claim it now.
    let won = claim_task(d, "tasks/a.md", "chat:two", Local::now(), None).unwrap();
    assert_eq!(won.owner.as_deref(), Some("chat:two"));
}

/// S24: a status write that ends the claim leaves no key in the file.
#[test]
fn a_claim_then_set_status_not_started_leaves_no_claim_keys() {
    let dir = vault();
    let d = dir.path();
    blocked_task(d, "tasks/a.md", "A", "Not started", "");
    claim_task(d, "tasks/a.md", "chat:one", Local::now(), None).unwrap();
    set_status(d, "tasks/a.md", "Not started").unwrap();
    let text = read(d, "tasks/a.md");
    assert!(!text.contains("claimed_by"), "{text}");
    assert!(!text.contains("claimed_at"), "{text}");
}

/// S24: a write that keeps the task in progress keeps its claim: no status
/// change, or a move between in-progress spellings.
#[test]
fn edits_that_stay_in_progress_keep_the_claim() {
    let dir = vault();
    let d = dir.path();
    blocked_task(d, "tasks/a.md", "A", "Not started", "");
    claim_task(d, "tasks/a.md", "chat:one", Local::now(), None).unwrap();
    let input = UpdateTaskInput {
        url: url("tasks/a.md"),
        priority: Some("P2".into()),
        ..Default::default()
    };
    apply_update(d, "tasks/a.md", &input).unwrap();
    assert_eq!(owner_of(d, "tasks/a.md").as_deref(), Some("chat:one"));
    set_status(d, "tasks/a.md", "Doing").unwrap();
    assert_eq!(owner_of(d, "tasks/a.md").as_deref(), Some("chat:one"));
    assert!(read(d, "tasks/a.md").contains("\nclaimed_at: "));
}

/// S24: a stale key (left by an outside editor or an older build) must not
/// come back to life when an editor moves the task to In progress again.
#[test]
fn an_editor_moving_a_stale_key_to_in_progress_does_not_revive_the_claim() {
    let dir = vault();
    let d = dir.path();
    let rel = "tasks/stale.md";
    write(
        d,
        rel,
        "---\ntitle: Stale\nstatus: Not started\nclaimed_by: \"chat:gone\"\nclaimed_at: \"2026-10-01T09:00:00+00:00\"\n---\n",
    );
    let edit = |status: &str| UpdateTaskInput {
        url: url(rel),
        status: Some(status.into()),
        ..Default::default()
    };
    apply_update(d, rel, &edit("In progress")).unwrap();
    let text = read(d, rel);
    assert!(!text.contains("claimed_by"), "{text}");
    assert!(!text.contains("claimed_at"), "{text}");
    // Someone is working it, so it isn't open, but not in the dead owner's
    // name, and the dead owner has nothing to release.
    let busy = claim_task(d, rel, "chat:two", Local::now(), None).unwrap_err();
    assert!(busy.starts_with(TASK_CLAIMED_ERROR), "{busy}");
    assert!(!busy.contains("chat:gone"), "{busy}");
    let gone = release_task(d, rel, "chat:gone", None).unwrap_err();
    assert!(gone.contains("isn't claimed"), "{gone}");
    // Back to Not started, another session's claim succeeds.
    apply_update(d, rel, &edit("Not started")).unwrap();
    let won = claim_task(d, rel, "chat:two", Local::now(), None).unwrap();
    assert!(!won.already);
    assert_eq!(won.owner.as_deref(), Some("chat:two"));
    assert_eq!(owner_of(d, rel).as_deref(), Some("chat:two"));
    assert_eq!(get(d, rel).unwrap().status, CLAIMED_STATUS);
}

/// S24: release reads only a live claim, so a stale key on a task that isn't
/// in progress has nothing to release and the file keeps its bytes.
#[test]
fn releasing_a_stale_claim_key_says_it_isnt_claimed() {
    let dir = vault();
    let d = dir.path();
    for (rel, status) in [("tasks/open.md", "Not started"), ("tasks/done.md", "Done")] {
        write(
            d,
            rel,
            &format!("---\ntitle: Stale\nstatus: {status}\nclaimed_by: \"chat:gone\"\n---\n"),
        );
        let before = read(d, rel);
        let refused = release_task(d, rel, "chat:gone", None).unwrap_err();
        assert!(refused.starts_with(TASK_CLAIMED_ERROR), "{refused}");
        assert!(refused.contains("isn't claimed"), "{refused}");
        assert_eq!(read(d, rel), before);
    }
}

#[test]
fn a_leftover_claim_on_an_open_status_is_stale() {
    let dir = vault();
    let d = dir.path();
    let rel = "tasks/stale.md";
    write(
        d,
        rel,
        "---\ntitle: Stale\nstatus: Not started\nclaimed_by: \"chat:gone\"\n---\n",
    );
    assert_eq!(next_claimable(d, list(d)).map(|t| t.url), Some(url(rel)));
    let won = claim_task(d, rel, "chat:one", Local::now(), None).unwrap();
    assert!(!won.already);
    assert_eq!(owner_of(d, rel).as_deref(), Some("chat:one"));
}

// OC5: attempt-bound completion and the three-failure circuit breaker.

fn oc5_today() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2026, 11, 17).unwrap()
}

#[test]
fn a_completion_with_an_old_attempt_id_cannot_close_a_newer_claim() {
    let dir = vault();
    let d = dir.path();
    blocked_task(d, "tasks/a.md", "A", "Not started", "");
    let first = claim_task(d, "tasks/a.md", "chat:one", Local::now(), None).unwrap();
    assert_eq!(first.attempt, Some(1));
    // The first attempt is replaced: released, then claimed again.
    release_task(d, "tasks/a.md", "chat:one", None).unwrap();
    let second = claim_task(d, "tasks/a.md", "chat:one", Local::now(), None).unwrap();
    assert_eq!(second.attempt, Some(2));
    // A late retry from the first attempt is refused and writes nothing.
    let before = read(d, "tasks/a.md");
    let stale =
        complete_claim(d, "tasks/a.md", "chat:one", 1, "Done", oc5_today(), None).unwrap_err();
    assert!(stale.starts_with(TASK_STALE_ATTEMPT_ERROR), "{stale}");
    assert!(stale.contains("attempt 2"), "{stale}");
    assert_eq!(read(d, "tasks/a.md"), before);
    assert_eq!(get(d, "tasks/a.md").unwrap().status, CLAIMED_STATUS);
    // Another run can't close it with the right number either.
    let foreign =
        complete_claim(d, "tasks/a.md", "chat:two", 2, "Done", oc5_today(), None).unwrap_err();
    assert!(foreign.starts_with(TASK_STALE_ATTEMPT_ERROR), "{foreign}");
    // The current attempt closes it, and the claim keys go.
    let done = complete_claim(d, "tasks/a.md", "chat:one", 2, "Done", oc5_today(), None).unwrap();
    assert_eq!(done.status, "Done");
    let text = read(d, "tasks/a.md");
    assert!(!text.contains("claimed_by"), "{text}");
    // Finished: the old attempt can't reopen or re-close it.
    let after =
        complete_claim(d, "tasks/a.md", "chat:one", 2, "Done", oc5_today(), None).unwrap_err();
    assert!(after.starts_with(TASK_STALE_ATTEMPT_ERROR), "{after}");
}

#[test]
fn the_third_failed_attempt_breaks_the_circuit_and_says_why() {
    let dir = vault();
    let d = dir.path();
    blocked_task(d, "tasks/a.md", "A", "Not started", "");
    for n in 1..=2u32 {
        let claim = claim_task(d, "tasks/a.md", "chat:one", Local::now(), None).unwrap();
        let failed = fail_claim(
            d,
            "tasks/a.md",
            "chat:one",
            claim.attempt.unwrap(),
            &format!("gate red {n}"),
            None,
        )
        .unwrap();
        assert_eq!(failed.failures, n);
        assert_eq!(failed.circuit_broken, None);
        assert_eq!(failed.status, RELEASED_STATUS);
        assert!(
            next_claimable(d, list(d)).is_some(),
            "retry {n} stays claimable"
        );
    }
    let claim = claim_task(d, "tasks/a.md", "chat:two", Local::now(), None).unwrap();
    assert_eq!(claim.attempt, Some(3));
    // A stale failure report from attempt 2 counts nothing.
    let stale = fail_claim(d, "tasks/a.md", "chat:one", 2, "late", None).unwrap_err();
    assert!(stale.starts_with(TASK_STALE_ATTEMPT_ERROR), "{stale}");
    let broken = fail_claim(
        d,
        "tasks/a.md",
        "chat:two",
        3,
        "cargo test failed: lease_lock",
        None,
    )
    .unwrap();
    assert_eq!(broken.failures, CIRCUIT_BREAK_AFTER);
    let reason = broken
        .circuit_broken
        .expect("third failure breaks the circuit");
    assert!(reason.contains("3 failed attempts"), "{reason}");
    assert!(reason.contains("cargo test failed: lease_lock"), "{reason}");
    assert!(is_hold_status(&broken.status), "{}", broken.status);
    // It stops retrying: not offered, and a claim is refused with the reason.
    assert!(next_claimable(d, list(d)).is_none());
    let refused = claim_task(d, "tasks/a.md", "chat:three", Local::now(), None).unwrap_err();
    assert!(refused.starts_with(TASK_CIRCUIT_BROKEN_ERROR), "{refused}");
    assert!(
        refused.contains("cargo test failed: lease_lock"),
        "{refused}"
    );
    // A person reopening it resets the breaker for a fresh run of attempts.
    set_status(d, "tasks/a.md", "Not started").unwrap();
    let again = claim_task(d, "tasks/a.md", "chat:three", Local::now(), None).unwrap();
    assert_eq!(again.attempt, Some(4));
    let text = read(d, "tasks/a.md");
    assert!(!text.contains("circuit_broken"), "{text}");
    assert!(!text.contains("failed_attempts"), "{text}");
}

/// OC5 review: break a task's circuit with three failed attempts.
fn break_circuit(d: &Path, rel: &str) {
    for n in 1..=CIRCUIT_BREAK_AFTER {
        let claim = claim_task(d, rel, "chat:one", Local::now(), None).unwrap();
        fail_claim(
            d,
            rel,
            "chat:one",
            claim.attempt.unwrap(),
            &format!("red {n}"),
            None,
        )
        .unwrap();
    }
}

const TRACKED_OC5: &str = "---\ntitle: \"Tracked\"\nstatus: \"Todo\"\nstatus_type: \"unstarted\"\ntracker: linear\ntracker_id: \"i1\"\ntracker_key: \"ENG-1\"\ntracker_state: \"active\"\n---\nbody\n";

fn linked_tasks(d: &Path) -> Vec<Task> {
    let mut tasks = list(d);
    for t in &mut tasks {
        if let Some(meta) = &mut t.tracker {
            meta.lock.kind = crate::tracker::ipc::TrackerLockKind::Linked;
        }
    }
    tasks
}

#[test]
fn a_tracker_task_circuit_holds_until_its_provider_status_changes() {
    let dir = vault();
    let d = dir.path();
    write(d, "tasks/t.md", TRACKED_OC5);
    assert!(next_claimable(d, linked_tasks(d)).is_some());
    break_circuit(d, "tasks/t.md");
    let text = read(d, "tasks/t.md");
    assert!(text.contains("circuit_broken"), "{text}");
    assert!(
        text.contains("\nstatus: \"Todo\""),
        "the provider's status is untouched: {text}"
    );
    // Broken: not offered and a claim says why.
    assert!(next_claimable(d, linked_tasks(d)).is_none());
    let refused = claim_task(d, "tasks/t.md", "chat:two", Local::now(), None).unwrap_err();
    assert!(refused.starts_with(TASK_CIRCUIT_BROKEN_ERROR), "{refused}");
    assert!(refused.contains("red 3"), "{refused}");
    // The person moves the issue in the tracker: the breaker resets.
    write(
        d,
        "tasks/t.md",
        &text.replace("\nstatus: \"Todo\"", "\nstatus: \"Backlog\""),
    );
    assert!(next_claimable(d, linked_tasks(d)).is_some());
    claim_task(d, "tasks/t.md", "chat:two", Local::now(), None).unwrap();
    let text = read(d, "tasks/t.md");
    assert!(!text.contains("circuit_broken"), "{text}");
    assert!(!text.contains("failed_attempts"), "{text}");
}

#[test]
fn a_successful_completion_clears_earlier_failures() {
    let dir = vault();
    let d = dir.path();
    blocked_task(d, "tasks/a.md", "A", "Not started", "");
    let claim = claim_task(d, "tasks/a.md", "chat:one", Local::now(), None).unwrap();
    fail_claim(
        d,
        "tasks/a.md",
        "chat:one",
        claim.attempt.unwrap(),
        "flaky",
        None,
    )
    .unwrap();
    let claim = claim_task(d, "tasks/a.md", "chat:one", Local::now(), None).unwrap();
    complete_claim(
        d,
        "tasks/a.md",
        "chat:one",
        claim.attempt.unwrap(),
        "Done",
        oc5_today(),
        None,
    )
    .unwrap();
    let text = read(d, "tasks/a.md");
    assert!(!text.contains("failed_attempts"), "{text}");
    assert!(!text.contains("last_failure"), "{text}");
}

#[test]
fn an_unreadable_attempt_count_refuses_a_claim_instead_of_reusing_an_id() {
    let dir = vault();
    let d = dir.path();
    write(
        d,
        "tasks/a.md",
        "---\ntitle: A\nstatus: Not started\nclaim_attempts: two\n---\n",
    );
    let before = read(d, "tasks/a.md");
    let refused = claim_task(d, "tasks/a.md", "chat:one", Local::now(), None).unwrap_err();
    assert!(refused.starts_with(TASK_CLAIMED_ERROR), "{refused}");
    assert!(refused.contains("claim_attempts"), "{refused}");
    assert_eq!(read(d, "tasks/a.md"), before);
}

#[test]
fn a_reopened_task_drops_its_breaker_and_never_cites_it_again() {
    let dir = vault();
    let d = dir.path();
    blocked_task(d, "tasks/a.md", "A", "Not started", "");
    break_circuit(d, "tasks/a.md");
    set_status(d, "tasks/a.md", "Not started").unwrap();
    let text = read(d, "tasks/a.md");
    assert!(!text.contains("circuit_broken"), "{text}");
    assert!(!text.contains("failed_attempts"), "{text}");
    // Parked again for another reason: refused as not open, without the old reason.
    set_status(d, "tasks/a.md", "On hold").unwrap();
    let refused = claim_task(d, "tasks/a.md", "chat:one", Local::now(), None).unwrap_err();
    assert!(refused.starts_with(TASK_CLAIMED_ERROR), "{refused}");
    assert!(!refused.contains("red 3"), "{refused}");
}
