//! V4 tests: the copy rules, the pure planner, the ssh command, and whole
//! cycles against a peer that runs the real peer program through `sh -c`
//! over a second temp folder (no ssh, no network).

use super::*;
use std::time::{Duration, UNIX_EPOCH};

fn e(size: u64, mtime: i64, hash: Option<&str>) -> Entry {
    Entry {
        size,
        mtime,
        hash: hash.map(str::to_string),
    }
}

fn listing(items: &[(&str, Entry)]) -> Listing {
    items
        .iter()
        .map(|(p, e)| (p.to_string(), e.clone()))
        .collect()
}

#[test]
fn copyable_takes_binaries_and_big_files_and_never_text_or_private_folders() {
    let yes = [
        ("workspaces/w/outputs/shot.png", 10),
        ("workspaces/w/outputs/report.PDF", 10),
        ("workspaces/w/outputs/screenshots/a.html", 10),
        ("workspaces/w/recordings/call.m4a", 10),
        ("workspaces/w/sources/export.csv", 10),
        ("workspaces/w/outputs/page.html", MAX_BYTES + 1),
        ("library/clip.mov", 10),
    ];
    for (p, size) in yes {
        assert!(copyable(p, size), "{p} should copy");
    }
    let no = [
        ("workspaces/w/notes.md", MAX_BYTES * 3),
        ("workspaces/w/big.txt", MAX_BYTES * 3),
        ("workspaces/w/data.json", MAX_BYTES * 3),
        ("workspaces/w/log.jsonl", MAX_BYTES * 3),
        ("workspaces/w/conf.yaml", MAX_BYTES * 3),
        ("workspaces/w/conf.YML", MAX_BYTES * 3),
        ("workspaces/w/Cargo.toml", MAX_BYTES * 3),
        ("workspaces/w/prod.env", MAX_BYTES * 3),
        ("workspaces/w/dump.sql", MAX_BYTES * 3),
        ("workspaces/w/outputs/page.html", 10),
        ("me/photo.png", 10),
        ("Memory/x.png", 10),
        ("learnings/x.png", 10),
        ("home/x.png", 10),
        ("tasks/x.png", 10),
        (".hollow-history/x.png", 10),
        ("workspaces/w/.hollow-attachments/x.png", 10),
        ("workspaces/w/.shot.png.hollow-part", 10),
        ("workspaces/w/node_modules/x.png", 10),
        ("workspaces/w/target/debug/x.bin", 10),
        ("workspaces/w/test-results/x.png", 10),
        ("workspaces/w/id_ed25519", MAX_BYTES * 3),
        ("workspaces/w/server.pem", 10),
        ("workspaces/w/db.sqlite-wal", 10),
        ("workspaces/w/x.png~", 10),
        ("workspaces//x.png", 10),
        ("workspaces/../x.png", 10),
    ];
    for (p, size) in no {
        assert!(!copyable(p, size), "{p} should never copy");
    }
}

#[test]
fn wanted_hashes_asks_only_what_the_plan_needs() {
    let local = listing(&[
        ("a.png", e(1, 1, None)),
        ("b.png", e(1, 1, None)),
        ("c.png", e(1, 1, Some("h"))),
        ("d.png", e(2, 1, None)),
    ]);
    let peer = listing(&[
        ("a.png", e(1, 1, None)),
        ("d.png", e(3, 1, None)),
        ("e.png", e(3, 1, None)),
        ("f.png", e(3, 1, Some("x"))),
    ]);
    let (here, there) = wanted_hashes(&local, &peer);
    assert_eq!(here, vec!["b.png", "d.png"]);
    assert_eq!(there, vec!["d.png", "e.png"]);
}

fn h(c: char) -> String {
    c.to_string().repeat(64)
}

#[test]
fn plan_covers_every_case() {
    let (a, b, c) = (h('a'), h('b'), h('c'));
    let local = listing(&[
        ("only-here.png", e(1, 1, Some(&a))),
        ("same-stat.png", e(1, 5, None)),
        ("same-hash.png", e(1, 5, Some(&a))),
        ("unhashed.png", e(1, 5, None)),
        ("kept.png", e(1, 5, Some(&a))),
        ("ours-newer.png", e(2, 9, Some(&b))),
        ("theirs-newer.png", e(1, 5, Some(&a))),
        ("ours-older.png", e(2, 1, Some(&b))),
        ("dir/clash.png", e(1, 5, Some(&a))),
    ]);
    let peer = listing(&[
        ("only-there.png", e(1, 1, Some(&a))),
        ("same-stat.png", e(1, 5, None)),
        ("same-hash.png", e(1, 7, Some(&a))),
        ("unhashed.png", e(2, 5, Some(&a))),
        ("kept.png", e(2, 5, Some(&b))),
        ("ours-newer.png", e(1, 5, Some(&a))),
        ("theirs-newer.png", e(2, 9, Some(&b))),
        ("ours-older.png", e(1, 5, Some(&a))),
        ("dir/clash.png", e(2, 6, Some(&c))),
        ("dir/clash (from mini).png", e(9, 9, Some(&c))),
    ]);
    let synced: BTreeMap<String, String> = [
        ("ours-newer.png", &a),
        ("theirs-newer.png", &a),
        ("ours-older.png", &a),
    ]
    .iter()
    .map(|(p, h)| (p.to_string(), h.to_string()))
    .collect();
    let clashes: BTreeMap<String, [String; 2]> =
        [("kept.png".to_string(), [a.clone(), b.clone()])].into();
    let got = plan(
        &local,
        &peer,
        &synced,
        &clashes,
        &BTreeSet::new(),
        ("laptop", "mini"),
    );
    let want = vec![
        // A space sorts before the dot, so the peer's earlier copy comes first.
        Action::FromPeer("dir/clash (from mini).png".into()),
        Action::KeepBoth {
            path: "dir/clash.png".into(),
            here: "dir/clash (from mini 2).png".into(),
            there: "dir/clash (from laptop).png".into(),
        },
        Action::Skip("kept.png".into(), SkipWhy::KeptBoth),
        Action::ToPeer("only-here.png".into()),
        Action::FromPeer("only-there.png".into()),
        Action::ReplacePeer("ours-newer.png".into()),
        Action::KeepBoth {
            path: "ours-older.png".into(),
            here: "ours-older (from mini).png".into(),
            there: "ours-older (from laptop).png".into(),
        },
        Action::Skip("same-hash.png".into(), SkipWhy::Same),
        Action::Skip("same-stat.png".into(), SkipWhy::Same),
        Action::ReplaceHere("theirs-newer.png".into()),
        Action::Skip("unhashed.png".into(), SkipWhy::Unhashed),
    ];
    assert_eq!(got, want);
}

