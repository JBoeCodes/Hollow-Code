//! What a resumed chat turn resends of its stable layers (agent
//! effectiveness F6, wave 3 · G2).
//!
//! Before G2 every turn carried the workspace envelope in full (its
//! documents and the attached-source excerpts, rebuilt from disk), and
//! Codex, Antigravity and Grok also carried Hollow's configured half (the
//! appended prompt) at the top of every prompt, although an exactly resumed
//! native session already holds both from earlier turns.
//!
//! **The baseline.** After a turn's prompt reached its agent, the
//! conversation's `derived_state` row of kind [`KIND`] records what that
//! native session now holds: the native id, the appended prompt's SHA-256
//! when it went in the prompt, the envelope preamble's SHA-256, each
//! envelope value as given (the workspace documents' text), each attached
//! source's entry hash and the excerpts given (path, lines, SHA-256), each
//! per-turn layer's SHA-256 (S02), and the resumed turns since the last
//! full send. It is written only once the
//! agent took the prompt (`chat::stable_layers_holder`): a finished reply
//! (a success, or denied actions only), under the id this turn's own stream
//! announced, never the resumed id the stream starts with. A prompt that
//! leaned on what a session holds (a delta, the held configured half) is
//! recorded only under that same id. A turn that failed, was stopped or
//! announced nothing leaves the previous baseline, which the session still
//! holds, so the next turn resends what the failed one carried.
//!
//! **A resumed turn** ([`holds`]) sends deltas when the plan resumes the
//! exact native session the baseline names and fewer than
//! [`REFRESH_TURNS`] resumed turns have passed since the last full send:
//! - the envelope as an update ([`envelope_delta`]): an unchanged document
//!   is a one-line marker, a changed one its changed lines (`+`/`-`) or,
//!   when that isn't smaller, its new text; an attached source whose entry
//!   is unchanged and that has no excerpt for this request is counted, not
//!   listed; an excerpt the session already has is named by its line range
//!   (`givenEarlier`), not resent. A changed preamble (a cut, a build note,
//!   the goal block) sends the whole envelope;
//! - for Codex, Antigravity and Grok, the configured half as one line
//!   ([`crate::agent::CONFIGURED_HELD`]); the workspace policy and
//!   permission paragraph still go on every turn. Claude's configured half
//!   is its system prompt, given to every process, so it is unchanged;
//! - the per-turn layers ([`TURN_LAYERS`], S02): approved recall, the
//!   composer's `<hollow-chat-intent>`, the `<hollow-skills>` index and the
//!   `<hollow-mode>` line are left out when the baseline records the same
//!   text's SHA-256 ([`layer_given`]); a changed one (a permission flip's
//!   mode line, new recall) is sent. The agent still has the earlier text in
//!   its history, and the appended prompt says the latest mode line applies.
//!   A turn with no fitting skill whose session holds an earlier block
//!   ([`holds_skills`]) sends an empty `<hollow-skills>` block once (A15).
//!   Each of them used to go on every turn, and every copy stayed in the
//!   session's history, cache-read on every later call.
//!
//! Anything else (a new, reset or retried session, another native id, a
//! session from before G2, the refresh) gets the full layers and a fresh
//! baseline. The refresh bounds what a provider's own context compaction
//! may have summarized away: Hollow can't see when Codex, Antigravity or
//! Grok compacts, so it resends everything every [`REFRESH_TURNS`]th
//! resumed turn rather than claim the session still holds it.
//!
//! **Re-grounding** (repo features R1, [`reground`]): a compaction can also
//! summarize away the user's always/never rules and the steps already done,
//! which only a fresh session's `<task-state>` record carries. So the next
//! resumed turn after a compaction the stream reported
//! ([`Baseline::compacted_turn`], any agent), and the refresh turn of every
//! agent but Claude (whose compaction Hollow sees), carry that record again.

use crate::run_manifest::sha256_hex;
use crate::workspaces::EnvelopeParts;
use serde_json::{Map, Value};
use std::path::Path;

/// The `derived_state` kind of a conversation's baseline.
pub(crate) const KIND: &str = "stableLayers";
const SCHEMA: u32 = 1;
/// A resumed session gets its stable layers in full again after this many
/// resumed turns of deltas.
pub(crate) const REFRESH_TURNS: u32 = 8;
/// Excerpts remembered as given, newest kept.
const MAX_EXCERPTS: usize = 400;

