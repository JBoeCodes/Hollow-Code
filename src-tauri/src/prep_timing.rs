//! Preparation phase timings for one run (agent effectiveness F8, wave 3 ·
//! lane O, PR O1: measure first).
//!
//! Before this a run's manifest timed only retrieval and connector
//! selection as two lumps (`preparation.retrievalMs`, `connectorsMs`), and
//! the receipt's `phaseTimings.spawnMs` was all of preparation in one
//! number. Now every preparation step a chat turn or a workflow run takes
//! is a mark on the monotonic clock ([`Mark`]: when it started, how long it
//! took), kept in memory on the run's manifest ([`PrepMarks`]) while the run
//! prepares, and turned into the manifest's `preparation.timeline` when the
//! receipt is written ([`timeline`]): one entry per [`Step`], each with its
//! start offset from admission and its duration, both `null` for a step that
//! didn't run on this run.
//!
//! Nothing here changes what a run does or in which order: the marks are
//! read from clocks the steps already pass, and nothing decides on them.
//! Names and milliseconds only, never text.
use crate::run_manifest::{PhaseTiming, PrepTimeline};
use std::time::{Duration, Instant};

/// One step: when it started on the monotonic clock and how long it took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Mark {
    pub started: Instant,
    pub ms: u64,
}

impl Mark {
    /// A step that started at `started` and ends now.
    pub(crate) fn since(started: Instant) -> Self {
        Mark {
            started,
            ms: crate::evidence::ms_since(started),
        }
    }

    fn end(&self) -> Instant {
        self.started + Duration::from_millis(self.ms)
    }
}

/// A clock reading carried on a serialized record (`RunManifest`,
/// `JevUse`, `RetrievalStats`, `PhaseTimings`): in memory only. Never
/// serialized (each such field is `#[serde(skip)]`) and ignored by
/// equality, so a record read back from disk equals the one written.
#[derive(Debug, Clone, Copy, Default)]
pub struct InMemory<T>(pub T);

impl<T> PartialEq for InMemory<T> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<T> Eq for InMemory<T> {}

/// The preparation steps, in the order the timeline lists the ones that
/// didn't run (the ones that ran are listed by start). Nested steps
/// (`retrievalSemantic` and `retrievalJev` inside `retrieval`,
/// `microsoftAuth` and `connectorsJev` inside `connectors`, `contextCompile`
/// inside a chat's `promptAssembly`, `codexInventory` inside `command`) are
/// listed with their parent's time included, never subtracted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Step {
    /// Admission's vault snapshot (`RunLease::baseline_ms`); the clock's 0.
    Admission,
    /// The request's resolution and approved recall (`TurnRequest::recall`).
    Recall,
    /// A workspace's envelope and attached-source chunks
    /// (`workspaces::context_for`); `null` outside a workspace.
    AttachedSources,
    /// The retrieval broker and its rerank (`prioritized_context`).
    Retrieval,
    /// The broker's semantic half, when it ran (embedding and lookup).
    RetrievalSemantic,
    /// Jev's passage rerank, when a request was sent.
    RetrievalJev,
    /// Library load, eligibility and "when relevant" selection
    /// (`connectors::attach`).
    Connectors,
    /// The Microsoft 365 token refresh inside it, when that connector was
    /// eligible.
    MicrosoftAuth,
    /// Jev's connector relevance request, when one was sent.
    ConnectorsJev,
    /// The host context compiler (a chat's `compile_turn`; a workflow's
    /// operator profile, `operator::compile`, when the profile is on).
    ContextCompile,
    /// Everything between connector selection and the command: the
    /// connectors note, the owned servers, the appended layers, effort (and
    /// its once-per-binary probe), the fingerprint and resume plan, the
    /// compiled body and the wrap (chat); the tool blocks, the spec and the
    /// budget probe (workflow).
    PromptAssembly,
    /// The pre-run repository snapshot (`repo_diff::begin`): beside
    /// retrieval and connector selection in a chat turn (S29), after the
    /// spec in a workflow run.
    RepoSnapshot,
    /// `agent::build_command` for the first attempt.
    Command,
    /// Codex's native MCP inventory (`codex mcp list`) inside it.
    CodexInventory,
    /// The spawn call → the activity clock's spawn stamp (when `drive`
    /// takes the process and its prompt is written). A steerable Claude
    /// turn's agent starts early (run 15 S2, `chat::EarlyStart`): then this
    /// spans its own start-up, beside the rest of preparation.
    Spawn,
    /// The spawn → the first stdout line.
    FirstStdout,
    /// The spawn → the first assistant text.
    FirstText,
}

