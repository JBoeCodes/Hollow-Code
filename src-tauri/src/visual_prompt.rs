//! "Visual answers in Hollow" (Canvas C3 wave 2 C4, plan §7): the one
//! visual-answer prompt every chat gets, with every provider (Claude, Codex,
//! Grok, Antigravity), in every chat type. It replaced the old HTML preference
//! prompt (quick and non-workspace chats only). Whiteboards were removed on
//! 2026-10-07: diagrams are canvases with the kit's `hc-flow`, and the board
//! guide (`whiteboard_prompt.md`) is no longer sent.
//!
//! In order: the decision rule (plain text, a canvas, a diagram canvas or an
//! ask-canvas) with the person's Canvases setting as one line, the "say it in
//! prose too" rule, the canvas guide (`canvas_prompt.md`: the fence, the id
//! rule, what the Canvas sandbox allows), the kit@1 reference
//! (`canvas_kit_prompt.md`, pinned to `src/canvas/kit/manifest.ts`), the
//! ask-canvas guide (Canvas C4 A3: when to ask with one, for this chat's
//! [`Asking`] channel, then `canvas_ask_prompt.md`: an example, the kit-ask
//! reference pinned to `src/canvas/kit/ask/manifest.ts`, and how the answer
//! arrives). With canvases Off the agent is asked for Markdown only and gets
//! no visual guide at all (a fence that arrives anyway still renders).
//!
//! The text is constant per [`CanvasFrequency`] and [`Asking`], and it is
//! part of the appended system prompt the resume fingerprint hashes, so
//! editing it, or a chat changing its Canvases choice, starts each affected
//! native session fresh once (with Hollow's retained conversation context).
//!
//! **Where the formats go** (agent effectiveness F6, wave 3 · G1). The
//! formats (the canvas guide, the kit reference and the ask-canvas guide:
//! [`visual_guide`], about 8 KB) are the *visual guide*. At
//! Most replies they stay in the appended prompt, as before. At When it
//! helps the appended prompt keeps only the decision rule, the "how often"
//! line, the prose rule, the ask "when" line and where the guide is
//! ([`visual_prompt`], about 2 KB; at Off only the first three), and the guide reaches the agent in a
//! turn's message instead, as a `<hollow-visual-guide>` block
//! ([`guide_block`]), when [`visual_cue`] finds the request visual (Hollow's
//! own cue words, several numbers, a named data file) or the guide file
//! couldn't be written, once per native session ([`delivery`]). Every chat
//! turn also writes it to `$HOLLOW_SCRATCH_DIR/hollow-visual-guide.md`
//! ([`write_guide_file`]), which the capability index names, so an agent
//! can read it whenever it wants to draw. The per-turn block is not in the
//! fingerprint: whether a turn looks visual never restarts a session.

/// The prompt's heading, once per chat prompt.
pub(crate) const HEADING: &str = "# Visual answers in Hollow";
/// The canvas guide: fence, id rule, saving, the Canvas sandbox.
pub(crate) const CANVAS_GUIDE: &str = include_str!("canvas_prompt.md");
/// The kit@1 reference (about 2 KB), pinned to the kit manifest.
pub(crate) const KIT_REFERENCE: &str = include_str!("canvas_kit_prompt.md");
/// How to compose a canvas, with two short exemplars (10-11 B2): the kit
/// sets the look, so the agent's job is choosing and ordering.
pub(crate) const DESIGN_GUIDE: &str = include_str!("canvas_design_prompt.md");
/// The ask-canvas guide after its "when" line (Canvas C4 A3): an example,
/// the kit-ask reference (pinned to the ask kit's manifest) and how the
/// answer arrives.
pub(crate) const ASK_GUIDE: &str = include_str!("canvas_ask_prompt.md");
/// The ask-canvas guide's heading.
const ASK_HEADING: &str = "## Ask-canvases";

/// How often the person wants canvases (Settings, Chat, Canvases, and a
/// chat's own choice in the model menu). The wire values are `off`, `helps`
/// and `most`; anything else reads as the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CanvasFrequency {
    /// Ask for no canvases.
    Off,
    /// The default: a canvas when the answer's shape calls for one.
    #[default]
    Helps,
    /// A canvas in most replies.
    Most,
}

impl CanvasFrequency {
    #[cfg(test)]
    pub const ALL: [CanvasFrequency; 3] = [
        CanvasFrequency::Off,
        CanvasFrequency::Helps,
        CanvasFrequency::Most,
    ];

    /// The wire value, or `None` for anything else.
    pub fn parse(value: &str) -> Option<CanvasFrequency> {
        match value.trim() {
            "off" => Some(CanvasFrequency::Off),
            "helps" => Some(CanvasFrequency::Helps),
            "most" => Some(CanvasFrequency::Most),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CanvasFrequency::Off => "off",
            CanvasFrequency::Helps => "helps",
            CanvasFrequency::Most => "most",
        }
    }
}

/// How a chat can ask the person a question, which decides when the
/// ask-canvas guide says to use one (Canvas C4 A3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asking {
    /// The `hollow_questions` tool is attached (Claude, Codex): question
    /// cards for simple choices, an ask-canvas when the answer is visual.
    Cards,
    /// No owned MCP, only a fenced `hollow-questions` block at the end of a
    /// reply (Grok, Antigravity; `src/hollowCards.ts`): an ask-canvas instead
    /// of a question card.
    Fenced,
    /// No question channel at all: an ask-canvas is the way to ask.
    NoCards,
}

impl Asking {
    /// From what the spawn attached (`AttachedCaps`).
    pub(crate) fn of(questions: bool, questions_fenced: bool) -> Asking {
        if questions {
            Asking::Cards
        } else if questions_fenced {
            Asking::Fenced
        } else {
            Asking::NoCards
        }
    }
}

/// A turn's frequency: the chat's own choice when it made one, else the
/// saved setting, else the default. An unknown value at either level is
/// skipped, never an error (an older Hollow reading a newer file).
pub(crate) fn resolve(chat: Option<&str>, setting: Option<&str>) -> CanvasFrequency {
    chat.and_then(CanvasFrequency::parse)
        .or_else(|| setting.and_then(CanvasFrequency::parse))
        .unwrap_or_default()
}

/// The frequency a turn runs at: below the person (`app_depth` above 0),
/// Most replies drops to When it helps unless the packet asks for a canvas
/// (overnight 9 A6). A lane's or helper's replies go to an agent, and Most
/// put the whole guide (about 12 KB) in every such turn's prompt; When it
/// helps sends it with a visual turn only. Off and When it helps stay.
pub(crate) fn for_turn(
    frequency: CanvasFrequency,
    app_depth: u32,
    wants_canvas: bool,
) -> CanvasFrequency {
    if app_depth > 0 && frequency == CanvasFrequency::Most && !wants_canvas {
        CanvasFrequency::Helps
    } else {
        frequency
    }
}

