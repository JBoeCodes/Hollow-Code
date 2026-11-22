//! The structured request a run is prepared from (agent effectiveness F2,
//! wave 1 · lane C).
//!
//! A chat send arrives as one composed prompt: the composer's conversation,
//! operator, page and intent blocks around the user's own words, and an
//! attached-images block after them. [`TurnRequest::admit`] takes it apart
//! ONCE, where the turn is accepted (`chat::accept_turn`), and the queued
//! payload carries the parts to preparation. Nothing downstream re-derives
//! them from the composed prompt.
//!
//! - `request`: the user's own text (`sessions::strip_injected_context`, the
//!   same text the turn's `asked` holds).
//! - `entities`: explicit file and entity ids the request names or attaches
//!   (vault-relative where the path is in the vault).
//! - `antecedents`: what a reference like "that project" or "the same file"
//!   resolves to, from this conversation's earlier requests
//!   ([`resolve_antecedents`]: deterministic, local, bounded, never a model).
//! - `context_blocks`: the injected blocks, kept separate and in order, by
//!   tag, size and SHA-256 only: their text is already in the queued
//!   prompt, and nothing downstream reads it from here.
//! - `task_state`: the conversation's task-state record as admitted (wave 2
//!   · K2, `task_state.rs`): its revision, last folded turn and item count.
//!   A reference only; the context compiler folds anything newer when the
//!   turn runs and sends the record itself on a fresh native session.
//!
//! Recall, connector selection and post-turn learning read the request plus
//! its resolved entities, never the composed prompt, so profile prose, page
//! context or an old conversation bundle can't pick memories or connectors
//! or trigger a learning extraction. A workflow's request is its prompt after
//! variable substitution ([`TurnRequest::from_text`]).
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

/// At most this many explicit entities per request.
pub(crate) const MAX_ENTITIES: usize = 8;
/// The antecedent resolver looks back over at most this many earlier user
/// requests of the same conversation.
pub(crate) const ANTECEDENT_TURNS: usize = 8;
/// Of each earlier request, only this many characters are read.
pub(crate) const ANTECEDENT_TURN_CHARS: usize = 4_000;
/// At most this many antecedents per request.
pub(crate) const MAX_ANTECEDENTS: usize = 3;
/// Note names are matched from at most this many notes per record folder.
pub(crate) const MAX_NOTES_PER_FOLDER: usize = 500;
/// The record folders whose note names count as entities when a request
/// names them, each read to depth 2 (`projects/{status}/{name}.md`).
const ENTITY_FOLDERS: [&str; 3] = ["projects", "people", "goals"];

/// One injected block of the composed prompt (`operator-context`,
/// `page-context`, `conversation-context`, `hollow-chat-intent`, …): its tag,
/// and its body's size and SHA-256. The body itself stays in the queued
/// prompt only (wave 1 follow-up: it used to be copied here too, doubling
/// the queued payload; no reader used the copy). A row queued with the old
/// `body` field still reads, with no size or hash.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextBlock {
    pub tag: String,
    #[serde(default)]
    pub bytes: usize,
    #[serde(default)]
    pub sha256: String,
}

/// An entity a reference in the request resolved to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Antecedent {
    /// The entity id, as [`TurnRequest::entities`] holds them.
    pub path: String,
    /// The earlier turn (`turns.ord`) whose request named it.
    pub from_turn: i64,
    /// The phrase that asked for it ("that project").
    pub reference: String,
}

/// The conversation's task-state record when the turn was admitted (agent
/// effectiveness F4, K2): which revision, through which turn, how many
/// items. `None` before the conversation has one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskState {
    #[serde(default)]
    pub revision: i64,
    #[serde(default)]
    pub through_turn: i64,
    #[serde(default)]
    pub items: usize,
}

/// See the module doc.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnRequest {
    pub request: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub antecedents: Vec<Antecedent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_blocks: Vec<ContextBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_state: Option<TaskState>,
}

impl TurnRequest {
    /// Admit a composed chat prompt: split it once, find the request's
    /// explicit entities (named paths and notes, attached images) and, when
    /// the request refers back ("that project"), resolve the reference from
    /// conversation `session_key`'s earlier requests.
    pub(crate) fn admit(vault: &Path, composed: &str, session_key: Option<&str>) -> Self {
        let split = crate::sessions::split_injected_context(composed);
        let notes = NoteIndex::load(vault);
        let mut entities = explicit_entities(vault, &notes, &split.user_text);
        for path in split
            .attached_images
            .as_deref()
            .map(attached_paths)
            .unwrap_or_default()
        {
            if let Some(id) = normalize_path(vault, &path) {
                push_unique(&mut entities, id);
            }
        }
        entities.truncate(MAX_ENTITIES);
        // The conversation is read only when the request refers back.
        let antecedents = session_key
            .filter(|_| refers_back(&split.user_text))
            .map(|key| {
                let earlier = crate::session_store::latest_asks(vault, key, ANTECEDENT_TURNS)
                    .unwrap_or_default();
                resolve_antecedents(vault, &notes, &split.user_text, &entities, &earlier)
            })
            .unwrap_or_default();
        TurnRequest {
            request: split.user_text,
            entities,
            antecedents,
            context_blocks: split
                .blocks
                .into_iter()
                .map(|(tag, body)| ContextBlock {
                    tag,
                    bytes: body.len(),
                    sha256: crate::run_manifest::sha256_hex(body.as_bytes()),
                })
                .collect(),
            task_state: session_key
                .and_then(|key| crate::task_state::stored(vault, key))
                .map(|state| TaskState {
                    revision: state.revision,
                    through_turn: state.through_turn,
                    items: state.record.items(),
                }),
        }
    }

    /// A request that is already the user's (or the workflow's) own text:
    /// a workflow prompt after variable substitution, or a chat row queued
    /// before these fields existed (its `asked`). Entities only; no
    /// conversation to resolve references from.
    pub(crate) fn from_text(vault: &Path, text: &str) -> Self {
        let notes = NoteIndex::load(vault);
        let mut entities = explicit_entities(vault, &notes, text);
        entities.truncate(MAX_ENTITIES);
        TurnRequest {
            request: text.to_string(),
            entities,
            ..TurnRequest::default()
        }
    }

    /// What recall and local connector selection match on: the request,
    /// then the display name of each resolved antecedent ("Atlas" for
    /// `projects/active/Atlas.md`). Folder names stay out, so `projects/`
    /// never matches a connector that mentions projects. Never sent to Jev
    /// ([`TurnRequest::connector_query`]).
    pub(crate) fn selection_query(&self) -> String {
        let mut query = self.request.clone();
        for antecedent in &self.antecedents {
            let name = display_name(&antecedent.path);
            if !name.is_empty() {
                query.push('\n');
                query.push_str(&name);
            }
        }
        query
    }

    /// What a workspace's attached sources are ranked against (agent
    /// effectiveness F3 part 2): [`TurnRequest::selection_query`], the
    /// request plus the names its references resolved to. Explicit entities
    /// are already the request's own words; attached image paths would only
    /// add noise. Local only: never sent anywhere.
    pub(crate) fn source_query(&self) -> String {
        self.selection_query()
    }

    /// What connector selection reads: Jev is sent the request alone (a
    /// resolved name is text the person didn't type this turn), the local
    /// rules read [`TurnRequest::selection_query`].
    pub(crate) fn connector_query(&self) -> crate::context_rank::ConnectorQuery {
        crate::context_rank::ConnectorQuery {
            request: self.request.clone(),
            local: self.selection_query(),
        }
    }

    /// The files and entities the request is about: its explicit entities,
    /// then its antecedents, without repeats.
    pub(crate) fn subject_paths(&self) -> Vec<String> {
        let mut out = self.entities.clone();
        for antecedent in &self.antecedents {
            push_unique(&mut out, antecedent.path.clone());
        }
        out
    }

    /// Approved recall for this request: its words and resolved names, with
    /// its subject paths. Whether it asks about the past is read from the
    /// request alone (agent effectiveness F7), as retrieval reads it.
    pub(crate) fn recall(&self, vault: &Path) -> crate::vault_learning::RecallResult {
        self.recall_in(vault, None)
    }

    /// [`Self::recall`] for a turn in workspace `scope` (its decision records
    /// join the global ones).
    pub(crate) fn recall_in(
        &self,
        vault: &Path,
        scope: Option<&str>,
    ) -> crate::vault_learning::RecallResult {
        self.recall_cued_by(vault, scope, &self.request)
    }

    /// [`Self::recall_in`] with whether it asks about the past read from
    /// `own`: for this request read as a turn's basis query
    /// ([`Self::reading`], overnight OB18, OB25), `own` is the request's own
    /// words, so a prior's terms never make it historical (F7).
    pub(crate) fn recall_cued_by(
        &self,
        vault: &Path,
        scope: Option<&str>,
        own: &str,
    ) -> crate::vault_learning::RecallResult {
        let engaged = crate::vault_embed_index::EngagedMeaning { vault };
        self.recall_meant(vault, scope, own, &engaged)
    }

