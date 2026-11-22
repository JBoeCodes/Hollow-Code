//! Tests for `mac_context.rs` (F3): the offer's timing per Mac pair, the
//! record, the card's rows and Connect's clone and home file steps.

use super::*;
use chrono::TimeZone;
use std::process::Command;

fn t(day: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 1, 9, 0, 0).unwrap() + ChronoDuration::days(day)
}

fn record(offered: Option<i64>, answer: Option<Answer>, answered: Option<i64>) -> PairRecord {
    PairRecord {
        offered_at: offered.map(t),
        answer,
        answered_at: answered.map(t),
        extra: Map::new(),
    }
}

fn state_with(pair: &str, r: PairRecord) -> OfferState {
    let mut s = OfferState::default();
    s.pairs.insert(pair.to_string(), r);
    s
}

#[test]
fn a_pair_is_the_same_pair_in_either_order() {
    assert_eq!(pair_key("m-aa", "m-bb"), pair_key("m-bb", "m-aa"));
    assert_eq!(pair_key("m-aa", "runner:mini"), "m-aa+runner:mini");
    assert_ne!(pair_key("m-aa", "m-bb"), pair_key("m-aa", "m-cc"));
    // The record written for A-B answers B-A.
    let s = state_with(
        &pair_key("m-aa", "m-bb"),
        record(Some(0), Some(Answer::Never), Some(0)),
    );
    assert!(!offer_due(&s, &pair_key("m-bb", "m-aa"), t(400)));
}

#[test]
fn a_new_pair_is_due() {
    assert!(offer_due(&OfferState::default(), "a+b", t(0)));
    // Another pair's answer doesn't cover this one.
    let s = state_with("a+c", record(Some(0), Some(Answer::Never), Some(0)));
    assert!(offer_due(&s, "a+b", t(0)));
}

#[test]
fn connected_and_never_are_final() {
    for answer in [Answer::Connected, Answer::Never] {
        let s = state_with("a+b", record(Some(0), Some(answer), Some(1)));
        for day in [1, 14, 15, 365] {
            assert!(!offer_due(&s, "a+b", t(day)), "{answer:?} day {day}");
            assert!(!card_open(&s, "a+b", t(day)), "{answer:?} day {day}");
        }
    }
}

#[test]
fn not_now_comes_back_after_fourteen_days_to_the_second() {
    let s = state_with("a+b", record(Some(0), Some(Answer::NotNow), Some(2)));
    assert!(!offer_due(&s, "a+b", t(2)));
    assert!(!offer_due(&s, "a+b", t(15)));
    assert!(!offer_due(&s, "a+b", t(16) - ChronoDuration::seconds(1)));
    assert!(offer_due(&s, "a+b", t(16)), "14 days after the answer");
    assert!(offer_due(&s, "a+b", t(40)));
    // Hidden in the meantime.
    assert!(!card_open(&s, "a+b", t(10)));
    assert!(card_open(&s, "a+b", t(16)));
}

#[test]
fn a_renewed_offer_counts_from_the_new_offer() {
    // Not now on day 2, offered again on day 16 and ignored.
    let s = state_with("a+b", record(Some(16), Some(Answer::NotNow), Some(2)));
    assert!(
        !offer_due(&s, "a+b", t(20)),
        "no second notification so soon"
    );
    assert!(card_open(&s, "a+b", t(20)), "the card waits for an answer");
    assert!(offer_due(&s, "a+b", t(30)));
}

#[test]
fn an_unanswered_offer_waits_on_the_card_and_renews_after_fourteen_days() {
    let s = state_with("a+b", record(Some(0), None, None));
    assert!(!offer_due(&s, "a+b", t(13)));
    assert!(card_open(&s, "a+b", t(13)));
    assert!(offer_due(&s, "a+b", t(14)));
}

#[test]
fn answers_parse_from_the_window() {
    assert_eq!(Answer::parse("not-now").unwrap(), Answer::NotNow);
    assert_eq!(Answer::parse("never").unwrap(), Answer::Never);
    assert!(Answer::parse("later").is_err());
}

