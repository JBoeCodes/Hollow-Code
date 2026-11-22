//! The host retrieval broker (agent effectiveness F3, wave 2 lane S): the one
//! place that decides which evidence a query gets, so candidates, ranking,
//! scope and provenance can't drift between the routes an agent has.
//!
//! Callers:
//! - automatic retrieval for chats and automations
//!   (`directory_connections::prioritized_context`, in the app);
//! - `search_vault` (`vault_mcp`, in its own MCP subprocess);
//! - the app actions `vault.vault_search`, `vault.semantic_search` and
//!   `vault.retrieve`, and the Tauri commands behind the first two.
//!
//! The pipeline, for one query:
//!
//! 1. **Scope** ([`resolve_scope`]): the connected project a run's folder
//!    belongs to, from the folder's workspace record (a connected project's
//!    workspace), a build workspace's bound repository, the project folder
//!    that contains it, or the project a linked git worktree was created
//!    from. Never from the folder's name; a project's workspace whose
//!    project left the index keeps its own id (no connected-source
//!    evidence), so scope never widens by accident. The vault itself is
//!    never scoped.
//! 2. **Candidates.** The lexical terms are `directory_connections::
//!    retrieval_terms` (≤ 16). Connected projects: name matches first, then
//!    the most recently changed, ≤ 4,000 files and 32 MB read. Vault (OB12):
//!    with the FTS index, BM25's best 200 eligible notes over the ORed terms
//!    (prefix from 3 characters; under 200, mid-word trigram matches too)
//!    plus notes whose stem names a term, the 20 newest and any the index
//!    can't vouch for; without one, or with a kinds filter, every eligible
//!    note (the linear scan).
//!    Newest first, ≤ 3 lines per note; the 32 MB scoring budget is charged
//!    only for notes that match (`vault_search::retrieval_scan_with`), and a
//!    256 MB read limit bounds a linear scan. Semantic (optional): when
//!    semantic search is on and its index is open in this process with
//!    embedded notes, the request is embedded on this Mac (bundled model, no
//!    network, no download) and the best chunk of up to [`SEMANTIC_NOTES`]
//!    eligible notes at or above [`SEMANTIC_MIN`] joins as candidates.
//!    Org (CX5b): while the org clone's root is set, its Markdown and text
//!    files, kind `org`, cited by their path in the clone and ranked like
//!    project files (`context_layers::org_passages`, its own read budget).
//! 3. **Rank the whole candidate set.** A vault-only set is ordered by each
//!    note's BM25 band (OB12: one band per factor of 1.25 below the best
//!    score; all 0 on a linear scan), then coverage, in-line coverage and
//!    newest. With any project passage the order is the pre-OB12 one:
//!    coverage, then in-line coverage, the band only a tiebreak, then
//!    (inside a tie) vault and project passages alternating, vault newest
//!    first, project by path ([`interleave`]). With semantic hits,
//!    reciprocal-rank fusion: `LEXICAL_WEIGHT / (RRF_K + lexical rank) +
//!    SEMANTIC_WEIGHT / (RRF_K + semantic rank)`, a missing half adding 0;
//!    ties go to the passage with a lexical rank, then the better lexical
//!    rank, the better semantic rank, the path and the line, so the order is
//!    total. Per-file diversity is a ranking rule ([`diversify`]): a file's
//!    third and later passages move behind every other file's first two;
//!    nothing is cut. The candidate set is bounded only by the scoring
//!    budget (tens of thousands of passages), so every ranking step is
//!    O(n log n) or better: a sort, then linear passes ([`rank_lexical`]).
//! 4. **Lifecycle** (agent effectiveness F7, `note_lifecycle::apply`): the
//!    one policy recall uses too, on the whole ranked set. On a
//!    current-state request a superseded learning is left out and its current
//!    version takes its place; on a historical one (`note_lifecycle::
//!    history_cue`) it stays, labelled, behind its current version. Exact
//!    duplicate learnings collapse to the newest; an unapproved learning is
//!    labelled. Linear, like the rest.
//! 5. **Pool**: the first [`POOL`] passages (`jev::MAX_CANDIDATES`), which a
//!    caller may rerank (automatic retrieval hands them to the context
//!    prioritizer).
//! 6. **One final evidence budget**, after any rerank: the diversity rule
//!    again (a reranker can't lift a file's third passage past the others),
//!    a historical passage's current version back ahead of it, then
//!    [`EVIDENCE_PASSAGES`] passages ([`evidence`]). `search_vault` returns
//!    up to its `limit` of the pool instead, so an explicit search can reach
//!    what the budget left out.
//!
//! Provenance: a vault passage's path is the absolute path under the
//! configured vault (the run manifest stores it vault-relative); a project
//! file keeps its absolute original path. Lines are 1-based; line 0 is an
//! unread file's metadata. The keyword note search (`search_notes`, every
//! word must match) is the same `vault_search` grammar the Search UI uses.
//!
//! Attached sources (a workspace's `sources/`) and the on-demand reader are
//! the broker's second half (`source_evidence`, re-exported here as
//! [`select_attached`] and [`read_source`]): query-centred chunks of every
//! attached file past a relevance floor share one 24 KB budget, and
//! `hollow_vault`'s `read_source`
//! reads the rest of an attached or retrieved file in bounded ranges.
//!
//! The `hollow_vault` subprocess has no semantic index or embedder, so
//! `search_vault` is always lexical; when the app blends semantic
//! candidates, its automatic retrieval can rank differently from
//! `search_vault` for the same query, and the manifest says why.
//!
//! **The semantic half runs beside the lexical scan** (agent effectiveness
//! F8, O2): [`retrieve_by`] starts it on its own thread before the scan and
//! takes its answer when ranking needs it, waiting no later than a run's
//! preparation deadline (`prep_deadline`). Past the deadline the request is
//! ranked lexical only (`semantic: "deadline"`) and the embedding finishes
//! unobserved; the lexical candidates are never cut.

use crate::directory_connections::Passage;
// Attached sources and the on-demand reader (F3 part 2) are the broker's
// second half, in their own module.
use crate::run_manifest::RetrievalStats;
pub use crate::source_evidence::{
    query_source, read_source, select as select_attached, ReadScope, SourceInput, SOURCE_BUDGET,
};
use crate::types::SearchResult;
use crate::vault_search::RetrievalCandidates;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Passages ranked into the pool a caller may rerank: Jev's per-call limit.
pub const POOL: usize = crate::jev::MAX_CANDIDATES;
/// The one final evidence budget, applied after reranking.
pub const EVIDENCE_PASSAGES: usize = 8;
/// Candidate lines per file; the ranking demotes those past [`DIVERSE_LINES`].
pub const LINES_PER_NOTE: usize = 3;
/// A file's passages that rank on their own merit before diversity demotes.
pub const DIVERSE_LINES: usize = 2;
/// Bytes of matching vault notes scored per request.
pub const VAULT_BUDGET: u64 = 32 * 1024 * 1024;
/// Notes the semantic half may contribute.
pub const SEMANTIC_NOTES: usize = 24;
/// The cosine floor for a semantic candidate (bge-small-en-v1.5, whose
/// L2-normalized query/passage similarities for unrelated text sit around
/// 0.4-0.55). Calibrated on the judged corpus (`vault_search_eval`).
pub const SEMANTIC_MIN: f32 = 0.6;
/// Reciprocal-rank fusion constant (Cormack et al.'s 60).
pub const RRF_K: f64 = 60.0;
pub const LEXICAL_WEIGHT: f64 = 1.0;
pub const SEMANTIC_WEIGHT: f64 = 1.0;
/// Characters of a semantic passage's excerpt (a lexical one is one line,
/// cut to the same 900).
const EXCERPT_CHARS: usize = 900;
/// Characters of the request embedded (the model reads 512 tokens anyway).
/// The real-vault eval (`retrieval_eval`) embeds its queries the same way.
pub(crate) const SEMANTIC_QUERY_CHARS: usize = 2_000;

// ── Scope ───────────────────────────────────────────────────────────────

/// Which connected project a query is limited to, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    /// A connected project id (`local-…`); `None`: every connected project.
    pub project: Option<String>,
    /// The workspace the run's folder belongs to, for provenance only.
    pub workspace: Option<String>,
    /// `workspace`, `workspace-disconnected`, `notion-twin` (a Notion
    /// project's workspace named like a connected project), `build-workspace`,
    /// `project-folder`, `git-worktree`, `explicit` or `none`.
    pub basis: &'static str,
}

impl Scope {
    /// Every connected project.
    pub fn all() -> Self {
        Scope {
            project: None,
            workspace: None,
            basis: "none",
        }
    }

    /// A project named by the caller (`HOLLOW_VAULT_SCOPE`, a test), or all.
    pub fn explicit(project: Option<&str>) -> Self {
        match project.map(str::trim).filter(|p| !p.is_empty()) {
            Some(id) => Scope {
                project: Some(id.to_string()),
                workspace: None,
                basis: "explicit",
            },
            None => Scope::all(),
        }
    }
}

