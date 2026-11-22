//! Owner-only runners (remote runners 2). A stand-in `ssh` answers `-G`
//! with two IdentityFiles in the rig (an agent-style `.pub` first, then a
//! plain key), and otherwise runs the command with `/bin/sh` in a fake
//! home, as the runner account would.

use super::*;
use serde_json::json;
use std::os::unix::fs::PermissionsExt;

const FAKE_SSH: &str = r#"#!/bin/sh
if [ "$1" = -G ]; then echo "user someone"; echo "identityfile $FAKE_OTHER.pub"; echo "identityfile $FAKE_KEY"; exit 0; fi
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o | -p | -i | -l) shift 2 ;;
    -*) shift ;;
    *) break ;;
  esac
done
host="$1"
shift
case "$host" in
  down) echo "ssh: connect to host down port 22: Operation timed out" >&2; exit 255 ;;
esac
cd "$FAKE_HOME" || exit 255
exec env -i HOME="$FAKE_HOME" PATH=/usr/bin:/bin:/usr/sbin:/sbin /bin/sh -c "$*"
"#;

struct Rig {
    dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Rig {
        let dir = tempfile::Builder::new()
            .prefix("rown")
            .tempdir_in("/tmp")
            .unwrap();
        let rig = Rig { dir };
        std::fs::create_dir_all(rig.home()).unwrap();
        let key = rig.dir.path().join("id_test");
        let other = rig.dir.path().join("id_other");
        for (path, comment) in [(&key, "test"), (&other, "other")] {
            let made = std::process::Command::new("ssh-keygen")
                .args(["-q", "-t", "ed25519", "-N", "", "-C", comment, "-f"])
                .arg(path)
                .status()
                .unwrap();
            assert!(made.success());
        }
        let ssh = rig.dir.path().join("fake-ssh");
        std::fs::write(
            &ssh,
            FAKE_SSH
                .replace("$FAKE_HOME", &rig.home().to_string_lossy())
                .replace("$FAKE_KEY", &key.to_string_lossy())
                .replace("$FAKE_OTHER", &other.to_string_lossy()),
        )
        .unwrap();
        std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
        rig
    }
    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }
    fn ssh(&self) -> String {
        self.dir
            .path()
            .join("fake-ssh")
            .to_string_lossy()
            .into_owned()
    }
    fn fingerprint(&self) -> String {
        pub_fingerprint(&self.dir.path().join("id_test.pub")).unwrap()
    }
    fn other_fingerprint(&self) -> String {
        pub_fingerprint(&self.dir.path().join("id_other.pub")).unwrap()
    }
}

const A: &str = "SHA256:bJV0Wf5YtfK6coilA2Ri3lxqNbTvuhT80c5/dHbqa+E";
const B: &str = "SHA256:DKM0J/Q+WsK8vW+BYCsU0Hihe4DP8Fm1fCsPiToRM+8";
const C: &str = "SHA256:7EIVMXjin9xAAsTVWCq0x0ctoQ4f/XwwmP38rpHImaM";
const ME: &str = "0123456789abcdef0123456789abcdef";
const THEM: &str = "fedcba9876543210fedcba9876543210";

fn mine(key: &str) -> Mine {
    Mine {
        install_id: Some(ME.into()),
        keys: vec![key.into()],
    }
}

/// A probe answer: the owner record, the keys listed, and the counts the
/// probe sends (unique keys, key lines, fingerprints read).
fn probe(owner: Value, keys: &[&str], lines: u64) -> Value {
    let mut unique: Vec<&str> = Vec::new();
    for k in keys {
        if !unique.contains(k) {
            unique.push(k);
        }
    }
    json!({
        "owner": owner,
        "keys": unique,
        "keyCount": unique.len(),
        "keyLines": lines,
        "keysRead": keys.len(),
    })
}

#[test]
fn install_id_is_made_once_and_kept() {
    let first = install_id().unwrap();
    assert!(valid_install_id(&first), "{first}");
    assert_eq!(install_id().unwrap(), first);
}

#[test]
fn fingerprints_and_ids_are_checked_for_shape() {
    assert!(valid_fingerprint(A));
    assert!(!valid_fingerprint("SHA256:x"));
    assert!(!valid_fingerprint("MD5:aa:bb"));
    assert!(!valid_fingerprint(&format!("{A} ")));
    assert!(valid_install_id(ME));
    assert!(!valid_install_id("0123456789ABCDEF0123456789ABCDEF"));
    assert!(!valid_install_id("short"));
}

