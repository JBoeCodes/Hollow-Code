//! SE9: one initialized Codex app-server per compatible account environment.
//! Each turn keeps a small inherited-fd relay so the existing process-group,
//! History and cancellation lifecycle still owns that turn and its tagged tools.
//! No prompt is replayed after a server failure. No grant enters server env.

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::os::fd::{AsRawFd, FromRawFd};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::process::Command;
use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};

const RELAY_FD: i32 = 3;
const MAX_CLIENTS: usize = 64;
const MAX_LINE: usize = crate::process_group::STDOUT_LINE_MAX;
const QUEUED_BYTES: usize = 64 * 1024 * 1024;
const IDLE: Duration = Duration::from_secs(60);
const STOP_WAIT: Duration = Duration::from_secs(1);
const RPC_WAIT: Duration = Duration::from_secs(30);
struct PoolEntry {
    id: u64,
    tx: mpsc::Sender<Join>,
}
type Pools = HashMap<[u8; 32], PoolEntry>;
static POOLS: OnceLock<Mutex<Pools>> = OnceLock::new();
static NEXT_POOL_ID: AtomicU64 = AtomicU64::new(1);

struct Join {
    socket: UnixStream,
    config: Map<String, Value>,
}

/// All process-specific overrides become immutable thread config. Header
/// values remain in memory, never argv or the shared process environment.
fn thread_config(cmd: &Command) -> Option<Map<String, Value>> {
    let args: Vec<_> = cmd.as_std().get_args().map(|s| s.to_str()).collect();
    if args.last().copied().flatten() != Some("app-server") {
        return None;
    }
    let env: HashMap<_, _> = cmd
        .as_std()
        .get_envs()
        .filter_map(|(k, v)| Some((k.to_str()?.to_string(), v?.to_str()?.to_string())))
        .collect();
    let mut config = Map::new();
    let mut i = 0;
    while i + 1 < args.len() {
        match args[i]? {
            "-c" | "--config" => {
                let (key, value) = args.get(i + 1).copied().flatten()?.split_once('=')?;
                let parsed: toml::Value = format!("value={value}").parse().ok()?;
                config.insert(key.into(), serde_json::to_value(parsed.get("value")?).ok()?);
                i += 2;
            }
            "--disable"
                if matches!(
                    args.get(i + 1).copied().flatten(),
                    Some("apps" | "plugins" | "remote_plugin")
                ) =>
            {
                i += 2
            }
            _ => return None,
        }
    }
    if let Some(servers) = config.get_mut("mcp_servers") {
        for entry in servers.as_object_mut()?.values_mut() {
            let entry = entry.as_object_mut()?;
            if entry.get("enabled") == Some(&Value::Bool(false)) {
                continue;
            }
            // Stdio servers retain the ordinary, private per-turn process.
            // A thread may never launch a server under another turn's identity.
            if entry.contains_key("command") || !entry.contains_key("url") {
                return None;
            }
            if let Some(headers) = entry.remove("env_http_headers") {
                let mut resolved = Map::new();
                for (header, name) in headers.as_object()? {
                    resolved.insert(header.clone(), json!(env.get(name.as_str()?)?));
                }
                entry.insert("http_headers".into(), Value::Object(resolved));
            }
        }
    }
    let shell: Map<_, _> = env
        .iter()
        .filter(|(k, _)| {
            (k.starts_with("HOLLOW_") || matches!(k.as_str(), "TMPDIR" | "TEMP" | "TMP"))
                && !k.starts_with(crate::run_auth::CODEX_HEADER_ENV_PREFIX)
        })
        .map(|(k, v)| (k.clone(), json!(v)))
        .collect();
    // The shell tag follows the thread, never the shared server's ancestry.
    if !shell.contains_key(crate::process_group::RUN_TAG_ENV) {
        return None;
    }
    config.insert("shell_environment_policy.set".into(), Value::Object(shell));
    Some(config)
}