fn canon(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// The deepest connected project whose folder is `path` or contains it.
fn project_containing(projects: &[(String, PathBuf)], path: &Path) -> Option<String> {
    projects
        .iter()
        .filter(|(_, root)| path.starts_with(root))
        .max_by_key(|(_, root)| root.components().count())
        .map(|(id, _)| id.clone())
}

/// The main repository of the linked git worktree at or above `cwd`: its
/// `.git` file names `<repo>/.git/worktrees/<name>`, whose `commondir` leads
/// to `<repo>/.git`. `None` for a main checkout or anything unreadable.
fn worktree_repository(cwd: &Path) -> Option<PathBuf> {
    let mut dir = Some(cwd);
    for _ in 0..32 {
        let d = dir?;
        let git = d.join(".git");
        if git.is_dir() {
            return None;
        }
        if git.is_file() {
            let text = std::fs::read_to_string(&git).ok()?;
            let gitdir = text.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
            let gitdir = if Path::new(gitdir).is_absolute() {
                PathBuf::from(gitdir)
            } else {
                d.join(gitdir)
            };
            let common = std::fs::read_to_string(gitdir.join("commondir")).ok()?;
            let common = gitdir.join(common.trim());
            return canon(&common).parent().map(Path::to_path_buf);
        }
        dir = d.parent();
    }
    None
}

/// The id prefix of a connected project (`directory_connections`), which a
/// project's workspace folder shares.
const DISCONNECTED_PREFIX: &str = "local-";

/// The retrieval scope of a run in `cwd`: resolved from records, never from
/// the folder's name (agent effectiveness F3; until 2026-10-06 only a
/// `local-…` folder name scoped, so a build worktree was never scoped).
///
/// 1. `cwd` is a workspace folder (`workspaces/<id>`) whose id is a
///    connected project in the project index: that project (`workspace`).
///    A project's workspace (`local-…`) whose project is no longer in the
///    index keeps its id, which no project matches
///    (`workspace-disconnected`): scope never widens by accident. Any other
///    folder inside the vault is unscoped.
/// 2. `cwd` is a build workspace's live worktree: the connected project that
///    is, or contains, the repository it was created from (`build-workspace`).
///    When no connected project contains it (its project left the index),
///    the workspace's id, which no project matches (`workspace-disconnected`).
/// 3. `cwd` is inside a connected project's folder (`project-folder`).
/// 4. `cwd` is a linked git worktree of a repository inside a connected
///    project (`git-worktree`).
///
/// The one rule for automatic retrieval, `search_vault`
/// (`HOLLOW_VAULT_SCOPE`), the `vault.retrieve` action and the manifest.
pub fn resolve_scope(vault: &Path, cwd: &Path) -> Scope {
    let projects: Vec<(String, PathBuf)> = crate::directory_connections::projects(vault)
        .into_iter()
        .map(|p| (p.id, canon(Path::new(&p.path))))
        .collect();
    let vault_c = canon(vault);
    let cwd_c = canon(cwd);
    if let Ok(rel) = cwd_c.strip_prefix(vault_c.join("workspaces")) {
        let mut parts = rel.components();
        let (Some(first), None) = (parts.next(), parts.next()) else {
            return Scope::all();
        };
        let id = first.as_os_str().to_string_lossy().into_owned();
        if projects.iter().any(|(p, _)| *p == id) {
            return Scope {
                project: Some(id.clone()),
                workspace: Some(id),
                basis: "workspace",
            };
        }
        // A Notion project's workspace named like a connected project keeps
        // that folder's scope (overnight 10-11 Y1: connecting no longer
        // makes its `local-…` twin).
        if crate::workspaces::is_notion_mirror(&id)
            || crate::workspaces::managed_project(vault, &id)
        {
            let project = crate::workspaces::summary(vault, &id)
                .ok()
                .and_then(|w| crate::directory_connections::twin_project(vault, &w.title));
            if let Some(project) = project {
                let basis = if crate::workspaces::tracker_managed(vault, &id) {
                    "tracker-twin"
                } else {
                    "notion-twin"
                };
                return Scope {
                    project: Some(project),
                    workspace: Some(id),
                    basis,
                };
            }
        }
        // A project's workspace (its id is the project's, `local-…`) whose
        // project left the index keeps that id as its scope, which no
        // connected project matches: no connected-source evidence, never
        // every project's (before S1 the `local-` folder name scoped it).
        if id.starts_with(DISCONNECTED_PREFIX) {
            return Scope {
                project: Some(id.clone()),
                workspace: Some(id),
                basis: "workspace-disconnected",
            };
        }
        return Scope {
            project: None,
            workspace: Some(id),
            basis: "none",
        };
    }
    if cwd_c.starts_with(&vault_c) {
        return Scope::all();
    }
    if let Some(id) = crate::build_workspaces::workspace_for_worktree(vault, &cwd_c) {
        let project = crate::build_workspaces::binding_for(vault, &id)
            .and_then(|b| project_containing(&projects, &canon(Path::new(&b.repo))));
        return match project {
            Some(project) => Scope {
                project: Some(project),
                workspace: Some(id),
                basis: "build-workspace",
            },
            // Its repository is in no connected project (the project left
            // the index, or never was one): scoped to the workspace's id,
            // which no project matches, like a disconnected project's
            // workspace. Never every project's evidence.
            None => Scope {
                project: Some(id.clone()),
                workspace: Some(id),
                basis: "workspace-disconnected",
            },
        };
    }
    if let Some(project) = project_containing(&projects, &cwd_c) {
        return Scope {
            project: Some(project),
            workspace: None,
            basis: "project-folder",
        };
    }
    if let Some(project) =
        worktree_repository(&cwd_c).and_then(|repo| project_containing(&projects, &repo))
    {
        return Scope {
            project: Some(project),
            workspace: None,
            basis: "git-worktree",
        };
    }
    Scope::all()
}

// ── Candidates and ranking ──────────────────────────────────────────────

/// A lexical candidate passage with its shared line score (see
/// `vault_search::best_lines`): `coverage` = distinct terms in the line plus
/// terms in the file's name; `in_line` = terms in the line alone.
pub(crate) struct Scored {
    /// A vault note's BM25 band (OB12, `vault_search::LinePassage::band`):
    /// it leads a vault-only list and is only a tiebreak after coverage once
    /// any project passage competes ([`interleave`]). 0 for project files
    /// and on a linear scan.
    pub band: usize,
    pub coverage: usize,
    pub in_line: usize,
    pub vault: bool,
    /// A vault note's mtime (RFC 3339, so it sorts as a string); empty for
    /// project files, which keep path order.
    pub recency: String,
    pub passage: Passage,
}

/// Best first in a vault-only list: band, then coverage, then in_line, then
/// newest, then path and line. With project passages see [`interleave`].
pub(crate) fn by_score(a: &Scored, b: &Scored) -> std::cmp::Ordering {
    a.band
        .cmp(&b.band)
        .then(b.coverage.cmp(&a.coverage))
        .then(b.in_line.cmp(&a.in_line))
        .then(b.recency.cmp(&a.recency))
        .then(a.passage.path.cmp(&b.passage.path))
        .then(a.passage.line.cmp(&b.passage.line))
}

/// Relevance first. A BM25 band (OB12) leads only when every candidate is a
/// vault passage (vault-only retrieval, what the real-vault eval measures):
/// then the order is [`by_score`], band first, and a note's second and third
/// passages rank one band below its best one (never below the lowest band
/// present), so a top note's weak line doesn't sit ahead of the next band's
/// strong one.
///
/// With ANY project passage the order is the pre-OB12 one: (coverage,
/// in_line) best first, the band only a tiebreak after them, then newest,
/// path and line. Groups of equal (coverage, in_line) alternate vault and
/// project passages, vault first, so neither source wins a true tie by the
/// accident of where its absolute path sorts; inside a group vault passages
/// run by band then newest and project passages by path (date-slugged
/// siblings, every instance of a recurring meeting, tie, and path order
/// would cite the oldest eight). OB12's review tried two ways of keeping
/// bands across sources (a coverage merge of the band-sorted vault list;
/// coverage-ordered slots filled in band order); each could push a strong
/// vault line behind dozens of weaker project lines, so a mixed list keeps
/// the pre-OB12 rule. With every band 0 either branch is the pre-OB12 order.
fn interleave(scored: Vec<Scored>) -> Vec<Passage> {
    let (mut vault, project): (Vec<Scored>, Vec<Scored>) =
        scored.into_iter().partition(|s| s.vault);
    vault.sort_by(by_score);
    let lowest = vault.iter().map(|s| s.band).max().unwrap_or(0);
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for s in &mut vault {
        // Sorted by band, then coverage: a note's first passage is its best.
        if !seen.insert(s.passage.path.clone()) {
            s.band = (s.band + 1).min(lowest);
        }
    }
    if project.is_empty() {
        vault.sort_by(by_score);
        return vault.into_iter().map(|s| s.passage).collect();
    }
    let mut scored = vault;
    scored.extend(project);
    scored.sort_by(|a, b| {
        b.coverage
            .cmp(&a.coverage)
            .then(b.in_line.cmp(&a.in_line))
            .then_with(|| by_score(a, b))
    });
    let mut out = Vec::with_capacity(scored.len());
    let mut rest = scored.into_iter().peekable();
    while let Some(first) = rest.next() {
        let key = (first.coverage, first.in_line);
        let mut vault = Vec::new();
        let mut project = Vec::new();
        let mut group = vec![first];
        while let Some(next) = rest.next_if(|s| (s.coverage, s.in_line) == key) {
            group.push(next);
        }
        for s in group {
            if s.vault {
                vault.push(s.passage);
            } else {
                project.push(s.passage);
            }
        }
        let (mut v, mut p) = (vault.into_iter(), project.into_iter());
        loop {
            let (a, b) = (v.next(), p.next());
            if a.is_none() && b.is_none() {
                break;
            }
            out.extend(a);
            out.extend(b);
        }
    }
    out
}

/// The lexical order of the whole candidate set: [`interleave`], the first
/// of any duplicate (path, line) kept, then [`diversify`]. O(n log n) in
/// the candidate set (the sort; the rest is linear): the set is every
/// matching note within the scoring budget, so no step may compare each
/// passage with every earlier one (until the S1 review the duplicate check
/// did, 10 s for 48,000 passages in a debug build).
fn rank_lexical(scored: Vec<Scored>) -> Vec<Passage> {
    let mut seen: std::collections::HashSet<(String, usize)> =
        std::collections::HashSet::with_capacity(scored.len());
    let lexical: Vec<Passage> = interleave(scored)
        .into_iter()
        .filter(|p| seen.insert((p.path.clone(), p.line)))
        .collect();
    diversify(lexical)
}

/// Per-file diversity as a ranking rule: each file's first [`DIVERSE_LINES`]
/// passages keep their places; its later ones move, in order, behind every
/// other passage. Stable; nothing is dropped.
pub(crate) fn diversify(passages: Vec<Passage>) -> Vec<Passage> {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let (mut first, mut later) = (Vec::new(), Vec::new());
    for p in passages {
        let n = seen.entry(p.path.clone()).or_insert(0);
        *n += 1;
        if *n <= DIVERSE_LINES {
            first.push(p);
        } else {
            later.push(p);
        }
    }
    first.extend(later);
    first
}

/// Vault notes automatic retrieval may cite: profiles (`me/`), conversation
/// records, workflow prompts and workspace folders other than their
/// `outputs/` are left out.
pub(crate) fn vault_eligible(rel: &str) -> bool {
    !(["me/", "sessions/", "workflows/"]
        .iter()
        .any(|prefix| rel.starts_with(prefix))
        || (rel.starts_with("workspaces/") && !rel.contains("/outputs/")))
}

/// A semantic hit from the index: a note and its best chunk.
pub(crate) type SemanticHit = crate::vault_embed_index::ChunkHit;

/// What the semantic half answered: hits, or why there are none.
pub(crate) type SemanticAnswer = Result<Vec<SemanticHit>, String>;

/// The app's semantic half for `vault`: the request embedded with the
/// bundled model in this process and looked up in this process's engaged
/// index. Never a network call or a download: an absent model file is a
/// reason, not a fetch.
fn semantic_in_process(vault: &Path, query: &str) -> SemanticAnswer {
    semantic_preparing(vault, query, &|| true)
}

fn semantic_preparing(vault: &Path, query: &str, allowed: &dyn Fn() -> bool) -> SemanticAnswer {
    if !allowed() {
        return Err("send preparation was cancelled or expired".into());
    }
    #[cfg(test)]
    if let Some(answer) = standin::answer(vault) {
        return answer;
    }
    crate::vault_embed_index::semantic_ready(vault).map_err(str::to_string)?;
    let dir = crate::embed_model::default_model_dir();
    if !dir.join("model.safetensors").is_file() {
        return Err("the embedding model isn't installed with this build".into());
    }
    let text: String = query.chars().take(SEMANTIC_QUERY_CHARS).collect();
    let qvec = crate::embed_model::embed_query_preparing(
        &dir,
        &text,
        crate::embed_model::request_epoch(),
        crate::vault_embed_index::is_semantic_enabled,
        allowed,
    )
    .map_err(|e| format!("the request couldn't be embedded ({e})"))?;
    crate::vault_embed_index::engaged_note_hits(vault, &qvec, SEMANTIC_NOTES)
        .map_err(str::to_string)
}

/// A slow semantic half for one test vault (tests have no embedder): it
/// answers "no hits" after its delay. Keyed by the vault, so other tests
/// never see it.
#[cfg(test)]
pub(crate) mod standin {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::Duration;

    static SLOW: Mutex<Vec<(PathBuf, Duration)>> = Mutex::new(Vec::new());

    pub(crate) fn slow_semantic(vault: &Path, delay: Duration) {
        SLOW.lock().unwrap().push((vault.to_path_buf(), delay));
    }

    pub(crate) fn clear(vault: &Path) {
        SLOW.lock().unwrap().retain(|(v, _)| v != vault);
    }

    pub(super) fn answer(vault: &Path) -> Option<super::SemanticAnswer> {
        let delay = SLOW
            .lock()
            .unwrap()
            .iter()
            .find(|(v, _)| v == vault)
            .map(|(_, d)| *d)?;
        std::thread::sleep(delay);
        Some(Ok(Vec::new()))
    }
}

/// A semantic hit as a citable passage: the first non-blank line of its
/// chunk, the chunk's text (whitespace collapsed) as the excerpt. A
/// frontmatter chunk (HF03) is cited by [`frontmatter_lines`]. `None` for an
/// HTML note (its offsets are into stripped text) or a note that can't be
/// read; a body chunk whose offsets no longer fit (the note changed since it
/// was embedded) cites the body's first non-blank line.
fn semantic_passage(vault: &Path, hit: &SemanticHit) -> Option<Passage> {
    let lower = hit.path.to_lowercase();
    if lower.ends_with(".html") || lower.ends_with(".htm") {
        return None;
    }
    let content = std::fs::read_to_string(vault.join(&hit.path)).ok()?;
    let (fm, body) = crate::frontmatter::parse_map(&content);
    let fm_end = content.len() - body.len();
    let (start, quoted): (usize, std::borrow::Cow<str>) = if hit.in_frontmatter {
        let (start, lines) = frontmatter_lines(&content, hit, fm_end)?;
        (start, lines.into())
    } else {
        let fits = hit.byte_start <= hit.byte_end
            && hit.byte_end <= body.len()
            && body.is_char_boundary(hit.byte_start)
            && body.is_char_boundary(hit.byte_end);
        let (span, start) = if fits {
            (&body[hit.byte_start..hit.byte_end], fm_end + hit.byte_start)
        } else {
            (body.as_str(), fm_end)
        };
        let lead = span.len() - span.trim_start().len();
        (start + lead, span.into())
    };
    let line = content[..start.min(content.len())].matches('\n').count() + 1;
    let excerpt: String = quoted
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(EXCERPT_CHARS)
        .collect();
    if excerpt.is_empty() {
        return None;
    }
    Some(Passage {
        path: vault.join(&hit.path).to_string_lossy().into(),
        line,
        excerpt,
        source: hit.kind.clone(),
        coverage: "excerpt".into(),
        score: 0,
        semantic: Some(hit.score),
        lifecycle: None,
        // Read with the text: a semantic hit is held to the same lifecycle
        // policy as a lexical one (F7).
        note: crate::note_lifecycle::of(&hit.path, &fm, &body),
        current: None,
    })
}

/// A frontmatter chunk's citation: the offset of its first embedded field's
/// line and those lines alone (`vault_embed_index::is_frontmatter_field`),
/// so a field between them (`assignee:`, a URL) is never quoted. Its span is
/// file offsets; when they no longer fit the frontmatter (the note changed
/// since it was embedded) the whole frontmatter block is read instead, since
/// a body-less task has no body to fall back to. `None` without such a line.
fn frontmatter_lines(content: &str, hit: &SemanticHit, fm_end: usize) -> Option<(usize, String)> {
    let fits = hit.byte_start <= hit.byte_end
        && hit.byte_end <= fm_end
        && content.is_char_boundary(hit.byte_start)
        && content.is_char_boundary(hit.byte_end);
    let (from, to) = if fits {
        (hit.byte_start, hit.byte_end)
    } else {
        crate::frontmatter::block_bounds(content).ok()?
    };
    let mut first = None;
    let mut kept = Vec::new();
    let mut offset = from;
    for line in content[from..to].split_inclusive('\n') {
        let at = offset;
        offset += line.len();
        let key = crate::frontmatter::top_level_key(line);
        if key.is_some_and(crate::vault_embed_index::is_frontmatter_field) {
            first = first.or(Some(at));
            kept.push(line.trim());
        }
    }
    Some((first?, kept.join(" ")))
}

/// Reciprocal-rank fusion of the lexical order and the semantic hits (see
/// the module doc). Each lexical passage of a note the semantic half found
/// gets that note's semantic rank; a note only the semantic half found joins
/// with one passage.
fn blend(lexical: Vec<Passage>, semantic: Vec<Passage>) -> Vec<Passage> {
    let sem_rank: std::collections::HashMap<String, (usize, f32)> = semantic
        .iter()
        .enumerate()
        .map(|(i, p)| (p.path.clone(), (i + 1, p.semantic.unwrap_or(0.0))))
        .collect();
    let lexical_paths: std::collections::HashSet<String> =
        lexical.iter().map(|p| p.path.clone()).collect();
    // (passage, lexical rank, semantic rank)
    let mut entries: Vec<(Passage, Option<usize>, Option<usize>)> = Vec::new();
    for (i, mut p) in lexical.into_iter().enumerate() {
        let sem = sem_rank.get(&p.path).copied();
        p.semantic = sem.map(|(_, s)| s);
        entries.push((p, Some(i + 1), sem.map(|(r, _)| r)));
    }
    for (i, p) in semantic.into_iter().enumerate() {
        if !lexical_paths.contains(&p.path) {
            entries.push((p, None, Some(i + 1)));
        }
    }
    let fused = |l: Option<usize>, s: Option<usize>| {
        l.map_or(0.0, |r| LEXICAL_WEIGHT / (RRF_K + r as f64))
            + s.map_or(0.0, |r| SEMANTIC_WEIGHT / (RRF_K + r as f64))
    };
    entries.sort_by(|a, b| {
        fused(b.1, b.2)
            .total_cmp(&fused(a.1, a.2))
            .then_with(|| a.1.is_none().cmp(&b.1.is_none()))
            .then_with(|| a.1.unwrap_or(usize::MAX).cmp(&b.1.unwrap_or(usize::MAX)))
            .then_with(|| a.2.unwrap_or(usize::MAX).cmp(&b.2.unwrap_or(usize::MAX)))
            .then_with(|| a.0.path.cmp(&b.0.path))
            .then_with(|| a.0.line.cmp(&b.0.line))
    });
    entries.into_iter().map(|(p, _, _)| p).collect()
}

/// One query for the broker.
pub struct Request<'a> {
    pub vault: &'a Path,
    /// The user's own words (already stripped of injected context).
    pub query: &'a str,
    pub scope: &'a Scope,
    /// Note kinds to keep (`connected_project` for project files); `None`:
    /// all. Applied before ranking.
    pub kinds: Option<&'a [String]>,
}