/// Only this Mac's key, recorded by this Hollow: nothing to say.
#[test]
fn an_owned_runner_with_only_its_key_is_quiet() {
    let o = judge(
        &probe(
            json!({ "installId": ME, "key": A, "at": "2026-11-03T19:00:00Z" }),
            &[A],
            1,
        ),
        &mine(A),
    );
    assert_eq!(o.state, "you");
    assert_eq!(o.recorded_at.as_deref(), Some("2026-11-03T19:00:00Z"));
    assert_eq!((o.keys, o.other_keys), (Some(1), Some(0)));
    assert!(!o.key_changed);
    assert!(o.warnings.is_empty(), "{:?}", o.warnings);
}

/// Another key in authorized_keys is the first warning; a key listed in
/// both files counts once.
#[test]
fn another_key_is_warned_about_first() {
    let o = judge(
        &probe(json!({ "installId": ME, "key": A }), &[A, B, B], 3),
        &mine(A),
    );
    assert_eq!((o.keys, o.other_keys), (Some(2), Some(1)));
    assert!(
        o.warnings[0].starts_with("1 other SSH key can log in"),
        "{:?}",
        o.warnings
    );
    assert_eq!(o.warnings.len(), 1, "{:?}", o.warnings);
    let two = judge(&probe(Value::Null, &[A, B, C], 3), &mine(A));
    assert_eq!(two.state, "none");
    assert_eq!(two.other_keys, Some(2));
    assert!(two.warnings[0].starts_with("2 other SSH keys can log in"));
}

/// Every key this Mac may offer counts as its own (review: ssh may use
/// the second IdentityFile, not the first).
#[test]
fn every_key_this_mac_offers_is_its_own() {
    let both = Mine {
        install_id: Some(ME.into()),
        keys: vec![C.into(), A.into()],
    };
    let o = judge(&probe(json!({ "installId": ME, "key": A }), &[A], 1), &both);
    assert_eq!((o.state, o.other_keys), ("you", Some(0)));
    assert!(o.warnings.is_empty() && !o.key_changed, "{:?}", o.warnings);
}

/// An owner record another Hollow wrote: its key counts as allowed (it is
/// the owner's), and the record itself is the warning.
#[test]
fn another_hollows_record_is_named() {
    let o = judge(
        &probe(
            json!({ "installId": THEM, "key": B, "at": "2026-10-01T08:00:00Z" }),
            &[A, B],
            2,
        ),
        &mine(A),
    );
    assert_eq!(o.state, "other");
    assert_eq!(o.other_keys, Some(0));
    assert_eq!(o.warnings.len(), 1);
    assert!(
        o.warnings[0].contains("Another Hollow recorded itself")
            && o.warnings[0].contains("(2026-10-01)")
    );
}

/// Another install id with this Mac's own key is another Hollow on this
/// Mac (a candidate or review build keeps its own app data): still you.
#[test]
fn another_instance_on_this_mac_is_still_you() {
    let o = judge(
        &probe(json!({ "installId": THEM, "key": A }), &[A], 1),
        &mine(A),
    );
    assert_eq!(o.state, "you");
    assert!(o.warnings.is_empty(), "{:?}", o.warnings);
    let unknown_id = Mine {
        install_id: None,
        keys: vec![A.into()],
    };
    assert_eq!(
        judge(
            &probe(json!({ "installId": THEM, "key": A }), &[A], 1),
            &unknown_id
        )
        .state,
        "you"
    );
}

#[test]
fn a_changed_key_and_unreadable_lines_are_named() {
    let o = judge(
        &probe(json!({ "installId": ME, "key": B }), &[A], 3),
        &mine(A),
    );
    assert_eq!(o.state, "you");
    assert!(o.key_changed);
    assert!(
        o.warnings.iter().any(|w| w.contains("no longer offers")),
        "{:?}",
        o.warnings
    );
    assert!(o
        .warnings
        .iter()
        .any(|w| w.starts_with("2 lines of its authorized_keys")));
}

/// More than 50 keys: the probe lists 50 and counts all; the unlisted
/// ones count as other keys, and no line reads as unreadable.
#[test]
fn keys_past_the_listed_fifty_count_as_other() {
    let listed: Vec<String> = (0..50).map(|i| format!("SHA256:{:0>43}", i)).collect();
    let mut keys: Vec<&str> = vec![A];
    keys.extend(listed.iter().take(49).map(String::as_str));
    let answer = json!({
        "owner": { "installId": ME, "key": A },
        "keys": keys, "keyCount": 60, "keyLines": 60, "keysRead": 60,
    });
    let o = judge(&answer, &mine(A));
    assert_eq!((o.keys, o.other_keys), (Some(60), Some(59)));
    assert!(
        !o.warnings.iter().any(|w| w.contains("couldn't be read")),
        "{:?}",
        o.warnings
    );
}

