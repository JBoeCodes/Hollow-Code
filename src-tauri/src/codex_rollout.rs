//! Overnight 9 A10: what a Codex turn's own rollout file says about its
//! context. `codex exec --json` streams no per-call usage (only the turn's
//! summed total at `turn.completed`) and no compaction event, so the
//! context gauge had no figure for Codex. Codex records both in its
//! rollout, `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<time>-<thread id>.jsonl`
//! (`CODEX_HOME` defaults to `~/.codex`; a self-hosted Codex run's is its
//! entry's own home, self-hosted models Q9, [`Watch::for_spec`]):
//!
//! - `{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":…},"model_context_window":…}}}`
//!   after each model call (the first of a file carries `info: null`);
//! - `{"type":"compacted","payload":{…}}` per compaction.
//!
//! **Whitelist.** A rollout holds the whole transcript and reaches several
//! MB, so [`Watch`] reads only what was appended since its offset, and
//! parses only lines whose top-level `type` (found in the line's first
//! bytes, by bytes: model text can put any character there) is `event_msg`
//! naming `token_count`, or `compacted`, or `turn_context`, of which only
//! the model id is kept (overnight 9 A13: it prices a default-model run at
//! the person's row for it); a transcript line is never parsed or kept.
//!
//! **Shape, not version.** The installed codex-cli (0.163.0) isn't in
//! `codex_probe::VERIFIED`, so nothing here gates on the version: a
//! `token_count` line counts only with a numeric `input_tokens` and
//! `model_context_window`, a `compacted` line only with an object payload;
//! any other shape is ignored silently and the gauge stays as it was.
//!
//! **Baseline.** The file is found and read when `thread.started` names the
//! thread (`AgentEvent::Session`): what it held then sets the figure (a
//! resumed thread's context shows at once) but not the compaction count, so
//! only compactions written during this turn count. A new thread's file
//! that appears only later is all this turn's. Codex appends a resumed
//! thread to its original file. The chat reads once more after the process
//! exits, for lines written after `turn.completed`.
//!
//! **Tail.** The baseline runs on the stream loop at every attempt, and a
//! rollout reaches tens of MB, so it reads only the file's tail
//! ([`read_tail`], RF-812-F1): the last MiB from its first whole line,
//! widened (2, 4, 8 MiB) while that holds no figure or no model. It counts
//! no compaction and keeps only the newest figure and model, which are at
//! the tail, so it finds what reading the whole file would whenever both
//! are within 8 MiB of the end (a file under 1 MiB is read whole).

use crate::agent::{AgentEvent, AgentKind};
use serde::de::IgnoredAny;
use serde::Deserialize;
use serde_json::Value;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// How far into a line its top-level `type` must appear (Codex writes
/// `{"timestamp":…,"ordinal":…,"type":…` first).
const HEAD_BYTES: usize = 160;
/// Day folders searched for a thread's file, newest first.
const MAX_DAY_DIRS: usize = 120;
/// Lookups before a turn stops looking for a file that never appeared.
const MAX_LOOKUPS: u32 = 12;
/// The payload type an `event_msg` must name to be parsed at all.
const TOKEN_COUNT: &[u8] = b"token_count";

/// `$CODEX_HOME`, else `~/.codex`.
pub fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))
}

/// Thread `thread_id`'s rollout under `home`: the newest day folder holding
/// a `rollout-…-<thread_id>.jsonl`, newest name first. `None` for an id
/// that isn't a plain id (it is part of a file name).
pub fn find_rollout(home: &Path, thread_id: &str) -> Option<PathBuf> {
    let id = thread_id.trim();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    let suffix = format!("-{id}.jsonl");
    let mut searched = 0;
    for year in sorted_dirs(&home.join("sessions")) {
        for month in sorted_dirs(&year) {
            for day in sorted_dirs(&month) {
                searched += 1;
                if searched > MAX_DAY_DIRS {
                    return None;
                }
                let mut hits: Vec<PathBuf> = std::fs::read_dir(&day)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| {
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| {
                                name.starts_with("rollout-") && name.ends_with(&suffix)
                            })
                    })
                    .collect();
                hits.sort();
                if let Some(hit) = hits.pop() {
                    return Some(hit);
                }
            }
        }
    }
    None
}

