//! M2 PR 3: the peer commands' hub (`commands::peer_api::Hub`) against this
//! module's rig as the other Mac, through a stand-in ssh and a real
//! Unix-socket tunnel: pairing, one tunnel per Mac, the list stamped with its
//! home, read, send, stop and a card answered there, offline (the Mac marked
//! offline, a send refused with `offline`), a 401 (pairing needed), the
//! event streams turned into `peer-chat-event`, Forget. No real ssh, data
//! folder or Keychain.
use super::peer_tests::{me, MACBOOK};
use super::*;
use crate::commands::peer_api::{Hub, HubSetup, PeerFailureCode, CHAT_EVENT, MACHINES_CHANGED};
use crate::core_host::{Core, RecordingHost};
use crate::peer_client::{FollowOptions, PeerState};
use crate::peer_machines::Machines;
use crate::peer_tunnel::{self, SshSpawner};

pub(super) struct MacBook {
    pub(super) hub: Arc<Hub>,
    pub(super) machines: Arc<Machines>,
    events: RecordingHost,
    dir: tempfile::TempDir,
    _records: tempfile::TempDir,
}

impl MacBook {
    /// A hub pairing through a stand-in ssh that prints `rig`'s pairing
    /// line and forwards its tunnels to `rig`'s port.
    pub(super) fn new(rig: &Rig) -> Self {
        let dir = peer_tunnel::tests::short_dir();
        let ssh = peer_tunnel::tests::fake_ssh(dir.path(), &rig.pair_line());
        let records = tempfile::tempdir().unwrap();
        let machines = Arc::new(Machines::at(records.path()));
        let hub = Hub::new(HubSetup {
            machines: machines.clone(),
            socket_dir: Some(dir.path().to_path_buf()),
            spawner: Arc::new(SshSpawner::with_program(&ssh)),
            ssh: ssh.into_os_string(),
            pair_command: "hollow serve pair --machine --json".into(),
            remote_port: rig.port,
            me: Arc::new(|| Ok::<_, String>(me())),
            dial: None,
            follow: FollowOptions {
                reconnect: vec![Duration::from_millis(50)],
                ..FollowOptions::default()
            },
            suggest: || Some("host-up".into()),
        });
        let events = RecordingHost::default();
        hub.attach(Core::new(events.clone()));
        Self {
            hub,
            machines,
            events,
            dir,
            _records: records,
        }
    }

    /// The tunnels ssh opened so far (`-N` runs), in order.
    fn tunnels_opened(&self) -> usize {
        std::fs::read_to_string(self.dir.path().join("ssh.args"))
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(" -N "))
            .count()
    }

    /// The newest `peer-machines-changed`'s state for `id`.
    fn last_state(&self, id: &str) -> Option<String> {
        self.events.of(MACHINES_CHANGED).last().and_then(|view| {
            view["machines"]
                .as_array()?
                .iter()
                .find(|m| m["machineId"] == id)
                .map(|m| m["state"].as_str().unwrap_or("").to_string())
        })
    }

    async fn book_hosts(&self, id: &str, hosts: &[&str]) {
        self.hub
            .set_hosts(id, hosts.iter().map(|h| h.to_string()).collect())
            .await
            .unwrap();
    }

    async fn state(&self, id: &str) -> (PeerState, Option<String>) {
        let view = self.hub.view().await.unwrap();
        let m = view
            .machines
            .into_iter()
            .find(|m| m.machine_id == id)
            .unwrap();
        (m.state, m.message)
    }
}

