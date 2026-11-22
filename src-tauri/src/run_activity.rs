//! Paperclip PR 15 (15b): how long a running agent has been quiet, what kind
//! of work it is doing, and the no-output watchdog's decision.
//!
//! One clock per run ([`RunActivity`]), shared by everything that reads it:
//! `process_group::drive` restamps it on every stdout and stderr line
//! ([`RunActivity::touch_output`]) and whenever its once-a-second process
//! sample shows real work ([`RunActivity::touch_process`]: a CPU delta or a
//! change in the run's process set, `process_group::ActivityProbe`); the chat
//! live record (`agent_sessions::attach_activity`, `agent_session_live`)
//! and the activity [`Ticker`] read it; and the watchdog's stall arm asks
//! [`stall_step`] whether the run has been quiet for too long.
//!
//! The watchdog runs on a MONOTONIC clock ([`mono_ms`], milliseconds since a
//! process-wide `Instant`), which does not advance while the machine sleeps
//! (`Instant` on macOS is `CLOCK_UPTIME_RAW`), so closing the lid never
//! counts as a quiet stretch. Wall time is kept only for what a person reads
//! (`lastEventAt`).
//!
//! Progress phases are a CLOSED set read from the tool NAME alone
//! ([`progress_for_tool`]); a tool's arguments are never looked at.
//!
//! Informational only, except the watchdog: a chat turn or workflow run of
//! any agent (merged plan slice 3; Codex only before it) that prints nothing
//! and does no process work for the configured time
//! (`agent_settings::run_limits`, default 30 min, null disables) is ended
//! like a stop, as `process_group::Ended::Stalled`, and recorded as the
//! `stalled` result kind with `result_kind::stalled_message`. It is never
//! continued or retried.
//!
//! Waiting on the user is not silence: while a Hollow approval or question
//! card is pending for the run, a [`CardHold`] pauses the watchdog
//! ([`RunActivity::quiet_since_mono_ms`]), and releasing it restarts the
//! quiet clock, so the time a card waits never counts.

use chrono::{DateTime, TimeZone, Utc};
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// How often the activity [`Ticker`] reports a running run's snapshot.
pub(crate) const TICK: Duration = Duration::from_secs(15);

/// Milliseconds since a process-wide monotonic epoch, plus one, so that 0
/// can mean "never" in [`RunActivity`]'s atomics.
pub(crate) fn mono_ms() -> i64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    let epoch = *EPOCH.get_or_init(Instant::now);
    i64::try_from(epoch.elapsed().as_millis()).unwrap_or(i64::MAX - 1) + 1
}

fn wall_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// What kind of work a run is doing, from its latest tool call. Wire values
/// are camelCase ([`ProgressPhase::as_str`]), mirrored in `src/runActivity.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProgressPhase {
    Researching,
    UsingTools,
    WorkingWithFiles,
    Coordinating,
}

impl ProgressPhase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ProgressPhase::Researching => "researching",
            ProgressPhase::UsingTools => "usingTools",
            ProgressPhase::WorkingWithFiles => "workingWithFiles",
            ProgressPhase::Coordinating => "coordinating",
        }
    }

    fn code(self) -> u8 {
        match self {
            ProgressPhase::Researching => 1,
            ProgressPhase::UsingTools => 2,
            ProgressPhase::WorkingWithFiles => 3,
            ProgressPhase::Coordinating => 4,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(ProgressPhase::Researching),
            2 => Some(ProgressPhase::UsingTools),
            3 => Some(ProgressPhase::WorkingWithFiles),
            4 => Some(ProgressPhase::Coordinating),
            _ => None,
        }
    }
}

/// A tool's family: `mcp:<server>` for an MCP tool (`mcp__<server>__<tool>`,
/// the name Claude uses and PR 13's Codex normalization produces), otherwise
/// the lowercased name.
pub(crate) fn tool_family(name: &str) -> String {
    let lower = name.trim().to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("mcp__") {
        let server = rest.split("__").next().unwrap_or("");
        return format!("mcp:{server}");
    }
    lower
}