#[test]
fn plan_keeps_both_when_nothing_was_copied_before_even_if_one_is_newer() {
    let local = listing(&[("x.png", e(1, 9, Some(&h('a'))))]);
    let peer = listing(&[("x.png", e(1, 5, Some(&h('b'))))]);
    let got = plan(
        &local,
        &peer,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeSet::new(),
        ("a", "b"),
    );
    assert!(matches!(&got[0], Action::KeepBoth { .. }), "{got:?}");
}

#[test]
fn ssh_argv_is_batch_mode_with_a_connect_timeout_and_no_forwarding() {
    let argv = ssh_argv("runner-host");
    assert_eq!(argv[0], "ssh");
    let joined = argv.join(" ");
    for o in [
        "BatchMode=yes",
        "ConnectTimeout=10",
        "PasswordAuthentication=no",
        "KbdInteractiveAuthentication=no",
        "ForwardAgent=no",
        "ClearAllForwardings=yes",
    ] {
        assert!(joined.contains(&format!("-o {o}")), "{o}");
    }
    assert_eq!(&argv[argv.len() - 2..], ["--", "runner-host"]);
}

#[test]
fn remote_command_quotes_every_argument() {
    let cmd = remote_command("recv", "Hollow/It's a vault", &["a b/c'd.png".to_string()]);
    assert!(cmd.starts_with("perl -e '"));
    assert!(cmd.ends_with(" -- 'recv' 'Hollow/It'\\''s a vault' 'a b/c'\\''d.png'"));
    assert!(cmd.contains("node_modules,__pycache__"));
    assert!(!cmd.contains("__NEVER__"));
}

#[test]
fn parse_listing_reads_host_and_filters() {
    let bytes = b"H\tbuild-mac\x0012\t100\tw/a.png\x003\t1\tw/notes.md\x00bad\x00";
    let (host, l) = parse_listing(bytes);
    assert_eq!(host.as_deref(), Some("build-mac"));
    assert_eq!(l.len(), 1);
    assert_eq!(l["w/a.png"], e(12, 100, None));
}

#[test]
fn remote_vault_paths() {
    assert!(valid_remote_vault("Hollow/Vault"));
    assert!(valid_remote_vault("/Users/x/Hollow/Vault"));
    for bad in ["", "/", "../x", "Hollow/../x", "a\nb", "a//b"] {
        assert!(!valid_remote_vault(bad), "{bad:?}");
    }
    assert_eq!(
        default_remote_vault(
            Path::new("/Users/j/Hollow/Vault"),
            Some(Path::new("/Users/j"))
        ),
        Some("Hollow/Vault".into())
    );
    assert_eq!(
        default_remote_vault(Path::new("/Volumes/v"), Some(Path::new("/Users/j"))),
        None
    );
}

