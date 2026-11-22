//! Tailscale, through its own command-line tool: who this Mac is on the
//! tailnet (its MagicDNS name and its user's login, which the server's Host
//! and `Tailscale-User-Login` checks need), whether `tailscale serve`
//! publishes the phone API on port 8443 or forwards raw TCP to it, and
//! publishing it or stopping. Tailnet only: Hollow never runs Funnel, and
//! Settings warns when the port is funnelled.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Where the tool is, in order: a package install, Homebrew, the app's own.
const CANDIDATES: [&str; 3] = [
    "/usr/local/bin/tailscale",
    "/opt/homebrew/bin/tailscale",
    "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
];
/// The tailnet port the phone API is published on.
pub(crate) const SERVE_PORT: u16 = 8443;
/// One call of the tool, at most.
const CALL_LIMIT: Duration = Duration::from_secs(15);
/// Output kept from one call (a large tailnet's status runs to megabytes).
const OUTPUT_MAX: usize = 8 * 1024 * 1024;

/// The tool, if installed.
pub(crate) fn cli() -> Option<PathBuf> {
    CANDIDATES
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
}

/// This Mac on the tailnet, as `tailscale status --json` reports it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SelfInfo {
    /// `Self.DNSName` without its trailing dot, lower case.
    pub dns_name: Option<String>,
    /// The `LoginName` of the Self node's user.
    pub login: Option<String>,
    /// `BackendState` is `Running`.
    pub running: bool,
}

pub(crate) fn parse_status(json: &Value) -> SelfInfo {
    let me = &json["Self"];
    let dns_name = me["DNSName"]
        .as_str()
        .map(|name| name.trim().trim_end_matches('.').to_ascii_lowercase())
        .filter(|name| !name.is_empty());
    let user = match &me["UserID"] {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(s.clone()),
        _ => None,
    };
    let login = user
        .and_then(|id| {
            json["User"][id.as_str()]["LoginName"]
                .as_str()
                .map(str::to_string)
        })
        .map(|login| login.trim().to_string())
        .filter(|login| !login.is_empty());
    SelfInfo {
        dns_name,
        login,
        running: json["BackendState"].as_str() == Some("Running"),
    }
}

/// What `tailscale serve status --json` says of port 8443, and of any raw
/// TCP forward to the phone API.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ServeInfo {
    /// 8443's `/` proxies to the phone API on loopback.
    pub published: bool,
    /// 8443 is open to the internet (Funnel). Hollow never turns it on.
    pub funnel: bool,
    /// 8443 serves something else.
    pub other: Option<String>,
    /// Tailnet ports that `tailscale serve --tcp` forwards straight to the
    /// phone API's port in this Mac's saved serve config (second review,
    /// 2026-11-04). A raw TCP forward passes on nothing of the caller (no
    /// `Tailscale-User-Login`, no `X-Forwarded-*`), so a request through it
    /// that names a loopback `Host` reads as one from this Mac itself and is
    /// judged by its token alone. Settings shows it as a problem, and Stop
    /// publishing removes it (`tailscale serve --tcp=<port> off`).
    pub tcp_forwards: Vec<u16>,
    /// The same, from a `tailscale serve` still running in the foreground
    /// (in a terminal, started without `--bg`). `off` edits only the saved
    /// config, so only Control-C in that terminal stops it, and Settings
    /// says so (third review, 2026-11-05).
    pub foreground_forwards: Vec<u16>,
    /// The same, in a Tailscale service's config, which `off` reaches only
    /// with `--service=`: Settings shows the command that removes each
    /// ([`ServiceForward::stop`]) and Hollow runs none of them, since a
    /// service is advertised to the whole tailnet.
    pub service_forwards: Vec<ServiceForward>,
}

/// A raw TCP forward to the phone API in a Tailscale service's config.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ServiceForward {
    /// The service's name, as `serve status` keys it (`svc:<name>`).
    pub service: String,
    /// The tailnet port it forwards.
    pub port: u16,
}

impl ServiceForward {
    /// The command that removes it, for the person to paste into a
    /// terminal, running the tool as `tool` ([`terminal_tool`]). Only for a
    /// name as Tailscale gives one (`svc:` and then lower-case letters,
    /// digits and hyphens): the name comes from Tailscale's status, and a
    /// shell would act on anything else in it (the review of the third
    /// review's fixes, 2026-11-05). `None` for any other name, which
    /// Settings tells the person how to remove in words.
    pub(crate) fn stop(&self, tool: &str) -> Option<String> {
        plain_service(&self.service).then(|| {
            format!(
                "{tool} serve --service={} --tcp={} off",
                self.service, self.port
            )
        })
    }
}