/// An unchanged workspace document's value in a delta.
pub(crate) const UNCHANGED: &str = "[Hollow: unchanged since given earlier in this conversation.]";
/// A value given earlier that the envelope no longer has.
const GONE: &str = "[Hollow: no longer in the workspace context.]";
/// The first line of a changed document's changed lines.
const CHANGED: &str = "[Hollow: changed since given earlier in this conversation. Lines marked + are new, lines marked - are gone; the rest is as given.]";
/// The preamble of an envelope delta.
pub(crate) const DELTA_PREAMBLE: &str = "Workspace context update. This conversation already holds the workspace context Hollow sent earlier, and its rules still apply. A document marked unchanged is as you were given it; a changed one carries its changed lines (+ new, - gone) or its new text. attachedSourceEvidence lists the sources with excerpts for this request: new excerpts in full, the ones you were already given by their line range (givenEarlier); unchangedSourceFiles counts the others. Read the workspace files when you need more.";

/// What a conversation's native session holds of its stable layers.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Baseline {
    schema: u32,
    /// The native session id.
    pub native: String,
    /// The appended prompt's SHA-256 when the prompt carried it (Codex,
    /// Antigravity, Grok); empty otherwise.
    pub configured: String,
    /// The envelope preamble's SHA-256 (a quick chat: its fixed text's);
    /// empty without an envelope.
    pub envelope: String,
    /// The envelope's values as given: a document's text, or another
    /// value's JSON. `attachedSourceEvidence` is kept as `sources` and
    /// `excerpts`.
    pub values: Vec<(String, String)>,
    /// Each attached source's entry (without its excerpts), hashed.
    pub sources: Vec<(String, String)>,
    /// Excerpts given: `path:from-to:sha256`.
    pub excerpts: Vec<String>,
    pub resumed_since_full: u32,
    /// The per-turn layers given (S02, [`TURN_LAYERS`]): each one's name and
    /// the SHA-256 of its text as the latest turn carried it (or left it
    /// out, the session holding it). A baseline from before S02 has none,
    /// so its next turn sends them all.
    #[serde(default)]
    pub layers: Vec<(String, String)>,
    /// The turn whose agent reported compacting this session's history
    /// (repo features R1); 0 for none. Set by the compacted turn itself
    /// ([`unsettled`], and its finished write), cleared by the next turn
    /// that finishes (its `pending` has 0), so a stopped or failed
    /// re-grounding turn leaves it and the next one re-grounds again.
    #[serde(default)]
    pub compacted_turn: i64,
}

/// Why a resumed turn carries the `<task-state>` record again (repo
/// features R1), which otherwise only a fresh session gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reground {
    /// The agent reported compacting this session's history during that
    /// turn (any agent whose stream says `compact_boundary`).
    Compacted(i64),
    /// The refresh (the resumed turn after [`REFRESH_TURNS`] resumed turns
    /// of deltas, [`holds`]) of an agent whose compaction
    /// Hollow can't see.
    Refresh,
}

impl Reground {
    /// The manifest's `compiler.taskState.reground`.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Reground::Compacted(_) => "compacted",
            Reground::Refresh => "refresh",
        }
    }
}

/// Whether Hollow sees `agent` compact its own history: only Claude's
/// stream is relied on for `compact_boundary`. Grok shares Claude's parser,
/// so one it reports still counts ([`reground`]), but nothing shows its CLI
/// emitting it, so it keeps the refresh.
pub(crate) fn sees_compaction(agent: crate::agent::AgentKind) -> bool {
    agent == crate::agent::AgentKind::Claude
}

/// Whether a resumed turn of native session `native` re-grounds the agent
/// with the task-state record (repo features R1): `given` is the stored
/// baseline (`None` on a turn that doesn't resume), `held` what [`holds`]
/// said of it. A compaction the stored baseline records for this same
/// session wins; otherwise the refresh, for an agent whose compaction
/// Hollow can't see ([`sees_compaction`]).
pub(crate) fn reground(
    given: Option<&Baseline>,
    native: Option<&str>,
    held: &Result<&Baseline, &'static str>,
    agent: crate::agent::AgentKind,
) -> Option<Reground> {
    if let Some(given) = given {
        if given.compacted_turn > 0
            && !given.native.is_empty()
            && native == Some(given.native.as_str())
        {
            return Some(Reground::Compacted(given.compacted_turn));
        }
    }
    (matches!(held, Err("refresh")) && !sees_compaction(agent)).then_some(Reground::Refresh)
}