/// The progress phase a tool call puts a run in. Reads only the name.
/// Hollow's teamwork server, `Task` and `Agent` coordinate; reading,
/// searching and fetching (Hollow's vault search included) research; file
/// writes and edits (Codex's `file_change`, `apply_patch`) work with files;
/// everything else (shell commands, other MCP servers) uses tools.
pub(crate) fn progress_for_tool(name: &str) -> ProgressPhase {
    match tool_family(name).as_str() {
        "mcp:hollow_team" | "task" | "agent" => ProgressPhase::Coordinating,
        "mcp:hollow_vault" | "read" | "glob" | "grep" | "ls" | "websearch" | "web_search"
        | "webfetch" | "web_fetch" | "notebookread" | "search_vault" => ProgressPhase::Researching,
        "write" | "edit" | "multiedit" | "notebookedit" | "apply_patch" | "applypatch"
        | "file_change" => ProgressPhase::WorkingWithFiles,
        _ => ProgressPhase::UsingTools,
    }
}

/// One run's activity clock. Every field is an atomic, so the stdout loop,
/// the stderr reader task, the ticker and `agent_session_live` share it
/// through an `Arc` without a lock. Monotonic fields are [`mono_ms`] values
/// (0 = never); wall fields are Unix milliseconds (0 = never).
#[derive(Debug, Default)]
pub(crate) struct RunActivity {
    spawned_mono: AtomicI64,
    spawned_wall_ms: AtomicI64,
    first_output_mono: AtomicI64,
    /// The first stdout line and the first assistant text (agent
    /// effectiveness F11): stderr is not the agent's output.
    first_stdout_mono: AtomicI64,
    first_text_mono: AtomicI64,
    /// The first model event and the tool calls before the first text
    /// (OB08a, [`Self::note_event`]).
    first_model_mono: AtomicI64,
    tools_before_text: AtomicU32,
    last_output_mono: AtomicI64,
    last_activity_mono: AtomicI64,
    last_output_wall_ms: AtomicI64,
    progress: AtomicU8,
    /// Pending Hollow cards ([`CardHold`]s alive) for this run.
    card_holds: AtomicU32,
}

/// What the activity events and `agent_session_live` carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActivitySnapshot {
    /// RFC 3339: the last stdout or stderr line, else the spawn; `None`
    /// before the agent started.
    pub last_event_at: Option<String>,
    pub progress: Option<&'static str>,
}

impl RunActivity {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// The agent was spawned. Idempotent: a retried attempt (PR 14b) keeps
    /// the first spawn time.
    pub(crate) fn mark_spawned(&self) {
        let now = mono_ms();
        if self
            .spawned_mono
            .compare_exchange(0, now, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            self.spawned_wall_ms.store(wall_ms(), Ordering::SeqCst);
        }
        // The quiet clock starts at the (latest) spawn.
        self.last_activity_mono.fetch_max(now, Ordering::SeqCst);
    }

    /// A stdout or stderr line arrived.
    pub(crate) fn touch_output(&self) {
        let now = mono_ms();
        let _ = self
            .first_output_mono
            .compare_exchange(0, now, Ordering::SeqCst, Ordering::SeqCst);
        self.last_output_mono.fetch_max(now, Ordering::SeqCst);
        self.last_activity_mono.fetch_max(now, Ordering::SeqCst);
        self.last_output_wall_ms.store(wall_ms(), Ordering::SeqCst);
    }

