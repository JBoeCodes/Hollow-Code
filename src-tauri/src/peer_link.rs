//! Peer link reconnect rules (PL1, the link M2 opens to another Mac:
//! `peer_tunnel` and `peer_client`, `docs/phone-api/machines.md`). Each rule
//! and where it lives:
//!
//! - **One retry owner per link.** Only `Tunnel::ensure` opens a link, and it
//!   does so holding the tunnel's state lock: callers that find the link down
//!   wait for that one opening and share it, so a reconnect never runs two
//!   loops. An event stream's reconnect (`peer_client::follow`) and a
//!   command's retry both dial through that same `ensure`.
//! - **Jittered exponential backoff.** The tunnel's waits after every alias
//!   failed (`peer_tunnel::BACKOFF`) and an event stream's reconnect waits
//!   (`peer_client::RECONNECT`) are ceilings; each wait is drawn by a
//!   [`Jitter`] between half the step and the whole step ([`jittered`]), so
//!   links that dropped together don't come back in lockstep. Tests pass a
//!   fixed draw and need no sleeps.
//! - **Liveness.** An event stream silent for `peer_client::WATCHDOG` (the
//!   other Mac pings every 15 s) is closed and reconnected; ssh's own
//!   `ServerAliveInterval` (`remote_session::SSH_OPTS`) ends a tunnel whose
//!   link went quiet, and the next `ensure` sees the dead ssh and opens it
//!   again.
//! - **A command is acknowledged only after its intent is saved, by id.**
//!   A send and a card answer carry a `clientMessageId`; the other Mac saves
//!   the answer under it before it answers and gives a repeat that answer
//!   (`phone_access::routes::send_once`, `remember`). So when the link drops
//!   before the answer, the client sends the same command once more with the
//!   same id ([`retry_command`]): the retry never repeats it. Before that
//!   retry the link the try used is closed (`Tunnel::close_if_opened_before`),
//!   because a quiet drop leaves ssh alive for up to a minute and the retry
//!   would otherwise go over the same dead link.

use crate::peer_client::PeerError;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

/// A command with an id is sent at most this many times: once, and once
/// more after a dropped link.
pub(crate) const COMMAND_TRIES: usize = 2;

/// Draws a reconnect wait's share of its step, in `0.0..=1.0`.
#[derive(Clone)]
pub(crate) struct Jitter(Arc<dyn Fn() -> f64 + Send + Sync>);

impl Jitter {
    pub(crate) fn random() -> Self {
        Self(Arc::new(rand::random::<f64>))
    }

    /// The same draw every time (tests; `1.0` is the whole step).
    #[cfg(test)]
    pub(crate) fn fixed(unit: f64) -> Self {
        Self(Arc::new(move || unit))
    }

    /// A wait of at most `ceiling`.
    pub(crate) fn wait(&self, ceiling: Duration) -> Duration {
        jittered(ceiling, (self.0)())
    }
}

impl Default for Jitter {
    fn default() -> Self {
        Self::random()
    }
}

impl std::fmt::Debug for Jitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Jitter")
    }
}

/// Half of `ceiling` plus `unit` of the other half ("equal jitter"): never
/// under half a step, never over one. A draw outside `0..=1` is clamped, and
/// one that isn't a number counts as the whole step.
pub(crate) fn jittered(ceiling: Duration, unit: f64) -> Duration {
    let unit = if unit.is_nan() {
        1.0
    } else {
        unit.clamp(0.0, 1.0)
    };
    let half = ceiling / 2;
    half + (ceiling - half).mul_f64(unit)
}

/// Whether a command's failure may mean the link dropped before its answer
/// (so the other Mac may have saved it): only then is it sent again.
pub(crate) fn link_dropped(error: &PeerError) -> bool {
    matches!(error, PeerError::Offline(_))
}

