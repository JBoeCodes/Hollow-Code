//! Row U6's first test (updates design section 9): what this build does
//! when a store's `PRAGMA user_version` is NEWER than the code knows, as an
//! older binary would meet it after Go back. Each test stamps a fixture
//! database with a higher version and opens it through the store's real
//! open path. The findings are in `docs/UPDATES.md`, "What an older binary
//! does with a newer database", and `goback::store_rule` follows them.

use rusqlite::Connection;
use std::path::Path;

fn version(db: &Path) -> i64 {
    Connection::open(db)
        .unwrap()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

fn table_exists(db: &Path, name: &str) -> bool {
    Connection::open(db)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name = ?1",
            [name],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
        == 1
}

/// Sessions: `session_store::open` never reads `user_version`. A database a
/// newer build stamped (with a table and a column this build doesn't know)
/// opens, takes writes and reads back as usual: nothing refuses, nothing
/// warns. Safe today only because every Sessions migration is additive
/// (`CREATE TABLE IF NOT EXISTS`, `ensure_column`), so a stamp above 0
/// would mean a change this build can't see: Go back treats it as
/// incompatible.
#[test]
fn sessions_opens_a_newer_stamped_database_blindly() {
    let vault = tempfile::tempdir().unwrap();
    let db = crate::session_store::db_path_for(vault.path()).unwrap();
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    {
        let conn = Connection::open(&db).unwrap();
        // A newer build's shape: today's table plus a column and a table
        // this build has never heard of.
        crate::session_store::apply_schema(&conn).unwrap();
        conn.execute_batch(
            "ALTER TABLE sessions ADD COLUMN future_flag INTEGER NOT NULL DEFAULT 7;
             CREATE TABLE future_rows(id INTEGER PRIMARY KEY, body TEXT);
             INSERT INTO future_rows(body) VALUES('kept');
             PRAGMA user_version = 9;",
        )
        .unwrap();
    }
    crate::session_store::start_turn(
        vault.path(),
        "s-goback-newer",
        "claude",
        "/tmp",
        "run-1",
        "hello",
    )
    .expect("the older code writes to the newer database without a word");
    let listed = crate::session_store::list_sessions(vault.path(), 10).unwrap();
    assert!(listed.iter().any(|s| s.key == "s-goback-newer"));
    assert_eq!(version(&db), 9, "the stamp is left as it was, unread");
    assert!(table_exists(&db, "future_rows"), "the unknown table stays");
    let flag: i64 = Connection::open(&db)
        .unwrap()
        .query_row(
            "SELECT future_flag FROM sessions WHERE key = 's-goback-newer'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(flag, 7, "rows it writes get the newer column's default");
}

/// The companion ledger (`Companion/applied.db`): the same. Its open path
/// (`Store::ledger`, reached here through `owed_receipts`) never reads
/// `user_version` and works on a newer-stamped file.
#[test]
fn companion_ledger_opens_a_newer_stamped_database_blindly() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Companion");
    std::fs::create_dir_all(&root).unwrap();
    let db = root.join("applied.db");
    Connection::open(&db)
        .unwrap()
        .execute_batch(
            "CREATE TABLE future_rows(id INTEGER PRIMARY KEY);
             PRAGMA user_version = 4;",
        )
        .unwrap();
    let store = crate::companion_inbox::Store::at(root);
    let owed = store
        .owed_receipts(10)
        .expect("the older code opens the newer ledger without a word");
    assert!(owed.is_empty());
    assert_eq!(version(&db), 4);
    assert!(table_exists(&db, "future_rows"));
}

/// The keyword index: `SearchIndex::open` refuses a newer index (search
/// scans the vault instead) and leaves its rows and stamp for the newer
/// build. Rebuildable, so Go back only says it is off.
#[test]
fn search_index_refuses_a_newer_index_and_leaves_it() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("search.db");
    let newer = crate::vault_search_index::SCHEMA_VERSION + 1;
    Connection::open(&db)
        .unwrap()
        .execute_batch(&format!(
            "CREATE TABLE meta(path TEXT PRIMARY KEY); PRAGMA user_version = {newer};"
        ))
        .unwrap();
    assert!(crate::vault_search_index::SearchIndex::open(&db).is_none());
    assert_eq!(version(&db), newer);
    assert!(table_exists(&db, "meta"));
}

