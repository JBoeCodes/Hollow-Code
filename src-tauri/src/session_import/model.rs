//! What a reader makes of one transcript file ([`Parsed`]), and the
//! [`Builder`] both readers feed: prompts, reply text, tool calls and their
//! results, in file order. It groups them into turns (a prompt and what
//! answered it), caps every text, keeps tool excerpts at
//! [`TOOL_EXCERPT_BYTES`] labelled [`NOT_RERUN`], hashes each source turn as
//! it closes, and drops the oldest turns while the conversation is over
//! [`CONVERSATION_MAX_BYTES`] (the commit writes a "Shortened" line for
//! them).

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};

/// A tool call's input and output excerpts keep at most this many bytes.
pub(crate) const TOOL_EXCERPT_BYTES: usize = 2 * 1024;
/// One imported conversation keeps at most this much text.
pub(crate) const CONVERSATION_MAX_BYTES: usize = 8 * 1024 * 1024;
/// One prompt or reply keeps at most this many characters (the session
/// store's own cap on a message).
pub(crate) const MESSAGE_MAX_CHARS: usize = 64_000;
/// Tool calls kept per turn.
const MAX_TOOLS: usize = 200;
/// The label on every imported tool call.
pub(crate) const NOT_RERUN: &str = "From the original session; not rerun.";

/// One tool call as imported.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParsedTool {
    pub id: String,
    pub name: String,
    /// The input, or `{truncated, preview}` past [`TOOL_EXCERPT_BYTES`].
    pub input: Value,
    /// The output's excerpt, when the file had one.
    pub output: Option<String>,
    pub is_error: bool,
}

/// One turn: a prompt and what answered it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ParsedTurn {
    pub at: Option<String>,
    pub ended_at: Option<String>,
    pub asked: String,
    pub reply: String,
    pub tools: Vec<ParsedTool>,
    /// Images and files the prompt carried, by name only (N28).
    pub files: Vec<String>,
}

impl ParsedTurn {
    /// Bytes this turn holds, for the conversation cap.
    pub fn weight(&self) -> usize {
        self.asked.len()
            + self.reply.len()
            + self
                .tools
                .iter()
                .map(|t| {
                    t.name.len()
                        + t.input.to_string().len()
                        + t.output.as_ref().map_or(0, String::len)
                })
                .sum::<usize>()
            + self.files.iter().map(String::len).sum::<usize>()
    }

    /// The prompt as stored: the text, then the attachment names.
    pub fn asked_text(&self) -> String {
        if self.files.is_empty() {
            return self.asked.clone();
        }
        let line = format!(
            "[Attached in the original session, not copied: {}]",
            self.files.join(", ")
        );
        if self.asked.trim().is_empty() {
            line
        } else {
            format!("{}\n\n{line}", self.asked)
        }
    }

    /// `<prompt hash>:<turn hash>`, 16 hex characters each. The prompt half
    /// tells a turn that grew (same prompt, longer answer) from another one.
    fn hash(&self) -> String {
        let asked = Sha256::digest(self.asked_text().as_bytes());
        let mut all = Sha256::new();
        all.update(self.asked_text().as_bytes());
        all.update([0]);
        all.update(self.reply.as_bytes());
        for tool in &self.tools {
            all.update([0]);
            all.update(tool.name.as_bytes());
            all.update([0]);
            all.update(tool.input.to_string().as_bytes());
            all.update([0]);
            all.update(tool.output.as_deref().unwrap_or("").as_bytes());
            all.update([u8::from(tool.is_error)]);
        }
        format!(
            "{}:{}",
            &format!("{asked:x}")[..16],
            &format!("{:x}", all.finalize())[..16]
        )
    }
}

/// The prompt half of a turn hash.
pub(crate) fn asked_half(hash: &str) -> &str {
    hash.split(':').next().unwrap_or("")
}

