//! L2: "Delete the memory X?" offers. Fix-in-build detection, which file
//! may be offered, order, the delete's bounds and rollbacks, Keep, Undo,
//! and that the kind stays out of T3b's copies.

use super::*;
use crate::tracker::linear::PERSONAL_TEAM_ID;
use serde_json::json;

const BUILT: &str = "2026-11-15T13:18:17.452Z";

fn lesson(n: u32, memory: &str, kind: &str, completed: Option<&str>) -> TrackerIssue {
    serde_json::from_value(json!({
        "id": format!("per-{n}"), "key": format!("PER-{n}"), "url": "https://tracker.example/x",
        "title": format!("Lesson {n}"),
        "description": format!("Text.\n\n>>> Agent notes\n\nWorkaround memory: `{memory}` in `~/.claude/projects/-Users-old-path-main/memory/`. When the fix is installed, delete that file and its line in `MEMORY.md`.\n\n>>>"),
        "state": {"id": "s", "name": "x", "kind": kind},
        "labels": ["hollow-lesson"], "teamId": PERSONAL_TEAM_ID,
        "updatedAt": "2026-11-15T00:00:00Z", "createdAt": "2026-11-12T00:00:00Z",
        "completedAt": completed
    }))
    .unwrap()
}

fn done(n: u32, memory: &str) -> TrackerIssue {
    lesson(n, memory, "completed", Some("2026-11-15T04:08:35.677Z"))
}

struct Fx {
    _vault: tempfile::TempDir,
    _claude: tempfile::TempDir,
    root: PathBuf,
    claude: PathBuf,
    folder: PathBuf,
}

/// The body a fixture memory `mN.md` gets: it names its lesson, PER-N.
fn body(name: &str) -> String {
    let n = name.trim_start_matches('m').trim_end_matches(".md");
    format!("workaround, see PER-{n}\n")
}

/// A vault with one workspace and its Claude memory folder holding `files`,
/// each listed in MEMORY.md, plus an unrelated `other.md`.
fn fixture(files: &[&str]) -> Fx {
    let vault = tempfile::tempdir().unwrap();
    let root = vault.path().canonicalize().unwrap();
    crate::workspaces::create(&root, "Main", "Everything").unwrap();
    let claude_tmp = tempfile::tempdir().unwrap();
    let claude = claude_tmp.path().canonicalize().unwrap();
    let folders = memory_folders(&root, &claude);
    assert_eq!(folders.len(), 1);
    let folder = folders[0].1.clone();
    std::fs::create_dir_all(&folder).unwrap();
    let mut index = String::from("# Memory\n");
    for f in files {
        std::fs::write(folder.join(f), body(f)).unwrap();
        index.push_str(&format!("- [{f}]({f}) a workaround\n"));
    }
    std::fs::write(folder.join("MEMORY.md"), &index).unwrap();
    std::fs::write(folder.join("other.md"), "unrelated\n").unwrap();
    Fx {
        _vault: vault,
        _claude: claude_tmp,
        root,
        claude,
        folder,
    }
}

fn offers(fx: &Fx, issues: &[TrackerIssue]) -> Vec<Offer> {
    offers_in(
        &fx.root,
        &fx.claude,
        BUILT,
        issues,
        &load(&fx.root).unwrap(),
    )
}

fn ids(fx: &Fx, issues: &[TrackerIssue]) -> Vec<String> {
    offers(fx, issues)
        .into_iter()
        .map(|o| o.source_id)
        .collect()
}

fn trash_count(fx: &Fx) -> usize {
    std::fs::read_dir(fx.root.join(".trash")).map_or(0, |d| d.count())
}

#[test]
fn a_fix_looks_installed_only_when_done_fifteen_minutes_before_the_build() {
    assert!(fix_in_build(&done(4, "a.md"), BUILT));
    // Exactly the margin before the build counts; a second less does not,
    // nor Done at the build time itself.
    let margin = lesson(4, "a.md", "completed", Some("2026-11-15T13:03:17.452Z"));
    assert!(fix_in_build(&margin, BUILT));
    let short = lesson(4, "a.md", "completed", Some("2026-11-15T13:03:18.452Z"));
    assert!(!fix_in_build(&short, BUILT));
    assert!(!fix_in_build(
        &lesson(4, "a.md", "completed", Some(BUILT)),
        BUILT
    ));
    // Not completed, or completed with no time, or cancelled: not in.
    assert!(!fix_in_build(&lesson(2, "a.md", "backlog", None), BUILT));
    assert!(!fix_in_build(&lesson(4, "a.md", "completed", None), BUILT));
    assert!(!fix_in_build(
        &lesson(4, "a.md", "canceled", Some("2026-11-14T00:00:00Z")),
        BUILT
    ));
    // An unknown build time offers nothing.
    assert!(!fix_in_build(&done(4, "a.md"), ""));
}

