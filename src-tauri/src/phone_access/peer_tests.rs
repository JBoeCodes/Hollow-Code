//! M2 PR 2: the peer client (`peer_client`, `peer_machines`, `peer_tunnel`)
//! against this module's rig, which stands in for the other Mac's Hollow:
//! pairing, every chat call, a route refused, the Host header, a revoked
//! token, the event stream with a forced disconnect, and pairing over a
//! stand-in ssh through a real Unix-socket tunnel. No real ssh, data folder
//! or Keychain; every port is a free one.
use super::*;
use crate::peer_client::{
    self, Dial, FollowEvent, FollowOptions, FollowSink, PairCode, PairSetup, PeerClient, PeerError,
    PeerState, ThisMac,
};
use crate::peer_machines::{MachineRecord, Machines};
use crate::peer_tunnel::{self, SshSpawner};

pub(super) const CODE: &str = "P33RC0DE";
pub(super) const MACBOOK: &str = "m-3f9c1a07e2b45d18";

pub(super) fn me() -> ThisMac {
    ThisMac {
        machine_id: MACBOOK.into(),
        slug: Some("macbook-pro".into()),
        name: "MacBook Pro".into(),
        version: "0.5.17 (abc1234)".into(),
    }
}

impl Rig {
    /// The rig's own `machineId` (its test `machine.json`).
    pub(super) fn machine_id(&self) -> String {
        match &self.ctx.machine {
            routes::MachineSource::At { path, .. } => {
                crate::context_roots::ensure_machine_id_at(path).unwrap()
            }
            _ => panic!("the rig has its own machine.json"),
        }
    }

    /// What `hollow serve pair --machine --json` would have printed, with
    /// the machine window open.
    pub(super) fn open_machine_window(&self) -> PairCode {
        self.ctx.machine_pairing.open_with(CODE, Utc::now());
        PairCode {
            code: CODE.into(),
            expires_at: None,
            machine: true,
            machine_id: Some(self.machine_id()),
            slug: None,
            name: Some("Test Mac".into()),
        }
    }

    pub(super) fn pair_line(&self) -> String {
        json!({"code": CODE, "expiresAt": null, "machine": true,
            "machineId": self.machine_id(), "slug": null, "name": "Test Mac"})
        .to_string()
    }

    fn bare_client(&self) -> PeerClient {
        PeerClient::new(Dial::Tcp(self.port), self.port, "Test Mac")
    }
}

/// Pair this "MacBook" with the rig over loopback TCP.
async fn paired(rig: &Rig) -> (Arc<Machines>, MachineRecord, PeerClient, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let machines = Arc::new(Machines::at(dir.path()));
    let code = rig.open_machine_window();
    let (record, client) = peer_client::pair_with_code(
        &machines,
        rig.bare_client(),
        &code,
        &me(),
        vec!["runner".into(), "runner-lan".into()],
        rig.port,
    )
    .await
    .unwrap();
    (machines, record, client, dir)
}

