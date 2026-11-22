//! Paperclip PR 14 (14a): what kind of result an agent run ended with.
//!
//! A pure module. [`classify`] reads only structured facts: how the run
//! ended (`process_group::Ended`), its exit status, the result envelope's
//! structured fields ([`crate::agent::ResultDetail`]), and, for a failed run
//! only, the envelope's error texts and the bounded stderr tail. Assistant
//! prose is never evidence: a successful run whose answer mentions "usage
//! limit reached" is still a success.
//!
//! Precedence (the rule):
//! 1. stalled (PR 15b's watchdog sets `stalled_minutes`);
//! 2. budget stop (PR 8b's `Ended::LimitReached`, the guard's notice, or
//!    Claude's `error_max_budget_usd` / `budget_exhausted`);
//! 3. Hollow's own ends: `Stopped` → cancelled, `TimedOut` → timed out;
//! 4. refusal (`stop_reason: "refusal"`), at any exit code;
//! 5. turn limit (`error_max_turns` / `max_turns`);
//! 6. actions denied by the permission mode (Antigravity's
//!    `denied_actions`, which still reports `SUCCESS` at exit 0);
//! 7. success when the exit is 0 and the envelope is not an error;
//! 8. otherwise the envelope texts: resume failed → sign-in (also HTTP
//!    401/403) → usage limit → rate limit, overload or network (also HTTP
//!    429, 529 and 5xx) → failed.
//!
//! The envelope regexes are shared with PR 9b's workflow retry
//! (`workflow_retry::classify_failure`), which keeps its own order (sign-in,
//! usage limit, rate limit, network). They are only ever applied to envelope
//! texts and a failed run's stderr.
//!
//! Where the label goes (one place per surface): chat composes
//! `"{label}: {message}"` once for a failed exit ([`ResultKind::label_error`]);
//! the frontend labels an error `result` event itself (`src/resultKind.ts`
//! mirrors these strings); workflows hand the RAW error to PR 9b's classifier
//! and label only the displayed `last_error`. The label of `Transient`
//! contains "rate limited", which 9b's `RATE_RE` would match first, so a
//! labelled string must never reach a classifier.

use crate::agent::{AgentEvent, AgentKind, ResultDetail};
use crate::process_group::Ended;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;

/// Claude's result subtype when Hollow's own `--max-budget-usd` stopped it.
pub(crate) const SUBTYPE_MAX_BUDGET: &str = "error_max_budget_usd";
/// Claude's result subtype when the run hit its turn limit.
pub(crate) const SUBTYPE_MAX_TURNS: &str = "error_max_turns";

/// The error a refusal is recorded with (chat and workflows alike). One line,
/// no em dash, no angle brackets.
pub(crate) const REFUSAL_ERROR: &str = "The model declined the request (refusal).";

/// How an agent run ended, as the receipt, the chat events and the workflow
/// status record it. Serialized camelCase (`as_str`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ResultKind {
    Success,
    Refusal,
    MaxTurns,
    ProviderQuota,
    Transient,
    Auth,
    ResumeFailed,
    PermissionDenied,
    Stalled,
    BudgetStop,
    Cancelled,
    TimedOut,
    Error,
}

impl ResultKind {
    /// Every kind, in declaration order (tests and the frontend mirror).
    #[cfg(test)]
    pub const ALL: [ResultKind; 13] = [
        ResultKind::Success,
        ResultKind::Refusal,
        ResultKind::MaxTurns,
        ResultKind::ProviderQuota,
        ResultKind::Transient,
        ResultKind::Auth,
        ResultKind::ResumeFailed,
        ResultKind::PermissionDenied,
        ResultKind::Stalled,
        ResultKind::BudgetStop,
        ResultKind::Cancelled,
        ResultKind::TimedOut,
        ResultKind::Error,
    ];

    /// The wire value (`resultKind`), identical to the serde name.
    pub fn as_str(self) -> &'static str {
        match self {
            ResultKind::Success => "success",
            ResultKind::Refusal => "refusal",
            ResultKind::MaxTurns => "maxTurns",
            ResultKind::ProviderQuota => "providerQuota",
            ResultKind::Transient => "transient",
            ResultKind::Auth => "auth",
            ResultKind::ResumeFailed => "resumeFailed",
            ResultKind::PermissionDenied => "permissionDenied",
            ResultKind::Stalled => "stalled",
            ResultKind::BudgetStop => "budgetStop",
            ResultKind::Cancelled => "cancelled",
            ResultKind::TimedOut => "timedOut",
            ResultKind::Error => "error",
        }
    }

    /// A short, note-safe label (no em dash, no `<`/`>`), mirrored in
    /// `src/resultKind.ts`.
    pub fn label(self) -> &'static str {
        match self {
            ResultKind::Success => "completed",
            ResultKind::Refusal => "the model declined the request",
            ResultKind::MaxTurns => "turn limit reached",
            ResultKind::ProviderQuota => "usage limit reached",
            ResultKind::Transient => "rate limited, overloaded or network error",
            ResultKind::Auth => "sign-in problem",
            ResultKind::ResumeFailed => "the previous agent session could not be resumed",
            ResultKind::PermissionDenied => "actions were denied by the permission mode",
            ResultKind::Stalled => "stalled: no output",
            ResultKind::BudgetStop => "budget reached",
            ResultKind::Cancelled => "cancelled",
            ResultKind::TimedOut => "timed out",
            ResultKind::Error => "failed",
        }
    }

    /// Anything but success.
    pub fn is_failure(self) -> bool {
        self != ResultKind::Success
    }

    /// Whether a failure message gets this kind's label in front
    /// ([`Self::label_error`]). Only the kinds whose raw message rarely says
    /// what happened: a budget stop, a stall, a refusal and denied actions
    /// already carry their own wording, and "failed" adds nothing.
    pub fn labels_error(self) -> bool {
        matches!(
            self,
            ResultKind::MaxTurns
                | ResultKind::ProviderQuota
                | ResultKind::Transient
                | ResultKind::Auth
                | ResultKind::ResumeFailed
        )
    }

    /// `"{label}: {message}"` for a kind that [labels](Self::labels_error)
    /// its errors, the message unchanged otherwise. Never doubled: a message
    /// that already starts with the label is returned as is.
    pub fn label_error(self, message: &str) -> String {
        let label = self.label();
        if !self.labels_error() || message.starts_with(label) {
            return message.to_string();
        }
        format!("{label}: {message}")
    }
}