/// `svc:` and then one or more lower-case ASCII letters, digits or hyphens.
fn plain_service(name: &str) -> bool {
    name.strip_prefix("svc:").is_some_and(|rest| {
        !rest.is_empty()
            && rest
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    })
}

/// How a command for the person's terminal runs the tool: `tailscale` when
/// a folder of their PATH (`path`) holds it, else the path Hollow found it
/// at (`cli`), quoted if a shell would split it. The Tailscale app's own
/// copy is on no PATH unless its command-line integration is installed.
pub(crate) fn terminal_tool(cli: Option<&Path>, path: &[PathBuf]) -> String {
    let on_path = path.iter().any(|dir| dir.join("tailscale").is_file());
    let Some(cli) = cli.filter(|_| !on_path) else {
        return "tailscale".to_string();
    };
    let cli = cli.to_string_lossy();
    let plain = cli
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-'));
    if plain {
        cli.into_owned()
    } else {
        format!("'{}'", cli.replace('\'', r"'\''"))
    }
}

/// Whether a `TCPForward` target (`host:port`) is the phone API on loopback.
fn forwards_to(target: &str, local_port: u16) -> bool {
    let target = target.trim();
    let target = target.strip_prefix("tcp://").unwrap_or(target);
    let Some((host, port)) = target.rsplit_once(':') else {
        return target.parse::<u16>().ok() == Some(local_port);
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    port.parse::<u16>().ok() == Some(local_port)
        && matches!(host, "127.0.0.1" | "localhost" | "::1" | "0.0.0.0" | "")
}

/// The ports whose raw TCP forward in one serve config (`TCP`, port to
/// handler) reaches the phone API, in order.
fn tcp_forwards(config: &Value, local_port: u16) -> Vec<u16> {
    let mut ports: Vec<u16> = config["TCP"]
        .as_object()
        .into_iter()
        .flat_map(|tcp| tcp.iter())
        .filter(|(_, handler)| {
            handler["TCPForward"]
                .as_str()
                .is_some_and(|target| forwards_to(target, local_port))
        })
        .filter_map(|(port, _)| port.parse::<u16>().ok())
        .collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// The same across every foreground `tailscale serve` (`Foreground`, keyed
/// by its session).
fn foreground_forwards(json: &Value, local_port: u16) -> Vec<u16> {
    let mut ports: Vec<u16> = json["Foreground"]
        .as_object()
        .into_iter()
        .flat_map(|each| each.values())
        .flat_map(|config| tcp_forwards(config, local_port))
        .collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// The same in each Tailscale service's config (`Services`, keyed by name).
fn service_forwards(json: &Value, local_port: u16) -> Vec<ServiceForward> {
    let mut forwards: Vec<ServiceForward> = json["Services"]
        .as_object()
        .into_iter()
        .flat_map(|each| each.iter())
        .flat_map(|(service, config)| {
            tcp_forwards(config, local_port)
                .into_iter()
                .map(|port| ServiceForward {
                    service: service.clone(),
                    port,
                })
        })
        .collect();
    forwards.sort_unstable();
    forwards
}

pub(crate) fn parse_serve(json: &Value, local_port: u16) -> ServeInfo {
    let suffix = format!(":{SERVE_PORT}");
    let ours = [
        format!("http://127.0.0.1:{local_port}"),
        format!("http://localhost:{local_port}"),
        format!("127.0.0.1:{local_port}"),
        format!("localhost:{local_port}"),
    ];
    let mut info = ServeInfo::default();
    if let Some(web) = json["Web"].as_object() {
        for (host_port, site) in web.iter().filter(|(k, _)| k.ends_with(&suffix)) {
            let proxy = site["Handlers"]["/"]["Proxy"]
                .as_str()
                .map(|p| p.trim_end_matches('/').to_string());
            match proxy {
                Some(p) if ours.contains(&p) => info.published = true,
                Some(p) => info.other = Some(p),
                None => info.other = Some(host_port.clone()),
            }
        }
    }
    if let Some(funnel) = json["AllowFunnel"].as_object() {
        info.funnel = funnel
            .iter()
            .any(|(k, v)| k.ends_with(&suffix) && v.as_bool() == Some(true));
    }
    info.tcp_forwards = tcp_forwards(json, local_port);
    info.foreground_forwards = foreground_forwards(json, local_port);
    info.service_forwards = service_forwards(json, local_port);
    info
}

/// Why a call of the tool failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CallFailed {
    /// In the tool's own words, or why it couldn't run.
    pub message: String,
    /// It outlasted its limit: Tailscale isn't answering, so a next call
    /// would most likely wait as long.
    pub timed_out: bool,
}

impl CallFailed {
    /// The tool answered with a refusal or failed, or couldn't be run.
    pub(crate) fn said(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            timed_out: false,
        }
    }

    /// The tool didn't finish within its limit.
    pub(crate) fn hung(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            timed_out: true,
        }
    }
}

