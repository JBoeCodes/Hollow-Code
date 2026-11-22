//! Pure parts: the offer rule, the phase machine, the manifest, the config
//! and the token shape.

use super::config::{self, Config};
use super::manifest;
use super::settings::{Store, UpdateSettings};
use super::state::{
    apply, decide, valid_version, Channel, Decision, Event, UpdatePhase, UpdateRelease,
};
use super::token;
use reqwest::Url;

const FIXTURE: &str = include_str!("../../tests/fixtures/updates/manifest-nightly.json");

fn release(version: &str) -> UpdateRelease {
    UpdateRelease {
        version: version.into(),
        notes: "notes".into(),
        pub_date: None,
        channel: Channel::Nightly,
        commit: None,
        downgrade: false,
        local: None,
    }
}

fn host() -> Url {
    Url::parse("https://updates.example.com").unwrap()
}

// ── The offer rule ──────────────────────────────────────────────────────

#[test]
fn a_higher_version_is_offered_and_equal_or_lower_is_not() {
    assert_eq!(
        decide("0.6.10", "0.6.11", None, false),
        Decision::Offer { downgrade: false }
    );
    assert_eq!(
        decide("0.5.15", "0.6.1", None, false),
        Decision::Offer { downgrade: false }
    );
    assert_eq!(decide("0.6.10", "0.6.10", None, false), Decision::Current);
    assert_eq!(decide("0.6.10", "0.6.9", None, false), Decision::Current);
    // Numeric, not text: 0.6.100 is higher than 0.6.99.
    assert_eq!(
        decide("0.6.99", "0.6.100", None, false),
        Decision::Offer { downgrade: false }
    );
}

#[test]
fn the_skipped_version_is_never_offered_but_a_newer_one_is() {
    assert_eq!(
        decide("0.6.10", "0.6.11", Some("0.6.11"), false),
        Decision::Skipped("0.6.11".into())
    );
    assert_eq!(
        decide("0.6.10", "0.6.11", Some("0.6.11"), true),
        Decision::Skipped("0.6.11".into())
    );
    assert_eq!(
        decide("0.6.10", "0.6.12", Some("0.6.11"), false),
        Decision::Offer { downgrade: false }
    );
}

#[test]
fn a_lower_version_is_offered_only_while_a_channel_switch_allows_it() {
    assert_eq!(
        decide("0.6.20", "0.6.12", None, true),
        Decision::Offer { downgrade: true }
    );
    assert_eq!(decide("0.6.20", "0.6.12", None, false), Decision::Current);
    // The same version is never a "downgrade".
    assert_eq!(decide("0.6.20", "0.6.20", None, true), Decision::Current);
}

#[test]
fn versions_that_dont_compare_are_never_offered() {
    assert_eq!(decide("0.6.20", "0.6.x", None, true), Decision::Current);
    assert_eq!(decide("dev", "0.6.21", None, false), Decision::Current);
    assert!(valid_version("0.6.1234"));
    for bad in [
        "0",
        "0.6.",
        "0..6",
        "v0.6.1",
        "0.6.1-beta",
        "0.6.1/../x",
        "1.2.3.4.5",
        "",
    ] {
        assert!(!valid_version(bad), "{bad}");
    }
}

// ── The phase machine ───────────────────────────────────────────────────

#[test]
fn a_check_goes_checking_then_available_downloading_ready() {
    let p = UpdatePhase::Idle;
    let p = apply(&p, Event::CheckStarted);
    assert_eq!(p, UpdatePhase::Checking);
    let p = apply(&p, Event::Offered(release("0.6.2")));
    assert!(matches!(p, UpdatePhase::Available { .. }));
    let p = apply(&p, Event::Progress(Some(40)));
    assert!(matches!(
        p,
        UpdatePhase::Downloading {
            percent: Some(40),
            ..
        }
    ));
    let p = apply(&p, Event::Progress(Some(80)));
    assert!(matches!(
        p,
        UpdatePhase::Downloading {
            percent: Some(80),
            ..
        }
    ));
    let p = apply(&p, Event::Verified(release("0.6.2")));
    assert_eq!(
        p,
        UpdatePhase::Ready {
            release: release("0.6.2")
        }
    );
    let p = apply(&p, Event::InstallStarted);
    assert_eq!(
        p,
        UpdatePhase::Installing {
            release: release("0.6.2")
        }
    );
}