/// The last result envelope a run printed, with any API-error messages
/// merged into its errors (Claude wraps some API errors as assistant
/// messages before an `is_error` result that may carry no text).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResultRecord {
    pub is_error: bool,
    pub subtype: String,
    pub text: Option<String>,
    pub detail: ResultDetail,
}

impl ResultRecord {
    /// The record of a `Result` event; `None` for any other event.
    pub fn from_event(evt: &AgentEvent) -> Option<Self> {
        match evt {
            AgentEvent::Result {
                text,
                is_error,
                subtype,
                detail,
                ..
            } => Some(ResultRecord {
                is_error: *is_error,
                subtype: subtype.clone(),
                text: text.clone(),
                detail: detail.clone(),
            }),
            _ => None,
        }
    }

    /// Fold in one API-error message: its text joins `detail.errors` (within
    /// the caps) and its status fills `api_error_status` when that is unset.
    pub fn merge_api_error(&mut self, text: &str, status: Option<u16>) {
        self.detail.push_error(text);
        if self.detail.api_error_status.is_none() {
            self.detail.api_error_status = status;
        }
    }

    /// The envelope's error text for a classifier or a message: the result
    /// text when it has any, else the structured errors joined by `"; "`.
    pub fn error_text(&self) -> Option<String> {
        if let Some(text) = self
            .text
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            return Some(text.to_string());
        }
        (!self.detail.errors.is_empty()).then(|| self.detail.errors.join("; "))
    }

    /// `stop_reason: "refusal"`.
    pub fn is_refusal(&self) -> bool {
        self.detail.stop_reason.as_deref() == Some("refusal")
    }

    /// Claude's turn limit.
    pub fn is_max_turns(&self) -> bool {
        self.subtype == SUBTYPE_MAX_TURNS
            || self.detail.terminal_reason.as_deref() == Some("max_turns")
    }

    /// Claude's own spending backstop.
    pub fn is_budget_stop(&self) -> bool {
        self.subtype == SUBTYPE_MAX_BUDGET
            || self.detail.terminal_reason.as_deref() == Some("budget_exhausted")
    }

    /// The envelope texts a failed run is classified from: the result text
    /// (only when the envelope is an error: on success it is the answer,
    /// which is prose) and every structured error.
    fn envelope_texts(&self) -> Vec<&str> {
        let mut texts = Vec::new();
        if self.is_error {
            if let Some(text) = self.text.as_deref() {
                texts.push(text);
            }
        }
        texts.extend(self.detail.errors.iter().map(String::as_str));
        texts
    }
}

/// Tracks a stream's result envelope: the last `Result`, with every
/// `ApiError` seen before or after it merged in. Fed every parsed event, in
/// order, by chat and workflows.
#[derive(Debug, Clone, Default)]
pub struct ResultTracker {
    record: Option<ResultRecord>,
    api_errors: Vec<(String, Option<u16>)>,
}

impl ResultTracker {
    pub fn observe(&mut self, evt: &AgentEvent) {
        match evt {
            AgentEvent::Result { .. } => {
                if let Some(mut record) = ResultRecord::from_event(evt) {
                    for (text, status) in &self.api_errors {
                        record.merge_api_error(text, *status);
                    }
                    self.record = Some(record);
                }
            }
            AgentEvent::ApiError { text, status } => {
                if self.api_errors.len() < crate::agent::RESULT_ERRORS_MAX {
                    self.api_errors.push((text.clone(), *status));
                }
                if let Some(record) = self.record.as_mut() {
                    record.merge_api_error(text, *status);
                }
            }
            _ => {}
        }
    }

    /// The envelope so far. A stream that printed API-error messages but no
    /// result reads as an error envelope carrying them.
    pub fn record(&self) -> Option<ResultRecord> {
        if let Some(record) = &self.record {
            return Some(record.clone());
        }
        if self.api_errors.is_empty() {
            return None;
        }
        let mut record = ResultRecord {
            is_error: true,
            ..ResultRecord::default()
        };
        for (text, status) in &self.api_errors {
            record.merge_api_error(text, *status);
        }
        Some(record)
    }
}

/// Everything [`classify`] looks at, from one exit path.
#[derive(Debug, Clone, Copy)]
pub struct ResultFacts<'a> {
    pub agent: AgentKind,
    pub ended: Ended,
    /// The CLI exited with status 0.
    pub exit_success: bool,
    pub result: Option<&'a ResultRecord>,
    /// The bounded stderr tail; read only when the run failed.
    pub stderr_tail: &'a str,
    /// PR 8b's budget guard ended the run (its notice was recorded).
    pub budget_stop: bool,
    /// PR 15b's watchdog: minutes without output before it ended the run.
    pub stalled_minutes: Option<u64>,
}

/// The note a stalled run is recorded with: PR 15b's watchdog (chat and
/// workflows) uses this one definition (cross-m34 M6).
pub fn stalled_message(minutes: u64) -> String {
    format!("stalled: no output for {minutes}m")
}