/// Run the tool with `args`: its output, or an error in its own words.
async fn run(args: &[&str]) -> Result<String, String> {
    call(args).await.map_err(|failed| failed.message)
}

/// [`run`], saying whether a failure was a timeout.
async fn call(args: &[&str]) -> Result<String, CallFailed> {
    let cli = cli().ok_or_else(|| {
        CallFailed::said("Tailscale isn't installed on this Mac (Hollow looks in /usr/local/bin, /opt/homebrew/bin and the Tailscale app).")
    })?;
    call_program(&cli, args, CALL_LIMIT).await
}

/// Run `program` (the tool; tests run their own) with `args`. A call that
/// outlasts `limit` is stopped, keeping what it printed (`tailscale serve`
/// waits after printing the address where Serve must first be allowed for
/// the tailnet), and fails as timed out.
pub(crate) async fn call_program(
    program: &Path,
    args: &[&str],
    limit: Duration,
) -> Result<String, CallFailed> {
    use tokio::io::AsyncReadExt;
    let mut child = tokio::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            CallFailed::said(format!(
                "Couldn't run Tailscale ({}): {e}",
                program.display()
            ))
        })?;
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let read = async {
        let read_out = async {
            if let Some(pipe) = stdout.as_mut() {
                let _ = pipe.take(OUTPUT_MAX as u64).read_to_end(&mut out).await;
            }
        };
        let read_err = async {
            if let Some(pipe) = stderr.as_mut() {
                let _ = pipe.take(OUTPUT_MAX as u64).read_to_end(&mut err).await;
            }
        };
        tokio::join!(read_out, read_err);
        child.wait().await
    };
    let status = match tokio::time::timeout(limit, read).await {
        Ok(status) => {
            status.map_err(|e| CallFailed::said(format!("Tailscale didn't finish: {e}")))?
        }
        Err(_) => {
            let said = format!(
                "{}{}",
                String::from_utf8_lossy(&out),
                String::from_utf8_lossy(&err)
            );
            let said = said.trim();
            return Err(CallFailed::hung(if said.is_empty() {
                format!(
                    "Tailscale didn't answer within {} seconds.",
                    limit.as_secs()
                )
            } else {
                format!("Tailscale is waiting: {said}")
            }));
        }
    };
    let text = String::from_utf8_lossy(&out).into_owned();
    if status.success() {
        Ok(text)
    } else {
        let said = format!("{}{}", String::from_utf8_lossy(&err), text);
        let said = said.trim();
        Err(CallFailed::said(if said.is_empty() {
            format!("Tailscale stopped with {status}.")
        } else {
            format!("Tailscale said: {said}")
        }))
    }
}

pub(crate) async fn status() -> Result<SelfInfo, String> {
    let text = run(&["status", "--json"]).await?;
    let json: Value = serde_json::from_str(&text)
        .map_err(|e| format!("Tailscale's status wasn't readable: {e}"))?;
    Ok(parse_status(&json))
}

pub(crate) async fn serve_status(local_port: u16) -> Result<ServeInfo, String> {
    let text = run(&["serve", "status", "--json"]).await?;
    if text.trim().is_empty() {
        return Ok(ServeInfo::default());
    }
    let json: Value = serde_json::from_str(&text)
        .map_err(|e| format!("Tailscale's serve status wasn't readable: {e}"))?;
    Ok(parse_serve(&json, local_port))
}

/// `tailscale serve --bg --https=8443 http://127.0.0.1:<port>`.
pub(crate) async fn publish(local_port: u16) -> Result<(), String> {
    let https = format!("--https={SERVE_PORT}");
    let target = format!("http://127.0.0.1:{local_port}");
    run(&["serve", "--bg", &https, &target]).await.map(|_| ())
}

/// Stop publishing what `serve_status` (`read`) found of the phone API:
/// each raw TCP forward to it in the saved config first (`tailscale serve
/// --tcp=<port> off`), then `tailscale serve --https=8443 --set-path=/ off`
/// only when 8443's `/` serves Hollow's own address; another mount on 8443
/// stays. Never another app's serve config (the
/// review of the third review's fixes, 2026-11-05: 8443 went off whenever
/// no saved forward was found, so a button left from an older status
/// removed another app's 8443), so a status Hollow couldn't read changes
/// nothing. A refused step doesn't stop the rest (third review, 2026-11-05:
/// the first error used to leave a saved forward on a higher port), but a
/// timed-out one does (the review of its fixes: a hung Tailscale held the
/// panel 15 seconds a step), and the error names each step that failed. A
/// foreground or service forward is left alone: `off` edits only the saved
/// config, so Settings says how to stop those.
pub(crate) async fn unpublish(read: Result<ServeInfo, String>) -> Result<(), String> {
    let serve = read.map_err(|error| {
        let error = error.trim().trim_end_matches('.');
        format!("Couldn't read what Tailscale serves, so Hollow changed nothing ({error}).")
    })?;
    unpublish_with(&serve, |args| async move {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        call(&args).await.map(|_| ())
    })
    .await
}