/// `dir`'s subfolders, newest (highest name) first.
fn sorted_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    dirs.reverse();
    dirs
}

/// What the lines read so far said.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Seen {
    /// The last call's prompt tokens and the model's window.
    pub figure: Option<(u64, u64)>,
    /// `compacted` lines.
    pub compactions: u32,
}

/// The top-level `type` of a rollout line, from its first bytes only.
fn line_type(line: &str) -> Option<&str> {
    let head = &line.as_bytes()[..line.len().min(HEAD_BYTES)];
    // The cut can fall inside a character: keep what is whole before it.
    let head = match std::str::from_utf8(head) {
        Ok(head) => head,
        Err(e) => std::str::from_utf8(&head[..e.valid_up_to()]).ok()?,
    };
    let at = head.find("\"type\":\"")? + 8;
    let rest = &head[at..];
    Some(&rest[..rest.find('"')?])
}

/// A `compacted` line's shape, checked without keeping its (large) history.
#[derive(Deserialize)]
struct CompactedShape {
    #[serde(rename = "type")]
    kind: String,
    #[allow(dead_code)]
    payload: ShapeObject,
}

/// Any JSON object, skipped.
struct ShapeObject;

impl<'de> Deserialize<'de> for ShapeObject {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = ShapeObject;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<ShapeObject, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(ShapeObject)
            }
        }
        d.deserialize_map(V)
    }
}

/// Fold one rollout line into `seen`. Anything but a well-shaped
/// `token_count` or `compacted` line changes nothing.
pub fn fold_line(line: &str, seen: &mut Seen) {
    match line_type(line) {
        Some("event_msg") => {
            // Bytes, not a str slice: the cut may fall inside a character
            // of model text (review blocking: a str slice there panics).
            let head = &line.as_bytes()[..line.len().min(HEAD_BYTES * 2)];
            if !head.windows(TOKEN_COUNT.len()).any(|w| w == TOKEN_COUNT) {
                return;
            }
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                return;
            };
            if value.get("type").and_then(Value::as_str) != Some("event_msg") {
                return;
            }
            let payload = value.get("payload").unwrap_or(&Value::Null);
            if payload.get("type").and_then(Value::as_str) != Some("token_count") {
                return;
            }
            let info = payload.get("info").unwrap_or(&Value::Null);
            let tokens = info
                .get("last_token_usage")
                .and_then(|usage| usage.get("input_tokens"))
                .and_then(Value::as_u64);
            let window = info.get("model_context_window").and_then(Value::as_u64);
            if let (Some(tokens), Some(window)) = (tokens, window) {
                if tokens > 0 && window > 0 {
                    seen.figure = Some((tokens, window));
                }
            }
        }
        Some("compacted")
            if serde_json::from_str::<CompactedShape>(line)
                .is_ok_and(|l| l.kind == "compacted") =>
        {
            seen.compactions = seen.compactions.saturating_add(1);
        }
        _ => {}
    }
}

/// A `turn_context` line's model, and nothing else from it.
#[derive(Deserialize)]
struct TurnContextShape {
    #[serde(rename = "type")]
    kind: String,
    payload: TurnContextPayload,
}

#[derive(Deserialize)]
struct TurnContextPayload {
    model: String,
}

/// Overnight 9 A13: the model a `turn_context` line says the turn runs
/// (`"payload":{…,"model":"gpt-…"}`), so a default-model Codex run can be
/// priced. Only a model id shape is kept (lowercase letters, digits, `-`,
/// `.`, `_`, at most 80): anything else is ignored like any other line.
pub fn turn_model(line: &str) -> Option<String> {
    if line_type(line) != Some("turn_context") {
        return None;
    }
    let shape = serde_json::from_str::<TurnContextShape>(line).ok()?;
    let model = shape.payload.model;
    let id = !model.is_empty()
        && model.len() <= 80
        && model
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '.' | '_'));
    (shape.kind == "turn_context" && id).then_some(model)
}