/// [`for_turn`]'s level, kept steady per native session (A6 review): below
/// the person at Most replies, a turn whose open session's stored
/// fingerprint (`stored`) matches the other of Most and When it helps runs
/// at that one, so a chat that alternates plain sends, sends asking for a
/// canvas and the person's own turns doesn't reset its session each time.
/// The person's turns (depth 0) always run at their choice.
pub(crate) fn steady(
    chosen: CanvasFrequency,
    app_depth: u32,
    wants_canvas: bool,
    stored: Option<&str>,
    fingerprint: impl Fn(CanvasFrequency) -> String,
) -> CanvasFrequency {
    let level = for_turn(chosen, app_depth, wants_canvas);
    if app_depth == 0 || chosen != CanvasFrequency::Most {
        return level;
    }
    let other = if level == CanvasFrequency::Most {
        CanvasFrequency::Helps
    } else {
        CanvasFrequency::Most
    };
    if stored.is_some_and(|stored| stored == fingerprint(other)) {
        other
    } else {
        level
    }
}

const FORMS_WITH_CANVASES: &str = "\
Hollow's chat renders more than text. Choose the form for each reply:

- Plain text (Markdown) for short, conversational or single-fact replies, \
and whenever a visual would add nothing.
- A canvas (a `hollow-canvas` fence) for status, numbers, \
comparisons, plans, timelines and choices: something the person takes in at \
a glance.
- A canvas with an `hc-flow` diagram for relationships and flows: \
architecture, processes, decisions, dependencies.
- An ask-canvas when you need the person's answer to go on and the \
answer is visual.";

const FORMS_WITHOUT_CANVASES: &str = "\
Canvases are off in this chat: answer in Markdown (lists and tables are \
fine).";

const HOW_OFTEN_HELPS: &str = "\
How often (the person's Canvases setting): when it helps. Draw a canvas only \
when the answer has one of those shapes; most short replies need none, and \
one canvas per reply is usually enough.";

const HOW_OFTEN_MOST: &str = "\
How often (the person's Canvases setting): most replies. Unless the reply is \
short or purely conversational, add one canvas that shows its substance: the \
status, the numbers, the options, the plan or the timeline.";

const HOW_OFTEN_OFF: &str = "\
How often (the person's Canvases setting): off. Do not write `hollow-canvas` \
or html fences in this chat, not even for a diagram.";

const PROSE_TOO: &str = "\
Always state the answer in prose too. A canvas illustrates the \
reply and never replaces it: give the conclusion in a sentence or two before \
the fence, so the reply reads on its own.

This is about how you present answers in chat, not how you work: keep using \
your normal tools, and never write a canvas or HTML into WORKSPACE.md, \
PLAN.md, SOURCES.md or other notes unless the person asks for HTML there. \
The Markdown files stay the record.";

/// File links (2026-11-10): a reply's file shows as its name, so a long path
/// costs nothing, and a path relative to another project's folder used to
/// fail to open.
const FILE_LINKS: &str = "\
Every file you name gets its path, in backticks or a link; it opens as a \
chip. Outside your folder, use a vault-relative (`workspaces/<id>/…`) or \
absolute path, never one relative to another project.";

const ASK_WHEN_CARDS: &str = "\
Ask only when you are blocked on a decision only the person can make. Use a \
question card (`ask_user_questions`, see \"Asking the user\") for simple \
choices: a few options, yes or no, a short answer. Use an ask-canvas when the \
answer is visual: ranking, ranges, a trade-off matrix, picking on a chart, \
reviewing a layout.";

const ASK_WHEN_FENCED: &str = "\
Ask only when you are blocked on a decision only the person can make. This \
chat has no question-card tool (Hollow can't attach its tools to this \
provider), so ask with an ask-canvas instead of a question card: for \
choosing among options and whenever the answer is visual (ranking, ranges, a \
trade-off matrix, picking on a chart, reviewing a layout). Keep a fenced \
`hollow-questions` block (see \"Asking the user\") for a single yes or no.";

const ASK_WHEN_NO_CARDS: &str = "\
Ask only when you are blocked on a decision only the person can make. This \
chat has no question cards, so ask with an ask-canvas: for choosing among \
options and whenever the answer is visual (ranking, ranges, a trade-off \
matrix, picking on a chart, reviewing a layout).";

/// When a chat that asks through `asking` asks, and with what.
fn ask_when(asking: Asking) -> &'static str {
    match asking {
        Asking::Cards => ASK_WHEN_CARDS,
        Asking::Fenced => ASK_WHEN_FENCED,
        Asking::NoCards => ASK_WHEN_NO_CARDS,
    }
}

/// The ask-canvas guide for a chat that asks through `asking`.
fn ask_guide(asking: Asking) -> String {
    format!(
        "{ASK_HEADING}\n\n{}\n\n{}",
        ask_when(asking),
        ASK_GUIDE.trim_end()
    )
}

/// The visual guide's file in a chat turn's scratch folder.
pub(crate) const GUIDE_FILE: &str = "hollow-visual-guide.md";
/// Where an agent reads the guide, as the prompts name it (only the
/// variable, never the turn's path: the appended prompt stays byte-stable).
pub(crate) const GUIDE_PATH: &str = "$HOLLOW_SCRATCH_DIR/hollow-visual-guide.md";
/// The per-turn block's tags.
pub(crate) const GUIDE_TAG: &str = "hollow-visual-guide";
const GUIDE_TITLE: &str = "# The visual guide";

/// The compact prompt's last paragraph at When it helps: where the formats
/// are.
const WHERE_THE_FORMATS_ARE: &str = "\
The formats (a `hollow-canvas` fence and the kit@1 elements, `hc-flow` \
diagrams included, and ask-canvases) are in Hollow's visual guide. Hollow adds it to a \
turn whose request looks visual, as a <hollow-visual-guide> block, once per \
conversation; you can read it at any time at \
$HOLLOW_SCRATCH_DIR/hollow-visual-guide.md. Read it before you write one of \
these fences unless it is already in this conversation: never write one from \
memory.";

/// The decision rule, the "how often" line and the prose rule: what every
/// chat's appended prompt opens with.
fn decision(frequency: CanvasFrequency) -> String {
    let canvases = frequency != CanvasFrequency::Off;
    let how_often = match frequency {
        CanvasFrequency::Off => HOW_OFTEN_OFF,
        CanvasFrequency::Helps => HOW_OFTEN_HELPS,
        CanvasFrequency::Most => HOW_OFTEN_MOST,
    };
    format!(
        "{HEADING}\n\n{}\n\n{how_often}\n\n{PROSE_TOO}\n\n{FILE_LINKS}",
        if canvases {
            FORMS_WITH_CANVASES
        } else {
            FORMS_WITHOUT_CANVASES
        }
    )
}

/// The formats for `frequency`, in a chat that asks through `asking`: the
/// canvas guide, the kit reference and the ask-canvas guide; none at Off.
/// Byte-stable for the same input.
fn formats(frequency: CanvasFrequency, asking: Asking) -> String {
    if frequency == CanvasFrequency::Off {
        return String::new();
    }
    format!(
        "{}\n\n{}\n\n{}\n\n{}",
        CANVAS_GUIDE.trim_end(),
        KIT_REFERENCE.trim_end(),
        DESIGN_GUIDE.trim_end(),
        ask_guide(asking)
    )
}

