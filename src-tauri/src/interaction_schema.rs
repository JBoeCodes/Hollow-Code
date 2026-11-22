//! Structured question cards (Paperclip PR 13): the pure half, with no I/O.
//!
//! An agent blocked on a decision only the user can make calls
//! `ask_user_questions` on the stateless `hollow_questions` MCP server
//! (`question_mcp.rs`, which validates with [`validate_request`] and answers
//! "Posted … End your turn now"). The GUI never trusts that subprocess: the
//! chat turn feeds every parsed stream event through an [`AskRecorder`], which
//! returns an [`AskCall`] only once the COMPLETED structured tool event says
//! the call was posted. The interactions store then re-validates and records
//! the card. Antigravity has no owned MCP, so it is told to end its reply with
//! one fenced `hollow-questions` block instead ([`extract_fenced`]).
//!
//! Since Paperclip PR 13 `agent::parse_codex` normalizes a Codex
//! `mcp_tool_call` to Claude's naming (`mcp__<server>__<tool>`, input = the
//! call's arguments, then a `ToolResult` whose `is_error` comes from the item's
//! status and error), so one rule detects the call for both agents:
//! [`QUALIFIED_TOOL`] plus a non-error result that starts with
//! [`POSTED_PREFIX`]. The prefix check matters for Codex, whose item has no
//! `isError` field: the MCP's own rejection text never starts with it.
//!
//! The request is a Hollow dialect of Paperclip's issue interaction schema
//! (`packages/shared/src/types/issue.ts`): questions with single, multi or text
//! answers, options with a `recommended` flag, custom answers, text
//! validation and `supersedeOnUserMessage`, plus confirmation (bound to a
//! target file hash), checkbox and per-item verdict modes. Unknown fields are
//! ignored; snake_case aliases are accepted for multi-word fields. The
//! submitted answers become the next chat turn, formatted by
//! [`format_answers`] as `Answered questions`, then each question on its own
//! line and the answer on the next. A custom answer is the person's words,
//! with no `Other:` prefix.

use crate::agent::{AgentEvent, AgentKind};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

/// The MCP server name (`hollow_` so `agent::codex_owned_servers` accepts it).
pub const SERVER: &str = "hollow_questions";
/// The one tool it lists.
pub const TOOL: &str = "ask_user_questions";
/// The tool's qualified name as both agents report it (Claude natively,
/// Codex after `parse_codex` normalizes its `mcp_tool_call` item).
pub const QUALIFIED_TOOL: &str = "mcp__hollow_questions__ask_user_questions";
/// Antigravity's fence language.
pub const FENCE_LANG: &str = "hollow-questions";
/// The first line of every submitted answer turn.
pub const ANSWER_HEADER: &str = "Answered questions";
/// How the MCP's success text starts. The recorder requires it, so a result
/// the transport reports as a success but that is really the MCP's rejection
/// never becomes a card.
pub const POSTED_PREFIX: &str = "Posted ";
/// The tool-use id of an Antigravity fenced request (one per run).
pub const FENCED_TOOL_USE_ID: &str = "fenced";

pub const MAX_QUESTIONS: usize = 8;
pub const MAX_OPTIONS: usize = 12;
/// Checkbox options and verdict items.
pub const MAX_CHOICES: usize = 20;
pub const LABEL_MAX: usize = 200;
pub const PROMPT_MAX: usize = 1000;
pub const MARKDOWN_MAX: usize = 4000;
/// A decision question's shared scenario (decision cards, 2026-11-17).
pub const SCENARIO_MAX: usize = 600;
/// What one option does in that scenario.
pub const OUTCOME_MAX: usize = 240;
pub const REQUEST_MAX_BYTES: usize = 32 * 1024;
const ID_MAX: usize = 40;
const PATH_MAX: usize = 1024;
const PATTERN_MAX: usize = 500;
const PATTERN_SIZE_LIMIT: usize = 64 * 1024;
const TEXT_ANSWER_MAX: usize = 10_000;
/// Cap on the formatted answer turn.
pub const ANSWER_PROMPT_MAX: usize = 16_000;

/// Appended (byte-stable, path-free) to a chat turn whose agent has the
/// `hollow_questions` server attached (Claude, Codex).
pub const GUIDANCE_MCP: &str = "\
# Asking the user (Hollow question cards)

Ask only when you are blocked on a decision only the user can make and cannot \
reasonably proceed on your own. Then call the hollow_questions tool \
ask_user_questions instead of asking in prose: ask at most 5 questions, offer \
concrete options, and mark the one you recommend. Only the COO may ask; \
never call it from a subagent or a delegated task. When it returns \"Posted\", \
end your turn immediately: call no more tools and do not continue the work. \
The user's answers arrive as their next message, starting \"Answered questions\".";

/// Appended to an Antigravity chat turn, which has no owned MCP.
pub const GUIDANCE_FENCED: &str = "\
# Asking the user (Hollow question cards)

Ask only when you are blocked on a decision only the user can make and cannot \
reasonably proceed on your own. Then end your reply with exactly one fenced code \
block whose language is hollow-questions, holding the request as JSON, for \
example {\"questions\":[{\"id\":\"layout\",\"prompt\":\"Which layout?\",\
\"answerMode\":\"single\",\"options\":[{\"id\":\"grid\",\"label\":\"Grid\",\
\"recommended\":true},{\"id\":\"list\",\"label\":\"List\"}]}]}. Ask at most 5 \
questions and mark the option you recommend. Only the COO may ask. Stop \
after the block; the user's answers arrive as their next message, starting \
\"Answered questions\".";

/// The guidance for a chat turn: the tool when the server is attached, the
/// fence for an agent without owned MCP, nothing otherwise.
pub fn guidance(mcp_attached: bool, fenced: bool) -> Option<&'static str> {
    if mcp_attached {
        Some(GUIDANCE_MCP)
    } else if fenced {
        Some(GUIDANCE_FENCED)
    } else {
        None
    }
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Questions,
    #[serde(alias = "request_confirmation", alias = "requestConfirmation")]
    Confirmation,
    Checkbox,
    #[serde(alias = "verdict")]
    Verdicts,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Questions => "questions",
            Mode::Confirmation => "confirmation",
            Mode::Checkbox => "checkbox",
            Mode::Verdicts => "verdicts",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnswerMode {
    #[default]
    Single,
    #[serde(alias = "multiple")]
    Multi,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputType {
    #[default]
    Text,
    Number,
    Integer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Approve,
    Reject,
    Defer,
}

impl Verdict {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Verdict::Approve => "Approve",
            Verdict::Reject => "Reject",
            Verdict::Defer => "Defer",
        }
    }
}

fn all_verdicts() -> Vec<Verdict> {
    vec![Verdict::Approve, Verdict::Reject, Verdict::Defer]
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionOption {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// What choosing this option does in the question's `scenario`: one
    /// plain line the card shows under the label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(default)]
    pub recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextValidation {
    #[serde(default, alias = "min_length", skip_serializing_if = "Option::is_none")]
    pub min_length: Option<u32>,
    #[serde(default, alias = "max_length", skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default, alias = "input_type")]
    pub input_type: InputType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub id: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    #[serde(default, alias = "help_text", skip_serializing_if = "Option::is_none")]
    pub help_text: Option<String>,
    /// One line under the question: why it is being asked. The card shows
    /// this, or the help text collapsed to one line when this is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// One concrete scenario, with numbers, that every option's `outcome`
    /// is about. The card shows it as one short paragraph under the prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenario: Option<String>,
    #[serde(default = "yes")]
    pub required: bool,
    #[serde(default, alias = "answer_mode")]
    pub answer_mode: AnswerMode,
    #[serde(default)]
    pub options: Vec<QuestionOption>,
    /// An "Other…" free-text row. On unless the agent turns it off
    /// (question cards, 2026-10-14: every choice question can be answered
    /// in the person's own words, as in Claude Code).
    #[serde(default = "yes", alias = "allow_custom")]
    pub allow_custom: bool,
    #[serde(
        default,
        alias = "custom_label",
        skip_serializing_if = "Option::is_none"
    )]
    pub custom_label: Option<String>,
    #[serde(
        default,
        alias = "text_validation",
        skip_serializing_if = "Option::is_none"
    )]
    pub text_validation: Option<TextValidation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetRef {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Confirmation {
    pub prompt: String,
    #[serde(
        default,
        alias = "details_markdown",
        skip_serializing_if = "Option::is_none"
    )]
    pub details_markdown: Option<String>,
    #[serde(
        default,
        alias = "accept_label",
        skip_serializing_if = "Option::is_none"
    )]
    pub accept_label: Option<String>,
    #[serde(
        default,
        alias = "reject_label",
        skip_serializing_if = "Option::is_none"
    )]
    pub reject_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<TargetRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Checkbox {
    pub prompt: String,
    #[serde(
        default,
        alias = "details_markdown",
        skip_serializing_if = "Option::is_none"
    )]
    pub details_markdown: Option<String>,
    #[serde(default)]
    pub options: Vec<QuestionOption>,
    #[serde(default, alias = "default_selected")]
    pub default_selected: Vec<String>,
    #[serde(
        default,
        alias = "min_selected",
        skip_serializing_if = "Option::is_none"
    )]
    pub min_selected: Option<u32>,
    #[serde(
        default,
        alias = "max_selected",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_selected: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<TargetRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerdictItem {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(
        default,
        alias = "preview_markdown",
        skip_serializing_if = "Option::is_none"
    )]
    pub preview_markdown: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Verdicts {
    pub prompt: String,
    #[serde(
        default,
        alias = "details_markdown",
        skip_serializing_if = "Option::is_none"
    )]
    pub details_markdown: Option<String>,
    #[serde(default)]
    pub items: Vec<VerdictItem>,
    #[serde(default = "all_verdicts")]
    pub verdicts: Vec<Verdict>,
    #[serde(default, alias = "require_reason_on")]
    pub require_reason_on: Vec<Verdict>,
    #[serde(default = "yes", alias = "allow_bulk_approve")]
    pub allow_bulk_approve: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<TargetRef>,
}

