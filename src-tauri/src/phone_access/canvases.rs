//! v1.2 (2026-11-07, overnight lane I1): canvases on the phone
//! (`docs/phone-api/canvases.md`, fixtures in
//! `docs/phone-api-fixtures/canvases/`).
//!
//! - **The kit** ([`kit`]): what every canvas document on the phone shares,
//!   built once from the Mac's own frame parts: the Canvas mode's policy
//!   ([`crate::artifacts::canvas_policy`]), the `--app-*` theme tokens light
//!   and dark (`THEME_TOKENS`' names, stock `src/styles/tokens.css` values),
//!   the artifact styles, the kit, the host runtime (`sandbox-doc.js`) and
//!   the ask kit, byte for byte. It holds no chat, canvas or vault data.
//! - **Reads** come from the canvas store (`workspace_canvases`), in the
//!   store the Mac's chat uses for that chat (the workspace whose folder
//!   holds the chat's folder, else the chat's own app-data store, as
//!   `chat::save_reply_canvases` picks it), and from the chat's transcript.
//!   Nothing here writes a file.
//! - **An ask-canvas's answer** ([`answer`]) is the window's Send under it:
//!   the text `canvasAnswerText` (`src/canvas/askAnswer.ts`) writes, sent as
//!   the person's composer send ([`routes::send_once`]), after the checks the
//!   window's Send makes (the current revision of an ask-canvas this chat
//!   hasn't answered).

use super::routes::{self, blocking, live_chat, ApiError, Ctx, Reply};
use super::store::Device;
use crate::next_step::CANVAS_ANSWER_LANG;
use crate::workspace_canvases::{self, Canvas, Scope};
use hyper::body::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, OnceLock};

const KIT_JS: &str = include_str!("../../../src/canvas/kit/generated/kit.min.js");
const KIT_CSS: &str = include_str!("../../../src/canvas/kit/generated/kit.min.css");
const ASK_JS: &str = include_str!("../../../src/canvas/kit/generated/kit-ask.min.js");
const ASK_CSS: &str = include_str!("../../../src/canvas/kit/generated/kit-ask.min.css");
const RUNTIME_JS: &str = include_str!("../../../src/components/sandbox-doc.js");
const ARTIFACT_STYLES_TS: &str = include_str!("../../../src/canvas/artifactStyles.ts");
const KIT_MANIFEST_TS: &str = include_str!("../../../src/canvas/kit/manifest.ts");
const TOKENS_CSS: &str = include_str!("../../../src/styles/tokens.css");