#[test]
fn status_is_fetch_on_demand_and_keeps_the_old_run_as_history() {
    let mut r = Record::default();
    let st = status_from(&r, Some("mini".into()), Some("v".into()));
    assert_eq!(
        (st.mode.as_str(), st.state.as_str()),
        ("fetchOnDemand", "ready")
    );
    assert_eq!(status_from(&r, None, Some("v".into())).state, "noPeer");
    assert_eq!(status_from(&r, Some("mini".into()), None).state, "noPeer");
    r.last_state = Some("offline".into());
    r.last_run_at = Some("2026-11-15T00:00:00Z".into());
    let st = status_from(&r, Some("mini".into()), Some("v".into()));
    assert_eq!(st.state, "ready");
    assert_eq!(st.last_run_at.as_deref(), Some("2026-11-15T00:00:00Z"));
    // An old file-copy.json that said on still reads; nothing turns it on.
    let old: Settings = serde_json::from_str(r#"{"enabled":true,"remoteVault":"V"}"#).unwrap();
    assert!(old.enabled);
    // The switch refuses on, saves off, and checks the vault path.
    let mut set = old.clone();
    assert_eq!(
        apply_switch(&mut set, Some(true), None),
        Err(RETIRED.to_string())
    );
    assert_eq!(set, old);
    apply_switch(&mut set, None, Some(" Other/Vault/ ".into())).unwrap();
    assert!(!set.enabled);
    assert_eq!(set.remote_vault.as_deref(), Some("Other/Vault"));
    assert!(apply_switch(&mut set, Some(false), Some("../x".into())).is_err());
    apply_switch(&mut set, Some(false), Some(String::new())).unwrap();
    assert_eq!(set.remote_vault, None);
}

#[test]
fn record_round_trips_as_json() {
    let mut r = Record {
        peer: Some("mini".into()),
        ..Record::default()
    };
    r.here.insert("a.png".into(), e(1, 2, Some("h")));
    r.clashes.insert("b.png".into(), ["x".into(), "y".into()]);
    let text = serde_json::to_string(&r).unwrap();
    assert!(text.contains("\"lastErrors\""));
    assert_eq!(serde_json::from_str::<Record>(&text).unwrap(), r);
}

// ── Whole cycles against a local peer ─────────────────────────────────

struct Rig {
    _tmp: tempfile::TempDir,
    here: PathBuf,
    there: PathBuf,
    record: Record,
}

impl Rig {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        // A space and a quote in the peer's path prove the quoting.
        let here = tmp.path().join("here vault");
        let there = tmp.path().join("there's vault");
        fs::create_dir_all(&here).unwrap();
        fs::create_dir_all(&there).unwrap();
        Rig {
            _tmp: tmp,
            here,
            there,
            record: Record::default(),
        }
    }

    fn peer(&self) -> ShellPeer {
        ShellPeer {
            id: "fake-peer".into(),
            prefix: vec!["/bin/sh".into(), "-c".into()],
            root: self.there.to_str().unwrap().into(),
            timeout: STEP_TIMEOUT,
        }
    }

    fn cycle(&mut self) -> Report {
        let mut peer = self.peer();
        run_cycle(
            &self.here,
            &mut self.record,
            &mut peer,
            "laptop",
            CYCLE_BYTES,
        )
    }

    fn cycle_with(&mut self, peer: &mut dyn Peer) -> Report {
        run_cycle(&self.here, &mut self.record, peer, "laptop", CYCLE_BYTES)
    }
}

fn put(root: &Path, rel: &str, body: &[u8], mtime: u64) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, body).unwrap();
    fs::File::options()
        .write(true)
        .open(&p)
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_secs(mtime))
        .unwrap();
}

fn read(root: &Path, rel: &str) -> Option<Vec<u8>> {
    fs::read(root.join(rel)).ok()
}

fn mtime(root: &Path, rel: &str) -> i64 {
    secs(fs::metadata(root.join(rel)).unwrap().modified().unwrap())
}

#[test]
fn a_round_trip_copies_binaries_both_ways_and_nothing_else() {
    let mut rig = Rig::new();
    put(
        &rig.here,
        "workspaces/w/outputs/shot.png",
        b"png here",
        1_700_000_000,
    );
    put(&rig.here, "workspaces/w/notes.md", b"text", 1_700_000_000);
    put(&rig.here, "me/face.png", b"private", 1_700_000_000);
    put(
        &rig.here,
        "workspaces/w/.hidden/x.png",
        b"dot",
        1_700_000_000,
    );
    put(
        &rig.there,
        "workspaces/w/recordings/call.m4a",
        b"audio there",
        1_700_000_100,
    );
    put(&rig.there, "workspaces/w/data.json", b"{}", 1_700_000_100);
    put(&rig.there, "tasks/t.png", b"task", 1_700_000_100);

    let r = rig.cycle();
    assert!(!r.offline && r.errors.is_empty(), "{r:?}");
    assert_eq!(r.there, vec!["workspaces/w/outputs/shot.png"]);
    assert_eq!(r.here, vec!["workspaces/w/recordings/call.m4a"]);
    assert_eq!(
        read(&rig.there, "workspaces/w/outputs/shot.png").unwrap(),
        b"png here"
    );
    assert_eq!(
        read(&rig.here, "workspaces/w/recordings/call.m4a").unwrap(),
        b"audio there"
    );
    assert_eq!(
        mtime(&rig.there, "workspaces/w/outputs/shot.png"),
        1_700_000_000
    );
    assert_eq!(
        mtime(&rig.here, "workspaces/w/recordings/call.m4a"),
        1_700_000_100
    );
    for (root, rel) in [
        (&rig.there, "workspaces/w/notes.md"),
        (&rig.there, "me/face.png"),
        (&rig.there, "workspaces/w/.hidden/x.png"),
        (&rig.here, "workspaces/w/data.json"),
        (&rig.here, "tasks/t.png"),
    ] {
        assert!(read(root, rel).is_none(), "{rel} must not be copied");
    }
    // No part files left behind.
    assert!(read(&rig.there, "workspaces/w/outputs/.shot.png.hollow-part").is_none());
    assert!(read(&rig.here, "workspaces/w/recordings/.call.m4a.hollow-part").is_none());

    // Second cycle: the other side has everything; nothing moves.
    let r = rig.cycle();
    assert!(
        r.here.is_empty() && r.there.is_empty() && r.errors.is_empty(),
        "{r:?}"
    );
    assert_eq!(rig.record.here.len(), 2);
    assert_eq!(rig.record.there.len(), 2);
}