    /// [`Self::recall_cued_by`] for a turn that retrieves nothing (a bare
    /// run, a skipped host turn or follow-up; OF26 review): it embeds
    /// nothing either, since no retrieval would reuse the vector, and ranks
    /// by stems (`RecallMeaning::Skipped`, or `Off` when semantic search is
    /// off or the model absent: second review).
    pub(crate) fn recall_without_meaning(
        &self,
        vault: &Path,
        scope: Option<&str>,
        own: &str,
    ) -> crate::vault_learning::RecallResult {
        let engaged = crate::vault_embed_index::EngagedMeaning { vault };
        self.recall_skipping_meaning(vault, scope, own, &engaged)
    }

    /// [`Self::recall_without_meaning`] with `source` (tests: a stand-in):
    /// it is only asked whether it is available, a check that never waits.
    pub(crate) fn recall_skipping_meaning(
        &self,
        vault: &Path,
        scope: Option<&str>,
        own: &str,
        source: &dyn crate::vault_learning::MeaningSource,
    ) -> crate::vault_learning::RecallResult {
        use crate::vault_learning::RecallMeaning;
        let mut recall = self.recall_ranked(vault, scope, own, None);
        recall.meaning = if source.available() {
            RecallMeaning::Skipped
        } else {
            RecallMeaning::Off
        };
        recall
    }

    /// [`Self::recall_cued_by`] for a turn whose prefetched recall missed its
    /// cutoff (J3a, `recall_prefetch`): ranked by stems now, starting no
    /// embedding (the prefetch's is still running and leaves its vector for
    /// retrieval), and recorded `TimedOut`, as a late vector always was.
    pub(crate) fn recall_past_cutoff(
        &self,
        vault: &Path,
        scope: Option<&str>,
        own: &str,
    ) -> crate::vault_learning::RecallResult {
        let engaged = crate::vault_embed_index::EngagedMeaning { vault };
        self.recall_late_with(vault, scope, own, &engaged)
    }

    /// [`Self::recall_past_cutoff`] with `source` (tests: a stand-in): it is
    /// only asked whether it is available. Not available is `Off`, nothing to
    /// embed `NoVector`, as [`Self::recall_meant`] records them.
    pub(crate) fn recall_late_with(
        &self,
        vault: &Path,
        scope: Option<&str>,
        own: &str,
        source: &dyn crate::vault_learning::MeaningSource,
    ) -> crate::vault_learning::RecallResult {
        use crate::vault_learning::RecallMeaning;
        let mut recall = self.recall_ranked(vault, scope, own, None);
        recall.meaning = if !source.available() {
            RecallMeaning::Off
        } else if self.meaning_query().is_none() {
            RecallMeaning::NoVector
        } else {
            RecallMeaning::TimedOut
        };
        recall
    }

    /// [`Self::recall_cued_by`] with its vectors from `source` (OF26): when
    /// [`Self::meaning_query`] embeds, global learnings rank by meaning too
    /// (`vault_learning::recall_with_scope`, which asks `source` once for
    /// every global learning's cosine); with nothing to embed or no vector,
    /// exactly as before. The result says which (`RecallResult::meaning`):
    /// nothing to embed is `NoVector` only when `source` is available,
    /// else `Off` (second review).
    pub(crate) fn recall_meant(
        &self,
        vault: &Path,
        scope: Option<&str>,
        own: &str,
        source: &dyn crate::vault_learning::MeaningSource,
    ) -> crate::vault_learning::RecallResult {
        use crate::vault_learning::RecallMeaning;
        let Some(text) = self.meaning_query() else {
            let mut recall = self.recall_ranked(vault, scope, own, None);
            recall.meaning = if source.available() {
                RecallMeaning::NoVector
            } else {
                RecallMeaning::Off
            };
            return recall;
        };
        let meant = std::cell::Cell::new(RecallMeaning::NoVector);
        let lookup = |paths: &[String]| match source.cosines(&text, paths) {
            Ok(cosines) => {
                meant.set(RecallMeaning::Used);
                cosines
            }
            Err(why) => {
                meant.set(why);
                std::collections::BTreeMap::new()
            }
        };
        let mut recall = self.recall_ranked(vault, scope, own, Some(&lookup));
        recall.meaning = meant.get();
        recall
    }

    /// Ranks recall and records its usage (`memory_maintenance::record_recall`),
    /// unless the calling thread is inside [`unrecorded`].
    fn recall_ranked(
        &self,
        vault: &Path,
        scope: Option<&str>,
        own: &str,
        meaning: Option<crate::vault_learning::Meaning<'_>>,
    ) -> crate::vault_learning::RecallResult {
        let result = crate::vault_learning::recall_with_scope(
            vault,
            &self.selection_query(),
            &self.subject_paths(),
            crate::note_lifecycle::history_cue(own),
            scope,
            meaning,
        );
        if !RECALL_UNRECORDED.with(std::cell::Cell::get)
            && crate::memory_maintenance::record_recall(
                vault,
                &result.observed,
                &result.items,
                scope,
            )
            .is_err()
        {
            eprintln!("[memory] usage could not be recorded; saved facts were left unchanged");
        }
        result
    }

    /// What meaning-ranked recall embeds (OF26): exactly the text retrieval's
    /// semantic half embeds for this request read as its turn's query
    /// (`directory_connections::prioritized_context` strips injected context;
    /// `retrieval_broker` asks only when the query has search terms and takes
    /// its first [`crate::embed_model::QUERY_CHARS`] characters), so the one
    /// cached query vector serves both. `None` when there is nothing to
    /// embed: an empty host subject (OB18) included, which would otherwise
    /// embed Hollow's own check-in template.
    pub(crate) fn meaning_query(&self) -> Option<String> {
        let request = crate::sessions::strip_injected_context(&self.request);
        if crate::directory_connections::retrieval_terms(&request).is_empty() {
            return None;
        }
        Some(
            request
                .chars()
                .take(crate::embed_model::QUERY_CHARS)
                .collect(),
        )
    }

    /// The user's side of the exchange a successful turn learns from: the
    /// request without an ask-canvas's hidden answer block, and the notes it
    /// is about (named or resolved), which the extractor may target.
    pub(crate) fn learning_side(&self) -> (String, Vec<String>) {
        (crate::next_step::strip(&self.request), self.subject_paths())
    }

    /// This request read as `query` (overnight OB18, OB25: a host turn's
    /// subject, a short follow-up's query), with the same entities and
    /// antecedents, for recall, attached sources and connector selection.
    pub(crate) fn reading(&self, query: &str) -> Self {
        TurnRequest {
            request: query.to_string(),
            ..self.clone()
        }
    }

    /// What this request's retrieval reads in conversation `key`
    /// ([`RetrievalBasis::of`]; `resumed`: the turn resumes its native
    /// session). The conversation is read only for a short request that is
    /// not Hollow's own.
    pub(crate) fn basis_in(&self, vault: &Path, key: &str, resumed: bool) -> RetrievalBasis {
        let request = self.request.as_str();
        if crate::main_agent::host_subject(request).is_some()
            || terms(request).len() >= FOLLOWUP_TERMS
        {
            return RetrievalBasis::of(request, &[], None, resumed);
        }
        let earlier =
            crate::session_store::person_asks(vault, key, PRIOR_TURNS).unwrap_or_default();
        let goal = if earlier.is_empty() {
            None
        } else {
            crate::task_state::stored(vault, key)
                .and_then(|state| state.record.goal)
                .map(|goal| goal.text)
        };
        RetrievalBasis::of(request, &earlier, goal.as_deref(), resumed)
    }
}

thread_local! {
    /// Set by [`unrecorded`] while its closure runs on this thread.
    static RECALL_UNRECORDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `f` with recall's usage recording off on this thread (J3a review):
/// a chat turn's prefetch computes recall before the lease, may be thrown
/// away (a deferred admission) or replaced (past its cutoff), so the chat
/// path ranks inside this and records once, for the recall the turn uses
/// (`chat_queue::turn_inputs_prefetched`). Every other caller still
/// records as it ranks. The flag is restored when `f` returns or panics.
pub(crate) fn unrecorded<T>(f: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            RECALL_UNRECORDED.with(|flag| flag.set(self.0));
        }
    }
    let _restore = Restore(RECALL_UNRECORDED.with(|flag| flag.replace(true)));
    f()
}

/// A request with fewer retrieval terms than this is a follow-up, and an
/// earlier request with at least this many can be its prior (overnight
/// OB25).
pub(crate) const FOLLOWUP_TERMS: usize = 4;
/// A follow-up's prior is looked for in this many earlier requests.
pub(crate) const PRIOR_TURNS: usize = ANTECEDENT_TURNS;
/// Words that answer or go ahead ("yes, merge it", "continue"): they still
/// lead a follow-up's query, but never count as a new topic, so a resumed
/// "yes, merge it" skips retrieval (overnight OB25). Any other word the
/// prior's query lacks turns the skip off.
const GO_AHEAD: [&str; 24] = [
    "yes", "yeah", "yep", "yup", "sure", "okay", "thanks", "thank", "great", "good", "fine",
    "sounds", "continue", "proceed", "ahead", "merge", "ship", "push", "retry", "again", "next",
    "now", "then", "too",
];