/// The conversation's baseline, if one is stored in this schema.
pub(crate) fn read(vault: &Path, key: &str) -> Option<(i64, Baseline)> {
    let row = crate::session_store::derived_state(vault, key, KIND)
        .ok()
        .flatten()?;
    let baseline = serde_json::from_str::<Baseline>(&row.payload)
        .ok()
        .filter(|b| b.schema == SCHEMA)?;
    Some((row.revision, baseline))
}

/// Store `baseline` as what turn `ord`'s native session holds.
pub(crate) fn write(vault: &Path, key: &str, ord: i64, baseline: &Baseline) {
    let revision = read(vault, key).map_or(0, |(revision, _)| revision) + 1;
    let payload = serde_json::to_string(baseline).unwrap_or_default();
    if let Err(error) = crate::session_store::put_derived_state(
        vault,
        key,
        KIND,
        &crate::session_store::DerivedRow {
            revision,
            through_ord: ord,
            payload,
        },
    ) {
        eprintln!("[chat] the stable-layer baseline could not be stored: {error}");
    }
}

/// The baseline a resumed turn of native session `native` may send deltas
/// against, or why not: `noBaseline`, `otherSession` or `refresh`.
pub(crate) fn holds<'a>(
    baseline: Option<&'a Baseline>,
    native: Option<&str>,
) -> Result<&'a Baseline, &'static str> {
    let baseline = baseline.ok_or("noBaseline")?;
    if baseline.native.is_empty() || native != Some(baseline.native.as_str()) {
        return Err("otherSession");
    }
    if baseline.resumed_since_full >= REFRESH_TURNS {
        return Err("refresh");
    }
    Ok(baseline)
}

/// The per-turn layers a held resumed turn leaves out when its session was
/// given them as is (S02), by their manifest names: approved recall, the
/// composer's chat intent, the skill index and the `<hollow-mode>` line.
pub(crate) const TURN_LAYERS: [&str; 4] = ["recall", "chatIntent", "skillIndex", "permissionMode"];

/// Whether the session `given` holds layer `name` exactly as `text`: then a
/// held resumed turn leaves it out. An empty layer is never "given".
pub(crate) fn layer_given(given: &Baseline, name: &str, text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let sha = sha256_hex(text.as_bytes());
    given.layers.iter().any(|(n, s)| n == name && *s == sha)
}

/// Whether native session `native` was given a `<hollow-skills>` block
/// (overnight A15), held or due for the refresh: an earlier turn's picks
/// stay in its history until a later block replaces them.
pub(crate) fn holds_skills(given: Option<&Baseline>, native: Option<&str>) -> bool {
    given.is_some_and(|b| {
        !b.native.is_empty()
            && native == Some(b.native.as_str())
            && b.layers.iter().any(|(name, _)| name == "skillIndex")
    })
}

/// After forgetting per-turn layers (an unsettled turn, a compaction), keep
/// a `skillIndex` entry with no SHA-256 when the session held a block
/// (`had`; A15 review): an earlier block may still be the latest there, so
/// [`holds_skills`] stays true, and an empty SHA matches no text, so
/// [`layer_given`] never leaves a block out against it.
fn keep_skills_unknown(layers: &mut Vec<(String, String)>, had: bool) {
    if had && !layers.iter().any(|(name, _)| name == "skillIndex") {
        layers.push(("skillIndex".to_string(), String::new()));
    }
}

/// Forget every per-turn layer as given after a compaction (R1), keeping
/// that a skills block may linger in the summary (A15 review).
pub(crate) fn forget_layers(baseline: &mut Baseline) {
    let had = baseline.layers.iter().any(|(name, _)| name == "skillIndex");
    baseline.layers.clear();
    keep_skills_unknown(&mut baseline.layers, had);
}

/// Record the per-turn `layers` (name, text) the session holds after this
/// turn: each non-empty one's SHA-256, whether this turn sent it or left it
/// out as given. An empty one is dropped, so it is sent again when it comes
/// back.
pub(crate) fn record_layers(baseline: &mut Baseline, layers: &[(&str, &str)]) {
    baseline.layers = layers
        .iter()
        .filter(|(_, text)| !text.is_empty())
        .map(|(name, text)| (name.to_string(), sha256_hex(text.as_bytes())))
        .collect();
}

