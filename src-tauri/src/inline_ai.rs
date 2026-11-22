//! Inline AI in the editor (overnight 10-11 B5): one tool-free model call
//! that rewrites, shortens, expands, fixes, translates or answers about a
//! piece of a note, returning only the replacement text. The editor shows it
//! as a suggestion the person accepts or rejects; nothing is written here.
//!
//! The same path as task Rephrase (`commands::run_assist`): the default
//! agent through `claude_runner::run_oneshot`, every tool denied, a throwaway
//! working folder, metered as `inline_ai`. An agent that can't run a
//! tool-free one-shot (no tool restrictions) is refused by the runner; that
//! error reaches the editor as is.
//!
//! Each request carries the editor's `requestId`; Dismiss, Esc or closing the
//! file calls `inline_ai_cancel` with it, which requests that run's stop
//! signal so the runner ends the model's processes (overnight S3). A cancel
//! that lands before its request registers is remembered briefly, so the
//! request then never starts; a cancel after the run finished is a no-op.

use crate::agent::Collected;
use crate::process_group::StopSignal;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

/// The longest selection one request may carry.
pub const MAX_TEXT_CHARS: usize = 20_000;
/// The longest instruction (Ask…, a Translate language).
pub const MAX_INSTRUCTION_CHARS: usize = 600;
/// The surrounding text sent for context, at most, on each side.
pub const MAX_CONTEXT_CHARS: usize = 1_500;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineAiInput {
    /// `rewrite`, `shorten`, `expand`, `fix`, `translate` or `ask`.
    pub action: String,
    /// The selected text; empty for an Ask at the cursor.
    pub text: String,
    /// Ask…'s instruction, or Translate's language.
    #[serde(default)]
    pub instruction: Option<String>,
    /// A little of the note around the selection, for tone and terms.
    #[serde(default)]
    pub context: Option<String>,
    /// The editor's id for this request, for `inline_ai_cancel`.
    #[serde(default)]
    pub request_id: Option<String>,
}

/// What a cancelled request returns. The editor has already dropped it.
pub const DISMISSED: &str = "Dismissed.";
/// How many early cancels (ids not running yet) are remembered.
const EARLY_CANCELS: usize = 32;

#[derive(Default)]
struct Stops {
    running: HashMap<String, Arc<StopSignal>>,
    early: VecDeque<String>,
}

static STOPS: Lazy<Mutex<Stops>> = Lazy::new(|| Mutex::new(Stops::default()));

/// A running request's claim on its id; dropping it unregisters. Also used by
/// the icon dialog's Generate a logo (`logo_options`, ids prefixed `logo:`),
/// so both share one stop table and one cancel path.
pub(crate) struct Registration {
    id: String,
    signal: Arc<StopSignal>,
}

impl Registration {
    pub(crate) fn claim(id: &str) -> Self {
        let mut stops = STOPS.lock().unwrap_or_else(PoisonError::into_inner);
        let signal = Arc::new(StopSignal::new());
        if let Some(i) = stops.early.iter().position(|e| e == id) {
            stops.early.remove(i);
            signal.request();
        }
        stops.running.insert(id.to_string(), Arc::clone(&signal));
        Self {
            id: id.to_string(),
            signal,
        }
    }

    /// This request's stop signal.
    pub(crate) fn signal(&self) -> Arc<StopSignal> {
        Arc::clone(&self.signal)
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut stops = STOPS.lock().unwrap_or_else(PoisonError::into_inner);
        if stops
            .running
            .get(&self.id)
            .is_some_and(|s| Arc::ptr_eq(s, &self.signal))
        {
            stops.running.remove(&self.id);
        }
    }
}

/// The editor's ids live under this prefix in the stop table, and logo ids
/// under `logo:` (`logo_options`), so a caller-supplied id in one space can
/// never name a request in the other.
const KEY_PREFIX: &str = "inline:";

/// The stop table key for the editor's request `id`.
fn key(id: &str) -> String {
    format!("{KEY_PREFIX}{id}")
}