    /// A stdout line arrived: [`Self::touch_output`], and the first one is
    /// remembered on its own.
    pub(crate) fn touch_stdout(&self) {
        let _ = self.first_stdout_mono.compare_exchange(
            0,
            mono_ms(),
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
        self.touch_output();
    }

    /// The stream carried assistant text; the first is remembered. Tests
    /// stamp it directly; a run's stream goes through [`Self::note_event`].
    #[cfg(test)]
    pub(crate) fn mark_text(&self) {
        self.mark_text_at(mono_ms());
    }

    /// Text is a model event too (OB08a).
    fn mark_text_at(&self, now: i64) {
        let _ = self
            .first_text_mono
            .compare_exchange(0, now, Ordering::SeqCst, Ordering::SeqCst);
        let _ = self
            .first_model_mono
            .compare_exchange(0, now, Ordering::SeqCst, Ordering::SeqCst);
    }

    /// One parsed stream event, read the same way for every agent (F11,
    /// OB08a). The model's events are its thinking
    /// ([`crate::agent::AgentEvent::Thinking`]), a tool call and non-blank
    /// assistant text: the first of them is remembered, and so is the first
    /// text; a tool call before the first text is counted (a subagent's
    /// included). Anything else (usage, session ids, the result, an API-error
    /// wrapper, Codex's `error` item) is not the model's work.
    pub(crate) fn note_event(&self, evt: &crate::agent::AgentEvent) {
        self.note_event_at(evt, mono_ms());
    }

    /// [`Self::note_event`] at `now`, a [`mono_ms`] value.
    fn note_event_at(&self, evt: &crate::agent::AgentEvent, now: i64) {
        use crate::agent::AgentEvent;
        match evt {
            AgentEvent::Thinking => {}
            // Codex's non-fatal `error` item, which `parse_codex`'s catch-all
            // shows as a one-shot tool row named `error`: a notice, not a
            // call the model made.
            AgentEvent::ToolUse { name, .. } if name == "error" => return,
            AgentEvent::ToolUse { .. } => {
                if self.first_text_mono.load(Ordering::SeqCst) == 0 {
                    self.tools_before_text.fetch_add(1, Ordering::SeqCst);
                }
            }
            AgentEvent::AssistantDelta(text) | AgentEvent::AssistantText(text)
                if !text.trim().is_empty() =>
            {
                self.mark_text_at(now);
            }
            _ => return,
        }
        let _ = self
            .first_model_mono
            .compare_exchange(0, now, Ordering::SeqCst, Ordering::SeqCst);
    }

    /// The run's processes did real work without printing (a CPU delta or a
    /// change in its process set). Resets the watchdog, never the quiet label.
    pub(crate) fn touch_process(&self) {
        self.last_activity_mono
            .fetch_max(mono_ms(), Ordering::SeqCst);
    }

    /// The last output line, wall time.
    pub(crate) fn last_output_at(&self) -> Option<DateTime<Utc>> {
        from_wall(self.last_output_wall_ms.load(Ordering::SeqCst))
    }

    /// The last output line, else the spawn, wall time (what "quiet" is
    /// measured from).
    pub(crate) fn last_event_at(&self) -> Option<DateTime<Utc>> {
        self.last_output_at()
            .or_else(|| from_wall(self.spawned_wall_ms.load(Ordering::SeqCst)))
    }

    /// The watchdog's reference: the latest of spawn, output and process
    /// activity, as a [`mono_ms`] value (0 before the spawn).
    pub(crate) fn last_activity_mono_ms(&self) -> i64 {
        self.last_activity_mono.load(Ordering::SeqCst)
    }

    /// What the watchdog measures quiet time from: [`Self::last_activity_mono_ms`],
    /// or now while a card is pending ([`CardHold`]), so a run waiting on
    /// the user is never quiet.
    pub(crate) fn quiet_since_mono_ms(&self) -> i64 {
        if self.card_holds.load(Ordering::SeqCst) > 0 {
            mono_ms()
        } else {
            self.last_activity_mono_ms()
        }
    }

    /// Milliseconds from the spawn to the first output line, if both happened.
    #[cfg(test)]
    pub(crate) fn first_output_after_spawn_ms(&self) -> Option<u64> {
        let spawned = self.spawned_mono.load(Ordering::SeqCst);
        let first = self.first_output_mono.load(Ordering::SeqCst);
        (spawned > 0 && first > 0).then(|| u64::try_from(first - spawned).unwrap_or(0))
    }

    /// Milliseconds from the spawn to the first output line (stdout or
    /// stderr), the first stdout line, the first model event and the first
    /// assistant text, each if both happened; and the tool calls before the
    /// first text (OB08a), once the model was heard from: a run that wrote no
    /// text has the count so far, a run with no model event none.
    pub(crate) fn first_outputs(&self) -> crate::evidence::FirstOutputs {
        let spawned = self.spawned_mono.load(Ordering::SeqCst);
        let after = |at: &AtomicI64| {
            let first = at.load(Ordering::SeqCst);
            (spawned > 0 && first > 0).then(|| u64::try_from(first - spawned).unwrap_or(0))
        };
        let model = after(&self.first_model_mono);
        let tools = self.tools_before_text.load(Ordering::SeqCst);
        crate::evidence::FirstOutputs {
            any: after(&self.first_output_mono),
            stdout: after(&self.first_stdout_mono),
            model,
            text: after(&self.first_text_mono),
            tools_before_text: model.map(|_| tools),
        }
    }

    /// Milliseconds since the spawn, if it happened.
    pub(crate) fn since_spawn_ms(&self) -> Option<u64> {
        let spawned = self.spawned_mono.load(Ordering::SeqCst);
        (spawned > 0).then(|| u64::try_from(mono_ms() - spawned).unwrap_or(0))
    }

    pub(crate) fn set_progress(&self, phase: ProgressPhase) {
        self.progress.store(phase.code(), Ordering::SeqCst);
    }

    pub(crate) fn progress(&self) -> Option<ProgressPhase> {
        ProgressPhase::from_code(self.progress.load(Ordering::SeqCst))
    }

    pub(crate) fn snapshot(&self) -> ActivitySnapshot {
        ActivitySnapshot {
            last_event_at: self.last_event_at().map(|at| at.to_rfc3339()),
            progress: self.progress().map(ProgressPhase::as_str),
        }
    }
}

/// Pauses a run's watchdog while a Hollow approval card waits on the user
/// (merged plan slice 3, A·R2). Held by the approval broker for as long as
/// it waits; dropping it restarts the quiet clock, so the wait is excluded
/// rather than counted. Several can be held at once. A question card never
/// holds it: `ask_user_questions` returns at once and the answer is the
/// user's next turn, so nothing in the run waits on one.
#[derive(Debug)]
pub(crate) struct CardHold(Arc<RunActivity>);

impl CardHold {
    pub(crate) fn new(activity: Arc<RunActivity>) -> Self {
        activity.card_holds.fetch_add(1, Ordering::SeqCst);
        CardHold(activity)
    }
}

impl Drop for CardHold {
    fn drop(&mut self) {
        // Restart the quiet clock first, so the watchdog never sees the
        // released run as quiet since before the card.
        self.0.touch_process();
        self.0.card_holds.fetch_sub(1, Ordering::SeqCst);
    }
}

fn from_wall(ms: i64) -> Option<DateTime<Utc>> {
    if ms <= 0 {
        return None;
    }
    Utc.timestamp_millis_opt(ms).single()
}

/// What the watchdog's stall arm does when its timer fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StallStep {
    /// Quiet for at least the limit: end the run.
    Fire,
    /// Activity happened since the timer was set: wait this many more ms.
    RearmIn(u64),
}