/// The baseline `stored` once a turn whose agent ran reached the session it
/// names (`reached`: the ids the turn announced or resumed), before the
/// turn's outcome is known (S02 review). Its message may or may not be in
/// that session's history now, and a per-turn layer it carried in another
/// text (a flipped mode line) may be the latest there, so only the layers
/// this turn's `pending` record holds identically stay given; when
/// `compacted` (the agent's own compaction ran), none does. The envelope,
/// excerpts and refresh count stay as they were. A compaction also marks
/// the session compacted during turn `ord` (R1), so the next resumed turn
/// re-grounds it whatever ends this one. A turn that then finishes
/// writes `pending` over it; one that doesn't (a stop, an API error, a
/// stall) leaves this. `None` when nothing changes.
pub(crate) fn unsettled(
    stored: &Baseline,
    pending: &Baseline,
    reached: &[Option<&str>],
    ord: i64,
    compacted: bool,
) -> Option<Baseline> {
    if stored.native.is_empty() || !reached.contains(&Some(stored.native.as_str())) {
        return None;
    }
    let mut kept = stored.clone();
    let held = |layer: &(String, String)| !compacted && pending.layers.contains(layer);
    let skills = holds_skills(Some(stored), Some(stored.native.as_str()));
    kept.layers.retain(held);
    keep_skills_unknown(&mut kept.layers, skills);
    if compacted {
        kept.compacted_turn = ord;
    }
    (kept != *stored).then_some(kept)
}

/// [`unsettled`], on the conversation's stored baseline, written as turn
/// `ord`'s when it changes.
pub(crate) fn unsettle(
    vault: &Path,
    key: &str,
    ord: i64,
    pending: &Baseline,
    reached: &[Option<&str>],
    compacted: bool,
) {
    let stored = read(vault, key).map(|(_, stored)| stored);
    let kept = match &stored {
        Some(stored) => unsettled(stored, pending, reached, ord, compacted),
        None => None,
    };
    let kept = kept.or_else(|| compacted_mark(stored.as_ref(), reached, ord, compacted));
    if let Some(kept) = kept {
        write(vault, key, ord, &kept);
    }
}

/// The baseline of a compacting turn whose session no stored baseline names
/// (R1 review): a session's first turn, or an attempt on a new native id,
/// stopped or failed before a finished turn recorded it. It names the
/// session the turn reached first (the id its stream announced), holds
/// nothing else and marks the compaction, so the next resumed turn of that
/// session re-grounds the agent ([`reground`]). Holding no envelope,
/// configured half or per-turn layer, it lets that turn send everything in
/// full. `None` without a compaction, without a reached id, or when the
/// stored baseline names a reached session ([`unsettled`]'s case).
pub(crate) fn compacted_mark(
    stored: Option<&Baseline>,
    reached: &[Option<&str>],
    ord: i64,
    compacted: bool,
) -> Option<Baseline> {
    if !compacted {
        return None;
    }
    if let Some(stored) = stored {
        if !stored.native.is_empty() && reached.contains(&Some(stored.native.as_str())) {
            return None;
        }
    }
    let native = reached.iter().flatten().find(|id| !id.is_empty())?;
    Some(Baseline {
        schema: SCHEMA,
        native: native.to_string(),
        compacted_turn: ord,
        ..Default::default()
    })
}

/// An envelope as a turn sends it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Envelope {
    pub text: String,
    pub unchanged: u32,
    pub changed: u32,
    pub referenced: u32,
}

fn excerpt_key(path: &str, excerpt: &Value) -> String {
    format!(
        "{path}:{}-{}:{}",
        excerpt["from"],
        excerpt["to"],
        sha256_hex(excerpt["text"].as_str().unwrap_or("").as_bytes())
    )
}

/// An attached source's entry without its excerpts, hashed.
fn entry_sha(entry: &Value) -> String {
    let mut entry = entry.clone();
    if let Some(object) = entry.as_object_mut() {
        object.remove("excerpts");
    }
    sha256_hex(entry.to_string().as_bytes())
}

fn sources_of(parts: &EnvelopeParts) -> &[Value] {
    parts
        .json
        .get("attachedSourceEvidence")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn path_of(entry: &Value) -> &str {
    entry["path"].as_str().unwrap_or("")
}

/// A value as the baseline keeps it: a string's text, else its JSON.
fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Keys that are not values: the sources and the omitted count.
fn is_value_key(key: &str) -> bool {
    key != "attachedSourceEvidence" && key != "omittedSourceFiles"
}

/// The non-blank lines of `text`, trimmed.
fn lines_of(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

/// `from`'s lines that `to` doesn't hold as often, in `from`'s order.
fn missing<'a>(from: &[&'a str], to: &[&str]) -> Vec<&'a str> {
    let mut left: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for line in to {
        *left.entry(line).or_default() += 1;
    }
    from.iter()
        .filter(|line| match left.get_mut(*line) {
            Some(count) if *count > 0 => {
                *count -= 1;
                false
            }
            _ => true,
        })
        .copied()
        .collect()
}