#[test]
fn a_ready_download_keeps_its_button_while_the_next_check_runs() {
    let ready = UpdatePhase::Ready {
        release: release("0.6.2"),
    };
    assert_eq!(apply(&ready, Event::CheckStarted), ready);
    let down = UpdatePhase::Downloading {
        release: release("0.6.2"),
        percent: None,
    };
    assert_eq!(apply(&down, Event::CheckStarted), down);
}

#[test]
fn progress_and_install_only_follow_their_own_phases() {
    assert_eq!(
        apply(&UpdatePhase::Checking, Event::Progress(Some(5))),
        UpdatePhase::Checking
    );
    assert_eq!(
        apply(
            &UpdatePhase::UpToDate { skipped: None },
            Event::InstallStarted
        ),
        UpdatePhase::UpToDate { skipped: None }
    );
    let installing = UpdatePhase::Installing {
        release: release("0.6.2"),
    };
    // Nothing but its own failure or refusal moves an install that started.
    assert_eq!(apply(&installing, Event::CheckStarted), installing);
    assert_eq!(
        apply(&installing, Event::Unconfigured("x".into())),
        installing
    );
    assert_eq!(
        apply(&installing, Event::BackToReady),
        UpdatePhase::Ready {
            release: release("0.6.2")
        }
    );
    assert_eq!(
        apply(&installing, Event::Failed("no".into())),
        UpdatePhase::Failed {
            reason: "no".into()
        }
    );
}

#[test]
fn unconfigured_nothing_new_and_failure_land_where_they_say() {
    assert_eq!(
        apply(&UpdatePhase::Checking, Event::Unconfigured("why".into())),
        UpdatePhase::NotSetUp {
            reason: "why".into()
        }
    );
    assert_eq!(
        apply(
            &UpdatePhase::Checking,
            Event::NothingNew {
                skipped: Some("0.6.3".into())
            }
        ),
        UpdatePhase::UpToDate {
            skipped: Some("0.6.3".into())
        }
    );
    assert_eq!(
        apply(&UpdatePhase::Checking, Event::Failed("offline".into())),
        UpdatePhase::Failed {
            reason: "offline".into()
        }
    );
}

#[test]
fn the_state_serializes_with_a_phase_tag_and_camel_case() {
    let json = serde_json::to_value(UpdatePhase::Downloading {
        release: release("0.6.2"),
        percent: Some(3),
    })
    .unwrap();
    assert_eq!(json["phase"], "downloading");
    assert_eq!(json["release"]["pubDate"], serde_json::Value::Null);
    assert_eq!(json["percent"], 3);
    assert_eq!(
        serde_json::to_value(UpdatePhase::UpToDate { skipped: None }).unwrap()["phase"],
        "upToDate"
    );
}

// ── The manifest ────────────────────────────────────────────────────────

#[test]
fn the_workers_manifest_parses() {
    let raw: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    let m = manifest::parse(&raw, Channel::Nightly, &host()).unwrap();
    assert_eq!(m.version, "0.6.1234");
    assert_eq!(m.notes, "What's new, as plain text");
    assert_eq!(m.pub_date.as_deref(), Some("2026-11-15T12:00:00Z"));
    assert_eq!(m.channel, Channel::Nightly);
    assert_eq!(
        m.commit.as_deref(),
        Some("0123456789abcdef0123456789abcdef01234567")
    );
    assert_eq!(m.build_id.as_deref(), Some("nightly-20261115-120000"));
    assert_eq!(
        m.url.as_str(),
        "https://updates.example.com/builds/0.6.1234/Hollow.app.tar.gz"
    );
    let r = m.release(false);
    assert_eq!((r.version.as_str(), r.downgrade), ("0.6.1234", false));
}