/// Wait for a `peer-chat-event` matching `what`.
async fn chat_event(book: &MacBook, what: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + NO_HANG;
    loop {
        if let Some(e) = book.events.of(CHAT_EVENT).into_iter().find(&what) {
            return e;
        }
        assert!(
            Instant::now() < deadline,
            "{:#?}",
            book.events.of(CHAT_EVENT)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_hub_pairs_and_steers_a_mac_through_one_tunnel() {
    let rig = rig().await;
    rig.open_machine_window();
    let book = MacBook::new(&rig);
    let vault = rig.vault.to_string_lossy().into_owned();
    let key = "s-20261115-0000d00d";
    seed_chat(
        &rig.vault,
        key,
        "claude",
        &vault,
        &[("Start the release", "Before I start:")],
        "acceptEdits",
    );

    // Pair: the record, the token in the secret store, online.
    let paired = book.hub.pair("host-up").await.unwrap();
    let id = rig.machine_id();
    assert_eq!(paired.machine_id, id);
    assert_eq!(paired.state, PeerState::Online);
    assert_eq!(paired.ssh_hosts, ["host-up"]);
    assert_eq!(paired.alias.as_deref(), Some("host-up"));
    assert_eq!(book.last_state(&id).as_deref(), Some("online"));
    let tunnel = book
        .hub
        .tunnel(&id)
        .expect("the hub holds the Mac's tunnel");
    let view = book.hub.machines(false).await.unwrap();
    assert_eq!(view.this_mac.as_ref().unwrap().machine_id, MACBOOK);
    assert_eq!(view.suggested_alias.as_deref(), Some("host-up"));
    assert_eq!(view.machines.len(), 1);

    // The list, stamped with its home; the detail.
    let page = book.hub.sessions(&id).await.unwrap();
    assert_eq!(page.home.machine_id, id);
    let row = page.sessions.iter().find(|r| r.session.key == key).unwrap();
    assert_eq!(row.home.name, "Test Mac");
    let json = serde_json::to_value(row).unwrap();
    assert_eq!(json["home"]["machineId"], id.as_str());
    assert_eq!(json["key"], key, "the wire row is flattened beside home");
    let detail = book.hub.detail(&id, key, None, None).await.unwrap();
    assert_eq!(detail.detail.turns.len(), 2);
    assert_eq!(
        detail.detail.permission_mode.as_deref(),
        Some("acceptEdits")
    );

    // A card answered at the chat's home.
    let card = seed_card(&rig.vault, key, 1, "toolu-hub", &three_questions());
    let mut answers = serde_json::Map::new();
    answers.insert("branch".into(), json!("main"));
    answers.insert("checks".into(), json!(["tests"]));
    answers.insert("note".into(), Value::Null);
    let took = book
        .hub
        .answer(
            &id,
            key,
            &card.id,
            "b1a2c3d4-0000-4000-8000-000000000001",
            &answers,
        )
        .await
        .unwrap();
    assert!(took.accepted);
    let cards = crate::interactions::list_for_session(&rig.vault, key).unwrap();
    assert_eq!(cards[0].status, "answered");
    let late = book
        .hub
        .answer(
            &id,
            key,
            &card.id,
            "b1a2c3d4-0000-4000-8000-000000000002",
            &answers,
        )
        .await
        .unwrap_err();
    assert_eq!(late.code, PeerFailureCode::QuestionClosed);
    assert_eq!(late.state, PeerState::Online);
    let mut nested = serde_json::Map::new();
    nested.insert("branch".into(), json!({"a": 1}));
    let refused = book
        .hub
        .answer(
            &id,
            key,
            &card.id,
            "b1a2c3d4-0000-4000-8000-000000000003",
            &nested,
        )
        .await
        .unwrap_err();
    assert_eq!(refused.code, PeerFailureCode::BadRequest);

    // A send to another chat lands in the home's queue; stop reaches it.
    let other = "s-20261115-0000d11d";
    seed_chat(
        &rig.vault,
        other,
        "claude",
        &vault,
        &[("Hi", "Hello")],
        "plan",
    );
    let sent = book
        .hub
        .send(
            &id,
            other,
            "Ship it",
            "b1a2c3d4-0000-4000-8000-000000000004",
        )
        .await
        .unwrap();
    assert!(sent.accepted);
    assert!(rig
        .host
        .sent
        .lock()
        .unwrap()
        .iter()
        .any(|i| i.session_key.as_deref() == Some(other)));
    book.hub.stop(&id, other).await.unwrap();
    assert!(rig.host.stopped.lock().unwrap().iter().any(|k| k == other));
    let blank = book
        .hub
        .send(&id, other, "  ", "b1a2c3d4-0000-4000-8000-000000000005")
        .await
        .unwrap_err();
    assert_eq!(blank.code, PeerFailureCode::BadRequest);
    let bad_id = book.hub.send(&id, other, "x", "a b").await.unwrap_err();
    assert_eq!(bad_id.code, PeerFailureCode::BadRequest);

    // Every call went through the same tunnel and the same ssh.
    assert!(Arc::ptr_eq(&tunnel, &book.hub.tunnel(&id).unwrap()));
    assert_eq!(book.tunnels_opened(), 1);

    // Pairing the same Mac again keeps its one tunnel.
    rig.open_machine_window();
    book.hub.pair("host-up").await.unwrap();
    assert!(Arc::ptr_eq(&tunnel, &book.hub.tunnel(&id).unwrap()));
    assert_eq!(book.machines.list().unwrap().len(), 1);

    // Forget: unpaired there, gone here with its tunnel and its ssh.
    let pid = tunnel.pid().await.expect("the tunnel's ssh runs");
    drop(tunnel);
    assert!(book.hub.forget(&id).await.unwrap());
    assert!(book.hub.tunnel(&id).is_none());
    assert!(rig.store.read().unwrap().machines.is_empty());
    assert!(book.machines.list().unwrap().is_empty());
    assert_eq!(crate::peer_machines::read_token(&id).unwrap(), None);
    assert!(crate::process_group::test_support::wait_gone(pid as i32, NO_HANG).await);
    let gone = book.hub.sessions(&id).await.unwrap_err();
    assert_eq!(gone.code, PeerFailureCode::NotFound);
    // Reading the list makes no tunnel for a Mac no longer paired.
    book.hub.view().await.unwrap();
    assert!(book.hub.tunnel(&id).is_none());
}

/// Review findings 1 and 2a: a Mac keeps its one tunnel, whose port never
/// moves: a call holding a record that names another port goes through the
/// tunnel as it is, and a pairing at another port is refused before any ssh
/// or stream is touched.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_macs_tunnel_port_never_moves_under_a_stale_record_or_another_pairing() {
    let rig = rig().await;
    let other = super::rig().await;
    rig.open_machine_window();
    let book = MacBook::new(&rig);
    let id = book.hub.pair("host-up").await.unwrap().machine_id;
    let tunnel = book.hub.tunnel(&id).unwrap();
    book.hub.watch(&id, None).await.unwrap();
    chat_event(&book, |e| e["kind"] == "connected" && e["key"].is_null()).await;
    let with_port = |port: u16| {
        let mut record = book.machines.list().unwrap().remove(0);
        record.remote_port = port;
        book.machines.upsert(record).unwrap();
    };

    // A record that says another port (as an older read would).
    with_port(other.port);
    book.hub.sessions(&id).await.unwrap();
    assert_eq!(tunnel.remote_port(), rig.port);
    assert!(Arc::ptr_eq(&tunnel, &book.hub.tunnel(&id).unwrap()));
    with_port(rig.port);

    // Pairing it at another port: refused before ssh opens a pairing
    // window there, nothing touched.
    let ssh_runs = || {
        std::fs::read_to_string(book.dir.path().join("ssh.args"))
            .unwrap()
            .lines()
            .count()
    };
    let before = ssh_runs();
    let refused = book.hub.pair_at("host-up", other.port).await.unwrap_err();
    assert_eq!(refused.code, PeerFailureCode::Local, "{refused:?}");
    assert_eq!(
        refused.message,
        format!(
            "host-up is paired here through port {}. Forget that Mac in Settings first to pair it through port {}.",
            rig.port, other.port
        )
    );
    assert_eq!(ssh_runs(), before, "no ssh ran");
    assert!(Arc::ptr_eq(&tunnel, &book.hub.tunnel(&id).unwrap()));
    assert_eq!(tunnel.remote_port(), rig.port);
    assert_eq!(book.tunnels_opened(), 1);
    assert_eq!(book.hub.watched(&id), (true, None));
    book.hub.sessions(&id).await.unwrap();
    assert!(tunnel.socket().exists());
    assert_eq!(book.state(&id).await.0, PeerState::Online);
}

/// Delta review 1: pairing again is the person's own try, never held back
/// by the backoff an unreachable alias left.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pairing_again_skips_the_backoff_of_earlier_failures() {
    let rig = rig().await;
    rig.open_machine_window();
    let book = MacBook::new(&rig);
    let id = book.hub.pair("host-up").await.unwrap().machine_id;
    book.book_hosts(&id, &["box-down"]).await;
    book.hub.tunnel(&id).unwrap().close().await;
    assert!(book.hub.sessions(&id).await.is_err());
    let waiting = book.hub.sessions(&id).await.unwrap_err();
    assert!(
        waiting.message.contains("tries again"),
        "{}",
        waiting.message
    );
    rig.open_machine_window();
    let paired = book.hub.pair("host-up").await.unwrap();
    assert_eq!(paired.state, PeerState::Online);
    assert_eq!(paired.ssh_hosts, ["host-up"]);
}

/// Round 3, finding 2: the backoff is cleared right before the pairing's
/// own opening, so a failed reconnect while the code was read (which sets a
/// new wait) doesn't hold the pairing back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reconnect_failing_while_the_code_is_read_doesnt_block_the_pairing() {
    let rig = rig().await;
    rig.open_machine_window();
    let book = MacBook::new(&rig);
    let id = book.hub.pair("host-lag").await.unwrap().machine_id;
    book.book_hosts(&id, &["box-down"]).await;
    let tunnel = book.hub.tunnel(&id).unwrap();
    tunnel.close().await;
    assert!(book.hub.sessions(&id).await.is_err());
    rig.open_machine_window();
    let pairing = tokio::spawn({
        let hub = book.hub.clone();
        async move { hub.pair("host-lag").await }
    });
    // While ssh reads the code (0.8 s), a reconnect tries the dead alias and
    // fails, setting a new wait.
    tokio::time::sleep(Duration::from_millis(300)).await;
    tunnel.retry_now();
    let during = book.hub.sessions(&id).await;
    assert!(during.is_err(), "{during:?} {:?}", tunnel.hosts());
    let paired = pairing.await.unwrap().unwrap();
    assert_eq!(paired.state, PeerState::Online);
    assert_eq!(paired.ssh_hosts, ["host-lag"]);
    book.hub.sessions(&id).await.unwrap();
}