#[test]
fn the_memory_name_comes_from_the_agent_notes_and_must_be_plain() {
    assert_eq!(
        memory_named(&done(4, "checkin-card-answers-keep-accept-edits.md").description).as_deref(),
        Some("checkin-card-answers-keep-accept-edits.md")
    );
    for bad in ["../x.md", "MEMORY.md", ".hidden.md", "a/b.md", "notes.txt"] {
        assert_eq!(
            memory_named(&format!("Workaround memory: `{bad}` in x")),
            None,
            "{bad}"
        );
    }
    assert_eq!(memory_named("no notes here"), None);
}

#[test]
fn issue_text_alone_never_picks_a_file() {
    let fx = fixture(&[]);
    // A mislabelled lesson naming an unrelated memory (not a known pair,
    // no key in the file or its index line) is not offered.
    let issues = [done(5, "other.md")];
    assert!(offers(&fx, &issues).is_empty());
    assert!(answer_in(&fx.root, &fx.claude, BUILT, &issues, "PER-5:other.md", true).is_err());
    assert!(fx.folder.join("other.md").exists());
    // A known memory claimed by the wrong issue is not offered either.
    std::fs::write(fx.folder.join("handover-self-wake.md"), "self wake\n").unwrap();
    assert!(offers(&fx, &[done(5, "handover-self-wake.md")]).is_empty());
    // Its own issue (the L1 pair) is, with no key in the file.
    assert_eq!(
        ids(&fx, &[done(8, "handover-self-wake.md")]),
        ["PER-8:handover-self-wake.md"]
    );
    // A file outside the table is vouched for by its index line naming the
    // key, and only the whole key: PER-1 is not PER-13.
    let index = fx.folder.join("MEMORY.md");
    std::fs::write(&index, "- [x](new-one.md) PER-13 workaround\n").unwrap();
    std::fs::write(fx.folder.join("new-one.md"), "x\n").unwrap();
    assert!(offers(&fx, &[done(1, "new-one.md")]).is_empty());
    assert_eq!(ids(&fx, &[done(13, "new-one.md")]), ["PER-13:new-one.md"]);
    assert!(names_key("see PER-4.", "PER-4"));
    assert!(!names_key("see PER-41", "PER-4"));
    assert!(!names_key("XHOL-4", "PER-4"));
}

#[test]
fn the_four_first_lead_then_issue_order_capped_and_only_existing_files() {
    let fx = fixture(&[
        "m1.md", "m4.md", "m5.md", "m9.md", "m12.md", "m13.md", "m14.md",
    ]);
    let mut issues = vec![
        done(14, "m14.md"),
        done(5, "m5.md"),
        done(4, "m4.md"),
        done(9, "m9.md"),
        done(12, "m12.md"),
        done(13, "m13.md"),
        done(1, "m1.md"),
        // Backlog: not offered, though its file exists.
        lesson(2, "m2.md", "backlog", None),
        // Done after this build: not yet.
        lesson(10, "m10.md", "completed", Some("2026-11-16T00:00:00Z")),
        // Done, but its file is already gone.
        done(11, "gone.md"),
    ];
    std::fs::write(fx.folder.join("m2.md"), body("m2.md")).unwrap();
    std::fs::write(fx.folder.join("m10.md"), body("m10.md")).unwrap();
    // Another team's issue with the label is never a lesson here.
    let mut eng = done(8, "m5.md");
    eng.team_id = "11111111-eng".into();
    eng.key = "ENG-8".into();
    issues.push(eng);
    let got = ids(&fx, &issues);
    assert_eq!(
        got,
        [
            "PER-1:m1.md",
            "PER-13:m13.md",
            "PER-12:m12.md",
            "PER-9:m9.md"
        ]
    );
    // Answering the four brings the rest, by issue number.
    for id in &got {
        answer_in(&fx.root, &fx.claude, BUILT, &issues, id, false).unwrap();
    }
    assert_eq!(
        ids(&fx, &issues),
        ["PER-4:m4.md", "PER-5:m5.md", "PER-14:m14.md"]
    );
    let first = &offers(&fx, &issues)[0];
    assert_eq!(first.title, "Delete the memory m4?");
    assert!(
        first
            .detail
            .contains("The fix looks installed (PER-4 was Done before this build)"),
        "{}",
        first.detail
    );
}

