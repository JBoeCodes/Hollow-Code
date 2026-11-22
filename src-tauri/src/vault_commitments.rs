//! First-class, vault-native commitments with approval-gated extraction.
//!
//! Commitment notes live at stable `commitments/*.md` paths. Status changes
//! rewrite frontmatter in place; they never move or rename the note. Model
//! extraction is deliberately proposal-only: a tool-less one-shot may enqueue
//! one typed item in [`crate::review_queue`], and only an explicit acceptance
//! reaches [`create`].

use crate::frontmatter;
use crate::review_queue::{self, NewReviewItem, ReviewItem};
use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path};
use std::sync::atomic::{AtomicU64, Ordering};

const COMMITMENTS_DIR: &str = "commitments";
const MAX_TITLE_CHARS: usize = 500;
const MAX_COUNTERPARTY_CHARS: usize = 300;
const MAX_CONTEXT_CHARS: usize = 20_000;
const MAX_EVIDENCE_CHARS: usize = 1_000;

const DIRECTIONS: &[&str] = &["owed_by_me", "owed_to_me"];
/// A proposal's direction when its source doesn't say who owes whom (master
/// plan #2: never guessed). The person picks one before accepting.
pub(crate) const UNKNOWN_DIRECTION: &str = "unknown";
const STATUSES: &[&str] = &["open", "fulfilled", "cancelled"];
const CHANNELS: &[&str] = &["email", "teams", "slack", "meeting", "manual"];

static ID_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Commitment {
    pub url: String,
    pub vault_path: String,
    pub title: String,
    pub direction: String,
    pub counterparty: String,
    #[serde(default)]
    pub person_path: Option<String>,
    #[serde(default)]
    pub project_url: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
    pub status: String,
    pub channel: String,
    #[serde(default)]
    pub source_path: Option<String>,
    pub created: String,
    #[serde(default)]
    pub fulfilled_at: Option<String>,
    pub fingerprint: String,
    #[serde(default)]
    pub context: String,
    /// `YYYY-MM-DD`: an open commitment stays out of the ledger's open list,
    /// Today and the overdue nudge until that day (overnight 10-11 A5).
    /// Closing it clears this.
    #[serde(default)]
    pub snoozed_until: Option<String>,
    /// The words it rests on, quoted from its source (an accepted proposal's
    /// evidence, one line), so the ledger can cite them.
    #[serde(default)]
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateCommitmentInput {
    pub title: String,
    pub direction: String,
    pub counterparty: String,
    #[serde(default)]
    pub person_path: Option<String>,
    #[serde(default)]
    pub project_url: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub evidence: Option<String>,
}

/// `None` leaves a field unchanged. For optional scalars, `Some("")` clears
/// the field, matching the existing task/project update convention.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCommitmentInput {
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub direction: Option<String>,
    #[serde(default)]
    pub counterparty: Option<String>,
    #[serde(default)]
    pub person_path: Option<String>,
    #[serde(default)]
    pub project_url: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
    /// `YYYY-MM-DD` snoozes an open commitment until that day; `""` wakes it.
    #[serde(default)]
    pub snoozed_until: Option<String>,
    /// Restores a fulfilled commitment's original stamp (the ledger's undo of
    /// a reopen, overnight 10-11/12 A9). Used only when a commitment that
    /// wasn't fulfilled becomes fulfilled and the value is an RFC 3339 time
    /// not in the future; otherwise the usual stamp applies. Webview only: the
    /// agent action `commitments.update_commitment` deliberately doesn't list
    /// it, so agents can't backdate a completion.
    #[serde(default)]
    pub fulfilled_at: Option<String>,
}

/// Raw markdown plus its stable URL, sufficient for a lossless one-click undo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TrashedCommitment {
    pub url: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommitmentProposalPayload {
    pub title: String,
    pub direction: String,
    pub counterparty: String,
    #[serde(default)]
    pub person_path: Option<String>,
    #[serde(default)]
    pub project_url: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
    pub channel: String,
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub rationale: String,
    #[serde(default)]
    pub evidence_excerpt: String,
    #[serde(default)]
    pub source_run_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CommitmentProposal {
    pub id: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub fingerprint: String,
    pub payload: CommitmentProposalPayload,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
struct Draft {
    title: String,
    direction: String,
    counterparty: String,
    person_path: Option<String>,
    project_url: Option<String>,
    due: Option<String>,
    status: String,
    channel: String,
    source_path: Option<String>,
    context: String,
    fulfilled_at: Option<String>,
    snoozed_until: Option<String>,
    evidence: Option<String>,
}

/// List all valid first-level commitment notes. Malformed hand-authored files
/// are skipped rather than making the whole People/Today resource fail.
pub fn list(vault: &Path) -> Vec<Commitment> {
    let root = vault.join(COMMITMENTS_DIR);
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
            continue;
        };
        if name.starts_with('.') || path.extension().and_then(|v| v.to_str()) != Some("md") {
            continue;
        }
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        let rel = format!("{COMMITMENTS_DIR}/{name}");
        if let Some(commitment) = parse_commitment(vault, &rel, &raw) {
            out.push(commitment);
        }
    }
    out.sort_by(|a, b| {
        status_rank(&a.status)
            .cmp(&status_rank(&b.status))
            .then_with(|| option_sort_key(&a.due).cmp(option_sort_key(&b.due)))
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
            .then_with(|| a.url.cmp(&b.url))
    });
    out
}

/// Create a commitment at a stable flat path. A semantically identical
/// commitment returns the existing record even when it is fulfilled or
/// cancelled; terminal state is intentionally not permission to duplicate it.
pub fn create(vault: &Path, input: CreateCommitmentInput) -> Result<Commitment, String> {
    let mut draft = draft_from_create(input);
    normalize_draft(vault, &mut draft, true, true)?;
    apply_terminal_stamp(&mut draft, None);
    let fingerprint = draft_fingerprint(&draft);

    let guard = crate::vault::lock_mutate();
    if let Some(existing) = find_by_fingerprint(vault, &fingerprint, None) {
        return Ok(existing);
    }

    let created = Utc::now().to_rfc3339();
    let rel = pick_stable_path(vault, &draft.title, &fingerprint, &created)?;
    let commitment = commitment_from_draft(rel, created, draft);
    // Under the mutate lock already (the fingerprint scan and the publish
    // are one step), so the `_locked` writer; `Absent` publishes with an
    // exclusive link, so a file that appeared at the picked path is kept.
    let written = crate::vault::write_note_expecting_locked(
        &guard,
        vault,
        &commitment.vault_path,
        &render(&commitment),
        crate::vault::Expected::Absent,
    )?;
    if let crate::vault::CasOutcome::Conflict(_) = written {
        return Err(format!(
            "refusing to overwrite existing commitment: {}",
            commitment.vault_path
        ));
    }
    Ok(commitment)
}