/// Cancel on the Pairing step: while ssh reads the code (0.8 s) the pairing
/// is cancelled; it ends with Cancel's sentence and keeps no record, no
/// token and no tunnel, and a second pairing then works.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_a_pairing_keeps_nothing_and_frees_the_next_one() {
    let rig = rig().await;
    rig.open_machine_window();
    let book = MacBook::new(&rig);
    assert!(!book.hub.cancel_pairing(), "nothing runs yet");
    let pairing = tokio::spawn({
        let hub = book.hub.clone();
        async move { hub.pair("host-lag").await }
    });
    // sleep-ok: let the stand-in ssh start reading the code
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(book.hub.cancel_pairing());
    let failed = pairing.await.unwrap().unwrap_err();
    assert!(failed.message.contains("cancelled"), "{}", failed.message);
    assert!(book.machines.list().unwrap().is_empty());
    assert!(book.hub.tunnel(&rig.machine_id()).is_none());
    assert!(!book.hub.cancel_pairing(), "the attempt is over");
    // Free again: the same Mac pairs.
    let paired = book.hub.pair("host-up").await.unwrap();
    assert_eq!(paired.state, PeerState::Online);
}

/// A typed address is checked before ssh runs: junk and loopback are refused
/// with no ssh call at all, and `host:port` reaches that Hollow port.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_typed_address_is_checked_first_and_its_port_is_used() {
    let rig = rig().await;
    rig.open_machine_window();
    let book = MacBook::new(&rig);
    for junk in [
        "https://host-up",
        "127.0.0.1",
        "localhost:47680",
        "host-up:0",
    ] {
        let refused = book.hub.pair(junk).await.unwrap_err();
        assert_eq!(refused.code, PeerFailureCode::BadRequest, "{junk}");
    }
    assert!(
        !book.dir.path().join("ssh.args").exists(),
        "no ssh ran for a refused address"
    );
    let paired = book
        .hub
        .pair(&format!("host-up:{}", rig.port))
        .await
        .unwrap();
    assert_eq!(paired.remote_port, rig.port);
    assert_eq!(paired.ssh_hosts, ["host-up"]);
}

