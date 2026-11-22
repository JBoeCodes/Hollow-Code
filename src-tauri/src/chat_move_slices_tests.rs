//! Lane M (2026-11-17): a chat longer than one sync moves in slices, in
//! order, each landed and recorded before the next, resumable after a
//! restart, with no turn lost or doubled and one owner at every step.
use super::*;
use crate::session_store as store;
use chrono::Utc;
use std::path::Path;

const MACBOOK: &str = "m-3f9c1a07e2b45d18";
const MINI: &str = "m-8d21c4be90f17a3e";

struct Two {
    _a: tempfile::TempDir,
    _b: tempfile::TempDir,
    a: PathBuf,
    b: PathBuf,
}

fn two() -> Two {
    let a_dir = tempfile::tempdir().unwrap();
    let b_dir = tempfile::tempdir().unwrap();
    let a = a_dir.path().canonicalize().unwrap();
    let b = b_dir.path().canonicalize().unwrap();
    crate::workspaces::ensure_quick_chats(&a).unwrap();
    crate::workspaces::ensure_quick_chats(&b).unwrap();
    Two {
        _a: a_dir,
        _b: b_dir,
        a,
        b,
    }
}

fn key(n: u32) -> String {
    format!("s-20261117-sl{n:06}")
}

/// A finished chat of `n` turns, each with its own question and answer.
fn seed(vault: &Path, key: &str, n: usize) {
    let cwd = crate::workspaces::ensure_quick_chats(vault).unwrap();
    let cwd = cwd.to_string_lossy().into_owned();
    for i in 0..n {
        let run = format!("run-{key}-{i}");
        let ord = store::start_turn(vault, key, "claude", &cwd, &run, &format!("Q{i}")).unwrap();
        store::record_turn_permission(vault, key, ord, "plan").unwrap();
        store::append_event(vault, key, ord, "assistant", &format!("A{i}")).unwrap();
        store::finish_turn(vault, key, ord, "success").unwrap();
    }
}