/// Apply a validated edit without changing the commitment's URL or filename.
pub fn update(vault: &Path, input: UpdateCommitmentInput) -> Result<Commitment, String> {
    let rel = commitment_rel(vault, &input.url)?;
    let person_was_explicit = input.person_path.is_some();
    let counterparty_changed = input.counterparty.is_some();

    let note = crate::vault::mutate_note(vault, &rel, |raw| {
        let existing = parse_commitment(vault, &rel, raw)
            .ok_or_else(|| format!("invalid commitment note: {rel}"))?;
        let previous_status = existing.status.clone();
        let mut draft = draft_from_commitment(&existing);

        if let Some(value) = input.title.as_deref() {
            draft.title = value.to_string();
        }
        if let Some(value) = input.direction.as_deref() {
            draft.direction = value.to_string();
        }
        if let Some(value) = input.counterparty.as_deref() {
            draft.counterparty = value.to_string();
            if !person_was_explicit {
                // A prior link belongs to the old counterparty. Resolve the
                // replacement only on an exact unique name match.
                draft.person_path = None;
            }
        }
        if let Some(value) = input.person_path.as_deref() {
            draft.person_path = optional_owned(value);
        }
        if let Some(value) = input.project_url.as_deref() {
            draft.project_url = optional_owned(value);
        }
        if let Some(value) = input.due.as_deref() {
            draft.due = optional_owned(value);
        }
        if let Some(value) = input.status.as_deref() {
            draft.status = value.to_string();
        }
        if let Some(value) = input.channel.as_deref() {
            draft.channel = value.to_string();
        }
        if let Some(value) = input.source_path.as_deref() {
            draft.source_path = optional_owned(value);
        }
        if let Some(value) = input.context.as_deref() {
            draft.context = value.to_string();
        }
        if let Some(value) = input.snoozed_until.as_deref() {
            draft.snoozed_until = optional_owned(value);
        }

        normalize_draft(
            vault,
            &mut draft,
            counterparty_changed && !person_was_explicit,
            true,
        )?;
        apply_terminal_stamp(
            &mut draft,
            Some((&previous_status, existing.fulfilled_at.clone())),
        );
        // An undo of a reopen puts the original stamp back, not "now". Only
        // for a commitment that wasn't already fulfilled (an existing date is
        // never edited), and never a stamp in the future.
        if draft.status == "fulfilled" && previous_status != "fulfilled" {
            if let Some(stamp) = input
                .fulfilled_at
                .as_deref()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .map(|t| t.with_timezone(&Utc))
                .filter(|t| *t <= Utc::now())
            {
                draft.fulfilled_at = Some(stamp.to_rfc3339());
            }
        }
        let next_fingerprint = draft_fingerprint(&draft);
        if let Some(duplicate) = find_by_fingerprint(vault, &next_fingerprint, Some(&rel)) {
            return Err(format!(
                "duplicate commitment already exists: {}",
                duplicate.url
            ));
        }

        let next = commitment_from_draft(rel.clone(), existing.created, draft);
        Ok(render(&next))
    })?;

    parse_commitment(vault, &rel, &note.content)
        .ok_or_else(|| format!("updated commitment could not be parsed: {rel}"))
}

/// Whether an open commitment is snoozed past `today` (`YYYY-MM-DD`): it
/// wakes on its snooze day.
pub fn is_snoozed(commitment: &Commitment, today: &str) -> bool {
    commitment.status == "open"
        && commitment
            .snoozed_until
            .as_deref()
            .is_some_and(|until| until > today)
}

pub fn set_status(vault: &Path, url: &str, status: &str) -> Result<Commitment, String> {
    update(
        vault,
        UpdateCommitmentInput {
            url: url.to_string(),
            status: Some(status.to_string()),
            ..Default::default()
        },
    )
}

/// Move a commitment to the vault's recoverable trash and return the exact
/// markdown plus stable URL needed by [`restore`].
pub fn trash(vault: &Path, url: &str) -> Result<TrashedCommitment, String> {
    let rel = commitment_rel(vault, url)?;
    let _guard = crate::vault::lock_mutate();
    let full = crate::vault::safe_join(vault, &rel)?;
    let content = fs::read_to_string(&full).map_err(|e| format!("read commitment: {e}"))?;
    parse_commitment(vault, &rel, &content)
        .ok_or_else(|| format!("invalid commitment note: {rel}"))?;
    crate::vault::delete_note(vault, &rel)?;
    Ok(TrashedCommitment {
        url: format!("vault:{rel}"),
        content,
    })
}

/// Restore exact raw markdown to its original stable path. Existing files and
/// semantic duplicates are never overwritten.
pub fn restore(vault: &Path, trashed: TrashedCommitment) -> Result<Commitment, String> {
    let rel = commitment_rel(vault, &trashed.url)?;
    let restored = parse_commitment(vault, &rel, &trashed.content)
        .ok_or_else(|| "trashed content is not a valid commitment".to_string())?;
    validate_commitment_links(vault, &restored, false)?;

    let guard = crate::vault::lock_mutate();
    let full = crate::vault::safe_join(vault, &rel)?;
    if full.exists() {
        return Err(format!("refusing to overwrite existing commitment: {rel}"));
    }
    if let Some(duplicate) = find_by_fingerprint(vault, &restored.fingerprint, None) {
        return Err(format!(
            "duplicate commitment already exists: {}",
            duplicate.url
        ));
    }
    let written = crate::vault::write_note_expecting_locked(
        &guard,
        vault,
        &rel,
        &trashed.content,
        crate::vault::Expected::Absent,
    )?;
    if let crate::vault::CasOutcome::Conflict(_) = written {
        return Err(format!("refusing to overwrite existing commitment: {rel}"));
    }
    parse_commitment(vault, &rel, &trashed.content)
        .ok_or_else(|| format!("restored commitment could not be parsed: {rel}"))
}

pub fn list_proposals(vault: &Path) -> Result<Vec<CommitmentProposal>, String> {
    review_queue::list(vault, Some("commitment"))?
        .into_iter()
        .map(proposal_from_item)
        .collect()
}

/// Validate and enqueue a proposal. Queue fingerprints remain sticky across
/// pending/accepted/rejected states. A directly tracked commitment blocks a
/// new queue item with the same semantic fingerprint.
pub fn submit_proposal(
    vault: &Path,
    payload: CommitmentProposalPayload,
) -> Result<CommitmentProposal, String> {
    submit_proposal_from(vault, payload, "chat", None)
}

pub(crate) fn submit_proposal_from(
    vault: &Path,
    payload: CommitmentProposalPayload,
    source: &str,
    source_path: Option<String>,
) -> Result<CommitmentProposal, String> {
    let payload = normalize_proposal_payload(vault, payload)?;
    let fingerprint = proposal_fingerprint(&payload);

    if let Some(existing) = review_queue::list(vault, Some("commitment"))?
        .into_iter()
        .find(|item| item.fingerprint == fingerprint)
    {
        return proposal_from_item(existing);
    }
    if let Some(existing) = find_by_fingerprint(vault, &fingerprint, None) {
        return Err(format!("commitment is already tracked: {}", existing.url));
    }

    proposal_from_item(review_queue::enqueue(
        vault,
        NewReviewItem {
            kind: "commitment".into(),
            fingerprint,
            payload: serde_json::to_value(payload)
                .map_err(|e| format!("encode commitment proposal: {e}"))?,
            risk: "local_write".into(),
            source: source.into(),
            source_path,
        },
    )?)
}

/// What an extracted commitment's rationale starts with (F3).
pub(crate) const EXTRACTED_RATIONALE: &str = "Extracted from";

