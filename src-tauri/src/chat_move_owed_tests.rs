//! Lane M: owed continuations and told notifications survive a restart,
//! fire once, and are pruned.
use super::*;

const NOW: i64 = 1_800_000_000_000;
const MV: &str = "mv-0123456789abcdef";

fn file() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Peers").join(FILE);
    (dir, path)
}

#[test]
fn an_owed_continuation_written_before_a_restart_is_taken_once_after_it() {
    let (_dir, path) = file();
    let mut book = Book::default();
    book.owe(MV, "s-a", NOW);
    book.save(&path).unwrap();
    drop(book);

    let mut book = Book::load(&path);
    assert!(book.owes("s-a"));
    assert_eq!(book.owed_moves(), vec![(MV.to_string(), "s-a".to_string())]);
    assert_eq!(book.take(MV).as_deref(), Some("s-a"));
    book.save(&path).unwrap();
    assert_eq!(book.take(MV), None, "taken once");

    let mut book = Book::load(&path);
    assert!(!book.owes("s-a"));
    assert_eq!(book.take(MV), None, "and not again after another restart");
}

#[test]
fn owing_a_move_twice_keeps_one_entry_and_its_first_time() {
    let mut book = Book::default();
    book.owe(MV, "s-a", NOW);
    book.owe(MV, "s-b", NOW + 5);
    assert_eq!(book.owed.len(), 1);
    assert_eq!(book.owed[0].key, "s-b");
    assert_eq!(book.owed[0].at_ms, NOW);
}

#[test]
fn an_owed_continuation_expires_and_a_told_notification_is_forgotten_later() {
    let mut book = Book::default();
    book.owe(MV, "s-a", NOW);
    assert!(book.mark_reported(MV, "stuck", NOW));
    book.prune(NOW + OWED_KEPT_MS - 1);
    assert!(book.owes("s-a"));
    book.prune(NOW + OWED_KEPT_MS);
    assert!(!book.owes("s-a"), "expired");
    assert!(
        !book.mark_reported(MV, "stuck", NOW + OWED_KEPT_MS),
        "still remembered"
    );
    book.prune(NOW + REPORTED_KEPT_MS - 1);
    assert!(!book.mark_reported(MV, "stuck", NOW + REPORTED_KEPT_MS - 1));
    book.prune(NOW + REPORTED_KEPT_MS);
    assert!(book.reported.is_empty());
}

#[test]
fn a_notification_fires_once_and_a_failed_post_can_be_tried_again() {
    let (_dir, path) = file();
    let mut book = Book::default();
    assert!(book.mark_reported(MV, "stuck", NOW));
    assert!(!book.mark_reported(MV, "stuck", NOW + 1), "never twice");
    assert!(
        book.mark_reported(MV, "home", NOW),
        "another kind is its own"
    );
    book.save(&path).unwrap();
    // After a restart it still isn't told again.
    let mut book = Book::load(&path);
    assert!(!book.mark_reported(MV, "stuck", NOW + 2));
    // Posting failed: the mark is taken back and the next try tells it.
    book.unmark_reported(MV, "home");
    assert!(book.mark_reported(MV, "home", NOW + 3));
}

#[test]
fn both_lists_are_bounded_and_drop_the_oldest() {
    let mut book = Book::default();
    for i in 0..OWED_MAX + 3 {
        book.owe(&format!("mv-{i:016x}"), "s-a", NOW + i as i64);
    }
    assert_eq!(book.owed.len(), OWED_MAX);
    assert!(
        book.take(&format!("mv-{:016x}", 0)).is_none(),
        "the oldest went"
    );
    assert!(book.take(&format!("mv-{:016x}", OWED_MAX + 2)).is_some());
    for i in 0..REPORTED_MAX + 3 {
        book.mark_reported(&format!("mv-{i:016x}"), "stuck", NOW + i as i64);
    }
    assert_eq!(book.reported.len(), REPORTED_MAX);
    assert!(book.mark_reported(&format!("mv-{:016x}", 0), "stuck", NOW + 10_000));
}

#[test]
fn a_missing_oversized_or_damaged_file_reads_as_empty_and_is_replaced() {
    let (_dir, path) = file();
    assert_eq!(Book::load(&path), Book::default());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"{ nope").unwrap();
    assert_eq!(Book::load(&path), Book::default());
    let bad = path.with_extension("json.bad");
    assert_eq!(
        std::fs::read(&bad).unwrap(),
        b"{ nope",
        "set aside, not lost"
    );
    assert!(!path.exists());
    std::fs::write(&path, vec![b' '; FILE_MAX as usize + 1]).unwrap();
    assert_eq!(Book::load(&path), Book::default());
    std::fs::write(
        &path,
        br#"{"version":1,"owed":[{"moveId":"","key":"s-a","atMs":5},{"moveId":"mv-1","key":"s-b","atMs":5}],"reported":[]}"#,
    )
    .unwrap();
    let book = Book::load(&path);
    assert_eq!(book.owed.len(), 1, "an entry with no move id is dropped");
    Book::default().save(&path).unwrap();
    assert_eq!(Book::load(&path), Book::default());
}

#[cfg(unix)]
#[test]
fn the_file_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, path) = file();
    let mut book = Book::default();
    book.owe(MV, "s-a", NOW);
    book.save(&path).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
