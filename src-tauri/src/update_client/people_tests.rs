//! Row U9: `/me` and allowed channels on a teammate's Mac, and People on the
//! publisher's. Fakes, temp folders and a one-shot server on 127.0.0.1; no
//! Keychain, ssh, R2 or outside network.

use super::client_tests::{rig, Setup, TOKEN};
use super::fetch::{CheckError, Fetcher, Pending};
use super::people::{self, Access, Me, MeAnswer, NO_CHANNEL, REVOKED};
use super::people_admin::{self, PeopleRunner, UpdatePerson};
use super::settings::UpdateSettings;
use super::state::{Channel, UpdatePhase};
use base64::Engine as _;
use reqwest::Url;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

// ── /me parsing and classification ──────────────────────────────────────

#[test]
fn me_reads_name_and_known_channels_only() {
    let me = people::parse_me(br#"{"name":"Sam","channels":["nightly","main"]}"#).unwrap();
    assert_eq!(
        me,
        Me {
            name: "Sam".into(),
            channels: vec![Channel::Main, Channel::Nightly],
        }
    );
    let me = people::parse_me(br#"{"name":"Sam","channels":["beta","main","main"]}"#).unwrap();
    assert_eq!(
        me.channels,
        vec![Channel::Main],
        "unknown and repeated channels drop"
    );
    assert_eq!(
        people::parse_me(br#"{"name":"Sam","channels":[]}"#)
            .unwrap()
            .channels,
        Vec::<Channel>::new()
    );
    let me = people::parse_me("{\"name\":\" S\\u0000a\\nm\\u2028 \",\"channels\":[]}".as_bytes())
        .unwrap();
    assert_eq!(
        me.name, "Sam",
        "control and separator characters leave the name"
    );
    let long = format!(r#"{{"name":"{}","channels":[]}}"#, "x".repeat(500));
    assert_eq!(
        people::parse_me(long.as_bytes())
            .unwrap()
            .name
            .chars()
            .count(),
        people::MAX_NAME_CHARS
    );
    // Fields past {name, channels} (a newer Worker's) are ignored, never kept.
    assert_eq!(
        people::parse_me(br#"{"name":"Sam","channels":["main"],"tokenHash":"ab","team":"x"}"#)
            .unwrap(),
        Me {
            name: "Sam".into(),
            channels: vec![Channel::Main],
        }
    );
    // Format characters (BOM, zero widths, bidi overrides) leave the name too.
    assert_eq!(
        people::parse_me(
            "{\"name\":\"\u{feff}S\u{200b}a\u{202e}m\u{2066}\",\"channels\":[]}".as_bytes()
        )
        .unwrap()
        .name,
        "Sam"
    );
    // Both fields are needed, as their own types.
    for bad in [
        &br#"{"name":"Sam"}"#[..],
        br#"{"name":"Sam","channels":"main"}"#,
        br#"{"channels":[]}"#,
        br#"[]"#,
        b"not json",
    ] {
        assert!(
            people::parse_me(bad).is_err(),
            "{}",
            String::from_utf8_lossy(bad)
        );
    }
    assert!(people::parse_me(&vec![b' '; people::MAX_BODY + 1]).is_err());
}

#[test]
fn me_statuses_mean_what_they_say() {
    let ok = br#"{"name":"Sam","channels":["main"]}"#;
    assert!(matches!(people::classify(200, ok), MeAnswer::Known(_)));
    assert_eq!(people::classify(401, b"Unauthorized\n"), MeAnswer::Refused);
    assert_eq!(people::classify(404, b""), MeAnswer::Unsupported);
    assert!(matches!(
        people::classify(200, b"<html>"),
        MeAnswer::Network(_)
    ));
    assert!(matches!(people::classify(503, b""), MeAnswer::Network(_)));
    assert!(matches!(people::classify(403, b""), MeAnswer::Network(_)));
}

// ── /me against a local server ──────────────────────────────────────────

/// Answers one request with `status` and `body` (and `extra` headers), and
/// hands back the request line and its Authorization header.
fn one_shot(
    status: &'static str,
    body: &'static str,
    extra: &'static str,
) -> (Url, std::thread::JoinHandle<(String, Option<String>)>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let handle = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let mut auth = None;
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
                break;
            }
            if let Some((k, v)) = header.split_once(':') {
                if k.eq_ignore_ascii_case("authorization") {
                    auth = Some(v.trim().to_string());
                }
            }
        }
        let mut out = stream;
        let _ = write!(
            out,
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\n{extra}Connection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = out.flush();
        (line.trim().to_string(), auth)
    });
    (url, handle)
}

#[test]
fn fetch_me_sends_the_bearer_token_to_slash_me_only() {
    let (host, server) = one_shot("200 OK", r#"{"name":"Sam","channels":["main"]}"#, "");
    let answer = people::fetch_me(&host, TOKEN);
    let (line, auth) = server.join().unwrap();
    assert_eq!(line, "GET /me HTTP/1.1");
    assert_eq!(auth.as_deref(), Some(format!("Bearer {TOKEN}").as_str()));
    assert_eq!(
        answer,
        MeAnswer::Known(Me {
            name: "Sam".into(),
            channels: vec![Channel::Main],
        })
    );

    let (host, server) = one_shot("401 Unauthorized", "Unauthorized\n", "");
    assert_eq!(people::fetch_me(&host, TOKEN), MeAnswer::Refused);
    server.join().unwrap();

    // A redirect is not followed (the token would go with it).
    let (host, server) = one_shot("302 Found", "", "Location: http://127.0.0.1:9/me\r\n");
    let answer = people::fetch_me(&host, TOKEN);
    server.join().unwrap();
    match answer {
        MeAnswer::Network(why) => assert!(!why.contains(TOKEN)),
        other => panic!("a redirect must not be followed: {other:?}"),
    }
}

// ── The client with a scripted /me ──────────────────────────────────────

#[derive(Clone, Default)]
struct MeFetcher {
    me: Arc<Mutex<VecDeque<MeAnswer>>>,
    checks: Arc<Mutex<Vec<String>>>,
    channel: Arc<Mutex<VecDeque<CheckError>>>,
}

impl Fetcher for MeFetcher {
    fn check(&self, endpoint: &Url, _token: &str) -> Result<Option<Box<dyn Pending>>, CheckError> {
        self.checks
            .lock()
            .unwrap()
            .push(endpoint.path().to_string());
        Err(self
            .channel
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(CheckError::NothingPublished))
    }
    fn me(&self, _host: &Url, token: &str) -> MeAnswer {
        assert_eq!(token, TOKEN);
        self.me
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(MeAnswer::Unsupported)
    }
}

fn with_me(answers: Vec<MeAnswer>) -> (super::client_tests::Rig, MeFetcher) {
    let fetcher = MeFetcher::default();
    fetcher.me.lock().unwrap().extend(answers);
    let r = rig(Setup {
        fetcher: Some(Box::new(fetcher.clone())),
        ..Setup::default()
    });
    (r, fetcher)
}

fn known(name: &str, channels: &[Channel]) -> MeAnswer {
    MeAnswer::Known(Me {
        name: name.into(),
        channels: channels.to_vec(),
    })
}

#[test]
fn me_fills_the_state_and_limits_the_channel_picker() {
    let (r, f) = with_me(vec![known("Sam", &[Channel::Main])]);
    let before = r.client.state();
    assert_eq!(
        before.allowed_channels,
        vec![Channel::Main, Channel::Nightly],
        "both until /me answers"
    );
    assert_eq!(before.person_name, None);
    let state = r.client.check();
    assert_eq!(state.person_name.as_deref(), Some("Sam"));
    assert_eq!(state.allowed_channels, vec![Channel::Main]);
    assert_eq!(f.checks.lock().unwrap().as_slice(), ["/channels/main.json"]);
    // The picker's refusal is typed and changes nothing.
    let refused = r.client.set_channel(Channel::Nightly).unwrap_err();
    assert!(refused.starts_with("CHANNEL_NOT_ALLOWED: "), "{refused}");
    assert_eq!(r.client.settings().channel, Channel::Main);
    assert!(
        r.client.set_channel(Channel::Main).is_ok(),
        "the allowed one is fine"
    );
    // Kept across launches in the settings file, never the token.
    let saved = std::fs::read_to_string(r.client.settings_store_for_tests().path()).unwrap();
    assert!(saved.contains("\"personName\": \"Sam\""), "{saved}");
    assert!(!saved.contains(TOKEN));
}

#[test]
fn a_channel_no_longer_allowed_falls_back_to_the_first_allowed_with_a_note() {
    let (r, f) = with_me(vec![
        known("Sam", &[Channel::Main, Channel::Nightly]),
        known("Sam", &[Channel::Main]),
        known("Sam", &[Channel::Main, Channel::Nightly]),
    ]);
    r.client.check();
    r.client.set_channel(Channel::Nightly).unwrap();
    let state = r.client.check();
    assert_eq!(state.channel, Channel::Main);
    let note = state.note.clone().unwrap();
    assert!(
        note.contains("no longer includes Nightly") && note.contains("switched to Main"),
        "{note}"
    );
    assert!(
        !state.downgrade_allowed,
        "a forced move arms no downgrade, and disarms the one the person's own switch to Nightly armed"
    );
    assert!(note.contains("older than this one isn't offered"), "{note}");
    assert_eq!(
        f.checks.lock().unwrap().last().unwrap(),
        "/channels/main.json",
        "the same check goes on with Main"
    );
    // The note stays through later answers until access comes back.
    assert_eq!(r.client.state().note, Some(note));
    let state = r.client.check();
    assert_eq!(state.note, None, "access came back");
    assert_eq!(
        state.channel,
        Channel::Main,
        "and the channel stays where it moved"
    );
}

#[test]
fn no_channel_allowed_stops_the_check_with_plain_words() {
    let (r, f) = with_me(vec![known("Sam", &[])]);
    let state = r.client.check();
    assert_eq!(state.allowed_channels, Vec::<Channel>::new());
    assert!(matches!(&state.phase, UpdatePhase::Failed { reason } if reason == NO_CHANNEL));
    assert!(f.checks.lock().unwrap().is_empty(), "no channel request");
    assert!(r.client.set_channel(Channel::Nightly).is_err());
    assert!(r.client.set_channel(Channel::Main).is_err());
}

#[test]
fn a_revoked_token_fails_at_its_next_check_in_plain_words() {
    let (r, f) = with_me(vec![known("Sam", &[Channel::Main]), MeAnswer::Refused]);
    assert!(matches!(
        r.client.check().phase,
        UpdatePhase::UpToDate { .. }
    ));
    let state = r.client.check();
    match &state.phase {
        UpdatePhase::Failed { reason } => {
            assert_eq!(reason, REVOKED);
            assert!(reason.contains("did not accept this token (it may have been revoked)"));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        f.checks.lock().unwrap().len(),
        1,
        "the channel isn't asked after a refused /me"
    );
    assert!(
        state.connected,
        "the token stays until the person disconnects"
    );
}

#[test]
fn a_401_on_the_channel_reads_the_same_as_on_me() {
    // A Worker from before /me: 404 there, then the channel refuses.
    let (r, f) = with_me(vec![MeAnswer::Unsupported]);
    f.channel
        .lock()
        .unwrap()
        .push_back(CheckError::Unauthorized);
    let state = r.client.check();
    assert!(matches!(&state.phase, UpdatePhase::Failed { reason } if reason == REVOKED));
    assert_eq!(
        state.allowed_channels,
        vec![Channel::Main, Channel::Nightly],
        "nothing learned, both offered"
    );
}

#[test]
fn an_unreachable_me_keeps_what_was_known() {
    let (r, _f) = with_me(vec![
        known("Sam", &[Channel::Main]),
        MeAnswer::Network("offline".into()),
    ]);
    r.client.check();
    let state = r.client.check();
    assert_eq!(state.allowed_channels, vec![Channel::Main]);
    assert_eq!(state.person_name.as_deref(), Some("Sam"));
}

#[test]
fn a_new_token_or_a_disconnect_forgets_whose_it_was() {
    let (r, _f) = with_me(vec![known("Sam", &[Channel::Main])]);
    r.client.check();
    r.client
        .connect("Zz3sVb8xYk1mN0pQrStUvWxYz0123456789-_AbCdEf")
        .unwrap();
    let state = r.client.state();
    assert_eq!(state.person_name, None);
    assert_eq!(
        state.allowed_channels,
        vec![Channel::Main, Channel::Nightly]
    );

    let (r, _f) = with_me(vec![known("Sam", &[])]);
    r.client.check();
    r.client.disconnect().unwrap();
    let state = r.client.state();
    assert_eq!(state.person_name, None);
    assert_eq!(state.note, None, "the access note goes with it");
    assert_eq!(state.allowed_channels.len(), 2);
}

#[test]
fn reconcile_is_a_pure_settings_change_that_never_arms_a_downgrade() {
    let mut s = UpdateSettings {
        channel: Channel::Nightly,
        dismissed_version: Some("0.6.9".into()),
        // The person's own earlier switch had armed one.
        allow_downgrade_once: true,
        allow_downgrade_since_ms: Some(7),
        ..UpdateSettings::default()
    };
    let me = Me {
        name: "".into(),
        channels: vec![Channel::Main],
    };
    assert_eq!(
        people::reconcile(&mut s, &me),
        Access::Moved {
            from: Channel::Nightly,
            to: Channel::Main,
        }
    );
    assert_eq!(s.channel, Channel::Main);
    assert_eq!(s.person_name, None, "an empty name is no name");
    assert!(!s.allow_downgrade_once, "a forced move arms no downgrade");
    assert_eq!(
        s.allow_downgrade_since_ms, None,
        "and disarms a leftover one"
    );
    assert!(!s.downgrade_allowed(8));
    assert_eq!(
        s.dismissed_version.as_deref(),
        Some("0.6.9"),
        "the channel and the note only"
    );
    // A move from a never-armed state stays unarmed.
    let mut fresh = UpdateSettings {
        channel: Channel::Main,
        ..UpdateSettings::default()
    };
    let nightly_only = Me {
        name: "Sam".into(),
        channels: vec![Channel::Nightly],
    };
    assert!(matches!(
        people::reconcile(&mut fresh, &nightly_only),
        Access::Moved { .. }
    ));
    assert!(!fresh.allow_downgrade_once && fresh.allow_downgrade_since_ms.is_none());
    // Same leaves everything as it is, an armed downgrade the person made included.
    s.allow_downgrade_once = true;
    s.allow_downgrade_since_ms = Some(9);
    assert_eq!(people::reconcile(&mut s, &me), Access::Same);
    assert_eq!(s.allow_downgrade_since_ms, Some(9), "Same changes nothing");
}

// ── People on the publisher's Mac ───────────────────────────────────────

#[test]
fn a_new_token_is_43_base64url_characters_of_32_random_bytes() {
    let mut seen = std::collections::HashSet::new();
    let mut ones = 0u32;
    for _ in 0..64 {
        let token = people_admin::new_token(&mut people_admin::os_fill).unwrap();
        assert_eq!(token.len(), 43);
        assert!(super::token::valid_shape(&token));
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&token)
            .unwrap();
        assert_eq!(bytes.len(), 32);
        ones += bytes.iter().map(|b| b.count_ones()).sum::<u32>();
        assert!(seen.insert(token), "no token repeats");
    }
    // 64 x 256 bits: about half are set. The bounds are > 30 standard
    // deviations wide, so this only catches a broken source (all zero,
    // all one, a constant).
    assert!((6000..10400).contains(&ones), "{ones} bits set");
    // A fixed source gives the expected text.
    let fixed = people_admin::new_token(&mut |b: &mut [u8]| {
        b.fill(0xfb);
        Ok(())
    })
    .unwrap();
    assert_eq!(fixed, "-_v7".repeat(10) + "-_s");
    let failing = people_admin::new_token(&mut |_b: &mut [u8]| Err("no entropy".into()));
    assert!(failing.is_err());
}

#[test]
fn the_token_hash_is_lowercase_hex_sha256_of_its_text() {
    assert_eq!(
        people_admin::token_hash("abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[derive(Default)]
struct FakeRunner {
    calls: Mutex<Vec<(Vec<String>, Vec<u8>)>>,
    answer: Mutex<Option<Result<String, String>>>,
}

impl PeopleRunner for FakeRunner {
    fn run(&self, args: &[String], stdin: &[u8]) -> Result<String, String> {
        self.calls
            .lock()
            .unwrap()
            .push((args.to_vec(), stdin.to_vec()));
        self.answer
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| Ok(honest(args)))
    }
}

/// What a runner that did the change answers.
fn honest(args: &[String]) -> String {
    let person = |id: &str, name: &str, channels: &[String], revoked: bool| {
        serde_json::json!({
            "id": id, "name": name, "channels": channels, "revoked": revoked,
            "createdAt": "2026-11-15T12:00:00Z",
        })
        .to_string()
    };
    match args[0].as_str() {
        "add" => person("p-0123456789ab", &args[1], &args[2..], false),
        "revoke" => person(&args[1], "Sam", &["main".to_string()], true),
        "set-channels" => person(&args[1], "Sam", &args[2..], false),
        _ => r#"{"people":[]}"#.to_string(),
    }
}

fn publisher() -> UpdateSettings {
    UpdateSettings {
        publisher: true,
        ..UpdateSettings::default()
    }
}

#[test]
fn add_sends_only_the_hash_on_stdin_and_returns_the_link_once() {
    let runner = FakeRunner::default();
    let host = Url::parse("https://updates.example.com").unwrap();
    let added = people_admin::add(
        &publisher(),
        &host,
        &runner,
        &mut people_admin::os_fill,
        "  Sam  ",
        &[Channel::Nightly, Channel::Main, Channel::Main],
    )
    .unwrap();
    let token = added
        .install_link
        .strip_prefix("https://updates.example.com/install/")
        .unwrap()
        .to_string();
    assert!(super::token::valid_shape(&token));
    let calls = runner.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    let (args, stdin) = &calls[0];
    assert_eq!(args, &["add", "Sam", "main", "nightly"]);
    let hash = people_admin::token_hash(&token);
    assert_eq!(stdin, format!("{hash}\n").as_bytes(), "the hash, on stdin");
    for a in args {
        assert!(!a.contains(&token) && !a.contains(&hash), "never in argv");
    }
    assert!(
        !String::from_utf8_lossy(stdin).contains(&token),
        "the token never leaves"
    );
    assert_eq!(added.person.id, "p-0123456789ab");
    // Its Debug (a log line, a panic) hides the link.
    let debug = format!("{added:?}");
    assert!(!debug.contains(&token), "{debug}");
    // The serialized answer is the one place the link appears.
    let json = serde_json::to_value(&added).unwrap();
    assert_eq!(json["installLink"].as_str().unwrap(), added.install_link);
}

#[test]
fn the_link_is_never_stored_by_the_client() {
    // A full add through a client's settings: nothing on disk names the token.
    let r = rig(Setup::default());
    r.client.set_publisher(true).unwrap();
    let runner = FakeRunner::default();
    let host = Url::parse("https://updates.example.com").unwrap();
    let added = people_admin::add(
        &r.client.settings(),
        &host,
        &runner,
        &mut people_admin::os_fill,
        "Sam",
        &[Channel::Main],
    )
    .unwrap();
    let token = added.install_link.rsplit('/').next().unwrap().to_string();
    let dir = r
        .client
        .settings_store_for_tests()
        .path()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    for entry in walk(&dir) {
        let bytes = std::fs::read(&entry).unwrap_or_default();
        assert!(
            !String::from_utf8_lossy(&bytes).contains(&token),
            "{} holds the token",
            entry.display()
        );
    }
    let state = serde_json::to_string(&r.client.state()).unwrap();
    assert!(!state.contains(&token));
    assert!(!serde_json::to_string(&r.client.settings())
        .unwrap()
        .contains(&token));
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

#[test]
fn people_is_refused_off_a_publisher_mac_and_checks_its_input() {
    let runner = FakeRunner::default();
    let host = Url::parse("https://updates.example.com").unwrap();
    let not = UpdateSettings::default();
    assert_eq!(
        people_admin::list(&not, &runner).unwrap_err(),
        people_admin::NOT_PUBLISHER
    );
    assert!(people_admin::add(
        &not,
        &host,
        &runner,
        &mut people_admin::os_fill,
        "Sam",
        &[Channel::Main]
    )
    .is_err());
    assert!(people_admin::revoke(&not, &runner, "p-0123456789ab").is_err());
    assert!(people_admin::set_channels(&not, &runner, "p-0123456789ab", &[]).is_err());
    let p = publisher();
    for name in [
        "",
        "   ",
        "I\nan",
        "x".repeat(81).as_str(),
        "a".repeat(64).as_str(),
    ] {
        assert!(
            people_admin::add(
                &p,
                &host,
                &runner,
                &mut people_admin::os_fill,
                name,
                &[Channel::Main]
            )
            .is_err(),
            "{name:?}"
        );
    }
    assert!(
        people_admin::add(&p, &host, &runner, &mut people_admin::os_fill, "Sam", &[]).is_err(),
        "a new person needs a channel"
    );
    for id in ["", "../people", "p-1 2", "f".repeat(64).as_str(), "-rf"] {
        let refused = people_admin::revoke(&p, &runner, id);
        if id == "-rf" {
            // Accepted as an id word; it reaches the script quoted, as an argument.
            assert!(refused.is_ok());
        } else {
            assert!(refused.is_err(), "{id:?}");
        }
    }
    let calls = runner.calls.lock().unwrap();
    assert_eq!(
        calls.len(),
        1,
        "only the -rf revoke reached the runner: {calls:?}"
    );
    assert_eq!(calls[0].0, ["revoke", "-rf"]);
}

#[test]
fn revoke_set_channels_and_list_pass_checked_words_and_read_the_answer() {
    let runner = FakeRunner::default();
    let p = publisher();
    let person: UpdatePerson = people_admin::revoke(&p, &runner, "p-0123456789ab").unwrap();
    assert_eq!(person.name, "Sam");
    people_admin::set_channels(&p, &runner, "p-sam", &[Channel::Nightly]).unwrap();
    people_admin::set_channels(&p, &runner, "p-sam", &[]).unwrap();
    *runner.answer.lock().unwrap() = Some(Ok(
        r#"{"people":[{"id":"p-sam","name":"Sam","channels":[],"revoked":true,"createdAt":""}]}"#
            .into(),
    ));
    let listed = people_admin::list(&p, &runner).unwrap();
    assert_eq!(listed.people[0].channels, Vec::<Channel>::new());
    assert!(listed.people[0].revoked);
    let calls: Vec<Vec<String>> = runner
        .calls
        .lock()
        .unwrap()
        .iter()
        .map(|c| c.0.clone())
        .collect();
    assert_eq!(
        calls,
        vec![
            vec!["revoke", "p-0123456789ab"],
            vec!["set-channels", "p-sam", "nightly"],
            vec!["set-channels", "p-sam"],
            vec!["list"],
        ]
    );
    *runner.answer.lock().unwrap() = Some(Ok("{\"tokenHash\":\"x\"}".into()));
    assert!(
        people_admin::revoke(&p, &runner, "p-sam").is_err(),
        "an answer that isn't a person"
    );
    *runner.answer.lock().unwrap() = Some(Err(
        "The release builder said: no person has that id.".into()
    ));
    assert_eq!(
        people_admin::revoke(&p, &runner, "p-sam").unwrap_err(),
        "The release builder said: no person has that id."
    );
}

#[test]
fn the_remote_command_quotes_every_argument_for_the_shell() {
    // Run the quoted words through a real /bin/sh (no ssh) and read them back.
    let words = vec![
        "add".to_string(),
        "O'Brien; rm -rf ~ $(whoami) `id` \"x\" *".to_string(),
        "main".to_string(),
    ];
    let mut line = "printf '%s\\n'".to_string();
    for w in &words {
        line.push(' ');
        line.push_str(&people_admin::shell_quote(w));
    }
    let out = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(&line)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("{}\n", words.join("\n"))
    );
    let command = people_admin::remote_command(&words);
    assert!(command.starts_with("exec /bin/sh -c '"));
    assert!(
        command.ends_with(" people 'add' 'O'\\''Brien; rm -rf ~ $(whoami) `id` \"x\" *' 'main'"),
        "{command}"
    );
    assert!(people_admin::REMOTE_SCRIPT.contains("scripts/release/people.mjs\" \"$@\""));
}

#[test]
fn runner_exits_become_plain_words_without_secrets() {
    assert_eq!(
        people_admin::runner_result("mini", Some(0), "{}", "").unwrap(),
        "{}"
    );
    let err = people_admin::runner_result(
        "mini",
        Some(1),
        "",
        "node: warning\npeople: no person has that id\n",
    )
    .unwrap_err();
    assert_eq!(err, "The release builder said: no person has that id.");
    let err = people_admin::runner_result("mini", Some(1), "", "Error: R2_SECRET leaked here\n")
        .unwrap_err();
    assert!(
        !err.contains("R2_SECRET"),
        "only the script's own line is repeated: {err}"
    );
    assert!(people_admin::runner_result("mini", None, "", "")
        .unwrap_err()
        .contains("didn't finish"));
    assert!(people_admin::runner_result(
        "mini",
        Some(255),
        "",
        "ssh: connect to host mini port 22: Operation timed out"
    )
    .unwrap_err()
    .contains("mini"));
    // No runner named: refused before ssh.
    let none = people_admin::SshRunner {
        ssh: "/usr/bin/false".into(),
        host: None,
    };
    assert_eq!(
        none.run(&["list".into()], &[]).unwrap_err(),
        people_admin::NO_RUNNER
    );
}

#[test]
fn a_runner_whose_answer_does_not_match_the_change_is_refused() {
    let p = publisher();
    let host = Url::parse("https://updates.example.com").unwrap();
    let person = |id: &str, name: &str, channels: &[&str], revoked: bool| {
        Some(Ok(serde_json::json!({
            "id": id, "name": name, "channels": channels, "revoked": revoked, "createdAt": "",
        })
        .to_string()))
    };
    let lying = |answer: Option<Result<String, String>>| {
        let runner = FakeRunner::default();
        *runner.answer.lock().unwrap() = answer;
        runner
    };
    // Revoke: another id, or the person still active.
    for answer in [
        person("p-0000000000ff", "Sam", &["main"], true),
        person("p-0123456789ab", "Sam", &["main"], false),
    ] {
        assert_eq!(
            people_admin::revoke(&p, &lying(answer), "p-0123456789ab").unwrap_err(),
            people_admin::MISMATCH
        );
    }
    // Set-channels: other channels, fewer, more, another person.
    for answer in [
        person("p-sam", "Sam", &["nightly"], false),
        person("p-sam", "Sam", &[], false),
        person("p-sam", "Sam", &["main", "nightly"], false),
        person("p-other", "Sam", &["main"], false),
    ] {
        assert_eq!(
            people_admin::set_channels(&p, &lying(answer), "p-sam", &[Channel::Main]).unwrap_err(),
            people_admin::MISMATCH
        );
    }
    assert_eq!(
        people_admin::set_channels(
            &p,
            &lying(person("p-sam", "Sam", &["main"], false)),
            "p-sam",
            &[]
        )
        .unwrap_err(),
        people_admin::MISMATCH,
        "none asked for, Main answered"
    );
    // Add: revoked, another name or channels, an id add doesn't make.
    for answer in [
        person("p-0123456789ab", "Sam", &["main"], true),
        person("p-0123456789ab", "Other", &["main"], false),
        person("p-0123456789ab", "Sam", &["main", "nightly"], false),
        person("p-sam", "Sam", &["main"], false),
        person("p-0123456789AB", "Sam", &["main"], false),
        person("p-0123456789abc", "Sam", &["main"], false),
        person("x-0123456789ab", "Sam", &["main"], false),
    ] {
        let err = people_admin::add(
            &p,
            &host,
            &lying(answer),
            &mut people_admin::os_fill,
            "Sam",
            &[Channel::Main],
        )
        .unwrap_err();
        assert_eq!(
            err,
            people_admin::MISMATCH_ADD,
            "no link for an unconfirmed add, and the message says what to do"
        );
        assert!(err.contains("revoke") && err.contains("add them again"));
        assert!(!err.contains('\u{2014}') && !err.contains('\u{2013}'));
    }
    // The honest answers pass.
    let runner = FakeRunner::default();
    assert!(people_admin::revoke(&p, &runner, "p-sam").unwrap().revoked);
    assert_eq!(
        people_admin::set_channels(&p, &runner, "p-sam", &[Channel::Nightly, Channel::Main])
            .unwrap()
            .channels,
        vec![Channel::Main, Channel::Nightly]
    );
    let added = people_admin::add(
        &p,
        &host,
        &runner,
        &mut people_admin::os_fill,
        " Zoë ",
        &[Channel::Nightly],
    )
    .unwrap();
    assert_eq!(added.person.name, "Zoë");
    assert!(people_admin::new_id_shape("p-0123456789ab"));
    assert!(!people_admin::new_id_shape("p-0123456789a"));
}

#[test]
fn names_with_format_characters_are_refused_before_trimming() {
    // The same set people.mjs refuses; U+FEFF is trimmed by JS but not Rust.
    for name in [
        "\u{feff}Sam",
        "Sam\u{feff}",
        "I\u{200b}an",
        "I\u{200d}an",
        "\u{202e}naI",
        "Sam\u{2066}",
        "Ia\u{ad}n",
        "\u{85}Sam",
        "I\nan",
        "a\u{2028}b",
    ] {
        let err = people_admin::check_name(name).unwrap_err();
        assert!(err.contains("invisible formatting"), "{name:?}: {err}");
    }
    assert_eq!(
        people_admin::check_name(" Zoë Ångström ").unwrap(),
        "Zoë Ångström"
    );
    assert!(people::hidden_char('\u{e0041}') && !people::hidden_char('é'));
}

/// `hidden_char` is a hand copy of the JS name check's character set
/// (`\p{Cc}\p{Cf}` plus U+2028 and U+2029). `scripts/hidden-chars.fixture.json`
/// holds the code point ranges Node's regex refuses; the Node side
/// (`scripts/release-people.test.mjs`) checks it against the regex, this side
/// checks it against `hidden_char` over every scalar value, so a drift on
/// either side fails a test.
#[test]
fn hidden_char_matches_the_js_name_check_over_every_code_point() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../scripts/hidden-chars.fixture.json")).unwrap();
    let ranges: Vec<(u32, u32)> = fixture["ranges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r[0].as_u64().unwrap() as u32, r[1].as_u64().unwrap() as u32))
        .collect();
    let in_fixture = |c: u32| ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&c));
    for c in (0..=0x10FFFFu32).filter_map(char::from_u32) {
        assert_eq!(
            people::hidden_char(c),
            in_fixture(c as u32),
            "U+{:04X}: Rust and the JS name check disagree (fixture Unicode {})",
            c as u32,
            fixture["unicode"]
        );
    }
}

#[test]
fn people_ssh_refuses_an_unknown_host_key() {
    let args = people_admin::ssh_args("runner-host", &["list".to_string()]);
    let at = args
        .iter()
        .position(|a| a == "StrictHostKeyChecking=yes")
        .expect("strict host key checking");
    assert_eq!(args[at - 1], "-o");
    let dash = args.iter().position(|a| a == "--").unwrap();
    assert!(at < dash, "an option, before the host");
    assert_eq!(args[dash + 1], "runner-host");
    assert_eq!(args.len(), dash + 3, "one command line after the host");
    assert!(args.iter().any(|a| a == "BatchMode=yes"));
}

#[test]
fn the_install_link_is_wiped_when_dropped() {
    let mut secret = "https://updates.example.com/install/abc".to_string();
    people_admin::wipe(&mut secret);
    assert!(secret.is_empty());
}