#[test]
fn a_planted_name_clash_keeps_both_on_both_macs() {
    let mut rig = Rig::new();
    let rel = "workspaces/w/outputs/screenshots/home.png";
    put(&rig.here, rel, b"laptop version", 1_700_000_200);
    put(&rig.there, rel, b"mini version, longer", 1_700_000_100);

    let r = rig.cycle();
    assert!(r.errors.is_empty(), "{r:?}");
    assert_eq!(r.kept_both, vec![rel]);
    let peer = rig.record.peer_name.clone().unwrap();
    let theirs_here = format!("workspaces/w/outputs/screenshots/home (from {peer}).png");
    let ours_there = "workspaces/w/outputs/screenshots/home (from laptop).png";
    // Neither original was overwritten.
    assert_eq!(read(&rig.here, rel).unwrap(), b"laptop version");
    assert_eq!(read(&rig.there, rel).unwrap(), b"mini version, longer");
    assert_eq!(
        read(&rig.here, &theirs_here).unwrap(),
        b"mini version, longer"
    );
    assert_eq!(read(&rig.there, ours_there).unwrap(), b"laptop version");

    // Next cycle: each side gets the other's copy too; the clash is not
    // repeated.
    let r = rig.cycle();
    assert!(r.errors.is_empty() && r.kept_both.is_empty(), "{r:?}");
    for root in [&rig.here, &rig.there] {
        assert_eq!(read(root, &theirs_here).unwrap(), b"mini version, longer");
        assert_eq!(read(root, ours_there).unwrap(), b"laptop version");
    }
    assert_eq!(read(&rig.here, rel).unwrap(), b"laptop version");
    assert_eq!(read(&rig.there, rel).unwrap(), b"mini version, longer");
    let r = rig.cycle();
    assert_eq!(r, Report::default());
}

#[test]
fn newest_wins_only_over_the_version_both_held() {
    let mut rig = Rig::new();
    let rel = "workspaces/w/outputs/report.pdf";
    put(&rig.here, rel, b"v1", 1_700_000_000);
    rig.cycle();
    assert_eq!(read(&rig.there, rel).unwrap(), b"v1");
    put(&rig.here, rel, b"v2 newer", 1_700_000_500);
    let r = rig.cycle();
    assert!(r.errors.is_empty() && r.kept_both.is_empty(), "{r:?}");
    assert_eq!(read(&rig.there, rel).unwrap(), b"v2 newer");
    // And back: the peer writes a newer one.
    put(&rig.there, rel, b"v3 from the peer", 1_700_000_900);
    let r = rig.cycle();
    assert!(r.errors.is_empty() && r.kept_both.is_empty(), "{r:?}");
    assert_eq!(read(&rig.here, rel).unwrap(), b"v3 from the peer");
}

#[test]
fn never_deletes_a_file_removed_on_one_side_comes_back() {
    let mut rig = Rig::new();
    let rel = "workspaces/w/outputs/a.png";
    put(&rig.here, rel, b"keep me", 1_700_000_000);
    rig.cycle();
    fs::remove_file(rig.there.join(rel)).unwrap();
    let r = rig.cycle();
    assert_eq!(r.there, vec![rel]);
    assert_eq!(read(&rig.there, rel).unwrap(), b"keep me");
    assert_eq!(read(&rig.here, rel).unwrap(), b"keep me");
}

#[test]
fn an_interrupted_push_resumes_from_its_part() {
    let mut rig = Rig::new();
    let rel = "workspaces/w/outputs/big.mov";
    let body: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    put(&rig.here, rel, &body, 1_700_000_000);
    put(
        &rig.there,
        "workspaces/w/outputs/.big.mov.hollow-part",
        &body[..120_000],
        1,
    );
    let r = rig.cycle();
    assert!(r.errors.is_empty(), "{r:?}");
    assert_eq!(r.resumed, vec![(rel.to_string(), 120_000)]);
    assert_eq!(read(&rig.there, rel).unwrap(), body);
    assert!(read(&rig.there, "workspaces/w/outputs/.big.mov.hollow-part").is_none());
}

#[test]
fn an_interrupted_pull_resumes_and_a_bad_part_starts_over() {
    let mut rig = Rig::new();
    let rel = "workspaces/w/recordings/talk.m4a";
    let body: Vec<u8> = (0..200_000u32).map(|i| (i % 241) as u8).collect();
    put(&rig.there, rel, &body, 1_700_000_000);
    put(
        &rig.here,
        "workspaces/w/recordings/.talk.m4a.hollow-part",
        &body[..50_000],
        1,
    );
    let r = rig.cycle();
    assert!(r.errors.is_empty(), "{r:?}");
    assert_eq!(r.resumed, vec![(rel.to_string(), 50_000)]);
    assert_eq!(read(&rig.here, rel).unwrap(), body);

    // A part holding the wrong bytes fails its check, is removed, and the
    // next cycle copies cleanly.
    let rel2 = "workspaces/w/recordings/other.m4a";
    put(&rig.there, rel2, &body, 1_700_000_000);
    put(
        &rig.here,
        "workspaces/w/recordings/.other.m4a.hollow-part",
        &[7u8; 1000],
        1,
    );
    let r = rig.cycle();
    assert_eq!(r.errors.len(), 1, "{r:?}");
    assert!(read(&rig.here, rel2).is_none());
    assert!(read(&rig.here, "workspaces/w/recordings/.other.m4a.hollow-part").is_none());
    let r = rig.cycle();
    assert!(r.errors.is_empty(), "{r:?}");
    assert_eq!(read(&rig.here, rel2).unwrap(), body);
}