/// A validated, normalized request (defaults filled, other modes' sections
/// cleared). Stored as the card's `request` and read back by the frontend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionRequest {
    #[serde(default)]
    pub mode: Mode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(
        default,
        alias = "submit_label",
        skip_serializing_if = "Option::is_none"
    )]
    pub submit_label: Option<String>,
    #[serde(default = "yes", alias = "supersede_on_user_message")]
    pub supersede_on_user_message: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<Question>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<Confirmation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkbox: Option<Checkbox>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdicts: Option<Verdicts>,
}

impl QuestionRequest {
    /// The target file a confirmation, checkbox or verdicts card is bound to.
    pub fn target_path(&self) -> Option<&str> {
        let target = match self.mode {
            Mode::Questions => None,
            Mode::Confirmation => self.confirmation.as_ref().and_then(|c| c.target.as_ref()),
            Mode::Checkbox => self.checkbox.as_ref().and_then(|c| c.target.as_ref()),
            Mode::Verdicts => self.verdicts.as_ref().and_then(|v| v.target.as_ref()),
        };
        target.map(|t| t.path.as_str())
    }

    /// How many questions the card asks (1 for the single-prompt modes).
    pub fn question_count(&self) -> usize {
        match self.mode {
            Mode::Questions => self.questions.len(),
            _ => 1,
        }
    }
}

fn chars(s: &str) -> usize {
    s.chars().count()
}

fn required_text(value: &str, what: &str, max: usize) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{what} is required"));
    }
    if chars(value) > max {
        return Err(format!("{what} is longer than {max} characters"));
    }
    Ok(())
}

fn optional_text(value: &Option<String>, what: &str, max: usize) -> Result<(), String> {
    match value {
        Some(value) if chars(value) > max => Err(format!("{what} is longer than {max} characters")),
        _ => Ok(()),
    }
}

fn valid_id(id: &str, what: &str) -> Result<(), String> {
    let ok = !id.is_empty()
        && id.len() <= ID_MAX
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "{what} id `{id}` must be 1-{ID_MAX} letters, digits, `_` or `-`"
        ))
    }
}

fn unique_ids<'a>(ids: impl Iterator<Item = &'a str>, what: &str) -> Result<(), String> {
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(format!("duplicate {what} id `{id}`"));
        }
    }
    Ok(())
}

/// Collapse runs of whitespace to single spaces; blank becomes none.
fn collapse_ws(value: &mut Option<String>) {
    *value = value
        .as_ref()
        .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|text| !text.is_empty());
}

fn validate_options(options: &[QuestionOption], max: usize, what: &str) -> Result<(), String> {
    if options.is_empty() {
        return Err(format!("{what} needs at least one option"));
    }
    if options.len() > max {
        return Err(format!("{what} has more than {max} options"));
    }
    unique_ids(options.iter().map(|o| o.id.as_str()), "option")?;
    for option in options {
        valid_id(&option.id, "option")?;
        required_text(&option.label, "an option label", LABEL_MAX)?;
        optional_text(&option.description, "an option description", PROMPT_MAX)?;
        optional_text(&option.outcome, "an option outcome", OUTCOME_MAX)?;
    }
    Ok(())
}

fn validate_target(target: &Option<TargetRef>) -> Result<(), String> {
    if let Some(target) = target {
        if target.path.trim().is_empty() {
            return Err("target.path is empty".into());
        }
        if chars(&target.path) > PATH_MAX || target.path.contains('\0') {
            return Err("target.path is not a usable path".into());
        }
    }
    Ok(())
}

/// Compile a text-validation pattern that must match the WHOLE value. It is
/// compiled on its own first, so its parentheses are known to balance and the
/// `^(?:…)$` wrapper cannot be escaped by a crafted pattern.
fn compile_pattern(pattern: &str) -> Result<regex::Regex, String> {
    regex::RegexBuilder::new(pattern)
        .size_limit(PATTERN_SIZE_LIMIT)
        .build()
        .map_err(|e| format!("invalid pattern: {e}"))?;
    regex::RegexBuilder::new(&format!("^(?:{pattern})$"))
        .size_limit(PATTERN_SIZE_LIMIT)
        .build()
        .map_err(|e| format!("invalid pattern: {e}"))
}

fn validate_text_validation(tv: &TextValidation) -> Result<(), String> {
    if let Some(max) = tv.max_length {
        if max as usize > TEXT_ANSWER_MAX {
            return Err(format!("maxLength may be at most {TEXT_ANSWER_MAX}"));
        }
    }
    if let (Some(min), Some(max)) = (tv.min_length, tv.max_length) {
        if min > max {
            return Err("minLength is greater than maxLength".into());
        }
    }
    if let Some(pattern) = &tv.pattern {
        if chars(pattern) > PATTERN_MAX {
            return Err(format!("pattern is longer than {PATTERN_MAX} characters"));
        }
        compile_pattern(pattern)?;
    }
    let bounds = [tv.minimum, tv.maximum];
    if tv.input_type == InputType::Text && bounds.iter().any(Option::is_some) {
        return Err("minimum and maximum need inputType number or integer".into());
    }
    for bound in bounds.into_iter().flatten() {
        if !bound.is_finite() {
            return Err("minimum and maximum must be finite numbers".into());
        }
        if tv.input_type == InputType::Integer && bound.fract() != 0.0 {
            return Err("integer bounds must be whole numbers".into());
        }
    }
    if let (Some(min), Some(max)) = (tv.minimum, tv.maximum) {
        if min > max {
            return Err("minimum is greater than maximum".into());
        }
    }
    Ok(())
}

fn validate_questions(questions: &mut [Question]) -> Result<(), String> {
    if questions.is_empty() {
        return Err("questions mode needs at least one question".into());
    }
    if questions.len() > MAX_QUESTIONS {
        return Err(format!("at most {MAX_QUESTIONS} questions per card"));
    }
    unique_ids(questions.iter().map(|q| q.id.as_str()), "question")?;
    for q in questions.iter_mut() {
        valid_id(&q.id, "question")?;
        required_text(&q.prompt, "a question prompt", PROMPT_MAX)?;
        optional_text(&q.header, "a question header", LABEL_MAX)?;
        optional_text(&q.help_text, "question help text", PROMPT_MAX)?;
        collapse_ws(&mut q.reason);
        optional_text(&q.reason, "a question's reason", LABEL_MAX)?;
        collapse_ws(&mut q.scenario);
        optional_text(&q.scenario, "a question's scenario", SCENARIO_MAX)?;
        optional_text(&q.custom_label, "customLabel", LABEL_MAX)?;
        match q.answer_mode {
            AnswerMode::Single | AnswerMode::Multi => {
                for option in q.options.iter_mut() {
                    collapse_ws(&mut option.outcome);
                }
                validate_options(&q.options, MAX_OPTIONS, &format!("question `{}`", q.id))?;
                if q.text_validation.is_some() {
                    return Err(format!(
                        "question `{}`: textValidation applies only to text questions",
                        q.id
                    ));
                }
            }
            AnswerMode::Text => {
                if !q.options.is_empty() {
                    return Err(format!(
                        "question `{}`: a text question takes no options",
                        q.id
                    ));
                }
                q.allow_custom = false;
                if let Some(tv) = &q.text_validation {
                    validate_text_validation(tv)
                        .map_err(|e| format!("question `{}`: {e}", q.id))?;
                }
            }
        }
    }
    Ok(())
}

fn validate_confirmation(c: &Confirmation) -> Result<(), String> {
    required_text(&c.prompt, "confirmation.prompt", PROMPT_MAX)?;
    optional_text(
        &c.details_markdown,
        "confirmation.detailsMarkdown",
        MARKDOWN_MAX,
    )?;
    optional_text(&c.accept_label, "confirmation.acceptLabel", LABEL_MAX)?;
    optional_text(&c.reject_label, "confirmation.rejectLabel", LABEL_MAX)?;
    validate_target(&c.target)
}

fn validate_checkbox(c: &Checkbox) -> Result<(), String> {
    required_text(&c.prompt, "checkbox.prompt", PROMPT_MAX)?;
    optional_text(
        &c.details_markdown,
        "checkbox.detailsMarkdown",
        MARKDOWN_MAX,
    )?;
    validate_options(&c.options, MAX_CHOICES, "checkbox")?;
    unique_ids(
        c.default_selected.iter().map(String::as_str),
        "defaultSelected",
    )?;
    for id in &c.default_selected {
        if !c.options.iter().any(|o| &o.id == id) {
            return Err(format!(
                "checkbox.defaultSelected names unknown option `{id}`"
            ));
        }
    }
    let count = c.options.len() as u32;
    if let Some(max) = c.max_selected {
        if max == 0 || max > count {
            return Err(format!(
                "checkbox.maxSelected must be between 1 and {count}"
            ));
        }
    }
    if let Some(min) = c.min_selected {
        if min > count || c.max_selected.is_some_and(|max| min > max) {
            return Err("checkbox.minSelected is out of range".into());
        }
    }
    validate_target(&c.target)
}

fn dedupe_verdicts(list: &mut Vec<Verdict>) {
    let mut seen = HashSet::new();
    list.retain(|v| seen.insert(*v));
}