/// Queue an extracted commitment (master plan F3) as an ordinary proposal:
/// its direction from the person's point of view, the other party, the
/// anchor as its context. A promise whose owner the source doesn't make
/// clear is not queued as one (never guessed).
pub(crate) fn submit_extracted(
    vault: &Path,
    loaded: &crate::extract::Loaded,
    p: &crate::extract::Proposal,
    me: &str,
) -> Result<Result<(String, String), String>, String> {
    // Master plan #2: a promise whose direction the source leaves unclear is
    // queued as `unknown`; the person picks one before accepting.
    let named_other =
        |n: &Option<String>| n.as_deref().is_some_and(|n| !crate::extract::is_me(n, me));
    if named_other(&p.owner) && named_other(&p.counterparty) {
        return Ok(Err(
            "A promise between two other people isn't yours to track, so it wasn't queued.".into(),
        ));
    }
    let (direction, other) = crate::extract::direction(p, me);
    let direction = direction.unwrap_or(UNKNOWN_DIRECTION);
    let Some(counterparty) = other.filter(|o| !crate::extract::is_me(o, me)) else {
        return Ok(Err(
            "The source doesn't name the other party, so it wasn't queued.".into(),
        ));
    };
    let payload = CommitmentProposalPayload {
        title: p.text.clone(),
        direction: direction.into(),
        counterparty,
        person_path: None,
        project_url: None,
        due: p.due.clone(),
        channel: if loaded.queue_source == "meeting" {
            "meeting".into()
        } else {
            "manual".into()
        },
        source_path: loaded.source_path.clone(),
        context: format!("{} ({}): {}", p.source_title, p.at, p.anchor),
        rationale: format!("{EXTRACTED_RATIONALE} {} ({}).", p.source_title, p.at),
        evidence_excerpt: p.evidence_excerpt.clone(),
        source_run_id: None,
    };
    // What the proposal itself gets wrong is a refusal (reported, not
    // retried); a write that fails is an error (the source is read again).
    if let Err(reason) = normalize_proposal_payload(vault, payload.clone()) {
        return Ok(Err(reason));
    }
    let proposal = match submit_proposal_from(
        vault,
        payload,
        loaded.queue_source,
        loaded.source_path.clone(),
    ) {
        Ok(p) => p,
        Err(e) if e.starts_with("commitment is already tracked") => return Ok(Err(e)),
        Err(e) => return Err(e),
    };
    Ok(Ok(("commitment".into(), proposal.id)))
}

/// Commitments filtered by direction (`owed_by_me`, `owed_to_me`) and by a
/// person (their name in the counterparty, or their `people/` note),
/// case-insensitively. Master plan #2's "I owe" / "Owed to me".
pub fn filter(
    list: Vec<Commitment>,
    direction: Option<&str>,
    person: Option<&str>,
) -> Vec<Commitment> {
    let person = person
        .map(|p| p.trim().to_lowercase())
        .filter(|p| !p.is_empty());
    list.into_iter()
        .filter(|c| direction.is_none_or(|d| d.trim().is_empty() || c.direction == d.trim()))
        .filter(|c| {
            person.as_deref().is_none_or(|p| {
                c.counterparty.to_lowercase().contains(p)
                    || c.person_path
                        .as_deref()
                        .is_some_and(|path| path.to_lowercase().contains(p))
            })
        })
        .collect()
}

/// Reject directly, or claim an accepted payload and route it through normal
/// [`create`] validation/dedup before marking the queue item accepted.
pub fn resolve_proposal(
    vault: &Path,
    id: &str,
    action: &str,
    edited_payload: Option<CommitmentProposalPayload>,
) -> Result<CommitmentProposal, String> {
    match action.trim().to_lowercase().as_str() {
        "reject" => return proposal_from_item(review_queue::reject(vault, id)?),
        "accept" => {}
        other => return Err(format!("unknown commitment resolution: {other}")),
    }

    let existing = review_queue::list(vault, Some("commitment"))?
        .into_iter()
        .find(|item| item.id == id)
        .ok_or_else(|| format!("commitment proposal not found: {id}"))?;
    let payload = match edited_payload {
        Some(edited) => normalize_proposal_payload(vault, edited)?,
        None => normalize_proposal_payload(
            vault,
            serde_json::from_value(existing.payload.clone())
                .map_err(|e| format!("invalid commitment proposal payload: {e}"))?,
        )?,
    };
    if payload.direction == UNKNOWN_DIRECTION {
        return Err("Choose who owes whom first (I owe, or owed to me): the source didn't say, and Hollow doesn't guess.".into());
    }
    let approved =
        serde_json::to_value(&payload).map_err(|e| format!("encode commitment proposal: {e}"))?;
    review_queue::claim(vault, id, Some(approved))?;

    let create_result = create(vault, create_input_from_proposal(&payload));
    match create_result {
        Ok(_) => proposal_from_item(review_queue::accept(vault, id)?),
        Err(error) => {
            let _ = review_queue::fail(vault, id, &error);
            Err(error)
        }
    }
}

fn normalize_proposal_payload(
    vault: &Path,
    mut payload: CommitmentProposalPayload,
) -> Result<CommitmentProposalPayload, String> {
    let mut draft = Draft {
        title: payload.title,
        direction: payload.direction,
        counterparty: payload.counterparty,
        person_path: payload.person_path,
        project_url: payload.project_url,
        due: payload.due,
        status: "open".into(),
        channel: payload.channel,
        source_path: payload.source_path,
        context: payload.context,
        fulfilled_at: None,
        snoozed_until: None,
        evidence: None,
    };
    // An unknown direction stays unknown on a pending proposal; the rest of
    // the draft is checked as if it were either one.
    let unknown = draft.direction.trim() == UNKNOWN_DIRECTION;
    if unknown {
        draft.direction = DIRECTIONS[0].into();
    }
    normalize_draft(vault, &mut draft, true, true)?;
    if unknown {
        draft.direction = UNKNOWN_DIRECTION.into();
    }
    if draft.status != "open" {
        return Err("commitment proposals must create open commitments".into());
    }
    payload.title = draft.title;
    payload.direction = draft.direction;
    payload.counterparty = draft.counterparty;
    payload.person_path = draft.person_path;
    payload.project_url = draft.project_url;
    payload.due = draft.due;
    payload.channel = draft.channel;
    payload.source_path = draft.source_path;
    payload.context = draft.context;
    payload.rationale = cap_chars(payload.rationale.trim(), MAX_EVIDENCE_CHARS);
    payload.evidence_excerpt = cap_chars(payload.evidence_excerpt.trim(), MAX_EVIDENCE_CHARS);
    payload.source_run_id = payload
        .source_run_id
        .and_then(|value| optional_owned(&value));
    Ok(payload)
}

fn create_input_from_proposal(payload: &CommitmentProposalPayload) -> CreateCommitmentInput {
    CreateCommitmentInput {
        title: payload.title.clone(),
        direction: payload.direction.clone(),
        counterparty: payload.counterparty.clone(),
        person_path: payload.person_path.clone(),
        project_url: payload.project_url.clone(),
        due: payload.due.clone(),
        status: Some("open".into()),
        channel: Some(payload.channel.clone()),
        source_path: payload.source_path.clone(),
        context: payload.context.clone(),
        evidence: optional_owned(&payload.evidence_excerpt),
    }
}

fn proposal_from_item(item: ReviewItem) -> Result<CommitmentProposal, String> {
    if item.kind != "commitment" {
        return Err(format!(
            "review item {} is not a commitment proposal",
            item.id
        ));
    }
    let payload = serde_json::from_value(item.payload)
        .map_err(|e| format!("invalid commitment proposal {}: {e}", item.id))?;
    Ok(CommitmentProposal {
        id: item.id,
        status: item.status,
        created_at: item.created_at,
        updated_at: item.updated_at,
        fingerprint: item.fingerprint,
        payload,
        error: item.error,
    })
}

fn draft_from_create(input: CreateCommitmentInput) -> Draft {
    Draft {
        title: input.title,
        direction: input.direction,
        counterparty: input.counterparty,
        person_path: input.person_path,
        project_url: input.project_url,
        due: input.due,
        status: input.status.unwrap_or_else(|| "open".into()),
        channel: input.channel.unwrap_or_else(|| "manual".into()),
        source_path: input.source_path,
        context: input.context,
        fulfilled_at: None,
        snoozed_until: None,
        evidence: input.evidence,
    }
}

