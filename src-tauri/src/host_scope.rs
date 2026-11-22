//! A host scope guard (BG1, from BugHunter's scope matcher): one deterministic
//! matcher over host lists where **deny wins** and an allow list makes the
//! default **deny**. A bare domain matches itself and its subdomains, `*.x`
//! matches subdomains only, and a pattern the matcher cannot enforce (a
//! scheme, a path, a port, a mid-host wildcard, a single label, a non-ASCII
//! name) is rejected, never accepted. A refusal names its reason.
//!
//! Used at three places: `ui.read_tab` (a tab on a webmail or banking host is
//! refused: the built-in deny list), `open_url` (only when the person wrote a
//! list in `host-scope.json`), and the computer broker's touch audit (BG2: a
//! host an agent's call names that the lists do not allow is flagged on the
//! audit row). The matcher does not follow redirects or resolve names: each
//! navigation is checked by its caller. The sandbox VM's egress (CU5) and an
//! unattended automation's default-deny scope are not wired here.
//!
//! **Restrict-only ratchet (ND1, 2026-11-20).** An agent may add a host to a
//! deny list of its own (`host-scope-agent-deny.json`, [`add_agent_deny_in`])
//! with no approval, because that can only narrow what agents reach. Capped at
//! [`MAX_AGENT_DENY`] entries; only the person removes one
//! ([`remove_agent_deny_in`], their own click in Settings, Computer control).
//! An entry is checked as text (the same parser as the person's lists, so a
//! path, port or wildcard in the middle is refused), a pasted address reduces to
//! its host, and the file is read at every decision (never cached), so an
//! addition holds on the very next call. Hosts are not resolved: a redirect is
//! judged by the navigation that follows it. An unreadable agent file closes
//! the scope like an unreadable person's file.

use serde_json::Value;
use std::path::Path;

/// The person's lists, in Hollow's support folder.
pub(crate) const SCOPE_FILE: &str = "host-scope.json";

/// The agent's own deny entries, in Hollow's support folder.
pub(crate) const AGENT_DENY_FILE: &str = "host-scope-agent-deny.json";

/// Most entries an agent may add. Entries are never removed by an agent.
pub(crate) const MAX_AGENT_DENY: usize = 100;