fn validate_verdicts(v: &mut Verdicts) -> Result<(), String> {
    required_text(&v.prompt, "verdicts.prompt", PROMPT_MAX)?;
    optional_text(
        &v.details_markdown,
        "verdicts.detailsMarkdown",
        MARKDOWN_MAX,
    )?;
    if v.items.is_empty() {
        return Err("verdicts mode needs at least one item".into());
    }
    if v.items.len() > MAX_CHOICES {
        return Err(format!("at most {MAX_CHOICES} verdict items"));
    }
    unique_ids(v.items.iter().map(|i| i.id.as_str()), "item")?;
    for item in &v.items {
        valid_id(&item.id, "item")?;
        required_text(&item.label, "an item label", LABEL_MAX)?;
        optional_text(&item.description, "an item description", PROMPT_MAX)?;
        optional_text(
            &item.preview_markdown,
            "an item previewMarkdown",
            MARKDOWN_MAX,
        )?;
    }
    dedupe_verdicts(&mut v.verdicts);
    dedupe_verdicts(&mut v.require_reason_on);
    if v.verdicts.is_empty() {
        return Err("verdicts.verdicts must allow at least one verdict".into());
    }
    if let Some(extra) = v
        .require_reason_on
        .iter()
        .find(|r| !v.verdicts.contains(*r))
    {
        return Err(format!(
            "verdicts.requireReasonOn names `{}`, which is not an allowed verdict",
            extra.label().to_lowercase()
        ));
    }
    validate_target(&v.target)
}

/// Validate and normalize an `ask_user_questions` request. Both the MCP
/// subprocess and the GUI run it; the GUI never trusts the subprocess's word.
pub fn validate_request(value: &Value) -> Result<QuestionRequest, String> {
    if !value.is_object() {
        return Err("the request must be a JSON object".into());
    }
    let size = serde_json::to_string(value)
        .map(|s| s.len())
        .unwrap_or(usize::MAX);
    if size > REQUEST_MAX_BYTES {
        return Err(format!(
            "the request is {size} bytes; the limit is {REQUEST_MAX_BYTES}"
        ));
    }
    let mut req: QuestionRequest =
        serde_json::from_value(value.clone()).map_err(|e| format!("invalid request: {e}"))?;
    optional_text(&req.title, "title", LABEL_MAX)?;
    optional_text(&req.description, "description", PROMPT_MAX)?;
    optional_text(&req.submit_label, "submitLabel", LABEL_MAX)?;
    match req.mode {
        Mode::Questions => {
            validate_questions(&mut req.questions)?;
            req.confirmation = None;
            req.checkbox = None;
            req.verdicts = None;
        }
        Mode::Confirmation => {
            let c = req
                .confirmation
                .as_ref()
                .ok_or("confirmation mode needs a `confirmation` object")?;
            validate_confirmation(c)?;
            req.questions.clear();
            req.checkbox = None;
            req.verdicts = None;
        }
        Mode::Checkbox => {
            let c = req
                .checkbox
                .as_mut()
                .ok_or("checkbox mode needs a `checkbox` object")?;
            // A checkbox row has no outcome line: drop one rather than refuse.
            for option in c.options.iter_mut() {
                option.outcome = None;
            }
            validate_checkbox(c)?;
            req.questions.clear();
            req.confirmation = None;
            req.verdicts = None;
        }
        Mode::Verdicts => {
            let v = req
                .verdicts
                .as_mut()
                .ok_or("verdicts mode needs a `verdicts` object")?;
            validate_verdicts(v)?;
            req.questions.clear();
            req.confirmation = None;
            req.checkbox = None;
        }
    }
    Ok(req)
}

/// One question's answer: chosen option ids, a custom `other` text, or the
/// free `text` of a text question.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionAnswer {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selected: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemVerdict {
    pub verdict: Verdict,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A submitted answer. Which fields apply depends on the request's mode:
/// `answers` (questions), `accepted` + `note` (confirmation), `selected`
/// (checkbox), `verdicts` + `note` (verdicts).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub answers: BTreeMap<String, QuestionAnswer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selected: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub verdicts: BTreeMap<String, ItemVerdict>,
}