/// Fields of the wrong shape count as absent; with neither the owner's
/// key nor this Mac's known, other keys can't be counted; no key read at
/// all is said, not counted as zero.
#[test]
fn a_malformed_answer_judges_leniently() {
    let o = judge(
        &json!({ "owner": { "installId": 7, "key": ["x"], "at": {} }, "keys": [A, 3, "nope"], "keyCount": "two" }),
        &Mine::default(),
    );
    assert_eq!(o.state, "none");
    assert_eq!(o.recorded_at, None);
    assert_eq!((o.keys, o.other_keys), (Some(1), None));
    assert!(o.warnings.is_empty());
    let unread = judge(&json!({}), &mine(A));
    assert_eq!((unread.keys, unread.other_keys), (None, None));
    assert!(unread.warnings.is_empty());
    let empty = judge(&probe(Value::Null, &[], 0), &mine(A));
    assert_eq!((empty.keys, empty.other_keys), (None, None));
    assert!(
        empty.warnings[0].starts_with("No key could be read"),
        "{:?}",
        empty.warnings
    );
}

#[test]
fn the_keys_this_mac_offers_come_from_ssh_g() {
    let rig = Rig::new();
    let fps = key_fingerprints(&rig.ssh(), "fake");
    assert_eq!(fps, vec![rig.other_fingerprint(), rig.fingerprint()]);
    assert!(key_fingerprints(&rig.ssh(), "-oProxyCommand=x").is_empty());
}

/// Recording writes owner.json there (0600) with this Hollow's id and the
/// key the runner account lets in (the second this Mac offers here), and
/// the status probe then reads this Hollow as the owner.
#[test]
fn recording_the_owner_writes_one_private_file() {
    let rig = Rig::new();
    let first = record_owner(&rig.ssh(), "fake").unwrap();
    assert_eq!(
        first.key,
        rig.other_fingerprint(),
        "none authorized: ssh's first"
    );
    let public = std::fs::read_to_string(rig.dir.path().join("id_test.pub")).unwrap();
    std::fs::create_dir_all(rig.home().join(".ssh")).unwrap();
    std::fs::write(rig.home().join(".ssh/authorized_keys"), &public).unwrap();
    let record = record_owner(&rig.ssh(), "fake").unwrap();
    assert_eq!(record.install_id, install_id().unwrap());
    assert_eq!(record.key, rig.fingerprint());
    let path = rig.home().join(OWNER_FILE);
    let text = std::fs::read_to_string(&path).unwrap();
    let saved: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(saved["installId"], json!(record.install_id));
    assert_eq!(saved["key"], json!(record.key));
    assert_eq!(saved["v"], json!(1));
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, vec!["owner.json".to_string()]);
    let o = judge(
        &probe(saved.clone(), &[record.key.as_str()], 1),
        &Mine::for_host(&rig.ssh(), "fake"),
    );
    assert_eq!(o.state, "you");
    assert!(o.warnings.is_empty(), "{:?}", o.warnings);
}

#[test]
fn recording_says_why_it_failed() {
    let rig = Rig::new();
    let down = record_owner(&rig.ssh(), "down").unwrap_err();
    assert!(
        down.starts_with("down didn't answer: ssh: connect to host down"),
        "{down}"
    );
    let bad = record_owner(&rig.ssh(), "-oProxyCommand=x").unwrap_err();
    assert!(bad.contains("is not an SSH host name"), "{bad}");
    std::fs::write(rig.home().join("hollow-remote"), "a file, not a folder").unwrap();
    let blocked = record_owner(&rig.ssh(), "fake").unwrap_err();
    assert!(
        blocked.starts_with("fake couldn't write ~/hollow-remote/owner.json"),
        "{blocked}"
    );
}

#[test]
fn naming_and_turning_off_the_runner_writes_slot_remote() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hollow").join("slot-remote");
    set_runner_at(&path, Some(" my-runner "), None).unwrap();
    let on = std::fs::read_to_string(&path).unwrap();
    assert!(on.starts_with("# Hollow's remote runner"), "{on}");
    let first = on
        .lines()
        .map(|l| l.split('#').next().unwrap().trim())
        .find(|l| !l.is_empty());
    assert_eq!(first, Some("my-runner"));
    set_runner_at(&path, None, Some("my-runner".into())).unwrap();
    let off = std::fs::read_to_string(&path).unwrap();
    assert!(off.contains("\noff\n# was my-runner\n"), "{off}");
    for bad in ["-oProxyCommand=x", "two words", "off", ""] {
        assert!(set_runner_at(&path, Some(bad), None).is_err(), "{bad}");
    }
}