#[test]
fn an_unreachable_peer_is_skipped_quietly() {
    let mut rig = Rig::new();
    put(&rig.here, "workspaces/w/a.png", b"x", 1_700_000_000);
    let mut peer = ShellPeer {
        id: "fake-peer".into(),
        prefix: vec!["/bin/sh".into(), "-c".into(), "exit 255".into()],
        root: rig.there.to_str().unwrap().into(),
        timeout: STEP_TIMEOUT,
    };
    let r = rig.cycle_with(&mut peer);
    assert!(r.offline && r.errors.is_empty(), "{r:?}");
    assert!(read(&rig.there, "workspaces/w/a.png").is_none());
    assert_eq!(r.text(), "The other Mac didn't answer; skipped.");
}

#[test]
fn the_peer_refuses_escapes_and_symlinked_folders() {
    let rig = Rig::new();
    let outside = rig._tmp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, rig.there.join("linked")).unwrap();
    let mut peer = rig.peer();
    let entry = Entry {
        size: 1,
        mtime: 1,
        hash: Some(hash_reader(&mut &b"x"[..]).unwrap()),
    };
    for bad in [
        "../escape.png",
        "linked/x.png",
        "/etc/x.png",
        "a/.git/x.png",
        "",
    ] {
        let res = peer.receive(bad, 0, &entry, None, &mut &b"x"[..]);
        assert!(matches!(res, Err(PeerError::Failed(_))), "{bad}: {res:?}");
    }
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    assert!(!rig._tmp.path().join("escape.png").exists());
    let mut sink = Vec::new();
    assert!(peer.send("../there's vault/x", 0, &mut sink).is_err());
    // A symlinked file is not listed or sent.
    fs::write(outside.join("secret.png"), b"s").unwrap();
    std::os::unix::fs::symlink(outside.join("secret.png"), rig.there.join("ln.png")).unwrap();
    let (_, l) = peer.list().unwrap();
    assert!(l.is_empty(), "{l:?}");
    assert!(peer.send("ln.png", 0, &mut sink).is_err());
    assert!(sink.is_empty());
}

#[test]
fn this_side_refuses_symlinked_folders_too() {
    let mut rig = Rig::new();
    let outside = rig._tmp.path().join("outside-here");
    fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, rig.here.join("workspaces")).unwrap();
    put(&rig.there, "workspaces/w/a.png", b"x", 1_700_000_000);
    let r = rig.cycle();
    assert_eq!(r.errors.len(), 1, "{r:?}");
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
}

#[test]
fn copy_names_are_taken_in_any_case() {
    let local = listing(&[("x.png", e(1, 9, Some(&h('a'))))]);
    let peer = listing(&[
        ("x.png", e(1, 5, Some(&h('b')))),
        ("x (FROM mini).png", e(1, 5, Some(&h('c')))),
    ]);
    let reserved: BTreeSet<String> = ["x (From Laptop).png".to_string()].into();
    let got = plan(
        &local,
        &peer,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &reserved,
        ("laptop", "mini"),
    );
    assert!(
        got.contains(&Action::KeepBoth {
            path: "x.png".into(),
            here: "x (from mini 2).png".into(),
            there: "x (from laptop 2).png".into(),
        }),
        "{got:?}"
    );
}

#[test]
fn a_trailing_slash_in_the_remote_vault_changes_nothing() {
    assert_eq!(normalize_remote_vault(" Hollow/Vault// "), "Hollow/Vault");
    assert_eq!(normalize_remote_vault("///"), "/");
    assert!(!valid_remote_vault(&normalize_remote_vault("///")));
    for slashes in ["/", "//"] {
        let mut rig = Rig::new();
        put(&rig.here, "workspaces/w/a.png", b"here", 1_700_000_000);
        put(&rig.there, "workspaces/w/b.png", b"there", 1_700_000_000);
        let mut peer = rig.peer();
        peer.root.push_str(slashes);
        let r = rig.cycle_with(&mut peer);
        assert!(r.errors.is_empty(), "{r:?}");
        assert_eq!(read(&rig.there, "workspaces/w/a.png").unwrap(), b"here");
        assert_eq!(read(&rig.here, "workspaces/w/b.png").unwrap(), b"there");
        assert!(
            rig.record.there.contains_key("workspaces/w/b.png"),
            "{:?}",
            rig.record.there
        );
        let r = rig.cycle_with(&mut peer);
        assert!(r.here.is_empty() && r.there.is_empty(), "{r:?}");
    }
}