/// The note a run ended by its own deadline is recorded with (merged plan
/// slice 3, A·R2): chat turns and, below their fixed hour, workflow runs.
/// Worded apart from [`stalled_message`] so the receipt says which limit
/// ended the run.
pub fn deadline_message(minutes: u64) -> String {
    format!("timed out: the {minutes}m deadline passed")
}

/// Classify how a run ended. See the module doc for the precedence.
pub fn classify(facts: &ResultFacts<'_>) -> ResultKind {
    if facts.stalled_minutes.is_some() {
        return ResultKind::Stalled;
    }
    if facts.budget_stop || facts.result.is_some_and(ResultRecord::is_budget_stop) {
        return ResultKind::BudgetStop;
    }
    // Exhaustive on purpose (no `_` arm): a new ending must be classified.
    match facts.ended {
        // The watchdog's own ending (PR 15b) is a stall even when a caller
        // did not pass its minutes.
        Ended::Stalled => return ResultKind::Stalled,
        Ended::LimitReached => return ResultKind::BudgetStop,
        Ended::Stopped => return ResultKind::Cancelled,
        Ended::TimedOut => return ResultKind::TimedOut,
        Ended::Exited => {}
    }
    if let Some(kind) = facts.result.and_then(structural_kind) {
        return kind;
    }
    let reported_error = facts.result.is_some_and(|r| r.is_error);
    if facts.exit_success && !reported_error {
        return ResultKind::Success;
    }
    let mut texts: Vec<&str> = facts
        .result
        .map(ResultRecord::envelope_texts)
        .unwrap_or_default();
    texts.push(facts.stderr_tail);
    envelope_class(
        facts.agent,
        &texts,
        facts.result.and_then(|r| r.detail.api_error_status),
    )
}

/// The kind of a live result event, from the envelope alone (chat tags its
/// `result` event with it before the process has exited). Structured fields
/// first; then an error envelope's texts; a non-error envelope is a success.
pub fn classify_envelope(agent: AgentKind, record: &ResultRecord) -> ResultKind {
    if record.is_budget_stop() {
        return ResultKind::BudgetStop;
    }
    if let Some(kind) = structural_kind(record) {
        return kind;
    }
    if !record.is_error {
        return ResultKind::Success;
    }
    envelope_class(
        agent,
        &record.envelope_texts(),
        record.detail.api_error_status,
    )
}

/// Refusal, turn limit and denied actions: decided by structured fields
/// alone, whatever the exit code.
fn structural_kind(record: &ResultRecord) -> Option<ResultKind> {
    if record.is_refusal() {
        Some(ResultKind::Refusal)
    } else if record.is_max_turns() {
        Some(ResultKind::MaxTurns)
    } else if !record.detail.denied_actions.is_empty() {
        Some(ResultKind::PermissionDenied)
    } else {
        None
    }
}

/// The error a run whose exit looked successful is recorded with instead,
/// for the kinds that turn such a run into a failure: a refusal
/// ([`REFUSAL_ERROR`]) and actions the permission mode denied (naming them).
/// `None` for every other kind. One line, note-safe.
pub fn override_error(kind: ResultKind, record: Option<&ResultRecord>) -> Option<String> {
    match kind {
        ResultKind::Refusal => Some(REFUSAL_ERROR.to_string()),
        ResultKind::PermissionDenied => {
            let actions = record
                .map(|r| r.detail.denied_actions.join(", "))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "an action".to_string());
            Some(format!(
                "{}: {actions}. This agent cannot approve them in a headless run; use full access or another agent.",
                ResultKind::PermissionDenied.label()
            ))
        }
        _ => None,
    }
}

/// Classify a failed run's envelope texts (and HTTP status): resume failed,
/// then sign-in, usage limit, and rate limit / overload / network.
pub(crate) fn envelope_class(agent: AgentKind, texts: &[&str], status: Option<u16>) -> ResultKind {
    let any = |re: &Regex| texts.iter().any(|text| re.is_match(text));
    if any(resume_failure_re(agent)) {
        ResultKind::ResumeFailed
    } else if matches!(status, Some(401 | 403)) || any(&AUTH_RE) {
        ResultKind::Auth
    } else if agent == AgentKind::Codex && any(&CODEX_OVERLOAD_RE) {
        // Codex can return a generic overload code without an HTTP status.
        // Keep that transient before considering plan/billing wording.
        ResultKind::Transient
    } else if any(&QUOTA_RE) {
        ResultKind::ProviderQuota
    } else if matches!(status, Some(429 | 529 | 500..=504)) || any(&RATE_RE) || any(&NETWORK_RE) {
        ResultKind::Transient
    } else {
        ResultKind::Error
    }
}

/// Whether a run failed because its native session could not be resumed
/// (Claude's unknown session or poisoned message id, Codex's missing
/// thread, Antigravity's unknown conversation). Envelope texts and stderr
/// only, case-insensitive. PR 14b retries such a run once, fresh
/// (`chat::drive_turn_attempts`).
pub fn is_resume_failure(
    agent: AgentKind,
    result: Option<&ResultRecord>,
    stderr_tail: &str,
) -> bool {
    let re = resume_failure_re(agent);
    result.is_some_and(|r| r.envelope_texts().iter().any(|t| re.is_match(t)))
        || re.is_match(stderr_tail)
}

fn resume_failure_re(agent: AgentKind) -> &'static Regex {
    match agent {
        AgentKind::Claude => &CLAUDE_RESUME_RE,
        AgentKind::Codex => &CODEX_RESUME_RE,
        AgentKind::Antigravity => &ANTIGRAVITY_RESUME_RE,
        AgentKind::Grok => &GROK_RESUME_RE,
    }
}