/// Stop the request `id`: `true` when it was running. Idempotent; an id not
/// running yet is remembered (a bounded few) so it never starts.
pub fn cancel(id: &str) -> bool {
    cancel_key(&key(id))
}

/// Stop the request under the full stop-table `key` (already prefixed).
pub(crate) fn cancel_key(key: &str) -> bool {
    STOPS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .cancel(key)
}

impl Stops {
    fn cancel(&mut self, id: &str) -> bool {
        if let Some(signal) = self.running.get(id) {
            signal.request();
            return true;
        }
        if !self.early.iter().any(|e| e == id) {
            if self.early.len() >= EARLY_CANCELS {
                self.early.pop_front();
            }
            self.early.push_back(id.to_string());
        }
        false
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InlineAiResult {
    pub text: String,
}

fn cut(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// The prompt for one request, or why it can't be made.
pub fn inline_prompt(input: &InlineAiInput) -> Result<String, String> {
    let text = input.text.as_str();
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(format!("The selection is too long for inline AI (over {MAX_TEXT_CHARS} characters). Select less."));
    }
    let instruction = input
        .instruction
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(i) = instruction {
        if i.chars().count() > MAX_INSTRUCTION_CHARS {
            return Err(format!(
                "The instruction is too long (over {MAX_INSTRUCTION_CHARS} characters)."
            ));
        }
    }
    let task = match input.action.as_str() {
        "rewrite" => "Rewrite the text so it reads better: clearer and more natural, same meaning, same language, about the same length.".to_string(),
        "shorten" => "Shorten the text to about half its length. Keep every fact, name, number and decision; drop filler.".to_string(),
        "expand" => "Expand the text with useful detail and complete sentences, in the same voice. Don't invent facts, names or numbers.".to_string(),
        "fix" => "Fix spelling, grammar and punctuation only. Change nothing else: not the wording, tone or formatting.".to_string(),
        "translate" => {
            let lang = instruction.ok_or("Name the language to translate into.")?;
            format!("Translate the text into {lang}. Keep the Markdown formatting, names and numbers.")
        }
        "ask" => {
            let ask = instruction.ok_or("Say what you want written.")?;
            if text.trim().is_empty() {
                format!("Write the text this request asks for, ready to insert into the note: {ask}")
            } else {
                format!("Apply this request to the text: {ask}")
            }
        }
        other => return Err(format!("Unknown inline AI action: {other}")),
    };
    let mut prompt = String::new();
    prompt.push_str("You edit one piece of a Markdown note in place. ");
    prompt.push_str(&task);
    prompt.push_str("\n\nReply with ONLY the resulting text, exactly as it should appear in the note: no preamble, no explanation, no quotes, no code fence around it. Keep the text's Markdown (lists, links, emphasis) unless asked to change it.\n");
    if let Some(ctx) = input
        .context
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        prompt.push_str(
            "\nThe note around it, for tone and terms only (don't repeat it):\n<context>\n",
        );
        prompt.push_str(cut(ctx, MAX_CONTEXT_CHARS * 2));
        prompt.push_str("\n</context>\n");
    }
    if !text.trim().is_empty() {
        prompt.push_str("\nThe text:\n<text>\n");
        prompt.push_str(text);
        prompt.push_str("\n</text>\n");
    }
    Ok(prompt)
}

/// The model's reply as replacement text: a fence or quotes wrapped around
/// the whole reply, or a leading "Here is…:" line, come off, but only when
/// the selection itself had no such wrapper (a quoted line stays quoted, an
/// Ask for a code block keeps its fence).
pub fn clean_output(out: &str, input: &InlineAiInput) -> String {
    let original = input.text.trim_start();
    let mut s = out.trim().to_string();
    // A lead-in line ("Here is the rewrite:") the selection didn't have.
    if let Some(first) = s.lines().next() {
        let lower = first.to_lowercase();
        let lead_in = (lower.starts_with("here is")
            || lower.starts_with("here's")
            || lower.starts_with("sure"))
            && first.trim_end().ends_with(':');
        let original_first = original.lines().next().unwrap_or("");
        if lead_in && original_first.trim() != first.trim() && s.lines().count() > 1 {
            s = s[first.len()..].trim_start().to_string();
        }
    }
    // One fence around the whole reply, unless an Ask may have asked for a
    // code block or the selection was one: exactly two fence lines.
    let fences = s
        .lines()
        .filter(|l| l.trim_start().starts_with("```"))
        .count();
    if input.action != "ask"
        && !original.starts_with("```")
        && fences == 2
        && s.starts_with("```")
        && s.ends_with("```")
        && s.len() >= 6
    {
        let inner = &s[3..s.len() - 3];
        let inner = match inner.find('\n') {
            Some(i) if !inner[..i].contains(' ') => &inner[i + 1..],
            _ => inner,
        };
        s = inner.trim().to_string();
    }
    // Quotes around the whole reply, when the selection wasn't quoted.
    if !original.starts_with('"') && !original.starts_with('“') {
        for q in ['"', '“'] {
            let close = if q == '“' { '”' } else { '"' };
            if s.starts_with(q) && s.ends_with(close) && s.chars().count() >= 2 {
                let inner: String = s.chars().skip(1).take(s.chars().count() - 2).collect();
                if !inner.contains(q) {
                    s = inner.trim().to_string();
                }
            }
        }
    }
    s
}

/// One inline AI request: the prompt, one tool-free model call, the cleaned
/// reply.
pub async fn run(input: InlineAiInput) -> Result<InlineAiResult, String> {
    run_with(input, |prompt, stop| async move {
        let (kind, model) = match crate::agent_settings::resolve(None, None) {
            Ok(v) => v,
            Err(e) => return Collected::failed(e),
        };
        // A pure text transform: no tools and a throwaway cwd, so a prompt
        // injected through the note can't reach the vault.
        let cwd = std::env::temp_dir();
        crate::claude_runner::run_oneshot_supervised(
            kind,
            model,
            &prompt,
            crate::claude_runner::RunOpts {
                cwd: &cwd,
                timeout: std::time::Duration::from_secs(90),
                allowed_tools: &[],
                disallowed_tools: crate::claude_runner::NO_TOOLS,
                model: None,
                debug_log: None,
                meter: crate::usage_ledger::Meter::oneshot("inline_ai"),
                self_hosted: None,
            },
            Some(&stop),
        )
        .await
    })
    .await
}

/// [`run`] with the model call supplied (tests pass a fake). The call gets
/// the request's stop signal; the runner ends its processes when it fires.
async fn run_with<F, Fut>(input: InlineAiInput, call: F) -> Result<InlineAiResult, String>
where
    F: FnOnce(String, Arc<StopSignal>) -> Fut,
    Fut: Future<Output = Collected>,
{
    let prompt = inline_prompt(&input)?;
    let registration = input
        .request_id
        .as_deref()
        .map(|id| Registration::claim(&key(id)));
    let stop = registration
        .as_ref()
        .map(|r| Arc::clone(&r.signal))
        .unwrap_or_else(|| Arc::new(StopSignal::new()));
    if stop.requested() {
        return Err(DISMISSED.into());
    }
    let collected = call(prompt, stop).await;
    drop(registration);
    if collected.stopped {
        return Err(DISMISSED.into());
    }
    let out = collected.result?;
    let text = clean_output(&out, &input);
    if text.is_empty() {
        return Err("The model returned nothing. Try again.".into());
    }
    Ok(InlineAiResult { text })
}

/// Inline AI in the editor: a suggestion for the selection (or, for Ask at
/// the cursor, new text). Writes nothing.
#[tauri::command]
pub async fn inline_ai(input: InlineAiInput) -> Result<InlineAiResult, String> {
    run(input).await
}

/// Dismiss, Esc or closing the file on a pending inline AI request: stop its
/// model call. `true` when it was still running; a no-op once it finished.
#[tauri::command]
pub fn inline_ai_cancel(request_id: String) -> bool {
    cancel(&request_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(action: &str, text: &str, instruction: Option<&str>) -> InlineAiInput {
        InlineAiInput {
            action: action.into(),
            text: text.into(),
            instruction: instruction.map(String::from),
            context: None,
            request_id: None,
        }
    }

    fn with_id(id: &str) -> InlineAiInput {
        InlineAiInput {
            request_id: Some(id.into()),
            ..input("rewrite", "teh plan", None)
        }
    }

    fn answered(text: &str) -> Collected {
        let mut c = Collected::failed(String::new());
        c.result = Ok(text.into());
        c
    }

    #[tokio::test]
    async fn cancel_stops_the_running_call() {
        // A fake model call that runs until its stop signal fires, as the
        // runner does (it ends the processes and reports `stopped`).
        let run = tokio::spawn(run_with(
            with_id("s3-running"),
            |_prompt, stop| async move {
                stop.wait().await;
                let mut c = Collected::failed("stopped".into());
                c.stopped = true;
                c
            },
        ));
        for _ in 0..200 {
            if cancel("s3-running") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let out = tokio::time::timeout(std::time::Duration::from_secs(5), run)
            .await
            .expect("the cancel ended the call")
            .unwrap();
        assert_eq!(out.unwrap_err(), DISMISSED);
        // Finished: unregistered, so a second cancel is a no-op.
        assert!(!STOPS
            .lock()
            .unwrap()
            .running
            .contains_key(&key("s3-running")));
    }

    #[tokio::test]
    async fn an_editor_id_shaped_like_a_logo_id_is_not_a_logo_request() {
        // A logo request running under `logo:s2-shared` (the table key logo
        // ids get) is not stopped by an editor cancel of `logo:s2-shared`,
        // and an editor request with that id is not stopped by a logo cancel.
        let logo = Registration::claim("logo:s2-shared");
        assert!(!cancel("logo:s2-shared"));
        assert!(!logo.signal().requested());
        let run = run_with(with_id("logo:s2-editor"), |_prompt, stop| async move {
            stop.wait().await;
            let mut c = Collected::failed("stopped".into());
            c.stopped = true;
            c
        });
        let close = async {
            assert!(!crate::logo_options::cancel("s2-editor"));
            for _ in 0..2000 {
                if cancel("logo:s2-editor") {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            panic!("the editor's cancel never found its request");
        };
        let (out, ()) = tokio::join!(run, close);
        assert_eq!(out.unwrap_err(), DISMISSED);
        drop(logo);
    }

    #[tokio::test]
    async fn an_early_cancel_means_the_call_never_starts() {
        assert!(!cancel("s3-early"));
        assert!(!cancel("s3-early"), "idempotent");
        let out = run_with(with_id("s3-early"), |_p, _s| async {
            panic!("the model call must not start")
        })
        .await;
        assert_eq!(out.unwrap_err(), DISMISSED);
        assert!(!STOPS
            .lock()
            .unwrap()
            .early
            .iter()
            .any(|e| *e == key("s3-early")));
    }

    #[tokio::test]
    async fn a_finished_request_is_unaffected_and_early_cancels_stay_bounded() {
        let out = run_with(with_id("s3-done"), |_p, _s| async { answered("the plan") })
            .await
            .unwrap();
        assert_eq!(out.text, "the plan");
        // A late cancel (the answer already came) is a no-op, and a flood of
        // them never grows past the bound (a local table: the shared one is
        // in use by the other tests).
        let mut stops = Stops::default();
        for i in 0..(EARLY_CANCELS * 2) {
            assert!(!stops.cancel(&format!("late-{i}")));
        }
        assert_eq!(stops.early.len(), EARLY_CANCELS);
        assert_eq!(
            stops.early.front().unwrap(),
            &format!("late-{EARLY_CANCELS}")
        );
        // Without an id it still runs (nothing to cancel by).
        let out = run_with(input("fix", "teh", None), |_p, _s| async {
            answered("the")
        })
        .await;
        assert_eq!(out.unwrap().text, "the");
    }

    #[test]
    fn each_action_asks_for_only_the_text() {
        for action in ["rewrite", "shorten", "expand", "fix"] {
            let p = inline_prompt(&input(action, "teh plan", None)).unwrap();
            assert!(p.contains("<text>\nteh plan\n</text>"), "{action}");
            assert!(p.contains("Reply with ONLY the resulting text"), "{action}");
        }
        assert!(inline_prompt(&input("fix", "x", None))
            .unwrap()
            .contains("Change nothing else"));
    }

    #[test]
    fn translate_and_ask_need_their_instruction() {
        assert!(inline_prompt(&input("translate", "hola", None))
            .unwrap_err()
            .contains("language"));
        assert!(inline_prompt(&input("translate", "hola", Some("English")))
            .unwrap()
            .contains("into English"));
        assert!(inline_prompt(&input("ask", "", None)).is_err());
        let at_cursor =
            inline_prompt(&input("ask", "  ", Some("a three-row table of owners"))).unwrap();
        assert!(at_cursor.contains("ready to insert") && !at_cursor.contains("<text>"));
    }

    #[test]
    fn refuses_unknown_actions_and_oversized_input() {
        assert!(inline_prompt(&input("delete", "x", None)).is_err());
        assert!(
            inline_prompt(&input("rewrite", &"a".repeat(MAX_TEXT_CHARS + 1), None))
                .unwrap_err()
                .contains("too long")
        );
        assert!(inline_prompt(&input(
            "ask",
            "x",
            Some(&"b".repeat(MAX_INSTRUCTION_CHARS + 1))
        ))
        .unwrap_err()
        .contains("too long"));
    }

    #[test]
    fn context_is_bounded_and_labelled() {
        let mut i = input("rewrite", "x", None);
        i.context = Some("c".repeat(10_000));
        let p = inline_prompt(&i).unwrap();
        assert!(p.contains("<context>"));
        assert!(p.len() < 10_000);
    }

    #[test]
    fn cleans_wrappers_but_keeps_markdown() {
        let rw = input("rewrite", "plain text", None);
        assert_eq!(clean_output("  Better text.  ", &rw), "Better text.");
        assert_eq!(
            clean_output("Here is the rewritten text:\nBetter text.", &rw),
            "Better text."
        );
        assert_eq!(
            clean_output("```markdown\n- one\n- two\n```", &rw),
            "- one\n- two"
        );
        assert_eq!(clean_output("\"Quoted reply\"", &rw), "Quoted reply");
        assert_eq!(clean_output("“Curly”", &rw), "Curly");
        assert_eq!(
            clean_output("He said \"hi\" and \"bye\"", &rw),
            "He said \"hi\" and \"bye\""
        );
        assert_eq!(clean_output("**Bold** stays", &rw), "**Bold** stays");
    }

    #[test]
    fn keeps_what_the_selection_or_the_ask_wanted() {
        // An Ask for a snippet keeps its fence.
        let ask = input("ask", "", Some("a bash snippet"));
        assert_eq!(
            clean_output("```bash\nls -la\n```", &ask),
            "```bash\nls -la\n```"
        );
        // Two blocks: the outer markers aren't a wrapper.
        let rw = input("rewrite", "x", None);
        let two = "```js\na\n```\ntext\n```\nb\n```";
        assert_eq!(clean_output(two, &rw), two);
        // A selection that starts with the lead-in keeps it.
        let lead = input("rewrite", "Here's what we decided:\nShip Friday.", None);
        assert_eq!(
            clean_output("Here's what we decided:\nShip on Friday.", &lead),
            "Here's what we decided:\nShip on Friday."
        );
        // A quoted selection stays quoted.
        let quoted = input("fix", "\"teh quote\"", None);
        assert_eq!(clean_output("\"the quote\"", &quoted), "\"the quote\"");
    }
}