#[test]
fn delete_removes_only_that_file_and_its_line_and_undo_puts_both_back() {
    let fx = fixture(&["m4.md", "m5.md"]);
    let issues = [done(4, "m4.md"), done(5, "m5.md")];
    let index_before = std::fs::read_to_string(fx.folder.join("MEMORY.md")).unwrap();
    let got = answer_in(&fx.root, &fx.claude, BUILT, &issues, "PER-4:m4.md", true).unwrap();
    assert_eq!(got, "m4.md");
    assert!(!fx.folder.join("m4.md").exists());
    assert!(fx.folder.join("m5.md").exists());
    assert!(fx.folder.join("other.md").exists());
    let index = std::fs::read_to_string(fx.folder.join("MEMORY.md")).unwrap();
    assert_eq!(index, "# Memory\n- [m5.md](m5.md) a workaround\n");
    // A copy waits in the vault's .trash and History has a row.
    let trash: Vec<_> = std::fs::read_dir(fx.root.join(".trash"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(trash.len(), 1);
    assert!(
        trash[0].ends_with("-lesson-memory-PER-4-m4.md"),
        "{trash:?}"
    );
    let runs = crate::automated_history::list_runs(&fx.root, 10).unwrap();
    assert!(runs
        .iter()
        .any(|r| r.source == HISTORY_DELETED && r.source_id.as_deref() == Some("PER-4:m4.md")));
    // Never offered again, and a second answer is refused.
    assert_eq!(ids(&fx, &issues), ["PER-5:m5.md"]);
    assert!(answer_in(&fx.root, &fx.claude, BUILT, &issues, "PER-4:m4.md", true).is_err());

    restore_in(&fx.root, &fx.claude, "PER-4:m4.md").unwrap();
    assert_eq!(
        std::fs::read_to_string(fx.folder.join("m4.md")).unwrap(),
        body("m4.md")
    );
    assert_eq!(
        std::fs::read_to_string(fx.folder.join("MEMORY.md")).unwrap(),
        index_before
    );
    // Restored means kept: still not offered, and Undo can't run twice.
    assert_eq!(offers(&fx, &issues).len(), 1);
    assert!(restore_in(&fx.root, &fx.claude, "PER-4:m4.md").is_err());
}

#[test]
fn undo_after_the_index_changed_appends_the_line() {
    let fx = fixture(&["m4.md", "m5.md"]);
    let issues = [done(4, "m4.md")];
    answer_in(&fx.root, &fx.claude, BUILT, &issues, "PER-4:m4.md", true).unwrap();
    let index = fx.folder.join("MEMORY.md");
    std::fs::write(
        &index,
        "# Memory\n- [m5.md](m5.md) a workaround\n- [n](n.md) new\n",
    )
    .unwrap();
    restore_in(&fx.root, &fx.claude, "PER-4:m4.md").unwrap();
    assert_eq!(
        std::fs::read_to_string(&index).unwrap(),
        "# Memory\n- [m5.md](m5.md) a workaround\n- [n](n.md) new\n- [m4.md](m4.md) a workaround\n"
    );
}

#[test]
fn an_index_edited_mid_delete_changes_nothing() {
    let fx = fixture(&["m4.md"]);
    let issues = [done(4, "m4.md")];
    let index = fx.folder.join("MEMORY.md");
    FAULT.with(|f| f.set(Some(Fault::IndexEdited)));
    let err = answer_in(&fx.root, &fx.claude, BUILT, &issues, "PER-4:m4.md", true).unwrap_err();
    assert!(err.contains("MEMORY.md changed"), "{err}");
    assert!(fx.folder.join("m4.md").exists());
    let text = std::fs::read_to_string(&index).unwrap();
    assert!(
        text.contains("(m4.md)") && text.contains("(new.md)"),
        "{text}"
    );
    assert_eq!(trash_count(&fx), 0, "the copy is rolled back");
    // No answer was kept: it is offered again.
    assert_eq!(ids(&fx, &issues), ["PER-4:m4.md"]);

    // Same on Undo: the restore stops and changes nothing.
    answer_in(&fx.root, &fx.claude, BUILT, &issues, "PER-4:m4.md", true).unwrap();
    FAULT.with(|f| f.set(Some(Fault::IndexEdited)));
    let err = restore_in(&fx.root, &fx.claude, "PER-4:m4.md").unwrap_err();
    assert!(err.contains("MEMORY.md changed"), "{err}");
    assert!(!fx.folder.join("m4.md").exists());
    assert!(!std::fs::read_to_string(&index).unwrap().contains("(m4.md)"));
    restore_in(&fx.root, &fx.claude, "PER-4:m4.md").unwrap();
    assert!(fx.folder.join("m4.md").exists());
}

#[test]
fn a_failed_answer_save_deletes_nothing() {
    let fx = fixture(&["m4.md"]);
    let issues = [done(4, "m4.md")];
    let index_before = std::fs::read_to_string(fx.folder.join("MEMORY.md")).unwrap();
    FAULT.with(|f| f.set(Some(Fault::SaveFails)));
    assert!(answer_in(&fx.root, &fx.claude, BUILT, &issues, "PER-4:m4.md", true).is_err());
    assert!(fx.folder.join("m4.md").exists());
    assert_eq!(
        std::fs::read_to_string(fx.folder.join("MEMORY.md")).unwrap(),
        index_before
    );
    assert_eq!(trash_count(&fx), 0);
    assert_eq!(ids(&fx, &issues), ["PER-4:m4.md"]);
    // On Undo, a failed save takes the restored file and line back out.
    answer_in(&fx.root, &fx.claude, BUILT, &issues, "PER-4:m4.md", true).unwrap();
    FAULT.with(|f| f.set(Some(Fault::SaveFails)));
    assert!(restore_in(&fx.root, &fx.claude, "PER-4:m4.md").is_err());
    assert!(!fx.folder.join("m4.md").exists());
    assert!(!std::fs::read_to_string(fx.folder.join("MEMORY.md"))
        .unwrap()
        .contains("(m4.md)"));
}

#[test]
fn index_lines_match_relative_angle_and_titled_links() {
    for line in [
        "- [a](m4.md) x",
        "- [a](./m4.md) x",
        "- [a](<m4.md>) x",
        "- [a](m4.md \"Title\") x",
        "- [a](./m4.md 'Title') x",
    ] {
        assert!(links_to(line, "m4.md"), "{line}");
    }
    for line in ["- [a](m44.md)", "- [a](x/m4.md)", "m4.md alone"] {
        assert!(!links_to(line, "m4.md"), "{line}");
    }
}

#[test]
fn keep_records_the_choice_and_deletes_nothing() {
    let fx = fixture(&["m4.md"]);
    let issues = [done(4, "m4.md")];
    answer_in(&fx.root, &fx.claude, BUILT, &issues, "PER-4:m4.md", false).unwrap();
    assert!(fx.folder.join("m4.md").exists());
    assert!(std::fs::read_to_string(fx.folder.join("MEMORY.md"))
        .unwrap()
        .contains("(m4.md)"));
    assert!(offers(&fx, &issues).is_empty());
    assert!(restore_in(&fx.root, &fx.claude, "PER-4:m4.md").is_err());
}

#[test]
fn nothing_outside_an_offer_or_a_plain_file_is_ever_deleted() {
    let fx = fixture(&["m4.md"]);
    let issues = [done(4, "m4.md")];
    // An id that isn't offered names nothing, whatever it says.
    for id in [
        "PER-4:other.md",
        "PER-4:../../x.md",
        "PER-2:m4.md",
        "PER-4:MEMORY.md",
    ] {
        assert!(
            answer_in(&fx.root, &fx.claude, BUILT, &issues, id, true).is_err(),
            "{id}"
        );
    }
    assert!(fx.folder.join("other.md").exists());
    // A memory that is a link is never offered.
    std::fs::remove_file(fx.folder.join("m4.md")).unwrap();
    let outside = fx.claude.join("outside.md");
    std::fs::write(&outside, "keep me PER-4").unwrap();
    std::os::unix::fs::symlink(&outside, fx.folder.join("m4.md")).unwrap();
    assert!(offers(&fx, &issues).is_empty());
    assert!(outside.exists());
}

#[test]
fn two_index_lines_for_one_memory_refuse_the_delete() {
    let fx = fixture(&["m4.md"]);
    let index = fx.folder.join("MEMORY.md");
    let mut text = std::fs::read_to_string(&index).unwrap();
    text.push_str("- [again](./m4.md) twice\n");
    std::fs::write(&index, &text).unwrap();
    let issues = [done(4, "m4.md")];
    let err = answer_in(&fx.root, &fx.claude, BUILT, &issues, "PER-4:m4.md", true).unwrap_err();
    assert!(err.contains("2 lines"), "{err}");
    assert!(fx.folder.join("m4.md").exists());
    assert_eq!(std::fs::read_to_string(&index).unwrap(), text);
    assert_eq!(trash_count(&fx), 0);
}

#[test]
fn lesson_issues_are_read_from_the_mirror_ledger() {
    let fx = fixture(&["m4.md"]);
    let mut state = crate::tracker_sync::Ledger::new("linear", PERSONAL_TEAM_ID);
    let issue = done(4, "m4.md");
    state.records.insert(
        issue.id.clone(),
        serde_json::from_value(json!({
            "id": issue.id, "key": issue.key, "kind": "task", "path": "tasks/tracker-hol-4.md",
            "workspaceId": null, "base": {}, "issue": issue, "project": null
        }))
        .unwrap(),
    );
    crate::tracker_sync::save(&fx.root, &state).unwrap();
    let issues = crate::tracker_sync::mirrored_issues(&fx.root).unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(ids(&fx, &issues), ["PER-4:m4.md"]);
}

#[test]
fn the_kind_stays_in_hollow_never_a_linear_copy() {
    assert!(!crate::tracker::waiting_work::MIRRORED_KINDS.contains(&KIND));
}