/// Run a command that carries an id, sending it once more (the same request,
/// so the same id) when the link dropped before its answer. Before the retry,
/// `reset` gets when the try failed, to close the link it used (one opened
/// later is another caller's fresh one).
/// Every other answer, a refusal included, is returned as it came.
pub(crate) async fn retry_command<T, F, Fut, R, RFut>(
    what: &str,
    mut attempt: F,
    mut reset: R,
) -> Result<T, PeerError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, PeerError>>,
    R: FnMut(tokio::time::Instant) -> RFut,
    RFut: Future<Output = ()>,
{
    let mut tries = 1;
    loop {
        match attempt().await {
            Err(e) if tries < COMMAND_TRIES && link_dropped(&e) => {
                reset(tokio::time::Instant::now()).await;
                crate::launch::elog(&format!(
                    "[peer] {what}: the link dropped before the answer ({}); sending it once more with the same id",
                    e.message()
                ));
                tries += 1;
            }
            out => return out,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{jittered, retry_command, Jitter, COMMAND_TRIES};

    #[test]
    fn a_wait_is_between_half_its_step_and_the_whole_step() {
        let step = Duration::from_secs(10);
        assert_eq!(jittered(step, 0.0), Duration::from_secs(5));
        assert_eq!(jittered(step, 1.0), step);
        assert_eq!(jittered(step, 0.5), Duration::from_millis(7500));
        assert_eq!(jittered(step, -3.0), Duration::from_secs(5));
        assert_eq!(jittered(step, 7.0), step);
        assert_eq!(jittered(step, f64::NAN), step);
        assert_eq!(Jitter::fixed(0.0).wait(step), Duration::from_secs(5));
        for _ in 0..200 {
            let w = Jitter::random().wait(step);
            assert!(w >= Duration::from_secs(5) && w <= step, "{w:?}");
        }
    }

    async fn no_reset(_: tokio::time::Instant) {}

    #[tokio::test]
    async fn a_command_is_sent_again_only_after_a_dropped_link_and_only_once() {
        let tries = std::sync::atomic::AtomicUsize::new(0);
        let count = || tries.load(std::sync::atomic::Ordering::SeqCst);
        let bump = || tries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        // Dropped once, then answered.
        let out = retry_command(
            "a send",
            || async {
                match bump() {
                    0 => Err(PeerError::Offline("dropped".into())),
                    _ => Ok(7),
                }
            },
            no_reset,
        )
        .await;
        assert_eq!(out, Ok(7));
        assert_eq!(count(), 2);
        // Dropped every time: two tries, then the drop.
        tries.store(0, std::sync::atomic::Ordering::SeqCst);
        let out: Result<(), _> = retry_command(
            "a send",
            || async {
                bump();
                Err(PeerError::Offline("dropped".into()))
            },
            no_reset,
        )
        .await;
        assert!(matches!(out, Err(PeerError::Offline(_))));
        assert_eq!(count(), COMMAND_TRIES);
        // A refusal is an answer: never sent again.
        tries.store(0, std::sync::atomic::Ordering::SeqCst);
        let out: Result<(), _> = retry_command(
            "a send",
            || async {
                bump();
                Err(PeerError::Busy("busy".into()))
            },
            no_reset,
        )
        .await;
        assert!(matches!(out, Err(PeerError::Busy(_))));
        assert_eq!(count(), 1);
    }

    use crate::peer_client::{Dial, PeerClient, PeerError};
    use crate::peer_tunnel::tests::{fake_ssh, short_dir};
    use crate::peer_tunnel::{SshSpawner, Tunnel, TunnelError};
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// The tunnel's wait after every alias failed is drawn by its jitter.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_tunnels_backoff_is_jittered() {
        let dir = short_dir();
        let ssh = fake_ssh(dir.path(), "{}");
        let tunnel = Tunnel::new(
            "m-8d21c4be90f17a3e",
            vec!["down".into()],
            1,
            dir.path(),
            Arc::new(SshSpawner::with_program(&ssh)),
        )
        .unwrap()
        .with_jitter(Jitter::fixed(0.0));
        assert!(tunnel.ensure().await.is_err());
        match tunnel.ensure().await {
            // The first step is 2 s; a draw of 0 waits half of it.
            Err(TunnelError::Waiting { retry_in, .. }) => {
                assert!(retry_in <= Duration::from_secs(1), "{retry_in:?}")
            }
            other => panic!("{other:?}"),
        }
    }

    /// A stand-in for the other Mac's send route that saves the intent by
    /// `clientMessageId` before it answers (as `routes::send_once` does) and
    /// gives a repeat the first answer. The first send it takes, it applies
    /// and then drops the link (kills the tunnel's ssh group) before it can
    /// answer: the shape of a link that dies mid-command.
    struct Peer {
        applied: AtomicUsize,
        ids: Mutex<Vec<String>>,
        answers: Mutex<HashMap<String, Value>>,
        ssh_pid: AtomicU32,
    }

    async fn serve(peer: Arc<Peer>) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else {
                    return;
                };
                let peer = peer.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    let end = loop {
                        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break end;
                        }
                        match s.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    };
                    let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
                    let length: usize = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    while buf.len() < end + 4 + length {
                        match s.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let body: Value =
                        serde_json::from_slice(&buf[end + 4..end + 4 + length]).unwrap_or_default();
                    let id = body["clientMessageId"].as_str().unwrap_or("").to_string();
                    peer.ids.lock().unwrap().push(id.clone());
                    let remembered = peer.answers.lock().unwrap().get(&id).cloned();
                    let answer = match remembered {
                        Some(answer) => answer,
                        None => {
                            let n = peer.applied.fetch_add(1, Ordering::SeqCst) + 1;
                            let answer = json!({"accepted": true, "state": "queued", "queueId": n});
                            peer.answers.lock().unwrap().insert(id, answer.clone());
                            if n == 1 {
                                // Saved, then the link goes quiet before the
                                // answer: its ssh is stopped, not ended (alive
                                // to `try_wait`, as a quiet drop leaves it for
                                // up to a minute), and nothing is closed.
                                let pid = peer.ssh_pid.load(Ordering::SeqCst) as i32;
                                unsafe {
                                    libc::kill(-pid, libc::SIGSTOP);
                                }
                                // sleep-ok: holds the quiet connection open past the client's timeout.
                                tokio::time::sleep(Duration::from_secs(30)).await;
                                drop(s);
                                return;
                            }
                            answer
                        }
                    };
                    let body = answer.to_string();
                    let reply = format!(
                        "HTTP/1.1 202 X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = s.write_all(reply.as_bytes()).await;
                });
            }
        });
        (port, task)
    }

    fn openings(dir: &std::path::Path) -> usize {
        std::fs::read_to_string(dir.join("ssh.args"))
            .unwrap_or_default()
            .lines()
            .filter(|l| l.split(' ').any(|a| a == "-N"))
            .count()
    }

    /// PL1's done-when: a link that goes quiet mid-command (ssh still alive)
    /// is closed, reconnects once, and the command is not repeated (the retry
    /// carries the same id and gets the first answer).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_dropped_link_reconnects_once_and_repeats_no_command() {
        let dir = short_dir();
        let ssh = fake_ssh(dir.path(), "{}");
        let peer = Arc::new(Peer {
            applied: AtomicUsize::new(0),
            ids: Mutex::new(Vec::new()),
            answers: Mutex::new(HashMap::new()),
            ssh_pid: AtomicU32::new(0),
        });
        let (port, server) = serve(peer.clone()).await;
        let tunnel = Arc::new(
            Tunnel::new(
                "m-8d21c4be90f17a3e",
                vec!["host-up".into()],
                port,
                dir.path(),
                Arc::new(SshSpawner::with_program(&ssh)),
            )
            .unwrap(),
        );
        tunnel.ensure().await.unwrap();
        let first = tunnel.pid().await.unwrap();
        peer.ssh_pid.store(first, Ordering::SeqCst);
        assert_eq!(openings(dir.path()), 1);

        let client = PeerClient::new(Dial::Tunnel(tunnel.clone()), port, "Mac mini")
            .with_token("t".into())
            .with_timeout(Duration::from_millis(800));
        let id = "6f1c2a8e-0b7d-4c55-9a3e-2d4f8b1c0e77";
        let accepted = client.send("chat-1", "hello", id).await;
        let accepted: Result<_, PeerError> = accepted;
        let accepted = accepted.expect("the send survives the dropped link");
        assert_eq!(accepted.queue_id, 1);
        assert!(
            crate::process_group::test_support::wait_gone(first as i32, Duration::from_secs(5))
                .await,
            "the quiet link's ssh was closed before the retry"
        );
        assert_eq!(peer.applied.load(Ordering::SeqCst), 1, "applied once");
        assert_eq!(
            *peer.ids.lock().unwrap(),
            vec![id.to_string(), id.to_string()]
        );
        assert_eq!(openings(dir.path()), 2, "one reconnect");
        let second = tunnel.pid().await.unwrap();
        assert_ne!(first, second);

        // Several callers after another drop still open the link once: the
        // tunnel's lock is its one retry owner.
        unsafe {
            libc::kill(-(second as i32), libc::SIGKILL);
        }
        assert!(
            crate::process_group::test_support::wait_gone(second as i32, Duration::from_secs(5))
                .await,
            "the killed ssh is gone"
        );
        let (a, b, c) = tokio::join!(tunnel.ensure(), tunnel.ensure(), tunnel.ensure());
        assert!(a.is_ok() && b.is_ok() && c.is_ok());
        assert_eq!(openings(dir.path()), 3, "one more reconnect, not three");
        server.abort();
    }
}