/// The broker's answer.
pub struct Ranked {
    /// The ranked pool, best first, at most [`POOL`].
    pub passages: Vec<Passage>,
    /// Connection state and the connected-project read budget.
    pub warnings: Vec<String>,
    /// The vault scan's size, budget and read-limit notices. They describe
    /// the vault, not the query, so `search_vault` leaves them out.
    pub vault_notices: Vec<String>,
    pub stats: RetrievalStats,
    /// What made the request historical (`note_lifecycle::history_cue`):
    /// Hollow's own word for it, never the request's text.
    pub history: Option<&'static str>,
    /// The lifecycle policy's decisions on the candidate set (F7).
    pub lifecycle: Vec<crate::run_manifest::LifecycleDecision>,
}

/// The broker in this process: the FTS prefilter (engaged, else the app's
/// database read-only) and, in the app, the semantic half, waited for as
/// long as it takes.
pub fn retrieve(req: &Request) -> Ranked {
    retrieve_by(req, None)
}

/// [`retrieve`] with the semantic half started beside the lexical scan and
/// waited for until `deadline` at most (agent effectiveness F8, O2): past it,
/// lexical only, with `semantic: "deadline"` and its reason. The lexical
/// half always runs to completion. The semantic step's time on the run's
/// timeline runs from its start to its answer, or to the cut.
pub fn retrieve_by(req: &Request, deadline: Option<Instant>) -> Ranked {
    let vault = req.vault;
    let pending = wants_semantic(req)
        .then(|| Pending::start(vault, req.query))
        .flatten();
    let waited = std::cell::Cell::new(None);
    let mut ranked = retrieve_with(
        req,
        &|texts: &[String], notes: &[crate::types::Note]| {
            crate::vault_search_index::ranked_candidates(vault, texts, notes)
        },
        crate::vault_search_index::prefilter_kind(vault),
        &|query: &str| match &pending {
            Some(pending) => pending.answer(deadline, &waited),
            None => semantic_in_process(vault, query),
        },
    );
    if let Some(mark) = waited.get() {
        ranked.stats.semantic_ms = Some(mark.ms);
        ranked.stats.semantic_mark = crate::prep_timing::InMemory(Some(mark));
    }
    ranked
}

/// Whether [`retrieve_tuned`] will ask the semantic half: the request has
/// search terms and vault notes are wanted.
fn wants_semantic(req: &Request) -> bool {
    !crate::directory_connections::retrieval_terms(req.query).is_empty()
        && wants_vault_notes(req.kinds)
}

/// Whether `kinds` asks for vault notes: none given, or a kind that is
/// neither connected project files nor org files.
fn wants_vault_notes(kinds: Option<&[String]>) -> bool {
    kinds.is_none_or(|k| {
        k.iter()
            .any(|k| k != "connected_project" && k != crate::context_layers::ORG_KIND)
    })
}

/// The semantic half, started on its own thread.
struct Pending {
    answer: std::sync::mpsc::Receiver<(SemanticAnswer, crate::prep_timing::Mark)>,
    started: Instant,
}

impl Pending {
    /// `None` when no thread could be started (the half then runs inline).
    fn start(vault: &Path, query: &str) -> Option<Pending> {
        let (tx, answer) = std::sync::mpsc::channel();
        let (vault, query) = (vault.to_path_buf(), query.to_string());
        let started = Instant::now();
        std::thread::Builder::new()
            .name("hollow-semantic".into())
            .spawn(move || {
                let found = semantic_in_process(&vault, &query);
                let _ = tx.send((found, crate::prep_timing::Mark::since(started)));
            })
            .ok()?;
        Some(Pending { answer, started })
    }

    /// Its answer, waiting until `deadline` at most. `waited` gets the
    /// step's mark when it ran (an answer, or the cut); none when it said
    /// semantic search is off or unavailable, as before.
    fn answer(
        &self,
        deadline: Option<Instant>,
        waited: &std::cell::Cell<Option<crate::prep_timing::Mark>>,
    ) -> SemanticAnswer {
        use std::sync::mpsc::RecvTimeoutError;
        let got = match deadline {
            Some(at) => self
                .answer
                .recv_timeout(at.saturating_duration_since(Instant::now())),
            None => self
                .answer
                .recv()
                .map_err(|_| RecvTimeoutError::Disconnected),
        };
        match got {
            Ok((found, mark)) => {
                if found.is_ok() {
                    waited.set(Some(mark));
                }
                found
            }
            Err(RecvTimeoutError::Timeout) => {
                waited.set(Some(crate::prep_timing::Mark::since(self.started)));
                Err(crate::prep_deadline::SEMANTIC_MISSED.to_string())
            }
            Err(RecvTimeoutError::Disconnected) => {
                Err("the semantic half stopped before answering".into())
            }
        }
    }
}

/// [`retrieve`] without the semantic half, which `reason` explains (the
/// `hollow_vault` subprocess has no semantic index or embedder).
pub fn retrieve_lexical(req: &Request, reason: &str) -> Ranked {
    let vault = req.vault;
    retrieve_with(
        req,
        &|texts: &[String], notes: &[crate::types::Note]| {
            crate::vault_search_index::ranked_candidates(vault, texts, notes)
        },
        crate::vault_search_index::prefilter_kind(vault),
        &|_: &str| Err(reason.to_string()),
    )
}