/// [`unpublish`] of what was read, with each call of the tool made by
/// `call` (tests stand in their own).
pub(crate) async fn unpublish_with<F, Fut>(serve: &ServeInfo, mut call: F) -> Result<(), String>
where
    F: FnMut(Vec<String>) -> Fut,
    Fut: std::future::Future<Output = Result<(), CallFailed>>,
{
    let mut steps: Vec<(Vec<String>, String)> = serve
        .tcp_forwards
        .iter()
        .map(|port| {
            (
                vec!["serve".into(), format!("--tcp={port}"), "off".into()],
                format!("remove the --tcp forward on tailnet port {port}"),
            )
        })
        .collect();
    if serve.published {
        steps.push((
            vec![
                "serve".into(),
                format!("--https={SERVE_PORT}"),
                // Only Hollow's own mount at `/`: a bare `--https=8443 off`
                // removes every mount on the port, another app's `--set-path`
                // ones included (its "delete them?" prompt answers yes when
                // stdin isn't a terminal).
                "--set-path=/".into(),
                "off".into(),
            ],
            format!("stop publishing on port {SERVE_PORT}"),
        ));
    }
    let mut failed = Vec::new();
    let total = steps.len();
    for (done, (args, what)) in steps.into_iter().enumerate() {
        let Err(error) = call(args).await else {
            continue;
        };
        let message = error.message.trim().trim_end_matches('.');
        failed.push(format!("Couldn't {what} ({message})."));
        // A refusal leaves the next step worth trying; a timeout means
        // Tailscale isn't answering, and each step left would wait as long.
        if error.timed_out {
            if done + 1 < total {
                failed.push(
                    "Hollow stopped there, since Tailscale isn't answering: choose Stop publishing again once it is."
                        .into(),
                );
            }
            break;
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed.join(" "))
    }
}

/// Which of this server's names a request's `Host` gave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostName {
    /// `127.0.0.1:<port>` or `localhost:<port>`: this Mac itself.
    Loopback,
    /// This Mac's MagicDNS name: the tailnet, through `tailscale serve`.
    Tailnet,
}

/// The tailnet identity the server checks requests against, refreshed by
/// the server's loop and by Settings. Unknown (Tailscale not running) admits
/// only loopback hosts and refuses every request that names a tailnet user.
#[derive(Default)]
pub(crate) struct Identity {
    known: Mutex<Option<(Instant, SelfInfo)>>,
}

impl Identity {
    /// A fixed identity (tests).
    #[cfg(test)]
    pub(crate) fn fixed(info: SelfInfo) -> Self {
        Self {
            known: Mutex::new(Some((Instant::now(), info))),
        }
    }

    pub(crate) fn current(&self) -> SelfInfo {
        self.known
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|(_, info)| info.clone())
            .unwrap_or_default()
    }

    /// When it was last read.
    pub(crate) fn age(&self) -> Option<Duration> {
        self.known
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|(at, _)| at.elapsed())
    }

    pub(crate) fn set(&self, info: SelfInfo) {
        *self.known.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), info));
    }

    /// Read it again; a failure leaves it unknown (no tailnet host passes).
    pub(crate) async fn refresh(&self) -> Result<SelfInfo, String> {
        match status().await {
            Ok(info) => {
                self.set(info.clone());
                Ok(info)
            }
            Err(error) => {
                self.set(SelfInfo::default());
                Err(error)
            }
        }
    }

    /// Which of this server's names `host` (the `Host` header) is: loopback
    /// on `port`, or this Mac's MagicDNS name, bare or with the tailnet port;
    /// `None` for any other.
    pub(crate) fn host_name(&self, host: &str, port: u16) -> Option<HostName> {
        let host = host.trim().to_ascii_lowercase();
        if host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}") {
            return Some(HostName::Loopback);
        }
        let name = self.current().dns_name?;
        (host == name || host == format!("{name}:{SERVE_PORT}")).then_some(HostName::Tailnet)
    }

    /// Whether a request from tailnet user `login` may go on: only this
    /// Mac's own user.
    pub(crate) fn login_ok(&self, login: &str) -> bool {
        match self.current().login {
            Some(own) => !login.trim().is_empty() && own.eq_ignore_ascii_case(login.trim()),
            None => false,
        }
    }
}