/// The appended prompt's "Visual answers in Hollow" for `frequency`, in a
/// chat that asks through `asking`. Byte-stable for the same input (the
/// resume fingerprint hashes it). At Most replies it carries the whole
/// visual guide, as before G1; at When it helps, the decision rule, the
/// ask-canvas "when" line and where the guide is; at Off, the decision rule
/// alone.
pub(crate) fn visual_prompt(frequency: CanvasFrequency, asking: Asking) -> String {
    let mut prompt = decision(frequency);
    match frequency {
        CanvasFrequency::Most => {
            prompt.push_str("\n\n");
            prompt.push_str(&formats(frequency, asking));
        }
        CanvasFrequency::Helps => {
            prompt.push_str("\n\n");
            prompt.push_str(ask_when(asking));
            prompt.push_str("\n\n");
            prompt.push_str(WHERE_THE_FORMATS_ARE);
        }
        CanvasFrequency::Off => {}
    }
    prompt
}

/// The visual guide for `frequency` and `asking`: what [`GUIDE_FILE`] and a
/// [`guide_block`] hold; empty at Off (no formats to give). Byte-stable for
/// the same input.
pub(crate) fn visual_guide(frequency: CanvasFrequency, asking: Asking) -> String {
    if frequency == CanvasFrequency::Off {
        return String::new();
    }
    format!(
        "{GUIDE_TITLE}\n\nThe formats for visual answers in Hollow (see \"Visual answers in Hollow\" for when to use each).\n\n{}\n",
        formats(frequency, asking)
    )
}

/// The guide as one turn's `<hollow-visual-guide>` block, ahead of the
/// caller's text in the message.
pub(crate) fn guide_block(guide: &str) -> String {
    format!("<{GUIDE_TAG}>\n{}\n</{GUIDE_TAG}>\n\n", guide.trim_end())
}

/// Write the guide into a chat turn's scratch folder (`dir`), where the
/// capability index says it is. False when there is no folder or the write
/// failed: the turn then carries the guide itself ([`delivery`]).
pub(crate) fn write_guide_file(dir: Option<&std::path::Path>, guide: &str) -> bool {
    let Some(dir) = dir else {
        return false;
    };
    let path = dir.join(GUIDE_FILE);
    match std::fs::write(&path, guide) {
        Ok(()) => true,
        Err(error) => {
            eprintln!("[chat] the visual guide could not be written: {error}");
            false
        }
    }
}

/// Hollow's own words that make a request visual (whole words, any case):
/// the shapes "Visual answers in Hollow" draws for. Kept to forms of a
/// word, never a prefix, so "planet", "boarding" or "graphql" don't count.
const CUE_WORDS: &[&str] = &[
    "architecture",
    "board",
    "boards",
    "breakdown",
    "canvas",
    "canvases",
    "chart",
    "charts",
    "compare",
    "compared",
    "compares",
    "comparing",
    "comparison",
    "comparisons",
    "dashboard",
    "dashboards",
    "diagram",
    "diagrams",
    "draw",
    "flow",
    "flowchart",
    "flows",
    "gantt",
    "graph",
    "graphs",
    "kanban",
    "layout",
    "matrix",
    "milestone",
    "milestones",
    "mockup",
    "options",
    "plan",
    "planning",
    "plans",
    "plot",
    "progress",
    "rank",
    "ranking",
    "roadmap",
    "sketch",
    "status",
    "timeline",
    "timelines",
    "tradeoff",
    "tradeoffs",
    "trade-off",
    "trade-offs",
    "trend",
    "trends",
    "versus",
    "visual",
    "visualise",
    "visualize",
    "visualization",
    "visualisation",
    "visually",
    "visuals",
    "vs",
    "whiteboard",
    "wireframe",
];

/// File extensions that make a named file a data file.
const DATA_EXTENSIONS: &[&str] = &[
    "csv", "tsv", "xlsx", "xls", "json", "parquet", "numbers", "ods",
];

/// At least this many numbers in a request make it data-shaped.
const CUE_NUMBERS: usize = 4;

/// The request's words: runs of letters, digits and `-._/%$,`, lowercased,
/// with trailing sentence punctuation trimmed, so `PLAN.md` stays one word.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_alphanumeric() || "-._/%$,".contains(c)))
        .map(|w| w.trim_matches(|c: char| ".,".contains(c)).to_lowercase())
        .filter(|w| !w.is_empty())
        .collect()
}

fn is_number(word: &str) -> bool {
    let core = word.trim_matches(|c: char| "%$".contains(c));
    !core.is_empty()
        && core.chars().any(|c| c.is_ascii_digit())
        && core.chars().all(|c| c.is_ascii_digit() || ",.".contains(c))
}

fn is_data_file(name: &str) -> bool {
    let name = name.to_lowercase();
    name.rsplit_once('.')
        .is_some_and(|(stem, ext)| !stem.is_empty() && DATA_EXTENSIONS.contains(&ext))
}

/// Why a request looks visual, in Hollow's own words (never the request's
/// text): the cue word it used, `numbers` (at least [`CUE_NUMBERS`]) or
/// `data file` (a named `.csv`, `.xlsx`, … in the request or its
/// entities). `None`: a plain request. Deterministic and local.
pub(crate) fn visual_cue(request: &str, entities: &[String]) -> Option<&'static str> {
    let words = words(request);
    if let Some(cue) = CUE_WORDS.iter().find(|cue| words.iter().any(|w| w == *cue)) {
        return Some(cue);
    }
    if words.iter().any(|w| is_data_file(w)) || entities.iter().any(|e| is_data_file(e)) {
        return Some("data file");
    }
    if words.iter().filter(|w| is_number(w)).count() >= CUE_NUMBERS {
        return Some("numbers");
    }
    None
}

/// How one turn gets the visual guide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Delivery {
    /// In the appended prompt (Most replies).
    Prompt,
    /// In this turn's message.
    Turn,
    /// Already in this native session, from an earlier turn.
    Given,
    /// Not sent: the request isn't visual, and the file is there to read.
    OnDemand,
    /// None: Canvases are off, so there is no guide to give.
    Off,
}

impl Delivery {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Delivery::Prompt => "prompt",
            Delivery::Turn => "turn",
            Delivery::Given => "given",
            Delivery::OnDemand => "onDemand",
            Delivery::Off => "off",
        }
    }
}

/// Whether a turn carries the guide: never at Most (the prompt has it);
/// otherwise when the request looks visual (`cue`) or the guide file
/// couldn't be written (`file`), unless this native session was already
/// given this exact guide (`given`, its SHA-256 against `sha`; a fresh
/// session never was).
pub(crate) fn delivery(
    frequency: CanvasFrequency,
    cue: Option<&str>,
    file: bool,
    fresh: bool,
    given: Option<&str>,
    sha: &str,
) -> Delivery {
    match frequency {
        CanvasFrequency::Most => return Delivery::Prompt,
        CanvasFrequency::Off => return Delivery::Off,
        CanvasFrequency::Helps => {}
    }
    if cue.is_none() && file {
        return Delivery::OnDemand;
    }
    if !fresh && given == Some(sha) {
        return Delivery::Given;
    }
    Delivery::Turn
}

/// The `derived_state` kind recording which guide a conversation's native
/// session holds (its SHA-256; empty for none).
pub(crate) const GIVEN_KIND: &str = "visualGuide";