/// One Codex turn's view of its thread's rollout. See the module doc.
#[derive(Debug, Default)]
pub struct Watch {
    home: Option<PathBuf>,
    thread: Option<String>,
    path: Option<PathBuf>,
    offset: u64,
    lookups: u32,
    seen: Seen,
    /// The model the newest `turn_context` read so far named (A13).
    model: Option<String>,
    /// Tests: bytes read from rollout files so far (RF-812-F1).
    #[cfg(test)]
    read_bytes: u64,
}

impl Watch {
    /// A watch for a Codex turn; `None` for every other agent.
    pub fn for_agent(agent: AgentKind) -> Option<Self> {
        (agent == AgentKind::Codex).then(|| Self::in_home(codex_home()))
    }

    /// A watch for a Codex turn running `spec`: under its self-hosted
    /// entry's own `CODEX_HOME` when it has one (self-hosted models Q9,
    /// where its rollouts are), else as [`Self::for_agent`].
    pub fn for_spec(agent: AgentKind, spec: &crate::agent::RunSpec) -> Option<Self> {
        let own = spec
            .extras
            .self_hosted
            .as_ref()
            .and_then(|target| target.codex_home.clone());
        match own {
            Some(home) if agent == AgentKind::Codex => Some(Self::in_home(Some(home))),
            _ => Self::for_agent(agent),
        }
    }

    /// A watch reading under `home` (tests pass a folder).
    pub fn in_home(home: Option<PathBuf>) -> Self {
        Self {
            home,
            ..Self::default()
        }
    }

    /// Feed one stream event: the thread id names the file, read then as
    /// the baseline; a completed item or turn reads what was appended.
    /// Returns what changed (new compactions and the figure), if anything.
    pub fn observe(&mut self, evt: &AgentEvent) -> Option<Seen> {
        match evt {
            AgentEvent::Session(id) => {
                if self.thread.as_deref() == Some(id.as_str()) {
                    return None;
                }
                self.thread = Some(id.clone());
                self.path = None;
                self.lookups = 0;
                self.read(true)
            }
            AgentEvent::AssistantText(_)
            | AgentEvent::ToolResult { .. }
            | AgentEvent::Result { .. } => self.poll(),
            _ => None,
        }
    }

    /// Read the lines appended since the last read. A file first found
    /// here (after the thread started) is all this turn's, so its
    /// compactions count.
    pub fn poll(&mut self) -> Option<Seen> {
        self.read(false)
    }

    /// The model the thread's newest `turn_context` named, once read.
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// Find the file if needed, then fold what was appended. A `baseline`
    /// read (the file as the thread started, or one that shrank) sets the
    /// figure but counts no compaction.
    fn read(&mut self, baseline: bool) -> Option<Seen> {
        if self.path.is_none() {
            let (home, thread) = (self.home.as_deref()?, self.thread.as_deref()?);
            if self.lookups >= MAX_LOOKUPS {
                return None;
            }
            self.lookups += 1;
            self.path = Some(find_rollout(home, thread)?);
            self.offset = 0;
        }
        let before = self.seen;
        let mut read = Seen {
            figure: self.seen.figure,
            compactions: 0,
        };
        let mut read_bytes = 0;
        let path = self.path.as_deref()?;
        // A baseline always reads a file just found (offset 0): only its
        // tail (RF-812-F1).
        let (offset, restarted) = if baseline {
            let offset = read_tail(path, &mut read, &mut self.model, &mut read_bytes);
            (offset, false)
        } else {
            read_lines(
                path,
                self.offset,
                &mut read,
                &mut self.model,
                &mut read_bytes,
            )
        };
        #[cfg(test)]
        {
            self.read_bytes += read_bytes;
        }
        self.offset = offset;
        self.seen.figure = read.figure;
        if !baseline && !restarted {
            self.seen.compactions = self.seen.compactions.saturating_add(read.compactions);
        }
        (self.seen != before).then_some(Seen {
            figure: self.seen.figure,
            compactions: self.seen.compactions - before.compactions,
        })
    }
}