#[test]
fn a_manifest_for_another_channel_host_or_build_is_refused() {
    let raw: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    // Asked for main, got nightly.
    assert!(manifest::parse(&raw, Channel::Main, &host())
        .unwrap_err()
        .contains("nightly channel"));
    let mutate = |f: &dyn Fn(&mut serde_json::Value)| {
        let mut m = raw.clone();
        f(&mut m);
        manifest::parse(&m, Channel::Nightly, &host())
    };
    let url = |u: &str| {
        let u = u.to_string();
        move |m: &mut serde_json::Value| {
            m["platforms"]["darwin-aarch64"]["url"] = u.clone().into();
        }
    };
    // The token travels with the download: never to another host.
    assert!(mutate(&url(
        "https://evil.example/builds/0.6.1234/Hollow.app.tar.gz"
    ))
    .is_err());
    assert!(mutate(&url(
        "http://updates.example.com/builds/0.6.1234/Hollow.app.tar.gz"
    ))
    .is_err());
    assert!(mutate(&url(
        "https://updates.example.com:8443/builds/0.6.1234/Hollow.app.tar.gz"
    ))
    .is_err());
    // Another version's build, a query, another file.
    assert!(mutate(&url(
        "https://updates.example.com/builds/0.6.1/Hollow.app.tar.gz"
    ))
    .is_err());
    assert!(mutate(&url(
        "https://updates.example.com/builds/0.6.1234/Hollow.app.tar.gz?x=1"
    ))
    .is_err());
    assert!(mutate(&url(
        "https://updates.example.com/builds/0.6.1234/Hollow.dmg"
    ))
    .is_err());
    assert!(mutate(&|m| m["platforms"]["darwin-aarch64"]["signature"] = " ".into()).is_err());
    assert!(mutate(&|m| m["version"] = "0.6.12/../1".into()).is_err());
    assert!(mutate(&|m| {
        m["platforms"] =
            serde_json::json!({ "darwin-x86_64": m["platforms"]["darwin-aarch64"].clone() })
    })
    .is_err());
    assert!(manifest::parse(&serde_json::json!([]), Channel::Nightly, &host()).is_err());
    // Optional fields that are malformed are dropped, not trusted.
    let m = mutate(&|m| m["commit"] = "not-a-commit".into()).unwrap();
    assert_eq!(m.commit, None);
}

// ── Config ──────────────────────────────────────────────────────────────

fn config(pubkey: &str) -> Config {
    Config {
        host: config::host_from(None, false),
        pubkey: pubkey.into(),
        team: config::team_from(None),
        running_version: "0.6.1".into(),
        review_copy: false,
    }
}

const TOKEN: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJ0123-_6";

#[test]
fn the_host_defaults_to_the_worker_and_takes_only_an_origin_override() {
    assert_eq!(
        config::host_from(None, false).unwrap().as_str(),
        "https://updates.example.com/"
    );
    assert_eq!(
        config::host_from(Some("  ".into()), false)
            .unwrap()
            .as_str(),
        "https://updates.example.com/"
    );
    let local = config::host_from(Some("http://127.0.0.1:4100/".into()), false).unwrap();
    assert_eq!(
        config::endpoint(&local, Channel::Nightly).as_str(),
        "http://127.0.0.1:4100/channels/nightly.json"
    );
    // A release build takes only https, and never credentials.
    assert!(config::host_from(None, true).is_ok());
    assert!(config::host_from(Some("http://127.0.0.1:4100".into()), true).is_err());
    assert!(config::host_from(Some("https://user@updates.example.com".into()), true).is_err());
    let host = config::host_from(None, true).unwrap();
    let with_user =
        Url::parse("https://user:pw@updates.example.com/builds/0.6.1/Hollow.app.tar.gz").unwrap();
    assert!(!config::same_origin(&with_user, &host));
    // The overrides are read in tests and debug builds only; the source
    // gates them on `cfg(any(test, debug_assertions))`.
    let source = include_str!("config.rs");
    assert_eq!(
        source
            .matches("#[cfg(not(any(test, debug_assertions)))]")
            .count(),
        2
    );
    for bad in [
        "ftp://x",
        "https://u:p@updates.example.com",
        "https://updates.example.com/prefix",
        "https://updates.example.com?x=1",
        "not a url",
    ] {
        assert!(config::host_from(Some(bad.into()), false).is_err(), "{bad}");
    }
}

#[test]
fn the_client_is_inert_with_plain_words_until_key_host_and_token_exist() {
    // No public key (today's builds): not set up, whatever the token.
    let err = config("")
        .resolve(Channel::Main, Some(TOKEN.into()))
        .unwrap_err();
    assert_eq!(err, config::NOT_SET_UP);
    assert!(!config("").configured());
    // A key but no token: not connected.
    let c = config("RWQ-key");
    assert!(c.configured());
    assert_eq!(
        c.resolve(Channel::Main, None).unwrap_err(),
        config::NOT_CONNECTED
    );
    assert_eq!(
        c.resolve(Channel::Main, Some("short".into())).unwrap_err(),
        config::NOT_CONNECTED
    );
    // A bad host override.
    let mut bad = config("RWQ-key");
    bad.host = config::host_from(Some("ftp://x".into()), false);
    assert!(bad
        .resolve(Channel::Main, Some(TOKEN.into()))
        .unwrap_err()
        .starts_with(config::NOT_SET_UP));
    // A review copy never updates.
    let mut review = config("RWQ-key");
    review.review_copy = true;
    assert_eq!(
        review
            .resolve(Channel::Main, Some(TOKEN.into()))
            .unwrap_err(),
        config::REVIEW_COPY
    );
    // Everything present.
    let ready = c.resolve(Channel::Nightly, Some(TOKEN.into())).unwrap();
    assert!(
        !format!("{ready:?}").contains(TOKEN),
        "Debug redacts the token"
    );
    assert_eq!(
        ready.endpoint.as_str(),
        "https://updates.example.com/channels/nightly.json"
    );
}