/// Grok (1.0.41, recorded in fixtures/grok/resume-missing.stderr): an
/// unknown id is looked up locally, then remotely, and fails with both lines.
/// 1.0.46 (fixtures/grok/resume-sandbox-mismatch.stderr): a session keeps the
/// sandbox profile it started with and refuses another, so a chat switched
/// across `agent::grok_sandbox` profiles retries fresh under the new one.
/// OC1: an ACP turn's refused `session/load` reads the same way.
static GROK_RESUME_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        r#"(?i)session "[^"\n]{1,80}" not found locally|failed to restore session from remote"#,
        r"|cannot resume this session under sandbox profile",
        // OC1: Hollow's words for a refused ACP `session/load`
        // (`acp_client::LOAD_FAILED`), so an ACP turn retries fresh too.
        r#"|grok couldn't load session "[^"\n]{0,80}" over ACP"#,
    ))
    .expect("GROK_RESUME_RE")
});

/// Claude: the 2.1.283 binary's `No conversation found with session ID:`
/// (any casing), and a poisoned message id (Paperclip's retry case; the
/// wording is unverified, so the pattern stays narrow).
static CLAUDE_RESUME_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        r"(?i)no conversation found with session id",
        r"|previous_message_id\b[^\n]{0,120}\b(?:not found|invalid|unknown|does not exist)",
    ))
    .expect("CLAUDE_RESUME_RE")
});

static CODEX_RESUME_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        r"(?i)no saved session found|no rollout found for (?:thread|conversation) id",
        r"|thread not found|unknown (?:session|thread)\b",
    ))
    .expect("CODEX_RESUME_RE")
});

/// Unverified: no headless Antigravity resume failure has been recorded.
/// 2026-11-20: Hollow's words for a refused ACP `session/load`
/// (`acp_client`), so an ACP turn retries fresh too.
static ANTIGRAVITY_RESUME_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?i)conversation [^\n]{0,80}not found|unknown conversation|antigravity couldn't load session "[^"\n]{0,80}" over ACP"#,
    )
    .expect("ANTIGRAVITY_RESUME_RE")
});

/// An HTTP-ish status code only after an `api`, `http`/`https` (optionally
/// `/1.1`) or `status` prefix, with up to 16 non-digit characters between
/// ("API Error: ", "HTTP/1.1 ", "status_code=", "last status: ").
macro_rules! status_code {
    ($codes:literal) => {
        concat!(
            r"\b(?:api|https?|status)(?:/\d(?:\.\d)?)?[^0-9\n]{0,16}\b(?:",
            $codes,
            r")\b"
        )
    };
}

/// Sign-in failures. Moved verbatim from PR 9b's `workflow_retry`.
pub(crate) static AUTH_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        "(?i)",
        status_code!("401|403"),
        r"|authentication_error|invalid authentication|failed to authenticate",
        r"|not logged in|please run /login|login required|missing bearer",
        r"|invalid (?:x-)?api[ _-]?key|token (?:has )?expired|\bunauthori[sz]ed\b",
        r"|permission_error",
    ))
    .expect("AUTH_RE")
});

/// Plan and billing limits. PR 9b's patterns plus Paperclip's quota
/// phrases (additive).
pub(crate) static QUOTA_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        "(?i)usage[ -]limit|insufficient_quota|exceeded your (?:current )?quota",
        r"|quota exceeded|credit balance is too low|out of credits",
        r"|billing (?:hard )?limit",
        r"|(?:monthly )?spend(?:ing)?[ -]limit",
        r"|you(?:(?:'|’)ve| have) hit your (?:(?:session|weekly|5-hour|five-hour|usage|plan|account|opus|sonnet|haiku) )?limit|session limit (?:reached|exceeded)",
        r"|(?:5-hour|weekly|claude usage) limit reached|out of extra usage",
    ))
    .expect("QUOTA_RE")
});

/// P1d review fix: the credit and billing exhaustion `QUOTA_RE` also counts
/// (prepaid credits, a billing hard limit, an exceeded billing quota). No
/// usage window resets these, so a turn that hits one is never paused to
/// wait for a reset: it fails with the provider's words.
pub(crate) static CREDIT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        "(?i)credit balance (?:is )?too low|out of credits|insufficient (?:credits?|funds|balance)",
        r"|billing (?:hard )?limit|insufficient_quota|exceeded your (?:current )?quota|quota exceeded",
        r"|payment required|\b402\b|(?:monthly )?spend(?:ing)?[ -]limit",
    ))
    .expect("CREDIT_RE")
});

/// Whether `text` names credit or billing exhaustion ([`CREDIT_RE`]).
pub(crate) fn is_credit_exhaustion(text: &str) -> bool {
    CREDIT_RE.is_match(text)
}

/// Rate limits and overload. PR 9b's patterns plus Paperclip's transient
/// phrases (additive).
pub(crate) static RATE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        "(?i)",
        status_code!("429|529"),
        r"|\brate[ _-]?limit|too many requests|overloaded",
        r"|high demand|try again later|temporarily unavailable|overloaded_error|\bat capacity\b",
    ))
    .expect("RATE_RE")
});

/// Network and server errors. Moved verbatim from PR 9b's `workflow_retry`.
pub(crate) static NETWORK_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        "(?i)",
        status_code!("500|502|503|504"),
        r"|\b(?:ECONNRESET|ECONNREFUSED|ECONNABORTED|ETIMEDOUT|EAI_AGAIN|ENOTFOUND",
        r"|ENETUNREACH|EHOSTUNREACH)\b",
        r"|socket hang up|network error|connection (?:reset|refused|closed|error)",
        r"|fetch failed|getaddrinfo|stream disconnected|error sending request",
        r"|request timed out|service unavailable|bad gateway|gateway timeout",
    ))
    .expect("NETWORK_RE")
});

