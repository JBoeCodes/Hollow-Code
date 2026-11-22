//! Lane M: routed turns survive a restart, deliver once, and are pruned.
use super::*;

const NOW: i64 = 1_800_000_000_000;
const MINI: &str = "m-8d21c4be90f17a3e";

fn entry(key: &str, at_ms: i64) -> Entry {
    Entry {
        id: id_of(MINI, key, at_ms),
        machine_id: MINI.into(),
        machine: "Mac mini".into(),
        key: key.into(),
        what: "Mac mini's COO".into(),
        vault: "/tmp/vault".into(),
        at_ms,
        seen_busy: false,
        tries: 0,
        state: State::Pending,
        done_ms: None,
    }
}

fn file() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Peers").join(FILE);
    (dir, path)
}

#[test]
fn a_turn_written_before_a_restart_is_delivered_once_after_it() {
    let (_dir, path) = file();
    let mut book = Book::default();
    assert!(book.add(entry("s-a", NOW), NOW));
    book.save(&path).unwrap();
    drop(book); // the app quits
    assert!(has_pending(&path));

    let mut book = Book::load(&path);
    let id = id_of(MINI, "s-a", NOW);
    assert_eq!(book.pending().len(), 1);
    assert_eq!(book.pending()[0].id, id);
    // The first list after boot claims its one notification.
    assert!(book.claim(&id, NOW + 1000));
    book.save(&path).unwrap();
    assert!(!has_pending(&path));

    // Another restart, a replayed list, a second stream: nothing again.
    let mut book = Book::load(&path);
    assert_eq!(book.state_of(&id), Some(State::Delivered));
    assert!(book.pending().is_empty());
    assert!(!book.claim(&id, NOW + 2000), "never twice");
    assert!(!book.expire(&id, NOW + 2000));
}

#[test]
fn the_progress_a_turn_made_is_kept_too() {
    let (_dir, path) = file();
    let mut book = Book::default();
    book.add(entry("s-a", NOW), NOW);
    let id = id_of(MINI, "s-a", NOW);
    assert!(book.note_busy(&id));
    assert!(!book.note_busy(&id), "no change the second time");
    book.save(&path).unwrap();
    assert!(Book::load(&path).pending()[0].seen_busy);
}

#[test]
fn a_chat_with_a_turn_waiting_takes_no_second_and_a_later_route_is_new() {
    let mut book = Book::default();
    assert!(book.add(entry("s-a", NOW), NOW));
    assert!(
        !book.add(entry("s-a", NOW + 5), NOW + 5),
        "same chat waiting"
    );
    assert!(!book.add(entry("s-a", NOW), NOW), "same turn");
    assert!(book.add(entry("s-b", NOW + 5), NOW + 5), "another chat");
    assert_eq!(book.entries.len(), 2);
    let id = id_of(MINI, "s-a", NOW);
    assert!(book.claim(&id, NOW + 10));
    // The chat took another turn later: that one is waited for.
    assert!(book.add(entry("s-a", NOW + 20), NOW + 20));
    assert_eq!(book.pending().len(), 2);
    // The delivered one stays as a tombstone until pruned.
    assert_eq!(book.state_of(&id), Some(State::Delivered));
}

#[test]
fn a_notification_that_could_not_be_posted_is_given_back() {
    let mut book = Book::default();
    book.add(entry("s-a", NOW), NOW);
    let id = id_of(MINI, "s-a", NOW);
    assert!(!book.unclaim(&id, NOW), "only a claimed one");
    assert!(book.claim(&id, NOW + 1));
    assert!(book.unclaim(&id, NOW));
    assert_eq!(book.pending().len(), 1);
    assert!(book.claim(&id, NOW + 2), "and is claimed again");
    assert!(!book.claim(&id, NOW + 3));
    assert!(book.unclaim(&id, NOW));
    // A post that keeps failing is given up on, not retried for ever.
    for _ in 0..POST_TRIES_MAX - 3 {
        assert!(book.claim(&id, NOW));
        assert!(book.unclaim(&id, NOW));
    }
    assert!(book.claim(&id, NOW));
    assert!(!book.unclaim(&id, NOW), "the last try expires it");
    assert_eq!(book.state_of(&id), Some(State::Expired));
    assert!(!book.claim(&id, NOW));
}