#[test]
fn the_client_speaks_the_servers_peer_api() {
    assert_eq!(peer_client::PEER_API, wire::PEER_API);
    assert_eq!(
        crate::peer_machines::DEFAULT_REMOTE_PORT,
        server::PORT,
        "the port a Mac's Hollow answers on there"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mac_pairs_through_the_machine_window_and_keeps_its_token_out_of_the_record() {
    let rig = rig().await;
    let (machines, record, client, _dir) = paired(&rig).await;
    // The record: what the other Mac said, how to reach it, no token.
    assert_eq!(record.machine_id, rig.machine_id());
    assert_eq!(record.name, "Test Mac");
    assert_eq!(record.ssh_hosts, ["runner", "runner-lan"]);
    assert_eq!(record.remote_port, rig.port);
    assert!(record.device_id.starts_with("mc-"));
    assert_eq!(record.peer_api, 1);
    assert!(!record.token_revoked);
    assert_eq!(machines.list().unwrap(), vec![record.clone()]);
    let token = crate::peer_machines::read_token(&record.machine_id)
        .unwrap()
        .expect("the token is in the secret store");
    let text = std::fs::read_to_string(machines.path()).unwrap();
    assert!(!text.contains(&token), "the token never reaches the record");
    assert!(!format!("{client:?}").contains(&token));
    // The other side: one machine device for this Mac, by its id.
    let there = rig.store.read().unwrap().machines;
    assert_eq!(there.len(), 1);
    assert_eq!(there[0].device_id, record.device_id);
    assert_eq!(there[0].machine_id.as_deref(), Some(MACBOOK));
    assert_eq!(there[0].name, "MacBook Pro");
    // The check call works with the saved token.
    let info = client.machine().await.unwrap();
    assert_eq!(info.machine_id, record.machine_id);
    assert_eq!(
        info.capabilities,
        ["chats", "send", "stop", "cards", "events", "usage", "moves", "appends"]
    );

    // A spent window, a wrong code, a closed one.
    let again = PairCode {
        code: "WR0NGC0D".into(),
        ..rig.open_machine_window()
    };
    let wrong = peer_client::pair_with_code(
        &machines,
        rig.bare_client(),
        &again,
        &me(),
        vec!["runner".into()],
        rig.port,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            &wrong,
            PeerError::WrongCode {
                attempts_left: Some(4),
                ..
            }
        ),
        "{wrong:?}"
    );
    rig.ctx.machine_pairing.close();
    let closed = peer_client::pair_with_code(
        &machines,
        rig.bare_client(),
        &again,
        &me(),
        vec!["runner".into()],
        rig.port,
    )
    .await
    .unwrap_err();
    assert!(matches!(closed, PeerError::PairingClosed(_)), "{closed:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_client_lists_reads_sends_stops_and_answers_at_the_chats_home() {
    let rig = rig().await;
    let (_machines, _record, client, _dir) = paired(&rig).await;
    let vault = rig.vault.to_string_lossy().into_owned();
    let key = "s-20261115-0000beef";
    seed_chat(
        &rig.vault,
        key,
        "claude",
        &vault,
        &[("Start the release", "Before I start:")],
        "acceptEdits",
    );

    let page = client.sessions(None, 50).await.unwrap();
    let row = page
        .sessions
        .iter()
        .find(|s| s.key == key)
        .expect("the chat is listed");
    assert_eq!(row.state, "idle");
    assert_eq!(row.turn_count, 1);
    let detail = client.detail(key, None, 20).await.unwrap();
    assert_eq!(detail.session.key, key);
    assert_eq!(detail.turns.len(), 2, "a user and an assistant turn");
    assert_eq!(detail.permission_mode.as_deref(), Some("acceptEdits"));
    assert!(detail.live.is_none());

    // A card, answered at the chat's home.
    let card = seed_card(&rig.vault, key, 1, "toolu-peer", &three_questions());
    let mut answers = serde_json::Map::new();
    answers.insert("branch".into(), json!("main"));
    answers.insert("checks".into(), json!(["tests"]));
    answers.insert("note".into(), Value::Null);
    let took = client
        .answer(
            key,
            &card.id,
            "8a8b7c6d-5e4f-4a3b-8c2d-1e0f9a8b7c6d",
            &answers,
        )
        .await
        .unwrap();
    assert!(took.accepted);
    let cards = crate::interactions::list_for_session(&rig.vault, key).unwrap();
    assert_eq!(cards[0].status, "answered");
    let late = client
        .answer(
            key,
            &card.id,
            "9a8b7c6d-5e4f-4a3b-8c2d-1e0f9a8b7c6d",
            &answers,
        )
        .await
        .unwrap_err();
    assert!(matches!(late, PeerError::QuestionClosed(_)), "{late:?}");
    assert_eq!(late.state(), PeerState::Online);

    // A send: text only, the chat's own mode, a Mac's turn id. The answer
    // above already sent one turn.
    let sent = client
        .send(key, "Ship it", "7a8b7c6d-5e4f-4a3b-8c2d-1e0f9a8b7c6d")
        .await;
    // The answer's turn holds the chat: busy, said plainly, nothing lost.
    match sent {
        Err(PeerError::Busy(message)) => assert!(!message.is_empty()),
        Ok(accepted) => assert!(accepted.accepted),
        Err(other) => panic!("{other:?}"),
    }
    let other = "s-20261115-0000cafe";
    seed_chat(
        &rig.vault,
        other,
        "claude",
        &vault,
        &[("Hi", "Hello")],
        "plan",
    );
    let accepted = client
        .send(other, "Ship it", "6a8b7c6d-5e4f-4a3b-8c2d-1e0f9a8b7c6d")
        .await
        .unwrap();
    assert!(accepted.accepted);
    assert!(accepted.queue_id > 0);
    let inputs = rig.host.sent.lock().unwrap().clone();
    let input = inputs
        .iter()
        .find(|i| i.session_key.as_deref() == Some(other))
        .expect("the send reached the home's queue");
    assert!(input.prompt.ends_with("\n\nShip it"), "{}", input.prompt);
    assert_eq!(input.permission_mode.as_deref(), Some("plan"));
    assert_eq!(
        input.client_turn_id.as_deref(),
        Some("mac-6a8b7c6d-5e4f-4a3b-8c2d-1e0f9a8b7c6d")
    );

    // Stop.
    client.stop(other).await.unwrap();
    assert!(rig.host.stopped.lock().unwrap().iter().any(|k| k == other));

    // A chat that isn't there.
    let gone = client
        .detail("s-20261115-99999999", None, 20)
        .await
        .unwrap_err();
    assert!(matches!(gone, PeerError::NotFound(_)), "{gone:?}");
    // A key that isn't one is refused here, before any request.
    assert!(matches!(
        client.detail("../v1/me", None, 20).await,
        Err(PeerError::BadRequest(_))
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_route_a_mac_may_not_use_is_refused_plainly_and_the_host_header_is_set() {
    let rig = rig().await;
    let (machines, record, client, _dir) = paired(&rig).await;
    for path in ["/v1/me", "/v1/chat-controls", "/v1/tasks", "/v1/waiting"] {
        let refused = client.call_json("GET", path, None).await.unwrap_err();
        assert_eq!(
            refused,
            PeerError::Forbidden("Another Mac can't use this through Hollow's link.".into()),
            "{path}"
        );
    }
    // The Host header names the port the other Mac answers on, not what
    // this Mac dialed: with another port saved, the Host check refuses.
    let mut elsewhere = record.clone();
    elsewhere.remote_port = 1;
    let wrong = PeerClient::for_record(&elsewhere, Dial::Tcp(rig.port), machines.clone()).unwrap();
    assert_eq!(wrong.host_header(), "127.0.0.1:1");
    let misdirected = wrong.machine().await.unwrap_err();
    assert!(
        matches!(misdirected, PeerError::Misdirected(_)),
        "{misdirected:?}"
    );
    assert_eq!(client.host_header(), format!("127.0.0.1:{}", rig.port));
    // Nothing listening: offline, said plainly.
    let nobody = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = nobody.local_addr().unwrap().port();
    drop(nobody);
    let offline = PeerClient::for_record(&record, Dial::Tcp(port), machines)
        .unwrap()
        .machine()
        .await
        .unwrap_err();
    assert_eq!(offline.state(), PeerState::Offline);
    assert!(
        offline.message().starts_with("Can't reach Test Mac:"),
        "{offline}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_revoked_token_reads_as_pairing_needed_and_marks_the_record() {
    let rig = rig().await;
    let (machines, record, client, _dir) = paired(&rig).await;
    client.sessions(None, 10).await.unwrap();
    assert!(machines
        .get(&record.machine_id)
        .unwrap()
        .unwrap()
        .last_seen_at
        .is_some());
    assert!(rig.store.revoke(&record.device_id).unwrap());
    let revoked = client.sessions(None, 10).await.unwrap_err();
    assert!(matches!(revoked, PeerError::TokenRevoked(_)), "{revoked:?}");
    assert_eq!(revoked.state(), PeerState::PairingNeeded);
    assert_eq!(
        revoked.message(),
        "Test Mac no longer accepts this Mac: it was unpaired there. Pair it again in Settings."
    );
    let now = machines.get(&record.machine_id).unwrap().unwrap();
    assert!(now.token_revoked);
    // A client for a record marked so still calls, and hears 401 again.
    let again = PeerClient::for_record(&now, Dial::Tcp(rig.port), machines.clone()).unwrap();
    assert!(matches!(
        again.machine().await,
        Err(PeerError::TokenRevoked(_))
    ));
    // Pairing again clears it.
    let code = rig.open_machine_window();
    let (fresh, _client) = peer_client::pair_with_code(
        &machines,
        rig.bare_client(),
        &code,
        &me(),
        record.ssh_hosts.clone(),
        rig.port,
    )
    .await
    .unwrap();
    assert!(!fresh.token_revoked);
    assert_ne!(fresh.device_id, record.device_id);
    assert_eq!(machines.list().unwrap().len(), 1);
    // Forget: unpaired there first, then gone here, token and all.
    let client = PeerClient::for_record(&fresh, Dial::Tcp(rig.port), machines.clone()).unwrap();
    assert!(
        peer_client::forget_machine(&machines, &fresh.machine_id, Some(&client))
            .await
            .unwrap()
    );
    assert!(rig.store.read().unwrap().machines.is_empty());
    assert!(machines.list().unwrap().is_empty());
    assert_eq!(
        crate::peer_machines::read_token(&fresh.machine_id).unwrap(),
        None
    );
}

type Seen = Arc<Mutex<Vec<FollowEvent>>>;

fn recorder() -> (Seen, FollowSink) {
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let sink: FollowSink = {
        let seen = seen.clone();
        Arc::new(move |e| seen.lock().unwrap().push(e))
    };
    (seen, sink)
}

/// Wait until an event after `from` matches; its index.
async fn wait_for(
    seen: &Seen,
    from: usize,
    what: &str,
    matches: impl Fn(&FollowEvent) -> bool,
) -> usize {
    let deadline = Instant::now() + NO_HANG;
    loop {
        if let Some(i) = seen.lock().unwrap().iter().skip(from).position(&matches) {
            return from + i;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: {:#?}",
            seen.lock().unwrap()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn frame(e: &FollowEvent, name: &str) -> bool {
    matches!(e, FollowEvent::Event(f) if f.event == name)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_event_stream_follows_a_turn_resumes_after_a_drop_and_refetches_when_it_cant() {
    let rig = rig().await;
    let (_machines, record, client, _dir) = paired(&rig).await;
    let vault = rig.vault.to_string_lossy().into_owned();
    let key = "s-20261115-0000f00d";
    seed_chat(
        &rig.vault,
        key,
        "claude",
        &vault,
        &[("First", "Answer one")],
        "plan",
    );
    // The rig's live buffer has no worker: tick it.
    let ticker = {
        let live = rig.live.clone();
        tokio::spawn(async move {
            loop {
                live.tick(Instant::now());
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        })
    };
    let (seen, sink) = recorder();
    let options = FollowOptions {
        watchdog: Duration::from_secs(20),
        reconnect: vec![Duration::from_millis(50)],
        list_debounce: Duration::from_millis(100),
        ..FollowOptions::default()
    };
    let follow = peer_client::follow(Arc::new(client), Some(key.into()), options, sink);
    wait_for(&seen, 0, "connected", |e| {
        *e == FollowEvent::Connected { reconnected: false }
    })
    .await;

    // A turn starts and writes: `live`, then an `append` on the same view.
    let ord =
        session_store::start_turn(&rig.vault, key, "claude", &vault, "run-peer", "Second").unwrap();
    rig.live
        .ingest(&json!({"type": "runStarted", "conversationKey": key, "sessionKey": key}));
    // (The stream opened with `live: null`, the chat being idle.)
    let at = wait_for(
        &seen,
        0,
        "live",
        |e| matches!(e, FollowEvent::Event(f) if f.event == "live" && !f.data.is_null()),
    )
    .await;
    match &seen.lock().unwrap()[at] {
        FollowEvent::Event(f) => assert_eq!(f.data["ord"], ord),
        _ => unreachable!(),
    }
    rig.live
        .ingest(&json!({"type": "assistantDelta", "conversationKey": key, "text": "Drafting"}));
    // The text arrives as an `append` on the view held (or, coalesced, in a
    // new `live` frame); either way it isn't a refetch.
    wait_for(&seen, at + 1, "the text", |e| {
        matches!(e, FollowEvent::Event(f) if (f.event == "append" || f.event == "live") && f.data["text"] == "Drafting")
    })
    .await;
    assert!(!seen
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, FollowEvent::Detail(_))));

    // AG5: the other Mac drops the stream. The follow reconnects with the
    // last id it read and the server replays only what it missed: no refetch,
    // and the text written while it was down arrives once.
    let before = seen.lock().unwrap().len();
    rig.live.close_device(&record.device_id);
    rig.live
        .ingest(&json!({"type": "assistantDelta", "conversationKey": key, "text": " more"}));
    let dropped = wait_for(&seen, before, "disconnected", |e| {
        matches!(e, FollowEvent::Disconnected { .. })
    })
    .await;
    let back = wait_for(&seen, dropped, "reconnected", |e| {
        *e == FollowEvent::Connected { reconnected: true }
    })
    .await;
    wait_for(
        &seen,
        back,
        "the text written while down",
        |e| matches!(e, FollowEvent::Event(f) if f.event == "append" && f.data["text"] == " more"),
    )
    .await;
    // Quiet for a moment, so a late refetch or a repeat would show.
    tokio::time::sleep(Duration::from_millis(400)).await; // sleep-ok: nothing else may arrive
    {
        let seen = seen.lock().unwrap();
        assert!(
            !seen
                .iter()
                .skip(before)
                .any(|e| matches!(e, FollowEvent::Detail(_))),
            "a resumed stream refetched: {seen:#?}"
        );
        let texts: Vec<_> = seen
            .iter()
            .skip(before)
            .filter_map(|e| match e {
                FollowEvent::Event(f) if f.event == "append" => f.data["text"].as_str(),
                _ => None,
            })
            .collect();
        assert_eq!(texts, [" more"], "replayed once, no repeat");
    }

    // AG5: when the id can't be honoured (here a new epoch, as after a
    // restart) the server says so and the follow falls back to the refetch.
    let before = seen.lock().unwrap().len();
    rig.live.new_epoch_now();
    rig.live.close_device(&record.device_id);
    let dropped = wait_for(&seen, before, "disconnected again", |e| {
        matches!(e, FollowEvent::Disconnected { .. })
    })
    .await;
    let back = wait_for(&seen, dropped, "reconnected again", |e| {
        *e == FollowEvent::Connected { reconnected: true }
    })
    .await;
    let refetched = wait_for(&seen, back, "refetched", |e| {
        matches!(e, FollowEvent::Detail(_))
    })
    .await;
    match &seen.lock().unwrap()[refetched] {
        FollowEvent::Detail(d) => {
            assert_eq!(d.session.key, key);
            // The running turn's live view, as it stood while the stream was
            // down.
            let live = d.live.as_ref().expect("the running turn's live view");
            assert_eq!(
                (live.ord, live.text.as_str()),
                (ord as u32, "Drafting more")
            );
        }
        _ => unreachable!(),
    }

    // The turn ends: `turn`, then the detail with its answer.
    session_store::append_event(&rig.vault, key, ord, "assistant", "Drafting done").unwrap();
    session_store::finish_turn(&rig.vault, key, ord, "success").unwrap();
    let from = seen.lock().unwrap().len();
    rig.live
        .ingest(&json!({"type": "done", "conversationKey": key}));
    let ended = wait_for(&seen, from, "turn", |e| frame(e, "turn")).await;
    let after = wait_for(&seen, ended, "detail after the turn", |e| {
        matches!(e, FollowEvent::Detail(_))
    })
    .await;
    match &seen.lock().unwrap()[after] {
        FollowEvent::Detail(d) => assert!(d.live.is_none()),
        _ => unreachable!(),
    }

    // Revoked: following ends for good with pairing needed.
    assert!(rig.store.revoke(&record.device_id).unwrap());
    rig.live.close_device(&record.device_id);
    let end = wait_for(&seen, after, "ended", |e| {
        matches!(e, FollowEvent::Ended(_))
    })
    .await;
    match &seen.lock().unwrap()[end] {
        FollowEvent::Ended(e) => assert_eq!(e.state(), PeerState::PairingNeeded),
        _ => unreachable!(),
    }
    let deadline = Instant::now() + NO_HANG;
    while !follow.is_finished() {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    ticker.abort();
}

/// A Cancel that lands during the trade of a Mac already paired here leaves
/// the new pairing standing (no forgetting the Mac); for a new Mac it undoes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_late_cancel_keeps_a_mac_already_paired_and_undoes_a_new_one() {
    for already in [true, false] {
        let rig = rig().await;
        rig.open_machine_window();
        let dir = peer_tunnel::tests::short_dir();
        let ssh = peer_tunnel::tests::fake_ssh(dir.path(), &rig.pair_line());
        let records = tempfile::tempdir().unwrap();
        let machines = Arc::new(Machines::at(records.path()));
        let make = |machines: &Arc<Machines>| PairSetup {
            ssh: ssh.clone().into_os_string(),
            alias: "host-up".into(),
            more_hosts: vec![],
            command: "hollow serve pair --machine --json".into(),
            remote_port: rig.port,
            machines: machines.clone(),
            me: me(),
        };
        let spawner = Arc::new(SshSpawner::with_program(&ssh));
        let tunnels = |cancel: Option<Arc<peer_client::PairCancel>>| -> peer_client::TunnelFor {
            let (socket_dir, spawner) = (dir.path().to_path_buf(), spawner.clone());
            Box::new(move |id, hosts, port| {
                // Cancel arrives after the code was read, before the trade.
                if let Some(c) = &cancel {
                    c.cancel();
                }
                peer_tunnel::Tunnel::new(id, hosts, port, &socket_dir, spawner.clone())
                    .map(Arc::new)
                    .map_err(|e| PeerError::Local(e.to_string()))
            })
        };
        if already {
            peer_client::pair_over_ssh_with(make(&machines), tunnels(None))
                .await
                .unwrap();
            rig.open_machine_window();
        }
        let cancel = Arc::new(peer_client::PairCancel::default());
        let done = peer_client::pair_over_ssh_cancellable(
            make(&machines),
            tunnels(Some(cancel.clone())),
            Some(cancel),
        )
        .await;
        if already {
            assert!(done.is_ok(), "the Mac stays paired");
            assert_eq!(machines.list().unwrap().len(), 1);
        } else {
            assert!(done.is_err());
            assert!(machines.list().unwrap().is_empty());
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pairing_over_ssh_reads_the_code_and_talks_through_a_unix_socket_tunnel() {
    let rig = rig().await;
    rig.open_machine_window();
    let dir = peer_tunnel::tests::short_dir();
    let ssh = peer_tunnel::tests::fake_ssh(dir.path(), &rig.pair_line());
    let records = tempfile::tempdir().unwrap();
    let machines = Arc::new(Machines::at(records.path()));
    let vault = rig.vault.to_string_lossy().into_owned();
    seed_chat(
        &rig.vault,
        "s-20261115-0000abcd",
        "claude",
        &vault,
        &[("Hi", "Hello")],
        "plan",
    );
    let setup = PairSetup {
        ssh: ssh.clone().into_os_string(),
        alias: "host-up".into(),
        more_hosts: vec!["host-down".into(), "host-up".into()],
        command: "hollow serve pair --machine --json".into(),
        remote_port: rig.port,
        machines: machines.clone(),
        me: me(),
    };
    let socket_dir = dir.path().to_path_buf();
    let spawner = Arc::new(SshSpawner::with_program(&ssh));
    let tunnel_for: peer_client::TunnelFor = Box::new(move |id, hosts, port| {
        peer_tunnel::Tunnel::new(id, hosts, port, &socket_dir, spawner)
            .map(Arc::new)
            .map_err(|e| PeerError::Local(e.to_string()))
    });
    let paired = peer_client::pair_over_ssh_with(setup, tunnel_for)
        .await
        .unwrap();
    assert_eq!(paired.record.ssh_hosts, ["host-up", "host-down"]);
    assert_eq!(paired.tunnel.alias().await.as_deref(), Some("host-up"));
    let sock = paired.tunnel.socket().to_path_buf();
    assert!(sock.starts_with(dir.path()));
    // Through the tunnel's socket: the chat list, the identity.
    let page = paired.client.sessions(None, 50).await.unwrap();
    assert!(page.sessions.iter().any(|s| s.key == "s-20261115-0000abcd"));
    // Neither the code nor the token was ever in ssh's arguments.
    let token = crate::peer_machines::read_token(&paired.record.machine_id)
        .unwrap()
        .unwrap();
    let args = std::fs::read_to_string(dir.path().join("ssh.args")).unwrap();
    assert!(!args.contains(CODE) && !args.contains(&token), "{args}");
    let lines: Vec<&str> = args.lines().collect();
    assert!(
        lines[0].contains("BatchMode=yes")
            && lines[0].contains("-T -- host-up hollow serve pair --machine --json;"),
        "{}",
        lines[0]
    );
    assert!(
        lines[1].contains(&format!(
            "-L {}:127.0.0.1:{} -- host-up",
            sock.display(),
            rig.port
        )),
        "{}",
        lines[1]
    );
    assert!(!std::fs::read_to_string(machines.path())
        .unwrap()
        .contains(&token));

    // Its ssh dies: the next call opens the tunnel again.
    let pid = paired.tunnel.pid().await.unwrap();
    // SAFETY: signalling the stand-in ssh this test started.
    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    let deadline = Instant::now() + NO_HANG;
    while paired.tunnel.alias().await.is_some() {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    paired.client.machine().await.unwrap();
    assert_ne!(paired.tunnel.pid().await, Some(pid));

    // Forget: unpaired there through the tunnel, gone here.
    assert!(peer_client::forget_machine(
        &machines,
        &paired.record.machine_id,
        Some(&paired.client)
    )
    .await
    .unwrap());
    assert!(rig.store.read().unwrap().machines.is_empty());
    let pid = paired.tunnel.pid().await.unwrap();
    drop(paired);
    assert!(crate::process_group::test_support::wait_gone(pid as i32, NO_HANG).await);
    assert!(!sock.exists());
}

/// Review M1: a request's timeout doesn't cut the tunnel's opening short. The
/// first alias hangs longer than the request may take; the call still
/// succeeds through the second.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hanging_first_alias_falls_back_to_the_next_within_one_call() {
    let rig = rig().await;
    let (machines, record, _client, _records) = paired(&rig).await;
    let dir = peer_tunnel::tests::short_dir();
    let ssh = peer_tunnel::tests::fake_ssh(dir.path(), "{}");
    let tunnel = Arc::new(
        peer_tunnel::Tunnel::new(
            &record.machine_id,
            vec!["slow".into(), "host-up".into()],
            rig.port,
            dir.path(),
            Arc::new(SshSpawner::with_program(&ssh)),
        )
        .unwrap()
        .with_ready_within(Duration::from_millis(3000)),
    );
    let client = PeerClient::for_record(&record, Dial::Tunnel(tunnel.clone()), machines)
        .unwrap()
        .with_timeout(Duration::from_millis(1500));
    client.sessions(None, 10).await.unwrap();
    assert_eq!(tunnel.alias().await.as_deref(), Some("host-up"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pairing_code_over_an_unreachable_ssh_says_why() {
    let dir = peer_tunnel::tests::short_dir();
    let ssh = peer_tunnel::tests::fake_ssh(dir.path(), "{}");
    let down = peer_client::read_pair_code(&ssh.clone().into_os_string(), "host-down", "true")
        .await
        .unwrap_err();
    assert_eq!(down.state(), PeerState::Offline);
    assert!(down.message().starts_with("This Mac couldn't reach host-down over SSH: ssh: connect to host host-down port 22: Connection refused."), "{down}");
    // An answer that isn't a machine window's.
    let phone =
        peer_tunnel::tests::fake_ssh(dir.path(), r#"{"code":"AAAA-BBBB","expiresAt":null}"#);
    let old = peer_client::read_pair_code(&phone.into_os_string(), "host-up", "true")
        .await
        .unwrap_err();
    assert_eq!(old.state(), PeerState::UpdateNeeded);
    assert!(matches!(
        peer_client::read_pair_code(&ssh.into_os_string(), "-oProxyCommand=x", "true").await,
        Err(PeerError::BadRequest(_))
    ));
}

/// A manual probe against a real Mac (never in the gate): pairs over real ssh
/// with `HOLLOW_PEER_PROBE_ALIAS`, running `HOLLOW_PEER_PROBE_COMMAND` there
/// (a `hollow serve pair --machine --json` with its data folder) and reaching
/// its Hollow on `HOLLOW_PEER_PROBE_PORT`, then lists, reads, tries a refused
/// route and a send to a chat that isn't there, opens the event stream, and
/// forgets the Mac. The token stays in the test's memory secret store; nothing
/// printed carries it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "manual: needs a reachable Mac running hollow serve"]
async fn peer_probe_against_a_real_mac() {
    let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} is unset"));
    let alias = var("HOLLOW_PEER_PROBE_ALIAS");
    let port: u16 = var("HOLLOW_PEER_PROBE_PORT").parse().unwrap();
    let dir = peer_tunnel::tests::short_dir();
    let records = tempfile::tempdir().unwrap();
    let machines = Arc::new(Machines::at(records.path()));
    let began = Instant::now();
    let paired = peer_client::pair_over_ssh_with(
        PairSetup {
            ssh: "ssh".into(),
            alias: alias.clone(),
            more_hosts: vec![],
            command: var("HOLLOW_PEER_PROBE_COMMAND"),
            remote_port: port,
            machines: machines.clone(),
            me: ThisMac {
                machine_id: "m-0000000000c0ffee".into(),
                slug: Some("probe".into()),
                name: "Peer probe".into(),
                version: "probe".into(),
            },
        },
        {
            let socket_dir = dir.path().to_path_buf();
            Box::new(move |id, hosts, port| {
                peer_tunnel::Tunnel::new(id, hosts, port, &socket_dir, Arc::new(SshSpawner::new()))
                    .map(Arc::new)
                    .map_err(|e| PeerError::Local(e.to_string()))
            })
        },
    )
    .await
    .unwrap();
    let r = &paired.record;
    println!(
        "paired in {} ms: machineId {}, name {:?}, device {}, version {:?}, via {:?}, socket {}",
        began.elapsed().as_millis(),
        r.machine_id,
        r.name,
        r.device_id,
        r.last_version,
        paired.tunnel.alias().await,
        paired.tunnel.socket().display()
    );
    let token = crate::peer_machines::read_token(&r.machine_id)
        .unwrap()
        .unwrap();
    assert!(!std::fs::read_to_string(machines.path())
        .unwrap()
        .contains(&token));
    println!("record file holds no token: ok");
    let info = paired.client.machine().await.unwrap();
    println!(
        "GET /v1/machine: headless {}, peerApi {}, capabilities {:?}",
        info.headless, info.peer_api, info.capabilities
    );
    let page = paired.client.sessions(None, 20).await.unwrap();
    println!("GET /v1/sessions: {} chats", page.sessions.len());
    if let Some(first) = page.sessions.first() {
        let d = paired.client.detail(&first.key, None, 5).await.unwrap();
        println!("GET /v1/sessions/{}: {} turns", first.key, d.turns.len());
    }
    let refused = paired
        .client
        .call_json("GET", "/v1/me", None)
        .await
        .unwrap_err();
    println!("GET /v1/me: {refused:?}");
    let missing = paired
        .client
        .send(
            "s-20261115-00000000",
            "probe",
            "00000000-0000-4000-8000-000000000000",
        )
        .await
        .unwrap_err();
    println!("send to a chat that isn't there: {missing:?}");
    let mut stream = paired
        .client
        .events(None, Duration::from_secs(40), None)
        .await
        .unwrap();
    let hello = stream.next().await.unwrap().unwrap();
    println!("events: first frame {} {}", hello.event, hello.data);
    drop(stream);
    assert!(
        peer_client::forget_machine(&machines, &r.machine_id, Some(&paired.client))
            .await
            .unwrap()
    );
    let after = PeerClient::new(Dial::Tunnel(paired.tunnel.clone()), port, &r.name)
        .with_token(token)
        .machine()
        .await
        .unwrap_err();
    println!("after forget, the old token: {:?}", after.state());
    let pid = paired.tunnel.pid().await.unwrap();
    drop(paired);
    assert!(crate::process_group::test_support::wait_gone(pid as i32, NO_HANG).await);
    println!("tunnel ssh {pid} ended on drop: ok");
}