#[test]
fn the_record_keeps_answers_and_unknown_fields_and_refuses_a_broken_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("offers.json");
    let offers = Offers::at(&path);
    offers.offered("a+b", t(0)).unwrap();
    offers.answer("a+b", Answer::NotNow, t(1)).unwrap();
    let s = offers.read();
    assert_eq!(s.version, VERSION);
    assert_eq!(s.pairs["a+b"].answer, Some(Answer::NotNow));
    assert_eq!(s.pairs["a+b"].offered_at, Some(t(0)));
    assert!(fs::read_to_string(&path).unwrap().contains("\"not-now\""));

    let mut raw: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    raw["later"] = Value::Bool(true);
    raw["pairs"]["a+b"]["note"] = Value::from("kept");
    fs::write(&path, raw.to_string()).unwrap();
    offers.answer("a+b", Answer::Never, t(2)).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("\"later\"") && text.contains("\"kept\""),
        "{text}"
    );

    fs::write(&path, "{not json").unwrap();
    assert!(offers.answer("a+b", Answer::Connected, t(3)).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "{not json");
    assert_eq!(offers.read(), OfferState::default());
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A repo with one commit holding `files`, to clone from by path.
fn source_repo(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    for (rel, text) in files {
        let p = dir.path().join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }
    git(dir.path(), &["add", "-A"]);
    git(
        dir.path(),
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ],
    );
    dir
}

fn roots_at(home: &Path) -> RootsAt {
    RootsAt {
        home: home.to_path_buf(),
        settings: home.join("app").join("context-roots.json"),
    }
}

fn here_in(at: &RootsAt, slug: Option<&str>) -> Here {
    Here {
        home: at.home.clone(),
        saved: roots::settings_at(&at.settings),
        slug: slug.map(String::from),
        ..Default::default()
    }
}

fn leftovers(parent: &Path) -> Vec<String> {
    fs::read_dir(parent)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.contains("hollow-clone"))
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn a_clone_goes_to_the_default_root_and_is_saved_then_kept() {
    let home = tempfile::tempdir().unwrap();
    let at = roots_at(home.path());
    let src = source_repo(&[("me/about.md", "hi\n")]);
    let url = src.path().to_string_lossy().into_owned();

    let first = clone_layer(&at, Layer::Personal, Some(&url)).await;
    assert_eq!(first.state, "done", "{first:?}");
    let saved = roots::settings_at(&at.settings);
    let root = PathBuf::from(saved.personal.clone().unwrap());
    assert!(root.ends_with("Hollow/Context/personal"), "{root:?}");
    assert_eq!(
        fs::read_to_string(root.join("me/about.md")).unwrap(),
        "hi\n"
    );
    assert!(leftovers(root.parent().unwrap()).is_empty());

    let again = clone_layer(&at, Layer::Personal, Some("/nowhere")).await;
    assert_eq!(again.state, "skipped", "{again:?}");
    assert!(again.text.contains("Already cloned"));
}

#[tokio::test]
async fn a_folder_with_other_files_is_never_cloned_over() {
    let home = tempfile::tempdir().unwrap();
    let at = roots_at(home.path());
    let target = home.path().join("Hollow/Context/org");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("notes.md"), "mine").unwrap();
    let src = source_repo(&[("INDEX.md", "x")]);
    let s = clone_layer(&at, Layer::Org, Some(&src.path().to_string_lossy())).await;
    assert_eq!(s.state, "refused", "{s:?}");
    assert_eq!(fs::read_to_string(target.join("notes.md")).unwrap(), "mine");
    assert!(!target.join(".git").exists());
    assert!(roots::settings_at(&at.settings).org.is_none());
}

#[tokio::test]
async fn a_failed_clone_leaves_nothing_and_saves_nothing() {
    let home = tempfile::tempdir().unwrap();
    let at = roots_at(home.path());
    let missing = home.path().join("no-such-repo");
    let s = clone_layer(&at, Layer::Org, Some(&missing.to_string_lossy())).await;
    assert_eq!(s.state, "failed", "{s:?}");
    assert!(s.text.contains("Nothing was cloned"), "{}", s.text);
    let parent = home.path().join("Hollow/Context");
    assert!(leftovers(&parent).is_empty(), "{:?}", leftovers(&parent));
    assert!(!parent.join("org").exists());
    assert!(roots::settings_at(&at.settings).org.is_none());
}

#[tokio::test]
async fn credentials_and_missing_addresses_are_refused_before_git_runs() {
    let home = tempfile::tempdir().unwrap();
    let at = roots_at(home.path());
    let s = clone_layer(
        &at,
        Layer::Personal,
        Some("https://tok3n@example.com/a.git"),
    )
    .await;
    assert_eq!(s.state, "refused");
    assert!(!s.text.contains("tok3n"), "{}", s.text);
    let s = clone_layer(&at, Layer::Personal, None).await;
    assert_eq!(s.state, "failed");
    assert!(s.text.contains("address"));
    assert!(!home.path().join("Hollow/Context").exists());
}

#[test]
fn clone_errors_name_access_without_a_secret() {
    let text = clone_failure(
        Some(128),
        "fatal: https://user:pw@h/a.git: Repository not found.",
        "git@github.com:me/a.git",
    );
    assert!(text.contains("sign in to git"), "{text}");
    let text = clone_failure(
        Some(128),
        "fatal: https://user:pw@h/a.git broke",
        "ssh://me:pw@h/a",
    );
    assert!(!text.contains("pw"), "{text}");
}