/// The `--app-*` tokens a frame sees, the `src/styles/tokens.css` token each
/// reads, and the frame's fallback: `THEME_TOKENS` in
/// `src/components/htmlSandbox.ts` (a test holds the two together).
pub(crate) const THEME_TOKENS: [(&str, &str, &str); 46] = [
    ("--app-ink", "--ink", "#1a1a1a"),
    ("--app-canvas", "--canvas", "#ffffff"),
    ("--app-surface", "--surface-card", "#ffffff"),
    ("--app-primary", "--primary", "#b5251c"),
    ("--app-accent", "--accent-teal", "#5f6b7a"),
    ("--app-muted", "--muted", "#626262"),
    ("--app-hairline", "--hairline", "#d5d6d8"),
    (
        "--app-mono",
        "--mono",
        "ui-monospace, SFMono-Regular, monospace",
    ),
    ("--app-display", "--display", "system-ui, sans-serif"),
    ("--app-surface-soft", "--surface-soft", "#f3f3f4"),
    ("--app-surface-strong", "--surface-cream-strong", "#e8e8e9"),
    ("--app-surface-raised", "--surface-raised", "#ffffff"),
    ("--app-hairline-soft", "--hairline-soft", "#e8e8e9"),
    ("--app-body", "--body", "#454545"),
    ("--app-body-strong", "--body-strong", "#393839"),
    ("--app-muted-soft", "--muted-soft", "#666666"),
    ("--app-primary-active", "--primary-active", "#dd3227"),
    ("--app-primary-disabled", "--primary-disabled", "#d6c7c5"),
    ("--app-on-primary", "--on-primary", "#ffffff"),
    ("--app-on-danger", "--on-error", "#ffffff"),
    (
        "--app-primary-soft",
        "--primary-soft",
        "rgba(221, 50, 39, .06)",
    ),
    (
        "--app-primary-soft-strong",
        "--primary-soft-strong",
        "rgba(221, 50, 39, .12)",
    ),
    ("--app-info", "--accent-teal", "#5f6b7a"),
    ("--app-info-soft", "--teal-soft", "rgba(95, 107, 122, 0.12)"),
    ("--app-success", "--success", "#3d7634"),
    (
        "--app-success-soft",
        "--success-soft",
        "rgba(74, 143, 63, 0.14)",
    ),
    ("--app-warning", "--warning", "#896213"),
    (
        "--app-warning-soft",
        "--amber-soft",
        "rgba(184, 132, 26, 0.14)",
    ),
    ("--app-danger", "--error", "#b73d3d"),
    (
        "--app-danger-soft",
        "--error-soft",
        "rgba(183, 61, 61, 0.12)",
    ),
    ("--app-risk", "--risk", "#855699"),
    ("--app-risk-soft", "--risk-soft", "rgba(138, 90, 158, 0.14)"),
    ("--app-syn-keyword", "--syn-keyword", "#b5251c"),
    ("--app-syn-string", "--syn-string", "#3b7556"),
    ("--app-syn-comment", "--syn-comment", "#666666"),
    ("--app-syn-number", "--syn-number", "#805320"),
    ("--app-syn-function", "--syn-function", "#3a64b0"),
    ("--app-syn-type", "--syn-type", "#2f6f8f"),
    ("--app-syn-builtin", "--syn-builtin", "#805320"),
    ("--app-syn-operator", "--syn-operator", "#626262"),
    ("--app-syn-punct", "--syn-punct", "#666666"),
    ("--app-syn-property", "--syn-property", "#3a64b0"),
    ("--app-syn-tag", "--syn-tag", "#b5251c"),
    ("--app-syn-attr", "--syn-attr", "#2f6f8f"),
    ("--app-syn-meta", "--syn-meta", "#805320"),
    ("--app-text-scale", "--text-scale", "1"),
];

/// A chat's canvas list holds at most this many entries (the newest).
pub(crate) const CHAT_LIST_MAX: usize = 200;
/// A summary the kit drafts, at most (UTF-16 units, `LIMITS.draftSummaryChars`).
pub(crate) const SUMMARY_MAX: usize = 500;
/// The sent lines `answered` carries, at most (`LIMITS.answeredChars`).
pub(crate) const ANSWERED_MAX: usize = 2000;

// ── Wire ─────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CanvasKit {
    pub kit: String,
    pub sha256: String,
    pub head: String,
    pub ask_head: String,
    pub themes: Themes,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Themes {
    pub light: BTreeMap<String, String>,
    pub dark: BTreeMap<String, String>,
}