/// The stall decision, over MONOTONIC milliseconds ([`mono_ms`]) only, so a
/// wall-clock change or a sleep can never end a run.
pub(crate) fn stall_step(now_mono_ms: i64, last_activity_mono_ms: i64, limit_ms: i64) -> StallStep {
    let limit = limit_ms.max(1);
    let quiet = now_mono_ms.saturating_sub(last_activity_mono_ms).max(0);
    if quiet >= limit {
        StallStep::Fire
    } else {
        StallStep::RearmIn(u64::try_from(limit - quiet).unwrap_or(1).max(1))
    }
}

/// Whole minutes of a stall limit, for [`crate::result_kind::stalled_message`]
/// (at least 1, so a sub-minute test limit never reads "0m").
pub(crate) fn stall_minutes(limit: Option<Duration>) -> u64 {
    limit.map(|d| (d.as_secs() / 60).max(1)).unwrap_or(1)
}

/// Emits a running run's [`ActivitySnapshot`] every period until dropped.
/// Always a named local of the run it reports, so it can never outlive it.
pub(crate) struct Ticker(tokio::task::JoinHandle<()>);

impl Drop for Ticker {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Start a [`Ticker`]: the first snapshot one `period` from now, then every
/// `period`. Needs a Tokio runtime (Tauri's, or a test's).
pub(crate) fn spawn_ticker<F>(activity: Arc<RunActivity>, period: Duration, emit: F) -> Ticker
where
    F: Fn(ActivitySnapshot) + Send + Sync + 'static,
{
    let period = period.max(Duration::from_millis(1));
    Ticker(tokio::spawn(async move {
        let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            emit(activity.snapshot());
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn stall_step_uses_monotonic_quiet_time() {
        let limit = 30 * 60 * 1000;
        let now = 1_000_000_000;
        assert_eq!(stall_step(now, now - limit, limit), StallStep::Fire);
        assert_eq!(stall_step(now, now - limit - 5, limit), StallStep::Fire);
        assert_eq!(
            stall_step(now, now - limit / 2, limit),
            StallStep::RearmIn((limit / 2) as u64)
        );
        // Activity stamped "after" now (a race between the arm and a line)
        // is never negative quiet time.
        assert_eq!(
            stall_step(now, now + 10, limit),
            StallStep::RearmIn(limit as u64)
        );
        // The inputs are monotonic: the activity clock itself reads only the
        // monotonic fields for the watchdog.
        let activity = RunActivity::new();
        assert_eq!(activity.last_activity_mono_ms(), 0);
        activity.mark_spawned();
        let spawned = activity.last_activity_mono_ms();
        assert!(spawned > 0);
        std::thread::sleep(Duration::from_millis(5));
        activity.touch_process();
        assert!(activity.last_activity_mono_ms() > spawned);
        assert!(
            activity.last_output_at().is_none(),
            "process work is not output"
        );
        assert_eq!(stall_minutes(Some(Duration::from_secs(30 * 60))), 30);
        assert_eq!(stall_minutes(Some(Duration::from_millis(400))), 1);
    }

    /// Merged plan slice 3 (A·R2): a pending card makes the run never
    /// quiet, and releasing it restarts the quiet clock (the wait is
    /// excluded, not counted). Holds nest.
    #[test]
    fn a_pending_card_pauses_the_quiet_clock() {
        let limit = 50;
        let activity = Arc::new(RunActivity::new());
        activity.mark_spawned();
        let spawned = activity.last_activity_mono_ms();
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(
            stall_step(mono_ms(), activity.quiet_since_mono_ms(), limit),
            StallStep::Fire,
            "unheld, the run is quiet past the limit"
        );
        let first = CardHold::new(Arc::clone(&activity));
        let second = CardHold::new(Arc::clone(&activity));
        std::thread::sleep(Duration::from_millis(80));
        assert!(matches!(
            stall_step(mono_ms(), activity.quiet_since_mono_ms(), limit),
            StallStep::RearmIn(_)
        ));
        drop(first);
        assert!(matches!(
            stall_step(mono_ms(), activity.quiet_since_mono_ms(), limit),
            StallStep::RearmIn(_)
        ));
        drop(second);
        assert!(
            activity.last_activity_mono_ms() > spawned + 100,
            "the release restarted the quiet clock"
        );
        assert!(matches!(
            stall_step(mono_ms(), activity.quiet_since_mono_ms(), limit),
            StallStep::RearmIn(_)
        ));
        assert_eq!(activity.card_holds.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn progress_for_tool_maps_names_to_a_closed_set() {
        use ProgressPhase::*;
        for (name, want) in [
            ("Read", Researching),
            ("Glob", Researching),
            ("Grep", Researching),
            ("LS", Researching),
            ("WebSearch", Researching),
            ("WebFetch", Researching),
            ("NotebookRead", Researching),
            ("mcp__hollow_vault__search_vault", Researching),
            ("Write", WorkingWithFiles),
            ("Edit", WorkingWithFiles),
            ("MultiEdit", WorkingWithFiles),
            ("NotebookEdit", WorkingWithFiles),
            ("Task", Coordinating),
            ("Agent", Coordinating),
            ("mcp__hollow_team__ask_agent", Coordinating),
            ("Bash", UsingTools),
            ("mcp__hollow_tasks__create_task", UsingTools),
            ("mcp__hollow_computer__open_url", UsingTools),
            ("", UsingTools),
            // SYNTHETIC: no recorded Codex fixture has these item types;
            // `parse_codex` surfaces them under their item type.
            ("web_search", Researching),
            ("file_change", WorkingWithFiles),
            // The legacy MCP pair without server/tool names (PR 13 keeps
            // it as a fallback) has no server to key on.
            ("mcp_tool_call", UsingTools),
        ] {
            assert_eq!(progress_for_tool(name), want, "{name}");
        }
        assert_eq!(
            tool_family("mcp__hollow_team__ask_agent"),
            "mcp:hollow_team"
        );
        assert_eq!(tool_family("Read"), "read");
        for phase in [Researching, UsingTools, WorkingWithFiles, Coordinating] {
            assert_eq!(ProgressPhase::from_code(phase.code()), Some(phase));
        }
        assert_eq!(
            [Researching, UsingTools, WorkingWithFiles, Coordinating].map(ProgressPhase::as_str),
            [
                "researching",
                "usingTools",
                "workingWithFiles",
                "coordinating"
            ]
        );
    }

    /// OB08a: the first model event, the first text and the tool calls
    /// before it, from a fake stream on an injected clock: thinking 2 s after
    /// the spawn, a tool call, then text at 10 s. Later events never move
    /// the firsts, and a tool call after the text is not counted.
    #[test]
    fn first_model_event_and_tools_before_text_from_a_fake_stream() {
        use crate::agent::AgentEvent;
        // Spawned at 1 000 on the monotonic clock; `ms` after the spawn.
        let spawned = 1_000;
        let at = |activity: &RunActivity, evt: AgentEvent, ms: i64| {
            activity.note_event_at(&evt, spawned + ms);
        };
        let clock = || {
            let activity = RunActivity::new();
            activity.spawned_mono.store(spawned, Ordering::SeqCst);
            activity
        };
        let tool = || AgentEvent::ToolUse {
            id: "t1".into(),
            name: "Read".into(),
            input: serde_json::Value::Null,
        };
        let text = |t: &str| AgentEvent::AssistantDelta(t.into());
        let activity = clock();
        // Not the model's work: a boundary, a blank delta, an API error,
        // Codex's `error` item (a tool row named `error`).
        let api_error = AgentEvent::ApiError {
            text: "API Error: 529".into(),
            status: Some(529),
        };
        let codex_error = AgentEvent::ToolUse {
            id: "item_9".into(),
            name: "error".into(),
            input: serde_json::Value::Null,
        };
        at(&activity, AgentEvent::TurnBoundary, 500);
        at(&activity, text("  "), 900);
        at(&activity, api_error, 1_000);
        at(&activity, codex_error, 1_500);
        let none = activity.first_outputs();
        assert_eq!(
            (none.model, none.text, none.tools_before_text),
            (None, None, None),
            "no model event yet"
        );
        at(&activity, AgentEvent::Thinking, 2_000);
        at(&activity, tool(), 4_000);
        // A run that ended here wrote no text: the count so far.
        let silent = activity.first_outputs();
        assert_eq!(
            (silent.model, silent.text, silent.tools_before_text),
            (Some(2_000), None, Some(1))
        );
        at(&activity, text("Done."), 10_000);
        at(&activity, tool(), 11_000);
        at(&activity, AgentEvent::Thinking, 12_000);
        at(&activity, AgentEvent::AssistantText("More.".into()), 13_000);
        let first = activity.first_outputs();
        assert_eq!(first.model, Some(2_000));
        assert_eq!(first.text, Some(10_000));
        assert_eq!(first.tools_before_text, Some(1));
        assert_eq!((first.any, first.stdout), (None, None), "no lines");

        // A tool call first is the first model event; so is text alone.
        let tools_first = clock();
        at(&tools_first, tool(), 3_000);
        assert_eq!(tools_first.first_outputs().model, Some(3_000));
        let text_only = clock();
        at(&text_only, AgentEvent::AssistantText("Hi.".into()), 700);
        let first = text_only.first_outputs();
        assert_eq!(
            (first.model, first.text, first.tools_before_text),
            (Some(700), Some(700), Some(0))
        );
        // Before the spawn nothing is measured.
        let unspawned = RunActivity::new();
        unspawned.note_event_at(&AgentEvent::Thinking, spawned);
        assert_eq!(unspawned.first_outputs().tools_before_text, None);
    }

    #[tokio::test]
    async fn activity_clock_and_ticker() {
        let activity = Arc::new(RunActivity::new());
        assert_eq!(
            activity.snapshot(),
            ActivitySnapshot {
                last_event_at: None,
                progress: None
            }
        );
        activity.mark_spawned();
        let spawned_at = activity
            .last_event_at()
            .expect("the spawn is the first event");
        assert!(activity.first_output_after_spawn_ms().is_none());
        std::thread::sleep(Duration::from_millis(3));
        activity.touch_output();
        let first = activity
            .first_output_after_spawn_ms()
            .expect("first output");
        std::thread::sleep(Duration::from_millis(3));
        activity.touch_output();
        assert_eq!(
            activity.first_output_after_spawn_ms(),
            Some(first),
            "set once"
        );
        assert!(activity.last_output_at().unwrap() >= spawned_at);
        assert!(activity.since_spawn_ms().unwrap() >= first);
        // A second spawn (a retried attempt) keeps the first spawn time.
        activity.mark_spawned();
        assert_eq!(activity.first_output_after_spawn_ms(), Some(first));
        activity.set_progress(ProgressPhase::WorkingWithFiles);
        assert_eq!(activity.progress(), Some(ProgressPhase::WorkingWithFiles));
        assert_eq!(activity.snapshot().progress, Some("workingWithFiles"));

        let seen = Arc::new(Mutex::new(Vec::new()));
        let ticker = {
            let seen = Arc::clone(&seen);
            spawn_ticker(
                Arc::clone(&activity),
                Duration::from_millis(30),
                move |snap| {
                    seen.lock().unwrap().push(snap);
                },
            )
        };
        tokio::time::sleep(Duration::from_millis(150)).await;
        drop(ticker);
        let count = seen.lock().unwrap().len();
        assert!(count >= 2, "got {count} snapshots");
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .all(|s| s.last_event_at.is_some()));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(seen.lock().unwrap().len(), count, "none after drop");

        assert_eq!(
            crate::result_kind::stalled_message(30),
            "stalled: no output for 30m"
        );
    }
}