/// The semantic index: `EmbedIndex::open` refuses the same way.
#[test]
fn embed_index_refuses_a_newer_index_and_leaves_it() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("embed.db");
    let newer = crate::vault_embed_index::SCHEMA_VERSION + 1;
    Connection::open(&db)
        .unwrap()
        .execute_batch(&format!(
            "CREATE TABLE chunks(path TEXT); PRAGMA user_version = {newer};"
        ))
        .unwrap();
    assert!(crate::vault_embed_index::EmbedIndex::open(&db).is_none());
    assert_eq!(version(&db), newer);
    assert!(table_exists(&db, "chunks"));
}

/// The Projects content index: its reader (`Snapshot::open`) wants exactly
/// its own version, and its writer (`open_rw`) refuses a newer one through
/// the same `newer_schema` check, so the index is off and kept.
#[test]
fn project_index_refuses_a_newer_index_and_leaves_it() {
    let vault = tempfile::tempdir().unwrap();
    let projects = tempfile::tempdir().unwrap();
    let on = crate::project_text_index::on_path(vault.path());
    std::fs::create_dir_all(on.parent().unwrap()).unwrap();
    std::fs::write(&on, b"").unwrap();
    let db = crate::project_text_index::db_path_for(vault.path(), projects.path()).unwrap();
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    let newer = crate::project_text_index::SCHEMA_VERSION + 1;
    {
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE files(id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE, \
               size INTEGER NOT NULL, mtime INTEGER NOT NULL, kind TEXT NOT NULL, \
               units INTEGER NOT NULL DEFAULT 0, chars INTEGER NOT NULL DEFAULT 0, error TEXT); \
             PRAGMA user_version = {newer};"
        ))
        .unwrap();
        assert_eq!(
            crate::vault_search_index::newer_schema(
                &conn,
                crate::project_text_index::SCHEMA_VERSION
            ),
            Some(newer),
            "the writer's refusal check sees it"
        );
    }
    assert!(crate::project_text_index::Snapshot::open(vault.path(), projects.path()).is_none());
    assert_eq!(version(&db), newer);
    let _ = std::fs::remove_file(&on);
    let _ = std::fs::remove_file(&db);
}

/// Review M1: Sessions and the companion ledger stamp their own
/// `SCHEMA_VERSION` on open, raising and never lowering it, and Go back's
/// `known_schema` names exactly those constants, so raising one is all a
/// breaking migration needs for Go back to see it.
#[test]
fn sessions_and_the_companion_ledger_stamp_what_go_back_compares() {
    let known = super::goback::known_schema();
    assert_eq!(known["sessions"], crate::session_store::SCHEMA_VERSION);
    assert_eq!(
        known["companion"],
        crate::companion_inbox::LEDGER_SCHEMA_VERSION
    );

    let dir = tempfile::tempdir().unwrap();
    let sessions = dir.path().join("sessions.db");
    crate::session_store::apply_schema(&Connection::open(&sessions).unwrap()).unwrap();
    assert_eq!(version(&sessions), crate::session_store::SCHEMA_VERSION);
    let root = dir.path().join("Companion");
    crate::companion_inbox::Store::at(root.clone())
        .owed_receipts(1)
        .unwrap();
    assert_eq!(
        version(&root.join("applied.db")),
        crate::companion_inbox::LEDGER_SCHEMA_VERSION
    );

    // The stamp raises a lower version and never lowers a higher one.
    let db = dir.path().join("stamp.db");
    let conn = Connection::open(&db).unwrap();
    crate::hollow_databases::stamp_schema_version(&conn, 3).unwrap();
    assert_eq!(version(&db), 3);
    crate::hollow_databases::stamp_schema_version(&conn, 1).unwrap();
    assert_eq!(version(&db), 3, "a newer build's stamp stays");
    // Through the real open path: a newer stamp is left as it was.
    conn.pragma_update(None, "user_version", 0).unwrap();
    crate::session_store::apply_schema(&conn).unwrap();
    assert_eq!(version(&db), crate::session_store::SCHEMA_VERSION);
}