/// A changed document: its changed lines when that is less than half its
/// new text, else the new text.
fn changed_value(old: &str, new: &str) -> String {
    let (old_lines, new_lines) = (lines_of(old), lines_of(new));
    let mut text = format!("{CHANGED}\n");
    for line in missing(&old_lines, &new_lines) {
        text.push_str(&format!("- {line}\n"));
    }
    for line in missing(&new_lines, &old_lines) {
        text.push_str(&format!("+ {line}\n"));
    }
    if text.len() * 2 < new.len() {
        text
    } else {
        new.to_string()
    }
}

/// The envelope a held resumed turn sends instead of `full`, against what
/// the session was `given`; `None` when only the whole envelope will do
/// (its preamble changed). A quick chat's fixed envelope, unchanged, is
/// sent as nothing.
pub(crate) fn envelope_delta(
    full: &str,
    parts: Option<&EnvelopeParts>,
    given: &Baseline,
) -> Option<Envelope> {
    let Some(parts) = parts else {
        return (sha256_hex(full.as_bytes()) == given.envelope).then(Envelope::default);
    };
    if sha256_hex(parts.preamble.as_bytes()) != given.envelope {
        return None;
    }
    let given_value = |key: &str| {
        given
            .values
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    let mut out = Map::new();
    let mut delta = Envelope::default();
    for (key, value) in &parts.json {
        if key == "attachedSourceEvidence" {
            let mut listed = Vec::new();
            let mut quiet = 0u32;
            for entry in value.as_array().map_or(&[][..], Vec::as_slice) {
                let path = path_of(entry);
                let excerpts = entry["excerpts"].as_array().map_or(&[][..], Vec::as_slice);
                let same = given
                    .sources
                    .iter()
                    .any(|(p, sha)| p == path && *sha == entry_sha(entry));
                if same && excerpts.is_empty() {
                    quiet += 1;
                    continue;
                }
                let (had, new): (Vec<&Value>, Vec<&Value>) = excerpts
                    .iter()
                    .partition(|e| given.excerpts.contains(&excerpt_key(path, e)));
                let mut object = Map::new();
                if same {
                    for field in ["path", "coverage", "note"] {
                        if let Some(v) = entry.get(field) {
                            object.insert(field.into(), v.clone());
                        }
                    }
                } else if let Some(whole) = entry.as_object() {
                    object = whole.clone();
                    object.remove("excerpts");
                }
                if !new.is_empty() {
                    object.insert(
                        "excerpts".into(),
                        Value::Array(new.into_iter().cloned().collect()),
                    );
                }
                if !had.is_empty() {
                    delta.referenced += had.len() as u32;
                    object.insert(
                        "givenEarlier".into(),
                        Value::Array(
                            had.iter()
                                .map(|e| Value::String(format!("{}-{}", e["from"], e["to"])))
                                .collect(),
                        ),
                    );
                }
                listed.push(Value::Object(object));
            }
            out.insert("attachedSourceEvidence".into(), Value::Array(listed));
            if quiet > 0 {
                out.insert("unchangedSourceFiles".into(), quiet.into());
            }
            continue;
        }
        if !is_value_key(key) {
            out.insert(key.clone(), value.clone());
            continue;
        }
        let now = value_text(value);
        match given_value(key) {
            Some(old) if old == now => {
                delta.unchanged += 1;
                out.insert(key.clone(), UNCHANGED.into());
            }
            Some(old) if value.is_string() => {
                delta.changed += 1;
                out.insert(key.clone(), changed_value(old, &now).into());
            }
            _ => {
                delta.changed += 1;
                out.insert(key.clone(), value.clone());
            }
        }
    }
    for (key, _) in &given.values {
        if !parts.json.contains_key(key) {
            delta.changed += 1;
            out.insert(key.clone(), GONE.into());
        }
    }
    delta.text = crate::workspaces::render_envelope(DELTA_PREAMBLE, &out);
    Some(delta)
}

/// What a native session holds after a turn that sent the envelope `full`
/// (with `parts`) in full and, when `configured` isn't empty, the appended
/// prompt with that SHA-256 in the prompt.
pub(crate) fn full_baseline(
    native: &str,
    configured: &str,
    full: Option<&str>,
    parts: Option<&EnvelopeParts>,
) -> Baseline {
    let mut baseline = Baseline {
        schema: SCHEMA,
        native: native.to_string(),
        configured: configured.to_string(),
        ..Default::default()
    };
    record_envelope(&mut baseline, full, parts);
    baseline
}

/// `given` after a resumed turn that sent deltas of the envelope `full`.
pub(crate) fn advanced(
    given: &Baseline,
    full: Option<&str>,
    parts: Option<&EnvelopeParts>,
) -> Baseline {
    let mut next = Baseline {
        schema: SCHEMA,
        native: given.native.clone(),
        configured: given.configured.clone(),
        excerpts: given.excerpts.clone(),
        resumed_since_full: given.resumed_since_full + 1,
        ..Default::default()
    };
    record_envelope(&mut next, full, parts);
    next
}

/// Record the envelope's preamble, values, sources and (adding to what is
/// there) excerpts.
fn record_envelope(baseline: &mut Baseline, full: Option<&str>, parts: Option<&EnvelopeParts>) {
    match (full, parts) {
        (_, Some(parts)) => {
            baseline.envelope = sha256_hex(parts.preamble.as_bytes());
            baseline.values = parts
                .json
                .iter()
                .filter(|(key, _)| is_value_key(key))
                .map(|(key, value)| (key.clone(), value_text(value)))
                .collect();
            for entry in sources_of(parts) {
                let path = path_of(entry);
                baseline.sources.push((path.to_string(), entry_sha(entry)));
                for excerpt in entry["excerpts"].as_array().map_or(&[][..], Vec::as_slice) {
                    let key = excerpt_key(path, excerpt);
                    if !baseline.excerpts.contains(&key) {
                        baseline.excerpts.push(key);
                    }
                }
            }
            let excess = baseline.excerpts.len().saturating_sub(MAX_EXCERPTS);
            baseline.excerpts.drain(..excess);
        }
        (Some(text), None) => baseline.envelope = sha256_hex(text.as_bytes()),
        (None, None) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parts(plan: &str, excerpts: Value) -> EnvelopeParts {
        let mut json = Map::new();
        json.insert("WORKSPACE.md".into(), "# Brief\n\nOutcome.".into());
        json.insert("PLAN.md".into(), plan.into());
        json.insert(
            "attachedSourceEvidence".into(),
            json!([
                { "path": "sources/a.md", "coverage": "excerpt", "note": "excerpt", "includedBytes": 10, "excerpts": excerpts },
                { "path": "sources/b.md", "coverage": "not_selected", "note": "present", "includedBytes": 0 }
            ]),
        );
        EnvelopeParts {
            preamble: "Work in the current workspace.".into(),
            json,
        }
    }

    fn plan(n: usize) -> String {
        (1..=n)
            .map(|i| format!("- [ ] Step {i:03} of the plan, with enough words to matter.\n"))
            .collect()
    }

    #[test]
    fn an_unchanged_envelope_is_markers_and_ranges() {
        let first = parts(&plan(40), json!([{ "from": 1, "to": 40, "text": "alpha" }]));
        let given = full_baseline("native-1", "", Some(&first.render()), Some(&first));
        let again = parts(
            &plan(40),
            json!([{ "from": 1, "to": 40, "text": "alpha" }, { "from": 80, "to": 90, "text": "beta" }]),
        );
        let delta = envelope_delta(&again.render(), Some(&again), &given).unwrap();
        assert!(delta
            .text
            .starts_with(&format!("<hollow-workspace>\n{DELTA_PREAMBLE}\n")));
        assert!(!delta.text.contains("Step 001"), "{}", delta.text);
        assert_eq!(
            (delta.unchanged, delta.changed, delta.referenced),
            (2, 0, 1)
        );
        assert!(
            delta.text.contains(r#""givenEarlier":["1-40"]"#),
            "{}",
            delta.text
        );
        assert!(delta.text.contains("beta") && !delta.text.contains("alpha"));
        assert!(delta.text.contains(r#""unchangedSourceFiles":1"#));
        assert!(delta.text.len() < again.render().len());
    }

    #[test]
    fn a_changed_document_carries_its_changed_lines() {
        let first = parts(&plan(40), json!([]));
        let given = full_baseline("native-1", "", Some(&first.render()), Some(&first));
        let edited =
            plan(40).replace("- [ ] Step 007", "- [x] Step 007") + "- [ ] Step 041 is new.\n";
        let next = parts(&edited, json!([]));
        let delta = envelope_delta(&next.render(), Some(&next), &given).unwrap();
        assert_eq!(delta.changed, 1);
        assert!(delta.text.contains("- - [ ] Step 007") && delta.text.contains("+ - [x] Step 007"));
        assert!(delta.text.contains("+ - [ ] Step 041 is new."));
        assert!(!delta.text.contains("Step 008"));
        // A rewrite is sent whole.
        let rewritten = parts("Something else entirely.", json!([]));
        let delta = envelope_delta(&rewritten.render(), Some(&rewritten), &given).unwrap();
        assert!(delta.text.contains("Something else entirely."));
    }

    #[test]
    fn a_new_preamble_or_another_session_or_the_refresh_sends_everything() {
        let first = parts(&plan(4), json!([]));
        let given = full_baseline("native-1", "abc", Some(&first.render()), Some(&first));
        let mut cut = first.clone();
        cut.preamble
            .push_str(" Some workspace content did not fit.");
        assert!(envelope_delta(&cut.render(), Some(&cut), &given).is_none());
        assert_eq!(holds(None, Some("native-1")), Err("noBaseline"));
        assert_eq!(holds(Some(&given), Some("native-2")), Err("otherSession"));
        assert_eq!(holds(Some(&given), None), Err("otherSession"));
        assert!(holds(Some(&given), Some("native-1")).is_ok());
        let mut aged = given.clone();
        for _ in 0..REFRESH_TURNS {
            aged = advanced(&aged, Some(&first.render()), Some(&first));
        }
        assert_eq!(aged.configured, "abc");
        assert_eq!(holds(Some(&aged), Some("native-1")), Err("refresh"));
    }

    #[test]
    fn a_quick_chat_envelope_is_sent_once() {
        let quick = crate::workspaces::QUICK_CHAT_ENVELOPE;
        let given = full_baseline("n", "", Some(quick), None);
        assert_eq!(
            envelope_delta(quick, None, &given),
            Some(Envelope::default())
        );
        assert_eq!(
            envelope_delta("<hollow-workspace>other</hollow-workspace>", None, &given),
            None
        );
    }

    /// S02: a per-turn layer is given only as the exact text recorded (an
    /// empty one never), and a baseline stored before S02, with no
    /// `layers`, still reads, holding none.
    #[test]
    fn a_per_turn_layer_is_given_only_as_recorded() {
        let mut given = full_baseline("native-1", "", None, None);
        record_layers(
            &mut given,
            &[("recall", "R"), ("chatIntent", ""), ("mode", "M")],
        );
        assert!(layer_given(&given, "recall", "R"));
        assert!(!layer_given(&given, "recall", "R, changed"));
        assert!(!layer_given(&given, "mode", "R"));
        assert!(!layer_given(&given, "chatIntent", ""));
        assert_eq!(given.layers.len(), 2);
        let mut old = serde_json::to_value(&given).unwrap();
        old.as_object_mut().unwrap().remove("layers");
        let read: Baseline = serde_json::from_value(old).unwrap();
        assert!(read.layers.is_empty());
        assert_eq!(read.native, "native-1");
    }

    /// S02 review (B1, M1): a turn that reached the stored session keeps
    /// only the layers it holds identically (none after a compaction);
    /// one on another session changes nothing.
    #[test]
    fn an_unsettled_turn_keeps_only_identical_layers() {
        let mut stored = full_baseline("native-1", "", None, None);
        record_layers(&mut stored, &[("chatIntent", "I"), ("mode", "accept")]);
        let mut pending = full_baseline("", "", None, None);
        record_layers(&mut pending, &[("chatIntent", "I"), ("mode", "bypass")]);
        let reached = [Some("native-1"), None];
        let kept = unsettled(&stored, &pending, &reached, 4, false).unwrap();
        assert!(layer_given(&kept, "chatIntent", "I"));
        assert!(!layer_given(&kept, "mode", "accept"));
        assert_eq!(kept.native, "native-1");
        assert_eq!(kept.compacted_turn, 0);
        let compacted = unsettled(&stored, &stored, &reached, 4, true).unwrap();
        assert!(compacted.layers.is_empty());
        // R1: the compaction is recorded with its turn, even with no layer
        // to drop, and kept by a later turn that doesn't finish.
        assert_eq!(compacted.compacted_turn, 4);
        let bare = full_baseline("native-1", "", None, None);
        let marked = unsettled(&bare, &bare, &reached, 4, true).unwrap();
        assert_eq!(marked.compacted_turn, 4);
        assert_eq!(unsettled(&marked, &marked, &reached, 5, false), None);
        assert_eq!(unsettled(&stored, &stored, &reached, 4, false), None);
        let other = [Some("native-2")];
        assert_eq!(unsettled(&stored, &pending, &other, 4, false), None);
    }

    /// R1 review: a compacting turn whose session no stored baseline names
    /// (a first turn, or a new native id, stopped before a finished turn
    /// recorded it) leaves a baseline naming the announced session with
    /// the mark and nothing else, so its next resumed turn re-grounds the
    /// agent and still sends everything in full: no envelope, quick-chat
    /// envelope, configured half or per-turn layer counts as held.
    #[test]
    fn a_compaction_on_an_unrecorded_session_is_marked() {
        use crate::agent::AgentKind::Codex;
        let reached = [Some("native-2"), Some("native-1")];
        let marked = compacted_mark(None, &reached, 1, true).unwrap();
        assert_eq!(
            (marked.native.as_str(), marked.compacted_turn),
            ("native-2", 1)
        );
        assert_eq!(compacted_mark(None, &reached, 1, false), None);
        assert_eq!(compacted_mark(None, &[None], 1, true), None);
        let stored = full_baseline("native-9", "", None, None);
        let other = compacted_mark(Some(&stored), &reached, 3, true).unwrap();
        assert_eq!(
            (other.native.as_str(), other.compacted_turn),
            ("native-2", 3)
        );
        let named = full_baseline("native-1", "", None, None);
        assert_eq!(compacted_mark(Some(&named), &reached, 3, true), None);

        let native = Some("native-2");
        let held = holds(Some(&marked), native);
        assert_eq!(
            reground(Some(&marked), native, &held, Codex),
            Some(Reground::Compacted(1))
        );
        let given = held.unwrap();
        let first = parts(&plan(4), json!([]));
        assert_eq!(envelope_delta(&first.render(), Some(&first), given), None);
        let quick = crate::workspaces::QUICK_CHAT_ENVELOPE;
        assert_eq!(envelope_delta(quick, None, given), None);
        assert!(!layer_given(
            given,
            "permissionMode",
            "<hollow-mode>x</hollow-mode>"
        ));
        assert!(given.configured.is_empty() && given.resumed_since_full == 0);
    }

    /// R1: a compaction the stored baseline records re-grounds the next
    /// resumed turn of that same session, for any agent (Grok's stream is
    /// Claude's format); the refresh re-grounds every agent but Claude,
    /// whose compaction Hollow sees.
    #[test]
    fn reground_follows_the_compaction_and_the_refresh() {
        use crate::agent::AgentKind::{Antigravity, Claude, Codex, Grok};
        let first = parts(&plan(4), json!([]));
        let mut aged = full_baseline("native-1", "", Some(&first.render()), Some(&first));
        let fresh = aged.clone();
        for _ in 0..REFRESH_TURNS {
            aged = advanced(&aged, Some(&first.render()), Some(&first));
        }
        let native = Some("native-1");
        let refresh = holds(Some(&aged), native);
        assert_eq!(refresh, Err("refresh"));
        assert_eq!(reground(Some(&aged), native, &refresh, Claude), None);
        for agent in [Codex, Antigravity, Grok] {
            assert_eq!(
                reground(Some(&aged), native, &refresh, agent),
                Some(Reground::Refresh),
                "{agent:?}"
            );
        }
        let held = holds(Some(&fresh), native);
        assert_eq!(reground(Some(&fresh), native, &held, Codex), None);
        let mut compacted = fresh.clone();
        compacted.compacted_turn = 3;
        let held = holds(Some(&compacted), native);
        for agent in [Grok, Claude] {
            assert_eq!(
                reground(Some(&compacted), native, &held, agent),
                Some(Reground::Compacted(3))
            );
        }
        // Another session, or a turn that doesn't resume, isn't re-grounded.
        let other = holds(Some(&compacted), Some("native-2"));
        assert_eq!(
            reground(Some(&compacted), Some("native-2"), &other, Grok),
            None
        );
        assert_eq!(reground(None, native, &holds(None, native), Grok), None);
        // A baseline stored before R1 reads as never compacted.
        let mut old = serde_json::to_value(&compacted).unwrap();
        old.as_object_mut().unwrap().remove("compacted_turn");
        let read: Baseline = serde_json::from_value(old).unwrap();
        assert_eq!(read.compacted_turn, 0);
    }

    #[test]
    fn the_baseline_round_trips_through_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let key = "s-20261007-g2000009";
        let first = parts(&plan(4), json!([{ "from": 1, "to": 4, "text": "x" }]));
        let given = full_baseline("native-1", "", Some(&first.render()), Some(&first));
        crate::session_store::start_turn(dir.path(), key, "codex", "/tmp", "run-1", "hi").unwrap();
        write(dir.path(), key, 1, &given);
        assert_eq!(read(dir.path(), key).map(|(_, b)| b), Some(given));
    }
}