/// Provider overload (R1): HTTP 529 and its wordings. A subset of
/// [`RATE_RE`] without the plan's own rate limit (429).
static OVERLOAD_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        "(?i)",
        status_code!("529"),
        r"|overloaded|high demand|temporarily unavailable|\bat capacity\b",
    ))
    .expect("OVERLOAD_RE")
});

/// Whether a failed run shows an outage rather than its account's limit
/// (R1, chat quota failover): an HTTP 529 or 5xx status, an overload, or a
/// network error in the envelope texts or the stderr tail. Chat never moves
/// a turn to another account on one, so a real outage cannot loop through
/// every account.
pub(crate) fn is_outage(record: Option<&ResultRecord>, stderr_tail: &str) -> bool {
    if matches!(
        record.and_then(|r| r.detail.api_error_status),
        Some(529 | 500..=504)
    ) {
        return true;
    }
    let mut texts = record.map(ResultRecord::envelope_texts).unwrap_or_default();
    texts.push(stderr_tail);
    texts
        .iter()
        .any(|text| OVERLOAD_RE.is_match(text) || NETWORK_RE.is_match(text))
}

/// Codex's overload (overnight 9 A3): `codex_error_info` `server_overloaded`
/// and its "Selected model is at capacity" wording (codex-cli 0.163.0,
/// `fixtures/results/codex-at-capacity.jsonl`).
static CODEX_OVERLOAD_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\bserver_overloaded\b|\bat capacity\b").expect("CODEX_OVERLOAD_RE")
});