/// A disposable send's semantic work runs inline on its capacity-owning
/// worker, never on Pending's detached thread. Cancellation and expiry are
/// checked again before a model waiting on the cache may load or infer.
pub(crate) fn retrieve_preparing(req: &Request, allowed: &dyn Fn() -> bool) -> Ranked {
    let vault = req.vault;
    retrieve_with(
        req,
        &|texts: &[String], notes: &[crate::types::Note]| {
            crate::vault_search_index::ranked_candidates(vault, texts, notes)
        },
        crate::vault_search_index::prefilter_kind(vault),
        &|query| semantic_preparing(vault, query, allowed),
    )
}

/// [`retrieve`] with the prefilter and the semantic half given (tests).
pub(crate) fn retrieve_with(
    req: &Request,
    prefilter: &dyn Fn(&[String], &[crate::types::Note]) -> RetrievalCandidates,
    prefilter_kind: &str,
    semantic: &dyn Fn(&str) -> SemanticAnswer,
) -> Ranked {
    retrieve_tuned(req, VAULT_BUDGET, prefilter, prefilter_kind, semantic)
}

/// [`retrieve_with`] with the vault scoring budget given, so an evaluation
/// can grow a vault past the budget without writing 32 MB.
pub(crate) fn retrieve_tuned(
    req: &Request,
    vault_budget: u64,
    prefilter: &dyn Fn(&[String], &[crate::types::Note]) -> RetrievalCandidates,
    prefilter_kind: &str,
    semantic: &dyn Fn(&str) -> SemanticAnswer,
) -> Ranked {
    let started = Instant::now();
    let mut stats = RetrievalStats {
        prefilter: prefilter_kind.into(),
        budget: EVIDENCE_PASSAGES as u32,
        semantic: "off".into(),
        ..Default::default()
    };
    let terms = crate::directory_connections::retrieval_terms(req.query);
    if terms.is_empty() {
        stats.semantic_reason = Some("the request has no search terms".into());
        stats.ms = crate::evidence::ms_since(started);
        return Ranked {
            passages: vec![],
            warnings: vec![],
            vault_notices: vec![],
            stats,
            history: None,
            lifecycle: vec![],
        };
    }
    let wants_projects = req
        .kinds
        .is_none_or(|k| k.iter().any(|k| k == "connected_project"));
    let wants_vault = wants_vault_notes(req.kinds);
    // The org clone's files (CX5b, kind `org`): only while its root is set.
    let wants_org = req
        .kinds
        .is_none_or(|k| k.iter().any(|k| k == crate::context_layers::ORG_KIND));
    let mut warnings = Vec::new();
    let mut scored: Vec<Scored> = Vec::new();
    // Connection state and the last refresh's warnings travel with every
    // answer; the project half reads only when projects are wanted.
    let root = crate::directory_connections::projects_root(req.vault, &mut warnings);
    if let Some(root) = root.filter(|_| wants_projects) {
        stats.project_files_read = crate::directory_connections::project_passages(
            req.vault,
            &root,
            &terms,
            req.scope.project.as_deref(),
            &mut scored,
            &mut warnings,
        ) as u64;
    }
    if let Some(org) = crate::context_layers::org_root().filter(|_| wants_org) {
        crate::context_layers::org_passages(&org, &terms, &mut scored, &mut warnings);
    }
    let mut vault_notices = Vec::new();
    if wants_vault {
        let (lines, scan) = crate::vault_search::retrieval_scan_with(
            req.vault,
            &terms,
            &vault_eligible,
            req.kinds,
            crate::vault_search::ScanLimits {
                budget: vault_budget,
                read: crate::vault_search::RETRIEVAL_READ_BYTES.max(vault_budget),
                lines: LINES_PER_NOTE,
            },
            &mut vault_notices,
            prefilter,
        );
        stats.notes_eligible = scan.eligible as u64;
        stats.candidate_source = Some(if scan.indexed { "bm25" } else { "linear" }.into());
        stats.candidates = Some(scan.candidates as u64);
        stats.notes_read = scan.read as u64;
        stats.notes_matched = scan.matched as u64;
        stats.bytes_read = scan.bytes_read;
        stats.bytes_scored = scan.bytes_charged;
        stats.stopped = scan.stopped.map(str::to_string);
        for p in lines {
            scored.push(Scored {
                band: p.band,
                coverage: p.coverage,
                in_line: p.in_line,
                vault: true,
                recency: p.mtime,
                passage: Passage {
                    path: req.vault.join(&p.rel).to_string_lossy().into(),
                    line: p.line,
                    excerpt: p.excerpt,
                    source: p.kind,
                    coverage: "excerpt".into(),
                    score: u32::try_from(p.coverage).unwrap_or(u32::MAX),
                    semantic: None,
                    lifecycle: None,
                    note: p.note,
                    current: None,
                },
            });
        }
    }
    let lexical = rank_lexical(scored);
    let ranked = if wants_vault {
        let sem_started = Instant::now();
        match semantic(req.query) {
            Ok(hits) => {
                let mark = crate::prep_timing::Mark::since(sem_started);
                stats.semantic_ms = Some(mark.ms);
                stats.semantic_mark = crate::prep_timing::InMemory(Some(mark));
                let found: Vec<Passage> = hits
                    .iter()
                    .filter(|h| h.score >= SEMANTIC_MIN && vault_eligible(&h.path))
                    .filter(|h| req.kinds.is_none_or(|k| k.contains(&h.kind)))
                    .filter_map(|h| semantic_passage(req.vault, h))
                    .collect();
                stats.semantic_hits = found.len() as u32;
                stats.semantic = "blended".into();
                if found.is_empty() {
                    stats.semantic_reason = Some(format!(
                        "no note reached the semantic floor ({SEMANTIC_MIN})"
                    ));
                    lexical
                } else {
                    diversify(blend(lexical, found))
                }
            }
            Err(reason) => {
                stats.semantic = if reason == "semantic search is off" {
                    "off".into()
                } else if reason == crate::prep_deadline::SEMANTIC_MISSED {
                    "deadline".into()
                } else {
                    "unavailable".into()
                };
                stats.semantic_reason = Some(reason);
                lexical
            }
        }
    } else {
        stats.semantic_reason = Some(if wants_org {
            "only connected project or org files were asked for".into()
        } else {
            "only connected project files were asked for".into()
        });
        lexical
    };
    // One lifecycle policy (F7): on the whole ranked set, before the pool
    // cut, so a current version can take a superseded note's place.
    let history = crate::note_lifecycle::history_cue(req.query);
    let applied = crate::note_lifecycle::apply(req.vault, ranked, history, req.kinds);
    let mut passages = applied.passages;
    passages.truncate(POOL);
    stats.pool = passages.len() as u32;
    stats.ms = crate::evidence::ms_since(started);
    Ranked {
        passages,
        warnings,
        vault_notices,
        stats,
        history,
        lifecycle: applied.decisions,
    }
}

/// The one final evidence budget: the first [`EVIDENCE_PASSAGES`] of a
/// ranked (and possibly reranked) list, after the diversity rule is applied
/// again (a reranker may lift a file's third passage; it still ranks behind
/// every other file's first two) and a historical passage's current version
/// is put back ahead of it (`note_lifecycle::current_first`). Idempotent on
/// the broker's own order.
pub fn evidence(ranked: Vec<Passage>) -> Vec<Passage> {
    let mut out = crate::note_lifecycle::current_first(diversify(ranked));
    out.truncate(EVIDENCE_PASSAGES);
    out
}

/// The coverage notice when the budget left ranked passages out.
pub fn budget_notice(ranked: usize) -> Option<String> {
    (ranked > EVIDENCE_PASSAGES).then(|| {
        format!(
            "{EVIDENCE_PASSAGES} of {ranked} ranked passages fit the evidence budget; search_vault returns more for a targeted query."
        )
    })
}

// ── Note search (the Search UI's and search_vault's keyword list) ───────

/// Keyword note search: every word must match (`vault_search`'s grammar
/// and ranking). The Search UI, the `vault.vault_search` action and
/// `search_vault`'s note list.
pub fn search_notes(
    vault: &Path,
    query: &str,
    limit: usize,
    kinds: Option<&[String]>,
) -> Vec<SearchResult> {
    crate::vault::search(vault, query, limit, kinds)
}