/// Give chat `key`'s session and turns one fixed time, so two chats seeded
/// at different moments encode to bundles of the same size. Without it the
/// size can differ by bytes: `start_turn` and `finish_turn` write
/// `to_rfc3339()`, which prints 0, 3, 6 or 9 fractional digits by the value.
fn pin_times(vault: &Path, key: &str) {
    const AT: &str = "2026-11-17T12:00:00.000000+00:00";
    store::with_write(vault, "test", |tx| {
        tx.execute(
            "UPDATE sessions SET created_at = ?2, last_activity_at = ?2 WHERE key = ?1",
            params![key, AT],
        )
        .and_then(|_| {
            tx.execute(
                "UPDATE turns SET started_at = ?2, ended_at = ?2 WHERE session_key = ?1",
                params![key, AT],
            )
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
    .unwrap();
}

/// `(ord, run id, question)` of every turn of `key`, in order.
fn turns_of(vault: &Path, key: &str) -> Vec<(i64, Option<String>, Option<String>)> {
    store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT ord, run_id, asked FROM turns WHERE session_key = ?1 ORDER BY ord")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![key], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .unwrap()
}

fn count(vault: &Path, table: &str) -> i64 {
    store::with_read(vault, |conn| {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .map_err(|e| e.to_string())
    })
    .unwrap()
}

fn caps(turns: usize) -> Caps {
    Caps {
        turns,
        ..Caps::SYNC
    }
}

fn commit_bytes(vault: &Path, id: &str, bytes: &[u8]) -> Result<Imported, MoveError> {
    let bundle: MoveBundle = serde_json::from_slice(bytes).unwrap();
    assert_eq!(bundle.move_id, id);
    import_move(vault, &bundle, MACBOOK, Utc::now())
}

/// Drive a sliced move to its end between two vaults, as the sender does:
/// the turns each sync carried.
fn drive(two: &Two, key: &str, root: &str, caps: Caps) -> Result<Vec<usize>, MoveError> {
    let mut sizes = Vec::new();
    loop {
        let (bundle, bytes, next) =
            begin_move_out_sliced(&two.a, key, MACBOOK, MINI, root, caps, Utc::now())?;
        sizes.push(bundle.turns.len());
        match next {
            Next::Slice(id) => {
                let staged = commit_bytes(&two.b, &id, &bytes)?;
                assert_eq!(staged.outcome, Some("staged"));
                slice_landed(&two.a, &id, Utc::now())?;
            }
            Next::Move => {
                commit_bytes(&two.b, root, &bytes)?;
                finish_move_out(&two.a, root, Utc::now())?;
                return Ok(sizes);
            }
        }
    }
}

/// The chat is whole on the other Mac and no slice is left anywhere.
fn assert_moved(two: &Two, key: &str, n: usize) {
    let sent = turns_of(&two.a, key);
    let got = turns_of(&two.b, key);
    assert_eq!(got.len(), n);
    assert_eq!(got, sent, "the same turns, in order, none doubled or lost");
    assert_eq!(
        got.iter().map(|t| t.0).collect::<Vec<_>>(),
        (1..=n as i64).collect::<Vec<_>>()
    );
    let there = store::session_by_key(&two.b, key).unwrap().unwrap();
    assert_eq!(there.turn_count as usize, n);
    assert_eq!(home_of(&two.b, key).unwrap().unwrap().state, ARRIVED);
    assert_eq!(home_of(&two.a, key).unwrap().unwrap().state, AWAY);
    assert_eq!(count(&two.a, "chat_slices_out"), 0);
    assert_eq!(count(&two.b, "chat_slices_in"), 0);
    assert!(admission_refusal(&two.b, key).is_none());
}

#[test]
fn a_chat_at_the_cap_goes_whole_and_one_past_it_in_slices() {
    // (turns, the cap, the turns each sync carries)
    let cases: [(usize, usize, &[usize]); 7] = [
        (4, 5, &[4]),
        (5, 5, &[5]),
        (6, 5, &[5, 1]),
        (10, 5, &[5, 5]),
        (11, 5, &[5, 5, 1]),
        (26, 5, &[5, 5, 5, 5, 5, 1]),
        (1, 1, &[1]),
    ];
    for (i, (n, cap, want)) in cases.into_iter().enumerate() {
        let two = two();
        let k = key(i as u32);
        seed(&two.a, &k, n);
        let root = new_move_id();
        let sizes = drive(&two, &k, &root, caps(cap)).unwrap();
        assert_eq!(sizes, want, "{n} turns at {cap}");
        assert_moved(&two, &k, n);
    }
}

#[test]
fn a_chat_that_fits_goes_exactly_as_before() {
    let two = two();
    let k = key(50);
    seed(&two.a, &k, 4);
    let root = new_move_id();
    let (plain, bytes) = begin_move_out(&two.a, &k, MACBOOK, MINI, &root, Utc::now()).unwrap();
    let (bundle, again, next) =
        begin_move_out_sliced(&two.a, &k, MACBOOK, MINI, &root, Caps::SYNC, Utc::now()).unwrap();
    assert_eq!(next, Next::Move);
    assert_eq!(bundle, plain);
    assert_eq!(again, bytes, "byte for byte");
    assert_eq!(bundle.version, BUNDLE_VERSION);
    assert!(bundle.slices.is_none());
    assert!(!String::from_utf8(bytes).unwrap().contains("slices"));
    assert_eq!(count(&two.a, "chat_slices_out"), 0);
}

#[test]
fn the_byte_cap_slices_too_one_byte_past_the_bundle() {
    let probe_two = two();
    let probe_key = key(60);
    seed(&probe_two.a, &probe_key, 8);
    pin_times(&probe_two.a, &probe_key);
    let probe = new_move_id();
    let (_, bytes) =
        begin_move_out(&probe_two.a, &probe_key, MACBOOK, MINI, &probe, Utc::now()).unwrap();
    let whole = bytes.len();
    // Abandon the probe; each case below is a chat of its own.
    for (i, (bundle, slices)) in [(whole, false), (whole + 1, false), (whole - 1, true)]
        .into_iter()
        .enumerate()
    {
        let two = two();
        let k = key(61 + i as u32);
        seed(&two.a, &k, 8);
        pin_times(&two.a, &k);
        let caps = Caps {
            turns: TURNS_MAX,
            bundle,
            slice_bytes: whole / 2,
        };
        let root = new_move_id();
        let sizes = drive(&two, &k, &root, caps).unwrap();
        assert_eq!(sizes.len() > 1, slices, "bundle cap {bundle} of {whole}");
        assert_eq!(sizes.iter().sum::<usize>(), 8);
        assert_moved(&two, &k, 8);
    }
}

#[test]
fn a_move_carries_at_most_the_slice_limit_and_says_so() {
    let two = two();
    // One turn a sync: SLICES_MAX earlier slices and the last one.
    let fits = key(70);
    seed(&two.a, &fits, SLICES_MAX + 1);
    let sizes = drive(&two, &fits, &new_move_id(), caps(1)).unwrap();
    assert_eq!(sizes.len(), SLICES_MAX + 1);
    assert_moved(&two, &fits, SLICES_MAX + 1);

    let long = key(71);
    seed(&two.a, &long, SLICES_MAX + 2);
    let err = drive(&two, &long, &new_move_id(), caps(1)).unwrap_err();
    assert_eq!(err.code, "too_large");
    // The refusal changed no ownership: the chat is still here, leaving
    // until the sender calls the move off.
    assert!(store::session_by_key(&two.b, &long).unwrap().is_none());
}

#[test]
fn a_restart_after_any_step_continues_where_it_stopped() {
    let two = two();
    let k = key(80);
    seed(&two.a, &k, 11);
    let root = new_move_id();
    let c = caps(5);
    let ask = || begin_move_out_sliced(&two.a, &k, MACBOOK, MINI, &root, c, Utc::now()).unwrap();

    // Slice 0 lands and is recorded.
    let (first, bytes, next) = ask();
    let Next::Slice(id0) = next else {
        panic!("a slice first")
    };
    assert_eq!(first.turns.len(), 5);
    commit_bytes(&two.b, &id0, &bytes).unwrap();
    slice_landed(&two.a, &id0, Utc::now()).unwrap();

    // Slice 1 is begun and the app stops before it is sent: the restart asks
    // again and gets the same slice under the same id, not slice 0.
    let (second, bytes, next) = ask();
    let Next::Slice(id1) = next else {
        panic!("slice 1")
    };
    assert_ne!(id1, id0);
    let (replay, replay_bytes, next) = ask();
    assert_eq!(next, Next::Slice(id1.clone()));
    assert_eq!(replay, second);
    assert_eq!(replay_bytes, bytes);

    // It reached the other Mac but the answer was lost: sending it again
    // answers already and stores nothing twice.
    let landed = commit_bytes(&two.b, &id1, &bytes).unwrap();
    assert!(!landed.already);
    assert_eq!(count(&two.b, "chat_slices_in"), 2);
    let again = commit_bytes(&two.b, &id1, &bytes).unwrap();
    assert!(again.already);
    assert_eq!(count(&two.b, "chat_slices_in"), 2);
    slice_landed(&two.a, &id1, Utc::now()).unwrap();
    slice_landed(&two.a, &id1, Utc::now()).unwrap();

    // The rest is the move's own bundle: it imports all 11.
    let (last, bytes, next) = ask();
    assert_eq!(next, Next::Move);
    assert_eq!(last.turns.len(), 1);
    assert_eq!(last.move_id, root);
    let imported = commit_bytes(&two.b, &root, &bytes).unwrap();
    assert_eq!((imported.turns, imported.already), (11, false));
    // The same commit again (its answer was lost) changes nothing.
    assert!(commit_bytes(&two.b, &root, &bytes).unwrap().already);
    finish_move_out(&two.a, &root, Utc::now()).unwrap();
    assert_moved(&two, &k, 11);
}

#[test]
fn nothing_of_a_chat_shows_on_the_other_mac_until_its_last_slice() {
    let two = two();
    let k = key(90);
    seed(&two.a, &k, 7);
    let root = new_move_id();
    let (_, bytes, next) =
        begin_move_out_sliced(&two.a, &k, MACBOOK, MINI, &root, caps(3), Utc::now()).unwrap();
    let Next::Slice(id) = next else { panic!() };
    commit_bytes(&two.b, &id, &bytes).unwrap();
    assert!(store::session_by_key(&two.b, &k).unwrap().is_none());
    assert!(home_of(&two.b, &k).unwrap().is_none());
    assert_eq!(count(&two.b, "turns"), 0);
    // While it waits, the chat is this Mac's alone and runs nowhere.
    assert!(admission_refusal(&two.a, &k).is_some());
    assert_eq!(home_of(&two.a, &k).unwrap().unwrap().state, LEAVING);
}

#[test]
fn a_last_slice_that_finds_a_slice_missing_is_refused_and_the_move_can_be_called_off() {
    let two = two();
    let k = key(100);
    seed(&two.a, &k, 11);
    let root = new_move_id();
    let c = caps(5);
    let last;
    loop {
        let (_, bytes, next) =
            begin_move_out_sliced(&two.a, &k, MACBOOK, MINI, &root, c, Utc::now()).unwrap();
        match next {
            Next::Slice(id) => {
                commit_bytes(&two.b, &id, &bytes).unwrap();
                slice_landed(&two.a, &id, Utc::now()).unwrap();
            }
            Next::Move => {
                last = bytes;
                break;
            }
        }
    }
    // The other Mac loses a waiting slice (pruned, a restored database).
    store::with_write(&two.b, "test", |tx| {
        tx.execute("DELETE FROM chat_slices_in WHERE no = 1", [])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .unwrap();
    let err = commit_bytes(&two.b, &root, &last).unwrap_err();
    assert_eq!(err.code, "conflict");
    assert!(store::session_by_key(&two.b, &k).unwrap().is_none());
    // Its answer proves nothing landed: the chat runs here again.
    abort_move_out(&two.a, &root, MINI, TargetSaid::CommitRefused, Utc::now()).unwrap();
    assert!(admission_refusal(&two.a, &k).is_none());
    assert_eq!(count(&two.a, "chat_slices_out"), 0);
    assert_eq!(turns_of(&two.a, &k).len(), 11);
    // The DELETE the sender then sends clears what waits there.
    cancel_move_in(&two.b, &root, MACBOOK, Utc::now()).unwrap();
    assert_eq!(count(&two.b, "chat_slices_in"), 0);
}

#[test]
fn a_move_called_off_drops_its_slices_and_refuses_the_late_ones() {
    let two = two();
    let k = key(110);
    seed(&two.a, &k, 11);
    let root = new_move_id();
    let c = caps(5);
    let (_, bytes0, next) =
        begin_move_out_sliced(&two.a, &k, MACBOOK, MINI, &root, c, Utc::now()).unwrap();
    let Next::Slice(id0) = next else { panic!() };
    commit_bytes(&two.b, &id0, &bytes0).unwrap();
    slice_landed(&two.a, &id0, Utc::now()).unwrap();
    let (_, bytes1, next) =
        begin_move_out_sliced(&two.a, &k, MACBOOK, MINI, &root, c, Utc::now()).unwrap();
    let Next::Slice(id1) = next else { panic!() };
    assert_eq!(count(&two.b, "chat_slices_in"), 1);

    let said = cancel_move_in(&two.b, &root, MACBOOK, Utc::now()).unwrap();
    assert_eq!(said, Cancelled::NotImported);
    assert_eq!(count(&two.b, "chat_slices_in"), 0);
    // A slice that was still on the wire is refused, never kept.
    let late = commit_bytes(&two.b, &id1, &bytes1).unwrap_err();
    assert_eq!(late.code, "cancelled");
    assert_eq!(count(&two.b, "chat_slices_in"), 0);
    abort_move_out(
        &two.a,
        &root,
        MINI,
        TargetSaid::Cancelled(Cancelled::NotImported),
        Utc::now(),
    )
    .unwrap();
    assert_eq!(count(&two.a, "chat_slices_out"), 0);
    assert!(admission_refusal(&two.a, &k).is_none());
}

#[test]
fn slices_arrive_in_order_from_the_same_mac_and_old_ones_are_pruned() {
    let two = two();
    let k = key(120);
    seed(&two.a, &k, 11);
    let root = new_move_id();
    let c = caps(5);
    let (_, bytes0, next) =
        begin_move_out_sliced(&two.a, &k, MACBOOK, MINI, &root, c, Utc::now()).unwrap();
    let Next::Slice(id0) = next else { panic!() };
    slice_landed(&two.a, &id0, Utc::now()).unwrap();
    let (b1, bytes1, next) =
        begin_move_out_sliced(&two.a, &k, MACBOOK, MINI, &root, c, Utc::now()).unwrap();
    let Next::Slice(id1) = next else { panic!() };

    // Slice 1 before slice 0: refused.
    let err = commit_bytes(&two.b, &id1, &bytes1).unwrap_err();
    assert_eq!(err.code, "conflict");
    assert_eq!(count(&two.b, "chat_slices_in"), 0);
    // Another Mac can't add to this one's move.
    let b0: MoveBundle = serde_json::from_slice(&bytes0).unwrap();
    import_move(&two.b, &b0, MACBOOK, Utc::now()).unwrap();
    let stranger = import_move(&two.b, &b1, "m-1111222233334444", Utc::now()).unwrap_err();
    assert_eq!(stranger.code, "forbidden");

    // An old slice is dropped by the next one that arrives (a day later).
    let later = Utc::now() + chrono::Duration::hours(25);
    let (_, bytes_x, _) = {
        let key2 = key(121);
        seed(&two.a, &key2, 7);
        let root2 = new_move_id();
        begin_move_out_sliced(&two.a, &key2, MACBOOK, MINI, &root2, c, Utc::now()).unwrap()
    };
    let bx: MoveBundle = serde_json::from_slice(&bytes_x).unwrap();
    import_move(&two.b, &bx, MACBOOK, later).unwrap();
    assert_eq!(count(&two.b, "chat_slices_in"), 1, "only the new one");
    // And so the old move's last slice finds its slice gone.
    slice_landed(&two.a, &id1, Utc::now()).unwrap();
    let (_, last, next) =
        begin_move_out_sliced(&two.a, &k, MACBOOK, MINI, &root, c, Utc::now()).unwrap();
    assert_eq!(next, Next::Move);
    let err = commit_bytes(&two.b, &root, &last).unwrap_err();
    assert_eq!(err.code, "conflict");
}

#[test]
fn the_slice_table_and_the_waiting_room_are_bounded() {
    let two = two();
    let k = key(130);
    seed(&two.a, &k, 4);
    let root = new_move_id();
    let (_, bytes, next) =
        begin_move_out_sliced(&two.a, &k, MACBOOK, MINI, &root, caps(1), Utc::now()).unwrap();
    let Next::Slice(_) = next else { panic!() };
    let mut bundle: MoveBundle = serde_json::from_slice(&bytes).unwrap();
    // Fill the waiting room with other moves' slices, then one more.
    for i in 0..SLICES_WAITING_MAX {
        bundle.move_id = new_move_id();
        let slices = bundle.slices.as_mut().unwrap();
        slices.root = new_move_id();
        bundle.key = key(1000 + i as u32);
        import_move(&two.b, &bundle, MACBOOK, Utc::now()).unwrap();
    }
    bundle.move_id = new_move_id();
    bundle.slices.as_mut().unwrap().root = new_move_id();
    bundle.key = key(2000);
    let err = import_move(&two.b, &bundle, MACBOOK, Utc::now()).unwrap_err();
    assert_eq!(err.code, "busy");
    assert_eq!(count(&two.b, "chat_slices_in"), SLICES_WAITING_MAX);
}

#[test]
fn a_sliced_bundle_with_a_bad_shape_is_refused() {
    let two = two();
    let k = key(140);
    seed(&two.a, &k, 7);
    let root = new_move_id();
    let (_, bytes, _) =
        begin_move_out_sliced(&two.a, &k, MACBOOK, MINI, &root, caps(3), Utc::now()).unwrap();
    let good: MoveBundle = serde_json::from_slice(&bytes).unwrap();
    let refused = |edit: &dyn Fn(&mut MoveBundle)| {
        let mut b = good.clone();
        edit(&mut b);
        import_move(&two.b, &b, MACBOOK, Utc::now())
            .unwrap_err()
            .code
    };
    assert_eq!(refused(&|b| b.version = BUNDLE_VERSION), "bad_request");
    assert_eq!(
        refused(&|b| b.slices.as_mut().unwrap().root = b.move_id.clone()),
        "bad_request"
    );
    assert_eq!(
        refused(&|b| b.slices.as_mut().unwrap().root = "nope".into()),
        "bad_request"
    );
    assert_eq!(
        refused(&|b| b.slices.as_mut().unwrap().no = SLICES_MAX as u32),
        "bad_request"
    );
    assert_eq!(refused(&|b| b.turns.clear()), "bad_request");
    assert_eq!(
        refused(&|b| {
            b.append = Some(crate::chat_offline::Append {
                to: "s-other".into(),
                base: crate::chat_offline::Base {
                    ord: 0,
                    run_id: None,
                },
            })
        }),
        "bad_request"
    );
    assert_eq!(count(&two.b, "chat_slices_in"), 0);
}

#[test]
fn a_chat_that_cannot_be_sliced_is_still_refused_and_stays_put() {
    let two = two();
    let k = key(150);
    seed(&two.a, &k, 2);
    // Caps so small that even one turn doesn't fit a bundle.
    let tiny = Caps {
        turns: 5,
        bundle: 10,
        slice_bytes: 5,
    };
    let root = new_move_id();
    let err =
        begin_move_out_sliced(&two.a, &k, MACBOOK, MINI, &root, tiny, Utc::now()).unwrap_err();
    assert_eq!(err.code, "too_large");
    assert!(
        home_of(&two.a, &k).unwrap().is_none(),
        "nothing was changed"
    );
    assert_eq!(count(&two.a, "chat_slices_out"), 0);
}

/// One earlier slice of a fresh chat (`n` turns, cap 3), as bytes.
fn first_slice(two: &Two, n: u32, turns: usize) -> MoveBundle {
    let k = key(n);
    seed(&two.a, &k, turns);
    let (_, bytes, next) = begin_move_out_sliced(
        &two.a,
        &k,
        MACBOOK,
        MINI,
        &new_move_id(),
        caps(3),
        Utc::now(),
    )
    .unwrap();
    assert!(matches!(next, Next::Slice(_)));
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn the_bytes_of_waiting_slices_are_bounded() {
    let two = two();
    let one = first_slice(&two, 160, 7);
    let size = serde_json::to_string(&one.turns).unwrap().len() as i64;
    // Room for exactly two of them.
    let mut a = one.clone();
    a.slices.as_mut().unwrap().root = new_move_id();
    a.move_id = new_move_id();
    a.key = key(161);
    let mut b = a.clone();
    b.slices.as_mut().unwrap().root = new_move_id();
    b.move_id = new_move_id();
    b.key = key(162);
    let mut c = b.clone();
    c.slices.as_mut().unwrap().root = new_move_id();
    c.move_id = new_move_id();
    c.key = key(163);
    let max = size * 2;
    store_slice(&two.b, &a, MACBOOK, Utc::now(), max).unwrap();
    store_slice(&two.b, &b, MACBOOK, Utc::now(), max).unwrap();
    let err = store_slice(&two.b, &c, MACBOOK, Utc::now(), max).unwrap_err();
    assert_eq!(err.code, "busy");
    assert_eq!(count(&two.b, "chat_slices_in"), 2);
    // A move called off frees its bytes.
    cancel_move_in(
        &two.b,
        &a.slices.as_ref().unwrap().root,
        MACBOOK,
        Utc::now(),
    )
    .unwrap();
    store_slice(&two.b, &c, MACBOOK, Utc::now(), max).unwrap();
    assert_eq!(count(&two.b, "chat_slices_in"), 2);
}

#[test]
fn waiting_slices_expire_after_their_keeping_time() {
    let two = two();
    let one = first_slice(&two, 170, 7);
    let at = Utc::now();
    let mk = |n: u32| {
        let mut b = one.clone();
        b.slices.as_mut().unwrap().root = new_move_id();
        b.move_id = new_move_id();
        b.key = key(n);
        b
    };
    store_slice(&two.b, &mk(171), MACBOOK, at, i64::MAX).unwrap();
    let kept = chrono::Duration::from_std(SLICES_KEPT_FOR).unwrap();
    // A second just inside the keeping time: the first stays.
    store_slice(
        &two.b,
        &mk(172),
        MACBOOK,
        at + kept - chrono::Duration::seconds(1),
        i64::MAX,
    )
    .unwrap();
    assert_eq!(count(&two.b, "chat_slices_in"), 2);
    // One just past it: the first is pruned, the second is not yet old.
    store_slice(
        &two.b,
        &mk(173),
        MACBOOK,
        at + kept + chrono::Duration::seconds(1),
        i64::MAX,
    )
    .unwrap();
    assert_eq!(count(&two.b, "chat_slices_in"), 2);
}