#[test]
fn the_pubkey_comes_from_plugins_updater_and_is_empty_without_it() {
    assert_eq!(config::pubkey_from_plugins(&serde_json::json!({})), "");
    assert_eq!(
        config::pubkey_from_plugins(&serde_json::json!({ "updater": { "pubkey": "" } })),
        ""
    );
    assert_eq!(
        config::pubkey_from_plugins(&serde_json::json!({ "updater": { "pubkey": " abc " } })),
        "abc"
    );
    // The shipped config has the entry the plugin needs to start, empty
    // until the key pair exists (row U11).
    let conf: serde_json::Value =
        serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
    assert_eq!(config::pubkey_from_plugins(&conf["plugins"]), "");
    assert!(conf["plugins"]["updater"].is_object());
    assert_ne!(
        conf["bundle"]["createUpdaterArtifacts"],
        serde_json::json!(true),
        "the release builder signs the archive itself (row U2)"
    );
}

#[test]
fn the_team_defaults_to_the_developer_id_team() {
    use crate::update_install::verify::Team;
    assert_eq!(
        config::team_from(None).unwrap(),
        Team::Id("TEAM000000".into())
    );
    assert_eq!(config::team_from(Some("-".into())).unwrap(), Team::AdHoc);
    assert!(config::team_from(Some("nope".into())).is_err());
}

// ── Token and settings ──────────────────────────────────────────────────

#[test]
fn a_token_is_exactly_43_base64url_characters() {
    assert!(token::valid_shape(TOKEN));
    assert!(!token::valid_shape(&TOKEN[..42]));
    assert!(!token::valid_shape(&format!("{TOKEN}A")));
    assert!(!token::valid_shape(&TOKEN.replace('-', "+")));
    assert!(!token::valid_shape(&TOKEN.replace('-', "=")));
    assert_eq!(token::accept(&format!("  {TOKEN}\n")).unwrap(), TOKEN);
    let refused = token::accept("not-a-token-but-a-secret").unwrap_err();
    assert!(!refused.contains("not-a-token-but-a-secret"));
}

#[test]
fn settings_default_to_main_with_overnight_off_and_never_hold_a_token() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("sub").join("update-settings.json"));
    assert_eq!(store.read(), UpdateSettings::default());
    assert_eq!(store.read().channel, Channel::Main);
    assert!(!store.read().install_overnight);
    let saved = store
        .update(|s| {
            s.channel = Channel::Nightly;
            s.install_overnight = true;
        })
        .unwrap();
    assert_eq!(store.read(), saved);
    let text = std::fs::read_to_string(store.path()).unwrap();
    assert!(text.contains("\"installOvernight\": true"));
    assert!(!text.to_lowercase().contains("token"));
    // A file from an older build with fewer fields, or a broken one.
    std::fs::write(store.path(), r#"{"channel":"nightly"}"#).unwrap();
    assert_eq!(store.read().channel, Channel::Nightly);
    std::fs::write(store.path(), r#"{"skippedVersion":"0.6.9""#).unwrap();
    assert_eq!(store.read(), UpdateSettings::default());
    // The unreadable file is kept aside, not overwritten by the next save.
    let bad = store.path().with_file_name("update-settings.json.bad");
    assert!(std::fs::read_to_string(&bad).unwrap().contains("0.6.9"));
    assert!(!store.path().exists());
    // The downgrade permission lasts 24 hours.
    let mut s = UpdateSettings::default();
    assert!(!s.downgrade_allowed(1_000));
    s.allow_downgrade_once = true;
    s.allow_downgrade_since_ms = Some(1_000);
    assert!(s.downgrade_allowed(1_000 + super::settings::DOWNGRADE_WINDOW_MS - 1));
    assert!(!s.downgrade_allowed(1_000 + super::settings::DOWNGRADE_WINDOW_MS));
}