/// Hosts an agent may not read a tab from, whatever the person lists: webmail
/// and banking. A person's `allow` entry does not lift these; their file's
/// `allowDefaultDenied` list does, host by host.
pub(crate) const DEFAULT_DENY: &[&str] = &[
    "mail.google.com",
    "outlook.live.com",
    "outlook.office.com",
    "outlook.office365.com",
    "mail.yahoo.com",
    "mail.proton.me",
    "proton.me",
    "icloud.com",
    "chase.com",
    "bankofamerica.com",
    "wellsfargo.com",
    "citi.com",
    "capitalone.com",
    "usbank.com",
    "schwab.com",
    "fidelity.com",
    "vanguard.com",
    "paypal.com",
    "venmo.com",
    "americanexpress.com",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Pattern {
    /// `example.com`: the host itself and every subdomain.
    Domain(String),
    /// `*.example.com`: subdomains only.
    Subdomains(String),
}

impl Pattern {
    pub(crate) fn matches(&self, host: &str) -> bool {
        match self {
            Pattern::Domain(d) => {
                host == d
                    || host
                        .strip_suffix(d.as_str())
                        .is_some_and(|r| r.ends_with('.'))
            }
            Pattern::Subdomains(d) => host
                .strip_suffix(d.as_str())
                .is_some_and(|r| r.ends_with('.')),
        }
    }

    /// Whether every host `other` matches is also matched by `self`.
    fn covers(&self, other: &Pattern) -> bool {
        match (self, other) {
            (Pattern::Domain(d), Pattern::Domain(o)) => o == d || o.ends_with(&format!(".{d}")),
            (Pattern::Domain(d), Pattern::Subdomains(o)) => o == d || o.ends_with(&format!(".{d}")),
            (Pattern::Subdomains(d), Pattern::Domain(o)) => o.ends_with(&format!(".{d}")),
            (Pattern::Subdomains(d), Pattern::Subdomains(o)) => {
                o == d || o.ends_with(&format!(".{d}"))
            }
        }
    }

    fn text(&self) -> String {
        match self {
            Pattern::Domain(d) => d.clone(),
            Pattern::Subdomains(d) => format!("*.{d}"),
        }
    }
}

fn valid_label(l: &str) -> bool {
    !l.is_empty()
        && l.len() <= 63
        && !l.starts_with('-')
        && !l.ends_with('-')
        && l.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn valid_host(h: &str) -> bool {
    h.len() <= 253 && h.split('.').count() >= 2 && h.split('.').all(valid_label)
}

/// Parse one pattern, or say why the matcher cannot enforce it.
pub(crate) fn parse_pattern(raw: &str) -> Result<Pattern, String> {
    let p = raw.trim().to_ascii_lowercase();
    let p = p.strip_suffix('.').unwrap_or(&p).to_string();
    if p.is_empty() {
        return Err("an empty pattern".into());
    }
    if !p.is_ascii() {
        return Err(format!(
            "`{raw}` is not plain ASCII (write a punycode name)"
        ));
    }
    if p.contains("://")
        || p.contains('/')
        || p.contains(':')
        || p.contains('@')
        || p.contains('?')
        || p.contains('#')
        || p.contains(char::is_whitespace)
    {
        return Err(format!(
            "`{raw}` has a scheme, path, port or other part the matcher does not enforce"
        ));
    }
    if let Some(rest) = p.strip_prefix("*.") {
        return if valid_host(rest) {
            Ok(Pattern::Subdomains(rest.to_string()))
        } else {
            Err(format!("`{raw}` is not a valid wildcard host"))
        };
    }
    if p.contains('*') {
        return Err(format!(
            "`{raw}`: a wildcard is allowed only as a leading `*.`"
        ));
    }
    if valid_host(&p) {
        Ok(Pattern::Domain(p))
    } else {
        Err(format!(
            "`{raw}` is not a valid domain (at least two labels, letters, digits and hyphens)"
        ))
    }
}

/// The host of an http or https address, lowercase, without user, port or
/// trailing dot. `None` for anything else (other schemes, an IP literal in
/// brackets, a non-ASCII or malformed host), which callers treat as denied.
pub(crate) fn host_of(url: &str) -> Option<String> {
    let url = url.trim();
    // A backslash is a slash to a browser: refuse (fail closed) rather than
    // judge a different host than the one that opens.
    if url.contains('\\') {
        return None;
    }
    let lower = url.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = host
        .rsplit_once(':')
        .filter(|(_, p)| p.bytes().all(|b| b.is_ascii_digit()))
        .map_or(host, |(h, _)| h);
    let host = host.strip_suffix('.').unwrap_or(host);
    if host.starts_with('[') || !host.is_ascii() {
        return None;
    }
    // An IPv4 literal is a host too: it only matches a pattern that spells it.
    (host.split('.').count() >= 2 && host.split('.').all(valid_label)).then(|| host.to_string())
}

/// The hosts of the web addresses written in `text` (http and https), in
/// order, each once, at most [`MAX_TOUCHED`]. A scan, not a parse: it finds
/// `http://` or `https://` anywhere and reads up to the next space, quote,
/// bracket or comma.
pub(crate) fn hosts_in(text: &str) -> Vec<String> {
    const MAX_TOUCHED: usize = 20;
    let lower = text.to_ascii_lowercase();
    let mut out: Vec<String> = Vec::new();
    let mut from = 0;
    while out.len() < MAX_TOUCHED {
        let Some(at) = ["http://", "https://"]
            .iter()
            .filter_map(|p| lower[from..].find(p).map(|i| from + i))
            .min()
        else {
            break;
        };
        let end = lower[at..]
            .find(|c: char| {
                c.is_whitespace()
                    || matches!(
                        c,
                        '"' | '\'' | '<' | '>' | ')' | '(' | ',' | '\\' | '`' | ']' | '['
                    )
            })
            .map_or(lower.len(), |i| at + i);
        if let Some(h) = host_of(&lower[at..end]) {
            if !out.contains(&h) {
                out.push(h);
            }
        }
        from = end.max(at + 1);
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Decision {
    Allowed,
    Denied(String),
}

#[derive(Debug, Default, Clone)]
pub(crate) struct Scope {
    pub allow: Vec<Pattern>,
    pub deny: Vec<Pattern>,
    /// Hosts agents denied for themselves (ND1): deny entries too, kept apart
    /// so a refusal says where the block came from.
    pub agent_deny: Vec<Pattern>,
    /// Entries of the person's file the matcher rejected. A scope with any is
    /// closed: it denies every host, so a typo never widens what is reachable.
    pub invalid: Vec<String>,
}

impl Scope {
    /// Deny wins; an allow list makes everything it doesn't name denied.
    pub(crate) fn decide(&self, host: &str) -> Decision {
        let host = host.trim().to_ascii_lowercase();
        if !self.invalid.is_empty() {
            return Decision::Denied(format!(
                "the host list has an entry it cannot enforce: {}",
                self.invalid[0]
            ));
        }
        if let Some(p) = self.deny.iter().find(|p| p.matches(&host)) {
            return Decision::Denied(format!("{host} is on the deny list ({})", p.text()));
        }
        if let Some(p) = self.agent_deny.iter().find(|p| p.matches(&host)) {
            return Decision::Denied(format!(
                "{host} is on the list agents blocked for themselves ({}); the person can remove it in Settings, Computer control",
                p.text()
            ));
        }
        if !self.allow.is_empty() && !self.allow.iter().any(|p| p.matches(&host)) {
            return Decision::Denied(format!("{host} is not on the allow list"));
        }
        Decision::Allowed
    }

    pub(crate) fn decide_url(&self, url: &str) -> Decision {
        match host_of(url) {
            Some(h) => self.decide(&h),
            None => Decision::Denied(
                "the address has no host this guard can check (http and https only)".into(),
            ),
        }
    }

    /// The built-in deny list, minus any host the person listed under
    /// `allowDefaultDenied`.
    pub(crate) fn built_in(lifted: &[Pattern]) -> Self {
        let deny = DEFAULT_DENY
            .iter()
            .filter_map(|d| parse_pattern(d).ok())
            .filter(|d| {
                !lifted.iter().any(|l| {
                    l.matches(match d {
                        Pattern::Domain(x) | Pattern::Subdomains(x) => x,
                    })
                })
            })
            .collect();
        Self {
            deny,
            ..Self::default()
        }
    }

    /// Parse the person's file into `(scope, lifted)`; a missing file is an
    /// empty scope, an unreadable one is closed.
    pub(crate) fn load_user_from(dir: &Path) -> (Self, Vec<Pattern>) {
        let path = dir.join(SCOPE_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            // No file: no lists. A file that exists but can't be read closes
            // the scope.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return (Self::default(), Vec::new())
            }
            Err(_) => {
                return (
                    Self {
                        invalid: vec!["the file can't be read".into()],
                        ..Self::default()
                    },
                    Vec::new(),
                );
            }
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            return (
                Self {
                    invalid: vec!["the file is not valid JSON".into()],
                    ..Self::default()
                },
                Vec::new(),
            );
        };
        let mut scope = Self::default();
        let mut lifted = Vec::new();
        if !v.is_object() {
            scope.invalid.push("the file is not a JSON object".into());
        }
        for (key, target) in [("allow", 0), ("deny", 1), ("allowDefaultDenied", 2)] {
            let list = match v.get(key) {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(a)) => a.clone(),
                Some(_) => {
                    scope.invalid.push(format!("`{key}` is not a list"));
                    Vec::new()
                }
            };
            for item in &list {
                let Some(raw) = item.as_str() else {
                    scope
                        .invalid
                        .push(format!("`{key}` has an entry that is not text"));
                    continue;
                };
                match parse_pattern(raw) {
                    Ok(p) => match target {
                        0 => scope.allow.push(p),
                        1 => scope.deny.push(p),
                        _ => lifted.push(p),
                    },
                    Err(e) => scope.invalid.push(e),
                }
            }
        }
        (scope, lifted)
    }

    /// Add what an agent denied (ND1) to this scope's deny list. An agent
    /// file that cannot be read closes the scope.
    fn with_agent_deny(mut self, dir: &Path) -> Self {
        match read_agent_deny(dir) {
            Ok(entries) => {
                for e in entries {
                    match parse_pattern(&e.pattern) {
                        Ok(p) => self.agent_deny.push(p),
                        Err(why) => self.invalid.push(format!("agent deny list: {why}")),
                    }
                }
            }
            Err(why) => self.invalid.push(why),
        }
        self
    }

    /// The person's lists plus what agents denied (empty without files).
    pub(crate) fn user() -> Self {
        crate::instance::support_dir()
            .map(|d| Self::load_user_from(&d).0.with_agent_deny(&d))
            .unwrap_or_default()
    }

    /// The built-in deny list plus the person's lists: what a tab read and
    /// the touch audit check against.
    pub(crate) fn effective_from(dir: Option<&Path>) -> Self {
        let (user, lifted) = dir.map(Self::load_user_from).unwrap_or_default();
        let mut scope = Self::built_in(&lifted);
        scope.allow = user.allow;
        scope.deny.extend(user.deny);
        scope.invalid = user.invalid;
        match dir {
            Some(dir) => scope.with_agent_deny(dir),
            None => scope,
        }
    }

    pub(crate) fn effective() -> Self {
        Self::effective_from(crate::instance::support_dir().as_deref())
    }
}

/// One host an agent denied.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentDenyEntry {
    /// The pattern as the matcher reads it (`example.com`, `*.example.com`).
    pub pattern: String,
    /// RFC 3339.
    pub added_at: String,
    /// Which chat or run added it, for the person's list.
    pub by: String,
}

/// What [`add_agent_deny_in`] did.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddOutcome {
    pub added: bool,
    pub pattern: String,
    /// Why nothing was added, when nothing was.
    pub note: Option<String>,
    pub entries: usize,
    pub capacity: usize,
}