#[test]
fn hostile_file_names_round_trip_both_ways() {
    let mut rig = Rig::new();
    let names = [
        "workspaces/w/new\nline.png",
        "workspaces/w/tab\there.png",
        "workspaces/w/$(touch pwned).png",
        "workspaces/w/-rf.png",
        "workspaces/w/back`tick`.png",
        "workspaces/w/quote's \"both\".png",
    ];
    for (i, n) in names.iter().enumerate() {
        put(&rig.here, n, format!("here {i}").as_bytes(), 1_700_000_000);
        let there = n.replace(".png", " there.png");
        put(
            &rig.there,
            &there,
            format!("there {i}").as_bytes(),
            1_700_000_000,
        );
    }
    let r = rig.cycle();
    assert!(r.errors.is_empty(), "{r:?}");
    assert_eq!(r.here.len(), names.len());
    assert_eq!(r.there.len(), names.len());
    for (i, n) in names.iter().enumerate() {
        assert_eq!(
            read(&rig.there, n).unwrap(),
            format!("here {i}").as_bytes(),
            "{n}"
        );
        let there = n.replace(".png", " there.png");
        assert_eq!(
            read(&rig.here, &there).unwrap(),
            format!("there {i}").as_bytes(),
            "{n}"
        );
    }
    assert!(!rig.there.join("pwned").exists() && !Path::new("pwned").exists());
    assert!(!rig.here.join("pwned").exists());
    let r = rig.cycle();
    assert!(
        r.here.is_empty() && r.there.is_empty() && r.errors.is_empty(),
        "{r:?}"
    );
}

/// A peer whose pushes to a name containing `fail` fail `left` times.
struct Flaky {
    inner: ShellPeer,
    fail: &'static str,
    left: usize,
}

impl Peer for Flaky {
    fn id(&self) -> String {
        self.inner.id()
    }
    fn list(&mut self) -> Result<(String, Listing), PeerError> {
        self.inner.list()
    }
    fn hashes(&mut self, paths: &[String]) -> Result<BTreeMap<String, String>, PeerError> {
        self.inner.hashes(paths)
    }
    fn partial(&mut self, rel: &str) -> Result<u64, PeerError> {
        self.inner.partial(rel)
    }
    fn receive(
        &mut self,
        rel: &str,
        offset: u64,
        entry: &Entry,
        expect: Option<&Entry>,
        data: &mut (dyn Read + Send),
    ) -> Result<(), PeerError> {
        if rel.contains(self.fail) && self.left > 0 {
            self.left -= 1;
            return Err(PeerError::Failed("planted failure".into()));
        }
        self.inner.receive(rel, offset, entry, expect, data)
    }
    fn send(&mut self, rel: &str, offset: u64, out: &mut dyn Write) -> Result<(), PeerError> {
        self.inner.send(rel, offset, out)
    }
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().into_string().unwrap())
        .filter(|n| !n.starts_with('.'))
        .collect();
    v.sort();
    v
}

#[test]
fn a_failed_push_after_keep_both_retries_without_new_copies() {
    let mut rig = Rig::new();
    let rel = "workspaces/w/shot.png";
    put(&rig.here, rel, b"laptop", 1_700_000_200);
    put(&rig.there, rel, b"mini!!", 1_700_000_100);
    let mut peer = Flaky {
        inner: rig.peer(),
        fail: "(from laptop)",
        left: 2,
    };
    let r = rig.cycle_with(&mut peer);
    assert_eq!(r.kept_both, vec![rel]);
    assert_eq!(r.errors.len(), 1, "{r:?}");
    let r = rig.cycle_with(&mut peer);
    assert!(r.kept_both.is_empty(), "{r:?}");
    assert_eq!(r.errors.len(), 1, "{r:?}");
    let r = rig.cycle_with(&mut peer);
    assert!(r.kept_both.is_empty() && r.errors.is_empty(), "{r:?}");
    let r = rig.cycle_with(&mut peer);
    assert!(r.errors.is_empty(), "{r:?}");
    let peer_name = rig.record.peer_name.clone().unwrap();
    let want = {
        let mut v = vec![
            "shot (from laptop).png".to_string(),
            format!("shot (from {peer_name}).png"),
            "shot.png".to_string(),
        ];
        v.sort();
        v
    };
    let w = Path::new("workspaces/w");
    assert_eq!(names_in(&rig.here.join(w)), want);
    assert_eq!(names_in(&rig.there.join(w)), want);
    assert_eq!(
        read(&rig.there, "workspaces/w/shot (from laptop).png").unwrap(),
        b"laptop"
    );
    assert!(rig.record.pending.is_empty());
    let r = rig.cycle();
    assert_eq!(r, Report::default());
}

#[test]
fn a_replace_rehashes_and_keeps_both_when_the_cache_is_stale() {
    let mut rig = Rig::new();
    let rel = "workspaces/w/report.pdf";
    put(&rig.here, rel, b"v1", 1_700_000_000);
    rig.cycle();
    // The peer's file changes with the same size and mtime (the cache
    // would still say v1); this Mac writes a newer one.
    put(&rig.there, rel, b"X1", 1_700_000_000);
    put(&rig.here, rel, b"v2 newer", 1_700_000_500);
    let r = rig.cycle();
    assert_eq!(r.kept_both, vec![rel], "{r:?}");
    assert_eq!(read(&rig.there, rel).unwrap(), b"X1");
    assert_eq!(read(&rig.here, rel).unwrap(), b"v2 newer");
}