fn draft_from_commitment(commitment: &Commitment) -> Draft {
    Draft {
        title: commitment.title.clone(),
        direction: commitment.direction.clone(),
        counterparty: commitment.counterparty.clone(),
        person_path: commitment.person_path.clone(),
        project_url: commitment.project_url.clone(),
        due: commitment.due.clone(),
        status: commitment.status.clone(),
        channel: commitment.channel.clone(),
        source_path: commitment.source_path.clone(),
        context: commitment.context.clone(),
        fulfilled_at: commitment.fulfilled_at.clone(),
        snoozed_until: commitment.snoozed_until.clone(),
        evidence: commitment.evidence.clone(),
    }
}

fn normalize_draft(
    vault: &Path,
    draft: &mut Draft,
    resolve_person: bool,
    require_link_targets: bool,
) -> Result<(), String> {
    draft.title = required_text("title", &draft.title, MAX_TITLE_CHARS)?;
    draft.direction = canonical_choice("direction", &draft.direction, DIRECTIONS)?;
    draft.counterparty =
        required_text("counterparty", &draft.counterparty, MAX_COUNTERPARTY_CHARS)?;
    draft.status = canonical_choice("status", &draft.status, STATUSES)?;
    draft.channel = canonical_choice("channel", &draft.channel, CHANNELS)?;
    draft.due = canonical_due(draft.due.as_deref())?;
    draft.person_path = canonical_person_path(
        vault,
        draft.person_path.as_deref().unwrap_or(""),
        require_link_targets,
    )?;
    if draft.person_path.is_none() && resolve_person {
        draft.person_path = resolve_exact_person(vault, &draft.counterparty)
            .map(|path| canonical_person_path(vault, &path, require_link_targets))
            .transpose()?
            .flatten();
    }
    draft.project_url =
        canonical_project_url(vault, draft.project_url.as_deref(), require_link_targets)?;
    draft.source_path =
        canonical_source_path(vault, draft.source_path.as_deref(), require_link_targets)?;
    if draft.context.chars().count() > MAX_CONTEXT_CHARS {
        return Err(format!(
            "commitment context is too long (max {MAX_CONTEXT_CHARS} characters)"
        ));
    }
    draft.context = draft.context.trim().to_string();
    // Only an open commitment can be snoozed: closing one wakes it.
    draft.snoozed_until = if draft.status == "open" {
        canonical_day("snooze date", draft.snoozed_until.as_deref())?
    } else {
        None
    };
    draft.evidence = draft
        .evidence
        .as_deref()
        .map(|value| {
            cap_chars(
                &value.split_whitespace().collect::<Vec<_>>().join(" "),
                MAX_EVIDENCE_CHARS,
            )
        })
        .filter(|value| !value.is_empty());
    Ok(())
}

fn apply_terminal_stamp(draft: &mut Draft, previous: Option<(&str, Option<String>)>) {
    if draft.status == "fulfilled" {
        draft.fulfilled_at = match previous {
            Some(("fulfilled", stamp)) if stamp.is_some() => stamp,
            _ => Some(Utc::now().to_rfc3339()),
        };
    } else {
        draft.fulfilled_at = None;
    }
}

fn commitment_from_draft(rel: String, created: String, draft: Draft) -> Commitment {
    let fingerprint = draft_fingerprint(&draft);
    Commitment {
        url: format!("vault:{rel}"),
        vault_path: rel,
        title: draft.title,
        direction: draft.direction,
        counterparty: draft.counterparty,
        person_path: draft.person_path,
        project_url: draft.project_url,
        due: draft.due,
        status: draft.status,
        channel: draft.channel,
        source_path: draft.source_path,
        created,
        fulfilled_at: draft.fulfilled_at,
        fingerprint,
        context: draft.context,
        snoozed_until: draft.snoozed_until,
        evidence: draft.evidence,
    }
}

fn parse_commitment(vault: &Path, rel: &str, raw: &str) -> Option<Commitment> {
    if validate_commitment_rel(rel).is_err() {
        return None;
    }
    let (map, body) = frontmatter::parse_map(raw);
    if map.get("type").is_some_and(|value| value != "commitment") {
        return None;
    }
    let mut draft = Draft {
        title: map.get("title")?.clone(),
        direction: map.get("direction")?.clone(),
        counterparty: map.get("counterparty")?.clone(),
        person_path: nonempty(&map, "person_path"),
        project_url: nonempty(&map, "project_url"),
        due: nonempty(&map, "due"),
        status: map.get("status").cloned().unwrap_or_else(|| "open".into()),
        channel: map
            .get("channel")
            .cloned()
            .unwrap_or_else(|| "manual".into()),
        source_path: nonempty(&map, "source_path"),
        context: body.trim().to_string(),
        fulfilled_at: nonempty(&map, "fulfilled_at"),
        // A hand-edited, malformed snooze date wakes the commitment rather
        // than hiding the whole note.
        snoozed_until: canonical_day("snooze date", nonempty(&map, "snoozed_until").as_deref())
            .ok()
            .flatten(),
        evidence: nonempty(&map, "evidence"),
    };
    normalize_draft(vault, &mut draft, false, false).ok()?;
    if draft.status != "fulfilled" {
        draft.fulfilled_at = None;
    }
    Some(commitment_from_draft(
        rel.to_string(),
        map.get("created").cloned().unwrap_or_default(),
        draft,
    ))
}

fn render(commitment: &Commitment) -> String {
    let mut lines = vec!["---".to_string()];
    frontmatter::push_kv(&mut lines, "type", "commitment");
    frontmatter::push_kv(&mut lines, "title", &commitment.title);
    frontmatter::push_kv(&mut lines, "direction", &commitment.direction);
    frontmatter::push_kv(&mut lines, "counterparty", &commitment.counterparty);
    if let Some(value) = commitment.person_path.as_deref() {
        frontmatter::push_kv(&mut lines, "person_path", value);
    }
    if let Some(value) = commitment.project_url.as_deref() {
        frontmatter::push_kv(&mut lines, "project_url", value);
    }
    if let Some(value) = commitment.due.as_deref() {
        frontmatter::push_kv(&mut lines, "due", value);
    }
    if let Some(value) = commitment.snoozed_until.as_deref() {
        frontmatter::push_kv(&mut lines, "snoozed_until", value);
    }
    frontmatter::push_kv(&mut lines, "status", &commitment.status);
    frontmatter::push_kv(&mut lines, "channel", &commitment.channel);
    if let Some(value) = commitment.source_path.as_deref() {
        frontmatter::push_kv(&mut lines, "source_path", value);
    }
    if let Some(value) = commitment.evidence.as_deref() {
        frontmatter::push_kv(&mut lines, "evidence", value);
    }
    frontmatter::push_kv(&mut lines, "created", &commitment.created);
    if let Some(value) = commitment.fulfilled_at.as_deref() {
        frontmatter::push_kv(&mut lines, "fulfilled_at", value);
    }
    frontmatter::push_kv(&mut lines, "fingerprint", &commitment.fingerprint);
    lines.push("tags:".into());
    lines.push("  - commitments".into());
    lines.push(format!("  - status/{}", commitment.status));
    lines.push(format!(
        "  - direction/{}",
        commitment.direction.replace('_', "-")
    ));
    lines.push("---".into());
    lines.push(String::new());
    if !commitment.context.is_empty() {
        lines.push(commitment.context.clone());
        lines.push(String::new());
    }
    lines.join("\n")
}

fn validate_commitment_links(
    vault: &Path,
    commitment: &Commitment,
    require_exists: bool,
) -> Result<(), String> {
    canonical_person_path(
        vault,
        commitment.person_path.as_deref().unwrap_or(""),
        require_exists,
    )?;
    canonical_project_url(vault, commitment.project_url.as_deref(), require_exists)?;
    canonical_source_path(vault, commitment.source_path.as_deref(), require_exists)?;
    Ok(())
}