#[test]
fn the_home_file_is_written_once_and_never_replaced() {
    let root = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(root.path()).unwrap();
    let s = write_home_file(&root, "mini");
    assert_eq!(s.state, "done", "{s:?}");
    let file = root.join("home/mini.md");
    assert_eq!(fs::read_to_string(&file).unwrap(), home_starter("mini"));
    fs::write(&file, "edited").unwrap();
    let s = write_home_file(&root, "mini");
    assert_eq!(s.state, "skipped");
    assert_eq!(fs::read_to_string(&file).unwrap(), "edited");
    assert_eq!(
        fs::read_dir(root.join("home")).unwrap().count(),
        1,
        "no temp file left"
    );
}

#[test]
fn the_home_file_never_goes_through_a_link() {
    let root = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(root.path()).unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), root.join("home")).unwrap();
    let s = write_home_file(&root, "mini");
    assert_eq!(s.state, "refused", "{s:?}");
    assert_eq!(fs::read_dir(elsewhere.path()).unwrap().count(), 0);
    assert_eq!(write_home_file(&root, "Not A Slug").state, "refused");
}

#[tokio::test]
async fn connect_clones_names_this_mac_and_writes_its_home_file() {
    let home = tempfile::tempdir().unwrap();
    let at = roots_at(home.path());
    let src = source_repo(&[("me/about.md", "hi\n")]);
    let req = ConnectRequest {
        pair: Some("a+b".into()),
        layers: vec![
            "home".into(),
            "personal".into(),
            "vault".into(),
            "shell".into(),
        ],
        personal_url: Some(src.path().to_string_lossy().into_owned()),
        slug: Some("studio".into()),
        ..Default::default()
    };
    let mut named = None;
    let steps = connect_in(&at, &req, None, Some("suggested".into()), |s| {
        named = Some(s.to_string());
        Ok(Some(s.to_string()))
    })
    .await;
    assert_eq!(named.as_deref(), Some("studio"));
    let by = |l: &str| steps.iter().find(|s| s.layer == l).unwrap().state.clone();
    assert_eq!(by("shell"), "refused");
    assert_eq!(by("personal"), "done");
    assert_eq!(by("home"), "done", "{steps:?}");
    assert_eq!(by("vault"), "held");
    assert!(!all_connected(&steps), "an unknown layer is not connected");
    let root = PathBuf::from(roots::settings_at(&at.settings).personal.unwrap());
    assert!(root.join("home/studio.md").is_file());

    // Everything there now: the card has nothing missing.
    let rows = layer_rows(&here_in(&at, Some("studio")));
    let state = |l: &str| rows.iter().find(|r| r.layer == l).unwrap().state.clone();
    assert_eq!(state("personal"), "connected");
    assert_eq!(state("home"), "connected");
    assert_eq!(state("org"), "missing");
    assert_eq!(state("vault"), "held");
}

#[tokio::test]
async fn home_waits_for_the_personal_clone_and_a_name() {
    let home = tempfile::tempdir().unwrap();
    let at = roots_at(home.path());
    let req = ConnectRequest {
        layers: vec!["home".into()],
        ..Default::default()
    };
    let steps = connect_in(&at, &req, None, None, |_| panic!("no name to save")).await;
    assert_eq!(steps[0].state, "failed");
    assert!(steps[0].text.contains("name"), "{}", steps[0].text);
    let steps = connect_in(&at, &req, Some("mini".into()), None, |_| unreachable!()).await;
    assert_eq!(steps[0].state, "failed");
    assert!(
        steps[0].text.contains("personal clone"),
        "{}",
        steps[0].text
    );
    let req = ConnectRequest {
        layers: vec!["home".into()],
        slug: Some("Bad Name".into()),
        ..Default::default()
    };
    let steps = connect_in(&at, &req, None, None, |_| {
        panic!("an invalid name is never saved")
    })
    .await;
    assert_eq!(steps[0].state, "failed");
}