/// The agent deny file: none is an empty list; one that cannot be read or
/// understood is an error, which closes the scope (never an empty list).
pub(crate) fn read_agent_deny(dir: &Path) -> Result<Vec<AgentDenyEntry>, String> {
    let text = match std::fs::read_to_string(dir.join(AGENT_DENY_FILE)) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err("the agent deny list can't be read".into()),
    };
    let v: Value = serde_json::from_str(&text)
        .map_err(|_| "the agent deny list is not valid JSON".to_string())?;
    serde_json::from_value(v.get("entries").cloned().unwrap_or(Value::Array(vec![])))
        .map_err(|_| "the agent deny list has an entry it cannot read".to_string())
}

fn write_agent_deny(dir: &Path, entries: &[AgentDenyEntry]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("couldn't prepare the deny list: {e}"))?;
    let body = serde_json::to_vec_pretty(&serde_json::json!({ "entries": entries }))
        .map_err(|e| e.to_string())?;
    crate::vault::atomic_write(&dir.join(AGENT_DENY_FILE), &body)
        .map_err(|e| format!("couldn't write the deny list: {e}"))
}

/// The in-process half of [`AgentDenyLock`].
static AGENT_DENY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// An agent's text for a host: a pasted address reduces to its host (no
/// network: a short link is not followed), anything else goes through the
/// matcher's own parser, so what is accepted is exactly what can be enforced.
fn agent_pattern(raw: &str) -> Result<Pattern, String> {
    let raw = raw.trim();
    if raw.contains("://") {
        let host = host_of(raw).ok_or_else(|| {
            format!("`{raw}` has no host this guard can check (http and https only)")
        })?;
        return parse_pattern(&host);
    }
    parse_pattern(raw)
}