impl Step {
    pub(crate) const ALL: [Step; 17] = [
        Step::Admission,
        Step::Recall,
        Step::AttachedSources,
        Step::Retrieval,
        Step::RetrievalSemantic,
        Step::RetrievalJev,
        Step::Connectors,
        Step::MicrosoftAuth,
        Step::ConnectorsJev,
        Step::ContextCompile,
        Step::PromptAssembly,
        Step::RepoSnapshot,
        Step::Command,
        Step::CodexInventory,
        Step::Spawn,
        Step::FirstStdout,
        Step::FirstText,
    ];

    /// The step's name in the manifest.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Step::Admission => "admission",
            Step::Recall => "recall",
            Step::AttachedSources => "attachedSources",
            Step::Retrieval => "retrieval",
            Step::RetrievalSemantic => "retrievalSemantic",
            Step::RetrievalJev => "retrievalJev",
            Step::Connectors => "connectors",
            Step::MicrosoftAuth => "microsoftAuth",
            Step::ConnectorsJev => "connectorsJev",
            Step::ContextCompile => "contextCompile",
            Step::PromptAssembly => "promptAssembly",
            Step::RepoSnapshot => "repoSnapshot",
            Step::Command => "command",
            Step::CodexInventory => "codexInventory",
            Step::Spawn => "spawn",
            Step::FirstStdout => "firstStdout",
            Step::FirstText => "firstText",
        }
    }
}

/// The marks one run's preparation recorded so far. The first mark per step
/// wins (a retried attempt's command is not preparation).
#[derive(Debug, Clone, Default)]
pub(crate) struct PrepMarks {
    marks: Vec<(Step, Mark)>,
    /// When the spawn was called; its end is the activity clock's stamp,
    /// known only once the run has driven.
    spawn_called: Option<Instant>,
}

impl PrepMarks {
    pub(crate) fn record(&mut self, step: Step, mark: Mark) {
        if !self.marks.iter().any(|(s, _)| *s == step) {
            self.marks.push((step, mark));
        }
    }

    /// Record `mark` for `step` when there is one.
    pub(crate) fn record_some(&mut self, step: Step, mark: Option<Mark>) {
        if let Some(mark) = mark {
            self.record(step, mark);
        }
    }

    /// The agent's spawn is being called now (the first attempt's).
    pub(crate) fn spawn_called(&mut self, at: Instant) {
        self.spawn_called.get_or_insert(at);
    }

    /// Fold `other`'s marks in (marks taken before the manifest existed).
    pub(crate) fn extend(&mut self, other: PrepMarks) {
        for (step, mark) in other.marks {
            self.record(step, mark);
        }
        if let Some(at) = other.spawn_called {
            self.spawn_called(at);
        }
    }

    pub(crate) fn get(&self, step: Step) -> Option<Mark> {
        self.marks
            .iter()
            .find_map(|(s, mark)| (*s == step).then_some(*mark))
    }
}

/// What the run's receipt timings know about admission and the agent's
/// spawn (`evidence::PhaseTimings`): admission completed at `admitted`
/// after a `baseline_ms` snapshot; the agent spawned `spawn_ms` after
/// admission and printed its first stdout line and first text that long
/// after the spawn.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RunClock {
    pub admitted: Option<Instant>,
    pub baseline_ms: Option<u64>,
    pub spawn_ms: Option<u64>,
    pub first_stdout_ms: Option<u64>,
    pub first_text_ms: Option<u64>,
}

/// The run's timeline: `None` without an admission clock (a run that was
/// never admitted, a manifest built outside a run). The clock's 0 is
/// admission's start (its completion less its snapshot), every step is
/// listed (17), those that ran by start offset (ties in [`Step::ALL`]
/// order), then those that didn't, with `null` start and duration.
/// `totalMs` is admission's start → the spawn stamp (`null` when the agent
/// never spawned).
pub(crate) fn timeline(marks: &PrepMarks, clock: &RunClock) -> Option<PrepTimeline> {
    let admitted = clock.admitted?;
    let baseline = clock.baseline_ms.unwrap_or(0);
    let origin = admitted
        .checked_sub(Duration::from_millis(baseline))
        .unwrap_or(admitted);
    let offset = |at: Instant| {
        u64::try_from(at.saturating_duration_since(origin).as_millis()).unwrap_or(u64::MAX)
    };
    // The spawn stamp, on the same clock: `spawn_ms` after admission.
    let spawned = clock
        .spawn_ms
        .map(|ms| admitted + Duration::from_millis(ms));
    let span = |step: Step| -> (Option<u64>, Option<u64>) {
        match step {
            Step::Admission => (Some(0), clock.baseline_ms),
            Step::Spawn => match (marks.spawn_called, spawned) {
                (Some(called), Some(at)) => (
                    Some(offset(called)),
                    Some(
                        u64::try_from(at.saturating_duration_since(called).as_millis())
                            .unwrap_or(u64::MAX),
                    ),
                ),
                _ => (None, None),
            },
            Step::FirstStdout | Step::FirstText => {
                let after = if step == Step::FirstStdout {
                    clock.first_stdout_ms
                } else {
                    clock.first_text_ms
                };
                match (spawned, after) {
                    (Some(at), Some(ms)) => (Some(offset(at)), Some(ms)),
                    _ => (None, None),
                }
            }
            _ => match marks.get(step) {
                Some(mark) => (Some(offset(mark.started)), Some(mark.ms)),
                None => (None, None),
            },
        }
    };
    let mut phases: Vec<(usize, PhaseTiming)> = Step::ALL
        .iter()
        .enumerate()
        .map(|(i, step)| {
            let (start_ms, duration_ms) = span(*step);
            (
                i,
                PhaseTiming {
                    name: step.name().to_string(),
                    start_ms,
                    duration_ms,
                },
            )
        })
        .collect();
    phases.sort_by_key(|(i, p)| (p.start_ms.is_none(), p.start_ms.unwrap_or(0), *i));
    Some(PrepTimeline {
        origin: "admission".into(),
        total_ms: spawned.map(offset),
        phases: phases.into_iter().map(|(_, p)| p).collect(),
    })
}