/// One transcript file, read.
#[derive(Debug, Clone, Default)]
pub(crate) struct Parsed {
    /// The source's own session id (Claude `sessionId`, Codex
    /// `session_meta.id`).
    pub id: Option<String>,
    pub cwd: Option<String>,
    /// The source's title (Claude Code's `summary` line).
    pub title: Option<String>,
    pub created_at: Option<String>,
    pub last_at: Option<String>,
    /// The kept turns, oldest first.
    pub turns: Vec<ParsedTurn>,
    /// Every source turn's hash, oldest first, the dropped ones included.
    pub turn_hashes: Vec<String>,
    /// Oldest turns left out by the conversation cap, and their bytes.
    pub dropped_turns: usize,
    pub dropped_bytes: usize,
    /// Prompts plus replies, across every source turn.
    pub messages: usize,
    /// Sidechain (subagent) lines: counted, never turns.
    pub sidechain: usize,
    /// Lines that weren't JSON or were too long.
    pub malformed: usize,
    /// The last line was cut off mid-write.
    pub truncated: bool,
    /// The file looked like this source's format at all.
    pub recognized: bool,
    /// A Codex subagent's own rollout.
    pub subagent: bool,
    /// ChatGPT: branches of the conversation tree not on the imported path.
    pub other_branches: usize,
    /// Gemini: "Grouped by time" (no conversation id in the export; the
    /// commit stores it as a note turn).
    pub grouping: Option<String>,
}

impl Parsed {
    /// SHA-256 over every source turn's hash: what dedupe compares.
    pub fn content_hash(&self) -> String {
        let mut hasher = Sha256::new();
        for hash in &self.turn_hashes {
            hasher.update(hash.as_bytes());
            hasher.update([b'\n']);
        }
        format!("{:x}", hasher.finalize())
    }

    /// The source title, else the first prompt cut to 80 characters.
    pub fn display_title(&self) -> String {
        if let Some(title) = self
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            return clip(title, 80);
        }
        let first = self
            .turns
            .iter()
            .map(|t| t.asked.trim())
            .find(|t| !t.is_empty())
            .unwrap_or("");
        if first.is_empty() {
            "Imported conversation".to_string()
        } else {
            clip(first, 80)
        }
    }
}

/// `text` on one line, at most `max` characters.
pub(crate) fn clip(text: &str, max: usize) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        return one;
    }
    let mut out: String = one.chars().take(max.saturating_sub(3)).collect();
    out.push_str("...");
    out
}

/// `text` cut to [`MESSAGE_MAX_CHARS`], saying so.
fn cap_message(text: &str) -> String {
    if text.chars().count() <= MESSAGE_MAX_CHARS {
        return text.to_string();
    }
    let kept: String = text.chars().take(MESSAGE_MAX_CHARS).collect();
    format!("{kept}\n\n(Cut here: the rest is in the original file.)")
}

/// The first `max` bytes of `text`, on a character boundary, marked when cut.
pub(crate) fn excerpt(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}... (cut at 2 KB)", &text[..end])
}

/// A tool input as kept: the JSON itself when small, else a preview.
pub(crate) fn bounded_input(input: Value) -> Value {
    let text = input.to_string();
    if text.len() <= TOOL_EXCERPT_BYTES {
        return input;
    }
    json!({"truncated": true, "preview": excerpt(&text, TOOL_EXCERPT_BYTES)})
}

/// Turns as a transcript streams by (both readers feed it).
pub(crate) struct Builder {
    turns: VecDeque<ParsedTurn>,
    /// The turn still taking replies (not hashed yet).
    open: Option<ParsedTurn>,
    bytes: usize,
    max_bytes: usize,
    out: Parsed,
    /// Tool id to its index in the open turn.
    tools: HashMap<String, usize>,
    /// The open turn already has its reply counted as a message.
    replied: bool,
}

impl Builder {
    pub fn with_cap(max_bytes: usize) -> Self {
        Builder {
            turns: VecDeque::new(),
            open: None,
            bytes: 0,
            max_bytes,
            out: Parsed::default(),
            tools: HashMap::new(),
            replied: false,
        }
    }

    /// The parse so far, for a reader's own fields (id, cwd, counts).
    pub fn meta(&mut self) -> &mut Parsed {
        &mut self.out
    }