fn non_empty(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn question_label(q: &Question) -> &str {
    q.header
        .as_deref()
        .filter(|h| !h.trim().is_empty())
        .unwrap_or(&q.prompt)
}

fn check_text_answer(q: &Question, text: &str) -> Result<(), String> {
    let label = question_label(q);
    if chars(text) > TEXT_ANSWER_MAX {
        return Err(format!("the answer to `{label}` is too long"));
    }
    let Some(tv) = &q.text_validation else {
        return Ok(());
    };
    let n = chars(text) as u32;
    if tv.min_length.is_some_and(|min| n < min) {
        return Err(format!("the answer to `{label}` is too short"));
    }
    if tv.max_length.is_some_and(|max| n > max) {
        return Err(format!("the answer to `{label}` is too long"));
    }
    if let Some(pattern) = &tv.pattern {
        if !compile_pattern(pattern)?.is_match(text) {
            return Err(format!(
                "the answer to `{label}` does not match the expected format"
            ));
        }
    }
    let number = match tv.input_type {
        InputType::Text => return Ok(()),
        InputType::Number => text
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .ok_or_else(|| format!("the answer to `{label}` must be a number"))?,
        InputType::Integer => text
            .parse::<i64>()
            .map(|n| n as f64)
            .map_err(|_| format!("the answer to `{label}` must be a whole number"))?,
    };
    if tv.minimum.is_some_and(|min| number < min) || tv.maximum.is_some_and(|max| number > max) {
        return Err(format!("the answer to `{label}` is out of range"));
    }
    Ok(())
}

fn check_choices(
    selected: &[String],
    options: &[QuestionOption],
    what: &str,
) -> Result<(), String> {
    unique_ids(selected.iter().map(String::as_str), "selected option")?;
    for id in selected {
        if !options.iter().any(|o| &o.id == id) {
            return Err(format!("{what}: unknown option `{id}`"));
        }
    }
    Ok(())
}

/// Validate a submitted answer against its request and return it normalized
/// (empty custom texts dropped, texts trimmed).
pub fn validate_answer(req: &QuestionRequest, value: &Value) -> Result<Answer, String> {
    let mut answer: Answer =
        serde_json::from_value(value.clone()).map_err(|e| format!("invalid answer: {e}"))?;
    match req.mode {
        Mode::Questions => {
            if let Some(unknown) = answer
                .answers
                .keys()
                .find(|id| !req.questions.iter().any(|q| &&q.id == id))
            {
                return Err(format!("unknown question `{unknown}`"));
            }
            for q in &req.questions {
                let label = question_label(q).to_string();
                let a = answer.answers.entry(q.id.clone()).or_default();
                a.other = non_empty(&a.other);
                a.text = non_empty(&a.text);
                match q.answer_mode {
                    AnswerMode::Single | AnswerMode::Multi => {
                        if a.text.is_some() {
                            return Err(format!("`{label}` takes a choice, not free text"));
                        }
                        check_choices(&a.selected, &q.options, &label)?;
                        if q.answer_mode == AnswerMode::Single && a.selected.len() > 1 {
                            return Err(format!("`{label}` takes one choice"));
                        }
                        if let Some(other) = &a.other {
                            if !q.allow_custom {
                                return Err(format!("`{label}` does not accept a custom answer"));
                            }
                            if chars(other) > PROMPT_MAX {
                                return Err(format!("the custom answer to `{label}` is too long"));
                            }
                            if q.answer_mode == AnswerMode::Single && !a.selected.is_empty() {
                                return Err(format!(
                                    "`{label}`: choose one option or Other, not both"
                                ));
                            }
                        }
                        if q.required && a.selected.is_empty() && a.other.is_none() {
                            return Err(format!("`{label}` needs an answer"));
                        }
                    }
                    AnswerMode::Text => {
                        if !a.selected.is_empty() || a.other.is_some() {
                            return Err(format!("`{label}` takes free text, not a choice"));
                        }
                        match &a.text {
                            None if q.required => return Err(format!("`{label}` needs an answer")),
                            None => {}
                            Some(text) => check_text_answer(q, text)?,
                        }
                    }
                }
            }
            answer.accepted = None;
            answer.selected.clear();
            answer.verdicts.clear();
        }
        Mode::Confirmation => {
            if answer.accepted.is_none() {
                return Err("choose accept or reject".into());
            }
            answer.note = non_empty(&answer.note);
            optional_text(&answer.note, "the note", PROMPT_MAX)?;
            answer.answers.clear();
            answer.selected.clear();
            answer.verdicts.clear();
        }
        Mode::Checkbox => {
            let c = req.checkbox.as_ref().ok_or("the request has no checkbox")?;
            check_choices(&answer.selected, &c.options, "checkbox")?;
            let n = answer.selected.len() as u32;
            if c.min_selected.is_some_and(|min| n < min) {
                return Err("select more options".into());
            }
            if c.max_selected.is_some_and(|max| n > max) {
                return Err("select fewer options".into());
            }
            answer.answers.clear();
            answer.accepted = None;
            answer.note = None;
            answer.verdicts.clear();
        }
        Mode::Verdicts => {
            let v = req
                .verdicts
                .as_ref()
                .ok_or("the request has no verdict items")?;
            if let Some(unknown) = answer
                .verdicts
                .keys()
                .find(|id| !v.items.iter().any(|i| &&i.id == id))
            {
                return Err(format!("unknown item `{unknown}`"));
            }
            for item in &v.items {
                let Some(given) = answer.verdicts.get_mut(&item.id) else {
                    return Err(format!("`{}` needs a verdict", item.label));
                };
                if !v.verdicts.contains(&given.verdict) {
                    return Err(format!(
                        "`{}`: {} is not an allowed verdict",
                        item.label,
                        given.verdict.label().to_lowercase()
                    ));
                }
                given.reason = non_empty(&given.reason);
                optional_text(&given.reason, "a reason", PROMPT_MAX)?;
                if v.require_reason_on.contains(&given.verdict) && given.reason.is_none() {
                    return Err(format!("`{}` needs a reason", item.label));
                }
            }
            answer.note = non_empty(&answer.note);
            optional_text(&answer.note, "the note", PROMPT_MAX)?;
            answer.answers.clear();
            answer.accepted = None;
            answer.selected.clear();
        }
    }
    Ok(answer)
}

/// Strip control characters (tabs become spaces), keeping line breaks, and
/// trim each line's end.
fn clean_value(value: &str) -> String {
    value
        .lines()
        .map(|line| {
            line.chars()
                .map(|c| if c == '\t' { ' ' } else { c })
                .filter(|c| !c.is_control())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A label on one line: whitespace collapsed, control characters dropped,
/// at most `cap` characters ([`LABEL_MAX`] for a short label, [`PROMPT_MAX`]
/// for a card's own prompt).
fn one_line_capped(label: &str, cap: usize) -> String {
    let collapsed = label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>();
    collapsed.chars().take(cap).collect()
}

pub(crate) fn push_line(out: &mut String, label: &str, value: &str) {
    push_line_capped(out, label, value, LABEL_MAX);
}

/// A short label on one line ([`one_line_capped`] at [`LABEL_MAX`]): an
/// ask-canvas summary line without a `label: value` split (`oneLine` in
/// `src/canvas/askAnswer.ts`; the phone's canvas answers, v1.2).
pub(crate) fn one_line(label: &str) -> String {
    one_line_capped(label, LABEL_MAX)
}

/// [`push_line`] with the label cut at `cap` characters (a question's whole
/// prompt may run to [`PROMPT_MAX`]).
fn push_line_capped(out: &mut String, label: &str, value: &str, cap: usize) {
    out.push_str("\n- ");
    out.push_str(&one_line_capped(label, cap));
    out.push_str(": ");
    let value = clean_value(value);
    let mut lines = value.trim().lines();
    out.push_str(lines.next().unwrap_or(""));
    for line in lines {
        out.push_str("\n  ");
        out.push_str(line);
    }
}

fn chosen_labels(options: &[QuestionOption], selected: &[String]) -> Vec<String> {
    options
        .iter()
        .filter(|o| selected.contains(&o.id))
        .map(|o| o.label.trim().to_string())
        .collect()
}

/// One question, then its answer on the next line. Continuation lines of a
/// multiline answer stay indented two spaces. The question is one line, cut
/// at `cap` ([`PROMPT_MAX`] for a card's own prompt, [`LABEL_MAX`] for a
/// short label such as Target).
fn push_pair(out: &mut String, question: &str, value: &str, cap: usize) {
    out.push_str("\n\n");
    out.push_str(&one_line_capped(question, cap));
    out.push('\n');
    let value = clean_value(value);
    let mut lines = value.trim().lines();
    out.push_str(lines.next().unwrap_or(""));
    for line in lines {
        out.push_str("\n  ");
        out.push_str(line);
    }
}

/// The answer turn: `Answered questions`, then each question on its own line
/// and the answer on the next (continuation lines indented two spaces),
/// capped at [`ANSWER_PROMPT_MAX`] characters. Each question is named by its
/// whole prompt (up to [`PROMPT_MAX`]), not its short header, and a custom
/// answer is the person's own words with no `Other:` prefix. A verdicts card
/// leads with its own question (2026-11-04): the answers then read on their
/// own in whichever native session gets them, the one that asked or, when
/// the conversation has moved on (a fresh session after an hour away, a
/// reset, a check-in's card answered later), another.
pub fn format_answers(req: &QuestionRequest, answer: &Answer) -> String {
    let mut out = String::from(ANSWER_HEADER);
    match req.mode {
        Mode::Questions => {
            for q in &req.questions {
                let a = answer.answers.get(&q.id).cloned().unwrap_or_default();
                let value = match q.answer_mode {
                    AnswerMode::Single | AnswerMode::Multi => {
                        let mut parts = chosen_labels(&q.options, &a.selected);
                        if let Some(other) = non_empty(&a.other) {
                            parts.push(other);
                        }
                        if parts.is_empty() {
                            "No answer".to_string()
                        } else {
                            parts.join(", ")
                        }
                    }
                    AnswerMode::Text => non_empty(&a.text).unwrap_or_else(|| "No answer".into()),
                };
                push_pair(&mut out, &q.prompt, &value, PROMPT_MAX);
            }
        }
        Mode::Confirmation => {
            if let Some(c) = &req.confirmation {
                let value = if answer.accepted == Some(true) {
                    "Accepted"
                } else {
                    "Rejected"
                };
                push_pair(&mut out, &c.prompt, value, PROMPT_MAX);
                if let Some(path) = c.target.as_ref().map(|t| t.path.as_str()) {
                    push_pair(&mut out, "Target", path, LABEL_MAX);
                }
            }
            if let Some(note) = non_empty(&answer.note) {
                push_pair(&mut out, "Note", &note, LABEL_MAX);
            }
        }
        Mode::Checkbox => {
            if let Some(c) = &req.checkbox {
                let chosen = chosen_labels(&c.options, &answer.selected);
                let value = if chosen.is_empty() {
                    "None selected".to_string()
                } else {
                    chosen.join(", ")
                };
                push_pair(&mut out, &c.prompt, &value, PROMPT_MAX);
            }
        }
        Mode::Verdicts => {
            if let Some(v) = &req.verdicts {
                push_pair(&mut out, "Question", &v.prompt, PROMPT_MAX);
                for item in &v.items {
                    let value = match answer.verdicts.get(&item.id) {
                        Some(given) => match non_empty(&given.reason) {
                            Some(reason) => format!("{} (reason: {reason})", given.verdict.label()),
                            None => given.verdict.label().to_string(),
                        },
                        None => "No answer".to_string(),
                    };
                    push_pair(&mut out, &item.label, &value, LABEL_MAX);
                }
            }
            if let Some(note) = non_empty(&answer.note) {
                push_pair(&mut out, "Note", &note, LABEL_MAX);
            }
        }
    }
    cap_prompt(out)
}

fn cap_prompt(out: String) -> String {
    const TAIL: &str = "\n(answers truncated)";
    if chars(&out) <= ANSWER_PROMPT_MAX {
        return out;
    }
    let keep = ANSWER_PROMPT_MAX - chars(TAIL);
    let mut capped: String = out.chars().take(keep).collect();
    capped.push_str(TAIL);
    capped
}

/// `Some` when a tool use is a question-card call: its arguments as a JSON
/// object, or why they cannot be read. `None` for every other tool.
pub fn detect_ask(name: &str, input: &Value) -> Option<Result<Value, String>> {
    if name != QUALIFIED_TOOL {
        return None;
    }
    Some(match input {
        Value::Object(_) => Ok(input.clone()),
        Value::String(raw) => serde_json::from_str::<Value>(raw)
            .ok()
            .filter(Value::is_object)
            .ok_or_else(|| "the arguments are not a JSON object".to_string()),
        Value::Null => Err("the call carried no arguments".into()),
        _ => Err("the arguments are not a JSON object".into()),
    })
}

/// Whether a raw Claude stream line belongs to a subagent: `Task` subagent
/// tool calls arrive as ordinary `assistant`/`user` events, told apart only by
/// a non-null top-level `parent_tool_use_id`. A card from a subagent would
/// outlive the question behind it, so those calls are never recorded.
pub fn claude_is_nested(raw_line: &str) -> bool {
    serde_json::from_str::<Value>(raw_line)
        .ok()
        .and_then(|v| {
            v.get("parent_tool_use_id")
                .and_then(Value::as_str)
                .map(|s| !s.is_empty())
        })
        .unwrap_or(false)
}

/// A question-card call the recorder saw complete successfully.
#[derive(Debug, Clone, PartialEq)]
pub struct AskCall {
    pub tool_use_id: String,
    /// The request as sent, or why it could not be read (recorded as a
    /// visible rejection, never silently dropped).
    pub arguments: Result<Value, String>,
    /// The native session that asked: its answers go there when the
    /// conversation still resumes it, else as an ordinary message.
    pub native_session_id: Option<String>,
}

/// Watches one chat turn's parsed events for question-card calls. It tracks
/// the native session id itself (seeded with the resume id, then every
/// `Session` event), so the id stored with a card is exactly the one the next
/// turn resumes. Pure: fixture replays drive it in tests.
#[derive(Debug, Default)]
pub struct AskRecorder {
    native: Option<String>,
    pending: HashMap<String, Result<Value, String>>,
}

fn usable_native(id: &str) -> bool {
    !id.is_empty() && id != "antigravity"
}

impl AskRecorder {
    pub fn new(resume: Option<String>) -> Self {
        AskRecorder {
            native: resume.filter(|id| usable_native(id)),
            pending: HashMap::new(),
        }
    }

    /// The native session id seen so far.
    pub fn native(&self) -> Option<&str> {
        self.native.as_deref()
    }

    /// Feed one parsed event and the raw line it came from. Returns the call
    /// once its result says it was posted: `is_error` false and the text
    /// starting with [`POSTED_PREFIX`]. A tool use id is remembered from its
    /// (possibly repeated) `ToolUse` and forgotten at its result either way.
    pub fn feed(&mut self, agent: AgentKind, raw_line: &str, evt: &AgentEvent) -> Option<AskCall> {
        match evt {
            AgentEvent::Session(id) => {
                if usable_native(id) {
                    self.native = Some(id.clone());
                }
                None
            }
            AgentEvent::ToolUse { id, name, input } => {
                let arguments = detect_ask(name, input)?;
                if agent == AgentKind::Claude && claude_is_nested(raw_line) {
                    return None;
                }
                self.pending.insert(id.clone(), arguments);
                None
            }
            AgentEvent::ToolResult {
                id,
                content,
                is_error,
            } => {
                let arguments = self.pending.remove(id)?;
                if *is_error || !content.starts_with(POSTED_PREFIX) {
                    return None;
                }
                Some(AskCall {
                    tool_use_id: id.clone(),
                    arguments,
                    native_session_id: self.native.clone(),
                })
            }
            _ => None,
        }
    }

    /// The fenced request at the end of an Antigravity reply, if any.
    pub fn fenced(&self, text: &str) -> Option<Result<Value, String>> {
        extract_fenced(text)
    }
}

/// The last complete ```` ```hollow-questions ```` block in `text`, parsed.
/// `None` when there is none; `Some(Err)` when its body is not valid JSON.
pub fn extract_fenced(text: &str) -> Option<Result<Value, String>> {
    let mut last: Option<String> = None;
    let mut in_block = false;
    let mut body: Vec<&str> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if !in_block {
            if trimmed
                .strip_prefix("```")
                .is_some_and(|rest| rest.trim() == FENCE_LANG)
            {
                in_block = true;
                body.clear();
            }
        } else if trimmed.len() >= 3 && trimmed.chars().all(|c| c == '`') {
            last = Some(body.join("\n"));
            in_block = false;
        } else {
            body.push(line);
        }
    }
    last.map(|body| {
        serde_json::from_str::<Value>(&body)
            .map_err(|e| format!("the hollow-questions block is not valid JSON: {e}"))
    })
}

/// The MCP tool definition `question_mcp` lists.
pub fn tool_spec() -> Value {
    let option = json!({
        "type": "object",
        "properties": {
            "id": { "type": "string", "description": "1-40 letters, digits, _ or -" },
            "label": { "type": "string" },
            "description": { "type": "string" },
            "recommended": { "type": "boolean" }
        },
        "required": ["id", "label"]
    });
    // A question's option also says what it does in the question's scenario.
    let mut question_option = option.clone();
    question_option["properties"]["outcome"] = json!({ "type": "string", "description": "What choosing this does in the scenario: one plain line, with numbers." });
    let target = json!({
        "type": "object",
        "description": "The file this decision is about; the card goes stale if it changes before the user answers.",
        "properties": { "path": { "type": "string" } },
        "required": ["path"]
    });
    json!({
        "name": TOOL,
        "description": "Ask the user structured questions as a card in Hollow's chat, then END YOUR TURN. Use only when you are blocked on a decision only the user can make. For the COO only: never call it from a subagent or a delegated task. Ask at most 5 questions, offer concrete options, and mark the one you recommend. When this returns \"Posted\", stop: call no more tools. The answers arrive as the user's next message, starting \"Answered questions\", then each question on its own line and the answer on the next. A custom answer is the person's words, with no Other: prefix. Modes: questions (default: single, multi or text answers), confirmation (accept or reject, optionally bound to a target file), checkbox (pick several), verdicts (approve, reject or defer each item).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "mode": { "type": "string", "enum": ["questions", "confirmation", "checkbox", "verdicts"] },
                "title": { "type": "string" },
                "description": { "type": "string" },
                "submitLabel": { "type": "string" },
                "supersedeOnUserMessage": { "type": "boolean", "description": "Default true: a free-text message from the user closes the card." },
                "questions": {
                    "type": "array",
                    "maxItems": MAX_QUESTIONS,
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "prompt": { "type": "string" },
                            "header": { "type": "string", "description": "Short label the card shows for the question (the answers name the whole prompt)." },
                            "reason": { "type": "string", "description": "One line under the question: why you are asking. The card shows this line and nothing longer." },
                            "helpText": { "type": "string", "description": "Shown under the question as one line when reason is absent." },
                            "scenario": { "type": "string", "description": "For a decision: one concrete scenario with numbers that every option's outcome is about (card shows it as one short paragraph, 600 characters at most)." },
                            "required": { "type": "boolean" },
                            "answerMode": { "type": "string", "enum": ["single", "multi", "text"] },
                            "options": { "type": "array", "maxItems": MAX_OPTIONS, "items": question_option },
                            "allowCustom": { "type": "boolean", "description": "Default true: an \"Other…\" row for the user's own words. Set false only when no other answer makes sense." },
                            "customLabel": { "type": "string" },
                            "textValidation": {
                                "type": "object",
                                "properties": {
                                    "minLength": { "type": "integer" },
                                    "maxLength": { "type": "integer" },
                                    "pattern": { "type": "string", "description": "Must match the whole answer." },
                                    "inputType": { "type": "string", "enum": ["text", "number", "integer"] },
                                    "minimum": { "type": "number" },
                                    "maximum": { "type": "number" }
                                }
                            }
                        },
                        "required": ["id", "prompt"]
                    }
                },
                "confirmation": {
                    "type": "object",
                    "properties": {
                        "prompt": { "type": "string" },
                        "detailsMarkdown": { "type": "string" },
                        "acceptLabel": { "type": "string" },
                        "rejectLabel": { "type": "string" },
                        "target": target.clone()
                    },
                    "required": ["prompt"]
                },
                "checkbox": {
                    "type": "object",
                    "properties": {
                        "prompt": { "type": "string" },
                        "detailsMarkdown": { "type": "string" },
                        "options": { "type": "array", "maxItems": MAX_CHOICES, "items": option.clone() },
                        "defaultSelected": { "type": "array", "items": { "type": "string" } },
                        "minSelected": { "type": "integer" },
                        "maxSelected": { "type": "integer" },
                        "target": target.clone()
                    },
                    "required": ["prompt", "options"]
                },
                "verdicts": {
                    "type": "object",
                    "properties": {
                        "prompt": { "type": "string" },
                        "detailsMarkdown": { "type": "string" },
                        "items": {
                            "type": "array",
                            "maxItems": MAX_CHOICES,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "id": { "type": "string" },
                                    "label": { "type": "string" },
                                    "description": { "type": "string" },
                                    "previewMarkdown": { "type": "string" }
                                },
                                "required": ["id", "label"]
                            }
                        },
                        "verdicts": { "type": "array", "items": { "type": "string", "enum": ["approve", "reject", "defer"] } },
                        "requireReasonOn": { "type": "array", "items": { "type": "string", "enum": ["approve", "reject", "defer"] } },
                        "allowBulkApprove": { "type": "boolean" },
                        "target": target.clone()
                    },
                    "required": ["prompt", "items"]
                }
            }
        },
        "annotations": {
            "title": "Ask the user",
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        }
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const CLAUDE_ASK: &str = include_str!("../fixtures/interactions/claude-ask.jsonl");
    pub(crate) const CLAUDE_ASK_ERROR: &str =
        include_str!("../fixtures/interactions/claude-ask-error.jsonl");
    pub(crate) const CLAUDE_ASK_NESTED: &str =
        include_str!("../fixtures/interactions/claude-ask-nested.jsonl");
    pub(crate) const CODEX_ASK: &str = include_str!("../fixtures/interactions/codex-ask.jsonl");
    pub(crate) const CODEX_ASK_TURN2: &str =
        include_str!("../fixtures/interactions/codex-ask-turn2.jsonl");
    pub(crate) const CODEX_ASK_FAILED: &str =
        include_str!("../fixtures/interactions/codex-ask-failed.jsonl");
    pub(crate) const CODEX_ASK_REJECTED: &str =
        include_str!("../fixtures/interactions/codex-ask-rejected.jsonl");

    /// Replay a recorded (or synthetic) stream through the real parser and a
    /// fresh recorder, the way the chat drive closure does.
    pub(crate) fn replay(
        kind: AgentKind,
        text: &str,
        resume: Option<&str>,
    ) -> (Vec<AskCall>, AskRecorder) {
        let mut state = crate::agent::ParseState::default();
        let mut recorder = AskRecorder::new(resume.map(str::to_string));
        let mut calls = Vec::new();
        for line in text.lines() {
            for evt in crate::agent::parse_line(kind, line, &mut state) {
                if let Some(call) = recorder.feed(kind, line, &evt) {
                    calls.push(call);
                }
            }
        }
        (calls, recorder)
    }

    pub(crate) fn layout_request() -> Value {
        json!({
            "questions": [
                { "id": "layout", "prompt": "Which layout?", "options": [
                    { "id": "grid", "label": "Grid", "recommended": true },
                    { "id": "list", "label": "List" }
                ]},
                { "id": "notes", "prompt": "Anything else?", "answerMode": "text", "required": false }
            ]
        })
    }

    #[test]
    fn validate_request_accepts_every_mode_and_fills_defaults() {
        let questions = validate_request(&layout_request()).unwrap();
        assert_eq!(questions.mode, Mode::Questions);
        assert!(questions.supersede_on_user_message);
        assert!(questions.questions[0].required, "required defaults on");
        assert_eq!(questions.questions[0].answer_mode, AnswerMode::Single);
        assert!(
            questions.questions[0].options[0].recommended,
            "recommended survives"
        );
        assert!(!questions.questions[1].required);
        let stored = serde_json::to_value(&questions).unwrap();
        assert_eq!(stored["questions"][0]["answerMode"], json!("single"));
        assert_eq!(stored["supersedeOnUserMessage"], json!(true));

        let confirm = validate_request(&json!({
            "mode": "request_confirmation",
            "confirmation": { "prompt": "Delete it?", "target": { "path": "notes/a.md" } },
            "questions": [{ "id": "x", "prompt": "ignored" }]
        }))
        .unwrap();
        assert_eq!(confirm.mode, Mode::Confirmation);
        assert!(
            confirm.questions.is_empty(),
            "other modes' sections are cleared"
        );
        assert_eq!(confirm.target_path(), Some("notes/a.md"));

        let checkbox = validate_request(&json!({
            "mode": "checkbox",
            "checkbox": { "prompt": "Which?", "options": [{ "id": "a", "label": "A" }, { "id": "b", "label": "B" }], "max_selected": 1 }
        }))
        .unwrap();
        assert_eq!(
            checkbox.checkbox.unwrap().max_selected,
            Some(1),
            "snake_case aliases are read"
        );

        let verdicts = validate_request(&json!({
            "mode": "verdicts",
            "verdicts": { "prompt": "Review", "items": [{ "id": "one", "label": "One" }], "requireReasonOn": ["reject"] }
        }))
        .unwrap();
        assert_eq!(verdicts.question_count(), 1);
        let v = verdicts.verdicts.unwrap();
        assert_eq!(v.verdicts, all_verdicts(), "all verdicts by default");
        assert!(v.allow_bulk_approve);
    }

    #[test]
    fn validate_request_rejects_bad_shapes() {
        let long = "x".repeat(PROMPT_MAX + 1);
        let too_many: Vec<Value> = (0..=MAX_QUESTIONS)
            .map(|i| json!({ "id": format!("q{i}"), "prompt": "p", "options": [{ "id": "a", "label": "A" }] }))
            .collect();
        let many_options: Vec<Value> = (0..=MAX_OPTIONS)
            .map(|i| json!({ "id": format!("o{i}"), "label": "L" }))
            .collect();
        let cases = [
            json!("not an object"),
            json!({ "questions": [] }),
            json!({ "questions": too_many }),
            json!({ "questions": [{ "id": "bad id", "prompt": "p", "options": [{ "id": "a", "label": "A" }] }] }),
            json!({ "questions": [{ "id": "a", "prompt": "p", "options": [{ "id": "a", "label": "A" }] }, { "id": "a", "prompt": "q", "options": [{ "id": "a", "label": "A" }] }] }),
            json!({ "questions": [{ "id": "a", "prompt": "  ", "options": [{ "id": "a", "label": "A" }] }] }),
            json!({ "questions": [{ "id": "a", "prompt": long, "options": [{ "id": "a", "label": "A" }] }] }),
            json!({ "questions": [{ "id": "a", "prompt": "p" }] }),
            json!({ "questions": [{ "id": "a", "prompt": "p", "options": many_options }] }),
            json!({ "questions": [{ "id": "a", "prompt": "p", "options": [{ "id": "x", "label": "X" }, { "id": "x", "label": "Y" }] }] }),
            json!({ "questions": [{ "id": "a", "prompt": "p", "answerMode": "text", "options": [{ "id": "x", "label": "X" }] }] }),
            json!({ "questions": [{ "id": "a", "prompt": "p", "answerMode": "text", "textValidation": { "pattern": "(" } }] }),
            json!({ "questions": [{ "id": "a", "prompt": "p", "answerMode": "text", "textValidation": { "minLength": 5, "maxLength": 2 } }] }),
            json!({ "questions": [{ "id": "a", "prompt": "p", "answerMode": "text", "textValidation": { "inputType": "integer", "minimum": 1.5 } }] }),
            json!({ "questions": [{ "id": "a", "prompt": "p", "answerMode": "text", "textValidation": { "minimum": 1 } }] }),
            json!({ "questions": [{ "id": "a", "prompt": "p", "answerMode": "maybe" }] }),
            json!({ "mode": "confirmation" }),
            json!({ "mode": "confirmation", "confirmation": { "prompt": "ok?", "target": { "path": "" } } }),
            json!({ "mode": "checkbox", "checkbox": { "prompt": "p", "options": [] } }),
            json!({ "mode": "checkbox", "checkbox": { "prompt": "p", "options": [{ "id": "a", "label": "A" }], "defaultSelected": ["z"] } }),
            json!({ "mode": "checkbox", "checkbox": { "prompt": "p", "options": [{ "id": "a", "label": "A" }], "maxSelected": 2 } }),
            json!({ "mode": "verdicts", "verdicts": { "prompt": "p", "items": [] } }),
            json!({ "mode": "verdicts", "verdicts": { "prompt": "p", "items": [{ "id": "a", "label": "A" }], "verdicts": ["approve"], "requireReasonOn": ["reject"] } }),
            json!({ "title": "x".repeat(LABEL_MAX + 1), "questions": [{ "id": "a", "prompt": "p", "options": [{ "id": "a", "label": "A" }] }] }),
            json!({ "questions": [{ "id": "a", "prompt": "x".repeat(REQUEST_MAX_BYTES), "options": [{ "id": "a", "label": "A" }] }] }),
        ];
        for case in cases {
            assert!(validate_request(&case).is_err(), "accepted: {case}");
        }
    }

    /// Question cards, 2026-10-14: a choice question takes the person's own
    /// words ("Other…") unless the agent turns that off; a text question never.
    #[test]
    fn other_is_on_unless_the_agent_turns_it_off() {
        let req = validate_request(&json!({
            "questions": [
                { "id": "a", "prompt": "A?", "options": [{ "id": "x", "label": "X" }] },
                { "id": "b", "prompt": "B?", "allowCustom": false, "options": [{ "id": "x", "label": "X" }] },
                { "id": "c", "prompt": "C?", "answerMode": "text" }
            ]
        }))
        .unwrap();
        let custom: Vec<bool> = req.questions.iter().map(|q| q.allow_custom).collect();
        assert_eq!(custom, [true, false, false]);
        assert!(validate_answer(&req, &json!({ "answers": { "a": { "other": "mine" }, "b": { "selected": ["x"] }, "c": { "text": "t" } } })).is_ok());
        assert!(validate_answer(&req, &json!({ "answers": { "a": { "selected": ["x"] }, "b": { "other": "mine" }, "c": { "text": "t" } } })).is_err());
    }

    #[test]
    fn validate_answer_enforces_the_request() {
        let req = validate_request(&json!({
            "questions": [
                { "id": "layout", "prompt": "Which layout?", "allowCustom": false, "options": [{ "id": "grid", "label": "Grid" }, { "id": "list", "label": "List" }] },
                { "id": "tags", "prompt": "Tags?", "answerMode": "multi", "required": false, "allowCustom": true, "options": [{ "id": "a", "label": "A" }, { "id": "b", "label": "B" }] },
                { "id": "code", "prompt": "Code?", "answerMode": "text", "required": false, "textValidation": { "pattern": "a+" } },
                { "id": "count", "prompt": "How many?", "answerMode": "text", "required": false, "textValidation": { "inputType": "integer", "minimum": 1, "maximum": 5 } }
            ]
        }))
        .unwrap();
        let ok = |answers: Value| validate_answer(&req, &json!({ "answers": answers }));
        assert!(ok(json!({ "layout": { "selected": ["grid"] } })).is_ok());
        let bad = [
            json!({}),
            json!({ "layout": { "selected": ["grid", "list"] } }),
            json!({ "layout": { "selected": ["nope"] } }),
            json!({ "layout": { "other": "mine" } }),
            json!({ "layout": { "text": "grid" } }),
            json!({ "layout": { "selected": ["grid"] }, "ghost": { "text": "x" } }),
            json!({ "layout": { "selected": ["grid"] }, "tags": { "selected": ["a", "a"] } }),
            json!({ "layout": { "selected": ["grid"] }, "code": { "text": "xa" } }),
            json!({ "layout": { "selected": ["grid"] }, "count": { "text": "7" } }),
            json!({ "layout": { "selected": ["grid"] }, "count": { "text": "2.5" } }),
            json!({ "layout": { "selected": ["grid"] }, "code": { "selected": ["a"] } }),
        ];
        for answers in bad {
            assert!(ok(answers.clone()).is_err(), "accepted: {answers}");
        }
        let full = ok(json!({
            "layout": { "selected": ["list"] },
            "tags": { "selected": ["b"], "other": "  extra  " },
            "code": { "text": "aaa" },
            "count": { "text": "3" }
        }))
        .unwrap();
        assert_eq!(
            full.answers["tags"].other.as_deref(),
            Some("extra"),
            "custom text is trimmed"
        );

        let confirm = validate_request(
            &json!({ "mode": "confirmation", "confirmation": { "prompt": "Go?" } }),
        )
        .unwrap();
        assert!(validate_answer(&confirm, &json!({})).is_err());
        assert!(validate_answer(&confirm, &json!({ "accepted": true })).is_ok());

        let checkbox = validate_request(&json!({ "mode": "checkbox", "checkbox": { "prompt": "Pick", "options": [{ "id": "a", "label": "A" }, { "id": "b", "label": "B" }], "minSelected": 1, "maxSelected": 1 } })).unwrap();
        assert!(validate_answer(&checkbox, &json!({ "selected": [] })).is_err());
        assert!(validate_answer(&checkbox, &json!({ "selected": ["a", "b"] })).is_err());
        assert!(validate_answer(&checkbox, &json!({ "selected": ["b"] })).is_ok());

        let verdicts = validate_request(&json!({ "mode": "verdicts", "verdicts": { "prompt": "Review", "items": [{ "id": "one", "label": "One" }, { "id": "two", "label": "Two" }], "verdicts": ["approve", "reject"], "requireReasonOn": ["reject"] } })).unwrap();
        assert!(
            validate_answer(
                &verdicts,
                &json!({ "verdicts": { "one": { "verdict": "approve" } } })
            )
            .is_err(),
            "every item needs a verdict"
        );
        assert!(validate_answer(&verdicts, &json!({ "verdicts": { "one": { "verdict": "approve" }, "two": { "verdict": "defer" } } })).is_err(), "defer is not allowed");
        assert!(validate_answer(&verdicts, &json!({ "verdicts": { "one": { "verdict": "approve" }, "two": { "verdict": "reject" } } })).is_err(), "reject needs a reason");
        assert!(validate_answer(&verdicts, &json!({ "verdicts": { "one": { "verdict": "approve" }, "two": { "verdict": "reject", "reason": "risky" } } })).is_ok());
    }

    /// Canvas C4 A2: an ask-canvas's answer lines (`src/canvas/askAnswer.ts`
    /// `answerLine`) and a question card's are one shape; both are tested
    /// against the same golden cases.
    #[test]
    fn push_line_matches_the_answer_lines_shared_with_ask_canvases() {
        const GOLDEN: &str = include_str!("../../src/canvas/answerLines.golden.json");
        let golden: Value = serde_json::from_str(GOLDEN).unwrap();
        let cases = golden["cases"].as_array().unwrap();
        assert!(cases.len() > 10);
        for case in cases {
            let label = case["label"].as_str().unwrap();
            let value = case["value"].as_str().unwrap();
            let mut out = String::new();
            push_line(&mut out, label, value);
            assert_eq!(
                out,
                format!("\n{}", case["line"].as_str().unwrap()),
                "label {label:?}"
            );
        }
    }

    /// The answers name each question by its whole prompt (one line), never
    /// the header, so they read on their own in whichever session gets them;
    /// the first line stays the header every reader keys on.
    #[test]
    fn answers_name_each_question_in_full() {
        let multi = validate_request(&json!({ "questions": [
            { "id": "tags", "header": "Tags", "prompt": "Which tags should I apply to the note?", "answerMode": "multi", "allowCustom": true, "options": [{ "id": "a", "label": "Alpha" }, { "id": "b", "label": "Beta" }] },
            { "id": "why", "prompt": "Why?\n(one line)", "answerMode": "text" }
        ]}))
        .unwrap();
        let answer = validate_answer(
            &multi,
            &json!({ "answers": {
                "tags": { "selected": ["b"] },
                "why": { "text": "Because" }
            }}),
        )
        .unwrap();
        assert_eq!(
            format_answers(&multi, &answer),
            "Answered questions\n\nWhich tags should I apply to the note?\nBeta\n\nWhy? (one line)\nBecause"
        );
        // A long question (prompts run to PROMPT_MAX) arrives whole, never
        // cut at the 200-character label cap.
        let long = "Which of these should go first? ".repeat(15);
        let req = validate_request(&json!({ "questions": [
            { "id": "q", "header": "Order", "prompt": long, "answerMode": "text" }
        ]}))
        .unwrap();
        let answer =
            validate_answer(&req, &json!({ "answers": { "q": { "text": "The memo" } } })).unwrap();
        assert_eq!(
            format_answers(&req, &answer),
            format!("Answered questions\n\n{}\nThe memo", long.trim())
        );
    }

    #[test]
    fn format_answers_starts_with_the_header_and_one_line_per_answer() {
        let req = validate_request(&layout_request()).unwrap();
        let answer = validate_answer(
            &req,
            &json!({ "answers": { "layout": { "selected": ["grid"] } } }),
        )
        .unwrap();
        assert_eq!(
            format_answers(&req, &answer),
            "Answered questions\n\nWhich layout?\nGrid\n\nAnything else?\nNo answer"
        );

        let multi = validate_request(&json!({ "questions": [
            { "id": "tags", "header": "Tags", "prompt": "Which tags should I apply to the note?", "answerMode": "multi", "allowCustom": true, "options": [{ "id": "a", "label": "Alpha" }, { "id": "b", "label": "Beta" }] },
            { "id": "why", "prompt": "Why?\n(one line)", "answerMode": "text" }
        ]}))
        .unwrap();
        let answer = validate_answer(
            &multi,
            &json!({ "answers": {
                "tags": { "selected": ["b", "a"], "other": "Gamma" },
                "why": { "text": "First line\n\tsecond\u{7}line" }
            }}),
        )
        .unwrap();
        assert_eq!(
            format_answers(&multi, &answer),
            "Answered questions\n\nWhich tags should I apply to the note?\nAlpha, Beta, Gamma\n\nWhy? (one line)\nFirst line\n   secondline"
        );

        let confirm = validate_request(&json!({ "mode": "confirmation", "confirmation": { "prompt": "Delete the draft?", "target": { "path": "notes/draft.md" } } })).unwrap();
        let answer =
            validate_answer(&confirm, &json!({ "accepted": false, "note": "keep it" })).unwrap();
        assert_eq!(
            format_answers(&confirm, &answer),
            "Answered questions\n\nDelete the draft?\nRejected\n\nTarget\nnotes/draft.md\n\nNote\nkeep it"
        );

        let checkbox = validate_request(&json!({ "mode": "checkbox", "checkbox": { "prompt": "Which files?", "options": [{ "id": "a", "label": "a.md" }, { "id": "b", "label": "b.md" }] } })).unwrap();
        let none = validate_answer(&checkbox, &json!({ "selected": [] })).unwrap();
        assert_eq!(
            format_answers(&checkbox, &none),
            "Answered questions\n\nWhich files?\nNone selected"
        );
        // A confirmation's or checkbox card's long prompt arrives whole too,
        // never cut at the 200-character label cap.
        let long = "Should I delete every draft in the notes folder older than a week? ".repeat(5);
        let confirm = validate_request(
            &json!({ "mode": "confirmation", "confirmation": { "prompt": long } }),
        )
        .unwrap();
        let answer = validate_answer(&confirm, &json!({ "accepted": true })).unwrap();
        assert_eq!(
            format_answers(&confirm, &answer),
            format!("Answered questions\n\n{}\nAccepted", long.trim())
        );
        let checkbox = validate_request(&json!({ "mode": "checkbox", "checkbox": { "prompt": long, "options": [{ "id": "a", "label": "a.md" }] } })).unwrap();
        let none = validate_answer(&checkbox, &json!({ "selected": [] })).unwrap();
        assert_eq!(
            format_answers(&checkbox, &none),
            format!("Answered questions\n\n{}\nNone selected", long.trim())
        );

        let verdicts = validate_request(&json!({ "mode": "verdicts", "verdicts": { "prompt": "Review", "items": [{ "id": "one", "label": "One" }, { "id": "two", "label": "Two" }] } })).unwrap();
        let answer = validate_answer(&verdicts, &json!({ "verdicts": { "one": { "verdict": "approve" }, "two": { "verdict": "reject", "reason": "too risky" } } })).unwrap();
        assert_eq!(
            format_answers(&verdicts, &answer),
            "Answered questions\n\nQuestion\nReview\n\nOne\nApprove\n\nTwo\nReject (reason: too risky)"
        );

        // The whole turn is capped.
        let long: Vec<Value> = (0..MAX_QUESTIONS)
            .map(|i| json!({ "id": format!("q{i}"), "prompt": format!("Q{i}"), "answerMode": "text" }))
            .collect();
        let req = validate_request(&json!({ "questions": long })).unwrap();
        let answers: serde_json::Map<String, Value> = (0..MAX_QUESTIONS)
            .map(|i| {
                (
                    format!("q{i}"),
                    json!({ "text": "y".repeat(TEXT_ANSWER_MAX) }),
                )
            })
            .collect();
        let answer = validate_answer(&req, &json!({ "answers": answers })).unwrap();
        let capped = format_answers(&req, &answer);
        assert_eq!(capped.chars().count(), ANSWER_PROMPT_MAX);
        assert!(capped.starts_with(ANSWER_HEADER));
        assert!(capped.ends_with("(answers truncated)"));
    }

    #[test]
    fn detect_ask_matches_the_qualified_name_only() {
        let input = layout_request();
        assert_eq!(detect_ask(QUALIFIED_TOOL, &input), Some(Ok(input.clone())));
        assert_eq!(detect_ask("mcp__hollow_vault__search_vault", &input), None);
        assert_eq!(
            detect_ask(TOOL, &input),
            None,
            "the bare tool name is not a Hollow MCP call"
        );
        assert_eq!(detect_ask("AskUserQuestion", &input), None);
        assert_eq!(
            detect_ask(QUALIFIED_TOOL, &json!(input.to_string())),
            Some(Ok(input.clone()))
        );
        assert!(matches!(
            detect_ask(QUALIFIED_TOOL, &Value::Null),
            Some(Err(_))
        ));
        assert!(matches!(
            detect_ask(QUALIFIED_TOOL, &json!([1])),
            Some(Err(_))
        ));
    }

    #[test]
    fn claude_stream_fixture_yields_exactly_one_ask_after_its_result() {
        let (calls, recorder) = replay(AgentKind::Claude, CLAUDE_ASK, None);
        assert_eq!(
            calls.len(),
            1,
            "the repeated tool_use and ToolSearch add nothing"
        );
        assert_eq!(calls[0].tool_use_id, "toolu-001");
        assert_eq!(calls[0].native_session_id.as_deref(), Some("id-101"));
        let request = validate_request(calls[0].arguments.as_ref().unwrap()).unwrap();
        assert_eq!(request.questions.len(), 2);
        assert_eq!(recorder.native(), Some("id-101"));

        let (errored, _) = replay(AgentKind::Claude, CLAUDE_ASK_ERROR, None);
        assert!(errored.is_empty(), "an is_error result is never a card");
    }

    #[test]
    fn a_subagent_question_call_is_not_recorded() {
        let (calls, _) = replay(AgentKind::Claude, CLAUDE_ASK_NESTED, None);
        assert!(calls.is_empty(), "{calls:?}");
        assert!(claude_is_nested(
            r#"{"type":"assistant","parent_tool_use_id":"toolu-task"}"#
        ));
        assert!(!claude_is_nested(
            r#"{"type":"assistant","parent_tool_use_id":null}"#
        ));
        assert!(!claude_is_nested("not json"));
    }

    #[test]
    fn codex_stream_fixture_yields_exactly_one_ask() {
        let (calls, _) = replay(AgentKind::Codex, CODEX_ASK, None);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_use_id, "item_1");
        assert_eq!(calls[0].native_session_id.as_deref(), Some("id-201"));
        assert!(calls[0].arguments.as_ref().unwrap()["questions"].is_array());

        // A resumed turn reuses the item id; the recorder reports it again
        // (the store keys cards per run, not per item id).
        let (again, _) = replay(AgentKind::Codex, CODEX_ASK_TURN2, Some("id-201"));
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].tool_use_id, "item_1");
        assert_eq!(again[0].native_session_id.as_deref(), Some("id-201"));

        for (name, text) in [
            ("failed", CODEX_ASK_FAILED),
            ("rejected", CODEX_ASK_REJECTED),
        ] {
            let (calls, _) = replay(AgentKind::Codex, text, None);
            assert!(calls.is_empty(), "{name}: {calls:?}");
        }
    }

    #[test]
    fn the_recorder_ignores_the_antigravity_sentinel_and_keeps_the_resume_id() {
        let mut recorder = AskRecorder::new(Some("antigravity".into()));
        assert_eq!(recorder.native(), None);
        recorder = AskRecorder::new(Some("thread-9".into()));
        assert_eq!(recorder.native(), Some("thread-9"));
        recorder.feed(
            AgentKind::Antigravity,
            "",
            &AgentEvent::Session("antigravity".into()),
        );
        assert_eq!(recorder.native(), Some("thread-9"));
        recorder.feed(
            AgentKind::Antigravity,
            "",
            &AgentEvent::Session("conv-2".into()),
        );
        assert_eq!(recorder.native(), Some("conv-2"));
    }

    #[test]
    fn extract_fenced_takes_the_last_hollow_questions_block() {
        let text = "Intro\n```json\n{\"a\":1}\n```\n```hollow-questions\n{\"questions\":[1]}\n```\nthen\n  ```hollow-questions  \n{\"questions\":[2]}\n  ```\n";
        assert_eq!(extract_fenced(text), Some(Ok(json!({ "questions": [2] }))));
        assert!(matches!(
            extract_fenced("```hollow-questions\nnot json\n```"),
            Some(Err(_))
        ));
        assert_eq!(extract_fenced("```json\n{}\n```"), None);
        assert_eq!(
            extract_fenced("```hollow-questions\n{\"unterminated\":true}"),
            None
        );
        assert_eq!(extract_fenced("plain text"), None);
        let recorder = AskRecorder::default();
        assert_eq!(
            recorder.fenced("```hollow-questions\n{}\n```"),
            Some(Ok(json!({})))
        );
    }

    #[test]
    fn guidance_selects_mcp_or_fenced_and_is_byte_stable() {
        assert_eq!(guidance(true, false), Some(GUIDANCE_MCP));
        assert_eq!(guidance(false, true), Some(GUIDANCE_FENCED));
        assert_eq!(guidance(true, true), Some(GUIDANCE_MCP), "the tool wins");
        assert_eq!(guidance(false, false), None);
        for text in [GUIDANCE_MCP, GUIDANCE_FENCED] {
            assert!(text.contains("only when"));
            assert!(text.contains("COO"));
            assert!(text.contains(ANSWER_HEADER));
            assert!(
                !text.contains('/'),
                "path-free guidance keeps the prompt byte-stable"
            );
        }
        assert!(GUIDANCE_MCP.contains(TOOL));
        assert!(GUIDANCE_FENCED.contains(FENCE_LANG));
        // The fenced example is itself a valid request.
        let example = &GUIDANCE_FENCED
            [GUIDANCE_FENCED.find('{').unwrap()..=GUIDANCE_FENCED.rfind('}').unwrap()];
        assert!(validate_request(&serde_json::from_str::<Value>(example).unwrap()).is_ok());
    }

    #[test]
    fn a_question_reason_is_one_line_under_the_question() {
        let req = validate_request(&json!({ "questions": [
            { "id": "layout", "prompt": "Which layout?", "reason": "  The page\nneeds one. ", "options": [{ "id": "grid", "label": "Grid" }] }
        ]}))
        .unwrap();
        assert_eq!(
            req.questions[0].reason.as_deref(),
            Some("The page needs one.")
        );
        assert!(req.questions[0].help_text.is_none());
        let both = validate_request(&json!({ "questions": [
            { "id": "layout", "prompt": "Which layout?", "reason": "Short", "helpText": "Longer help that stays", "options": [{ "id": "grid", "label": "Grid" }] }
        ]}))
        .unwrap();
        assert_eq!(both.questions[0].reason.as_deref(), Some("Short"));
        assert_eq!(
            both.questions[0].help_text.as_deref(),
            Some("Longer help that stays")
        );
        let blank = validate_request(&json!({ "questions": [
            { "id": "layout", "prompt": "Which layout?", "reason": "  \n  ", "options": [{ "id": "grid", "label": "Grid" }] }
        ]}))
        .unwrap();
        assert!(blank.questions[0].reason.is_none());
        let long = "r".repeat(LABEL_MAX + 1);
        assert!(validate_request(&json!({ "questions": [
            { "id": "layout", "prompt": "Which layout?", "reason": long, "options": [{ "id": "grid", "label": "Grid" }] }
        ]}))
        .is_err());
    }

    fn decision_request() -> Value {
        json!({ "questions": [{
            "id": "plan",
            "prompt": "Which plan?",
            "scenario": "  You have 40 seats\nand pay $12 each. ",
            "options": [
                { "id": "annual", "label": "Annual", "outcome": " $384 a year,\n$96 less.", "description": "Cheapest for steady use.", "recommended": true },
                { "id": "monthly", "label": "Monthly" }
            ]
        }]})
    }

    #[test]
    fn a_decision_card_carries_a_scenario_and_a_per_option_outcome() {
        let req = validate_request(&decision_request()).unwrap();
        let q = &req.questions[0];
        assert_eq!(
            q.scenario.as_deref(),
            Some("You have 40 seats and pay $12 each.")
        );
        assert_eq!(
            q.options[0].outcome.as_deref(),
            Some("$384 a year, $96 less.")
        );
        assert!(q.options[1].outcome.is_none());
        // It round-trips in camelCase and an absent field stays absent.
        let value = serde_json::to_value(&req).unwrap();
        assert_eq!(
            value["questions"][0]["scenario"],
            json!("You have 40 seats and pay $12 each.")
        );
        assert_eq!(
            value["questions"][0]["options"][0]["outcome"],
            json!("$384 a year, $96 less.")
        );
        assert!(value["questions"][0]["options"][1].get("outcome").is_none());
        let again: QuestionRequest = serde_json::from_value(value).unwrap();
        assert_eq!(again.questions[0], req.questions[0]);
    }

    #[test]
    fn a_question_without_the_new_fields_is_unchanged() {
        let req = validate_request(&json!({ "questions": [
            { "id": "layout", "prompt": "Which layout?", "options": [{ "id": "grid", "label": "Grid" }] }
        ]}))
        .unwrap();
        assert!(req.questions[0].scenario.is_none());
        assert!(req.questions[0].options[0].outcome.is_none());
        let value = serde_json::to_value(&req).unwrap();
        assert!(value["questions"][0].get("scenario").is_none());
        assert!(value["questions"][0]["options"][0].get("outcome").is_none());
        let blank = validate_request(&json!({ "questions": [
            { "id": "layout", "prompt": "Which layout?", "scenario": " \n ", "options": [{ "id": "grid", "label": "Grid", "outcome": "  " }] }
        ]}))
        .unwrap();
        assert!(blank.questions[0].scenario.is_none());
        assert!(blank.questions[0].options[0].outcome.is_none());
    }

    #[test]
    fn scenario_and_outcome_have_caps() {
        let mut ok = decision_request();
        ok["questions"][0]["scenario"] = json!("s".repeat(SCENARIO_MAX));
        ok["questions"][0]["options"][0]["outcome"] = json!("o".repeat(OUTCOME_MAX));
        assert!(validate_request(&ok).is_ok());
        let mut long_scenario = decision_request();
        long_scenario["questions"][0]["scenario"] = json!("s".repeat(SCENARIO_MAX + 1));
        let err = validate_request(&long_scenario).unwrap_err();
        assert!(err.contains("scenario"), "{err}");
        let mut long_outcome = decision_request();
        long_outcome["questions"][0]["options"][0]["outcome"] = json!("o".repeat(OUTCOME_MAX + 1));
        let err = validate_request(&long_outcome).unwrap_err();
        assert!(err.contains("outcome"), "{err}");
    }

    #[test]
    fn a_checkbox_option_outcome_is_ignored_not_refused() {
        let req = validate_request(&json!({ "mode": "checkbox", "checkbox": {
            "prompt": "Which?",
            "options": [{ "id": "a", "label": "A", "outcome": "o".repeat(OUTCOME_MAX + 1) }]
        }}))
        .unwrap();
        let value = serde_json::to_value(&req).unwrap();
        assert!(value["checkbox"]["options"][0].get("outcome").is_none());
    }

    #[test]
    fn a_fenced_block_with_a_scenario_and_outcomes_parses() {
        let text = format!("Need a call.\n```{FENCE_LANG}\n{}\n```", decision_request());
        let value = extract_fenced(&text).unwrap().unwrap();
        let req = validate_request(&value).unwrap();
        assert!(req.questions[0].scenario.is_some());
        assert!(req.questions[0].options[0].outcome.is_some());
    }

    #[test]
    fn the_tool_spec_lists_scenario_and_outcome_for_questions_only() {
        let spec = tool_spec();
        let q = &spec["inputSchema"]["properties"]["questions"]["items"]["properties"];
        assert_eq!(q["scenario"]["type"], json!("string"));
        assert_eq!(
            q["options"]["items"]["properties"]["outcome"]["type"],
            json!("string")
        );
        let checkbox = &spec["inputSchema"]["properties"]["checkbox"]["properties"]["options"];
        assert!(checkbox["items"]["properties"].get("outcome").is_none());
    }

    #[test]
    fn the_tool_spec_is_read_only_and_names_the_tool() {
        let spec = tool_spec();
        assert_eq!(spec["name"], json!(TOOL));
        assert_eq!(spec["annotations"]["readOnlyHint"], json!(true));
        assert_eq!(spec["annotations"]["destructiveHint"], json!(false));
        assert_eq!(spec["inputSchema"]["type"], json!("object"));
        assert!(spec["description"].as_str().unwrap().contains("COO"));
    }
}