/// Build the command for `spec`'s first attempt and time it: the build,
/// and Codex's native MCP inventory inside it.
pub(crate) fn timed_command(
    kind: crate::agent::AgentKind,
    spec: &crate::agent::RunSpec,
) -> (
    tokio::process::Command,
    Mark,
    Option<crate::codex_inventory::Inventory>,
) {
    let started = Instant::now();
    let (cmd, inventory) = crate::agent::build_command_timed(kind, spec);
    (cmd, Mark::since(started), inventory)
}

/// When a mark ends (for a step that starts where another ends).
pub(crate) fn end_of(mark: &Mark) -> Instant {
    mark.end()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(origin: Instant, ms: u64) -> Instant {
        origin + Duration::from_millis(ms)
    }

    fn mark(origin: Instant, start: u64, ms: u64) -> Mark {
        Mark {
            started: at(origin, start),
            ms,
        }
    }

    fn find<'a>(t: &'a PrepTimeline, name: &str) -> &'a PhaseTiming {
        t.phases
            .iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("{name} is listed"))
    }

    /// A chat turn's marks become a timeline from admission's start: every
    /// step listed once, those that ran ordered by start with their
    /// durations, nested steps inside their parents, the spawn and first
    /// outputs from the receipt's clock, and `null` for steps that didn't
    /// run.
    #[test]
    fn a_chat_turns_marks_become_an_ordered_timeline_with_nulls_for_skipped_steps() {
        // Admission's snapshot took 40 ms and completed at `admitted`.
        let origin = Instant::now();
        let admitted = at(origin, 40);
        let mut marks = PrepMarks::default();
        marks.record(Step::Recall, mark(origin, 41, 5));
        marks.record(Step::Retrieval, mark(origin, 50, 300));
        marks.record(Step::RetrievalJev, mark(origin, 100, 250));
        marks.record(Step::Connectors, mark(origin, 350, 4010));
        marks.record(Step::ConnectorsJev, mark(origin, 355, 4000));
        // Since S29 the snapshot starts beside retrieval, not after
        // assembly; recorded after the others here (production records
        // it first), it is still listed by its start.
        marks.record(Step::RepoSnapshot, mark(origin, 50, 278));
        marks.record(Step::PromptAssembly, mark(origin, 4360, 20));
        marks.record(Step::ContextCompile, mark(origin, 4370, 3));
        marks.record(Step::Command, mark(origin, 4383, 1));
        // A retried attempt's command is not preparation.
        marks.record(Step::Command, mark(origin, 9000, 7));
        marks.spawn_called(at(origin, 4384));
        let clock = RunClock {
            admitted: Some(admitted),
            baseline_ms: Some(40),
            // Spawned 4346 ms after admission: 4386 from its start.
            spawn_ms: Some(4346),
            first_stdout_ms: Some(30),
            first_text_ms: Some(900),
        };
        let t = timeline(&marks, &clock).expect("an admitted run has a timeline");
        assert_eq!(t.origin, "admission");
        assert_eq!(t.total_ms, Some(4386));
        assert_eq!(t.phases.len(), Step::ALL.len(), "every step is listed");
        let names: Vec<&str> = t.phases.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "admission",
                "recall",
                "retrieval",
                // Starts with retrieval: ties go in step order.
                "repoSnapshot",
                "retrievalJev",
                "connectors",
                "connectorsJev",
                "promptAssembly",
                "contextCompile",
                "command",
                "spawn",
                "firstStdout",
                "firstText",
                // Didn't run, in step order.
                "attachedSources",
                "retrievalSemantic",
                "microsoftAuth",
                "codexInventory",
            ]
        );
        let ran: Vec<u64> = t.phases.iter().filter_map(|p| p.start_ms).collect();
        assert!(ran.windows(2).all(|w| w[0] <= w[1]), "{ran:?}");
        assert_eq!(find(&t, "admission").start_ms, Some(0));
        assert_eq!(find(&t, "admission").duration_ms, Some(40));
        assert_eq!(find(&t, "connectors").start_ms, Some(350));
        assert_eq!(find(&t, "connectors").duration_ms, Some(4010));
        assert_eq!(
            find(&t, "command").start_ms,
            Some(4383),
            "the first attempt's"
        );
        // The snapshot runs beside retrieval and ends before prompt
        // assembly starts (S29).
        let (snapshot, assembly) = (find(&t, "repoSnapshot"), find(&t, "promptAssembly"));
        assert_eq!(snapshot.start_ms, find(&t, "retrieval").start_ms);
        let snapshot_end = snapshot.start_ms.unwrap() + snapshot.duration_ms.unwrap();
        assert!(snapshot_end <= assembly.start_ms.unwrap());
        assert_eq!(find(&t, "spawn").start_ms, Some(4384));
        assert_eq!(find(&t, "spawn").duration_ms, Some(2));
        assert_eq!(find(&t, "firstStdout").start_ms, Some(4386));
        assert_eq!(find(&t, "firstStdout").duration_ms, Some(30));
        assert_eq!(find(&t, "firstText").duration_ms, Some(900));
        // Nested steps lie inside their parents.
        for (child, parent) in [
            ("retrievalJev", "retrieval"),
            ("connectorsJev", "connectors"),
            ("contextCompile", "promptAssembly"),
        ] {
            let (c, p) = (find(&t, child), find(&t, parent));
            let (cs, ps) = (c.start_ms.unwrap(), p.start_ms.unwrap());
            assert!(ps <= cs, "{child} starts inside {parent}");
            assert!(
                cs + c.duration_ms.unwrap() <= ps + p.duration_ms.unwrap(),
                "{child} ends inside {parent}"
            );
        }
        for skipped in [
            "attachedSources",
            "retrievalSemantic",
            "microsoftAuth",
            "codexInventory",
        ] {
            let p = find(&t, skipped);
            assert_eq!((p.start_ms, p.duration_ms), (None, None), "{skipped}");
        }
        // Serialized, a skipped step's start and duration are explicit nulls.
        let json = serde_json::to_value(&t).unwrap();
        let skipped = json["phases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "codexInventory")
            .unwrap();
        assert!(skipped["startMs"].is_null() && skipped.get("startMs").is_some());
        assert!(skipped["durationMs"].is_null() && skipped.get("durationMs").is_some());
        assert_eq!(json["totalMs"], 4386);
    }

    /// A run that never spawned has its preparation steps and nulls for the
    /// spawn, the first outputs and the total; a run with no admission clock
    /// has no timeline at all.
    #[test]
    fn an_unspawned_run_has_null_spawn_and_total_and_an_unadmitted_one_has_none() {
        let origin = Instant::now();
        let mut marks = PrepMarks::default();
        marks.record(Step::Retrieval, mark(origin, 3, 10));
        marks.spawn_called(at(origin, 20));
        let clock = RunClock {
            admitted: Some(origin),
            baseline_ms: None,
            ..Default::default()
        };
        let t = timeline(&marks, &clock).unwrap();
        assert_eq!(t.total_ms, None);
        assert_eq!(find(&t, "admission").start_ms, Some(0));
        assert_eq!(find(&t, "admission").duration_ms, None, "not measured");
        assert_eq!(find(&t, "retrieval").start_ms, Some(3));
        for step in ["spawn", "firstStdout", "firstText"] {
            let p = find(&t, step);
            assert_eq!((p.start_ms, p.duration_ms), (None, None), "{step}");
        }
        assert!(timeline(&marks, &RunClock::default()).is_none());
    }

    #[test]
    fn marks_taken_before_the_manifest_fold_in_and_the_first_wins() {
        let origin = Instant::now();
        let mut early = PrepMarks::default();
        early.record(Step::Recall, mark(origin, 1, 2));
        early.spawn_called(at(origin, 5));
        let mut marks = PrepMarks::default();
        marks.record(Step::Recall, mark(origin, 9, 9));
        marks.extend(early);
        assert_eq!(marks.get(Step::Recall), Some(mark(origin, 9, 9)));
        assert_eq!(marks.spawn_called, Some(at(origin, 5)));
        marks.record_some(Step::Spawn, None);
        assert_eq!(marks.get(Step::Spawn), None);
    }

    #[test]
    fn in_memory_values_never_serialize_and_never_differ() {
        assert_eq!(InMemory(Some(Instant::now())), InMemory(None));
        let names: Vec<&str> = Step::ALL.iter().map(|s| s.name()).collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "{names:?}");
    }
}