/// What a chat turn's query was read from (overnight OB18, OB25), as the
/// manifest's `retrieval.queryBasis` records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueryBasis {
    /// The request itself, as before.
    Request,
    /// A wake's chats or a check-in's new signals (`main_agent::host_subject`).
    HostSubject,
    /// A host turn with an empty subject: nothing retrieved.
    SkippedHost,
    /// A short follow-up's terms, then its prior's.
    FollowupPrior,
    /// A resumed follow-up that adds nothing to its prior: nothing
    /// retrieved.
    SkippedFollowup,
}

impl QueryBasis {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            QueryBasis::Request => "request",
            QueryBasis::HostSubject => "hostSubject",
            QueryBasis::SkippedHost => "skipped-host",
            QueryBasis::FollowupPrior => "followup+prior",
            QueryBasis::SkippedFollowup => "skipped-followup",
        }
    }

    /// The turn retrieves nothing and keeps its conversation's last
    /// connector decision, with no Jev call (as a bare turn retrieves
    /// nothing).
    pub(crate) fn skips(self) -> bool {
        matches!(self, QueryBasis::SkippedHost | QueryBasis::SkippedFollowup)
    }

    /// A Hollow-authored turn's subject: part of its own request, so Jev
    /// may be sent it.
    pub(crate) fn is_host(self) -> bool {
        matches!(self, QueryBasis::HostSubject | QueryBasis::SkippedHost)
    }
}

/// What a chat turn's retrieval, recall and attached sources read
/// (overnight OB18, OB25): `query` in place of the request, or the request
/// itself (`None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RetrievalBasis {
    pub basis: QueryBasis,
    pub query: Option<String>,
}

impl RetrievalBasis {
    fn request() -> Self {
        RetrievalBasis {
            basis: QueryBasis::Request,
            query: None,
        }
    }

    /// This basis once the turn's resume plan is known (overnight R1
    /// review): a follow-up skips only when its native session really
    /// resumes (`resumes`); on a fresh one (a reset, a new session) it
    /// retrieves with its prior's terms.
    pub(crate) fn on_plan(self, resumes: bool) -> Self {
        if self.basis == QueryBasis::SkippedFollowup && !resumes {
            return RetrievalBasis {
                basis: QueryBasis::FollowupPrior,
                ..self
            };
        }
        self
    }

    /// [`TurnRequest::basis_in`] with the conversation given: `earlier`, its
    /// earlier non-host requests, newest first
    /// (`session_store::person_asks`), and its task-state goal. A wake or
    /// check-in reads its subject, and skips with an empty one. A short
    /// request after an earlier non-host turn borrows its prior: the newest
    /// earlier request with [`FOLLOWUP_TERMS`] terms, else the goal
    /// ([`retrieval_basis`]).
    pub(crate) fn of(request: &str, earlier: &[String], goal: Option<&str>, resumed: bool) -> Self {
        if let Some(subject) = crate::main_agent::host_subject(request) {
            return if terms(&subject).is_empty() {
                RetrievalBasis {
                    basis: QueryBasis::SkippedHost,
                    query: Some(String::new()),
                }
            } else {
                RetrievalBasis {
                    basis: QueryBasis::HostSubject,
                    query: Some(subject),
                }
            };
        }
        if earlier.is_empty() {
            return Self::request();
        }
        let prior = earlier
            .iter()
            .map(String::as_str)
            .find(|ask| terms(ask).len() >= FOLLOWUP_TERMS)
            .or(goal);
        retrieval_basis(request, prior, resumed)
    }
}

/// A short follow-up's basis (overnight OB25): a request with
/// [`FOLLOWUP_TERMS`] terms or more, or no prior, is read as itself;
/// otherwise its query is its own terms, then the prior's, capped at 16 by
/// retrieval's tokenizer. A resumed turn whose request adds no term the
/// prior's query lacks ([`GO_AHEAD`] words aside) skips retrieval: its
/// native session already holds that topic.
pub(crate) fn retrieval_basis(request: &str, prior: Option<&str>, resumed: bool) -> RetrievalBasis {
    let own = terms(request);
    let Some(prior) = prior.filter(|_| own.len() < FOLLOWUP_TERMS) else {
        return RetrievalBasis::request();
    };
    let held = terms(prior);
    if held.is_empty() {
        return RetrievalBasis::request();
    }
    let query = terms(&format!("{request}\n{prior}")).join(" ");
    let adds = own
        .iter()
        .any(|term| !held.contains(term) && !GO_AHEAD.contains(&term.as_str()));
    RetrievalBasis {
        basis: if resumed && !adds {
            QueryBasis::SkippedFollowup
        } else {
            QueryBasis::FollowupPrior
        },
        query: Some(query),
    }
}

/// Retrieval's terms of `text` (`directory_connections::retrieval_terms`).
pub(crate) fn terms(text: &str) -> Vec<String> {
    crate::directory_connections::retrieval_terms(text)
        .into_iter()
        .map(|term| term.text)
        .collect()
}

/// Note names of the record folders, read once per admission.
pub(crate) struct NoteIndex {
    /// (phrase to match, case-sensitive first letter, vault-relative path)
    names: Vec<(String, bool, String)>,
}

impl NoteIndex {
    /// Up to [`MAX_NOTES_PER_FOLDER`] markdown notes per record folder, to
    /// depth 2, in sorted order. A missing folder is simply empty.
    pub(crate) fn load(vault: &Path) -> Self {
        let mut names = Vec::new();
        for folder in ENTITY_FOLDERS {
            let root = vault.join(folder);
            if !root.is_dir() {
                continue;
            }
            let notes = walkdir::WalkDir::new(&root)
                .max_depth(2)
                .sort_by_file_name()
                .into_iter()
                .filter_map(Result::ok)
                .filter(|e| e.file_type().is_file())
                .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
                .take(MAX_NOTES_PER_FOLDER);
            for entry in notes {
                let Ok(rel) = entry.path().strip_prefix(vault) else {
                    continue;
                };
                let rel = rel.to_string_lossy().replace('\\', "/");
                let stem = display_name(&rel);
                if stem.chars().count() < 3 {
                    continue;
                }
                let capital = stem.chars().next().is_some_and(char::is_uppercase);
                names.push((lowercase(&stem), capital, rel));
            }
        }
        NoteIndex { names }
    }

    #[cfg(test)]
    fn of(names: &[&str]) -> Self {
        NoteIndex {
            names: names
                .iter()
                .map(|rel| {
                    let stem = display_name(rel);
                    let capital = stem.chars().next().is_some_and(char::is_uppercase);
                    (lowercase(&stem), capital, rel.to_string())
                })
                .collect(),
        }
    }
}

/// A note's name as a person would say it: the file stem, with `-` and `_`
/// as spaces.
fn display_name(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    let stem = file.strip_suffix(".md").unwrap_or(file);
    stem.replace(['-', '_'], " ").trim().to_string()
}

fn push_unique(out: &mut Vec<String>, id: String) {
    if !id.is_empty() && !out.contains(&id) {
        out.push(id);
    }
}

/// The explicit entities `text` names, in order: path-like tokens (a `/`
/// and a file extension, or a `.md` name, `vault:` allowed), then the record
/// notes it names. A note whose name starts with a capital matches only
/// where the text also capitalizes it, so a person called "Will" is not
/// every "will". Bounded by [`MAX_ENTITIES`].
pub(crate) fn explicit_entities(vault: &Path, notes: &NoteIndex, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in text.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '`' | '"' | '\'' | '(' | ')' | '<' | '>' | '[' | ']' | '{' | '}' | ',' | ';' | '|'
            )
    }) {
        let token = token.trim_end_matches(['.', ':', '!', '?']);
        if is_path_like(token) {
            if let Some(id) = normalize_path(vault, token) {
                push_unique(&mut out, id);
            }
        }
        if out.len() >= MAX_ENTITIES {
            return out;
        }
    }
    let lowered = Lowered::new(text);
    for (phrase, capital, rel) in &notes.names {
        if names_phrase(&lowered, phrase, *capital) {
            push_unique(&mut out, rel.clone());
        }
        if out.len() >= MAX_ENTITIES {
            break;
        }
    }
    out
}

fn is_path_like(token: &str) -> bool {
    if token.is_empty() || token.len() > 400 || token.contains("://") {
        return false;
    }
    let token = token.strip_prefix("vault:").unwrap_or(token);
    let last = token.rsplit('/').next().unwrap_or(token);
    let has_ext = last.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && (1..=8).contains(&ext.len())
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
            && ext.chars().any(|c| c.is_ascii_alphabetic())
    });
    has_ext && (token.contains('/') || last.ends_with(".md"))
}