/// The guide conversation `key`'s native session was given, if any.
pub(crate) fn given(vault: &std::path::Path, key: &str) -> Option<String> {
    crate::session_store::derived_state(vault, key, GIVEN_KIND)
        .ok()
        .flatten()
        .map(|row| row.payload)
        .filter(|sha| !sha.is_empty())
}

/// Record what the native session holds after turn `ord`: `sha` when it was
/// just sent, empty when a fresh session wasn't given it. A resumed turn
/// that sent nothing changes nothing.
pub(crate) fn record_given(vault: &std::path::Path, key: &str, ord: i64, sha: &str) {
    let revision = crate::session_store::derived_state(vault, key, GIVEN_KIND)
        .ok()
        .flatten()
        .map_or(0, |row| row.revision);
    if let Err(error) = crate::session_store::put_derived_state(
        vault,
        key,
        GIVEN_KIND,
        &crate::session_store::DerivedRow {
            revision: revision + 1,
            through_ord: ord,
            payload: sha.to_string(),
        },
    ) {
        eprintln!("[chat] the visual guide's delivery could not be stored: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifacts::{ArtifactMode, Nonce};
    use std::collections::BTreeSet;

    /// Every chat's prompt, at every canvas setting, tells the agent to give
    /// a path for any file it names, so the app can make it a chip that opens
    /// (2026-11-11: the file-link promise lives in the app, not in memory).
    #[test]
    fn every_chat_prompt_asks_for_a_path_for_each_file_named() {
        for frequency in [
            CanvasFrequency::Off,
            CanvasFrequency::Helps,
            CanvasFrequency::Most,
        ] {
            let text = decision(frequency);
            assert!(
                text.contains("Every file you name gets its path"),
                "{frequency:?}"
            );
            assert!(text.contains("it opens as a chip"), "{frequency:?}");
        }
    }

    const KIT_MANIFEST: &str = include_str!("../../src/canvas/kit/manifest.ts");
    const ASK_MANIFEST: &str = include_str!("../../src/canvas/kit/ask/manifest.ts");
    const ASK_MODEL_TS: &str = include_str!("../../src/canvas/kit/ask/model.ts");
    const ASK_FIELD_TS: &str = include_str!("../../src/canvas/kit/ask/field.ts");
    const ASK_ANSWER_TS: &str = include_str!("../../src/canvas/askAnswer.ts");
    const BRIDGE_TS: &str = include_str!("../../src/canvasBridge.ts");
    const HOLLOW_CARDS_TS: &str = include_str!("../../src/hollowCards.ts");
    /// Each ask element's source, where its attributes are read.
    const ASK_SOURCES: [(&str, &str); 9] = [
        (
            "hc-ask",
            include_str!("../../src/canvas/kit/ask/components/ask.ts"),
        ),
        (
            "hc-choice",
            include_str!("../../src/canvas/kit/ask/components/choice.ts"),
        ),
        (
            "hc-multi",
            include_str!("../../src/canvas/kit/ask/components/multi.ts"),
        ),
        (
            "hc-range",
            include_str!("../../src/canvas/kit/ask/components/range.ts"),
        ),
        (
            "hc-rank",
            include_str!("../../src/canvas/kit/ask/components/rank.ts"),
        ),
        (
            "hc-text",
            include_str!("../../src/canvas/kit/ask/components/text.ts"),
        ),
        (
            "hc-date",
            include_str!("../../src/canvas/kit/ask/components/date.ts"),
        ),
        (
            "hc-matrix",
            include_str!("../../src/canvas/kit/ask/components/matrix.ts"),
        ),
        (
            "hc-pick",
            include_str!("../../src/canvas/kit/ask/components/pick.ts"),
        ),
    ];
    const ALL_ASKING: [Asking; 3] = [Asking::Cards, Asking::Fenced, Asking::NoCards];
    const CHAT_CANVAS_TS: &str = include_str!("../../src/canvas/chatCanvas.ts");
    const FENCE_TS: &str = include_str!("../../src/canvas/fence.ts");

    /// The `'hc-…'` names in `source`'s `const <list> = [ … ]`.
    fn listed(source: &str, list: &str) -> BTreeSet<String> {
        let start = source
            .find(&format!("{list} = ["))
            .unwrap_or_else(|| panic!("{list} in its manifest"));
        let rest = &source[start..];
        let rest = &rest[..rest.find(']').expect("end of the list")];
        regex::Regex::new(r"'(hc-[a-z-]+)'")
            .unwrap()
            .captures_iter(rest)
            .map(|c| c[1].to_string())
            .collect()
    }

    /// `KIT_COMPONENTS` and `KIT_VERSION` as `manifest.ts` declares them.
    fn manifest() -> (String, BTreeSet<String>) {
        let version = regex::Regex::new(r"KIT_VERSION = '([^']+)'")
            .unwrap()
            .captures(KIT_MANIFEST)
            .expect("KIT_VERSION in manifest.ts")[1]
            .to_string();
        let start = KIT_MANIFEST
            .find("KIT_COMPONENTS = [")
            .expect("KIT_COMPONENTS in manifest.ts");
        let list = &KIT_MANIFEST[start..];
        let list = &list[..list.find(']').expect("end of KIT_COMPONENTS")];
        let names = regex::Regex::new(r"'(hc-[a-z-]+)'")
            .unwrap()
            .captures_iter(list)
            .map(|c| c[1].to_string())
            .collect::<BTreeSet<_>>();
        assert!(names.len() >= 14, "manifest parse: {names:?}");
        (version, names)
    }

    /// Every `hc-…` element the text names, the root aside.
    fn named_elements(text: &str) -> BTreeSet<String> {
        regex::Regex::new(r"\bhc-[a-z]+(?:-[a-z]+)*")
            .unwrap()
            .find_iter(text)
            .map(|m| m.as_str().to_string())
            .filter(|name| name != "hc-canvas")
            .collect()
    }

    /// What an agent reads once it has the visual guide: the appended
    /// prompt's block, then the guide (at Most the block already holds it).
    fn whole(frequency: CanvasFrequency, asking: Asking) -> String {
        match frequency {
            CanvasFrequency::Most | CanvasFrequency::Off => visual_prompt(frequency, asking),
            _ => format!(
                "{}\n\n{}",
                visual_prompt(frequency, asking),
                visual_guide(frequency, asking)
            ),
        }
    }

    /// `text` with every run of whitespace read as one space.
    fn flat(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn frequencies_parse_and_resolve_chat_first() {
        for frequency in CanvasFrequency::ALL {
            assert_eq!(CanvasFrequency::parse(frequency.as_str()), Some(frequency));
        }
        assert_eq!(
            CanvasFrequency::parse(" most "),
            Some(CanvasFrequency::Most)
        );
        assert_eq!(CanvasFrequency::parse("always"), None);
        assert_eq!(CanvasFrequency::parse(""), None);
        assert_eq!(resolve(None, None), CanvasFrequency::Helps);
        assert_eq!(resolve(None, Some("off")), CanvasFrequency::Off);
        assert_eq!(resolve(Some("most"), Some("off")), CanvasFrequency::Most);
        assert_eq!(
            resolve(Some("bogus"), Some("off")),
            CanvasFrequency::Off,
            "an unknown chat value falls back to the setting"
        );
        assert_eq!(resolve(Some(""), Some("most")), CanvasFrequency::Most);
        assert_eq!(resolve(None, Some("bogus")), CanvasFrequency::Helps);
    }

    /// Overnight 9 A6: a child turn gets at most the short visual block.
    #[test]
    fn a_child_turn_at_most_gets_the_short_visual_block() {
        use CanvasFrequency::*;
        assert_eq!(for_turn(Most, 1, false), Helps);
        assert_eq!(for_turn(Most, 2, false), Helps);
        assert_eq!(for_turn(Most, 0, false), Most, "the person's own turn");
        assert_eq!(for_turn(Helps, 1, false), Helps);
        assert_eq!(for_turn(Off, 1, true), Off, "never raised");
        let asking = Asking::Cards;
        assert!(
            visual_prompt(for_turn(Most, 1, false), asking).len() * 4
                < visual_prompt(Most, asking).len(),
            "the short block is a fraction of Most's"
        );
    }

    #[test]
    fn a_child_asked_for_a_canvas_keeps_the_full_block() {
        assert_eq!(
            for_turn(CanvasFrequency::Most, 1, true),
            CanvasFrequency::Most
        );
    }

    /// A6 review: a child chat's level sticks to its open session, so
    /// alternating turns don't reset it; the person's turn uses their own.
    #[test]
    fn a_child_turn_keeps_its_sessions_level() {
        use CanvasFrequency::*;
        let fp = |level: CanvasFrequency| format!("fp-{}", level.as_str());
        let at_most = Some("fp-most");
        let at_helps = Some("fp-helps");
        assert_eq!(steady(Most, 1, false, None, fp), Helps, "a new session");
        assert_eq!(
            steady(Most, 1, false, at_most, fp),
            Most,
            "the person typed"
        );
        assert_eq!(steady(Most, 1, true, at_helps, fp), Helps, "a canvas ask");
        assert_eq!(steady(Most, 1, true, None, fp), Most);
        assert_eq!(
            steady(Most, 0, false, at_helps, fp),
            Most,
            "the person's turn"
        );
        assert_eq!(steady(Most, 1, false, Some("fp-other"), fp), Helps);
        assert_eq!(steady(Helps, 1, true, at_most, fp), Helps, "never raised");
        assert_eq!(steady(Off, 1, false, at_helps, fp), Off);
    }

    /// Each choice changes the prompt's line. At Most the formats stay in
    /// the prompt; at When it helps and Off they are in the visual guide
    /// (G1), which the prompt points to. Off asks for Markdown only and has
    /// no guide (whiteboards were removed on 2026-10-07).
    #[test]
    fn the_prompt_line_follows_the_canvases_choice() {
        let helps = visual_prompt(CanvasFrequency::Helps, Asking::Cards);
        let most = visual_prompt(CanvasFrequency::Most, Asking::Cards);
        let off = visual_prompt(CanvasFrequency::Off, Asking::Cards);
        assert_eq!(
            helps,
            visual_prompt(CanvasFrequency::Helps, Asking::Cards),
            "byte-stable"
        );
        assert_ne!(helps, most);
        for prompt in [&helps, &most, &off] {
            assert!(prompt.starts_with(HEADING));
            assert_eq!(prompt.matches(HEADING).count(), 1);
            assert!(prompt.contains("Always state the answer in prose too."));
            assert!(prompt.contains("never write a canvas or HTML into WORKSPACE.md"));
            assert!(!prompt.contains("(below)"), "nothing points below");
        }
        assert!(most.ends_with(ASK_GUIDE.trim_end()));
        assert!(helps.contains(GUIDE_PATH), "names where the guide is");
        assert!(helps.contains(&format!("<{GUIDE_TAG}>")));
        assert!(!off.contains(GUIDE_PATH), "Off has no guide to point to");
        for prompt in [&helps, &most, &off] {
            assert!(!prompt.contains("hollow-whiteboard"), "boards are gone");
            for word in [" board", " boards", "whiteboard", "Board"] {
                assert!(!prompt.contains(word), "{word:?}");
            }
        }
        assert!(helps.contains("Canvases setting): when it helps."));
        assert!(most.contains("Canvases setting): most replies."));
        assert!(off.contains("Canvases setting): off. Do not write `hollow-canvas`"));
        assert!(most.contains(CANVAS_GUIDE.trim_end()));
        assert!(!helps.contains(CANVAS_GUIDE.trim_end()));
        for asking in ALL_ASKING {
            let guide = visual_guide(CanvasFrequency::Helps, asking);
            assert!(guide.contains(CANVAS_GUIDE.trim_end()));
            assert!(guide.contains(KIT_REFERENCE.trim_end()));
            assert!(guide.contains(ASK_GUIDE.trim_end()));
            assert!(guide.ends_with(&format!("{}\n", ASK_GUIDE.trim_end())));
            assert!(!guide.contains("hollow-whiteboard"));
            assert_eq!(
                whole(CanvasFrequency::Helps, asking)
                    .replace(&visual_prompt(CanvasFrequency::Helps, asking), ""),
                format!("\n\n{guide}")
            );
            // Most's prompt is the decision rule and the same formats.
            let most = visual_prompt(CanvasFrequency::Most, asking);
            assert!(most.ends_with(
                guide
                    .split_once("\n\n")
                    .unwrap()
                    .1
                    .split_once("\n\n")
                    .unwrap()
                    .1
                    .trim_end()
            ));
        }
        for prompt in [&helps, &most] {
            assert!(prompt.contains("- A canvas (a `hollow-canvas` fence"));
            assert!(prompt.contains("- A canvas with an `hc-flow` diagram"));
            assert!(prompt.contains("- An ask-canvas when"));
        }
        // Off asks for no canvas of any kind, whatever the chat's channel.
        for asking in ALL_ASKING {
            let off = whole(CanvasFrequency::Off, asking);
            assert_eq!(off, whole(CanvasFrequency::Off, Asking::Cards));
            assert!(visual_guide(CanvasFrequency::Off, asking).is_empty());
            assert!(off.contains("Canvases are off in this chat: answer in Markdown"));
            assert!(!off.contains(CANVAS_GUIDE.trim_end()));
            assert!(!off.contains(KIT_REFERENCE.trim_end()));
            assert!(!off.contains(ASK_GUIDE.trim_end()));
            assert!(!off.contains("- A canvas"));
            assert!(!off.contains("<hc-canvas"));
            assert!(!off.to_lowercase().contains("ask-canvas"));
        }
    }

    /// The kit reference names exactly the kit's elements: none it lacks,
    /// none left out, at the manifest's version.
    #[test]
    fn the_kit_reference_matches_the_kit_manifest() {
        let (version, components) = manifest();
        assert!(
            KIT_REFERENCE.contains(&format!("({version})")),
            "names {version}"
        );
        let ask = listed(ASK_MANIFEST, "KIT_ASK_COMPONENTS");
        let known: BTreeSet<String> = components.union(&ask).cloned().collect();
        for asking in ALL_ASKING {
            let prompt = visual_prompt(CanvasFrequency::Most, asking);
            let named = named_elements(&prompt);
            let unknown: Vec<_> = named.difference(&known).collect();
            assert!(
                unknown.is_empty(),
                "the prompt names elements neither kit has: {unknown:?}"
            );
        }
        let in_reference = named_elements(KIT_REFERENCE);
        let missing: Vec<_> = components.difference(&in_reference).collect();
        assert!(
            missing.is_empty(),
            "the kit reference leaves out: {missing:?}"
        );
        for name in &components {
            assert!(
                KIT_REFERENCE.contains(&format!("`<{name}")),
                "{name} has its own entry with its attributes"
            );
        }
    }

    /// 10-11 B2: the design guidance sits in the guide after the kit
    /// reference, and its exemplars are whole canvases in the kit's syntax.
    #[test]
    fn the_guide_teaches_composition_with_exemplars() {
        for asking in ALL_ASKING {
            let guide = visual_guide(CanvasFrequency::Helps, asking);
            let kit = guide.find(KIT_REFERENCE.trim_end()).unwrap();
            let design = guide.find(DESIGN_GUIDE.trim_end()).unwrap();
            assert!(kit < design, "design follows the kit reference");
            assert!(visual_prompt(CanvasFrequency::Most, asking).contains(DESIGN_GUIDE.trim_end()));
        }
        let flat = flat(DESIGN_GUIDE);
        for fact in [
            "The title states the conclusion",
            "Values under 10 characters; qualifiers in `delta`.",
            "Tones carry meaning; one warning stands out.",
            "Every canvas is dithered as a whole by default.",
            "Never set root `texture=\"plain\"`.",
            "Host accessibility and Texture Off always win.",
            "no colours of your own",
        ] {
            assert!(flat.contains(fact), "the design guide should say: {fact}");
        }
        let roots = regex::Regex::new(r#"<hc-canvas id="([a-z0-9-]+)" title="[^"]+">"#).unwrap();
        assert_eq!(roots.find_iter(DESIGN_GUIDE).count(), 2, "two exemplars");
        assert_eq!(DESIGN_GUIDE.matches("</hc-canvas>").count(), 2);
    }

    /// The canvas fence rules the prompt states are the parser's.
    #[test]
    fn the_fence_rules_match_the_parser() {
        assert!(crate::source_pins::contains(
            FENCE_TS,
            "export const MAX_BODY_BYTES = 128 * 1024;"
        ));
        assert_eq!(crate::canvas_fence::MAX_BODY_BYTES, 128 * 1024);
        assert!(CANVAS_GUIDE.contains("at most 128 KB"));
        assert_eq!(crate::canvas_fence::MAX_TITLE_CHARS, 180);
        assert!(CANVAS_GUIDE.contains("at most 180 characters"));
        assert_eq!(
            (
                crate::canvas_fence::MIN_HEIGHT,
                crate::canvas_fence::MAX_HEIGHT
            ),
            (80, 1600)
        );
        assert!(CANVAS_GUIDE.contains("whole pixels from 80 to 1600"));
        assert!(CANVAS_GUIDE.contains("1–64 lowercase letters, digits or hyphens"));
        assert!(crate::canvas_fence::valid_canvas_id("release-status"));
        assert!(!crate::canvas_fence::valid_canvas_id(&"a".repeat(65)));
        assert!(!crate::canvas_fence::valid_canvas_id("Release"));
        // The example in the guide is a valid, closed canvas.
        let example = CANVAS_GUIDE
            .split("```hollow-canvas\n")
            .nth(1)
            .and_then(|rest| rest.split("\n```").next())
            .expect("an example fence");
        let valid = crate::canvas_fence::validate(example).expect("the example validates");
        assert_eq!(valid.root.id, "release-status");
        let counted = crate::canvas_fence::reply_canvases(CANVAS_GUIDE);
        assert_eq!(counted.count, 1, "one canvas fence in the guide");
    }

    /// The claims match the policy `artifacts.rs` serves and the modes the
    /// chat opens canvases and html fences in (plan problem P1, again):
    /// Canvas runs inline scripts with no network, and Interactive adds
    /// HTTPS only by the person's choice.
    #[test]
    fn the_prompt_describes_the_served_canvas_sandbox() {
        let canvas = ArtifactMode::Canvas.csp(&Nonce::for_tests());
        for directive in [
            "script-src 'unsafe-inline'",
            "connect-src 'none'",
            "img-src data: blob:",
            "font-src data:",
            "worker-src 'none'",
            "frame-src 'none'",
            "object-src 'none'",
            "form-action 'none'",
        ] {
            assert!(
                canvas.contains(directive),
                "Canvas serves {directive}: {canvas}"
            );
        }
        assert!(
            !canvas.contains("https:"),
            "Canvas has no network: {canvas}"
        );
        let interactive = ArtifactMode::Interactive.csp(&Nonce::for_tests());
        assert!(interactive.contains("connect-src https:"));
        assert!(crate::source_pins::contains(
            CHAT_CANVAS_TS,
            "export const CANVAS_FENCE_MODE: ArtifactMode = 'canvas';"
        ));
        assert!(crate::source_pins::contains(
            CHAT_CANVAS_TS,
            "return inWorkspace ? 'canvas' : 'offline';"
        ));

        // Matched with line breaks read as spaces, so rewrapping the
        // Markdown never breaks a pin.
        let p = flat(&whole(CanvasFrequency::Helps, Asking::Cards));
        for fact in [
            "in Hollow's sandbox; no modes to pick",
            "Your inline `<script>` and `<style>` run",
            "There is no network: no fetch, no CDN",
            "Only data: and blob: images load.",
            "No forms, popups, workers, frames or plugins.",
            "after the person confirms the exact address",
            "Hollow asks the person whether this canvas may reach the internet. Never rely on that: inline what you need.",
            "outside workspaces and Chats its scripts stay off",
            "Always state the answer in prose too.",
        ] {
            assert!(p.contains(fact), "the prompt should say: {fact}");
        }
        for false_claim in [
            "Offline mode: HTML",
            "your <script>s do not run",
            "CDN library loads",
            "https:// and data: assets load",
            "pull a library from an https CDN",
            "Links open in a new tab",
            "network access",
            "Interactive mode",
            "Canvas mode",
        ] {
            assert!(!p.contains(false_claim), "the prompt claims: {false_claim}");
        }
    }

    /// The curated panel views (Canvas C6 · K1/K2) were removed in the COO
    /// rebuild, so the guide no longer offers a `section` attribute, and a
    /// fence that still carries one is an ordinary canvas.
    #[test]
    fn the_guide_offers_no_curated_panel_view() {
        let p = flat(&whole(CanvasFrequency::Helps, Asking::Cards));
        assert!(!p.contains("Curated panel view"));
        assert!(!p.contains("section=\""));
        let old = "<hc-canvas id=\"a\" title=\"A\" section=\"plan\"><p>x</p></hc-canvas>";
        assert!(crate::canvas_fence::validate(old).is_ok());
    }

    /// The ask-canvas guide's reference names exactly the ask kit's
    /// elements, each with attributes its component really reads, and its
    /// example is a valid ask-canvas built from both kits (Canvas C4 A3).
    #[test]
    fn the_ask_reference_matches_the_ask_kit_manifest() {
        let (version, _) = manifest();
        assert!(ASK_GUIDE.contains(&format!("({version}, only in ask-canvases)")));
        let ask = listed(ASK_MANIFEST, "KIT_ASK_COMPONENTS");
        assert_eq!(ask.len(), 9, "{ask:?}");
        let sources: BTreeSet<String> = ASK_SOURCES.iter().map(|(n, _)| n.to_string()).collect();
        assert_eq!(sources, ask, "a source per ask element");
        let in_guide = named_elements(ASK_GUIDE);
        let missing: Vec<_> = ask.difference(&in_guide).collect();
        assert!(
            missing.is_empty(),
            "the ask reference leaves out: {missing:?}"
        );
        // Each entry `<hc-x a b c>` names attributes its component reads.
        let entry = regex::Regex::new(r"`<(hc-[a-z]+)((?: [a-z]+)*)>`").unwrap();
        let mut entries = BTreeSet::new();
        for c in entry.captures_iter(ASK_GUIDE) {
            let (name, attrs) = (c[1].to_string(), c[2].to_string());
            let Some((_, source)) = ASK_SOURCES.iter().find(|(n, _)| *n == name) else {
                continue; // the example's elements are checked by the manifest above
            };
            for attr in attrs.split_whitespace() {
                assert!(
                    source.contains(&format!("'{attr}'")),
                    "{name} reads no `{attr}` attribute"
                );
            }
            entries.insert(name);
        }
        assert_eq!(entries, ask, "every ask element has its own entry");
        for attr in ["'name'", "'label'", "'required'"] {
            assert!(ASK_FIELD_TS.contains(attr), "every field reads {attr}");
        }
        // `* ` marks the recommended option.
        assert!(ASK_MODEL_TS.contains(r"const recommended = /^\*\s+/.test(t);"));
        // The example is one valid ask-canvas.
        let example = ASK_GUIDE
            .split("```hollow-canvas\n")
            .nth(1)
            .and_then(|rest| rest.split("\n```").next())
            .expect("an example fence");
        let valid = crate::canvas_fence::validate(example).expect("the example validates");
        assert!(valid.root.ask, "the example is an ask-canvas");
        assert_eq!(crate::canvas_fence::reply_canvases(ASK_GUIDE).count, 1);
        let used = named_elements(example);
        assert!(used.contains("hc-ask") && used.is_subset(&ask), "{used:?}");
    }

    /// What the guide says about an ask-canvas's answer is what the chat does
    /// (C4 A2; question cards, 2026-10-14): Hollow's own Send under the
    /// canvas sends the answers straight to the agent, the message carries `Answered canvas "<id>":` lines and the exact JSON in
    /// a `hollow-canvas-answer` block, the lines may be cut short, and the
    /// canvas locks once sent. And each chat is told the right way to ask
    /// for its channel: question cards for simple choices where the tool is
    /// attached; an ask-canvas instead of a question card for Grok and
    /// Antigravity, which can't take Hollow's tools.
    #[test]
    fn the_ask_canvas_claims_match_the_chat() {
        let guide = flat(ASK_GUIDE);
        for fact in [
            "Hollow draws the Send button under the canvas; a canvas script can't send.",
            "When the person presses it, Hollow sends the answers straight to you.",
            "`Answered canvas \"<id>\":`",
            "`- label: value` line per answer (cut short when long)",
            "`hollow-canvas-answer` block holding `{\"canvasId\", \"revision\", \"answers\"}` as JSON",
            "maps each field's `name` to its value",
            "Once sent, the canvas locks",
            "end your turn after the fence",
        ] {
            assert!(guide.contains(fact), "the ask guide should say: {fact}");
        }
        assert!(ASK_ANSWER_TS
            .contains(r#"return `Answered canvas "${answer.canvasId}":\n${lines.join('\n')}\n\n"#));
        assert!(ASK_ANSWER_TS.contains(
            "const json = JSON.stringify({ canvasId: answer.canvasId, revision: answer.revision, answers: answer.answers });"
        ));
        assert!(ASK_ANSWER_TS.contains("export const ANSWER_FENCE = 'hollow-canvas-answer';"));
        assert_eq!(crate::next_step::CANVAS_ANSWER_LANG, "hollow-canvas-answer");
        assert!(
            BRIDGE_TS.contains("draftSummaryChars: 500,"),
            "lines cut at 500"
        );
        assert!(BRIDGE_TS.contains("kind: 'answered'"), "the lock");
        // Question cards, 2026-10-14: the frame answers only Hollow's own
        // Send (`collect`), and the host sends what it gets. Overnight 10
        // A7: the kit hears `collect` first (capture phase) and stops it
        // there, so a canvas script never answers it.
        let (_, ask_ts) = ASK_SOURCES[0];
        assert!(ask_ts.contains(
            "if (d.kind === 'collect') {\n    e.stopImmediatePropagation();\n    if (typeof d.nonce === 'string') collect(d.nonce);\n  }\n}, true);"
        ));
        assert!(!ask_ts.contains("hc-ask-send"));
        assert!(BRIDGE_TS.contains("if (!g.requested) return 'unrequested';"));

        let cards = flat(&whole(CanvasFrequency::Helps, Asking::Cards));
        let fenced = flat(&whole(CanvasFrequency::Helps, Asking::Fenced));
        let none = flat(&whole(CanvasFrequency::Helps, Asking::NoCards));
        let visual = "ranking, ranges, a trade-off matrix, picking on a chart, reviewing a layout";
        for p in [&cards, &fenced, &none] {
            assert!(p.contains(visual), "the plan's visual answers");
            assert!(p.contains("## Ask-canvases"));
            assert!(
                p.contains("Ask only when you are blocked on a decision only the person can make.")
            );
        }
        assert!(cards.contains("Use a question card (`ask_user_questions`, see \"Asking the user\") for simple choices"));
        assert!(cards.contains("Use an ask-canvas when the answer is visual"));
        assert!(fenced.contains("so ask with an ask-canvas instead of a question card"));
        assert!(fenced.contains("Keep a fenced `hollow-questions` block"));
        for p in [&fenced, &none] {
            assert!(!p.contains("ask_user_questions"), "no tool it lacks");
        }
        assert!(!none.contains("hollow-questions"));
        assert_eq!(Asking::of(true, false), Asking::Cards);
        assert_eq!(Asking::of(true, true), Asking::Cards);
        assert_eq!(Asking::of(false, true), Asking::Fenced);
        assert_eq!(Asking::of(false, false), Asking::NoCards);
        // Fenced is the chat with no owned MCP: Grok and Antigravity.
        assert!(HOLLOW_CARDS_TS.contains("const NO_OWNED_MCP = new Set(['antigravity', 'grok']);"));
        assert!(crate::source_pins::contains(
            include_str!("mcp_attach.rs"),
            "Surface::Chat => caps.questions_fenced = true,"
        ));
    }

    /// Size: the kit reference is about 2–3 KB, the ask guide about 2 KB.
    /// Since G1 (F6) the appended block at When it helps and Off is about
    /// 2 KB (the decision rule and where the guide is), and the guide holds
    /// the formats; Most keeps the whole block, as before.
    #[test]
    fn the_prompt_stays_small() {
        let kit = KIT_REFERENCE.trim_end().len();
        assert!(
            (1_800..=3_072).contains(&kit),
            "kit reference is {kit} bytes"
        );
        assert!(
            CANVAS_GUIDE.len() <= 3_200,
            "canvas guide is {} bytes",
            CANVAS_GUIDE.len()
        );
        let ask = ASK_GUIDE.trim_end().len();
        assert!(ask <= 2_300, "ask guide is {ask} bytes");
        let design = DESIGN_GUIDE.trim_end().len();
        assert!(design <= 2_400, "design guide is {design} bytes");
        for asking in ALL_ASKING {
            let most = visual_prompt(CanvasFrequency::Most, asking).len();
            let helps = visual_prompt(CanvasFrequency::Helps, asking).len();
            let off = visual_prompt(CanvasFrequency::Off, asking).len();
            let guide = visual_guide(CanvasFrequency::Helps, asking).len();
            eprintln!("visual prompt {asking:?}: helps {helps}, most {most}, off {off} bytes; guide {guide}");
            assert!(
                most <= 16_000 && guide <= 16_000,
                "{asking:?}: most {most}, guide {guide}"
            );
            assert!(helps <= 2_400, "{asking:?}: helps {helps}");
            assert!(off <= 1_200, "{asking:?}: off {off}");
            assert!(helps + 7_000 <= most, "the formats left the prompt");
            assert!(visual_guide(CanvasFrequency::Off, asking).is_empty());
        }
    }

    /// G1: what makes a request visual is Hollow's own list of words (whole
    /// words, never a prefix), four or more numbers, or a named data file;
    /// the cue is Hollow's word, never the request's text.
    #[test]
    fn a_visual_request_is_told_from_a_plain_one() {
        for (request, cue) in [
            ("Compare the three pricing plans in a chart.", Some("chart")),
            (
                "Draw the architecture of the sync service",
                Some("architecture"),
            ),
            ("Can you make a TIMELINE for the launch?", Some("timeline")),
            ("Option A vs option B?", Some("options").or(Some("vs"))),
            ("Weekly sign-ups: 120, 180, 260 and 310.", Some("numbers")),
            ("Summarize sales.csv for me", Some("data file")),
            ("What's the status of the release?", Some("status")),
            ("What time zone is the Lisbon office in?", None),
            (
                "Summarize what the attached sources say about churn, with citations.",
                None,
            ),
            ("Update PLAN.md with the new owner", None),
            ("Is the planet boarding at gate 4?", None),
            ("Use the graphql endpoint", None),
            ("Fix the bug from 2024", None),
            ("", None),
        ] {
            let got = visual_cue(request, &[]);
            match cue {
                Some(_) => assert!(got.is_some(), "{request:?} looks visual"),
                None => assert_eq!(got, None, "{request:?} is plain"),
            }
            if let Some(word) = got {
                assert!(
                    CUE_WORDS.contains(&word) || ["numbers", "data file"].contains(&word),
                    "{word} is Hollow's own"
                );
            }
        }
        assert_eq!(visual_cue("Compare these", &[]), Some("compare"));
        assert_eq!(
            visual_cue("What does this say?", &["sources/q3.xlsx".to_string()]),
            Some("data file"),
            "an attached data file"
        );
        assert_eq!(
            visual_cue("What does this say?", &["notes/q3.md".to_string()]),
            None
        );
        assert_eq!(visual_cue("Steps 1, 2, 3", &[]), None, "three numbers");
        // The cue words are sorted and unique (and every one is a word).
        let mut sorted = CUE_WORDS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), CUE_WORDS.len());
    }

    /// G1: the guide goes in a turn's message when the request looks visual
    /// or the file couldn't be written, once per native session; never at
    /// Most (the prompt has it); a fresh session is never "already given".
    #[test]
    fn the_guide_goes_once_per_session_when_the_turn_needs_it() {
        let sha = "abc";
        let helps = CanvasFrequency::Helps;
        assert_eq!(
            delivery(CanvasFrequency::Most, Some("chart"), false, true, None, sha),
            Delivery::Prompt
        );
        // Canvases off: there is no guide to give, whatever the turn.
        for (cue, file, fresh) in [
            (Some("chart"), true, true),
            (None, false, true),
            (Some("chart"), false, false),
        ] {
            assert_eq!(
                delivery(CanvasFrequency::Off, cue, file, fresh, None, sha),
                Delivery::Off
            );
        }
        {
            let frequency = helps;
            assert_eq!(
                delivery(frequency, None, true, true, None, sha),
                Delivery::OnDemand
            );
            assert_eq!(
                delivery(frequency, None, true, false, None, sha),
                Delivery::OnDemand
            );
            assert_eq!(
                delivery(frequency, Some("chart"), true, true, None, sha),
                Delivery::Turn
            );
            assert_eq!(
                delivery(frequency, Some("chart"), true, false, None, sha),
                Delivery::Turn
            );
            assert_eq!(
                delivery(frequency, Some("chart"), true, false, Some(sha), sha),
                Delivery::Given
            );
            assert_eq!(
                delivery(frequency, Some("chart"), true, true, Some(sha), sha),
                Delivery::Turn,
                "a fresh session holds nothing"
            );
            assert_eq!(
                delivery(frequency, Some("chart"), true, false, Some("old"), sha),
                Delivery::Turn,
                "a newer guide goes again"
            );
            // No file to read: the turn carries it, whatever the request.
            assert_eq!(
                delivery(frequency, None, false, true, None, sha),
                Delivery::Turn
            );
            assert_eq!(
                delivery(frequency, None, false, false, Some(sha), sha),
                Delivery::Given
            );
        }
    }

    /// G1: the agent can always fetch the full guide. It is written to the
    /// turn's scratch folder under the name the prompts give; without a
    /// folder nothing is written and the turn carries the guide.
    #[test]
    fn the_guide_file_is_where_the_prompt_says() {
        let dir = tempfile::tempdir().unwrap();
        let guide = visual_guide(CanvasFrequency::Helps, Asking::Cards);
        assert!(write_guide_file(Some(dir.path()), &guide));
        assert_eq!(
            std::fs::read_to_string(dir.path().join(GUIDE_FILE)).unwrap(),
            guide
        );
        assert_eq!(
            GUIDE_PATH,
            format!("${}/{GUIDE_FILE}", crate::scratch::SCRATCH_ENV)
        );
        assert!(!write_guide_file(None, &guide));
        assert!(!write_guide_file(Some(&dir.path().join("gone")), &guide));
        // The block closes once, at its end.
        let block = guide_block(&guide);
        assert!(block.starts_with(&format!("<{GUIDE_TAG}>\n{GUIDE_TITLE}")));
        assert_eq!(block.matches(&format!("</{GUIDE_TAG}>")).count(), 1);
        assert!(block.ends_with(&format!("</{GUIDE_TAG}>\n\n")));
        for frequency in CanvasFrequency::ALL {
            for asking in ALL_ASKING {
                assert!(!visual_guide(frequency, asking).contains(GUIDE_TAG));
            }
        }
    }
}