fn find_by_fingerprint(
    vault: &Path,
    fingerprint: &str,
    exclude_rel: Option<&str>,
) -> Option<Commitment> {
    list(vault).into_iter().find(|commitment| {
        commitment.fingerprint == fingerprint
            && exclude_rel
                .map(|rel| commitment.vault_path != rel)
                .unwrap_or(true)
    })
}

fn draft_fingerprint(draft: &Draft) -> String {
    let normalized = format!(
        "{}\0{}\0{}\0{}",
        draft.direction,
        normalize_words(&draft.counterparty),
        normalize_words(&draft.title),
        draft.project_url.as_deref().unwrap_or("").to_lowercase(),
    );
    sha256(&normalized)
}

fn proposal_fingerprint(payload: &CommitmentProposalPayload) -> String {
    draft_fingerprint(&Draft {
        title: payload.title.clone(),
        direction: payload.direction.clone(),
        counterparty: payload.counterparty.clone(),
        person_path: payload.person_path.clone(),
        project_url: payload.project_url.clone(),
        due: payload.due.clone(),
        status: "open".into(),
        channel: payload.channel.clone(),
        source_path: payload.source_path.clone(),
        context: payload.context.clone(),
        fulfilled_at: None,
        snoozed_until: None,
        evidence: None,
    })
}

fn pick_stable_path(
    vault: &Path,
    title: &str,
    fingerprint: &str,
    created: &str,
) -> Result<String, String> {
    let seq = ID_SEQ.fetch_add(1, Ordering::Relaxed);
    let suffix = sha256(&format!("{fingerprint}\0{created}\0{seq}"));
    let slug = slugify(title);
    for width in [12usize, 20, 32, 64] {
        let rel = format!("{COMMITMENTS_DIR}/{slug}-{}.md", &suffix[..width]);
        let full = crate::vault::safe_join(vault, &rel)?;
        if !full.exists() {
            return Ok(rel);
        }
    }
    Err("could not allocate a unique commitment path".into())
}

fn commitment_rel(vault: &Path, url_or_rel: &str) -> Result<String, String> {
    let rel = url_or_rel
        .trim()
        .strip_prefix("vault:")
        .unwrap_or(url_or_rel.trim());
    validate_commitment_rel(rel)?;
    crate::vault::safe_join(vault, rel)?;
    Ok(rel.to_string())
}

fn validate_commitment_rel(rel: &str) -> Result<(), String> {
    let parts: Vec<_> = Path::new(rel).components().collect();
    let valid = matches!(parts.as_slice(), [Component::Normal(root), Component::Normal(file)]
        if *root == std::ffi::OsStr::new(COMMITMENTS_DIR)
            && file.to_string_lossy().ends_with(".md")
            && !file.to_string_lossy().starts_with('.'));
    if valid {
        Ok(())
    } else {
        Err(format!(
            "commitment path must be {COMMITMENTS_DIR}/*.md: {rel}"
        ))
    }
}

fn canonical_person_path(
    vault: &Path,
    value: &str,
    require_exists: bool,
) -> Result<Option<String>, String> {
    canonical_link(
        vault,
        Some(value),
        Some("people/"),
        "person path",
        require_exists,
    )
}

fn canonical_project_url(
    vault: &Path,
    value: Option<&str>,
    require_exists: bool,
) -> Result<Option<String>, String> {
    Ok(canonical_link(
        vault,
        value,
        Some("projects/"),
        "project URL",
        require_exists,
    )?
    .map(|path| format!("vault:{path}")))
}

fn canonical_source_path(
    vault: &Path,
    value: Option<&str>,
    require_exists: bool,
) -> Result<Option<String>, String> {
    canonical_link(vault, value, None, "source path", require_exists)
}

fn canonical_link(
    vault: &Path,
    value: Option<&str>,
    required_prefix: Option<&str>,
    label: &str,
    require_exists: bool,
) -> Result<Option<String>, String> {
    let Some(raw) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let rel = raw.strip_prefix("vault:").unwrap_or(raw);
    if let Some(prefix) = required_prefix {
        if !rel.starts_with(prefix) {
            return Err(format!("{label} must stay under {prefix}: {rel}"));
        }
    }
    if !rel.ends_with(".md") {
        return Err(format!("{label} must reference a markdown note: {rel}"));
    }
    if Path::new(rel).components().any(|component| {
        matches!(component, Component::Normal(name) if name.to_string_lossy().starts_with('.'))
    }) {
        return Err(format!("{label} cannot reference hidden vault data: {rel}"));
    }
    let full = crate::vault::safe_join(vault, rel)?;
    if require_exists && !full.is_file() {
        return Err(format!("{label} does not exist: {rel}"));
    }
    Ok(Some(rel.to_string()))
}

/// Resolve only a unique case-insensitive full-name match. Partial and fuzzy
/// matches are intentionally ignored so an autonomous detector cannot attach
/// a promise to the wrong person.
fn resolve_exact_person(vault: &Path, counterparty: &str) -> Option<String> {
    let mut matches = crate::vault_people::list(vault)
        .into_iter()
        .filter(|person| person.name.trim().eq_ignore_ascii_case(counterparty.trim()))
        .map(|person| person.path);
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

fn canonical_due(value: Option<&str>) -> Result<Option<String>, String> {
    canonical_day("due date", value)
}

fn canonical_day(label: &str, value: Option<&str>) -> Result<Option<String>, String> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| format!("{label} must be exact YYYY-MM-DD: {value}"))?;
    if parsed.format("%Y-%m-%d").to_string() != value {
        return Err(format!("{label} must be exact YYYY-MM-DD: {value}"));
    }
    Ok(Some(value.to_string()))
}

fn canonical_choice(label: &str, value: &str, allowed: &[&str]) -> Result<String, String> {
    let normalized = value.trim().to_lowercase().replace('-', "_");
    if allowed.contains(&normalized.as_str()) {
        Ok(normalized)
    } else {
        Err(format!("unsupported {label}: {value}"))
    }
}

fn required_text(label: &str, value: &str, max: usize) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("commitment {label} is required"));
    }
    if value.chars().count() > max {
        return Err(format!(
            "commitment {label} is too long (max {max} characters)"
        ));
    }
    Ok(value.to_string())
}

fn optional_owned(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn nonempty(map: &BTreeMap<String, String>, key: &str) -> Option<String> {
    map.get(key).and_then(|value| optional_owned(value))
}

fn normalize_words(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn sha256(value: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(value.as_bytes());
    format!("{:x}", hash.finalize())
}

fn slugify(value: &str) -> String {
    let words = value
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-");
    let slug = if words.is_empty() {
        "commitment"
    } else {
        &words
    };
    slug.chars().take(90).collect()
}

fn cap_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        format!(
            "{}…",
            value
                .chars()
                .take(max.saturating_sub(1))
                .collect::<String>()
        )
    }
}

fn status_rank(status: &str) -> u8 {
    match status {
        "open" => 0,
        "fulfilled" => 1,
        "cancelled" => 2,
        _ => 3,
    }
}