#[test]
fn the_peer_rehashes_the_target_before_a_replace() {
    let rig = Rig::new();
    let rel = "w/a.png";
    put(&rig.there, rel, b"old", 1_700_000_000);
    let mut peer = rig.peer();
    let entry = Entry {
        size: 3,
        mtime: 1_700_000_100,
        hash: Some(hash_reader(&mut &b"new"[..]).unwrap()),
    };
    let wrong = Entry {
        size: 3,
        mtime: 1_700_000_000,
        hash: Some(h('0')),
    };
    let res = peer.receive(rel, 0, &entry, Some(&wrong), &mut &b"new"[..]);
    assert!(matches!(res, Err(PeerError::Failed(_))), "{res:?}");
    assert_eq!(read(&rig.there, rel).unwrap(), b"old");
    assert!(read(&rig.there, "w/.a.png.hollow-part").is_none());
    let right = Entry {
        hash: Some(hash_reader(&mut &b"old"[..]).unwrap()),
        ..wrong
    };
    peer.receive(rel, 0, &entry, Some(&right), &mut &b"new"[..])
        .unwrap();
    assert_eq!(read(&rig.there, rel).unwrap(), b"new");
}

#[test]
fn a_stuck_step_is_killed_by_its_timeout() {
    let rig = Rig::new();
    let mut peer = ShellPeer {
        id: "stuck".into(),
        prefix: vec!["/bin/sh".into(), "-c".into(), "sleep 30; :".into()],
        root: rig.there.to_str().unwrap().into(),
        timeout: Duration::from_secs(1),
    };
    let started = std::time::Instant::now();
    let res = peer.list();
    assert!(matches!(res, Err(PeerError::Offline(_))), "{res:?}");
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn the_byte_cap_leaves_the_rest_for_the_next_cycle() {
    let mut rig = Rig::new();
    for n in ["a", "b", "c"] {
        put(&rig.here, &format!("w/{n}.png"), &[1u8; 100], 1_700_000_000);
    }
    let mut peer = rig.peer();
    let r = run_cycle(&rig.here, &mut rig.record, &mut peer, "laptop", 150);
    assert_eq!((r.there.len(), r.deferred), (1, 2), "{r:?}");
    let r = run_cycle(&rig.here, &mut rig.record, &mut peer, "laptop", 150);
    assert_eq!((r.there.len(), r.deferred), (1, 1), "{r:?}");
    let r = run_cycle(&rig.here, &mut rig.record, &mut peer, "laptop", 150);
    assert_eq!((r.there.len(), r.deferred), (1, 0), "{r:?}");
    assert_eq!(
        names_in(&rig.there.join("w")),
        vec!["a.png", "b.png", "c.png"]
    );
}

// ── On demand (N15) ───────────────────────────────────────────────────

#[test]
fn fetch_brings_one_file_and_nothing_else() {
    let rig = Rig::new();
    put(
        &rig.there,
        "workspaces/w/outputs/a.png",
        b"png bytes",
        1_700_000_000,
    );
    put(
        &rig.there,
        "workspaces/w/outputs/b.pdf",
        b"pdf",
        1_700_000_001,
    );
    let mut peer = rig.peer();
    let got = fetch_one(&rig.here, &mut peer, "workspaces/w/outputs/a.png").unwrap();
    assert_eq!(got.state, "fetched");
    assert_eq!((got.bytes, got.resumed_from), (9, 0));
    assert!(got.from.is_some());
    assert_eq!(
        read(&rig.here, "workspaces/w/outputs/a.png").unwrap(),
        b"png bytes"
    );
    assert_eq!(
        mtime(&rig.here, "workspaces/w/outputs/a.png"),
        1_700_000_000
    );
    // Only what was asked; no part file left; the peer untouched.
    assert_eq!(
        names_in(&rig.here.join("workspaces/w/outputs")),
        vec!["a.png"]
    );
    assert!(!rig
        .here
        .join("workspaces/w/outputs/.a.png.hollow-part")
        .exists());
    assert_eq!(
        read(&rig.there, "workspaces/w/outputs/a.png").unwrap(),
        b"png bytes"
    );
    // Listing shows what is still missing here.
    let list = peer_missing(&rig.here, &mut peer, "workspaces/", 10).unwrap();
    let paths: Vec<_> = list.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, vec!["workspaces/w/outputs/b.pdf"]);
    assert_eq!(list.total, 1);
    assert!(peer_missing(&rig.here, &mut peer, "other/", 10)
        .unwrap()
        .files
        .is_empty());
    assert!(peer_missing(&rig.here, &mut peer, "../x", 10).is_err());
}

#[test]
fn fetch_never_overwrites_a_file_here() {
    let rig = Rig::new();
    put(&rig.here, "workspaces/w/a.png", b"mine", 1_700_000_000);
    put(
        &rig.there,
        "workspaces/w/a.png",
        b"theirs, newer",
        1_800_000_000,
    );
    // A peer that fails every step proves nothing was asked of it.
    let mut dead = ShellPeer {
        id: "fake-peer".into(),
        prefix: vec!["/bin/sh".into(), "-c".into(), "exit 3".into()],
        root: rig.there.to_str().unwrap().into(),
        timeout: STEP_TIMEOUT,
    };
    let got = fetch_one(&rig.here, &mut dead, "workspaces/w/a.png").unwrap();
    assert_eq!(got.state, "alreadyHere");
    assert_eq!(read(&rig.here, "workspaces/w/a.png").unwrap(), b"mine");
    // A file landing here between the check and the link is not clobbered.
    put(&rig.there, "workspaces/w/b.png", b"theirs", 1_800_000_000);
    let mut peer = rig.peer();
    let (_, e) = peer.stat("workspaces/w/b.png").unwrap();
    let mut e = e.unwrap();
    e.hash = peer
        .hashes(&["workspaces/w/b.png".into()])
        .unwrap()
        .remove("workspaces/w/b.png");
    put(&rig.here, "workspaces/w/b.png", b"raced", 1_700_000_000);
    assert!(pull(
        &rig.here,
        &mut peer,
        "workspaces/w/b.png",
        "workspaces/w/b.png",
        &e,
        None
    )
    .is_err());
    assert_eq!(read(&rig.here, "workspaces/w/b.png").unwrap(), b"raced");
}