/// Fold the complete lines of `path` from byte `offset` into `seen`, and
/// return the offset after the last complete line (a line still being
/// written waits for the next read), and whether the file had shrunk below
/// `offset` and was read again from its start. Read errors leave the offset
/// where it was. Every appended byte is read; only whitelisted lines are
/// parsed ([`fold_line`]), and `model` keeps the model the last
/// `turn_context` line named ([`turn_model`], A13).
fn read_lines(
    path: &Path,
    offset: u64,
    seen: &mut Seen,
    model: &mut Option<String>,
    read_bytes: &mut u64,
) -> (u64, bool) {
    let Ok(mut file) = File::open(path) else {
        return (offset, false);
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let restarted = len < offset;
    let offset = if restarted { 0 } else { offset };
    if len == offset || file.seek(SeekFrom::Start(offset)).is_err() {
        return (offset, restarted);
    }
    let mut bytes = Vec::with_capacity((len - offset) as usize);
    let read = file.take(len - offset).read_to_end(&mut bytes);
    *read_bytes += bytes.len() as u64;
    if read.is_err() {
        return (offset, restarted);
    }
    let Some(end) = fold_lines(&bytes, seen, model) else {
        return (offset, restarted);
    };
    (offset + end as u64 + 1, restarted)
}

/// Fold the complete lines of `bytes` (each ended by `\n`) into `seen` and
/// `model`; the index of the last `\n`, `None` when there is none.
fn fold_lines(bytes: &[u8], seen: &mut Seen, model: &mut Option<String>) -> Option<usize> {
    let end = bytes.iter().rposition(|b| *b == b'\n')?;
    for line in bytes[..end].split(|b| *b == b'\n') {
        if let Ok(line) = std::str::from_utf8(line) {
            let line = line.trim_end_matches('\r');
            fold_line(line, seen);
            if let Some(named) = turn_model(line) {
                *model = Some(named);
            }
        }
    }
    Some(end)
}

/// The baseline's first window, and the widest it grows to (RF-812-F1).
const TAIL_BYTES: u64 = 1 << 20;
const TAIL_MAX_BYTES: u64 = 8 << 20;

/// RF-812-F1: the baseline read of a file found as the thread started (see
/// the module doc's **Tail**): fold the complete lines of the last
/// [`TAIL_BYTES`] of `path`, from the first line that starts inside it,
/// into `seen` and `model`, doubling the window while it holds no figure or
/// no model, up to [`TAIL_MAX_BYTES`]. Each byte is read once: a wider
/// window reads only what lies before the last one. Returns the offset
/// after the last complete line; the file's end when the widest window
/// holds no line end at all (one line over 8 MiB is transcript, and the
/// fragment of it the next read sees is no JSON line, so it is ignored).
fn read_tail(
    path: &Path,
    seen: &mut Seen,
    model: &mut Option<String>,
    read_bytes: &mut u64,
) -> u64 {
    let Ok(mut file) = File::open(path) else {
        return 0;
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    // `bytes` holds the file from `from` to `len`.
    let mut bytes: Vec<u8> = Vec::new();
    let mut from = len;
    let mut window = TAIL_BYTES;
    loop {
        let start = len.saturating_sub(window);
        // One byte before the window says whether its first line is whole.
        let wanted = start.saturating_sub(1);
        if wanted < from {
            let mut earlier = Vec::with_capacity((from - wanted) as usize);
            let read = file
                .seek(SeekFrom::Start(wanted))
                .and_then(|_| (&mut file).take(from - wanted).read_to_end(&mut earlier));
            *read_bytes += earlier.len() as u64;
            if read.is_err() || earlier.len() as u64 != from - wanted {
                return 0;
            }
            earlier.extend_from_slice(&bytes);
            bytes = earlier;
            from = wanted;
        }
        // The first line that starts inside the window: at its start for
        // the whole file, else after the first line end in what was read.
        let first = if start == 0 {
            Some(0)
        } else {
            bytes.iter().position(|b| *b == b'\n').map(|at| at + 1)
        };
        let mut found = Seen::default();
        let mut named = None;
        let end = first.and_then(|first| {
            fold_lines(&bytes[first..], &mut found, &mut named).map(|end| first + end)
        });
        let done = found.figure.is_some() && named.is_some();
        if done || start == 0 || window >= TAIL_MAX_BYTES {
            if found.figure.is_some() {
                seen.figure = found.figure;
            }
            seen.compactions = seen.compactions.saturating_add(found.compactions);
            if named.is_some() {
                *model = named;
            }
            return match end {
                Some(end) => from + end as u64 + 1,
                None if start == 0 => 0,
                None => len,
            };
        }
        window *= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../fixtures/codex/rollout-compacted.jsonl");
    const THREAD: &str = "01a1527c-17b9-7c81-8388-f68ab4b72876";

    fn home_with(lines: &str) -> (tempfile::TempDir, PathBuf) {
        let home = tempfile::tempdir().unwrap();
        let day = home.path().join("sessions/2026/10/19");
        std::fs::create_dir_all(&day).unwrap();
        let file = day.join(format!("rollout-2026-10-19T00-47-07-{THREAD}.jsonl"));
        std::fs::write(&file, lines).unwrap();
        (home, file)
    }

    fn append(file: &Path, text: &str) {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(file).unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }

    #[test]
    fn codex_rollout_reads_last_call_context_and_window() {
        let mut seen = Seen::default();
        for line in FIXTURE.lines() {
            fold_line(line, &mut seen);
        }
        // The last token_count wins; the first one (`info: null`) is skipped.
        assert_eq!(seen.figure, Some((231_046, 258_400)));
        assert_eq!(seen.compactions, 1);

        // Found by thread id under CODEX_HOME's sessions folder, newest day first.
        let (home, file) = home_with(FIXTURE);
        let older = home.path().join("sessions/2026/10/18");
        std::fs::create_dir_all(&older).unwrap();
        std::fs::write(
            older.join(format!("rollout-2026-10-18T09-00-00-{THREAD}.jsonl")),
            "",
        )
        .unwrap();
        assert_eq!(
            find_rollout(home.path(), THREAD).as_deref(),
            Some(file.as_path())
        );
        assert_eq!(find_rollout(home.path(), "../etc"), None);
        assert_eq!(find_rollout(home.path(), "nope"), None);

        // A turn's watch: the thread id names the file, read at once (a
        // resumed thread's figure shows from the start).
        let mut watch = Watch::in_home(Some(home.path().to_path_buf()));
        let first = watch.observe(&AgentEvent::Session(THREAD.into())).unwrap();
        assert_eq!(first.figure, Some((231_046, 258_400)));
        // The file's earlier compaction is the baseline, not this turn's.
        assert_eq!(first.compactions, 0);
        assert_eq!(
            watch.observe(&AgentEvent::AssistantText("x".into())),
            None,
            "nothing new"
        );
        assert_eq!(
            watch.observe(&AgentEvent::Session(THREAD.into())),
            None,
            "same thread"
        );

        // Only complete lines are read; a half-written one waits.
        let next = r#"{"timestamp":"t","ordinal":99,"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":240000},"model_context_window":258400}}}"#;
        append(&file, &next[..40]);
        assert_eq!(watch.poll(), None);
        append(&file, &format!("{}\n", &next[40..]));
        assert_eq!(watch.poll().unwrap().figure, Some((240_000, 258_400)));
    }

    #[test]
    fn codex_compaction_regrounds_next_turn() {
        let (home, file) = home_with("");
        let mut watch = Watch::in_home(Some(home.path().to_path_buf()));
        watch.observe(&AgentEvent::Session(THREAD.into()));
        assert_eq!(watch.poll(), None);
        // This turn's compaction (written after the baseline read) counts.
        append(&file, FIXTURE);
        let seen = watch.poll().unwrap();
        assert_eq!(seen.compactions, 1);
        // The stream state then carries it: the receipt's count, and R1's
        // re-ground of the next turn (`stable_layers::unsettled` with
        // `compacted`, as for a Claude `compact_boundary`).
        let mut state = crate::agent::ParseState::default();
        crate::chat::apply_codex_rollout(&mut state, seen);
        assert!(state.compacted);
        assert_eq!(state.compactions, 1);
        let marked =
            crate::stable_layers::compacted_mark(None, &[Some(THREAD)], 4, state.compacted)
                .expect("a compaction marks the session");
        assert_eq!(marked.compacted_turn, 4);
        assert!(crate::stable_layers::compacted_mark(None, &[Some(THREAD)], 4, false).is_none());

        // A new thread's file that appears only after the thread started is
        // all this turn's: its compaction counts too.
        let late = tempfile::tempdir().unwrap();
        let mut watch = Watch::in_home(Some(late.path().to_path_buf()));
        assert_eq!(
            watch.observe(&AgentEvent::Session(THREAD.into())),
            None,
            "no file yet"
        );
        let day = late.path().join("sessions/2026/10/19");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(
            day.join(format!("rollout-2026-10-19T03-00-00-{THREAD}.jsonl")),
            FIXTURE,
        )
        .unwrap();
        let seen = watch
            .observe(&AgentEvent::ToolResult {
                id: "c1".into(),
                content: String::new(),
                is_error: false,
            })
            .unwrap();
        assert_eq!(
            (seen.compactions, seen.figure),
            (1, Some((231_046, 258_400)))
        );
    }

    /// Review blocking: model text with a multibyte character where the
    /// pre-filter cuts the line must not panic (release builds abort).
    #[test]
    fn a_multibyte_cut_is_skipped_safely() {
        let mut seen = Seen::default();
        for pad in 0..4 {
            let line = format!(
                r#"{{"timestamp":"t{}","type":"event_msg","payload":{{"type":"task_complete","last_agent_message":"{}"}}}}"#,
                "0".repeat(pad),
                "é—✓".repeat(120)
            );
            fold_line(&line, &mut seen);
            let typed = format!(
                r#"{{"timestamp":"{}","type":"compacted","payload":{{}}}}"#,
                "é".repeat(100 + pad)
            );
            fold_line(&typed, &mut seen);
        }
        assert_eq!(seen, Seen::default());
    }

    #[test]
    fn an_unknown_line_shape_is_ignored() {
        let mut seen = Seen::default();
        for line in [
            // A transcript line quoting a token_count line.
            r#"{"timestamp":"t","type":"response_item","payload":{"type":"message","content":"{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":9},\"model_context_window\":9}}}"}}"#,
            // Strings, not numbers; a missing window; no info.
            r#"{"timestamp":"t","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":"200000"},"model_context_window":258400}}}"#,
            r#"{"timestamp":"t","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":200000}}}}"#,
            r#"{"timestamp":"t","type":"event_msg","payload":{"type":"token_count","info":null}}"#,
            // A compacted line whose payload isn't an object; a broken line.
            r#"{"timestamp":"t","type":"compacted","payload":"x"}"#,
            r#"{"timestamp":"t","type":"compacted","payload":{"#,
            "not json",
            "",
        ] {
            fold_line(line, &mut seen);
        }
        assert_eq!(seen, Seen::default());
    }

    /// Overnight 9 A13: the watch keeps the model the newest `turn_context`
    /// named, and only a model id shape.
    #[test]
    fn a_rollout_names_the_model_its_turn_ran() {
        let (home, _file) = home_with(FIXTURE);
        let mut watch = Watch::in_home(Some(home.path().to_path_buf()));
        assert_eq!(watch.model(), None);
        watch.observe(&AgentEvent::Session(THREAD.into()));
        assert_eq!(watch.model(), Some("gpt-6-astra"));

        for line in [
            // Not a model id; not a string; no model; another line type.
            r#"{"timestamp":"t","type":"turn_context","payload":{"model":"GPT 6; rm -rf"}}"#,
            r#"{"timestamp":"t","type":"turn_context","payload":{"model":6}}"#,
            r#"{"timestamp":"t","type":"turn_context","payload":{"cwd":"/w"}}"#,
            r#"{"timestamp":"t","type":"response_item","payload":{"type":"message","content":"{\"type\":\"turn_context\",\"payload\":{\"model\":\"gpt-x\"}}"}}"#,
            "not json",
        ] {
            assert_eq!(turn_model(line), None, "{line}");
        }
        assert_eq!(
            turn_model(r#"{"timestamp":"t","type":"turn_context","payload":{"cwd":"/w","model":"gpt-5.3-codex"}}"#).as_deref(),
            Some("gpt-5.3-codex")
        );
    }

    const MIB: usize = 1 << 20;
    const COMPACTED: &str = "{\"timestamp\":\"t\",\"type\":\"compacted\",\"payload\":{}}\n";

    /// At least `bytes` of transcript lines (`response_item`, never parsed).
    fn filler(bytes: usize) -> String {
        let line = format!(
            r#"{{"timestamp":"t","type":"response_item","payload":{{"type":"message","content":"{}"}}}}"#,
            "x".repeat(1000)
        ) + "\n";
        line.repeat(bytes.div_ceil(line.len()))
    }

    fn token_count(tokens: u64) -> String {
        format!(
            r#"{{"timestamp":"t","type":"event_msg","payload":{{"type":"token_count","info":{{"last_token_usage":{{"input_tokens":{tokens}}},"model_context_window":258400}}}}}}"#
        ) + "\n"
    }

    fn turn_context(model: &str) -> String {
        format!(
            r#"{{"timestamp":"t","type":"turn_context","payload":{{"cwd":"/w","model":"{model}"}}}}"#
        ) + "\n"
    }

    /// What reading the whole file from its start finds: the figure, the
    /// model and the offset after its last complete line.
    fn whole(file: &Path) -> (Option<(u64, u64)>, Option<String>, u64) {
        let (mut seen, mut model, mut read_bytes) = (Seen::default(), None, 0);
        let (offset, _) = read_lines(file, 0, &mut seen, &mut model, &mut read_bytes);
        (seen.figure, model, offset)
    }

    /// A watch's baseline read of `text` as its thread's rollout.
    fn baseline(text: &str) -> (tempfile::TempDir, PathBuf, Watch, Option<Seen>) {
        let (home, file) = home_with(text);
        let mut watch = Watch::in_home(Some(home.path().to_path_buf()));
        let seen = watch.observe(&AgentEvent::Session(THREAD.into()));
        (home, file, watch, seen)
    }

    /// RF-812-F1: the baseline runs on the stream loop at every attempt, and
    /// rollouts reach tens of MB, so it reads only the file's tail: the last
    /// MiB (plus the byte before it, to know whether its first line is
    /// whole), widened while that holds no figure or model, at most 8 MiB.
    /// What it finds is what the whole file says.
    #[test]
    fn a_large_rollout_is_read_from_its_tail() {
        // The newest figure and model are in the last MiB, behind 6 MiB of
        // transcript, an older figure and model, and an old compaction.
        let text = format!(
            "{}{}{COMPACTED}{}{}{}",
            turn_context("gpt-old"),
            token_count(1_000),
            filler(6 * MIB),
            turn_context("gpt-6-astra"),
            token_count(200_000)
        );
        let (_home, file, mut watch, seen) = baseline(&text);
        assert!(
            watch.read_bytes <= MIB as u64 + 1,
            "read {} bytes",
            watch.read_bytes
        );
        assert_eq!(
            seen,
            Some(Seen {
                figure: Some((200_000, 258_400)),
                compactions: 0
            })
        );
        assert_eq!(watch.model(), Some("gpt-6-astra"));
        let (figure, model, offset) = whole(&file);
        assert_eq!(
            (figure, model.as_deref(), offset),
            (Some((200_000, 258_400)), watch.model(), watch.offset)
        );
        assert_eq!(watch.offset, text.len() as u64);
        // Later reads take only what was appended: this turn's compaction.
        let before = watch.read_bytes;
        append(&file, COMPACTED);
        assert_eq!(watch.poll().map(|s| s.compactions), Some(1));
        assert_eq!(watch.read_bytes - before, COMPACTED.len() as u64);

        // The newest figure and model 3 MiB from the end: the window widens
        // (1, 2, then 4 MiB), each byte read once.
        let text = format!(
            "{}{}{}",
            turn_context("gpt-6-astra"),
            token_count(150_000),
            filler(3 * MIB)
        );
        let (_home, file, watch, seen) = baseline(&text);
        assert!(
            watch.read_bytes <= 4 * MIB as u64 + 1,
            "read {} bytes",
            watch.read_bytes
        );
        assert_eq!(seen.and_then(|s| s.figure), Some((150_000, 258_400)));
        let (figure, model, offset) = whole(&file);
        assert_eq!(
            (figure, model.as_deref(), offset),
            (seen.and_then(|s| s.figure), watch.model(), watch.offset)
        );

        // Neither within 8 MiB: the baseline stops there with no figure,
        // its offset at the end, so later reads take only new lines.
        let text = format!(
            "{}{}{}",
            turn_context("gpt-6-astra"),
            token_count(150_000),
            filler(9 * MIB)
        );
        let (_home, _file, watch, seen) = baseline(&text);
        assert!(
            watch.read_bytes <= 8 * MIB as u64 + 1,
            "read {} bytes",
            watch.read_bytes
        );
        assert_eq!((seen, watch.model()), (None, None));
        assert_eq!(watch.offset, text.len() as u64);
    }

    /// RF-812-F1: a rollout that fits the first window is read whole, so
    /// the baseline finds exactly what reading the whole file does,
    /// including where a half-written last line leaves the offset.
    #[test]
    fn a_small_rollout_baseline_equals_the_whole_file_read() {
        for text in [
            FIXTURE.to_string(),
            format!("{FIXTURE}{}", &token_count(5)[..40]),
            String::new(),
            "no line end yet".to_string(),
        ] {
            let (_home, file, watch, seen) = baseline(&text);
            let (figure, model, offset) = whole(&file);
            assert_eq!(seen.and_then(|s| s.figure), figure);
            assert_eq!(seen.map_or(0, |s| s.compactions), 0);
            assert_eq!(watch.model().map(str::to_string), model);
            assert_eq!(watch.offset, offset);
            assert_eq!(watch.read_bytes, text.len() as u64);
        }
    }

    #[test]
    fn other_agents_get_no_watch() {
        assert!(Watch::for_agent(AgentKind::Claude).is_none());
        assert!(Watch::for_agent(AgentKind::Grok).is_none());
    }

    /// Self-hosted models Q9: a self-hosted Codex turn's rollout is in its
    /// entry's own `CODEX_HOME`, and the turn's watch reads it there; any
    /// other Codex turn's watch reads the usual home.
    #[test]
    fn a_self_hosted_codex_turn_reads_its_own_homes_rollout() {
        let (home, _file) = home_with(FIXTURE);
        let target = crate::self_hosted::RunTarget {
            entry: "qwen-codex".into(),
            name: "Qwen on Codex".into(),
            base_url: "http://127.0.0.1:41234".into(),
            run_key: crate::self_hosted::RunKey::from("hsr_0123456789abcdef0123456789abcdef"),
            served_model: "qwen3.8-27b".into(),
            context_window: 131_072,
            effort_levels: vec!["low".into(), "medium".into()],
            api_style: crate::self_hosted_registry::API_OPENAI.into(),
            codex_home: Some(home.path().to_path_buf()),
        };
        let spec = |self_hosted: Option<crate::self_hosted::RunTarget>| crate::agent::RunSpec {
            prompt: String::new(),
            resume: None,
            model: Some("qwen3.8-27b".into()),
            perm: "acceptEdits".into(),
            cwd: PathBuf::from("/tmp"),
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            interactive: true,
            mcp_config: None,
            append_system_prompt: None,
            process_scope: crate::agent::ProcessScope::GuiOwned,
            usage_stream: false,
            max_budget_usd: None,
            extras: crate::agent::RunExtras {
                self_hosted,
                ..crate::agent::RunExtras::default()
            },
        };
        let own = spec(Some(target.clone()));
        let mut watch = Watch::for_spec(AgentKind::Codex, &own).unwrap();
        assert_eq!(watch.home.as_deref(), Some(home.path()));
        let seen = watch.observe(&AgentEvent::Session(THREAD.into())).unwrap();
        assert_eq!(seen.figure, Some((231_046, 258_400)));
        // Another Codex turn reads the usual home; Claude gets no watch.
        let plain = Watch::for_spec(AgentKind::Codex, &spec(None)).unwrap();
        assert_eq!(plain.home, codex_home());
        assert!(Watch::for_spec(AgentKind::Claude, &own).is_none());
    }
}