    /// A timestamp seen on any line.
    pub fn saw_time(&mut self, at: Option<&str>) {
        let Some(at) = at.filter(|a| !a.is_empty()) else {
            return;
        };
        if self.out.created_at.is_none() {
            self.out.created_at = Some(at.to_string());
        }
        self.out.last_at = Some(at.to_string());
    }

    fn close(&mut self) {
        let Some(turn) = self.open.take() else { return };
        self.tools.clear();
        self.replied = false;
        self.out.turn_hashes.push(turn.hash());
        self.bytes += turn.weight();
        self.turns.push_back(turn);
        while self.bytes > self.max_bytes && self.turns.len() > 1 {
            if let Some(dropped) = self.turns.pop_front() {
                let weight = dropped.weight();
                self.bytes = self.bytes.saturating_sub(weight);
                self.out.dropped_turns += 1;
                self.out.dropped_bytes += weight;
            }
        }
    }

    fn open_turn(&mut self, at: Option<&str>) -> &mut ParsedTurn {
        if self.open.is_none() {
            self.open = Some(ParsedTurn {
                at: at.map(str::to_string),
                ended_at: at.map(str::to_string),
                ..ParsedTurn::default()
            });
        }
        self.open.as_mut().expect("a turn was just opened")
    }

    /// A prompt from the person. Consecutive prompts with no reply between
    /// them join one turn.
    pub fn user(&mut self, text: &str, files: Vec<String>, at: Option<&str>) {
        if text.trim().is_empty() && files.is_empty() {
            return;
        }
        let answered = self
            .open
            .as_ref()
            .is_some_and(|t| !t.reply.is_empty() || !t.tools.is_empty());
        if answered {
            self.close();
        }
        if self.open.is_none() {
            self.out.messages += 1;
        }
        let turn = self.open_turn(at);
        if !text.trim().is_empty() {
            let joined = if turn.asked.is_empty() {
                text.to_string()
            } else {
                format!("{}\n\n{text}", turn.asked)
            };
            turn.asked = cap_message(&joined);
        }
        turn.files.extend(files);
    }

    fn mark_reply(&mut self) {
        if !self.replied {
            self.replied = true;
            self.out.messages += 1;
        }
    }

    /// Reply text from the agent.
    pub fn assistant(&mut self, text: &str, at: Option<&str>) {
        if text.trim().is_empty() {
            return;
        }
        self.mark_reply();
        let turn = self.open_turn(at);
        if let Some(at) = at {
            turn.ended_at = Some(at.to_string());
        }
        let joined = if turn.reply.is_empty() {
            text.to_string()
        } else {
            format!("{}\n\n{text}", turn.reply)
        };
        turn.reply = cap_message(&joined);
    }

    /// A tool call the agent made.
    pub fn tool_call(&mut self, id: &str, name: &str, input: Value, at: Option<&str>) {
        self.mark_reply();
        let turn = self.open_turn(at);
        if let Some(at) = at {
            turn.ended_at = Some(at.to_string());
        }
        if turn.tools.len() >= MAX_TOOLS {
            return;
        }
        let index = turn.tools.len();
        turn.tools.push(ParsedTool {
            id: id.to_string(),
            name: if name.trim().is_empty() {
                "tool".to_string()
            } else {
                name.to_string()
            },
            input: bounded_input(input),
            output: None,
            is_error: false,
        });
        if !id.is_empty() {
            self.tools.insert(id.to_string(), index);
        }
    }

    /// A tool call's output, matched to its call in the open turn.
    pub fn tool_result(&mut self, id: &str, output: &str, is_error: bool) {
        let Some(&index) = self.tools.get(id) else {
            return;
        };
        if let Some(tool) = self.open.as_mut().and_then(|t| t.tools.get_mut(index)) {
            tool.output = Some(excerpt(output, TOOL_EXCERPT_BYTES));
            tool.is_error = is_error;
        }
    }

    pub fn finish(mut self) -> Parsed {
        self.close();
        self.out.turns = self.turns.into_iter().collect();
        self.out
    }
}