#[test]
fn the_card_and_the_notification_follow_the_pair_and_the_missing_layers() {
    let home = tempfile::tempdir().unwrap();
    let at = roots_at(home.path());
    let here = here_in(&at, None);
    let mini = OtherMac {
        id: "runner:mini".into(),
        name: "mini".into(),
        kind: "runner".into(),
    };
    let others = vec![mini.clone()];
    let empty = OfferState::default();

    let due = due_pairs(&here, "m-1", &others, &empty, t(0));
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].0, "m-1+runner:mini");
    let offer = offer_in(&here, "m-1", &others, &empty, t(0), false).unwrap();
    assert!(offer.open);
    assert_eq!(offer.pair.as_deref(), Some("m-1+runner:mini"));
    assert_eq!(offer.other_kind.as_deref(), Some("runner"));

    let never = state_with(
        "m-1+runner:mini",
        record(Some(0), Some(Answer::Never), Some(0)),
    );
    assert!(due_pairs(&here, "m-1", &others, &never, t(30)).is_empty());
    assert!(offer_in(&here, "m-1", &others, &never, t(30), false).is_none());
    // Settings still reaches it, without the offer's buttons.
    let later = offer_in(&here, "m-1", &others, &never, t(30), true).unwrap();
    assert!(!later.open);
    assert_eq!(later.layers.len(), 4);

    // No other Mac: nothing offered, the rows still there on request.
    assert!(offer_in(&here, "m-1", &[], &empty, t(0), false).is_none());
    assert!(offer_in(&here, "m-1", &[], &empty, t(0), true)
        .unwrap()
        .pair
        .is_none());
}

#[test]
fn the_check_stamps_a_due_pair_once_so_a_relaunch_never_repeats_it() {
    let home = tempfile::tempdir().unwrap();
    let at = roots_at(home.path());
    let here = here_in(&at, None);
    let others = vec![OtherMac {
        id: "m-2".into(),
        name: "studio".into(),
        kind: "peer".into(),
    }];
    let offers = Offers::at(&home.path().join("offers.json"));
    let first = stamp_due(&offers, &here, "m-1", &others, t(0)).unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(offers.read().pairs["m-1+m-2"].offered_at, Some(t(0)));
    assert!(stamp_due(&offers, &here, "m-1", &others, t(0))
        .unwrap()
        .is_empty());
    assert!(stamp_due(&offers, &here, "m-1", &others, t(13))
        .unwrap()
        .is_empty());
    assert_eq!(
        stamp_due(&offers, &here, "m-1", &others, t(14))
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn odd_addresses_are_refused_before_git_runs() {
    let home = tempfile::tempdir().unwrap();
    let at = roots_at(home.path());
    for url in [
        "-uhack",
        "--upload-pack=touch /tmp/x",
        "ssh://user:pw@example.com/a.git",
        "https://user:pw@example.com/a.git",
        "ext::sh -c touch",
    ] {
        let s = clone_layer(&at, Layer::Org, Some(url)).await;
        assert_eq!(s.state, "refused", "{url}: {s:?}");
        assert!(!s.text.contains("pw"), "{}", s.text);
    }
    assert!(!home.path().join("Hollow/Context").exists());
}

#[tokio::test]
async fn an_overlapping_root_is_refused_before_any_clone() {
    let home = tempfile::tempdir().unwrap();
    let at = roots_at(home.path());
    let inside = home.path().join("Hollow/Context/personal/org");
    fs::create_dir_all(&inside).unwrap();
    roots::set_root_at(
        &at.settings,
        &at.home,
        Layer::Org,
        Some(&inside.to_string_lossy()),
    )
    .unwrap();
    let src = source_repo(&[("me/a.md", "x")]);
    let s = clone_layer(&at, Layer::Personal, Some(&src.path().to_string_lossy())).await;
    assert_eq!(s.state, "refused", "{s:?}");
    assert!(s.text.contains("separate"), "{}", s.text);
    let parent = home.path().join("Hollow/Context");
    assert!(leftovers(&parent).is_empty());
    assert!(!parent.join("personal/.git").exists());
    assert!(roots::settings_at(&at.settings).personal.is_none());
}

#[test]
fn a_second_connect_is_refused_while_one_runs() {
    let first = begin_connect().unwrap();
    let again = begin_connect().err().unwrap();
    assert!(again.contains("already running"), "{again}");
    drop(first);
    drop(begin_connect().unwrap());
}

#[test]
fn connected_waits_for_every_row_not_only_the_ticked_ones() {
    let ok = vec![step("personal", "done", "")];
    let row = |layer: &str, state: &str| LayerRow {
        layer: layer.into(),
        state: state.into(),
        text: String::new(),
        target: None,
        url: None,
    };
    let all_here = vec![
        row("personal", "connected"),
        row("org", "connected"),
        row("home", "connected"),
        row("vault", "held"),
    ];
    let org_missing = vec![
        row("personal", "connected"),
        row("org", "missing"),
        row("home", "connected"),
        row("vault", "held"),
    ];
    assert!(connected_now(&ok, &all_here));
    assert!(!connected_now(&ok, &org_missing), "an unticked missing row");
    assert!(!connected_now(&[step("org", "failed", "")], &all_here));
    assert!(!connected_now(&ok, &[]), "rows that weren't read");
}