#[test]
fn fetch_refuses_text_private_and_escaping_paths_before_asking() {
    let rig = Rig::new();
    put(&rig.there, "workspaces/w/notes.md", b"text", 1_700_000_000);
    let mut dead = ShellPeer {
        id: "fake-peer".into(),
        prefix: vec!["/bin/sh".into(), "-c".into(), "exit 3".into()],
        root: rig.there.to_str().unwrap().into(),
        timeout: STEP_TIMEOUT,
    };
    for bad in [
        "workspaces/w/notes.md",
        "workspaces/w/data.json",
        "me/photo.png",
        "memory/x.png",
        "workspaces/.git/x.png",
        "../escape.png",
        "/etc/x.png",
        "",
    ] {
        let err = fetch_one(&rig.here, &mut dead, bad).unwrap_err();
        assert!(
            err.contains("not a vault path")
                || err.contains("only files the vault's git keeps out"),
            "{bad}: {err}"
        );
    }
    assert!(!rig.here.join("workspaces").exists());
    // A small file the vault's git carries is not fetchable through the peer.
    put(&rig.there, "workspaces/w/page.html", b"<p>", 1_700_000_000);
    let mut peer = rig.peer();
    let err = fetch_one(&rig.here, &mut peer, "workspaces/w/page.html").unwrap_err();
    assert!(err.contains("not on"), "{err}");
    assert!(!rig.here.join("workspaces").exists());
}

#[test]
fn fetch_of_a_missing_or_linked_file_fails_and_writes_nothing() {
    let rig = Rig::new();
    let outside = rig._tmp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.png"), b"s").unwrap();
    std::os::unix::fs::symlink(outside.join("secret.png"), rig.there.join("ln.png")).unwrap();
    std::os::unix::fs::symlink(&outside, rig.there.join("linked")).unwrap();
    let mut peer = rig.peer();
    for missing in ["workspaces/w/none.png", "ln.png", "linked/secret.png"] {
        let err = fetch_one(&rig.here, &mut peer, missing).unwrap_err();
        assert!(err.contains("not on"), "{missing}: {err}");
    }
    assert_eq!(fs::read_dir(&rig.here).unwrap().count(), 0);
}

#[test]
fn an_interrupted_fetch_resumes_from_its_part() {
    let rig = Rig::new();
    let body: Vec<u8> = (0..5000u32).flat_map(|i| i.to_le_bytes()).collect();
    put(&rig.there, "captures/big.mov", &body, 1_700_000_000);
    fs::create_dir_all(rig.here.join("captures")).unwrap();
    fs::write(
        rig.here.join("captures/.big.mov.hollow-part"),
        &body[..7000],
    )
    .unwrap();
    let mut peer = rig.peer();
    let got = fetch_one(&rig.here, &mut peer, "captures/big.mov").unwrap();
    assert_eq!((got.state.as_str(), got.resumed_from), ("fetched", 7000));
    assert_eq!(read(&rig.here, "captures/big.mov").unwrap(), body);
    assert!(!rig.here.join("captures/.big.mov.hollow-part").exists());
}

#[test]
fn an_unreachable_peer_fails_the_fetch_plainly() {
    let rig = Rig::new();
    let mut peer = ShellPeer {
        id: "fake-peer".into(),
        prefix: vec!["/bin/sh".into(), "-c".into(), "exit 255".into()],
        root: rig.there.to_str().unwrap().into(),
        timeout: STEP_TIMEOUT,
    };
    let err = fetch_one(&rig.here, &mut peer, "captures/a.png").unwrap_err();
    assert!(err.contains("did not answer"), "{err}");
    assert_eq!(fs::read_dir(&rig.here).unwrap().count(), 0);
}

#[test]
fn a_part_linked_elsewhere_starts_over_and_leaves_that_file_alone() {
    let rig = Rig::new();
    let body = b"the full recording".to_vec();
    put(&rig.there, "captures/big.mov", &body, 1_700_000_000);
    put(&rig.here, "captures/moved.mov", b"the full", 1_600_000_000);
    fs::hard_link(
        rig.here.join("captures/moved.mov"),
        rig.here.join("captures/.big.mov.hollow-part"),
    )
    .unwrap();
    let mut peer = rig.peer();
    let got = fetch_one(&rig.here, &mut peer, "captures/big.mov").unwrap();
    assert_eq!(got.resumed_from, 0);
    assert_eq!(read(&rig.here, "captures/big.mov").unwrap(), body);
    assert_eq!(read(&rig.here, "captures/moved.mov").unwrap(), b"the full");
}