/// Whether a failed attempt is a provider outage worth one later resume of
/// the same session (overnight 9 A3, `chat::Next::Backoff`): a transient
/// failure ([`envelope_class`]) that [`is_outage`] calls an outage, or
/// Codex's own overload. A plan's limit, a sign-in failure or a bare 429
/// never is.
pub(crate) fn is_backoff_outage(
    agent: AgentKind,
    record: Option<&ResultRecord>,
    stderr_tail: &str,
) -> bool {
    let mut texts = record.map(ResultRecord::envelope_texts).unwrap_or_default();
    texts.push(stderr_tail);
    let status = record.and_then(|r| r.detail.api_error_status);
    envelope_class(agent, &texts, status) == ResultKind::Transient
        && (is_outage(record, stderr_tail)
            || (agent == AgentKind::Codex && texts.iter().any(|t| CODEX_OVERLOAD_RE.is_match(t))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spend_and_usage_limit_rejections_are_quota_errors() {
        for text in [
            "You've hit your monthly spend limit · raise it at https://claude.ai/settings/usage · your 5-hour limit resets 11:30pm",
            "You reached your monthly spending limit",
            "Monthly spending-limit reached",
            "Account spend limit exceeded",
            "Your usage-limit has been reached",
        ] {
            assert_eq!(envelope_class(AgentKind::Claude, &[text], None), ResultKind::ProviderQuota, "{text}");
            assert_eq!(envelope_class(AgentKind::Claude, &[text], Some(401)), ResultKind::Auth, "{text}");
        }
    }

    #[test]
    fn subscription_quota_and_model_capacity_are_not_the_same_failure() {
        assert_eq!(
            envelope_class(
                AgentKind::Claude,
                &["You have hit your session limit"],
                Some(429)
            ),
            ResultKind::ProviderQuota
        );
        assert_eq!(
            envelope_class(
                AgentKind::Claude,
                &["You've hit your session limit"],
                Some(429)
            ),
            ResultKind::ProviderQuota
        );
        assert_eq!(
            envelope_class(
                AgentKind::Claude,
                &["You've hit your session limit"],
                Some(401)
            ),
            ResultKind::Auth
        );
        assert_eq!(
            envelope_class(
                AgentKind::Claude,
                &["The selected model is at capacity"],
                Some(429)
            ),
            ResultKind::Transient
        );
        assert!(is_outage(None, "The selected model is at capacity"));
        assert_eq!(
            envelope_class(AgentKind::Claude, &["Too many requests"], Some(429)),
            ResultKind::Transient
        );
        assert_eq!(
            envelope_class(
                AgentKind::Claude,
                &["You've hit your rate limit"],
                Some(429)
            ),
            ResultKind::Transient
        );
    }

    use crate::agent::{parse_line, ParseState};

    /// R1: an overload, a 5xx or a network error is an outage; a plan's
    /// usage limit and its 429 are not.
    #[test]
    fn outages_are_told_apart_from_an_accounts_limit() {
        let overloaded = record_of(
            AgentKind::Claude,
            include_str!("../fixtures/results/claude-overloaded.jsonl"),
        );
        assert!(is_outage(overloaded.as_ref(), ""));
        let quota = record_of(
            AgentKind::Claude,
            include_str!("../fixtures/results/claude-quota.jsonl"),
        );
        assert!(!is_outage(quota.as_ref(), ""));
        assert!(is_outage(None, "Error: fetch failed (ECONNRESET)"));
        assert!(is_outage(None, "API Error: 503 service unavailable"));
        assert!(!is_outage(
            None,
            "API Error: 429 {\"type\":\"rate_limit_error\"}"
        ));
        assert!(!is_outage(None, ""));
    }

    /// The tracked envelope of a whole stdout capture.
    fn record_of(agent: AgentKind, stdout: &str) -> Option<ResultRecord> {
        let mut state = ParseState::default();
        let mut tracker = ResultTracker::default();
        for line in stdout.lines() {
            for evt in parse_line(agent, line, &mut state) {
                tracker.observe(&evt);
            }
        }
        tracker.record()
    }

    fn exited(
        agent: AgentKind,
        exit_success: bool,
        record: Option<&ResultRecord>,
        stderr: &str,
    ) -> ResultKind {
        classify(&ResultFacts {
            agent,
            ended: Ended::Exited,
            exit_success,
            result: record,
            stderr_tail: stderr,
            budget_stop: false,
            stalled_minutes: None,
        })
    }

    fn fixture(agent: AgentKind, stdout: &str, exit_success: bool) -> ResultKind {
        let record = record_of(agent, stdout);
        exited(agent, exit_success, record.as_ref(), "")
    }

    #[test]
    fn recorded_streams_classify_as_success_or_budget() {
        use AgentKind::{Claude, Codex};
        assert_eq!(
            fixture(
                Claude,
                include_str!("../fixtures/usage/claude-oneshot.jsonl"),
                true
            ),
            ResultKind::Success
        );
        assert_eq!(
            fixture(
                Claude,
                include_str!("../fixtures/usage/claude-max-budget.jsonl"),
                false
            ),
            ResultKind::BudgetStop
        );
        assert_eq!(
            fixture(
                Codex,
                include_str!("../fixtures/usage/codex-exec.jsonl"),
                true
            ),
            ResultKind::Success
        );
        // "Account unavailable" names no known failure.
        assert_eq!(
            fixture(
                Codex,
                include_str!("../fixtures/usage/codex-failed.jsonl"),
                true
            ),
            ResultKind::Error
        );
        // The live result event of the budget capture says the same.
        let budget = record_of(
            Claude,
            include_str!("../fixtures/usage/claude-max-budget.jsonl"),
        )
        .unwrap();
        assert_eq!(classify_envelope(Claude, &budget), ResultKind::BudgetStop);
    }

    #[test]
    fn refusal_max_turns_quota_transient_auth_fixtures() {
        use AgentKind::{Claude, Codex};
        // A refusal exits 0 and says success: only the structured field tells.
        let refusal = record_of(
            Claude,
            include_str!("../fixtures/results/claude-refusal.jsonl"),
        )
        .unwrap();
        assert!(!refusal.is_error);
        assert_eq!(
            exited(Claude, true, Some(&refusal), ""),
            ResultKind::Refusal
        );
        assert_eq!(
            exited(Claude, false, Some(&refusal), ""),
            ResultKind::Refusal,
            "at any exit code"
        );
        assert_eq!(classify_envelope(Claude, &refusal), ResultKind::Refusal);

        assert_eq!(
            fixture(
                Claude,
                include_str!("../fixtures/results/claude-max-turns.jsonl"),
                false
            ),
            ResultKind::MaxTurns
        );
        assert_eq!(
            fixture(
                Claude,
                include_str!("../fixtures/results/claude-quota.jsonl"),
                false
            ),
            ResultKind::ProviderQuota
        );
        assert_eq!(
            fixture(
                Claude,
                include_str!("../fixtures/results/claude-overloaded.jsonl"),
                false
            ),
            ResultKind::Transient
        );
        assert_eq!(
            fixture(
                Claude,
                include_str!("../fixtures/results/claude-auth.jsonl"),
                false
            ),
            ResultKind::Auth
        );
        // An is_error envelope at exit 0 is still a failure.
        assert_eq!(
            fixture(
                Claude,
                include_str!("../fixtures/results/claude-auth.jsonl"),
                true
            ),
            ResultKind::Auth
        );
        assert_eq!(
            fixture(
                Codex,
                include_str!("../fixtures/results/codex-quota.jsonl"),
                false
            ),
            ResultKind::ProviderQuota
        );
        assert_eq!(
            fixture(
                Codex,
                include_str!("../fixtures/results/codex-high-demand.jsonl"),
                false
            ),
            ResultKind::Transient
        );

        // HTTP status alone decides when the text is unhelpful.
        let mut bare = ResultRecord {
            is_error: true,
            ..ResultRecord::default()
        };
        bare.merge_api_error("API Error", Some(529));
        assert_eq!(classify_envelope(Claude, &bare), ResultKind::Transient);
        bare.detail.api_error_status = Some(403);
        assert_eq!(classify_envelope(Claude, &bare), ResultKind::Auth);
    }

    /// Overnight 9 A3: an OpenAI overload ended a Codex lane as
    /// "usage limit reached", because `QUOTA_RE`'s "at capacity" won before
    /// Transient. It is an outage, the stored error keeps the provider's
    /// words and its `codex_error_info`, and the attempt loop may retry it.
    #[test]
    fn codex_at_capacity_is_transient() {
        let stdout = include_str!("../fixtures/results/codex-at-capacity.jsonl");
        let record = record_of(AgentKind::Codex, stdout).expect("a result");
        assert_eq!(
            record.detail.errors,
            vec![
                "Selected model is at capacity. Please try a different model.".to_string(),
                "server_overloaded".to_string(),
            ]
        );
        assert_eq!(
            classify_envelope(AgentKind::Codex, &record),
            ResultKind::Transient
        );
        assert_eq!(
            fixture(AgentKind::Codex, stdout, false),
            ResultKind::Transient
        );
        assert!(is_backoff_outage(AgentKind::Codex, Some(&record), ""));
        // A Codex plan limit is still a limit, and never retried.
        let quota = record_of(
            AgentKind::Codex,
            include_str!("../fixtures/results/codex-quota.jsonl"),
        );
        assert!(!is_backoff_outage(AgentKind::Codex, quota.as_ref(), ""));
        assert!(is_backoff_outage(
            AgentKind::Codex,
            None,
            "We're currently experiencing high demand"
        ));
    }

    /// Capacity is a provider outage; recorded subscription limits stay quota.
    #[test]
    fn claude_capacity_and_recorded_subscription_quota_are_distinct() {
        use AgentKind::Claude;
        assert_eq!(
            envelope_class(Claude, &["Selected model is at capacity."], None),
            ResultKind::Transient
        );
        assert_eq!(
            fixture(
                Claude,
                include_str!("../fixtures/results/claude-quota.jsonl"),
                false
            ),
            ResultKind::ProviderQuota
        );
        let overloaded = record_of(
            Claude,
            include_str!("../fixtures/results/claude-overloaded.jsonl"),
        );
        assert_eq!(
            classify_envelope(Claude, overloaded.as_ref().unwrap()),
            ResultKind::Transient
        );
        assert!(is_backoff_outage(Claude, overloaded.as_ref(), ""));
        assert!(!is_backoff_outage(
            Claude,
            None,
            "API Error: 429 {\"type\":\"rate_limit_error\"}"
        ));
        assert!(is_backoff_outage(
            Claude,
            None,
            "Selected model is at capacity."
        ));
    }

    /// Paperclip PR 14a, cross-m34 §8: headless Antigravity auto-denies a
    /// write, then reports `SUCCESS` at exit 0 with an empty response.
    #[test]
    fn antigravity_denied_actions_are_not_a_success() {
        let stdout = include_str!("../fixtures/results/antigravity-denied-write.jsonl");
        let record = record_of(AgentKind::Antigravity, stdout).expect("a result");
        assert!(!record.is_error, "the CLI reports SUCCESS");
        assert_eq!(record.detail.denied_actions, vec!["write_file".to_string()]);
        let kind = exited(AgentKind::Antigravity, true, Some(&record), "");
        assert_eq!(kind, ResultKind::PermissionDenied);
        assert!(kind.is_failure());
        assert_eq!(
            classify_envelope(AgentKind::Antigravity, &record),
            ResultKind::PermissionDenied
        );
        let error = override_error(kind, Some(&record)).unwrap();
        assert!(
            error.starts_with("actions were denied by the permission mode: write_file."),
            "{error}"
        );
        assert!(!error.contains('\u{2014}') && !error.contains('<') && !error.contains('>'));
        // A run that denied nothing is an ordinary success.
        let mut clean = record.clone();
        clean.detail.denied_actions.clear();
        assert_eq!(
            exited(AgentKind::Antigravity, true, Some(&clean), ""),
            ResultKind::Success
        );
    }

    #[test]
    fn grok_resume_failure_matches_the_recorded_stderr() {
        let stderr = include_str!("../fixtures/grok/resume-missing.stderr");
        assert!(is_resume_failure(AgentKind::Grok, None, stderr));
        assert!(!is_resume_failure(
            AgentKind::Grok,
            None,
            "Error: rate limited"
        ));
        // Merged plan slice 2: a mode switch across sandbox profiles.
        let mismatch = include_str!("../fixtures/grok/resume-sandbox-mismatch.stderr");
        assert!(is_resume_failure(AgentKind::Grok, None, mismatch));
        // ...and a switch up to auto/bypass, which pass `--sandbox off`.
        let to_off = include_str!("../fixtures/grok/resume-sandbox-to-off.stderr");
        assert!(is_resume_failure(AgentKind::Grok, None, to_off));
    }

    #[test]
    fn resume_failure_shapes() {
        use AgentKind::{Claude, Codex};
        let unknown = record_of(
            Claude,
            include_str!("../fixtures/results/claude-unknown-session.jsonl"),
        );
        assert!(is_resume_failure(Claude, unknown.as_ref(), ""));
        assert_eq!(
            exited(Claude, false, unknown.as_ref(), ""),
            ResultKind::ResumeFailed
        );

        let stderr_only = include_str!("../fixtures/results/claude-unknown-session.stderr.txt");
        assert!(
            is_resume_failure(Claude, None, stderr_only),
            "stderr only, 'session ID:' casing"
        );
        assert_eq!(
            exited(Claude, false, None, stderr_only),
            ResultKind::ResumeFailed
        );

        let poisoned = record_of(
            Claude,
            include_str!("../fixtures/results/claude-poisoned-message-id.jsonl"),
        );
        assert!(is_resume_failure(Claude, poisoned.as_ref(), ""));

        // The API-error wrapper's text is merged into a result that has none.
        let wrapped = record_of(
            Claude,
            include_str!("../fixtures/results/claude-poisoned-api-error.jsonl"),
        )
        .unwrap();
        assert_eq!(wrapped.text, None);
        assert_eq!(wrapped.detail.api_error_status, Some(400));
        assert!(is_resume_failure(Claude, Some(&wrapped), ""));
        assert_eq!(
            classify_envelope(Claude, &wrapped),
            ResultKind::ResumeFailed
        );

        let codex = include_str!("../fixtures/results/codex-unknown-thread.stderr.txt");
        assert!(is_resume_failure(Codex, None, codex));
        assert_eq!(exited(Codex, false, None, codex), ResultKind::ResumeFailed);

        let quota = record_of(
            Claude,
            include_str!("../fixtures/results/claude-quota.jsonl"),
        );
        assert!(!is_resume_failure(Claude, quota.as_ref(), ""));
        // Patterns are per agent.
        assert!(!is_resume_failure(Codex, None, stderr_only));
    }

    #[test]
    fn assistant_prose_never_classifies() {
        let stdout = concat!(
            r#"{"type":"system","subtype":"init","session_id":"id-1"}"#,
            "\n",
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"Your usage limit reached 80%. No conversation found with session ID: x"}]}}"#,
            "\n",
            r#"{"type":"result","subtype":"success","is_error":false,"result":"Your usage limit reached 80%. No conversation found with session ID: x","stop_reason":"end_turn","session_id":"id-1"}"#,
        );
        let record = record_of(AgentKind::Claude, stdout);
        assert_eq!(
            exited(AgentKind::Claude, true, record.as_ref(), ""),
            ResultKind::Success
        );
        assert!(!is_resume_failure(AgentKind::Claude, record.as_ref(), ""));
        // Stderr is read only for a failed run.
        assert_eq!(
            exited(
                AgentKind::Claude,
                true,
                record.as_ref(),
                "API Error: 429 Too Many Requests"
            ),
            ResultKind::Success
        );
        assert_eq!(
            exited(AgentKind::Codex, true, None, "status 429"),
            ResultKind::Success
        );
    }

    #[test]
    fn own_stops_and_stall_take_precedence() {
        let facts = |ended, budget_stop, stalled_minutes| ResultFacts {
            agent: AgentKind::Claude,
            ended,
            exit_success: false,
            result: None,
            stderr_tail: "API Error: 529 overloaded",
            budget_stop,
            stalled_minutes,
        };
        assert_eq!(
            classify(&facts(Ended::Stopped, false, None)),
            ResultKind::Cancelled
        );
        assert_eq!(
            classify(&facts(Ended::TimedOut, false, None)),
            ResultKind::TimedOut
        );
        assert_eq!(
            classify(&facts(Ended::LimitReached, false, None)),
            ResultKind::BudgetStop
        );
        // The guard's notice may arrive with a stop.
        assert_eq!(
            classify(&facts(Ended::Stopped, true, None)),
            ResultKind::BudgetStop
        );
        assert_eq!(
            classify(&facts(Ended::Stopped, false, Some(30))),
            ResultKind::Stalled
        );
        // PR 15b's watchdog ending is a stall with or without its minutes.
        assert_eq!(
            classify(&facts(Ended::Stalled, false, Some(30))),
            ResultKind::Stalled
        );
        assert_eq!(
            classify(&facts(Ended::Stalled, false, None)),
            ResultKind::Stalled
        );
        assert_eq!(stalled_message(30), "stalled: no output for 30m");
        assert_eq!(deadline_message(20), "timed out: the 20m deadline passed");
        // A refusal envelope does not outrank Hollow's own stop.
        let refusal = ResultRecord {
            detail: ResultDetail {
                stop_reason: Some("refusal".into()),
                ..ResultDetail::default()
            },
            ..ResultRecord::default()
        };
        let stopped = ResultFacts {
            result: Some(&refusal),
            ..facts(Ended::Stopped, false, None)
        };
        assert_eq!(classify(&stopped), ResultKind::Cancelled);
        for kind in [
            ResultKind::Cancelled,
            ResultKind::TimedOut,
            ResultKind::BudgetStop,
        ] {
            assert!(kind.is_failure());
        }
        assert!(!ResultKind::Success.is_failure());
    }

    #[test]
    fn labels_are_note_safe_applied_once_and_serde_matches() {
        for kind in ResultKind::ALL {
            let label = kind.label();
            assert!(
                !label.contains('\u{2014}') && !label.contains('<') && !label.contains('>'),
                "{label}"
            );
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                serde_json::json!(kind.as_str())
            );
        }
        let raw = "Claude AI usage limit reached|1790374200";
        let once = ResultKind::ProviderQuota.label_error(raw);
        assert_eq!(once, format!("usage limit reached: {raw}"));
        assert_eq!(
            ResultKind::ProviderQuota.label_error(&once),
            once,
            "never doubled"
        );
        assert_eq!(
            ResultKind::Error.label_error("claude exited 1"),
            "claude exited 1"
        );
        assert_eq!(
            ResultKind::BudgetStop.label_error("Budget limit reached: x"),
            "Budget limit reached: x"
        );
        assert_eq!(
            override_error(ResultKind::Refusal, None).as_deref(),
            Some(REFUSAL_ERROR)
        );
        assert_eq!(override_error(ResultKind::ProviderQuota, None), None);
    }

    /// The frontend mirror (`src/resultKind.ts`) carries every wire value and
    /// label, so the two can't drift apart silently.
    #[test]
    fn labels_match_frontend_mirror() {
        let source = include_str!("../../src/resultKind.ts");
        for kind in ResultKind::ALL {
            assert!(
                source.contains(&format!("{}: '{}'", kind.as_str(), kind.label())),
                "{} missing",
                kind.as_str()
            );
        }
        assert!(source.contains(REFUSAL_ERROR));
        for kind in ResultKind::ALL.into_iter().filter(|k| k.labels_error()) {
            assert!(source.contains(&format!("'{}'", kind.as_str())));
        }
    }

    /// Paperclip PR 9b's table inputs keep their classes under the shared,
    /// extended regexes (the full table lives in `workflow_retry`).
    #[test]
    fn shared_regexes_keep_9b_classes_and_add_paperclip_phrases() {
        assert!(QUOTA_RE.is_match("You’ve hit your weekly limit"));
        assert!(QUOTA_RE.is_match("Session limit reached"));
        assert!(QUOTA_RE.is_match("Claude usage limit reached"));
        assert!(RATE_RE.is_match("We're experiencing high demand"));
        assert!(RATE_RE.is_match(r#"{"type":"overloaded_error"}"#));
        assert!(!RATE_RE.is_match("hit a moderate limit"));
        assert!(!QUOTA_RE.is_match("Reached maximum budget ($0.01)"));
        assert!(!RATE_RE.is_match("syntax error on line 429 of file"));
        assert!(NETWORK_RE.is_match("Error: read ECONNRESET"));
        assert_eq!(
            envelope_class(AgentKind::Claude, &["Error: read ECONNRESET"], None),
            ResultKind::Transient
        );
    }
}