/// Add one host to the agent deny list: no approval, because it only narrows
/// access. Refused when the list holds [`MAX_AGENT_DENY`] entries or the text
/// is not a pattern the matcher can enforce. A host already denied (by the
/// built-in list, the person's list or an earlier entry) adds nothing.
pub(crate) fn add_agent_deny_in(dir: &Path, raw: &str, by: &str) -> Result<AddOutcome, String> {
    let pattern = agent_pattern(raw)?;
    let text = pattern.text();
    let bare = match &pattern {
        Pattern::Domain(d) | Pattern::Subdomains(d) => d.as_str(),
    };
    if SHARED_SUFFIXES.contains(&bare) {
        return Err(format!(
            "`{text}` is a shared suffix: denying it would block every site under it. Name the one site (for example `name.{bare}`)."
        ));
    }
    let _lock = AgentDenyLock::take(dir)?;
    let mut entries = read_agent_deny(dir)?;
    let outcome = |added: bool, note: Option<&str>, entries: usize| AddOutcome {
        added,
        pattern: text.clone(),
        note: note.map(str::to_string),
        entries,
        capacity: MAX_AGENT_DENY,
    };
    if entries.iter().any(|e| e.pattern == text) {
        return Ok(outcome(
            false,
            Some("already on the agent deny list"),
            entries.len(),
        ));
    }
    let existing = Scope::effective_from(Some(dir));
    if existing.invalid.is_empty() {
        if let Some(p) = existing
            .deny
            .iter()
            .chain(existing.agent_deny.iter())
            .find(|p| p.covers(&pattern))
        {
            let note = format!("already denied ({})", p.text());
            return Ok(outcome(false, Some(&note), entries.len()));
        }
    }
    let by: String = by.chars().filter(|c| !c.is_control()).take(80).collect();
    if entries.iter().filter(|e| e.by == by).count() >= MAX_PER_CHAT {
        return Err(format!(
            "this chat has already added {MAX_PER_CHAT} sites to the agent deny list; ask the person before blocking more"
        ));
    }
    if entries.len() >= MAX_AGENT_DENY {
        return Err(format!(
            "the agent deny list is full ({MAX_AGENT_DENY} entries); only the person can remove entries, in Settings, Computer control"
        ));
    }
    entries.push(AgentDenyEntry {
        pattern: text.clone(),
        added_at: chrono::Utc::now().to_rfc3339(),
        by,
    });
    write_agent_deny(dir, &entries)?;
    Ok(outcome(true, None, entries.len()))
}