fn option_sort_key(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("9999-12-31")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn vault() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    fn input(title: &str) -> CreateCommitmentInput {
        CreateCommitmentInput {
            title: title.into(),
            direction: "owed_by_me".into(),
            counterparty: "Jordan Smith".into(),
            due: Some("2026-07-16".into()),
            channel: Some("manual".into()),
            context: "Context and exact promise.".into(),
            ..Default::default()
        }
    }

    fn proposal(title: &str) -> CommitmentProposalPayload {
        CommitmentProposalPayload {
            title: title.into(),
            direction: "owed_by_me".into(),
            counterparty: "Jordan Smith".into(),
            person_path: None,
            project_url: None,
            due: Some("2026-07-16".into()),
            channel: "email".into(),
            source_path: None,
            context: "Promised in a follow-up.".into(),
            rationale: "Explicit future promise".into(),
            evidence_excerpt: "I will send it Friday.".into(),
            source_run_id: Some("run-1".into()),
        }
    }

    fn write(vault: &Path, rel: &str, raw: &str) {
        let path = vault.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, raw).unwrap();
    }

    fn add_person(vault: &Path, filename: &str, title: &str) {
        write(
            vault,
            &format!("people/{filename}.md"),
            &format!("---\ntitle: {title}\n---\nNotes"),
        );
    }

    #[test]
    fn public_types_serialize_camel_case() {
        let value = serde_json::to_value(CreateCommitmentInput {
            title: "Send deck".into(),
            direction: "owed_by_me".into(),
            counterparty: "Jordan".into(),
            person_path: Some("people/Jordan.md".into()),
            project_url: Some("vault:projects/active/Launch.md".into()),
            source_path: Some("emails/sent/x.md".into()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(value["personPath"], "people/Jordan.md");
        assert_eq!(value["projectUrl"], "vault:projects/active/Launch.md");
        assert_eq!(value["sourcePath"], "emails/sent/x.md");
        assert!(value.get("person_path").is_none());

        let trashed: TrashedCommitment = serde_json::from_value(json!({
            "url": "vault:commitments/send-deck.md",
            "content": "raw"
        }))
        .unwrap();
        assert_eq!(trashed.content, "raw");
    }

    #[test]
    fn create_round_trips_links_and_resolves_one_exact_person() {
        let dir = vault();
        add_person(dir.path(), "Jordan Smith", "Jordan Smith");
        write(
            dir.path(),
            "projects/active/Launch.md",
            "---\ntitle: Launch\n---\n",
        );
        write(dir.path(), "emails/sent/follow-up.md", "sent message");

        let mut create_input = input("Send Jordan the revised deck");
        create_input.project_url = Some("projects/active/Launch.md".into());
        create_input.source_path = Some("vault:emails/sent/follow-up.md".into());
        create_input.channel = Some("teams".into());
        let created = create(dir.path(), create_input).unwrap();

        assert!(created.url.starts_with("vault:commitments/"));
        assert_eq!(
            created.person_path.as_deref(),
            Some("people/Jordan Smith.md")
        );
        assert_eq!(
            created.project_url.as_deref(),
            Some("vault:projects/active/Launch.md")
        );
        assert_eq!(
            created.source_path.as_deref(),
            Some("emails/sent/follow-up.md")
        );
        assert_eq!(created.status, "open");
        assert_eq!(list(dir.path()), vec![created.clone()]);

        let raw = fs::read_to_string(dir.path().join(&created.vault_path)).unwrap();
        assert!(raw.contains("type: commitment"));
        assert!(raw.contains("direction: owed_by_me"));
        assert!(raw.contains("Context and exact promise."));
    }

    #[test]
    fn exact_person_resolution_refuses_ambiguous_or_partial_matches() {
        let dir = vault();
        add_person(dir.path(), "Jordan-one", "Jordan Smith");
        add_person(dir.path(), "Jordan-two", "Jordan Smith");
        let ambiguous = create(dir.path(), input("Send first deck")).unwrap();
        assert_eq!(ambiguous.person_path, None);

        let mut partial_input = input("Send second deck");
        partial_input.counterparty = "Jordan".into();
        let partial = create(dir.path(), partial_input).unwrap();
        assert_eq!(partial.person_path, None);
    }

    #[test]
    fn terminal_status_does_not_escape_fingerprint_dedup() {
        let dir = vault();
        let first = create(dir.path(), input("Send the revised deck")).unwrap();
        let fulfilled = set_status(dir.path(), &first.url, "fulfilled").unwrap();
        assert!(fulfilled.fulfilled_at.is_some());

        let mut duplicate_input = input("  SEND   THE revised DECK ");
        duplicate_input.due = Some("2026-08-01".into());
        duplicate_input.channel = Some("slack".into());
        duplicate_input.context = "Different capture context".into();
        let duplicate = create(dir.path(), duplicate_input).unwrap();

        assert_eq!(duplicate.url, first.url);
        assert_eq!(duplicate.status, "fulfilled");
        assert_eq!(list(dir.path()).len(), 1);
        assert_eq!(duplicate.fingerprint, first.fingerprint);
    }

    #[test]
    fn update_and_status_changes_preserve_path_and_manage_fulfilled_stamp() {
        let dir = vault();
        let original = create(dir.path(), input("Send deck")).unwrap();
        let updated = update(
            dir.path(),
            UpdateCommitmentInput {
                url: original.url.clone(),
                title: Some("Send final deck".into()),
                due: Some("".into()),
                channel: Some("meeting".into()),
                context: Some("Updated context".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(updated.url, original.url);
        assert_eq!(updated.vault_path, original.vault_path);
        assert_eq!(updated.due, None);
        assert_eq!(updated.channel, "meeting");

        let fulfilled = set_status(dir.path(), &updated.url, "fulfilled").unwrap();
        let stamp = fulfilled.fulfilled_at.clone().unwrap();
        let fulfilled_again = set_status(dir.path(), &updated.url, "fulfilled").unwrap();
        assert_eq!(
            fulfilled_again.fulfilled_at.as_deref(),
            Some(stamp.as_str())
        );

        let reopened = set_status(dir.path(), &updated.url, "open").unwrap();
        assert_eq!(reopened.url, original.url);
        assert_eq!(reopened.fulfilled_at, None);
    }

    #[test]
    fn validates_due_enums_and_required_text() {
        let dir = vault();
        for bad_due in ["2026-7-01", "2026-02-30", "tomorrow"] {
            let mut bad = input("Send deck");
            bad.due = Some(bad_due.into());
            assert!(create(dir.path(), bad).unwrap_err().contains("YYYY-MM-DD"));
        }

        let mut bad = input("Send deck");
        bad.direction = "mine".into();
        assert!(create(dir.path(), bad).unwrap_err().contains("direction"));
        let mut bad = input("Send deck");
        bad.status = Some("done".into());
        assert!(create(dir.path(), bad).unwrap_err().contains("status"));
        let mut bad = input("Send deck");
        bad.channel = Some("sms".into());
        assert!(create(dir.path(), bad).unwrap_err().contains("channel"));
        let bad = input("   ");
        assert!(create(dir.path(), bad).unwrap_err().contains("title"));
    }

    #[test]
    fn rejects_unsafe_missing_or_wrong_kind_links() {
        let dir = vault();
        let mut bad = input("Send deck");
        bad.person_path = Some("people/../../outside.md".into());
        assert!(create(dir.path(), bad).unwrap_err().contains("path escape"));

        let mut bad = input("Send deck");
        bad.project_url = Some("vault:people/Jordan.md".into());
        assert!(create(dir.path(), bad).unwrap_err().contains("projects/"));

        let mut bad = input("Send deck");
        bad.source_path = Some(".review-queue.json.md".into());
        assert!(create(dir.path(), bad).unwrap_err().contains("hidden"));

        let mut bad = input("Send deck");
        bad.source_path = Some("emails/missing.md".into());
        assert!(create(dir.path(), bad)
            .unwrap_err()
            .contains("does not exist"));
    }

    #[test]
    fn undoing_a_reopen_keeps_the_original_fulfilled_stamp() {
        let dir = vault();
        let created = create(dir.path(), input("Send the deck")).unwrap();
        let done = set_status(dir.path(), &created.url, "fulfilled").unwrap();
        let stamp = done.fulfilled_at.clone().expect("fulfilled is stamped");
        let reopened = set_status(dir.path(), &created.url, "open").unwrap();
        assert_eq!(reopened.fulfilled_at, None);
        std::thread::sleep(std::time::Duration::from_millis(5));
        // The ledger's undo sends the stamp it had.
        let undone = update(
            dir.path(),
            UpdateCommitmentInput {
                url: created.url.clone(),
                status: Some("fulfilled".into()),
                fulfilled_at: Some(stamp.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(undone.fulfilled_at.as_deref(), Some(stamp.as_str()));
        // Not a date: ignored, the commitment is stamped now as before.
        set_status(dir.path(), &created.url, "open").unwrap();
        let fresh = update(
            dir.path(),
            UpdateCommitmentInput {
                url: created.url.clone(),
                status: Some("fulfilled".into()),
                fulfilled_at: Some("yesterday".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let fresh_stamp = fresh.fulfilled_at.clone();
        assert!(fresh_stamp
            .as_deref()
            .is_some_and(|t| t != "yesterday" && t != stamp));
        // An existing completion's date is never edited, and a future stamp is ignored.
        let again = update(
            dir.path(),
            UpdateCommitmentInput {
                url: created.url.clone(),
                status: Some("fulfilled".into()),
                fulfilled_at: Some(stamp.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(again.fulfilled_at, fresh_stamp);
        set_status(dir.path(), &created.url, "open").unwrap();
        let future = update(
            dir.path(),
            UpdateCommitmentInput {
                url: created.url.clone(),
                status: Some("fulfilled".into()),
                fulfilled_at: Some("2999-01-01T00:00:00+00:00".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(future.fulfilled_at.is_some_and(|t| !t.starts_with("2999")));
        // A stamp never applies to a commitment that isn't fulfilled.
        let open = update(
            dir.path(),
            UpdateCommitmentInput {
                url: created.url,
                status: Some("open".into()),
                fulfilled_at: Some(stamp),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(open.fulfilled_at, None);
    }

    #[test]
    fn update_rejects_collision_with_another_commitment() {
        let dir = vault();
        let first = create(dir.path(), input("Send first deck")).unwrap();
        let second = create(dir.path(), input("Send second deck")).unwrap();
        let err = update(
            dir.path(),
            UpdateCommitmentInput {
                url: second.url,
                title: Some(first.title),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.contains("duplicate commitment"));
        assert_eq!(list(dir.path()).len(), 2);
    }

    #[test]
    fn trash_and_restore_are_lossless_and_never_clobber() {
        let dir = vault();
        let created = create(dir.path(), input("Send deck")).unwrap();
        let before = fs::read_to_string(dir.path().join(&created.vault_path)).unwrap();
        let trashed = trash(dir.path(), &created.url).unwrap();
        assert_eq!(trashed.url, created.url);
        assert_eq!(trashed.content, before);
        assert!(!dir.path().join(&created.vault_path).exists());
        assert!(list(dir.path()).is_empty());

        let restored = restore(dir.path(), trashed.clone()).unwrap();
        assert_eq!(restored.url, created.url);
        assert_eq!(
            fs::read_to_string(dir.path().join(&created.vault_path)).unwrap(),
            before
        );
        assert!(restore(dir.path(), trashed)
            .unwrap_err()
            .contains("overwrite"));

        let outside = TrashedCommitment {
            url: "vault:tasks/not-a-commitment.md".into(),
            content: before,
        };
        assert!(restore(dir.path(), outside)
            .unwrap_err()
            .contains("commitments/*.md"));
    }

    #[test]
    fn snooze_round_trips_wakes_on_its_day_and_closing_clears_it() {
        let dir = vault();
        let made = create(dir.path(), input("Send deck")).unwrap();
        assert_eq!(made.snoozed_until, None);
        let snoozed = update(
            dir.path(),
            UpdateCommitmentInput {
                url: made.url.clone(),
                snoozed_until: Some("2026-07-20".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(snoozed.snoozed_until.as_deref(), Some("2026-07-20"));
        assert_eq!(snoozed.fingerprint, made.fingerprint);
        assert_eq!(
            list(dir.path())[0].snoozed_until.as_deref(),
            Some("2026-07-20")
        );
        assert!(is_snoozed(&snoozed, "2026-07-19"));
        assert!(!is_snoozed(&snoozed, "2026-07-20"));
        // A bad date is refused; the note keeps its snooze.
        assert!(update(
            dir.path(),
            UpdateCommitmentInput {
                url: made.url.clone(),
                snoozed_until: Some("next week".into()),
                ..Default::default()
            },
        )
        .unwrap_err()
        .contains("snooze date"));
        // Closing it wakes it; reopening doesn't bring the snooze back.
        let done = set_status(dir.path(), &made.url, "fulfilled").unwrap();
        assert_eq!(done.snoozed_until, None);
        assert!(!is_snoozed(&done, "2026-07-01"));
        let reopened = set_status(dir.path(), &made.url, "open").unwrap();
        assert_eq!(reopened.snoozed_until, None);
        // "" wakes an open one.
        update(
            dir.path(),
            UpdateCommitmentInput {
                url: made.url.clone(),
                snoozed_until: Some("2026-07-20".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let woke = update(
            dir.path(),
            UpdateCommitmentInput {
                url: made.url.clone(),
                snoozed_until: Some(String::new()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(woke.snoozed_until, None);
    }

    #[test]
    fn a_malformed_hand_written_snooze_wakes_the_note_instead_of_hiding_it() {
        let dir = vault();
        write(
            dir.path(),
            "commitments/hand.md",
            "---\ntitle: Call Sam\ndirection: owed_by_me\ncounterparty: Sam\nsnoozed_until: soon\nstatus: open\nchannel: manual\ncreated: 2026-07-13T12:00:00Z\n---\n",
        );
        let all = list(dir.path());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].snoozed_until, None);
    }

    #[test]
    fn an_accepted_proposal_keeps_its_evidence_as_one_quoted_line() {
        let dir = vault();
        let mut payload = proposal("Send the board deck");
        payload.evidence_excerpt = "  I'll send the deck\n  by Friday.  ".into();
        let pending = submit_proposal(dir.path(), payload).unwrap();
        resolve_proposal(dir.path(), &pending.id, "accept", None).unwrap();
        let all = list(dir.path());
        assert_eq!(
            all[0].evidence.as_deref(),
            Some("I'll send the deck by Friday.")
        );
        let raw = fs::read_to_string(dir.path().join(&all[0].vault_path)).unwrap();
        assert!(raw.contains("evidence:"), "{raw}");
        // Editing other fields keeps it.
        let edited = update(
            dir.path(),
            UpdateCommitmentInput {
                url: all[0].url.clone(),
                due: Some("2026-07-30".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            edited.evidence.as_deref(),
            Some("I'll send the deck by Friday.")
        );
    }

    #[test]
    fn hand_authored_plan_schema_gets_a_stable_computed_fingerprint() {
        let dir = vault();
        write(
            dir.path(),
            "commitments/send-jordan.md",
            "---\ntitle: Send Jordan the revised deck\ndirection: owed_by_me\ncounterparty: Jordan Smith\ndue: 2026-07-16\nstatus: open\nchannel: teams\ncreated: 2026-07-13T12:00:00Z\n---\nContext and exact promise.\n",
        );
        let first = list(dir.path());
        let second = list(dir.path());
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].fingerprint, second[0].fingerprint);
        assert_eq!(first[0].context, "Context and exact promise.");
    }

    #[test]
    fn rejected_proposal_is_sticky_and_never_creates_a_note() {
        let dir = vault();
        let pending = submit_proposal(dir.path(), proposal("Send deck")).unwrap();
        assert_eq!(pending.status, "pending");
        assert!(list(dir.path()).is_empty());
        let rejected = resolve_proposal(dir.path(), &pending.id, "reject", None).unwrap();
        assert_eq!(rejected.status, "rejected");
        let duplicate = submit_proposal(dir.path(), proposal("  SEND  deck ")).unwrap();
        assert_eq!(duplicate.id, pending.id);
        assert_eq!(duplicate.status, "rejected");
        assert!(list(dir.path()).is_empty());
    }

    #[test]
    fn accepting_edited_proposal_uses_normal_create_and_is_idempotent() {
        let dir = vault();
        add_person(dir.path(), "Jordan Smith", "Jordan Smith");
        let pending = submit_proposal(dir.path(), proposal("Send draft deck")).unwrap();
        let mut edited = pending.payload.clone();
        edited.title = "Send final deck".into();
        edited.channel = "teams".into();
        let accepted = resolve_proposal(dir.path(), &pending.id, "accept", Some(edited)).unwrap();
        assert_eq!(accepted.status, "accepted");
        let commitments = list(dir.path());
        assert_eq!(commitments.len(), 1);
        assert_eq!(commitments[0].title, "Send final deck");
        assert_eq!(
            commitments[0].person_path.as_deref(),
            Some("people/Jordan Smith.md")
        );

        // The original detector meaning is sticky in the queue; the edited
        // accepted meaning is independently blocked by the durable record.
        assert_eq!(
            submit_proposal(dir.path(), proposal("Send draft deck"))
                .unwrap()
                .status,
            "accepted"
        );
        let mut accepted_meaning = proposal("Send final deck");
        accepted_meaning.channel = "slack".into();
        assert!(submit_proposal(dir.path(), accepted_meaning)
            .unwrap_err()
            .contains("already tracked"));
    }

    #[test]
    fn direct_existing_commitment_blocks_new_proposal_even_when_cancelled() {
        let dir = vault();
        let existing = create(dir.path(), input("Send deck")).unwrap();
        set_status(dir.path(), &existing.url, "cancelled").unwrap();
        assert!(submit_proposal(dir.path(), proposal("Send deck"))
            .unwrap_err()
            .contains("already tracked"));
    }
}

/// Master plan #2: the commitment tracker over extracted promises.
#[cfg(test)]
mod tracker_tests {
    use super::*;
    use crate::extract::tests::{block_on, source, vault_with_transcript, Fake, ANSWER};
    use crate::extract::{run, Source};
    use chrono::Utc;

    const SECOND: &str = "# Vendor call\n\n\
[00:01 - 00:09] Alex: Right, I'll send Ana the Q3 numbers by Friday.\n\
[00:09 - 00:15] Dana: The VendorSync export is owed to Raj, someone has it.\n\
[00:15 - 00:22] Raj: Ana owes Dana the venue list.\n";

    const SECOND_ANSWER: &str = r#"[
 {"kind":"commitment","text":"Send Ana the Q3 numbers","owner":"Alex Morgan","counterparty":"Ana","due":null,"segment":"S1","evidence":"I'll send Ana the Q3 numbers by Friday"},
 {"kind":"commitment","text":"Hand over the VendorSync export","owner":null,"counterparty":"Raj","due":null,"segment":"S2","evidence":"The VendorSync export is owed to Raj"},
 {"kind":"commitment","text":"Send Dana the venue list","owner":"Ana","counterparty":"Dana","due":null,"segment":"S3","evidence":"Ana owes Dana the venue list"}
]"#;

    fn second(v: &Path) -> Source {
        std::fs::write(
            v.join("workspaces/ops/sources/recording-vendor-call-def.md"),
            SECOND,
        )
        .unwrap();
        Source::Recording {
            path: "workspaces/ops/sources/recording-vendor-call-def.md".into(),
        }
    }

    #[test]
    fn a_duplicate_across_two_meetings_an_unclear_owner_and_two_others() {
        let dir = vault_with_transcript();
        let v = dir.path();
        block_on(run(
            v,
            &source(),
            &Fake::new(ANSWER),
            "Alex Morgan",
            Utc::now(),
        ))
        .unwrap();
        let report = block_on(run(
            v,
            &second(v),
            &Fake::new(SECOND_ANSWER),
            "Alex Morgan",
            Utc::now(),
        ))
        .unwrap();
        let reasons: Vec<&str> = report.dropped.iter().map(|d| d.reason.as_str()).collect();
        assert!(
            reasons
                .iter()
                .any(|r| r.starts_with("already tracked or proposed")),
            "the Q3 promise from the first meeting: {reasons:?}"
        );
        assert!(
            reasons.iter().any(|r| r.contains("two other people")),
            "Ana owes Dana isn't the person's: {reasons:?}"
        );
        assert_eq!(report.queued.len(), 1, "{:?}", report.queued);
        let unclear = list_proposals(v)
            .unwrap()
            .into_iter()
            .find(|p| p.payload.title == "Hand over the VendorSync export")
            .unwrap();
        assert_eq!(unclear.payload.direction, UNKNOWN_DIRECTION);
        assert_eq!(unclear.payload.counterparty, "Raj");
        // Never guessed: accepting as it is refuses; the person picks.
        let err = resolve_proposal(v, &unclear.id, "accept", None).unwrap_err();
        assert!(err.contains("Choose who owes whom"), "{err}");
        assert!(list(v).is_empty());
        let mut picked = unclear.payload.clone();
        picked.direction = "owed_to_me".into();
        resolve_proposal(v, &unclear.id, "accept", Some(picked)).unwrap();
        let tracked = list(v);
        assert_eq!(tracked.len(), 1);
        assert_eq!(
            (
                tracked[0].direction.as_str(),
                tracked[0].counterparty.as_str()
            ),
            ("owed_to_me", "Raj")
        );
        assert!(
            tracked[0]
                .context
                .contains("recording-vendor-call-def.md@00:09"),
            "{}",
            tracked[0].context
        );
    }

    #[test]
    fn filters_by_direction_and_person() {
        let dir = vault_with_transcript();
        let v = dir.path();
        for (title, direction, who) in [
            ("Q3 numbers", "owed_by_me", "Ana Ruiz"),
            ("VendorSync export", "owed_to_me", "Raj"),
            ("Venue list", "owed_to_me", "Ana Ruiz"),
        ] {
            create(
                v,
                CreateCommitmentInput {
                    title: title.into(),
                    direction: direction.into(),
                    counterparty: who.into(),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        let titles = |d: Option<&str>, p: Option<&str>| {
            let mut t: Vec<String> = filter(list(v), d, p).into_iter().map(|c| c.title).collect();
            t.sort();
            t
        };
        assert_eq!(titles(Some("owed_by_me"), None), vec!["Q3 numbers"]);
        assert_eq!(titles(Some("owed_to_me"), Some("ana")), vec!["Venue list"]);
        assert_eq!(titles(None, Some("ANA")).len(), 2);
        assert_eq!(titles(Some(""), None).len(), 3);
    }

    #[test]
    fn the_notification_counts_promises_first() {
        let dir = vault_with_transcript();
        let v = dir.path();
        let report = block_on(run(
            v,
            &source(),
            &Fake::new(ANSWER),
            "Alex Morgan",
            Utc::now(),
        ))
        .unwrap();
        let (title, body) = crate::extract::found_message(&report).unwrap();
        assert_eq!(title, "2 promises found in Ops sync");
        assert!(
            body.starts_with("Also 1 task, 1 decision, 1 fact about a person."),
            "{body}"
        );
        let skipped = block_on(run(
            v,
            &source(),
            &Fake::new(ANSWER),
            "Alex Morgan",
            Utc::now(),
        ))
        .unwrap();
        assert!(crate::extract::found_message(&skipped).is_none());
    }
}