/// An ask-canvas revision this chat answered: the revision and the lines the
/// person sent.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Answered {
    pub revision: u64,
    pub summary: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatCanvas {
    pub turn_id: String,
    pub index: u32,
    pub id: String,
    pub title: String,
    pub ask: bool,
    pub height: Option<u32>,
    pub revision: Option<u64>,
    pub latest_revision: Option<u64>,
    pub updated_at: Option<String>,
    pub answered: Option<Answered>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatCanvases {
    pub kit_sha256: String,
    pub canvases: Vec<ChatCanvas>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CanvasDetail {
    pub id: String,
    pub title: String,
    pub revision: u64,
    pub latest_revision: u64,
    pub ask: bool,
    pub height: Option<u32>,
    pub kit: Option<String>,
    pub updated_at: String,
    pub source: String,
    pub body: String,
    pub state: Value,
    pub answered: Option<Answered>,
    pub kit_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceCanvas {
    pub id: String,
    pub title: String,
    pub revision: u64,
    pub updated_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceCanvases {
    pub kit_sha256: String,
    pub canvases: Vec<WorkspaceCanvas>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnswerRequest {
    #[serde(default)]
    pub revision: Option<Value>,
    #[serde(default)]
    pub answers: Option<Value>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub client_message_id: Option<String>,
}

// ── The kit ──────────────────────────────────────────────────────────────

/// The quoted text after `marker` up to `end`, in a TypeScript source.
fn ts_constant<'a>(source: &'a str, marker: &str, end: &str) -> &'a str {
    let Some(at) = source.find(marker) else {
        return "";
    };
    let rest = &source[at + marker.len()..];
    rest.find(end).map_or("", |stop| &rest[..stop])
}

/// `ARTIFACT_STYLES` in `src/canvas/artifactStyles.ts`.
fn artifact_styles() -> &'static str {
    ts_constant(ARTIFACT_STYLES_TS, "export const ARTIFACT_STYLES = `", "`;")
}

/// `KIT_VERSION` in `src/canvas/kit/manifest.ts` (`kit@1`).
pub(crate) fn kit_version() -> &'static str {
    ts_constant(KIT_MANIFEST_TS, "export const KIT_VERSION = '", "'")
}

/// `css` without its `/* … */` comments.
fn without_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(open) = rest.find("/*") {
        out.push_str(&rest[..open]);
        match rest[open + 2..].find("*/") {
            Some(close) => rest = &rest[open + 2 + close + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// The custom properties declared in every top-level block of `css` whose
/// selector line is exactly `selector {`, later blocks over earlier ones.
pub(super) fn declarations(css: &str, selector: &str) -> HashMap<String, String> {
    let css = without_comments(css);
    let open = format!("{selector} {{");
    let mut out = HashMap::new();
    let mut from = 0;
    while let Some(found) = css[from..].find(&open) {
        let at = from + found;
        from = at + open.len();
        // The selector starts its line.
        if at > 0 && css.as_bytes()[at - 1] != b'\n' {
            continue;
        }
        let rest = &css[from..];
        // A block of declarations ends at its first `}`, on its own line or
        // on the selector's (tokens.css has one-line blocks).
        let end = rest.find('}').unwrap_or(rest.len());
        for decl in rest[..end].split(';') {
            let Some((name, value)) = decl.split_once(':') else {
                continue;
            };
            let name = name.trim();
            if name.starts_with("--") {
                out.insert(name.to_string(), value.trim().to_string());
            }
        }
    }
    out
}

/// A token's value with `var()` references resolved in `theme`, then in
/// `light`; `None` when it can't be.
fn resolved(
    value: &str,
    theme: &HashMap<String, String>,
    light: &HashMap<String, String>,
    depth: u8,
) -> Option<String> {
    let v = value.trim();
    if let Some(inner) = v.strip_prefix("var(").and_then(|s| s.strip_suffix(')')) {
        let (name, fallback) = match inner.split_once(',') {
            Some((name, fallback)) => (name.trim(), Some(fallback.trim())),
            None => (inner.trim(), None),
        };
        if depth >= 4 {
            return None;
        }
        return match theme.get(name).or_else(|| light.get(name)) {
            Some(found) => resolved(found, theme, light, depth + 1),
            None => fallback.and_then(|f| resolved(f, theme, light, depth + 1)),
        };
    }
    (!v.is_empty() && !v.contains("var(")).then(|| v.to_string())
}

/// A value the host runtime's `theme` rule takes (`UNSAFE_VALUE` in
/// `sandbox-doc.js`): 1 to 400 characters, none of `;{}<>\` or controls.
fn safe_theme_value(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= 400
        && !value
            .chars()
            .any(|c| matches!(c, ';' | '{' | '}' | '<' | '>' | '\\') || c.is_control())
}

/// The stock light and dark values of [`THEME_TOKENS`].
fn themes() -> Themes {
    let light_css = declarations(TOKENS_CSS, ":root");
    let dark_css = declarations(TOKENS_CSS, ":root[data-theme=\"dark\"]");
    let mut light = BTreeMap::new();
    let mut dark = BTreeMap::new();
    for (name, token, fallback) in THEME_TOKENS {
        let l = light_css
            .get(token)
            .and_then(|v| resolved(v, &light_css, &light_css, 0))
            .filter(|v| safe_theme_value(v))
            .unwrap_or_else(|| fallback.to_string());
        let d = dark_css
            .get(token)
            .and_then(|v| resolved(v, &dark_css, &light_css, 0))
            .filter(|v| safe_theme_value(v))
            .unwrap_or_else(|| l.clone());
        light.insert(name.to_string(), l);
        dark.insert(name.to_string(), d);
    }
    Themes { light, dark }
}

fn css_vars(theme: &BTreeMap<String, String>) -> String {
    theme.iter().map(|(k, v)| format!("{k}:{v};")).collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn build_kit() -> CanvasKit {
    let themes = themes();
    let head = format!(
        "<meta charset=\"utf-8\">\n<meta http-equiv=\"Content-Security-Policy\" content=\"{policy}\">\n<meta http-equiv=\"x-dns-prefetch-control\" content=\"off\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<base target=\"_blank\">\n<style>:root{{color-scheme:light dark;{light}}}\n@media (prefers-color-scheme: dark){{:root{{{dark}}}}}\n{styles}</style>\n<style>{kit_css}</style>\n<script>{runtime}</script>\n<script>{kit_js}</script>",
        policy = crate::artifacts::canvas_policy(),
        light = css_vars(&themes.light),
        dark = css_vars(&themes.dark),
        styles = artifact_styles(),
        kit_css = KIT_CSS,
        runtime = RUNTIME_JS,
        kit_js = KIT_JS,
    );
    let ask_head = format!("<style>{ASK_CSS}</style>\n<script>{ASK_JS}</script>");
    let kit = kit_version().to_string();
    let mut digest = Sha256::new();
    for part in [
        kit.as_str(),
        head.as_str(),
        ask_head.as_str(),
        &serde_json::to_string(&themes).unwrap_or_default(),
    ] {
        digest.update(part.as_bytes());
        digest.update([0u8]);
    }
    CanvasKit {
        kit,
        sha256: hex(&digest.finalize()),
        head,
        ask_head,
        themes,
    }
}

/// The kit, built once.
pub(crate) fn kit() -> &'static CanvasKit {
    static KIT: OnceLock<CanvasKit> = OnceLock::new();
    KIT.get_or_init(build_kit)
}

/// `GET /v1/canvas-kit`: the kit's JSON, serialized once.
pub(crate) fn kit_reply() -> Reply {
    static BODY: OnceLock<Bytes> = OnceLock::new();
    let body = BODY
        .get_or_init(|| Bytes::from(serde_json::to_vec(kit()).unwrap_or_default()))
        .clone();
    Reply::Json { status: 200, body }
}

// ── Frame body ───────────────────────────────────────────────────────────

/// `canvasFrameBody` (`src/canvas/fence.ts`): the root's inner HTML in the
/// texture root the frame's styles and kit look for. `ascii` when the root
/// asks for it, else `dither` (a legacy `plain` root is dithered too).
pub(crate) fn frame_body(source: &str, inner: &str) -> String {
    let texture = match crate::canvas_fence::root_texture(source) {
        Some(t) if t.trim().eq_ignore_ascii_case("ascii") => "ascii",
        _ => "dither",
    };
    format!(
        "<div data-hc-texture-root texture=\"{texture}\" style=\"display:contents\">{inner}</div>"
    )
}

// ── Stores and answers ───────────────────────────────────────────────────

/// The chat's canvas store: the workspace whose folder holds its folder,
/// else its own app-data store (`chat::save_reply_canvases`).
fn chat_workspace(vault: &Path, cwd: &str) -> Option<String> {
    crate::workspaces::workspace_for_cwd(vault, Path::new(cwd))
        .ok()
        .flatten()
        .map(|w| w.id)
}

fn app_data() -> Result<std::path::PathBuf, ApiError> {
    crate::agent_settings::app_data_dir().map_err(ApiError::internal)
}

/// `text` without a leading `<hollow-chat-intent>` block, the hidden answer
/// blocks and the `Answered canvas "<id>":` header, trimmed and cut at
/// [`ANSWERED_MAX`] characters: the lines the person sees they sent
/// (`answeredCanvases` in `src/canvas/askAnswer.ts`).
pub(crate) fn sent_lines(text: &str) -> String {
    let mut text = text;
    if let Some(rest) = text.strip_prefix("<hollow-chat-intent>") {
        if let Some(end) = rest.find("</hollow-chat-intent>") {
            text = rest[end + "</hollow-chat-intent>".len()..].trim_start();
        }
    }
    let visible = crate::next_step::split(text, &[CANVAS_ANSWER_LANG]).0;
    let mut rest = visible.as_str();
    if let Some(after) = rest.strip_prefix("Answered canvas \"") {
        if let Some(close) = after.find("\":") {
            let id = &after[..close];
            if crate::canvas_fence::valid_canvas_id(id) {
                rest = after[close + 2..].trim_start_matches([' ', '\t']);
                rest = rest.strip_prefix('\n').unwrap_or(rest);
            }
        }
    }
    rest.trim().chars().take(ANSWERED_MAX).collect()
}

/// Every ask-canvas revision chat `key` answered, with the lines it sent:
/// the first answer to a revision is the one that locked it. The chat's
/// messages, then one still waiting in the run queue to start (its row's
/// `detail` is the person's text): the window's lock counts a message once
/// the queue took it, and a withdrawn one never. A store that can't be read
/// is an error (503 `unavailable`) for the answer route and the lists alike:
/// read as "nothing answered" it would offer a Send that can't go (review
/// of PR #1185).
fn answered_reads(vault: &Path, key: &str) -> Result<HashMap<(String, u64), String>, String> {
    let mut out = HashMap::new();
    let mut read = |text: &str| {
        for answer in crate::canvas_answers::answered(text) {
            out.entry(answer).or_insert_with(|| sent_lines(text));
        }
    };
    if let Some(asks) = crate::session_store::session_asks(vault, key, CANVAS_ANSWER_LANG)? {
        for text in &asks.asked {
            read(text);
        }
    }
    for row in crate::run_queue::live_rows(vault)? {
        if row.source == crate::run_queue::RunSource::Chat && row.source_id.as_deref() == Some(key)
        {
            if let Some(text) = row.detail.as_deref() {
                read(text);
            }
        }
    }
    Ok(out)
}

/// One canvas answer at a time, from its checks until its send is queued,
/// so two answers to one canvas can't both pass "not answered" (review of
/// PR #1185).
pub(super) fn answering() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn answered_for(
    answered: &HashMap<(String, u64), String>,
    id: &str,
    revision: Option<u64>,
) -> Option<Answered> {
    let revision = revision?;
    answered
        .get(&(id.to_string(), revision))
        .map(|summary| Answered {
            revision,
            summary: summary.clone(),
        })
}

fn sha256_hex(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

/// A `revision` query parameter: `None` without one; 400 unless a whole
/// number from 1.
fn revision_param(query: &str) -> Result<Option<u64>, ApiError> {
    match routes::query_param(query, "revision") {
        None => Ok(None),
        Some(raw) => match raw.trim().parse::<u64>() {
            Ok(n) if n >= 1 => Ok(Some(n)),
            _ => Err(ApiError::bad_request(
                "revision must be a whole number from 1.",
            )),
        },
    }
}

fn check_canvas_id(id: &str) -> Result<(), ApiError> {
    if crate::canvas_fence::valid_canvas_id(id) {
        Ok(())
    } else {
        Err(ApiError::bad_request(
            "A canvas id is 1 to 64 lowercase letters, digits or hyphens.",
        ))
    }
}

fn canvas_missing() -> ApiError {
    ApiError::not_found("That canvas isn't saved in Hollow.")
}

/// One saved canvas's revision `revision` (else its current one) as the
/// phone reads it.
fn detail_of(
    vault: &Path,
    app: &Path,
    scope: Scope,
    mut canvas: Canvas,
    revision: Option<u64>,
    answered: &HashMap<(String, u64), String>,
) -> Result<CanvasDetail, ApiError> {
    let (revision, source) = match revision {
        Some(n) if n != canvas.revision => {
            let kept = workspace_canvases::read_revision(vault, app, scope, &canvas.id, n)
                .map_err(|_| {
                    ApiError::not_found("That version of the canvas is no longer kept.")
                })?;
            (n, kept.html)
        }
        _ => (canvas.revision, std::mem::take(&mut canvas.html)),
    };
    let valid = crate::canvas_fence::validate(&source)
        .map_err(|e| ApiError::internal(format!("A saved canvas doesn't parse: {e}")))?;
    let body = frame_body(&source, valid.inner);
    let title = valid.root.title.clone();
    let (ask, height) = (valid.root.ask, valid.root.height);
    let answered = if ask {
        answered_for(answered, &canvas.id, Some(revision))
    } else {
        None
    };
    let state = match std::mem::take(&mut canvas.state) {
        Value::Object(map) => Value::Object(map),
        _ => Value::Null,
    };
    Ok(CanvasDetail {
        id: canvas.id,
        title,
        revision,
        latest_revision: canvas.revision,
        ask,
        height,
        kit: canvas.kit,
        updated_at: canvas.updated_at,
        source,
        body,
        state,
        answered,
        kit_sha256: kit().sha256.clone(),
    })
}

// ── Routes ───────────────────────────────────────────────────────────────

/// `GET /v1/sessions/{key}/canvases`.
pub(crate) async fn chat_list(ctx: &Arc<Ctx>, key: &str) -> Result<Reply, ApiError> {
    let vault = ctx.vault.clone();
    let key = key.to_string();
    let list = blocking(move || {
        let summary = live_chat(&vault, &key)?;
        let app = app_data()?;
        let workspace = chat_workspace(&vault, &summary.cwd);
        let scope = Scope {
            workspace_id: workspace.as_deref(),
            session_key: Some(&key),
        };
        let conversation =
            crate::session_store::read_conversation(&vault, &key).map_err(ApiError::internal)?;
        let answered = answered_reads(&vault, &key).map_err(ApiError::unavailable)?;
        let mut saved: HashMap<String, Option<Canvas>> = HashMap::new();
        let mut canvases = Vec::new();
        for turn in conversation.turns.iter().filter(|t| t.role == "assistant") {
            let mut seen: HashMap<String, u32> = HashMap::new();
            for body in crate::canvas_fence::reply_canvas_bodies(&turn.text) {
                let Ok(valid) = crate::canvas_fence::validate(&body) else {
                    continue;
                };
                let root = valid.root;
                let index = seen.entry(root.id.clone()).or_insert(0);
                let this = *index;
                *index += 1;
                let canvas = saved
                    .entry(root.id.clone())
                    .or_insert_with(|| {
                        workspace_canvases::read(&vault, &app, scope, &root.id)
                            .ok()
                            .flatten()
                    })
                    .as_ref();
                let sha = sha256_hex(&body);
                let revision = canvas.and_then(|c| {
                    c.revisions
                        .iter()
                        .find(|r| r.fence_sha256 == sha)
                        .map(|r| r.revision)
                });
                canvases.push(ChatCanvas {
                    turn_id: turn.id.clone(),
                    index: this,
                    answered: if root.ask {
                        answered_for(&answered, &root.id, revision)
                    } else {
                        None
                    },
                    id: root.id,
                    title: root.title,
                    ask: root.ask,
                    height: root.height,
                    revision,
                    latest_revision: canvas.map(|c| c.revision),
                    updated_at: canvas.map(|c| c.updated_at.clone()),
                });
            }
        }
        if canvases.len() > CHAT_LIST_MAX {
            canvases.drain(..canvases.len() - CHAT_LIST_MAX);
        }
        Ok(ChatCanvases {
            kit_sha256: kit().sha256.clone(),
            canvases,
        })
    })
    .await?;
    Ok(routes::json(200, &list))
}

/// `GET /v1/sessions/{key}/canvases/{id}`.
pub(crate) async fn chat_read(
    ctx: &Arc<Ctx>,
    key: &str,
    id: &str,
    query: &str,
) -> Result<Reply, ApiError> {
    check_canvas_id(id)?;
    let revision = revision_param(query)?;
    let vault = ctx.vault.clone();
    let key = key.to_string();
    let id = id.to_string();
    let detail = blocking(move || {
        let summary = live_chat(&vault, &key)?;
        let app = app_data()?;
        let workspace = chat_workspace(&vault, &summary.cwd);
        let scope = Scope {
            workspace_id: workspace.as_deref(),
            session_key: Some(&key),
        };
        let canvas = workspace_canvases::read(&vault, &app, scope, &id)
            .map_err(ApiError::internal)?
            .ok_or_else(canvas_missing)?;
        let answered = answered_reads(&vault, &key).map_err(ApiError::unavailable)?;
        detail_of(&vault, &app, scope, canvas, revision, &answered)
    })
    .await?;
    Ok(routes::json(200, &detail))
}

/// A workspace id as `workspaces::folder` takes it.
fn check_workspace_id(id: &str) -> Result<(), ApiError> {
    let ok = !id.is_empty()
        && id.len() <= 120
        && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-');
    if ok {
        Ok(())
    } else {
        Err(ApiError::not_found("That workspace isn't in Hollow."))
    }
}

/// The workspace's store, 404 when there is no such workspace.
fn workspace_scope(vault: &Path, id: &str) -> Result<(), ApiError> {
    check_workspace_id(id)?;
    crate::workspaces::summary(vault, id)
        .map(|_| ())
        .map_err(|_| ApiError::not_found("That workspace isn't in Hollow."))
}

/// `GET /v1/workspaces/{id}/canvases`.
pub(crate) async fn workspace_list(ctx: &Arc<Ctx>, id: &str) -> Result<Reply, ApiError> {
    let vault = ctx.vault.clone();
    let id = id.to_string();
    let list = blocking(move || {
        workspace_scope(&vault, &id)?;
        let app = app_data()?;
        let scope = Scope {
            workspace_id: Some(&id),
            session_key: None,
        };
        let canvases = workspace_canvases::list(&vault, &app, scope)
            .map_err(ApiError::internal)?
            .into_iter()
            // Only readable canvases under ids the read route takes.
            .filter(|c| c.error.is_none() && crate::canvas_fence::valid_canvas_id(&c.id))
            .map(|c| WorkspaceCanvas {
                id: c.id,
                title: c.title,
                revision: c.revision,
                updated_at: c.updated_at,
            })
            .collect();
        Ok(WorkspaceCanvases {
            kit_sha256: kit().sha256.clone(),
            canvases,
        })
    })
    .await?;
    Ok(routes::json(200, &list))
}

/// `GET /v1/workspaces/{id}/canvases/{canvasId}`.
pub(crate) async fn workspace_read(
    ctx: &Arc<Ctx>,
    id: &str,
    canvas_id: &str,
    query: &str,
) -> Result<Reply, ApiError> {
    check_canvas_id(canvas_id)?;
    let revision = revision_param(query)?;
    let vault = ctx.vault.clone();
    let id = id.to_string();
    let canvas_id = canvas_id.to_string();
    let detail = blocking(move || {
        workspace_scope(&vault, &id)?;
        let app = app_data()?;
        let scope = Scope {
            workspace_id: Some(&id),
            session_key: None,
        };
        let canvas = workspace_canvases::read(&vault, &app, scope, &canvas_id)
            .map_err(ApiError::internal)?
            .ok_or_else(canvas_missing)?;
        // Answers belong to a chat: none here.
        detail_of(&vault, &app, scope, canvas, revision, &HashMap::new())
    })
    .await?;
    Ok(routes::json(200, &detail))
}

/// `canvasAnswerText` (`src/canvas/askAnswer.ts`): the header, one line per
/// summary line (`answerLine` is `push_line`), then the answers as one line
/// of JSON in a hidden block. `answers`' keys come out in Hollow's JSON
/// order (sorted).
pub(crate) fn answer_text(id: &str, revision: u64, answers: &Value, summary: &str) -> String {
    let mut text = format!("Answered canvas \"{id}\":");
    for line in summary.split('\n').filter(|l| !l.trim().is_empty()) {
        match line.find(": ") {
            Some(at) if at > 0 => {
                crate::interaction_schema::push_line(&mut text, &line[..at], &line[at + 2..])
            }
            _ => {
                text.push_str("\n- ");
                text.push_str(&crate::interaction_schema::one_line(line));
            }
        }
    }
    let json = format!(
        "{{\"canvasId\":{},\"revision\":{revision},\"answers\":{}}}",
        serde_json::to_string(id).unwrap_or_default(),
        serde_json::to_string(answers).unwrap_or_default()
    );
    format!("{text}\n\n```{CANVAS_ANSWER_LANG}\n{json}\n```")
}

/// A draft's summary as the bridge takes it (`parseChildMessage`'s `draft`):
/// not blank, at most [`SUMMARY_MAX`] UTF-16 units, no control characters
/// but newlines and tabs.
fn summary_ok(summary: &str) -> bool {
    !summary.trim().is_empty()
        && summary.encode_utf16().count() <= SUMMARY_MAX
        && !summary
            .chars()
            .any(|c| matches!(c, '\u{0}'..='\u{8}' | '\u{b}'..='\u{1f}' | '\u{7f}'))
}

/// `POST /v1/sessions/{key}/canvases/{id}/answer`.
pub(crate) async fn answer(
    ctx: &Arc<Ctx>,
    device: &Device,
    key: &str,
    id: &str,
    body: &[u8],
) -> Result<Reply, ApiError> {
    check_canvas_id(id)?;
    let request: AnswerRequest = routes::parse_body(body)?;
    let revision = request
        .revision
        .as_ref()
        .and_then(Value::as_u64)
        .filter(|n| *n >= 1)
        .ok_or_else(|| ApiError::bad_request("revision must be a whole number from 1."))?;
    let answers = request
        .answers
        .filter(|a| a.is_object() && crate::canvas_answers::answers(a))
        .ok_or_else(|| ApiError::bad_request("answers isn't what the canvas's form sends."))?;
    let summary = request.summary.filter(|s| summary_ok(s)).ok_or_else(|| {
        ApiError::bad_request("summary must be 1 to 500 characters of the answers' lines.")
    })?;
    let message = routes::message_id(request.client_message_id.as_deref())?;
    // A retry after the answer went gets that answer, not "already answered".
    if let Some(reply) = routes::remembered(ctx, &device.device_id, &message).await {
        return Ok(reply);
    }
    // Held from the checks until the send is queued, even when the phone
    // hangs up (the guard moves into the send's own task).
    let held = answering().lock().await;
    // Again under the lock: a retry that overlapped the first request waited
    // here while that one sent, and gets its answer, not "already answered".
    if let Some(reply) = routes::remembered(ctx, &device.device_id, &message).await {
        return Ok(reply);
    }
    let text = {
        let vault = ctx.vault.clone();
        let key = key.to_string();
        let id = id.to_string();
        blocking(move || {
            let chat = live_chat(&vault, &key)?;
            let app = app_data()?;
            let workspace = chat_workspace(&vault, &chat.cwd);
            let scope = Scope {
                workspace_id: workspace.as_deref(),
                session_key: Some(&key),
            };
            let canvas = workspace_canvases::read(&vault, &app, scope, &id)
                .map_err(ApiError::internal)?
                .ok_or_else(canvas_missing)?;
            if !canvas.ask {
                return Err(ApiError::canvas_closed(
                    "This canvas has no questions to answer.",
                ));
            }
            if canvas.revision != revision {
                return Err(ApiError::canvas_closed(
                    "A newer version of this canvas replaced it. Answer the newest one.",
                ));
            }
            // Fails closed: an unreadable store refuses rather than reading
            // as "not answered".
            let answered = answered_reads(&vault, &key).map_err(ApiError::unavailable)?;
            if answered.contains_key(&(id.clone(), revision)) {
                return Err(ApiError::canvas_closed("This canvas was already answered."));
            }
            Ok(answer_text(&id, revision, &answers, &summary))
        })
        .await?
    };
    let send = routes::send_once(
        ctx.clone(),
        device.device_id.clone(),
        key.to_string(),
        text,
        Vec::new(),
        message,
        super::controls::Selection::default(),
    );
    routes::to_the_end(async move {
        let _held = held;
        send.await
    })
    .await
}