/// Review 2c: a pairing again restarts the streams that ran, with the new
/// token.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pairing_again_restarts_the_streams_that_ran() {
    let rig = rig().await;
    rig.open_machine_window();
    let book = MacBook::new(&rig);
    let id = book.hub.pair("host-up").await.unwrap().machine_id;
    let vault = rig.vault.to_string_lossy().into_owned();
    let key = "s-20261115-0000d66d";
    seed_chat(
        &rig.vault,
        key,
        "claude",
        &vault,
        &[("Hi", "Hello")],
        "plan",
    );
    book.hub.watch(&id, None).await.unwrap();
    book.hub.watch(&id, Some(key.into())).await.unwrap();
    rig.open_machine_window();
    book.hub.pair("host-up").await.unwrap();
    assert_eq!(book.hub.watched(&id), (true, Some(key.to_string())));
    // The old device is gone there; the restarted streams use the new one
    // and connect, and nothing the old ones said late changes the state.
    let mark = book.events.of(CHAT_EVENT).len();
    let deadline = Instant::now() + NO_HANG;
    loop {
        let after: Vec<Value> = book.events.of(CHAT_EVENT).into_iter().skip(mark).collect();
        let connected = |k: &Value| {
            after
                .iter()
                .any(|e| e["kind"] == "connected" && &e["key"] == k)
        };
        if connected(&Value::Null) && connected(&json!(key)) {
            assert!(after.iter().all(|e| e["kind"] != "ended"), "{after:#?}");
            break;
        }
        assert!(Instant::now() < deadline, "{after:#?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(book.state(&id).await.0, PeerState::Online);
    assert_eq!(book.hub.watched(&id), (true, Some(key.to_string())));
    book.hub.sessions(&id).await.unwrap();
}

/// Review finding 2: pairing again with a wrong code leaves the working
/// pairing as it was: its streams, its client and its aliases.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_pairing_again_leaves_the_working_one_alone() {
    let rig = rig().await;
    rig.open_machine_window();
    let book = MacBook::new(&rig);
    let id = book.hub.pair("host-up").await.unwrap().machine_id;
    book.book_hosts(&id, &["host-up", "host-up-2"]).await;
    book.hub.watch(&id, None).await.unwrap();
    chat_event(&book, |e| e["kind"] == "connected" && e["key"].is_null()).await;
    // The window there is open with another code than the one ssh prints.
    rig.ctx.machine_pairing.open_with("OTHERC0D", Utc::now());
    let wrong = book.hub.pair("host-up").await.unwrap_err();
    assert_eq!(wrong.code, PeerFailureCode::WrongCode, "{wrong:?}");
    assert_eq!(
        book.hub.watched(&id),
        (true, None),
        "the list stream runs on"
    );
    assert_eq!(
        book.machines.list().unwrap()[0].ssh_hosts,
        ["host-up", "host-up-2"]
    );
    book.hub.sessions(&id).await.unwrap();
    assert_eq!(book.state(&id).await.0, PeerState::Online);
    assert_eq!(book.tunnels_opened(), 1, "the same ssh throughout");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_offline_mac_is_marked_offline_and_a_send_is_refused_with_offline() {
    let rig = rig().await;
    rig.open_machine_window();
    let book = MacBook::new(&rig);
    let id = book.hub.pair("host-up").await.unwrap().machine_id;
    // Its only alias is unreachable now, and the running tunnel closes.
    book.hub
        .set_hosts(&id, vec!["box-down".into()])
        .await
        .unwrap();
    book.hub.tunnel(&id).unwrap().close().await;

    let failed = book.hub.sessions(&id).await.unwrap_err();
    assert_eq!(failed.code, PeerFailureCode::Offline, "{failed:?}");
    assert_eq!(failed.state, PeerState::Offline);
    assert!(failed.message.contains("box-down"), "{}", failed.message);
    let (state, message) = book.state(&id).await;
    assert_eq!(state, PeerState::Offline);
    assert!(message.unwrap().contains("box-down"));
    assert_eq!(book.last_state(&id).as_deref(), Some("offline"));

    let refused = book
        .hub
        .send(
            &id,
            "s-20261115-0000d22d",
            "Ship it",
            "c1a2c3d4-0000-4000-8000-000000000001",
        )
        .await
        .unwrap_err();
    assert_eq!(refused.code, PeerFailureCode::Offline, "{refused:?}");
    assert_eq!(refused.state, PeerState::Offline);
    let stop = book.hub.stop(&id, "s-20261115-0000d22d").await.unwrap_err();
    assert_eq!(stop.code, PeerFailureCode::Offline);
    // The serialized error the webview gets.
    let wire = serde_json::to_value(&refused).unwrap();
    assert_eq!(wire["code"], "offline");
    assert_eq!(wire["state"], "offline");

    // Back in reach: a reachable alias (it skips the wait) and it is online.
    book.hub
        .set_hosts(&id, vec!["host-up".into()])
        .await
        .unwrap();
    book.hub.sessions(&id).await.unwrap();
    assert_eq!(book.state(&id).await, (PeerState::Online, None));
    assert_eq!(book.last_state(&id).as_deref(), Some("online"));
    // A bad alias is refused before anything is saved.
    let bad = book
        .hub
        .set_hosts(&id, vec!["-oProxyCommand=x".into()])
        .await
        .unwrap_err();
    assert_eq!(bad.code, PeerFailureCode::BadRequest);
    assert_eq!(book.machines.list().unwrap()[0].ssh_hosts, ["host-up"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_401_flips_the_mac_to_pairing_needed_until_it_pairs_again() {
    let rig = rig().await;
    rig.open_machine_window();
    let book = MacBook::new(&rig);
    let paired = book.hub.pair("host-up").await.unwrap();
    let id = paired.machine_id.clone();
    let device = book.machines.list().unwrap()[0].device_id.clone();
    assert!(rig.store.revoke(&device).unwrap());

    let failed = book.hub.sessions(&id).await.unwrap_err();
    assert_eq!(failed.code, PeerFailureCode::PairingNeeded, "{failed:?}");
    assert_eq!(failed.state, PeerState::PairingNeeded);
    assert_eq!(book.state(&id).await.0, PeerState::PairingNeeded);
    assert_eq!(book.last_state(&id).as_deref(), Some("pairing-needed"));
    assert!(book.machines.list().unwrap()[0].token_revoked);
    let refused = book
        .hub
        .send(
            &id,
            "s-20261115-0000d33d",
            "Hi",
            "d1a2c3d4-0000-4000-8000-000000000001",
        )
        .await
        .unwrap_err();
    assert_eq!(refused.code, PeerFailureCode::PairingNeeded);
    // A watch hears it too and ends.
    book.hub.watch(&id, None).await.unwrap();
    let ended = chat_event(&book, |e| e["kind"] == "ended").await;
    assert_eq!(ended["data"]["code"], "pairing-needed");
    assert!(ended["key"].is_null());

    // Pairing again clears it; the new token is the one used.
    rig.open_machine_window();
    book.hub.pair("host-up").await.unwrap();
    book.hub.sessions(&id).await.unwrap();
    assert_eq!(book.state(&id).await.0, PeerState::Online);
    assert!(!book.machines.list().unwrap()[0].token_revoked);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watching_turns_the_streams_into_peer_chat_events_and_unwatch_stops_them() {
    let rig = rig().await;
    rig.open_machine_window();
    let book = MacBook::new(&rig);
    let id = book.hub.pair("host-up").await.unwrap().machine_id;
    let vault = rig.vault.to_string_lossy().into_owned();
    let key = "s-20261115-0000d44d";
    seed_chat(
        &rig.vault,
        key,
        "claude",
        &vault,
        &[("Hi", "Hello")],
        "plan",
    );

    book.hub.watch(&id, None).await.unwrap();
    book.hub.watch(&id, Some(key.into())).await.unwrap();
    assert_eq!(book.hub.watched(&id), (true, Some(key.to_string())));
    let connected = chat_event(&book, |e| e["kind"] == "connected" && e["key"] == key).await;
    assert_eq!(connected["machineId"], id.as_str());
    assert_eq!(connected["data"]["reconnected"], false);
    chat_event(&book, |e| e["kind"] == "connected" && e["key"].is_null()).await;
    // The chat's stream opens with its live view (none) and its state.
    let state = chat_event(&book, |e| e["kind"] == "state" && e["key"] == key).await;
    assert_eq!(state["data"]["state"], "idle");
    chat_event(&book, |e| e["kind"] == "live" && e["data"].is_null()).await;

    // A new chat replaces the one watched; unwatching stops each.
    let other = "s-20261115-0000d55d";
    seed_chat(
        &rig.vault,
        other,
        "claude",
        &vault,
        &[("Hi", "Hello")],
        "plan",
    );
    book.hub.watch(&id, Some(other.into())).await.unwrap();
    assert_eq!(book.hub.watched(&id), (true, Some(other.to_string())));
    book.hub.unwatch(&id, Some(key));
    assert_eq!(book.hub.watched(&id).1.as_deref(), Some(other));
    book.hub.unwatch(&id, Some(other));
    book.hub.unwatch(&id, None);
    assert_eq!(book.hub.watched(&id), (false, None));

    // With nothing watched and nothing used, the idle sweep closes the
    // tunnel; the next call opens it again.
    book.hub.close_idle(Duration::ZERO).await;
    assert!(book.hub.tunnel(&id).unwrap().alias().await.is_none());
    book.hub.sessions(&id).await.unwrap();
    assert_eq!(book.tunnels_opened(), 2);
}

/// Plan 2.7 and the peer contract: the hub keeps nothing about a remote
/// chat (no Sessions DB, no `interactions` row, no vault, no file but the
/// record), and no peer code calls the two routes a machine token is
/// refused (`/v1/me`, `/v1/chat-controls`).
#[test]
fn the_peer_code_keeps_nothing_and_never_calls_a_refused_route() {
    let api = include_str!("../commands/peer_api.rs");
    let client = include_str!("../peer_client.rs");
    let client = &client[..client.find("#[cfg(test)]\nmod tests").unwrap()];
    for (name, text) in [("peer_api.rs", api), ("peer_client.rs", client)] {
        for refused in [concat!("\"/v1/", "me\""), concat!("chat", "-controls")] {
            assert!(!text.contains(refused), "{name} names {refused}");
        }
    }
    for kept in [
        concat!("session", "_store"),
        concat!("interactions", "::"),
        concat!("std::fs", "::write"),
        concat!("vault", "_path"),
        concat!("state", "_file"),
    ] {
        assert!(!api.contains(kept), "peer_api.rs names {kept}");
    }
}