/// Meaning-based note search (the Vault UI's semantic mode and the
/// `vault.semantic_search` action): the query embedded with the bundled
/// model, cosine-ranked chunks deduped to the best per note. Empty when
/// semantic search is off, not indexed, or the query can't be embedded.
/// `request_epoch` is captured before the caller left the window's thread
/// (`embed_model::request_epoch`), so a disable in between cancels it.
pub fn semantic_notes(
    vault: &Path,
    query: &str,
    limit: usize,
    kinds: Option<&[String]>,
    request_epoch: u64,
) -> Vec<SearchResult> {
    let model_dir = crate::embed_model::default_model_dir();
    match crate::embed_model::embed_query_for_request(
        &model_dir,
        query,
        request_epoch,
        crate::vault_embed_index::is_semantic_enabled,
    ) {
        Ok(qvec) => crate::vault_embed_index::semantic_search(vault, &qvec, limit, kinds),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    fn put(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    fn passage(path: &str, line: usize) -> Passage {
        Passage {
            path: path.into(),
            line,
            excerpt: "x".into(),
            source: "note".into(),
            coverage: "excerpt".into(),
            score: 1,
            semantic: None,
            lifecycle: None,
            note: None,
            current: None,
        }
    }

    fn no_index(_: &[String], _: &[crate::types::Note]) -> RetrievalCandidates {
        None
    }

    fn lexical_only(_: &str) -> SemanticAnswer {
        Err("semantic search is off".into())
    }

    #[test]
    fn send_preparation_keeps_semantics_inline_and_checks_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        put(vault, "notes/atlas.md", "Atlas project delivery notes");
        let scope = Scope::all();
        let request = Request {
            vault,
            query: "atlas project",
            scope: &scope,
            kinds: None,
        };
        let delay = Duration::from_millis(100);
        standin::slow_semantic(vault, delay);
        let started = Instant::now();
        let _ = retrieve_preparing(&request, &|| true);
        assert!(
            started.elapsed() >= delay,
            "the caller owns the complete semantic operation"
        );
        let stopped = semantic_preparing(vault, request.query, &|| false);
        assert!(stopped.unwrap_err().contains("cancelled"));
        standin::clear(vault);
    }

    #[test]
    fn diversity_demotes_a_files_third_passage_and_drops_nothing() {
        let ranked = diversify(vec![
            passage("a", 1),
            passage("a", 2),
            passage("a", 3),
            passage("b", 1),
            passage("c", 1),
        ]);
        let order: Vec<(&str, usize)> = ranked.iter().map(|p| (p.path.as_str(), p.line)).collect();
        assert_eq!(
            order,
            vec![("a", 1), ("a", 2), ("b", 1), ("c", 1), ("a", 3)]
        );
        // A reranker that lifts a file's third passage doesn't get it past
        // the others into the budget; the broker's own order is unchanged.
        let lifted = evidence(vec![
            passage("a", 3),
            passage("a", 1),
            passage("a", 2),
            passage("b", 1),
        ]);
        let order: Vec<(&str, usize)> = lifted.iter().map(|p| (p.path.as_str(), p.line)).collect();
        assert_eq!(order, vec![("a", 3), ("a", 1), ("b", 1), ("a", 2)]);
        let again = evidence(ranked);
        let order: Vec<(&str, usize)> = again.iter().map(|p| (p.path.as_str(), p.line)).collect();
        assert_eq!(
            order,
            vec![("a", 1), ("a", 2), ("b", 1), ("c", 1), ("a", 3)]
        );
    }

    /// OB12 review (red-first): a BM25 band never ranks a vault note against
    /// project files (a mixed list is in coverage order, the band only a
    /// tiebreak). 30 project files that match
    /// one common word, and a band-2 vault note matching all four terms: the
    /// vault note stays at the top of the pool, as before OB12 (when bands
    /// compared across sources, the 30 band-0 project files filled the pool
    /// first). With every band 0 (a linear scan) the order is coverage alone.
    #[test]
    fn a_bm25_band_never_compares_across_sources() {
        let build = |target_band: usize| -> Vec<Scored> {
            let vault = |band, coverage, recency: &str, path: &str| Scored {
                band,
                coverage,
                in_line: coverage,
                vault: true,
                recency: recency.into(),
                passage: passage(path, 1),
            };
            let mut v = vec![
                vault(0, 2, "2026-10-01T00:00:00Z", "/v/notes/best.md"),
                vault(target_band, 4, "2025-01-01T00:00:00Z", "/v/notes/target.md"),
            ];
            for i in 0..30 {
                v.push(Scored {
                    band: 0,
                    coverage: 1,
                    in_line: 1,
                    vault: false,
                    recency: String::new(),
                    passage: passage(&format!("/p/alpha/file{i:02}.md"), 1),
                });
            }
            v
        };
        let paths = |ranked: Vec<Passage>| -> Vec<String> {
            ranked.into_iter().take(POOL).map(|p| p.path).collect()
        };
        // Project passages compete, so coverage leads: the band is only a
        // tiebreak, and the order is the pre-OB12 one whatever the bands.
        let banded = paths(rank_lexical(build(2)));
        let linear = paths(rank_lexical(build(0)));
        for order in [&banded, &linear] {
            assert_eq!(
                order[..3],
                [
                    "/v/notes/target.md",
                    "/v/notes/best.md",
                    "/p/alpha/file00.md"
                ]
            );
        }
    }

    /// Vault passages `(band, coverage, path, line)` plus `projects`
    /// project passages of coverage `project_coverage`.
    fn mixed(
        vault: &[(usize, usize, &str, usize)],
        projects: usize,
        project_coverage: usize,
    ) -> Vec<Scored> {
        let mut v: Vec<Scored> = vault
            .iter()
            .map(|&(band, coverage, path, line)| Scored {
                band,
                coverage,
                in_line: coverage,
                vault: true,
                recency: "2026-10-01T00:00:00Z".into(),
                passage: passage(path, line),
            })
            .collect();
        for i in 0..projects {
            v.push(Scored {
                band: 0,
                coverage: project_coverage,
                in_line: project_coverage,
                vault: false,
                recency: String::new(),
                passage: passage(&format!("/p/alpha/file{i:02}.md"), 1),
            });
        }
        v
    }

    /// Each passage's (path, line), in ranked order.
    fn ranked_lines(scored: Vec<Scored>) -> Vec<(String, usize)> {
        rank_lexical(scored)
            .into_iter()
            .map(|p| (p.path, p.line))
            .collect()
    }

    fn position_of(list: &[(String, usize)], path: &str, line: usize) -> usize {
        list.iter()
            .position(|(p, l)| p == path && *l == line)
            .unwrap()
    }

    /// OB12 delta reviews (red-first): with project passages competing, a
    /// vault note's weak line never holds a stronger vault line of a lower
    /// band out of the 24-passage pool. Three shapes the review found, each
    /// with 30 project passages of coverage 2 and B's line of coverage 4:
    /// A (band 0) with lines of coverage 3 and 1 and B in band 1 (a merge of
    /// the band-sorted vault list gave A3, 30 project, A1, B4); the same
    /// with B in band 2 (coverage-ordered slots filled in band order still
    /// gave A3, A1, 30 project, B4); and A with one coverage-1 line, B in
    /// band 1 (A, 30 project, B). The pre-OB12 order puts B first in each,
    /// and with every band 0 it is exactly that order: B4, A3, project, A1.
    #[test]
    fn a_top_notes_weak_line_never_holds_the_next_band_out_of_the_pool() {
        let shapes: [&[(usize, usize, &str, usize)]; 3] = [
            &[
                (0, 3, "/v/a.md", 1),
                (0, 1, "/v/a.md", 2),
                (1, 4, "/v/b.md", 1),
            ],
            &[
                (0, 3, "/v/a.md", 1),
                (0, 1, "/v/a.md", 2),
                (2, 4, "/v/b.md", 1),
            ],
            &[(0, 1, "/v/a.md", 1), (1, 4, "/v/b.md", 1)],
        ];
        for shape in shapes {
            let order = ranked_lines(mixed(shape, 30, 2));
            let b = position_of(&order, "/v/b.md", 1);
            assert!(b < POOL, "B at {b}: {order:?}");
            assert_eq!(b, 0, "{order:?}");
        }
        let linear = ranked_lines(mixed(
            &[
                (0, 3, "/v/a.md", 1),
                (0, 1, "/v/a.md", 2),
                (0, 4, "/v/b.md", 1),
            ],
            30,
            2,
        ));
        assert_eq!(position_of(&linear, "/v/b.md", 1), 0);
        assert_eq!(position_of(&linear, "/v/a.md", 1), 1);
        assert_eq!(linear[2].0, "/p/alpha/file00.md");
        assert_eq!(position_of(&linear, "/v/a.md", 2), 32);
    }

    /// OB12: with no project passage (vault-only retrieval, what the
    /// real-vault eval measures) the BM25 band leads, and a note's later
    /// lines rank one band below its best one: A (band 0: coverage 3 and 1)
    /// and B (band 1: coverage 4) rank A3, B4, A1. With every band 0 the
    /// order is the pre-OB12 one, coverage first: B4, A3, A1.
    #[test]
    fn a_vault_only_list_ranks_by_band_first() {
        let banded = ranked_lines(mixed(
            &[
                (0, 3, "/v/a.md", 1),
                (0, 1, "/v/a.md", 2),
                (1, 4, "/v/b.md", 1),
            ],
            0,
            0,
        ));
        let at = |path: &str, line: usize| (path.to_string(), line);
        assert_eq!(
            banded,
            vec![at("/v/a.md", 1), at("/v/b.md", 1), at("/v/a.md", 2)]
        );
        let linear = ranked_lines(mixed(
            &[
                (0, 3, "/v/a.md", 1),
                (0, 1, "/v/a.md", 2),
                (0, 4, "/v/b.md", 1),
            ],
            0,
            0,
        ));
        assert_eq!(
            linear,
            vec![at("/v/b.md", 1), at("/v/a.md", 1), at("/v/a.md", 2)]
        );
    }

    /// The candidate set is every matching note within the 32 MB scoring
    /// budget (up to 3 lines each), so the ranking stage must stay
    /// near-linear in it: 16,000 small notes a 16-term prompt matches are
    /// 48,000 passages. Measured against a plain sort of the same set in the
    /// same process, so machine load moves both sides.
    #[test]
    fn ranking_the_whole_candidate_set_stays_near_linear() {
        const NOTES: usize = 16_000;
        let build = || -> Vec<Scored> {
            let mut v = Vec::with_capacity(NOTES * LINES_PER_NOTE + 100);
            for i in 0..NOTES {
                for line in 1..=LINES_PER_NOTE {
                    v.push(Scored {
                        band: 0,
                        coverage: 1 + i % 4,
                        // Line 1 scores best, so a file's third line is line 3.
                        in_line: 1 + LINES_PER_NOTE - line,
                        vault: true,
                        recency: format!("2026-{:02}-{:02}T00:00:00Z", 1 + i % 12, 1 + i % 28),
                        passage: passage(&format!("/v/notes/meeting {i:05}.md"), line),
                    });
                }
            }
            // A duplicate (path, line) ranked lower: the first one stays.
            for i in 0..100 {
                let mut p = passage(&format!("/v/notes/meeting {i:05}.md"), 1);
                p.excerpt = "duplicate".into();
                v.push(Scored {
                    band: 0,
                    coverage: 0,
                    in_line: 0,
                    vault: false,
                    recency: String::new(),
                    passage: p,
                });
            }
            v
        };
        let mut sorted = build();
        let started = Instant::now();
        sorted.sort_by(by_score);
        let sort = started.elapsed();
        let input = build();
        let started = Instant::now();
        let ranked = rank_lexical(input);
        let rank = started.elapsed();
        assert_eq!(
            ranked.len(),
            NOTES * LINES_PER_NOTE,
            "each (path, line) once"
        );
        assert!(ranked.iter().all(|p| p.excerpt != "duplicate"));
        // Every file's third line ranks behind every file's first two.
        assert!(ranked[..NOTES * DIVERSE_LINES]
            .iter()
            .all(|p| p.line <= DIVERSE_LINES));
        assert!(
            rank < sort * 40 + std::time::Duration::from_millis(250),
            "ranking {} passages took {rank:?} against a {sort:?} sort",
            ranked.len()
        );
    }

    #[test]
    fn blend_is_reciprocal_rank_fusion_with_total_tie_breaks() {
        let mut s1 = passage("/v/only-semantic.md", 4);
        s1.semantic = Some(0.9);
        let mut s2 = passage("/v/both.md", 1);
        s2.semantic = Some(0.8);
        let lexical = vec![
            passage("/v/lex-a.md", 1),
            passage("/v/lex-b.md", 1),
            passage("/v/both.md", 7),
        ];
        let out = blend(lexical, vec![s1, s2]);
        let order: Vec<&str> = out.iter().map(|p| p.path.as_str()).collect();
        // both.md: 1/63 + 1/62 beats any single half; lex-a (1/61) ties
        // only-semantic (1/61) and wins the tie by having a lexical rank.
        assert_eq!(
            order,
            vec![
                "/v/both.md",
                "/v/lex-a.md",
                "/v/only-semantic.md",
                "/v/lex-b.md"
            ]
        );
        assert_eq!(out[0].line, 7, "a lexically found note keeps its own line");
        assert_eq!(out[0].semantic, Some(0.8));
        // Deterministic: the same input, the same order.
        let again = blend(
            vec![passage("/v/lex-a.md", 1), passage("/v/lex-b.md", 1)],
            vec![],
        );
        assert_eq!(again.len(), 2);
    }

    #[test]
    fn the_pool_is_wider_than_the_budget_and_ranked_before_any_cap() {
        let t = tempfile::Builder::new()
            .prefix("ae-pool-")
            .tempdir()
            .unwrap();
        let v = t.path().join("Vault");
        for i in 0..30 {
            put(
                &v,
                &format!("notes/Kestrel {i:02}.md"),
                "Kestrel status line.\n",
            );
        }
        let scope = Scope::all();
        let r = retrieve_with(
            &Request {
                vault: &v,
                query: "kestrel status",
                scope: &scope,
                kinds: None,
            },
            &no_index,
            "linear",
            &lexical_only,
        );
        assert_eq!(r.passages.len(), POOL);
        assert_eq!(r.stats.pool, POOL as u32);
        assert_eq!(r.stats.notes_matched, 30);
        assert_eq!(r.stats.semantic, "off");
        assert_eq!(evidence(r.passages).len(), EVIDENCE_PASSAGES);
        assert_eq!(
            budget_notice(POOL).unwrap(),
            "8 of 24 ranked passages fit the evidence budget; search_vault returns more for a targeted query."
        );
    }

    /// Agent effectiveness F8 (O2): the semantic half starts beside the
    /// lexical scan and is waited for until the preparation deadline at
    /// most. Past it the request is ranked lexical only, the manifest says
    /// `deadline` with the reason, the step's mark runs to the cut, and the
    /// lexical candidates are all there. Without a deadline it is waited for.
    #[test]
    fn the_semantic_half_runs_beside_the_scan_and_the_deadline_cuts_it() {
        use std::time::Duration;
        let t = tempfile::Builder::new()
            .prefix("ae-sem-deadline-")
            .tempdir()
            .unwrap();
        let v = t.path().canonicalize().unwrap().join("Vault");
        for i in 0..6 {
            put(
                &v,
                &format!("notes/Kestrel {i}.md"),
                &format!("# Kestrel {i}\n\nKestrel calibration figures for site {i}.\n"),
            );
        }
        let scope = Scope::all();
        let req = Request {
            vault: &v,
            query: "kestrel calibration figures",
            scope: &scope,
            kinds: None,
        };
        let lexical = retrieve_with(&req, &no_index, "linear", &lexical_only);
        standin::slow_semantic(&v, Duration::from_millis(1500));
        let started = Instant::now();
        let cut = retrieve_by(&req, Some(Instant::now() + Duration::from_millis(200)));
        let took = started.elapsed();
        assert!(
            took < Duration::from_millis(1200),
            "waited {took:?} for the semantic half"
        );
        assert_eq!(cut.stats.semantic, "deadline");
        assert_eq!(
            cut.stats.semantic_reason.as_deref(),
            Some(crate::prep_deadline::SEMANTIC_MISSED)
        );
        let mark = cut.stats.semantic_mark.0.expect("it ran until the cut");
        assert!((150..1200).contains(&mark.ms), "{mark:?}");
        assert_eq!(cut.stats.semantic_ms, Some(mark.ms));
        let paths = |r: &Ranked| {
            r.passages
                .iter()
                .map(|p| (p.path.clone(), p.line))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            paths(&cut),
            paths(&lexical),
            "every lexical candidate is kept"
        );
        assert!(!cut.passages.is_empty());
        // No deadline: waited for, and blended (the stand-in found nothing).
        let waited = retrieve_by(&req, None);
        assert_eq!(waited.stats.semantic, "blended");
        let mark = waited.stats.semantic_mark.0.unwrap();
        assert!(mark.ms >= 1400, "{mark:?}");
        standin::clear(&v);
        // A request without terms never starts it.
        let empty = Request {
            query: "the and",
            ..req
        };
        assert!(!wants_semantic(&empty));
    }

    /// A paraphrase shares no term with its note, so only the semantic half
    /// can find it; with semantic unavailable the reason is recorded and the
    /// lexical answer stands.
    #[test]
    fn semantic_candidates_join_only_when_available_and_above_the_floor() {
        let t = tempfile::Builder::new()
            .prefix("ae-sem-")
            .tempdir()
            .unwrap();
        let v = t.path().join("Vault");
        put(
            &v,
            "notes/Acme pricing decision.md",
            "---\ntype: note\n---\n# Acme pricing decision\n\nAcme moves to annual pricing at $40 per seat.\n",
        );
        put(
            &v,
            "notes/Yearly plan.md",
            "The yearly subscription plan, in brief.\n",
        );
        put(&v, "me/Profile.md", "Yearly subscription preferences.\n");
        let scope = Scope::all();
        let req = Request {
            vault: &v,
            query: "what will the yearly subscription cost that customer",
            scope: &scope,
            kinds: None,
        };
        let off = retrieve_with(&req, &no_index, "linear", &lexical_only);
        assert!(off
            .passages
            .iter()
            .all(|p| !p.path.ends_with("Acme pricing decision.md")));
        assert_eq!(
            off.stats.semantic_reason.as_deref(),
            Some("semantic search is off")
        );
        let unavailable = retrieve_with(&req, &no_index, "linear", &|_: &str| {
            Err("the semantic index isn't open in this process".into())
        });
        assert_eq!(unavailable.stats.semantic, "unavailable");
        // F8: only a semantic half that ran and answered is on the timeline.
        assert!(off.stats.semantic_mark.0.is_none());
        assert!(unavailable.stats.semantic_mark.0.is_none());
        let acme_text = fs::read_to_string(v.join("notes/Acme pricing decision.md")).unwrap();
        let (_, body) = crate::frontmatter::parse_map(&acme_text);
        let start = body.find("# Acme").unwrap();
        let hit = |path: &str, score: f32| SemanticHit {
            path: path.into(),
            kind: "note".into(),
            mtime: String::new(),
            byte_start: start,
            byte_end: body.len(),
            in_frontmatter: false,
            score,
        };
        let on = retrieve_with(&req, &no_index, "linear", &|_: &str| {
            Ok(vec![
                hit("notes/Acme pricing decision.md", 0.82),
                hit("me/Profile.md", 0.9),
                hit("notes/Yearly plan.md", 0.4),
            ])
        });
        assert_eq!(on.stats.semantic, "blended");
        assert_eq!(on.stats.semantic_hits, 1, "the floor and eligibility apply");
        let mark = on.stats.semantic_mark.0.expect("the semantic half ran");
        assert_eq!(Some(mark.ms), on.stats.semantic_ms);
        let mut manifest = crate::run_manifest::RunManifest::new();
        manifest.set_retrieval(
            crate::run_manifest::RetrievalRecord {
                stats: Some(on.stats.clone()),
                ..Default::default()
            },
            None,
        );
        assert_eq!(
            manifest
                .timer
                .0
                .get(crate::prep_timing::Step::RetrievalSemantic),
            Some(mark)
        );
        let acme = on
            .passages
            .iter()
            .find(|p| p.path.ends_with("Acme pricing decision.md"))
            .expect("the paraphrase's note joins through the semantic half");
        assert_eq!(acme.line, 4, "cites the chunk's first line in the file");
        assert!(
            acme.excerpt.starts_with("# Acme pricing decision"),
            "{}",
            acme.excerpt
        );
        assert!(on.passages.iter().all(|p| !p.path.contains("/me/")));
        // The lexical note is still there, and found by the lexical half.
        assert!(on
            .passages
            .iter()
            .any(|p| p.path.ends_with("Yearly plan.md")));
    }

    #[test]
    fn scope_comes_from_records_and_paths_never_a_folder_name() {
        let t = tempfile::Builder::new()
            .prefix("ae-scope-")
            .tempdir()
            .unwrap();
        let root = t.path().canonicalize().unwrap();
        let (projects, vault) = (root.join("Projects"), root.join("Vault"));
        put(&projects, "Alpha/README.md", "alpha");
        put(&projects, "Beta/README.md", "beta");
        fs::create_dir_all(&vault).unwrap();
        let conn = crate::directory_connections::connect(
            projects.to_str().unwrap(),
            vault.to_str().unwrap(),
            &root.join("config.json"),
        )
        .unwrap();
        let id = |title: &str| {
            conn.projects
                .iter()
                .find(|p| p.title == title)
                .unwrap()
                .id
                .clone()
        };
        let alpha = id("Alpha");
        // A connected project's workspace.
        let s = resolve_scope(&vault, &vault.join("workspaces").join(&alpha));
        assert_eq!(
            (s.project.as_deref(), s.basis),
            (Some(alpha.as_str()), "workspace")
        );
        // A Notion project's workspace named like a connected project keeps
        // that folder's scope (overnight 10-11 Y1).
        crate::workspaces::ensure_named(
            &vault,
            "notion-be7a0000000000000000000000000000",
            "Beta",
            "From Notion",
        )
        .unwrap();
        let s = resolve_scope(
            &vault,
            &vault.join("workspaces/notion-be7a0000000000000000000000000000"),
        );
        assert_eq!(
            (s.project.as_deref(), s.basis),
            (Some(id("Beta").as_str()), "notion-twin")
        );
        crate::workspaces::ensure_named(
            &vault,
            "notion-07e10000000000000000000000000000",
            "Gamma",
            "From Notion",
        )
        .unwrap();
        assert_eq!(
            resolve_scope(
                &vault,
                &vault.join("workspaces/notion-07e10000000000000000000000000000")
            )
            .basis,
            "none"
        );
        // A workspace whose project is no longer connected keeps its own id
        // (`a_disconnected_project_workspace_never_widens`); any other
        // workspace that isn't a connected project's sees every project.
        fs::create_dir_all(vault.join("workspaces/quick-chats")).unwrap();
        let s = resolve_scope(&vault, &vault.join("workspaces/quick-chats"));
        assert_eq!(
            (s.project, s.workspace.as_deref(), s.basis),
            (None, Some("quick-chats"), "none")
        );
        // The vault root and its other folders: every project.
        assert_eq!(resolve_scope(&vault, &vault), Scope::all());
        // Inside a project folder.
        let s = resolve_scope(&vault, &projects.join("Beta"));
        assert_eq!(
            (s.project.as_deref(), s.basis),
            (Some(id("Beta").as_str()), "project-folder")
        );
        // A linked git worktree, any name, of a repository in Alpha.
        let gitdir = projects.join("Alpha/.git/worktrees/odd-name-42");
        fs::create_dir_all(&gitdir).unwrap();
        fs::write(gitdir.join("commondir"), "../..\n").unwrap();
        let wt = root.join("elsewhere/odd-name-42");
        fs::create_dir_all(wt.join("src")).unwrap();
        fs::write(wt.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
        let s = resolve_scope(&vault, &wt.join("src"));
        assert_eq!(
            (s.project.as_deref(), s.basis),
            (Some(alpha.as_str()), "git-worktree")
        );
        // A build workspace's worktree: its bound repository's project.
        let bound = root.join("Worktrees/whatever-you-like");
        fs::create_dir_all(&bound).unwrap();
        crate::build_workspaces::bind_repo_for_test(
            &vault,
            "build-x",
            &projects.join("Beta"),
            &bound,
        );
        let s = resolve_scope(&vault, &bound);
        assert_eq!(
            (s.project.as_deref(), s.workspace.as_deref(), s.basis),
            (
                Some(id("Beta").as_str()),
                Some("build-x"),
                "build-workspace"
            )
        );
        // Anything else: every project.
        let plain = root.join("Worktrees/not-bound");
        fs::create_dir_all(&plain).unwrap();
        assert_eq!(resolve_scope(&vault, &plain), Scope::all());
    }

    /// A project's workspace (`workspaces/local-…`) whose project was
    /// disconnected, or has left the project index, was scoped to that id
    /// before S1 (so it got no connected-source evidence). Scope must never
    /// widen by accident: it stays that id, which no project matches.
    #[test]
    fn a_disconnected_project_workspace_never_widens() {
        let t = tempfile::Builder::new()
            .prefix("ae-orphan-")
            .tempdir()
            .unwrap();
        let root = t.path().canonicalize().unwrap();
        let (projects, vault) = (root.join("Projects"), root.join("Vault"));
        put(
            &projects,
            "Alpha/README.md",
            "Kestrel cadence lives here.\n",
        );
        put(
            &projects,
            "Beta/README.md",
            "Kestrel cadence lives here too.\n",
        );
        fs::create_dir_all(&vault).unwrap();
        crate::directory_connections::connect(
            projects.to_str().unwrap(),
            vault.to_str().unwrap(),
            &root.join("config.json"),
        )
        .unwrap();
        let gone = "local-000000000000000000000000";
        let cwd = vault.join("workspaces").join(gone);
        fs::create_dir_all(&cwd).unwrap();
        let s = resolve_scope(&vault, &cwd);
        assert_eq!(
            (s.project.as_deref(), s.workspace.as_deref(), s.basis),
            (Some(gone), Some(gone), "workspace-disconnected"),
            "a disconnected project's workspace widened its scope"
        );
        let from_projects = |scope: &Scope| {
            retrieve(&Request {
                vault: &vault,
                query: "kestrel cadence",
                scope,
                kinds: None,
            })
            .passages
            .into_iter()
            .filter(|p| Path::new(&p.path).starts_with(&projects))
            .count()
        };
        assert!(
            from_projects(&Scope::all()) > 0,
            "the fixture has project evidence"
        );
        assert_eq!(
            from_projects(&s),
            0,
            "no connected project: no connected-source evidence"
        );
        // The automatic route and `search_vault`'s scope agree.
        assert_eq!(
            crate::directory_connections::retrieval_scope(&vault, &cwd).as_deref(),
            Some(gone)
        );
        // Wave 2 follow-up: a build workspace whose repository is in no
        // connected project (its project left the index) is scoped the same
        // way, not widened to every project.
        let repo = root.join("Elsewhere/old-repo");
        let worktree = root.join("Worktrees/orphan-build");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        crate::build_workspaces::bind_repo_for_test(&vault, "build-orphan", &repo, &worktree);
        let b = resolve_scope(&vault, &worktree);
        assert_eq!(
            (b.project.as_deref(), b.workspace.as_deref(), b.basis),
            (
                Some("build-orphan"),
                Some("build-orphan"),
                "workspace-disconnected"
            ),
            "a build workspace whose project left the index widened its scope"
        );
        assert_eq!(from_projects(&b), 0);
        assert_eq!(
            crate::directory_connections::retrieval_scope(&vault, &worktree).as_deref(),
            Some("build-orphan")
        );
    }

    // ── Lifecycle (agent effectiveness F7) ──────────────────────────────

    use crate::note_lifecycle::fixture::{
        superseded_learning_vault, HISTORY_Q, NEW_CADENCE, NOW_Q, OLD_CADENCE,
    };

    fn cited(v: &Path, query: &str) -> Vec<(String, serde_json::Value)> {
        let scope = Scope::all();
        let r = retrieve_with(
            &Request {
                vault: v,
                query,
                scope: &scope,
                kinds: None,
            },
            &no_index,
            "linear",
            &lexical_only,
        );
        evidence(r.passages)
            .iter()
            .map(|p| {
                let rel = Path::new(&p.path)
                    .strip_prefix(v)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                (rel, serde_json::to_value(p).unwrap())
            })
            .collect()
    }

    fn first_at(list: &[(String, serde_json::Value)], path: &str) -> Option<usize> {
        list.iter().position(|(p, _)| p == path)
    }

    /// Red first (F7): on a current-state question retrieval cited the
    /// superseded learning as plain evidence while recall left it out. Both
    /// routes now agree: the correction, never the old statement.
    #[test]
    fn a_current_state_query_gets_the_correction_not_the_superseded_learning() {
        let t = tempfile::Builder::new()
            .prefix("ae-life-")
            .tempdir()
            .unwrap();
        let v = t.path().join("Vault");
        superseded_learning_vault(&v);
        let ev = cited(&v, NOW_Q);
        let paths: Vec<&str> = ev.iter().map(|(p, _)| p.as_str()).collect();
        assert!(
            !paths.contains(&OLD_CADENCE),
            "retrieval cited the superseded learning: {paths:?}"
        );
        assert!(paths.contains(&NEW_CADENCE), "the correction: {paths:?}");
        let recall = crate::vault_learning::recall(&v, NOW_Q, &[]);
        let recalled: Vec<&str> = recall.items.iter().map(|i| i.path.as_str()).collect();
        assert!(!recalled.contains(&OLD_CADENCE), "{recalled:?}");
        assert!(recalled.contains(&NEW_CADENCE), "{recalled:?}");
        // The routes agree: every learning retrieval cites is current to recall.
        for p in paths.iter().filter(|p| p.starts_with("learnings/")) {
            assert!(recalled.contains(p), "{p} is evidence but not recalled");
        }
    }

    /// A historical question finds the old version, labelled historical and
    /// naming its replacement, with the correction ranked ahead of it in
    /// both routes.
    #[test]
    fn a_historical_query_gets_both_versions_labelled_and_the_correction_first() {
        let t = tempfile::Builder::new()
            .prefix("ae-life-")
            .tempdir()
            .unwrap();
        let v = t.path().join("Vault");
        superseded_learning_vault(&v);
        let ev = cited(&v, HISTORY_Q);
        let (old, new) = (first_at(&ev, OLD_CADENCE), first_at(&ev, NEW_CADENCE));
        assert!(
            matches!((new, old), (Some(n), Some(o)) if n < o),
            "the correction must rank ahead of the old version: {:?}",
            ev.iter().map(|(p, _)| p).collect::<Vec<_>>()
        );
        let label = ev[old.unwrap()].1["lifecycle"].as_str().unwrap_or_default();
        assert!(
            label.contains("historical") && label.contains(NEW_CADENCE),
            "the old version's label: {label:?}"
        );
        let recall = crate::vault_learning::recall(&v, HISTORY_Q, &[]);
        let recalled: Vec<&str> = recall.items.iter().map(|i| i.path.as_str()).collect();
        let (o, n) = (
            recalled.iter().position(|p| *p == OLD_CADENCE),
            recalled.iter().position(|p| *p == NEW_CADENCE),
        );
        assert!(
            matches!((n, o), (Some(n), Some(o)) if n < o),
            "recall: {recalled:?}"
        );
        let item = serde_json::to_value(&recall.items[o.unwrap()]).unwrap();
        assert!(
            item["lifecycle"]
                .as_str()
                .is_some_and(|l| l.contains("historical")),
            "{item}"
        );
        assert!(recall.prompt_block.contains("historical"));
    }

    /// HF03: a frontmatter chunk's hit (a task with no body worth embedding)
    /// cites the frontmatter line its span starts on, never a body line, and
    /// quotes those frontmatter lines.
    #[test]
    fn a_frontmatter_chunk_hit_cites_a_frontmatter_line() {
        let t = tempfile::tempdir().unwrap();
        let v = t.path().join("Vault");
        // A body-less task with fields the chunk doesn't embed between two
        // that it does: those are never quoted.
        let text = "---\nid: t-9\ntitle: Renew permit\nassignee: dana@example.com\n\
                    notion_url: https://notion.so/abc123\ndescription: Elm St lot\n---\n";
        put(&v, "tasks/permit.md", text);
        let start = text.find("title:").unwrap();
        let end = text.find(" lot").unwrap() + " lot".len();
        let hit = SemanticHit {
            path: "tasks/permit.md".into(),
            kind: "task".into(),
            mtime: String::new(),
            byte_start: start,
            byte_end: end,
            in_frontmatter: true,
            score: 0.8,
        };
        let p = semantic_passage(&v, &hit).expect("a passage");
        assert_eq!(p.line, 3);
        assert_eq!(p.excerpt, "title: Renew permit description: Elm St lot");
        // Offsets past the frontmatter (the note changed since it was
        // embedded) read the frontmatter block, not the empty body.
        let stale = SemanticHit {
            byte_start: text.len() + 10,
            byte_end: text.len() + 40,
            ..hit
        };
        let p = semantic_passage(&v, &stale).expect("a passage");
        assert_eq!(p.line, 3);
        assert_eq!(p.excerpt, "title: Renew permit description: Elm St lot");
    }

    /// HF05: an unrelated note over 1 MB (a 1.8 MB `preview.html` was in
    /// 476 of 476 prompts) is skipped silently, and so is one whose folder
    /// alone the request names; a request naming its file still hears it was
    /// skipped. An HTML note's evidence is its text, at its own
    /// line, never markup.
    #[test]
    fn the_oversized_notice_needs_a_request_naming_the_note() {
        let t = tempfile::tempdir().unwrap();
        let v = t.path().join("Vault");
        let filler = "lorem ipsum ".repeat(95_000);
        let big = format!("<html><body>\n<p>{filler}</p>\n</body></html>\n");
        assert!(big.len() > 1_100_000, "over the 1 MB note limit");
        put(&v, "briefings/preview.html", &big);
        put(&v, "notes/owner.md", "Cobalt owner is Dana.\n");
        put(
            &v,
            "briefings/brief.html",
            "<html>\n<style>p { color: red }</style>\n\
             <p class=\"lead\">Cobalt delivery moved to <b>Thursday</b></p>\n</html>\n",
        );
        let scope = Scope::all();
        let ask = |query: &str| {
            retrieve_with(
                &Request {
                    vault: &v,
                    query,
                    scope: &scope,
                    kinds: None,
                },
                &no_index,
                "linear",
                &lexical_only,
            )
        };
        let unrelated = ask("When is the Cobalt delivery, and who is the owner?");
        let notices = &unrelated.vault_notices;
        assert!(notices.is_empty(), "{notices:?}");
        let brief = unrelated
            .passages
            .iter()
            .find(|p| p.path.ends_with("brief.html"))
            .expect("the brief's text matches");
        assert_eq!(brief.line, 3);
        assert_eq!(brief.excerpt, "Cobalt delivery moved to Thursday");
        // Its folder isn't its name (`briefings` is a schema word).
        let folder = ask("What is in my briefings on Cobalt?");
        assert!(folder.vault_notices.is_empty(), "a folder");
        let named = ask("Why is preview.html missing for Cobalt?");
        assert_eq!(
            named.vault_notices,
            vec!["1 vault note over 1 MB was not scanned for retrieval.".to_string()]
        );
    }

    /// A semantic hit is held to the same policy as a lexical one: the
    /// superseded learning the semantic half found is replaced by its
    /// current version, read from disk because it matched nothing itself.
    #[test]
    fn a_semantic_hit_on_a_superseded_learning_follows_the_same_policy() {
        let t = tempfile::Builder::new()
            .prefix("ae-life-")
            .tempdir()
            .unwrap();
        let v = t.path().join("Vault");
        superseded_learning_vault(&v);
        let text = fs::read_to_string(v.join(OLD_CADENCE)).unwrap();
        let (_, body) = crate::frontmatter::parse_map(&text);
        let start = body.find("Kestrel status").unwrap();
        let semantic = |_: &str| -> SemanticAnswer {
            Ok(vec![SemanticHit {
                path: OLD_CADENCE.into(),
                kind: "learning".into(),
                mtime: String::new(),
                byte_start: start,
                byte_end: body.len(),
                in_frontmatter: false,
                score: 0.8,
            }])
        };
        let scope = Scope::all();
        let r = retrieve_with(
            &Request {
                vault: &v,
                query: "which weekday does the memo ship",
                scope: &scope,
                kinds: None,
            },
            &no_index,
            "linear",
            &semantic,
        );
        assert_eq!(r.stats.semantic_hits, 1);
        let paths: Vec<String> = r
            .passages
            .iter()
            .map(|p| p.path.trim_start_matches(&*v.to_string_lossy()).to_string())
            .collect();
        assert_eq!(paths, [format!("/{NEW_CADENCE}")], "{paths:?}");
        assert_eq!(
            r.passages[0].lifecycle.as_deref(),
            Some(format!("current: supersedes {OLD_CADENCE}").as_str())
        );
        assert!(r.passages[0].excerpt.starts_with("Correction:"));
        let promoted = r
            .lifecycle
            .iter()
            .find(|d| d.action == "promoted")
            .expect("the promotion is recorded");
        assert_eq!(promoted.path, NEW_CADENCE);
    }

    /// Exact duplicates (the same statement, whitespace and case aside)
    /// collapse to the newest in both routes; an unapproved learning is
    /// labelled in retrieval and never recalled.
    #[test]
    fn duplicates_collapse_and_unapproved_learnings_are_labelled_in_both_routes() {
        let t = tempfile::Builder::new()
            .prefix("ae-life-")
            .tempdir()
            .unwrap();
        let v = t.path().join("Vault");
        let note = |statement: &str, at: &str, trust: &str| {
            format!(
                "---\ntype: learned-context\ntarget_kind: operator\ntarget_path: me/Decisions.md\nsubject: Quillfeather\ntrust: {trust}\nlearned_at: {at}\n---\n\n{statement}\n"
            )
        };
        put(
            &v,
            "learnings/a-older.md",
            &note(
                "Quillfeather invoices go to finance.",
                "2026-01-01T00:00:00Z",
                "approved",
            ),
        );
        put(
            &v,
            "learnings/b-newer.md",
            &note(
                "quillfeather  invoices go to FINANCE.",
                "2026-05-01T00:00:00Z",
                "approved",
            ),
        );
        put(
            &v,
            "learnings/c-pending.md",
            &note(
                "Quillfeather invoices skip review.",
                "2026-06-01T00:00:00Z",
                "pending",
            ),
        );
        let scope = Scope::all();
        let q = "quillfeather invoices";
        let r = retrieve_with(
            &Request {
                vault: &v,
                query: q,
                scope: &scope,
                kinds: None,
            },
            &no_index,
            "linear",
            &lexical_only,
        );
        let rel = |p: &Passage| {
            Path::new(&p.path)
                .strip_prefix(&v)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        };
        let cited: Vec<String> = evidence(r.passages).iter().map(rel).collect();
        assert!(
            !cited.contains(&"learnings/a-older.md".to_string()),
            "{cited:?}"
        );
        assert!(
            cited.contains(&"learnings/b-newer.md".to_string()),
            "{cited:?}"
        );
        let collapsed = r
            .lifecycle
            .iter()
            .find(|d| d.action == "collapsed")
            .unwrap();
        assert_eq!(
            (collapsed.path.as_str(), collapsed.current.as_deref()),
            ("learnings/a-older.md", Some("learnings/b-newer.md"))
        );
        let labelled = r
            .lifecycle
            .iter()
            .find(|d| d.status == "unapproved")
            .unwrap();
        assert_eq!(labelled.path, "learnings/c-pending.md");
        let again = retrieve_with(
            &Request {
                vault: &v,
                query: q,
                scope: &scope,
                kinds: None,
            },
            &no_index,
            "linear",
            &lexical_only,
        );
        let pending = again
            .passages
            .iter()
            .find(|p| p.path.ends_with("c-pending.md"))
            .unwrap();
        assert_eq!(
            pending.lifecycle.as_deref(),
            Some(crate::note_lifecycle::UNAPPROVED_LABEL)
        );
        let recall = crate::vault_learning::recall(&v, q, &[]);
        let recalled: Vec<&str> = recall.items.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(recalled, ["learnings/b-newer.md"]);
        assert!(recall
            .lifecycle
            .iter()
            .any(|d| d.action == "collapsed" && d.path == "learnings/a-older.md"));
    }

    /// The manifest records each lifecycle decision (route, path, status,
    /// action, reason) and what made a request historical, never its text;
    /// recall and retrieval decide the same for the same note.
    #[test]
    fn lifecycle_decisions_reach_the_manifest_and_both_routes_agree() {
        let t = tempfile::Builder::new()
            .prefix("ae-life-")
            .tempdir()
            .unwrap();
        let v = t.path().join("Vault");
        superseded_learning_vault(&v);
        for (q, cue, action) in [
            (NOW_Q, None, "excluded"),
            (HISTORY_Q, Some("before"), "historical"),
        ] {
            let prepared = tauri::async_runtime::block_on(
                crate::directory_connections::prioritized_context_via(
                    v.clone(),
                    v.clone(),
                    q.to_string(),
                    |_| async { None },
                ),
            )
            .unwrap();
            let recall = crate::vault_learning::recall(&v, q, &[]);
            let mut manifest = crate::run_manifest::RunManifest::new();
            manifest.add_lifecycle(recall.history, recall.lifecycle.clone());
            manifest.add_lifecycle(prepared.history, prepared.lifecycle.clone());
            let record = manifest.lifecycle.as_ref().unwrap();
            assert_eq!(record.history, cue.is_some());
            assert_eq!(record.cue.as_deref(), cue);
            for route in ["recall", "retrieval"] {
                let d = record
                    .decisions
                    .iter()
                    .find(|d| d.route == route && d.path == OLD_CADENCE)
                    .unwrap_or_else(|| panic!("{route}: {:?}", record.decisions));
                assert_eq!(
                    (d.status.as_str(), d.action.as_str(), d.current.as_deref()),
                    ("superseded", action, Some(NEW_CADENCE)),
                    "{route}"
                );
                assert!(d.reason.contains(NEW_CADENCE), "{}", d.reason);
            }
            let json = serde_json::to_string(&manifest).unwrap();
            assert!(
                !json.contains("How often") && !json.contains("What was"),
                "{json}"
            );
            assert!(!json.contains("every Friday"), "no note text: {json}");
            // The envelope explains a label only when one is there.
            assert_eq!(
                prepared.text.contains(crate::note_lifecycle::EVIDENCE_NOTE),
                cue.is_some() || prepared.text.contains("current: supersedes"),
            );
        }
    }

    /// Review fix (F7): the policy is for learnings only. Hollow files a
    /// replaced project as `projects/superseded/<name>.md` with `status:
    /// superseded`, and ADR-style notes and Notion pages keep a Superseded
    /// status or a `superseded_by` link of their own. A current-state
    /// question that names such a note still cites it, unlabelled, and the
    /// policy records nothing about it.
    #[test]
    fn a_superseded_project_or_decision_note_is_still_cited_on_a_question_that_names_it() {
        use crate::note_lifecycle::fixture::put;
        let t = tempfile::Builder::new()
            .prefix("ae-life-")
            .tempdir()
            .unwrap();
        let v = t.path().join("Vault");
        let project = "projects/superseded/HR Analytics Dashboard.md";
        put(
            &v,
            project,
            "---\ntitle: HR Analytics Dashboard\nstatus: superseded\n---\n\nThe HR Analytics Dashboard tracked headcount, attrition and overtime metrics.\n",
        );
        let adr = "decisions/adr-7 dashboard hosting.md";
        put(
            &v,
            adr,
            "---\ntitle: ADR 7 dashboard hosting\nstatus: Superseded\nsuperseded_by: decisions/adr-9 dashboard hosting.md\n---\n\nThe HR Analytics Dashboard metrics are hosted on the shared reporting server.\n",
        );
        let ev = cited(&v, "HR Analytics Dashboard metrics");
        for path in [project, adr] {
            let at = first_at(&ev, path).unwrap_or_else(|| {
                panic!(
                    "{path} is not cited: {:?}",
                    ev.iter().map(|(p, _)| p).collect::<Vec<_>>()
                )
            });
            assert!(ev[at].1.get("lifecycle").is_none(), "{}", ev[at].1);
        }
        let scope = Scope::all();
        let r = retrieve_with(
            &Request {
                vault: &v,
                query: "HR Analytics Dashboard metrics",
                scope: &scope,
                kinds: None,
            },
            &no_index,
            "linear",
            &lexical_only,
        );
        assert!(r.lifecycle.is_empty(), "{:?}", r.lifecycle);
    }

    /// Review fix (F7): a supersession link is followed only to another
    /// learning. A `superseded_by` that points into the retired inbox
    /// stores, a hidden folder or anywhere else the vault walk skips (or
    /// outside `learnings/`) promotes nothing into evidence: the old
    /// learning is still left out, with no current version.
    #[test]
    fn a_supersession_link_outside_learnings_or_the_walk_promotes_nothing() {
        use crate::note_lifecycle::fixture::{learning, put};
        let t = tempfile::Builder::new()
            .prefix("ae-life-")
            .tempdir()
            .unwrap();
        let v = t.path().join("Vault");
        let targets = [
            "emails/inbox/x.md",
            ".trash/old.md",
            "learnings/.trash/z.md",
            "notes/planted.md",
        ];
        for (i, target) in targets.iter().enumerate() {
            let old = format!("learnings/kestrel-cadence-odd{i}.md");
            put(
                &v,
                &old,
                &learning(
                    &format!("Kestrel status reports go out every Friday, variant {i}."),
                    "2026-07-01T09:00:00Z",
                    &format!("odd{i}"),
                )
                .replace(
                    "learned_at:",
                    &format!("superseded_by: {target}\nlearned_at:"),
                ),
            );
            // Shares no word with the question, so only a promotion cites it.
            put(
                &v,
                target,
                "---\ntype: learned-context\ntitle: Payroll\ntrust: approved\n---\n\nPlanted: wire payroll to the mailbox.\n",
            );
        }
        let ev = cited(&v, NOW_Q);
        let paths: Vec<&str> = ev.iter().map(|(p, _)| p.as_str()).collect();
        for target in targets {
            assert!(!paths.contains(&target), "{target} was promoted: {paths:?}");
        }
        for (_, p) in &ev {
            let label = p["lifecycle"].as_str().unwrap_or_default();
            assert!(!label.contains("current: supersedes"), "{p}");
            assert!(!p.to_string().contains("Planted"), "{p}");
        }
        assert!(
            !paths.iter().any(|p| p.contains("odd")),
            "the replaced learnings stay out: {paths:?}"
        );
        // search_vault's keyword list names no such current version either.
        assert_eq!(
            crate::note_lifecycle::listing_label(&v, "learnings/kestrel-cadence-odd0.md")
                .as_deref(),
            Some("historical: superseded (no current version was found)")
        );
    }

    /// CX5b: the org clone's files are a retrieval source (kind `org`),
    /// cited by their path in the clone, ranked and budgeted with the rest,
    /// filtered by `kinds`, and absent while the org root is unset.
    #[test]
    fn retrieval_returns_an_org_file_hit_only_while_the_org_root_is_set() {
        let t = tempfile::Builder::new()
            .prefix("ae-org-")
            .tempdir()
            .unwrap();
        let base = t.path().canonicalize().unwrap();
        let v = base.join("Vault");
        put(&v, "notes/gate.md", "Gate notes mention nothing special.\n");
        let org = base.join("org");
        put(
            &org,
            "glossary/terms.md",
            "# Glossary\n\nLPR: licence plate recognition.\n",
        );
        put(&org, "tools/x.md", "LPR tool\n");
        let scope = Scope::all();
        let ask = |kinds: Option<&[String]>| {
            retrieve_with(
                &Request {
                    vault: &v,
                    query: "what is LPR",
                    scope: &scope,
                    kinds,
                },
                &no_index,
                "linear",
                &lexical_only,
            )
        };
        let org_hits = |r: &Ranked| -> Vec<(String, usize)> {
            r.passages
                .iter()
                .filter(|p| p.source == crate::context_layers::ORG_KIND)
                .map(|p| (p.path.clone(), p.line))
                .collect()
        };
        assert!(org_hits(&ask(None)).is_empty(), "no root, no org source");
        crate::context_layers::set_test_org_root(Some(org.clone()));
        let r = ask(None);
        let hits = org_hits(&r);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(
            Path::new(&hits[0].0),
            org.join("glossary/terms.md"),
            "cites the clone path"
        );
        assert_eq!(hits[0].1, 3);
        assert!(evidence(r.passages)
            .iter()
            .any(|p| p.source == crate::context_layers::ORG_KIND));
        // `kinds` applies: org only reads no vault note, a note kind no org file.
        let only_org = vec![crate::context_layers::ORG_KIND.to_string()];
        let r = ask(Some(&only_org));
        assert_eq!(org_hits(&r).len(), r.passages.len());
        assert_eq!(r.stats.notes_eligible, 0);
        let notes = vec!["note".to_string()];
        assert!(org_hits(&ask(Some(&notes))).is_empty());
        crate::context_layers::set_test_org_root(None);
    }
}