#[test]
fn old_turns_expire_without_a_notification_and_tombstones_are_pruned() {
    let mut book = Book::default();
    book.add(entry("s-a", NOW), NOW);
    book.add(entry("s-b", NOW + 1), NOW + 1);
    // One millisecond short of the limit: still waiting.
    book.prune(NOW + FOLLOW_MAX_MS - 1);
    assert_eq!(book.pending().len(), 2);
    // At the limit the older expires (the other has a millisecond left).
    book.prune(NOW + FOLLOW_MAX_MS);
    let a = id_of(MINI, "s-a", NOW);
    assert_eq!(book.state_of(&a), Some(State::Expired));
    assert!(!book.claim(&a, NOW + FOLLOW_MAX_MS), "no notification");
    assert_eq!(book.pending().len(), 1);
    // Tombstones go once kept long enough, not before.
    let done_at = NOW + FOLLOW_MAX_MS;
    book.prune(done_at + DONE_KEPT_MS - 1);
    assert_eq!(book.state_of(&a), Some(State::Expired));
    book.prune(done_at + DONE_KEPT_MS);
    assert_eq!(book.state_of(&a), None);
}

#[test]
fn the_book_is_bounded_and_drops_done_entries_first() {
    let mut book = Book::default();
    for i in 0..ENTRIES_MAX {
        assert!(book.add(entry(&format!("s-{i}"), NOW + i as i64), NOW + 1000));
    }
    let first = id_of(MINI, "s-0", NOW);
    assert!(book.claim(&first, NOW + 2000));
    // One more: the delivered one makes room, no waiting turn is lost.
    assert!(book.add(entry("s-new", NOW + 3000), NOW + 3000));
    assert_eq!(book.entries.len(), ENTRIES_MAX);
    assert_eq!(book.state_of(&first), None);
    assert_eq!(book.pending().len(), ENTRIES_MAX);
    // Past that with nothing done, the oldest waiting turn goes.
    assert!(book.add(entry("s-newer", NOW + 4000), NOW + 4000));
    assert_eq!(book.entries.len(), ENTRIES_MAX);
    assert_eq!(book.state_of(&id_of(MINI, "s-1", NOW + 1)), None);
    assert!(book.state_of(&id_of(MINI, "s-newer", NOW + 4000)).is_some());
}

#[test]
fn a_file_that_is_missing_oversized_or_damaged_reads_as_empty() {
    let (_dir, path) = file();
    assert!(Book::load(&path).entries.is_empty());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"{ not json").unwrap();
    assert!(Book::load(&path).entries.is_empty());
    let bad = path.with_extension("json.bad");
    assert_eq!(
        std::fs::read(&bad).unwrap(),
        b"{ not json",
        "set aside, not lost"
    );
    assert!(!path.exists());
    assert!(!has_pending(&path));
    std::fs::write(&path, vec![b' '; FILE_MAX as usize + 1]).unwrap();
    assert!(Book::load(&path).entries.is_empty());
    // A wrong version is not read; an entry missing its chat is dropped.
    std::fs::write(&path, br#"{"version":9,"entries":[]}"#).unwrap();
    assert!(Book::load(&path).entries.is_empty());
    let mut bad = entry("s-a", NOW);
    bad.key.clear();
    let book = Book {
        version: VERSION,
        entries: vec![bad, entry("s-b", NOW)],
    };
    book.save(&path).unwrap();
    let loaded = Book::load(&path);
    assert_eq!(loaded.entries.len(), 1);
    assert_eq!(loaded.entries[0].key, "s-b");
    // The next save replaces a damaged file.
    std::fs::write(&path, b"junk").unwrap();
    Book::default().save(&path).unwrap();
    assert!(Book::load(&path).entries.is_empty());
}

#[cfg(unix)]
#[test]
fn the_file_is_private_and_leaves_no_temp_file() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, path) = file();
    let mut book = Book::default();
    book.add(entry("s-a", NOW), NOW);
    book.save(&path).unwrap();
    book.save(&path).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
}

#[test]
fn fields_are_clipped_and_stripped_of_control_characters() {
    let mut book = Book::default();
    let mut e = entry("s-a", NOW);
    e.what = format!("a\nb{}", "x".repeat(FIELD_MAX * 2));
    e.machine = "Mac\u{7}mini".into();
    assert!(book.add(e, NOW));
    let kept = &book.entries[0];
    assert_eq!(kept.what.chars().count(), FIELD_MAX);
    assert!(kept.what.starts_with("ab"));
    assert_eq!(kept.machine, "Macmini");
}

#[test]
fn a_restored_turn_keeps_its_age_and_id() {
    let at_ms = NOW - (11 * 3600 + 59 * 60) * 1000;
    let e = entry("s-a", at_ms);
    let t = crate::dispatcher_coo::RoutedTurn::restored(&e, NOW);
    assert_eq!(t.id(), e.id);
    assert_eq!(t.entry().id, e.id);
    assert!(t.at.elapsed() >= std::time::Duration::from_secs(11 * 3600 + 59 * 60));
    // A clock that went backwards doesn't panic: it counts as just routed.
    let future = crate::dispatcher_coo::RoutedTurn::restored(&entry("s-b", NOW + 5000), NOW);
    assert!(future.at.elapsed() < std::time::Duration::from_secs(5));
}