fn server_command(cmd: &Command) -> (Command, [u8; 32]) {
    let mut env: BTreeMap<_, _> = std::env::vars_os().collect();
    for (key, value) in cmd.as_std().get_envs() {
        match value {
            Some(v) => {
                env.insert(key.into(), v.into());
            }
            None => {
                env.remove(key);
            }
        }
    }
    env.retain(|k, _| !k.to_string_lossy().starts_with("HOLLOW_") || k == "HOLLOW_SELF_HOSTED_KEY");
    // Chat scratch paths belong to thread shells. They must neither split
    // the account pool nor disappear under an otherwise live shared server.
    for key in ["TMPDIR", "TEMP", "TMP"] {
        env.insert(key.into(), "/tmp".into());
    }
    let mut hash = Sha256::new();
    if let Ok(meta) = std::fs::metadata(cmd.as_std().get_program()) {
        hash.update(meta.len().to_le_bytes());
        if let Ok(at) = meta.modified().and_then(|at| {
            at.duration_since(std::time::UNIX_EPOCH)
                .map_err(std::io::Error::other)
        }) {
            hash.update(at.as_nanos().to_le_bytes());
        }
    }
    hash.update(cmd.as_std().get_program().as_encoded_bytes());
    for (key, value) in &env {
        hash.update((key.as_encoded_bytes().len() as u64).to_le_bytes());
        hash.update(key.as_encoded_bytes());
        hash.update((value.as_encoded_bytes().len() as u64).to_le_bytes());
        hash.update(value.as_encoded_bytes());
    }
    let mut server = Command::new(cmd.as_std().get_program());
    server
        .env_clear()
        .envs(env)
        .current_dir("/")
        .args([
            "--disable",
            "apps",
            "--disable",
            "plugins",
            "--disable",
            "remote_plugin",
            "app-server",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    crate::process_group::isolate(&mut server);
    (server, hash.finalize().into())
}

/// Only local GUI-owned, HTTP-only interactive turns reach this seam. A
/// missing runtime, non-UTF8 option or unsupported config keeps the old path.
pub(crate) fn relay(cmd: &Command) -> Option<(Command, std::os::unix::net::UnixStream)> {
    let runtime = tokio::runtime::Handle::try_current().ok()?;
    let config = thread_config(cmd)?;
    let exe = std::env::current_exe().ok()?;
    let (parent, child) = std::os::unix::net::UnixStream::pair().ok()?;
    parent.set_nonblocking(true).ok()?;
    let parent = UnixStream::from_std(parent).ok()?;
    let (server, key) = server_command(cmd);
    let mut pools = POOLS.get_or_init(Default::default).lock().ok()?;
    pools.retain(|_, entry| !entry.tx.is_closed());
    let entry = pools.entry(key).or_insert_with(|| {
        let (tx, rx) = mpsc::channel(MAX_CLIENTS);
        let id = NEXT_POOL_ID.fetch_add(1, Ordering::Relaxed);
        runtime.spawn(serve_pooled(server, rx, Some((key, id))));
        PoolEntry { id, tx }
    });
    entry
        .tx
        .try_send(Join {
            socket: parent,
            config,
        })
        .ok()?;
    let mut relay = Command::new(exe);
    relay
        .arg("codex-relay")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some(cwd) = cmd.as_std().get_current_dir() {
        relay.current_dir(cwd);
    }
    for (key, value) in cmd.as_std().get_envs() {
        match value {
            Some(v) => {
                relay.env(key, v);
            }
            None => {
                relay.env_remove(key);
            }
        }
    }
    if let Some(tag) = cmd
        .as_std()
        .get_envs()
        .find(|(k, _)| *k == crate::process_group::RUN_TAG_ENV)
        .and_then(|(_, v)| v)
        .and_then(|v| v.to_str())
    {
        if let Some(attempt) = crate::shared_mcp_host::attempt(tag) {
            crate::shared_mcp_host::retain(&mut relay, attempt);
        }
    }
    relay.process_group(0);
    let fd = child.as_raw_fd();
    // SAFETY: dup2/fcntl are async-signal-safe. The caller holds the returned
    // socket through spawn and drops it immediately afterwards, so the host
    // sees EOF when the actual relay closes, with no parent-side duplicate.
    unsafe {
        relay.pre_exec(move || {
            if libc::dup2(fd, RELAY_FD) == -1 || libc::fcntl(RELAY_FD, libc::F_SETFD, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Some((relay, child))
}

/// Hidden child entry point. An inherited connected fd is the whole channel;
/// there is no discoverable listener or token file. SIGTERM requests a scoped
/// interrupt and waits for the host's bounded retirement before exiting.
pub(crate) async fn run_relay() -> std::io::Result<()> {
    // SAFETY: the parent passes this fd only to this hidden command.
    let socket = unsafe { std::os::unix::net::UnixStream::from_raw_fd(RELAY_FD) };
    socket.set_nonblocking(true)?;
    let socket = UnixStream::from_std(socket)?;
    let (mut read, mut write) = socket.into_split();
    let mut signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let input = tokio::spawn(async move {
        let mut input = tokio::io::stdin();
        tokio::io::copy(&mut input, &mut write).await?;
        Ok::<_, std::io::Error>(())
    });
    let output = async {
        let mut output = tokio::io::stdout();
        tokio::io::copy(&mut read, &mut output).await?;
        output.flush().await
    };
    let mut input = input;
    tokio::pin!(output);
    tokio::select! {
        result = &mut output => { input.abort(); return result; },
        _ = signal.recv() => {},
        _ = &mut input => {},
    }
    if !input.is_finished() {
        input.abort();
        let _ = input.await;
    }
    // Dropping the writer half is the private Stop signal. Continue draining
    // stdout until the host has interrupted this turn or retired its server.
    output.await
}

struct Frame {
    line: String,
    _budget: OwnedSemaphorePermit,
}
enum Event {
    Client(u64, Value, OwnedSemaphorePermit),
    Gone(u64),
    Server(Value, OwnedSemaphorePermit),
    Dead,
}
struct Client {
    out: Option<mpsc::Sender<Frame>>,
    reader: tokio::task::JoinHandle<()>,
    writer: Option<tokio::task::JoinHandle<()>>,
    config: Map<String, Value>,
    thread: Option<String>,
    turn: Option<String>,
    started: bool,
    done: bool,
    closing: Option<Instant>,
    interrupt_sent: bool,
}
impl Drop for Client {
    fn drop(&mut self) {
        self.reader.abort();
        if let Some(writer) = &self.writer {
            writer.abort();
        }
    }
}
struct Pending {
    client: u64,
    local: Value,
    method: String,
    at: Instant,
}
struct Routes {
    clients: HashMap<u64, Client>,
    pending: HashMap<u64, Pending>,
    threads: HashMap<String, u64>,
    requests: HashMap<String, u64>,
    next: u64,
    bytes: Arc<Semaphore>,
}
impl Routes {
    /// Evict a single client whose outbound queue is full or closed without
    /// disrupting sibling turns. Keeping the entry in `closing` lets
    /// `closing_writes` interrupt any active turn and unsubscribe its thread.
    fn disconnect_client(&mut self, id: u64) {
        if let Some(client) = self.clients.get_mut(&id) {
            client.out = None;
            client.reader.abort();
            if let Some(writer) = client.writer.take() {
                writer.abort();
            }
            client.closing.get_or_insert_with(Instant::now);
        }
        self.requests.retain(|_, client| *client != id);
    }

    fn send(&mut self, client: u64, value: &Value) {
        let Some(out) = self.clients.get(&client).and_then(|c| c.out.as_ref()) else {
            return;
        };
        let line = value.to_string();
        let sent = u32::try_from(line.len())
            .ok()
            .and_then(|len| self.bytes.clone().try_acquire_many_owned(len).ok())
            .is_some_and(|budget| {
                out.try_send(Frame {
                    line,
                    _budget: budget,
                })
                .is_ok()
            });
        if !sent {
            self.disconnect_client(client);
        }
    }
    fn rpc(&mut self, client: u64, local: Value, method: &str, params: Value) -> Value {
        self.next += 1;
        self.pending.insert(
            self.next,
            Pending {
                client,
                local,
                method: method.into(),
                at: Instant::now(),
            },
        );
        json!({"jsonrpc":"2.0", "id":self.next, "method":method, "params":params})
    }
    fn client(&mut self, id: u64, value: Value) -> Result<Option<Value>, ()> {
        let method = value.get("method").and_then(Value::as_str);
        let local = value.get("id").cloned();
        let Some(method) = method else {
            let key = local.ok_or(())?.to_string();
            if self.requests.remove(&key) != Some(id) {
                return Err(());
            }
            return Ok(Some(value));
        };
        if method == "initialized" {
            return Ok(None);
        }
        let local = local.ok_or(())?;
        if method == "initialize" {
            self.send(
                id,
                &json!({"jsonrpc":"2.0", "id":local, "result":{"userAgent":"hollow-shared"}}),
            );
            return Ok(None);
        }
        let mut params = value.get("params").cloned().unwrap_or(json!({}));
        if matches!(method, "thread/start" | "thread/resume") {
            let client = self.clients.get_mut(&id).ok_or(())?;
            if client.thread.is_some()
                || self
                    .pending
                    .values()
                    .any(|p| p.client == id && p.method.starts_with("thread/"))
            {
                return Err(());
            }
            if let Some(thread) = params.get("threadId").and_then(Value::as_str) {
                if self.threads.contains_key(thread) {
                    return Err(());
                }
            }
            params
                .as_object_mut()
                .ok_or(())?
                .insert("config".into(), Value::Object(client.config.clone()));
        } else if matches!(method, "turn/start" | "turn/steer" | "turn/interrupt") {
            let client = self.clients.get_mut(&id).ok_or(())?;
            if params.get("threadId").and_then(Value::as_str) != client.thread.as_deref()
                || client.done
                || client.closing.is_some()
            {
                return Err(());
            }
            if method == "turn/start" {
                if client.started {
                    return Err(());
                }
                client.started = true;
            }
            if method != "turn/start"
                && params
                    .get(if method == "turn/steer" {
                        "expectedTurnId"
                    } else {
                        "turnId"
                    })
                    .and_then(Value::as_str)
                    != client.turn.as_deref()
            {
                return Err(());
            }
        } else {
            return Err(());
        }
        Ok(Some(self.rpc(id, local, method, params)))
    }
    fn server(&mut self, mut value: Value) -> Result<(), ()> {
        if value.get("method").is_none() {
            let Some(id) = value.get("id").and_then(Value::as_u64) else {
                return Ok(());
            };
            let Some(pending) = self.pending.remove(&id) else {
                return Ok(());
            };
            if pending.client == 0 {
                return if value.get("error").is_some() {
                    Err(())
                } else {
                    Ok(())
                };
            }
            let Some(client) = self.clients.get_mut(&pending.client) else {
                return Ok(());
            };
            if matches!(pending.method.as_str(), "thread/start" | "thread/resume") {
                if let Some(thread) = value.pointer("/result/thread/id").and_then(Value::as_str) {
                    if self.threads.contains_key(thread) {
                        return Err(());
                    }
                    self.threads.insert(thread.into(), pending.client);
                    client.thread = Some(thread.into());
                }
            }
            if pending.method == "turn/start" {
                if value.get("error").is_some_and(|error| !error.is_null()) {
                    client.done = true;
                }
                if let Some(turn) = value.pointer("/result/turn/id").and_then(Value::as_str) {
                    if client.turn.as_deref().is_some_and(|known| known != turn) {
                        return Err(());
                    }
                    client.turn = Some(turn.into());
                }
            }
            value["id"] = pending.local;
            self.send(pending.client, &value);
            return Ok(());
        }
        let thread = value.pointer("/params/threadId").and_then(Value::as_str);
        // This one notification describes the shared account, not a thread.
        // Preserve quota readings without broadcasting any conversation data.
        if value["method"] == "account/rateLimits/updated"
            && value
                .pointer("/params/rateLimits")
                .is_some_and(Value::is_object)
        {
            let ids: Vec<u64> = self.clients.keys().copied().collect();
            for id in ids {
                self.send(id, &value);
            }
            return Ok(());
        }
        let Some(id) = thread.and_then(|t| self.threads.get(t)).copied() else {
            // Account/global notifications are never broadcast between turns.
            return if value.get("id").is_some() {
                Err(())
            } else {
                Ok(())
            };
        };
        let client = self.clients.get_mut(&id).ok_or(())?;
        if value["method"] == "turn/started" && client.started {
            if let Some(turn) = value.pointer("/params/turn/id").and_then(Value::as_str) {
                if client.turn.as_deref().is_some_and(|known| known != turn) {
                    return Ok(());
                }
                client.turn = Some(turn.into());
            }
        }
        if value["method"] == "turn/completed"
            && value.pointer("/params/turn/id").and_then(Value::as_str) == client.turn.as_deref()
        {
            client.done = true;
        }
        if let Some(request) = value.get("id").filter(|_| client.out.is_some()) {
            self.requests.insert(request.to_string(), id);
        }
        self.send(id, &value);
        Ok(())
    }

    fn closing_writes(&mut self) -> Vec<Value> {
        let ids: Vec<_> = self
            .clients
            .iter()
            .filter(|(_, c)| c.closing.is_some())
            .map(|(id, _)| *id)
            .collect();
        let mut writes = Vec::new();
        for id in ids {
            let c = self.clients.get_mut(&id).expect("listed client");
            let pending = self.pending.values().any(|p| p.client == id);
            // A pending start still owns its eventual response and thread.
            // Once the id arrives, dispatch the already-requested Stop.
            if !c.done && c.started {
                if let (Some(thread), Some(turn)) = (&c.thread, &c.turn) {
                    if !c.interrupt_sent {
                        c.interrupt_sent = true;
                        let params = json!({"threadId":thread,"turnId":turn});
                        writes.push(self.rpc(0, Value::Null, "turn/interrupt", params));
                    }
                }
                continue;
            }
            if pending {
                continue;
            }
            if let Some(thread) = c.thread.clone() {
                self.threads.remove(&thread);
                writes.push(self.rpc(
                    0,
                    Value::Null,
                    "thread/unsubscribe",
                    json!({"threadId":thread}),
                ));
            }
            if let Some(mut client) = self.clients.remove(&id) {
                let writer = client.writer.take();
                // Drop the sender to close the queue, but let the writer
                // deliver its final queued reply/terminal before EOF.
                drop(client);
                if let Some(mut writer) = writer {
                    tokio::spawn(async move {
                        if tokio::time::timeout(STOP_WAIT, &mut writer).await.is_err() {
                            writer.abort();
                            let _ = writer.await;
                        }
                    });
                }
            }
            self.requests.retain(|_, client| *client != id);
        }
        writes
    }
}

async fn read_events<R: tokio::io::AsyncRead + Unpin>(
    read: R,
    tx: mpsc::Sender<Event>,
    client: Option<u64>,
    budget: Arc<Semaphore>,
) {
    let mut read = BufReader::new(read);
    loop {
        let mut bytes = Vec::new();
        // take bounds the allocation even if a peer never supplies a newline.
        let result = tokio::io::AsyncReadExt::take(&mut read, (MAX_LINE + 1) as u64)
            .read_until(b'\n', &mut bytes)
            .await;
        match result {
            Ok(0) | Err(_) => break,
            Ok(_) if bytes.len() > MAX_LINE => break,
            Ok(_) => {
                let Ok(held) = budget.clone().try_acquire_many_owned(bytes.len() as u32) else {
                    break;
                };
                if let Ok(value) = serde_json::from_slice(&bytes) {
                    let event = match client {
                        Some(id) => Event::Client(id, value, held),
                        None => Event::Server(value, held),
                    };
                    if tx.send(event).await.is_err() {
                        return;
                    }
                }
            }
        }
    }
    let _ = tx.send(client.map_or(Event::Dead, Event::Gone)).await;
}

async fn write_value(
    write: &mut (impl tokio::io::AsyncWrite + Unpin),
    value: &Value,
) -> Result<(), ()> {
    let line = format!("{value}\n");
    tokio::time::timeout(STOP_WAIT, write.write_all(line.as_bytes()))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
}

fn retire_pool(pool: Option<([u8; 32], u64)>, joins: &mut mpsc::Receiver<Join>) {
    let mut pools = pool.and_then(|_| POOLS.get_or_init(Default::default).lock().ok());
    if let (Some((key, id)), Some(pools)) = (pool, pools.as_mut()) {
        if pools.get(&key).is_some_and(|entry| entry.id == id) {
            pools.remove(&key);
        }
    }
    joins.close();
}

fn try_retire_idle(
    pool: Option<([u8; 32], u64)>,
    joins: &mut mpsc::Receiver<Join>,
) -> Option<Join> {
    let mut pools = pool.and_then(|_| POOLS.get_or_init(Default::default).lock().ok());
    if let Ok(join) = joins.try_recv() {
        return Some(join);
    }
    if let (Some((key, id)), Some(pools)) = (pool, pools.as_mut()) {
        if pools.get(&key).is_some_and(|entry| entry.id == id) {
            pools.remove(&key);
        }
    }
    joins.close();
    None
}

fn accept_client(
    routes: &mut Routes,
    next_client: &mut u64,
    Join { socket, config }: Join,
    tx: &mpsc::Sender<Event>,
    bytes: &Arc<Semaphore>,
) {
    *next_client += 1;
    let id = *next_client;
    let (read, mut write) = socket.into_split();
    let reader = tokio::spawn(read_events(read, tx.clone(), Some(id), bytes.clone()));
    let (out, mut lines) = mpsc::channel::<Frame>(64);
    let events = tx.clone();
    let writer = tokio::spawn(async move {
        while let Some(frame) = lines.recv().await {
            if write.write_all(frame.line.as_bytes()).await.is_err()
                || write.write_all(b"\n").await.is_err()
            {
                break;
            }
        }
        let _ = events.send(Event::Gone(id)).await;
    });
    routes.clients.insert(
        id,
        Client {
            out: Some(out),
            reader,
            writer: Some(writer),
            config,
            thread: None,
            turn: None,
            started: false,
            done: false,
            closing: None,
            interrupt_sent: false,
        },
    );
}

#[cfg(test)]
async fn serve(cmd: Command, joins: mpsc::Receiver<Join>) {
    serve_pooled(cmd, joins, None).await;
}

async fn serve_pooled(
    mut cmd: Command,
    mut joins: mpsc::Receiver<Join>,
    pool: Option<([u8; 32], u64)>,
) {
    let Ok(mut child) = cmd.spawn() else {
        retire_pool(pool, &mut joins);
        return;
    };
    let mut guard = crate::process_group::GroupGuard::adopt(&cmd, &child);
    let Some(mut input) = child.stdin.take() else {
        retire_pool(pool, &mut joins);
        return;
    };
    let Some(output) = child.stdout.take() else {
        retire_pool(pool, &mut joins);
        return;
    };
    let (tx, mut events) = mpsc::channel(256);
    let bytes = Arc::new(Semaphore::new(QUEUED_BYTES));
    let reader = tokio::spawn(read_events(output, tx.clone(), None, bytes.clone()));
    let mut routes = Routes {
        clients: HashMap::new(),
        pending: HashMap::new(),
        threads: HashMap::new(),
        requests: HashMap::new(),
        next: 1,
        bytes: bytes.clone(),
    };
    let init = json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"hollow","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":false}}});
    let initialized = async {
        write_value(&mut input, &init).await?;
        loop {
            match events.recv().await {
                Some(Event::Server(value, _budget))
                    if value.get("id").and_then(Value::as_u64) == Some(1) =>
                {
                    if value.get("error").is_some() {
                        return Err(());
                    }
                    return write_value(&mut input, &json!({"method":"initialized"})).await;
                }
                Some(Event::Dead) | None => return Err(()),
                _ => {}
            }
        }
    };
    if !matches!(
        tokio::time::timeout(Duration::from_secs(10), initialized).await,
        Ok(Ok(()))
    ) {
        retire_pool(pool, &mut joins);
        reader.abort();
        guard
            .settle(&mut child, &crate::process_group::Timings::default())
            .await;
        return;
    }
    let mut next_client = 0;
    let mut idle = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    loop {
        tokio::select! {
            biased;
            _ = child.wait() => break,
            _ = tick.tick() => {
                if routes.clients.is_empty() && idle.elapsed() >= IDLE {
                    if let Some(join) = try_retire_idle(pool, &mut joins) {
                        accept_client(&mut routes, &mut next_client, join, &tx, &bytes);
                        idle = Instant::now();
                        continue;
                    }
                    break;
                }
                if routes.pending.values().any(|p| p.at.elapsed() >= RPC_WAIT) { break; }
                if routes.clients.values().any(|c| c.closing.is_some_and(|at| at.elapsed() >= STOP_WAIT)) { break; }
            },
            join = joins.recv(), if routes.clients.len() < MAX_CLIENTS => {
                let Some(join) = join else { break; };
                accept_client(&mut routes, &mut next_client, join, &tx, &bytes);
            },
            event = events.recv() => {
                let result = match event {
                    Some(Event::Dead) | None => Err(()),
                    Some(Event::Client(id, _, _budget)) if !routes.clients.contains_key(&id) => Ok(()),
                    Some(Event::Client(id, value, _budget)) => match routes.client(id, value) {
                        Ok(Some(value)) => write_value(&mut input, &value).await,
                        Ok(None) => Ok(()), Err(()) => Err(()),
                    },
                    Some(Event::Server(value, _budget)) => routes.server(value),
                    Some(Event::Gone(id)) => {
                        if let Some(client) = routes.clients.get_mut(&id) {
                            client.closing.get_or_insert_with(Instant::now);
                        }
                        Ok(())
                    },
                };
                if result.is_err() { break; }
            },
        }
        let mut failed = false;
        for value in routes.closing_writes() {
            if write_value(&mut input, &value).await.is_err() {
                failed = true;
                break;
            }
        }
        if failed {
            break;
        }
        if !routes.clients.is_empty() {
            idle = Instant::now();
        }
    }
    // A crash, timeout or malformed route retires this generation. Evict the
    // pool entry and close the join channel before settling so newly admitted
    // turns spawn a fresh server instead of queueing onto a dying one. Hold
    // the relay pipes until the actual server and its descendants have settled.
    retire_pool(pool, &mut joins);
    reader.abort();
    let _ = child.start_kill();
    guard
        .settle(&mut child, &crate::process_group::Timings::default())
        .await;
    drop(routes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Write};
    use tokio::io::{AsyncBufReadExt, BufReader};

    fn command(mcp: &str) -> Command {
        let mut cmd = Command::new("codex");
        cmd.args(["--disable", "apps", "-c", mcp, "app-server"])
            .env(crate::process_group::RUN_TAG_ENV, "turn-a")
            .env("HOLLOW_MCP_SECRET_HEADER_0", "private-capability");
        cmd
    }

    #[test]
    fn thread_config_owns_headers_and_shell_identity() {
        let cmd = command("mcp_servers={a={url=\"http://127.0.0.1:1/mcp/a\",env_http_headers={Authorization=\"HOLLOW_MCP_SECRET_HEADER_0\"},tool_timeout_sec=480}}");
        let config = thread_config(&cmd).unwrap();
        assert_eq!(
            config["mcp_servers"]["a"]["http_headers"]["Authorization"],
            "private-capability"
        );
        assert_eq!(config["mcp_servers"]["a"]["tool_timeout_sec"], 480);
        assert!(config["mcp_servers"]["a"].get("env_http_headers").is_none());
        assert_eq!(
            config["shell_environment_policy.set"]["HOLLOW_RUN_TAG"],
            "turn-a"
        );
        assert!(config["shell_environment_policy.set"]
            .get("HOLLOW_MCP_SECRET_HEADER_0")
            .is_none());
        let (server, _) = server_command(&cmd);
        assert!(server
            .as_std()
            .get_envs()
            .all(|(key, _)| !key.to_string_lossy().starts_with("HOLLOW_")
                || key == crate::process_group::RUN_TAG_ENV));
        assert!(server
            .as_std()
            .get_args()
            .all(|arg| !arg.to_string_lossy().contains("private-capability")));
    }

    #[test]
    fn stdio_unknown_flags_and_unscoped_commands_keep_private_processes() {
        assert!(thread_config(&command("mcp_servers={a={command=\"hollow\"}}")).is_none());
        let mut cmd = command("mcp_servers={}");
        cmd.env_remove(crate::process_group::RUN_TAG_ENV);
        assert!(thread_config(&cmd).is_none());
        let mut cmd = Command::new("codex");
        cmd.args(["--profile", "unknown", "app-server"]);
        assert!(thread_config(&cmd).is_none());
    }

    #[test]
    fn account_key_excludes_turn_authority_and_includes_account_home() {
        let a = command("mcp_servers={}");
        let mut b = command("mcp_servers={}");
        b.env(crate::process_group::RUN_TAG_ENV, "turn-b")
            .env("HOLLOW_MCP_SECRET_HEADER_0", "other-capability");
        assert_eq!(server_command(&a).1, server_command(&b).1);
        b.env("CODEX_HOME", "/another/account");
        assert_ne!(server_command(&a).1, server_command(&b).1);
        let mut c = command("mcp_servers={}");
        c.env("TMPDIR", "/chat/a")
            .env("TEMP", "/chat/a")
            .env("TMP", "/chat/a");
        let mut d = command("mcp_servers={}");
        d.env("TMPDIR", "/chat/b")
            .env("TEMP", "/chat/b")
            .env("TMP", "/chat/b");
        assert_eq!(server_command(&c).1, server_command(&d).1);
        c.env("HOLLOW_SELF_HOSTED_KEY", "account-a");
        d.env("HOLLOW_SELF_HOSTED_KEY", "account-b");
        assert_ne!(server_command(&c).1, server_command(&d).1);
        assert!(server_command(&c)
            .0
            .as_std()
            .get_envs()
            .any(|(k, v)| k == "HOLLOW_SELF_HOSTED_KEY"
                && v == Some(std::ffi::OsStr::new("account-a"))));
    }

    #[tokio::test]
    async fn large_existing_protocol_lines_survive_and_queued_bytes_are_refunded() {
        let payload = json!({"method":"large","params":{"text":"x".repeat(5 * 1024 * 1024)}});
        let wire = format!("{payload}\n");
        let size = wire.len();
        let budget = Arc::new(Semaphore::new(size * 2));
        let (mut write, read) = tokio::io::duplex(65536);
        let writer = tokio::spawn(async move {
            write.write_all(wire.as_bytes()).await.unwrap();
        });
        let (tx, mut rx) = mpsc::channel(4);
        let reader = tokio::spawn(read_events(read, tx, None, budget.clone()));
        let event = tokio::time::timeout(Duration::from_secs(8), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match &event {
            Event::Server(value, _) => assert_eq!(
                value["params"]["text"].as_str().unwrap().len(),
                5 * 1024 * 1024
            ),
            _ => panic!("valid large line was discarded"),
        }
        assert_eq!(budget.available_permits(), size);
        drop(event);
        assert_eq!(budget.available_permits(), size * 2);
        writer.await.unwrap();
        reader.await.unwrap();
    }

    #[tokio::test]
    async fn queued_events_cannot_exceed_the_shared_wire_budget() {
        let wire = format!(
            "{}\n",
            json!({"method":"test","params":{"text":"x".repeat(1024)}})
        );
        let size = wire.len();
        let budget = Arc::new(Semaphore::new(size));
        let (mut write, read) = tokio::io::duplex(8192);
        write
            .write_all(format!("{wire}{wire}").as_bytes())
            .await
            .unwrap();
        drop(write);
        let (tx, mut rx) = mpsc::channel(4);
        let reader = tokio::spawn(read_events(read, tx, None, budget.clone()));
        let first = rx.recv().await.unwrap();
        assert!(matches!(first, Event::Server(..)));
        assert_eq!(budget.available_permits(), 0);
        assert!(matches!(rx.recv().await.unwrap(), Event::Dead));
        drop(first);
        assert_eq!(budget.available_permits(), size);
        reader.await.unwrap();
    }

    /// A real child process speaking only our synthetic local protocol.
    #[test]
    #[ignore = "synthetic child server invoked explicitly by the shared-owner tests"]
    fn server_helper() {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else {
                break;
            };
            let Ok(v) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let method = v["method"].as_str().unwrap_or("");
            let id = &v["id"];
            let p = &v["params"];
            if method == "turn/start" && p["input"] == "reject" {
                // sleep-ok: inject a delayed server rejection for pending-start cleanup.
                std::thread::sleep(Duration::from_millis(80));
                println!(
                    "{}",
                    json!({"id":id,"error":{"code":-1,"message":"synthetic rejected start"}})
                );
                std::io::stdout().flush().unwrap();
                continue;
            }
            let result = match method {
                "initialize" => json!({"userAgent":"fake"}),
                "initialized" => continue,
                "thread/start" => {
                    if p["delay"] == true {
                        // sleep-ok: inject a late thread id after the client has stopped.
                        std::thread::sleep(Duration::from_millis(80));
                    }
                    json!({"thread":{"id":p["config"]["test_name"]}})
                }
                "turn/start" => {
                    if p["input"] == "crash" {
                        std::process::exit(3);
                    }
                    if p["input"] == "delay" {
                        // sleep-ok: inject a late turn id to exercise deferred interrupt.
                        std::thread::sleep(Duration::from_millis(80));
                    }
                    json!({"turn":{"id":format!("turn-{}", p["threadId"].as_str().unwrap())}})
                }
                "turn/steer" => {
                    if p["input"] == "crash" {
                        std::process::exit(3);
                    }
                    if p["input"] == "eof_stdout" {
                        // SAFETY: test-only synthetic child closing its stdout
                        // to trigger host teardown while remaining alive.
                        unsafe {
                            libc::close(1);
                        }
                        continue;
                    }
                    if p["input"] == "flood" {
                        let pad = "x".repeat(16 * 1024);
                        for seq in 0..256 {
                            println!(
                                "{}",
                                json!({"method":"turn/delta","params":{"threadId":p["threadId"],"seq":seq,"text":pad}})
                            );
                        }
                        println!(
                            "{}",
                            json!({"method":"account/rateLimits/updated","params":{"rateLimits":{"primary":{"usedPercent":10}}}})
                        );
                        std::io::stdout().flush().unwrap();
                    }
                    if p["input"] == "ratelimit" {
                        println!(
                            "{}",
                            json!({"method":"account/rateLimits/updated","params":{"rateLimits":{"primary":{"usedPercent":25}}}})
                        );
                        std::io::stdout().flush().unwrap();
                    }
                    json!({"turnId":p["expectedTurnId"]})
                }
                "turn/interrupt" => {
                    println!(
                        "{}",
                        json!({"method":"turn/completed","params":{"threadId":p["threadId"],"turn":{"id":p["turnId"],"status":"interrupted"}}})
                    );
                    json!({})
                }
                "thread/unsubscribe" => json!({"status":"unsubscribed"}),
                _ => json!({}),
            };
            println!("{}", json!({"id":id,"result":result}));
            std::io::stdout().flush().unwrap();
        }
    }

    fn start_host() -> (mpsc::Sender<Join>, tokio::task::JoinHandle<()>) {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args([
            "--exact",
            "shared_codex::tests::server_helper",
            "--ignored",
            "--nocapture",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
        let (tx, rx) = mpsc::channel(4);
        (tx, tokio::spawn(serve(cmd, rx)))
    }

    type Peer = (
        BufReader<tokio::net::unix::OwnedReadHalf>,
        tokio::net::unix::OwnedWriteHalf,
    );
    async fn response(peer: &mut Peer) -> Option<Value> {
        let mut line = String::new();
        let count = tokio::time::timeout(Duration::from_secs(8), peer.0.read_line(&mut line))
            .await
            .unwrap()
            .unwrap();
        (count > 0).then(|| serde_json::from_str(&line).unwrap())
    }
    async fn send(peer: &mut Peer, v: Value) {
        write_value(&mut peer.1, &v).await.unwrap();
    }
    async fn peer(tx: &mpsc::Sender<Join>, name: &str) -> Peer {
        let (a, b) = UnixStream::pair().unwrap();
        tx.send(Join {
            socket: a,
            config: Map::from_iter([("test_name".into(), json!(name))]),
        })
        .await
        .unwrap();
        let (read, write) = b.into_split();
        let mut peer = (BufReader::new(read), write);
        send(&mut peer, json!({"id":1,"method":"initialize"})).await;
        assert_eq!(response(&mut peer).await.unwrap()["id"], 1);
        send(
            &mut peer,
            json!({"id":2,"method":"thread/start","params":{}}),
        )
        .await;
        assert_eq!(
            response(&mut peer).await.unwrap()["result"]["thread"]["id"],
            name
        );
        send(
            &mut peer,
            json!({"id":3,"method":"turn/start","params":{"threadId":name,"input":"wait"}}),
        )
        .await;
        assert_eq!(
            response(&mut peer).await.unwrap()["result"]["turn"]["id"],
            format!("turn-{name}")
        );
        peer
    }

    #[tokio::test]
    async fn same_local_rpc_ids_route_to_distinct_threads_and_stop_is_scoped() {
        let (tx, host) = start_host();
        let mut a = peer(&tx, "a").await;
        let mut b = peer(&tx, "b").await;
        // EOF on a's input requests interruption, then closes a's output.
        a.1.shutdown().await.unwrap();
        assert_eq!(response(&mut a).await.unwrap()["method"], "turn/completed");
        assert!(response(&mut a).await.is_none());
        send(&mut b, json!({"id":4,"method":"turn/steer","params":{"threadId":"b","expectedTurnId":"turn-b"}})).await;
        assert_eq!(
            response(&mut b).await.unwrap()["result"]["turnId"],
            "turn-b"
        );
        drop(a);
        drop(b);
        drop(tx);
        tokio::time::timeout(Duration::from_secs(8), host)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn server_crash_fails_every_active_turn_without_replaying() {
        let (tx, host) = start_host();
        let mut a = peer(&tx, "a").await;
        let mut b = peer(&tx, "b").await;
        send(&mut a, json!({"id":4,"method":"turn/steer","params":{"threadId":"a","expectedTurnId":"turn-a","input":"crash"}})).await;
        assert!(response(&mut a).await.is_none());
        assert!(response(&mut b).await.is_none());
        tokio::time::timeout(Duration::from_secs(8), host)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn stops_during_pending_starts_keep_ownership_and_leave_sibling_live() {
        let (tx, host) = start_host();
        let mut b = peer(&tx, "b").await;
        for during_turn in [false, true] {
            let (a, socket) = UnixStream::pair().unwrap();
            tx.send(Join {
                socket: a,
                config: Map::from_iter([("test_name".into(), json!("a"))]),
            })
            .await
            .unwrap();
            let (read, write) = socket.into_split();
            let mut a = (BufReader::new(read), write);
            send(&mut a, json!({"id":1,"method":"initialize"})).await;
            response(&mut a).await.unwrap();
            send(
                &mut a,
                json!({"id":2,"method":"thread/start","params":{"delay":!during_turn}}),
            )
            .await;
            if during_turn {
                response(&mut a).await.unwrap();
                send(
                    &mut a,
                    json!({"id":3,"method":"turn/start","params":{"threadId":"a","input":"delay"}}),
                )
                .await;
            }
            a.1.shutdown().await.unwrap();
            let mut interrupted = false;
            while let Some(v) = response(&mut a).await {
                interrupted |= v["method"] == "turn/completed";
            }
            assert_eq!(interrupted, during_turn);
            send(&mut b, json!({"id":4,"method":"turn/steer","params":{"threadId":"b","expectedTurnId":"turn-b"}})).await;
            assert_eq!(
                response(&mut b).await.unwrap()["result"]["turnId"],
                "turn-b"
            );
        }
        drop(b);
        drop(tx);
        tokio::time::timeout(Duration::from_secs(8), host)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn rejected_pending_start_drains_error_and_preserves_sibling() {
        let (tx, host) = start_host();
        let mut b = peer(&tx, "b").await;
        let (a, socket) = UnixStream::pair().unwrap();
        tx.send(Join {
            socket: a,
            config: Map::from_iter([("test_name".into(), json!("a"))]),
        })
        .await
        .unwrap();
        let (read, write) = socket.into_split();
        let mut a = (BufReader::new(read), write);
        send(&mut a, json!({"id":1,"method":"initialize"})).await;
        response(&mut a).await.unwrap();
        send(&mut a, json!({"id":2,"method":"thread/start","params":{}})).await;
        response(&mut a).await.unwrap();
        send(
            &mut a,
            json!({"id":3,"method":"turn/start","params":{"threadId":"a","input":"reject"}}),
        )
        .await;
        a.1.shutdown().await.unwrap();
        assert_eq!(
            response(&mut a).await.unwrap()["error"]["message"],
            "synthetic rejected start"
        );
        assert!(response(&mut a).await.is_none());
        send(&mut b, json!({"id":4,"method":"turn/steer","params":{"threadId":"b","expectedTurnId":"turn-b"}})).await;
        assert_eq!(
            response(&mut b).await.unwrap()["result"]["turnId"],
            "turn-b"
        );
        drop(a);
        drop(b);
        drop(tx);
        tokio::time::timeout(Duration::from_secs(8), host)
            .await
            .unwrap()
            .unwrap();
    }

    async fn response_for_id(peer: &mut Peer, id: u64) -> Value {
        loop {
            let msg = response(peer)
                .await
                .expect("peer remained connected while waiting for RPC response");
            if msg.get("id").and_then(Value::as_u64) == Some(id) {
                return msg;
            }
        }
    }

    #[tokio::test]
    async fn backpressured_client_is_evicted_without_killing_sibling() {
        let (tx, host) = start_host();
        let mut a = peer(&tx, "a").await;
        let mut b = peer(&tx, "b").await;
        // Flood client a while a stops reading so its bounded outbound queue
        // overflows; only a is evicted and interrupted, while b stays live.
        send(
            &mut a,
            json!({"id":4,"method":"turn/steer","params":{"threadId":"a","expectedTurnId":"turn-a","input":"flood"}}),
        )
        .await;
        send(
            &mut b,
            json!({"id":4,"method":"turn/steer","params":{"threadId":"b","expectedTurnId":"turn-b"}}),
        )
        .await;
        assert_eq!(
            response_for_id(&mut b, 4).await["result"]["turnId"],
            "turn-b"
        );
        // Draining a's buffered kernel bytes ends in EOF once a was evicted
        // (writer abort may end mid-frame, so drain raw bytes instead of JSON).
        tokio::time::timeout(
            Duration::from_secs(8),
            tokio::io::copy(&mut a.0, &mut tokio::io::sink()),
        )
        .await
        .expect("evicted peer reached EOF")
        .unwrap();
        send(
            &mut b,
            json!({"id":5,"method":"turn/steer","params":{"threadId":"b","expectedTurnId":"turn-b"}}),
        )
        .await;
        assert_eq!(
            response_for_id(&mut b, 5).await["result"]["turnId"],
            "turn-b"
        );
        drop(a);
        drop(b);
        drop(tx);
        tokio::time::timeout(Duration::from_secs(8), host)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn closed_client_disconnects_cleanly_without_killing_sibling() {
        let (tx, host) = start_host();
        let a = peer(&tx, "a").await;
        let mut b = peer(&tx, "b").await;
        // Drop both halves of a abruptly so a's writer breaks and closes its
        // channel while turn/interrupt -> turn/completed and broadcasts land.
        drop(a);
        send(
            &mut b,
            json!({"id":4,"method":"turn/steer","params":{"threadId":"b","expectedTurnId":"turn-b","input":"ratelimit"}}),
        )
        .await;
        assert_eq!(
            response_for_id(&mut b, 4).await["result"]["turnId"],
            "turn-b"
        );
        send(
            &mut b,
            json!({"id":5,"method":"turn/steer","params":{"threadId":"b","expectedTurnId":"turn-b"}}),
        )
        .await;
        assert_eq!(
            response_for_id(&mut b, 5).await["result"]["turnId"],
            "turn-b"
        );
        drop(b);
        drop(tx);
        tokio::time::timeout(Duration::from_secs(8), host)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn teardown_evicts_pool_entry_and_closes_joins_before_settle() {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args([
            "--exact",
            "shared_codex::tests::server_helper",
            "--ignored",
            "--nocapture",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
        let key = [0x5Au8; 32];
        let id = NEXT_POOL_ID.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel(4);
        POOLS
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .insert(key, PoolEntry { id, tx: tx.clone() });
        let host = tokio::spawn(serve_pooled(cmd, rx, Some((key, id))));
        let mut a = peer(&tx, "a").await;
        send(
            &mut a,
            json!({"id":4,"method":"turn/steer","params":{"threadId":"a","expectedTurnId":"turn-a","input":"eof_stdout"}}),
        )
        .await;
        tokio::time::timeout(Duration::from_secs(8), tx.closed())
            .await
            .expect("join channel closed on teardown");
        // Pool entry is gone and joins are closed before settle finishes.
        let replacement_id = NEXT_POOL_ID.fetch_add(1, Ordering::Relaxed);
        let (replacement_tx, _replacement_rx) = mpsc::channel(4);
        {
            let mut pools = POOLS.get_or_init(Default::default).lock().unwrap();
            assert!(
                !pools.contains_key(&key),
                "retiring server must remove its pool entry before settle"
            );
            pools.insert(
                key,
                PoolEntry {
                    id: replacement_id,
                    tx: replacement_tx,
                },
            );
        }
        let (sock, _peer_sock) = UnixStream::pair().unwrap();
        assert!(matches!(
            tx.try_send(Join {
                socket: sock,
                config: Map::new(),
            }),
            Err(mpsc::error::TrySendError::Closed(_))
        ));
        assert!(response(&mut a).await.is_none());
        tokio::time::timeout(Duration::from_secs(8), host)
            .await
            .unwrap()
            .unwrap();
        // Finishing settle must not evict a newer generation inserted under key.
        let mut pools = POOLS.get_or_init(Default::default).lock().unwrap();
        assert_eq!(pools.get(&key).map(|e| e.id), Some(replacement_id));
        pools.remove(&key);
    }
}