/// Remove one entry: the person's own click (no agent tool reaches this).
/// An empty `pattern` on a file that cannot be read resets it (the person's
/// repair for the state that closes the scope).
pub(crate) fn remove_agent_deny_in(
    dir: &Path,
    pattern: &str,
) -> Result<Vec<AgentDenyEntry>, String> {
    let _lock = AgentDenyLock::take(dir)?;
    let mut entries = match read_agent_deny(dir) {
        Ok(entries) => entries,
        Err(_) if pattern.trim().is_empty() => {
            write_agent_deny(dir, &[])?;
            return Ok(Vec::new());
        }
        Err(why) => return Err(why),
    };
    let wanted = pattern.trim().to_ascii_lowercase();
    let before = entries.len();
    entries.retain(|e| e.pattern.to_ascii_lowercase() != wanted);
    if entries.len() == before {
        return Err(format!("{pattern} isn't on the agent deny list"));
    }
    write_agent_deny(dir, &entries)?;
    Ok(entries)
}

/// Most entries one chat may add.
pub(crate) const MAX_PER_CHAT: usize = 20;

/// Shared suffixes a bare denial would take whole: registry suffixes and
/// multi-tenant hosting domains. A site under one is named in full.
const SHARED_SUFFIXES: &[&str] = &[
    "co.uk",
    "org.uk",
    "ac.uk",
    "gov.uk",
    "com.au",
    "net.au",
    "org.au",
    "co.nz",
    "co.jp",
    "co.in",
    "com.br",
    "com.cn",
    "com.mx",
    "co.za",
    "github.io",
    "githubusercontent.com",
    "amazonaws.com",
    "cloudfront.net",
    "herokuapp.com",
    "vercel.app",
    "netlify.app",
    "pages.dev",
    "workers.dev",
    "web.app",
    "firebaseapp.com",
    "azurewebsites.net",
    "blogspot.com",
    "appspot.com",
];

/// Serializes read-modify-write of the agent file: a mutex in this process
/// and, on unix, an exclusive `flock` on a sidecar file for other Hollow
/// processes (`hollow serve`) sharing the support folder.
struct AgentDenyLock {
    _guard: std::sync::MutexGuard<'static, ()>,
    #[cfg(unix)]
    _file: std::fs::File,
}

impl AgentDenyLock {
    fn take(dir: &Path) -> Result<Self, String> {
        let guard = AGENT_DENY_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("couldn't prepare the deny list: {e}"))?;
            let file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(dir.join(format!("{AGENT_DENY_FILE}.lock")))
                .map_err(|e| format!("couldn't lock the deny list: {e}"))?;
            // SAFETY: flock on a descriptor this struct owns until it drops.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
                return Err("couldn't lock the deny list".into());
            }
            Ok(Self {
                _guard: guard,
                _file: file,
            })
        }
        #[cfg(not(unix))]
        {
            let _ = dir;
            Ok(Self { _guard: guard })
        }
    }
}

#[cfg(test)]
#[path = "host_scope_tests.rs"]
mod tests;