/// A path token as an entity id: `vault:` and `./` dropped, an absolute path
/// inside the vault made vault-relative. `..` segments are refused.
fn normalize_path(vault: &Path, token: &str) -> Option<String> {
    let token = token.trim();
    let token = token.strip_prefix("vault:").unwrap_or(token);
    let token = token.strip_prefix("./").unwrap_or(token);
    if token.is_empty() || token.split('/').any(|seg| seg == "..") {
        return None;
    }
    let path = Path::new(token);
    if path.is_absolute() {
        if let Ok(rel) = path.strip_prefix(vault) {
            return Some(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Some(token.to_string())
}

/// `text` lowercased one character at a time. Note names and the text
/// they're found in ([`Lowered`]) both use it, so a word-final 'Σ' reads
/// the same in each.
fn lowercase(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}

/// `text` lowercased, with where each of its characters starts in the
/// lowercase text. Lowercasing can change a character's byte length ('İ'
/// U+0130 is 2 bytes and lowercases to the 3 of "i̇", 'ẞ' U+1E9E is 3 and
/// lowercases to the 2 of 'ß'), so a match in the lowercase text is mapped
/// back by character, never by byte offset, even when the totals agree.
struct Lowered<'a> {
    text: &'a str,
    lower: String,
    /// (offset in `lower`, offset in `text`) of each character, ascending.
    starts: Vec<(usize, usize)>,
}

impl<'a> Lowered<'a> {
    fn new(text: &'a str) -> Self {
        let mut lower = String::with_capacity(text.len());
        let mut starts = Vec::with_capacity(text.len());
        for (at, c) in text.char_indices() {
            starts.push((lower.len(), at));
            lower.extend(c.to_lowercase());
        }
        Lowered {
            text,
            lower,
            starts,
        }
    }

    /// The character of `text` whose lowercase starts at `lower_at`, if one
    /// does (`None` inside a character's expansion).
    fn original(&self, lower_at: usize) -> Option<char> {
        let i = self
            .starts
            .binary_search_by_key(&lower_at, |&(lower, _)| lower)
            .ok()?;
        self.text[self.starts[i].1..].chars().next()
    }

    /// The character of `text` just before the one whose lowercase starts
    /// at `lower_at` (or before the end, at the end).
    fn original_before(&self, lower_at: usize) -> Option<char> {
        let at = if lower_at == self.lower.len() {
            self.text.len()
        } else {
            let i = self
                .starts
                .binary_search_by_key(&lower_at, |&(lower, _)| lower)
                .ok()?;
            self.starts[i].1
        };
        self.text[..at].chars().next_back()
    }

    /// Whether `lower_at` is where a character's lowercase begins, or the end.
    fn on_char(&self, lower_at: usize) -> bool {
        lower_at == self.lower.len()
            || self
                .starts
                .binary_search_by_key(&lower_at, |&(lower, _)| lower)
                .is_ok()
    }
}

/// A character that continues a word: a letter or digit, or a combining
/// mark, which belongs to the character before it ("Atlas\u{307}" is not
/// "Atlas"). Std has no general-category table, so the marks are the
/// combining blocks every script shares.
fn word_char(c: char) -> bool {
    c.is_alphanumeric()
        || matches!(c,
            '\u{0300}'..='\u{036F}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE20}'..='\u{FE2F}')
}

/// Whether `text` says `phrase` (lowercase, [`lowercase`]) as whole words
/// of whole characters, capitalized where `capital` asks for it.
fn names_phrase(text: &Lowered<'_>, phrase: &str, capital: bool) -> bool {
    let lower = text.lower.as_str();
    let mut from = 0;
    while let Some(i) = lower[from..].find(phrase) {
        let start = from + i;
        let end = start + phrase.len();
        let first = text.original(start);
        // The neighbours come from the original text: 'İ' lowercases to
        // 'i' + U+0307, a mark rather than the letter that was there.
        let before = text.original_before(start);
        let after = text.original(end);
        let bounded = !before.is_some_and(word_char) && !after.is_some_and(word_char);
        let cased = !capital || first.is_some_and(char::is_uppercase);
        if first.is_some() && text.on_char(end) && bounded && cased {
            return true;
        }
        from = start + lower[start..].chars().next().map_or(1, char::len_utf8);
    }
    false
}

/// The kind of entity a reference phrase asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefKind {
    Project,
    Person,
    Goal,
    /// A file or note: any entity.
    File,
}

/// Reference phrases, matched as whole words. "this project" is left out on
/// purpose: in a workspace chat it means the workspace, not an earlier turn.
const REFERENCES: &[(&str, RefKind)] = &[
    ("that project", RefKind::Project),
    ("same project", RefKind::Project),
    ("previous project", RefKind::Project),
    ("that person", RefKind::Person),
    ("same person", RefKind::Person),
    ("that goal", RefKind::Goal),
    ("same goal", RefKind::Goal),
    ("that file", RefKind::File),
    ("same file", RefKind::File),
    ("previous file", RefKind::File),
    ("that note", RefKind::File),
    ("same note", RefKind::File),
    ("that doc", RefKind::File),
    ("that document", RefKind::File),
    ("same document", RefKind::File),
];

/// Reference phrases that say no kind of their own. They resolve only when
/// the request names a kind ([`KIND_NOUNS`]: "the project, that one", "the
/// same person as before"), and then only to an entity of that kind; on
/// their own ("format it as before") they resolve nothing (wave 1
/// follow-up: they used to take every entity of the earlier turn).
const GENERIC_REFERENCES: [&str; 3] = ["that one", "same one", "as before"];

/// The words that give a generic reference its kind, matched as whole
/// words. A file word asks for what "that file" does: any entity.
const KIND_NOUNS: &[(&str, RefKind)] = &[
    ("project", RefKind::Project),
    ("projects", RefKind::Project),
    ("person", RefKind::Person),
    ("people", RefKind::Person),
    ("goal", RefKind::Goal),
    ("goals", RefKind::Goal),
    ("file", RefKind::File),
    ("files", RefKind::File),
    ("note", RefKind::File),
    ("notes", RefKind::File),
    ("doc", RefKind::File),
    ("document", RefKind::File),
];

/// The references `lowered` says, each with the kind it asks for: the
/// phrases of [`REFERENCES`], then a generic phrase once per kind the
/// request names.
fn references_in(lowered: &Lowered) -> Vec<(&'static str, RefKind)> {
    let mut out: Vec<(&'static str, RefKind)> = REFERENCES
        .iter()
        .filter(|(phrase, _)| names_phrase(lowered, phrase, false))
        .copied()
        .collect();
    if let Some(generic) = GENERIC_REFERENCES
        .iter()
        .find(|phrase| names_phrase(lowered, phrase, false))
    {
        for (noun, kind) in KIND_NOUNS {
            if names_phrase(lowered, noun, false) && !out.iter().any(|(_, k)| k == kind) {
                out.push((generic, *kind));
            }
        }
    }
    out
}

fn kind_matches(kind: RefKind, entity: &str) -> bool {
    match kind {
        RefKind::Project => entity.starts_with("projects/"),
        RefKind::Person => entity.starts_with("people/"),
        RefKind::Goal => entity.starts_with("goals/"),
        RefKind::File => true,
    }
}

/// Whether `request` says a reference phrase that can resolve
/// ([`references_in`]).
pub(crate) fn refers_back(request: &str) -> bool {
    !references_in(&Lowered::new(request)).is_empty()
}

/// Resolve the request's references from the conversation's earlier
/// requests (`earlier`: `(ord, asked)`, newest first, as
/// `session_store::latest_asks` returns them). Deterministic and local:
///
/// - only when the request says a reference phrase ([`REFERENCES`], or a
///   generic one with a kind word: [`references_in`]) and names no entity
///   of that kind itself;
/// - at most [`ANTECEDENT_TURNS`] earlier requests, [`ANTECEDENT_TURN_CHARS`]
///   of each, read newest first, by the user only (never assistant replies,
///   injected blocks or tool output);
/// - the newest earlier request that named an entity of the kind wins, and
///   at most [`MAX_ANTECEDENTS`] are kept.
pub(crate) fn resolve_antecedents(
    vault: &Path,
    notes: &NoteIndex,
    request: &str,
    entities: &[String],
    earlier: &[(i64, String)],
) -> Vec<Antecedent> {
    let mut out: Vec<Antecedent> = Vec::new();
    let lowered = Lowered::new(request);
    let mut kinds_done = BTreeSet::new();
    for (phrase, kind) in references_in(&lowered) {
        let kind = &kind;
        if out.len() >= MAX_ANTECEDENTS {
            break;
        }
        if entities.iter().any(|e| kind_matches(*kind, e)) {
            continue;
        }
        if !kinds_done.insert(format!("{kind:?}")) {
            continue;
        }
        for (ord, asked) in earlier.iter().take(ANTECEDENT_TURNS) {
            let head: String = asked.chars().take(ANTECEDENT_TURN_CHARS).collect();
            let found: Vec<String> = explicit_entities(vault, notes, &head)
                .into_iter()
                .filter(|e| kind_matches(*kind, e))
                .collect();
            if found.is_empty() {
                continue;
            }
            for path in found {
                if out.len() >= MAX_ANTECEDENTS {
                    break;
                }
                if out.iter().any(|a| a.path == path) {
                    continue;
                }
                out.push(Antecedent {
                    path,
                    from_turn: *ord,
                    reference: phrase.to_string(),
                });
            }
            break;
        }
    }
    out
}

/// The paths of the trailing attachment blocks: an `<attached-images>`
/// block's `1. /path` lines and an `<attached-files>` block's
/// `1. name at /path` lines (C5, 2026-11-04). A file's name never holds a
/// `/` (the composer replaces it), so the path starts at the first one.
fn attached_paths(block: &str) -> Vec<String> {
    block
        .lines()
        .filter_map(|line| {
            let (n, rest) = line.trim().split_once(". ")?;
            let path = &rest[rest.find('/')?..];
            (!n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
                .then(|| path.trim().to_string())
        })
        .filter(|p| !p.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTENT: &str =
        "<hollow-chat-intent>\nInfer the appropriate approach.\n</hollow-chat-intent>\n\n";

    fn operator(body: &str) -> String {
        format!("<operator-context>\n{body}\n\n(Durable context about how the user works and wants you to respond. Apply it to every reply this session: match their communication style, honor their settled decisions and anti-patterns. Do not echo it back.)\n</operator-context>\n\n")
    }

    #[test]
    fn admission_splits_the_composed_prompt_once_and_keeps_the_blocks_apart() {
        let dir = tempfile::tempdir().unwrap();
        let composed = format!(
            "{}{INTENT}Summarize notes/plan.md please\n\n<attached-images count=\"1\">\nThe user attached 1 image to this message. Open it using your available image-viewing tool:\n1. {}/inbox/shot.png\n</attached-images>",
            operator("Always use Outlook."),
            dir.path().display()
        );
        let request = TurnRequest::admit(dir.path(), &composed, None);
        assert_eq!(request.request, "Summarize notes/plan.md please");
        assert_eq!(
            request.request,
            crate::sessions::strip_injected_context(&composed),
            "the request is exactly the turn's asked text"
        );
        assert_eq!(request.entities, vec!["notes/plan.md", "inbox/shot.png"]);
        let tags: Vec<&str> = request
            .context_blocks
            .iter()
            .map(|b| b.tag.as_str())
            .collect();
        assert_eq!(tags, vec!["operator-context", "hollow-chat-intent"]);
        // The block is referenced by size and hash, never copied: the
        // queued payload holds its text once, in the prompt.
        let split = crate::sessions::split_injected_context(&composed);
        let (_, operator_body) = &split.blocks[0];
        assert!(operator_body.starts_with("Always use Outlook."));
        assert_eq!(request.context_blocks[0].bytes, operator_body.len());
        assert_eq!(
            request.context_blocks[0].sha256,
            crate::run_manifest::sha256_hex(operator_body.as_bytes())
        );
        let payload = serde_json::to_string(&request).unwrap();
        assert!(!payload.contains("Always use Outlook."), "{payload}");
        assert!(request.antecedents.is_empty());
        assert_eq!(request.task_state, None);
    }

    #[test]
    fn explicit_entities_are_paths_and_named_record_notes() {
        let dir = tempfile::tempdir().unwrap();
        let notes = NoteIndex::of(&[
            "projects/active/Atlas.md",
            "people/Will.md",
            "goals/ship-hollow.md",
        ]);
        let found = explicit_entities(
            dir.path(),
            &notes,
            "Compare `vault:workspaces/w/outputs/report.md` with ./drafts/a.txt for Atlas, then ship hollow.",
        );
        assert_eq!(
            found,
            vec![
                "workspaces/w/outputs/report.md",
                "drafts/a.txt",
                "projects/active/Atlas.md",
                "goals/ship-hollow.md"
            ]
        );
        // A capitalized name needs a capital; version numbers, URLs and
        // `..` paths are not entities.
        assert!(explicit_entities(
            dir.path(),
            &notes,
            "I will check 0.5.13 at https://x.io/a.md"
        )
        .is_empty());
        assert_eq!(
            explicit_entities(dir.path(), &notes, "Ask Will"),
            vec!["people/Will.md"]
        );
        assert!(explicit_entities(dir.path(), &notes, "read ../secret/a.md").is_empty());
        // Absolute paths inside the vault become vault-relative.
        let inside = format!("open {}/notes/x.md", dir.path().display());
        assert_eq!(
            explicit_entities(dir.path(), &notes, &inside),
            vec!["notes/x.md"]
        );
    }

    /// Lowercasing can change a character's byte length: 'İ' (U+0130) is 2
    /// bytes and lowercases to "i̇" (3), 'ẞ' (U+1E9E) is 3 and lowercases
    /// to 'ß' (2). Byte offsets into the lowercased text are not offsets
    /// into the original, even when the two totals happen to agree.
    #[test]
    fn names_match_on_char_boundaries_when_lowercasing_changes_byte_lengths() {
        let dir = tempfile::tempdir().unwrap();
        let notes = NoteIndex::of(&[
            "people/Émile.md",
            "projects/active/Atlas.md",
            "people/Will.md",
        ]);
        // Equal totals: the capital check used to slice inside 'É' (panic)…
        assert_eq!(
            explicit_entities(dir.path(), &notes, "İ Émile ẞ"),
            vec!["people/Émile.md"]
        );
        // …read the capital one byte late (a miss)…
        assert_eq!(
            explicit_entities(dir.path(), &notes, "İ Atlas ẞ"),
            vec!["projects/active/Atlas.md"]
        );
        // …or took a lowercase word for a capitalized name.
        assert!(explicit_entities(dir.path(), &notes, "İ wIll ẞ").is_empty());
        // Unequal totals: the capital check used to be skipped altogether.
        assert!(explicit_entities(dir.path(), &notes, "İstanbul: I will check").is_empty());
        assert_eq!(
            explicit_entities(dir.path(), &notes, "İstanbul: ask Will"),
            vec!["people/Will.md"]
        );
    }

    /// #386 review: the characters around a match are read from the
    /// original text. 'İ' lowercases to 'i' + U+0307 (a combining dot), so
    /// the lowercase text put a mark, not a letter, in front of "atlas" in
    /// "İAtlas". And a combining mark belongs to the word it follows, so
    /// "Atlas\u{307}" is a different word, not "Atlas" and some punctuation.
    #[test]
    fn the_characters_around_a_match_are_read_from_the_original_text() {
        let dir = tempfile::tempdir().unwrap();
        let notes = NoteIndex::of(&["projects/active/Atlas.md", "people/émile.md"]);
        for glued in [
            "İAtlas is due",
            "Atlas\u{307} is due",
            "e\u{301}Atlas is due",
            "ask about XİAtlas",
            "see émile\u{301}",
        ] {
            assert!(
                explicit_entities(dir.path(), &notes, glued).is_empty(),
                "{glued:?} names no note"
            );
        }
        // A whole word next to 'İ' or a mark that ends another word still counts.
        assert_eq!(
            explicit_entities(dir.path(), &notes, "İ Atlas, e\u{301} done"),
            vec!["projects/active/Atlas.md"]
        );
        assert_eq!(
            explicit_entities(dir.path(), &notes, "ask émile, İ"),
            vec!["people/émile.md"]
        );
        assert!(!refers_back("İthat project again"));
        assert!(!refers_back("that project\u{307}"));
        assert!(refers_back("İ: that project again"));
    }

    #[test]
    fn entities_are_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let text: String = (0..20).map(|i| format!("a/{i}.md ")).collect();
        let found = explicit_entities(dir.path(), &NoteIndex::of(&[]), &text);
        assert_eq!(found.len(), MAX_ENTITIES);
    }

    #[test]
    fn a_reference_resolves_to_the_newest_earlier_entity_of_its_kind() {
        let dir = tempfile::tempdir().unwrap();
        let notes = NoteIndex::of(&["projects/active/Atlas.md", "projects/active/Borealis.md"]);
        let earlier = vec![
            (4, "thanks, looks good".to_string()),
            (
                3,
                "Summarize the Atlas project and notes/atlas.md".to_string(),
            ),
            (2, "Summarize the Borealis project".to_string()),
        ];
        let found = resolve_antecedents(
            dir.path(),
            &notes,
            "Do the same for that project",
            &[],
            &earlier,
        );
        assert_eq!(
            found,
            vec![Antecedent {
                path: "projects/active/Atlas.md".into(),
                from_turn: 3,
                reference: "that project".into(),
            }]
        );
        // A file reference takes any entity of the newest turn that has one.
        let file = resolve_antecedents(dir.path(), &notes, "open that file", &[], &earlier);
        let paths: Vec<&str> = file.iter().map(|a| a.path.as_str()).collect();
        assert_eq!(paths, vec!["notes/atlas.md", "projects/active/Atlas.md"]);
        // No reference phrase, or the request names its own project: nothing.
        assert!(resolve_antecedents(dir.path(), &notes, "Do the same", &[], &earlier).is_empty());
        assert!(resolve_antecedents(
            dir.path(),
            &notes,
            "Do that project like Borealis",
            &["projects/active/Borealis.md".into()],
            &earlier
        )
        .is_empty());
    }

    /// Wave 1 follow-up (red first): "that one", "same one" and "as before"
    /// say no kind of their own, so they resolve only when the request names
    /// the kind ("the project, that one"), and then only to an entity of that
    /// kind; on their own ("format it as before") they resolve nothing.
    #[test]
    fn generic_references_need_an_entity_of_the_kind_the_request_names() {
        let dir = tempfile::tempdir().unwrap();
        let notes = NoteIndex::of(&["projects/active/Atlas.md", "people/Robin.md"]);
        let earlier = vec![(
            2,
            "Ask Robin about the Atlas project and notes/atlas.md".to_string(),
        )];
        let resolve =
            |request: &str| resolve_antecedents(dir.path(), &notes, request, &[], &earlier);
        assert!(
            resolve("Format the summary as before").is_empty(),
            "a phrase with no kind pulled in every entity"
        );
        assert!(resolve("Do that one too").is_empty());
        let paths = |request: &str| -> Vec<String> {
            resolve(request).into_iter().map(|a| a.path).collect()
        };
        assert_eq!(
            paths("Run it on the project as before"),
            ["projects/active/Atlas.md"]
        );
        assert_eq!(paths("Message the person, that one"), ["people/Robin.md"]);
        assert!(
            paths("Open the file, same one as before").contains(&"notes/atlas.md".to_string()),
            "a file reference, like \"that file\""
        );
    }

    #[test]
    fn the_resolver_is_bounded_in_turns_and_characters() {
        let dir = tempfile::tempdir().unwrap();
        let notes = NoteIndex::of(&["projects/active/Atlas.md"]);
        // Named only in the ninth-newest request: out of reach.
        let mut earlier: Vec<(i64, String)> = (0..ANTECEDENT_TURNS as i64)
            .map(|i| (20 - i, "ok".to_string()))
            .collect();
        earlier.push((1, "the Atlas project".into()));
        assert!(resolve_antecedents(dir.path(), &notes, "that project", &[], &earlier).is_empty());
        // Named past the character cap of its request: out of reach.
        let long = format!("{} the Atlas project", "x ".repeat(ANTECEDENT_TURN_CHARS));
        assert!(
            resolve_antecedents(dir.path(), &notes, "that project", &[], &[(1, long)]).is_empty()
        );
    }

    #[test]
    fn selection_names_antecedents_without_their_folders() {
        let request = TurnRequest {
            request: "Do the same for that project".into(),
            antecedents: vec![Antecedent {
                path: "projects/active/Atlas-site.md".into(),
                from_turn: 1,
                reference: "that project".into(),
            }],
            entities: vec!["notes/a.md".into()],
            ..TurnRequest::default()
        };
        assert_eq!(
            request.selection_query(),
            "Do the same for that project\nAtlas site"
        );
        let query = request.connector_query();
        assert_eq!(query.request, "Do the same for that project");
        assert_eq!(query.local, request.selection_query());
        assert_eq!(
            request.subject_paths(),
            vec!["notes/a.md", "projects/active/Atlas-site.md"]
        );
    }

    /// Overnight OB25: a follow-up's query is its own terms, then its
    /// prior's, capped at 16 by retrieval's tokenizer; the task-state goal
    /// is the prior when no earlier request has four terms; a full request,
    /// or no earlier non-host turn, reads as the request. Resumed, a new
    /// word keeps retrieval on and a go-ahead skips it.
    #[test]
    fn a_follow_up_borrows_its_priors_terms_up_to_sixteen() {
        let long = (1..=17)
            .map(|i| format!("term{i:02}"))
            .collect::<Vec<_>>()
            .join(" ");
        let prior = Some(long.as_str());
        let basis = retrieval_basis("what about Dana?", prior, false);
        assert_eq!(basis.basis, QueryBasis::FollowupPrior);
        let query = basis.query.unwrap_or_default();
        assert!(query.starts_with("dana term01 term02 "), "{query}");
        assert_eq!(terms(&query).len(), 16);
        assert!(query.ends_with(" term15"), "{query}");

        let goal = Some("Ship the invoices export");
        let earlier = ["ok thanks".to_string(), "Fix it".to_string()];
        let basis = RetrievalBasis::of("continue", &earlier, goal, false);
        assert_eq!(
            basis.query.as_deref(),
            Some("continue ship invoices export")
        );
        let alone = RetrievalBasis::of("continue", &[], goal, true);
        assert_eq!(alone, RetrievalBasis::request());
        let full = retrieval_basis("Draft the Atlas weekly report", prior, true);
        assert_eq!(full, RetrievalBasis::request());

        let topic = retrieval_basis("now the invoices", prior, true);
        assert_eq!(topic.basis, QueryBasis::FollowupPrior);
        let ahead = retrieval_basis("yes, continue", prior, true);
        assert_eq!(ahead.basis, QueryBasis::SkippedFollowup);
        assert!(ahead.basis.skips() && !ahead.basis.is_host());
    }

    /// Overnight R1 review (S4): a follow-up's recall matches on its basis
    /// query, but whether it asks about the past is read from its own words
    /// (F7): a prior that asked about history doesn't make it historical.
    #[test]
    fn a_follow_ups_recall_reads_history_from_its_own_words() {
        use crate::note_lifecycle::fixture::{superseded_learning_vault, HISTORY_Q, OLD_CADENCE};
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        superseded_learning_vault(v);
        let own = "and the reports now?";
        // A basis query that carries a history cue (the raw join, so the
        // cue survives whatever the basis tokenizer keeps).
        let query = format!("{own} {HISTORY_Q}");
        assert!(crate::note_lifecycle::history_cue(&query).is_some());
        assert!(crate::note_lifecycle::history_cue(own).is_none());
        let request = TurnRequest::from_text(v, own);
        let read = request.reading(&query);
        let paths = |cue: &str| -> Vec<String> {
            read.recall_cued_by(v, None, cue)
                .items
                .into_iter()
                .map(|i| i.path)
                .collect()
        };
        assert!(!paths(own).contains(&OLD_CADENCE.to_string()));
        // Read from the basis query instead, it would have been historical.
        assert!(paths(&query).contains(&OLD_CADENCE.to_string()));
    }

    /// A stand-in meaning source (OF26, no model): records each text it is
    /// asked to embed and gives every learning the same close cosine; or,
    /// `off`, is unavailable (semantic search off) and answers nothing.
    struct StandIn {
        embedded: std::cell::RefCell<Vec<String>>,
        off: bool,
    }

    use std::collections::BTreeMap;

    impl crate::vault_learning::MeaningSource for StandIn {
        fn available(&self) -> bool {
            !self.off
        }

        fn cosines(
            &self,
            text: &str,
            paths: &[String],
        ) -> Result<BTreeMap<String, f32>, crate::vault_learning::RecallMeaning> {
            if self.off {
                return Err(crate::vault_learning::RecallMeaning::Off);
            }
            self.embedded.borrow_mut().push(text.to_string());
            Ok(paths.iter().map(|path| (path.clone(), 0.9)).collect())
        }
    }

    /// Red first (OF26 second review): a turn that retrieves nothing
    /// recorded `recallMeaning: "skipped"` even with semantic search off,
    /// where the manifest says nothing. Now it is `Skipped` only from a
    /// source that could have answered; either way nothing is embedded and
    /// the recall is the stem one. A request with nothing to embed is
    /// likewise `NoVector` only from an available source.
    #[test]
    fn a_skipped_recall_says_skipped_only_when_meaning_was_available() {
        use crate::vault_learning::RecallMeaning;
        let dir = fixtures::learned_vault();
        let v = dir.path();
        let json = |r: &crate::vault_learning::RecallResult| serde_json::to_string(r).unwrap();
        let read = TurnRequest::from_text(v, "Which mail client do we use?");
        let on = StandIn {
            embedded: Default::default(),
            off: false,
        };
        let off = StandIn {
            embedded: Default::default(),
            off: true,
        };
        let skipped = read.recall_skipping_meaning(v, None, &read.request, &on);
        assert_eq!(skipped.meaning, RecallMeaning::Skipped);
        let quiet = read.recall_skipping_meaning(v, None, &read.request, &off);
        assert_eq!(quiet.meaning, RecallMeaning::Off);
        assert_eq!(quiet.meaning.recorded(), None);
        assert!(on.embedded.borrow().is_empty());
        assert_eq!(json(&skipped), json(&quiet));
        let plain = read.recall_cued_by(v, None, &read.request);
        assert_eq!(json(&skipped), json(&plain));
        // Semantic search is off in tests: the app's source says `Off` too.
        let app = read.recall_without_meaning(v, None, &read.request);
        assert_eq!(app.meaning, RecallMeaning::Off);

        let nothing = TurnRequest::from_text(v, "");
        assert_eq!(nothing.meaning_query(), None);
        let meant = nothing.recall_meant(v, None, &nothing.request, &on);
        assert_eq!(meant.meaning, RecallMeaning::NoVector);
        let meant = nothing.recall_meant(v, None, &nothing.request, &off);
        assert_eq!(meant.meaning, RecallMeaning::Off);
        let meant = read.recall_meant(v, None, &read.request, &off);
        assert_eq!(meant.meaning, RecallMeaning::Off);
    }

    /// J3a review: `unrecorded` turns usage recording off only while its
    /// closure runs, nested or not, and leaves it as it found it.
    #[test]
    fn unrecorded_is_scoped_to_its_closure() {
        let off = || RECALL_UNRECORDED.with(std::cell::Cell::get);
        assert!(!off());
        let inner = unrecorded(|| {
            assert!(off());
            unrecorded(off)
        });
        assert!(inner);
        assert!(!off(), "restored after the outer call");
    }

    /// J3a: a turn whose prefetched recall missed its cutoff ranks by stems
    /// without embedding anything, exactly as a recall whose vector never
    /// came, and records `timedOut` only when meaning could have been used.
    #[test]
    fn a_recall_past_its_cutoff_ranks_by_stems_and_says_timed_out() {
        use crate::vault_learning::RecallMeaning;
        let dir = fixtures::learned_vault();
        let v = dir.path();
        let json = |r: &crate::vault_learning::RecallResult| serde_json::to_string(r).unwrap();
        let read = TurnRequest::from_text(v, "Which mail client do we use?");
        let on = StandIn {
            embedded: Default::default(),
            off: false,
        };
        let off = StandIn {
            embedded: Default::default(),
            off: true,
        };
        let late = read.recall_late_with(v, None, &read.request, &on);
        assert_eq!(late.meaning, RecallMeaning::TimedOut);
        assert_eq!(late.meaning.recorded().as_deref(), Some("timedOut"));
        assert!(on.embedded.borrow().is_empty(), "nothing embedded");
        let stems = read.recall_skipping_meaning(v, None, &read.request, &on);
        assert_eq!(json(&late), json(&stems));
        assert_eq!(
            read.recall_late_with(v, None, &read.request, &off).meaning,
            RecallMeaning::Off
        );
        let nothing = TurnRequest::from_text(v, "");
        assert_eq!(
            nothing
                .recall_late_with(v, None, &nothing.request, &on)
                .meaning,
            RecallMeaning::NoVector
        );
        // Semantic search is off in tests: the app's source says `Off`.
        let app = read.recall_past_cutoff(v, None, &read.request);
        assert_eq!(app.meaning, RecallMeaning::Off);
        assert_eq!(json(&app), json(&stems));
    }

    /// OF26: a turn's recall embeds the query retrieval embeds (its basis,
    /// cut as retrieval cuts it), not the raw request. A check-in with
    /// nothing new (an empty host subject, OB18) embeds nothing, so its
    /// recall is exactly the vector-free one; otherwise it would embed
    /// Hollow's own template. A wake embeds its subject, exactly the text
    /// retrieval embeds for it (review: tied to the broker's code, not just
    /// to `meaning_query`), and a vector then moves a global learning's
    /// score. Each recall says whether it used meaning.
    #[test]
    fn a_host_turn_with_an_empty_subject_embeds_nothing_and_recalls_as_before() {
        use crate::vault_learning::RecallMeaning;
        let dir = fixtures::learned_vault();
        let v = dir.path();
        let json = |r: &crate::vault_learning::RecallResult| serde_json::to_string(r).unwrap();
        let quiet = format!(
            "{} Nothing new came up since the last check-in.\n\nDecide what needs the person.",
            "⟲ Hollow: check-in at 10:00."
        );
        let basis = RetrievalBasis::of(&quiet, &[], None, true);
        assert_eq!(basis.basis, QueryBasis::SkippedHost);
        let request = TurnRequest::from_text(v, &quiet);
        assert!(request.meaning_query().is_some(), "the template has terms");
        let read = request.reading(basis.query.as_deref().unwrap_or_default());
        assert_eq!(read.meaning_query(), None);
        let source = StandIn {
            embedded: Default::default(),
            off: false,
        };
        let meant = read.recall_meant(v, None, &read.request, &source);
        assert!(source.embedded.borrow().is_empty());
        assert_eq!(meant.meaning, RecallMeaning::NoVector);
        let plain = read.recall_cued_by(v, None, &read.request);
        assert_eq!(json(&meant), json(&plain));

        let wake = format!(
            "{}\n\n- “Fix the Notion sync” in Launch (claude, session s-of26-child): done",
            crate::main_agent::WAKE_MESSAGE
        );
        let basis = RetrievalBasis::of(&wake, &[], None, true);
        assert_eq!(basis.basis, QueryBasis::HostSubject);
        let subject = basis.query.clone().unwrap_or_default();
        let read = TurnRequest::from_text(v, &wake).reading(&subject);
        let meant = read.recall_meant(v, None, &read.request, &source);
        let embedded = source.embedded.borrow().clone();
        assert_eq!(embedded, vec![read.meaning_query().unwrap()]);
        // What retrieval embeds for this turn: `prioritized_context` strips
        // the basis query, then the broker cuts it to its first 2,000
        // characters (`embed_model::tests::recall_and_retrieval_cut_a_query_alike`
        // pins that cut).
        let strip = "let request = crate::sessions::strip_injected_context(&query);";
        assert!(include_str!("directory_connections.rs").contains(strip));
        let retrieved: String = crate::sessions::strip_injected_context(&subject)
            .chars()
            .take(crate::embed_model::QUERY_CHARS)
            .collect();
        assert_eq!(embedded, vec![retrieved]);
        assert!(embedded[0].contains("Notion sync"));
        assert!(!embedded[0].contains("PLAN.md"));
        assert_eq!(meant.meaning, RecallMeaning::Used);
        let plain = read.recall_cued_by(v, None, &read.request);
        // Semantic search is off in tests: the app's source is never asked.
        assert_eq!(plain.meaning, RecallMeaning::Off);
        let score = |r: &crate::vault_learning::RecallResult| {
            r.items
                .iter()
                .find(|i| i.statement == fixtures::OUTLOOK)
                .map(|i| i.score)
        };
        assert!(score(&meant) > score(&plain), "{:?}", meant.items);

        // Retrieval reads at most the first 2,000 characters; so does recall.
        let long = TurnRequest::from_text(v, &"deploy ".repeat(400));
        let text = long.meaning_query().unwrap_or_default();
        assert_eq!(text.chars().count(), crate::embed_model::QUERY_CHARS);
    }

    /// Overnight R1's $0 counts (OB18, OB25): [`RetrievalBasis::of`] over
    /// every turn of a Sessions DB snapshot, in memory, printing counts only
    /// (never request text). A run resumed when its chat usage row's native
    /// session is the previous run's. The task-state goal is not read, so a
    /// follow-up that only a goal would serve stays on its request here.
    /// `HOLLOW_R1_DB=<snapshot> cargo test --lib r1_basis_counts -- --ignored --nocapture`
    #[test]
    #[ignore = "reads a Sessions DB snapshot: HOLLOW_R1_DB=<file>"]
    fn r1_basis_counts_on_a_snapshot() {
        let Ok(path) = std::env::var("HOLLOW_R1_DB") else {
            eprintln!("HOLLOW_R1_DB is not set: nothing counted");
            return;
        };
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY;
        let conn = rusqlite::Connection::open_with_flags(&path, flags).unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT t.session_key, t.asked, t.asked_by, t.run_id IS NOT NULL, \
                 (SELECT u.native_session FROM usage_ledger u WHERE u.run_id = t.run_id \
                 AND u.surface = 'chat' ORDER BY u.id LIMIT 1) \
                 FROM turns t WHERE t.asked IS NOT NULL ORDER BY t.session_key, t.ord",
            )
            .unwrap();
        type Row = (String, String, Option<String>, bool, Option<String>);
        let turns: Vec<Row> = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let (mut runs, mut host, mut empty, mut person) = (0usize, 0usize, 0usize, 0usize);
        let (mut short_before, mut short_after) = (0usize, 0usize);
        let (mut borrowed, mut skipped) = (0usize, 0usize);
        let (mut before, mut after) = (Vec::new(), Vec::new());
        let mut earlier: Vec<String> = Vec::new();
        let mut last_native: Option<String> = None;
        let mut key = "";
        for (session, asked, asked_by, ran, native) in &turns {
            if session.as_str() != key {
                key = session.as_str();
                earlier.clear();
                last_native = None;
            }
            let resumed = native.is_some() && *native == last_native;
            let hollow = asked_by
                .as_deref()
                .is_some_and(|by| by.starts_with("hollow:"));
            if *ran {
                runs += 1;
                let basis = RetrievalBasis::of(asked, &earlier, None, resumed);
                let own = terms(asked).len();
                let read = basis
                    .query
                    .as_deref()
                    .map_or(own, |query| terms(query).len());
                let skips = basis.basis.skips();
                if crate::main_agent::host_subject(asked).is_some() {
                    host += 1;
                    empty += usize::from(skips);
                    before.push(own);
                    after.push(if skips { 0 } else { read });
                } else if !hollow {
                    person += 1;
                    short_before += usize::from(own < FOLLOWUP_TERMS);
                    short_after += usize::from(!skips && read < FOLLOWUP_TERMS);
                    borrowed += usize::from(basis.basis == QueryBasis::FollowupPrior);
                    skipped += usize::from(skips);
                }
            }
            if !hollow && !crate::main_agent::is_wake_message(asked) {
                earlier.insert(0, asked.clone());
                earlier.truncate(PRIOR_TURNS);
            }
            if native.is_some() {
                last_native = native.clone();
            }
        }
        let median = |mut counts: Vec<usize>| {
            counts.sort_unstable();
            counts.get(counts.len() / 2).copied().unwrap_or(0)
        };
        println!("R1 counts: {} turns, {runs} chat runs", turns.len());
        println!(
            "OB18: {host} host runs (wakes, check-ins); {empty} empty subjects (retrievals and \
             Jev calls skipped); median query terms {} before, {} after",
            median(before),
            median(after)
        );
        let share = 100.0 * skipped as f64 / short_before.max(1) as f64;
        println!(
            "OB25: {person} non-host runs; {short_before} on 3 or fewer terms before, \
             {short_after} after; {borrowed} borrow a prior; {skipped} skipped \
             ({share:.1}% of the short ones)"
        );
    }

    #[test]
    fn note_index_reads_record_folders_to_depth_two() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        std::fs::create_dir_all(v.join("projects/active/deep")).unwrap();
        std::fs::create_dir_all(v.join("people")).unwrap();
        std::fs::write(v.join("projects/active/Atlas.md"), "").unwrap();
        std::fs::write(v.join("projects/active/deep/Hidden.md"), "").unwrap();
        std::fs::write(v.join("people/Jo.md"), "").unwrap();
        std::fs::write(v.join("people/Robin.md"), "").unwrap();
        let notes = NoteIndex::load(v);
        let rels: Vec<&str> = notes.names.iter().map(|(_, _, r)| r.as_str()).collect();
        assert_eq!(rels, vec!["projects/active/Atlas.md", "people/Robin.md"]);
    }

    #[test]
    fn a_payload_without_the_fields_reads_as_plain_text() {
        let old: TurnRequest =
            serde_json::from_value(serde_json::json!({"request": "hi"})).unwrap();
        assert_eq!(old.request, "hi");
        assert!(old.entities.is_empty() && old.context_blocks.is_empty());
        // A row queued while blocks carried their body still reads.
        let copied: TurnRequest = serde_json::from_value(serde_json::json!({
            "request": "hi",
            "contextBlocks": [{ "tag": "page-context", "body": "Chat tab" }]
        }))
        .unwrap();
        assert_eq!(copied.context_blocks[0].tag, "page-context");
        assert_eq!(copied.context_blocks[0].bytes, 0);
        let json = serde_json::to_value(TurnRequest {
            request: "hi".into(),
            ..TurnRequest::default()
        })
        .unwrap();
        assert_eq!(json, serde_json::json!({"request": "hi"}));
    }

    /// F7: whether recall treats a request as historical is read from the
    /// request's own words, as retrieval reads it; a resolved antecedent's
    /// name can't make a current-state question historical.
    #[test]
    fn recall_reads_history_from_the_request_alone() {
        use crate::note_lifecycle::fixture::{
            superseded_learning_vault, HISTORY_Q, NOW_Q, OLD_CADENCE,
        };
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        superseded_learning_vault(v);
        let paths = |text: &str| -> Vec<String> {
            TurnRequest::from_text(v, text)
                .recall(v)
                .items
                .into_iter()
                .map(|i| i.path)
                .collect()
        };
        assert!(paths(HISTORY_Q).contains(&OLD_CADENCE.to_string()));
        assert!(!paths(NOW_Q).contains(&OLD_CADENCE.to_string()));
        let named = TurnRequest {
            request: NOW_Q.into(),
            antecedents: vec![Antecedent {
                path: "notes/Cadence history before it changed.md".into(),
                from_turn: 1,
                reference: "that note".into(),
            }],
            ..TurnRequest::default()
        };
        assert!(named.selection_query().contains("history"));
        let recalled: Vec<String> = named.recall(v).items.into_iter().map(|i| i.path).collect();
        assert!(
            !recalled.contains(&OLD_CADENCE.to_string()),
            "a resolved name is not the request: {recalled:?}"
        );
    }
}

/// A vault with approved learnings for the F2 regression tests (chat,
/// workflows): one about the Atlas project, one about Borealis, one global
/// operator preference; and two "when relevant" connectors.
#[cfg(test)]
pub(crate) mod fixtures {
    use crate::vault_learning::{resolve_proposal, submit_proposal, LearningProposalPayload};
    use std::fs;

    pub(crate) const ATLAS: &str = "Reports go out on Fridays.";
    pub(crate) const BOREALIS: &str = "Budget reviews need finance sign-off.";
    pub(crate) const OUTLOOK: &str = "Always use Outlook for email.";

    /// (id, name, description) of two library connectors.
    pub(crate) const CONNECTORS: [(&str, &str, &str); 2] = [
        ("plaud", "Plaud", "Meeting recordings and transcripts"),
        ("notion", "Notion", "Pages and databases"),
    ];

    fn learn(vault: &std::path::Path, kind: &str, target: &str, subject: &str, statement: &str) {
        let proposal = submit_proposal(
            vault,
            LearningProposalPayload {
                kind: if kind == "operator" {
                    "preference"
                } else {
                    "project"
                }
                .into(),
                target_kind: kind.into(),
                target_path: target.into(),
                subject: subject.into(),
                statement: statement.into(),
                rationale: "fixture".into(),
                evidence_excerpt: String::new(),
                source_run_id: None,
                possible_duplicate: None,
                reject_reason: None,
                batch_id: None,
                learned_path: None,
                edits_path: None,
                edits_hash: None,
                learned_line: None,
                already_in: None,
                retires: None,
                retires_note: None,
            },
        )
        .unwrap();
        resolve_proposal(vault, &proposal.id, "accept", None).unwrap();
    }

    pub(crate) fn learned_vault() -> tempfile::TempDir {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        fs::create_dir_all(v.join("projects/active")).unwrap();
        fs::create_dir_all(v.join("me")).unwrap();
        fs::write(v.join("projects/active/Atlas.md"), "# Atlas\n").unwrap();
        fs::write(v.join("projects/active/Borealis.md"), "# Borealis\n").unwrap();
        fs::write(v.join("me/Guardrails.md"), "# Guardrails\n").unwrap();
        learn(
            v,
            "project",
            "projects/active/Atlas.md",
            "release cadence",
            ATLAS,
        );
        learn(
            v,
            "project",
            "projects/active/Borealis.md",
            "budget",
            BOREALIS,
        );
        // A global operator learning saved before accepts went into `me/`
        // (J2): still in `learnings/`, still recalled for every request.
        fs::create_dir_all(v.join("learnings")).unwrap();
        fs::write(
            v.join("learnings/email-platform-fixture.md"),
            format!(
                "---\ntype: learned-context\ntitle: {OUTLOOK}\ntarget_kind: operator\ntarget_path: me/Guardrails.md\nsubject: email platform\ntrust: approved\nproposal_id: review-fixture\nlearned_at: {}\n---\n\n{OUTLOOK}\n",
                chrono::Utc::now().to_rfc3339()
            ),
        )
        .unwrap();
        dir
    }

    /// The recalled statements, sorted.
    pub(crate) fn statements(recall: &crate::vault_learning::RecallResult) -> Vec<String> {
        let mut out: Vec<String> = recall.items.iter().map(|i| i.statement.clone()).collect();
        out.sort();
        out
    }

    /// Which fixture connectors the local rules attach for `query`.
    pub(crate) fn local_selection(query: &str) -> Vec<&'static str> {
        CONNECTORS
            .iter()
            .filter(|(id, name, description)| {
                crate::context_rank::locally_relevant(query, id, name, description)
            })
            .map(|(id, _, _)| *id)
            .collect()
    }

    /// The composer's operator block around `body` (`ChatBody.tsx`).
    pub(crate) fn operator_block(body: &str) -> String {
        format!("<operator-context>\n{body}\n\n(Durable context about how the user works and wants you to respond. Apply it to every reply this session: match their communication style, honor their settled decisions and anti-patterns. Do not echo it back.)\n</operator-context>\n\n")
    }

    pub(crate) const INTENT_BLOCK: &str = "<hollow-chat-intent>\nInfer the appropriate approach from the user's current request: answer, act, review, or automate as needed.\n</hollow-chat-intent>\n\n";

    /// Profile prose that shares words with Borealis's learning and Plaud's
    /// description, but has nothing to do with the request.
    pub(crate) const NOISY_PROFILE: &str =
        "I care about Borealis budget reviews and finance. I keep meeting recordings and transcripts.";
    pub(crate) const PLAIN_PROFILE: &str = "I write short emails.";
}
