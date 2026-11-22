//! Test-only relevance evaluation for [`crate::vault_search`]: a judged corpus,
//! Paperclip's metrics (`searchQualityMetrics`: nDCG@5 and reciprocal rank),
//! and `legacy_search`, a verbatim copy of the pre-Paperclip single-needle
//! loop, so one run reports before and after on the same fixture.
//!
//! The grades were authored per query, independently of the ranking constants,
//! and the mtimes are adversarial: most targets are old (2025) and most
//! distractors new (2026), so recency can't carry a case. If a case fails, fix
//! the ranker. Never edit a judgment to make a test pass.
//!
//! RETRIEVAL (agent effectiveness F3, 2026-10-06). The same judged queries
//! also grade automatic retrieval (the notes an agent's 8 evidence passages
//! cite, in order): `legacy_retrieval`, a copy of the pre-F3 vault half (byte
//! budget charged before the FTS skip, 2 lines per note, cut to 8 before any
//! rerank), against the host retrieval broker (`retrieval_broker`). Three
//! cases are added ([`RETRIEVAL_CASES`]): R1 an older exact match behind a
//! vault grown with newer notes past the budget; R2 a paraphrase that shares
//! no word with its note (only the semantic blend can find it); R3 an
//! arbitrarily named worktree that must keep its project's scope.
//!
//! LIFECYCLE (agent effectiveness F7, 2026-10-06): [`LIFECYCLE_CASES`] grade
//! recall and retrieval on the corpus plus an approved learning superseded
//! by an approved correction (`note_lifecycle::fixture`). The old statement
//! shares more words with both questions. L1, current state: the correction
//! must outrank the old statement in both routes, and the old one must not
//! be evidence. L2, a historical question: both versions, the correction
//! first, the old one labelled historical.

use crate::types::SearchResult;
use std::fs;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

pub(crate) struct Doc {
    pub key: &'static str,
    pub path: &'static str,
    pub old: bool,
    pub content: &'static str,
}

pub(crate) const CORPUS: &[Doc] = &[
    Doc { key: "acme-pricing", path: "notes/Acme pricing decision.md", old: true, content: "# Acme pricing decision\n\nDecision: Acme moves to annual pricing at $40 per seat from October.\n" },
    Doc { key: "acme-meeting", path: "calendar/2026-08-12-1000-acme-quarterly-review.md", old: false, content: "---\ntype: meeting\nsubject: Acme quarterly review\nattendees: Dana Whitfield, Priya Raman\n---\n## Agenda\n- Renewal timeline\n\n## Notes\nThe pricing decision is still open for the enterprise tier.\n" },
    Doc { key: "acme-recording", path: "recordings/2026-08-12-1000-acme-quarterly-review.md", old: false, content: "---\ntype: recording\ntitle: Acme quarterly review\n---\n## Summary\nDana Whitfield walked through usage.\n\n## Transcript\n[00:01] Priya: pricing came up briefly.\n[00:14] Dana: we need a decision before renewal.\n[00:30] Priya: Acme wants a discount.\n" },
    Doc { key: "pricing-page", path: "tasks/update-pricing-page.md", old: false, content: "---\ntitle: Update pricing page\nstatus: Not started\n---\nRefresh the pricing page copy for the website.\n" },
    Doc { key: "pricing-faq", path: "notes/Pricing FAQ.md", old: false, content: "# Pricing FAQ\nAnnual plans are billed upfront. See pricing tiers below.\n" },
    Doc { key: "timeout-runbook", path: "notes/Connection timeout runbook.md", old: true, content: "# Connection timeout runbook\nRepair steps when the sync hits a connection timeout.\n" },
    Doc { key: "timeout-scattered", path: "notes/Retry after timeout.md", old: false, content: "# Retry after timeout\nConnection retry happens after a timeout; repair is automatic.\n" },
    Doc { key: "sync-meeting", path: "calendar/2026-09-01-0900-weekly-sync.md", old: true, content: "---\ntype: meeting\nsubject: Weekly Sync\n---\n## Notes\nRoadmap review.\n" },
    Doc { key: "sync-recording", path: "recordings/2026-09-01-0900-weekly-sync.md", old: true, content: "---\ntype: recording\ntitle: Weekly Sync\n---\n## Summary\nPlatform roadmap status.\n" },
    Doc { key: "standup", path: "notes/Standup 2026-09-10.md", old: false, content: "# Standup\nThe platform roadmap slipped a week; raised at the weekly sync.\nCobalt budget is with finance and the owner is unclear.\n" },
    Doc { key: "platform-roadmap", path: "projects/active/Platform roadmap.md", old: true, content: "---\ntitle: Platform roadmap\nstatus: Active\n---\nQuarterly plan for the platform team.\n" },
    Doc { key: "api-auth", path: "tasks/fix-api-authentication.md", old: true, content: "---\ntitle: Fix API authentication\n---\nTokens expire early.\n" },
    Doc { key: "mobile-api", path: "projects/active/Mobile API.md", old: true, content: "---\ntitle: Mobile API\n---\nBuild the mobile API for the app.\n" },
    Doc { key: "api-incidental", path: "notes/Capistrano migration.md", old: false, content: "# Capistrano migration\nAPI details appear here as an incidental mention.\n" },
    Doc { key: "api-decoy", path: "notes/Rapid prototyping.md", old: false, content: "# Rapid prototyping\nCapistrano scripts and rapid mockups.\n" },
    Doc { key: "mobile-ui", path: "tasks/build-mobile-ui.md", old: true, content: "---\ntitle: Build mobile UI\n---\nScreens for the mobile app.\n" },
    Doc { key: "ui-decoy", path: "tasks/build-release.md", old: false, content: "---\ntitle: Build release\n---\nPackage the build for release.\n" },
    Doc { key: "mobile-nav", path: "projects/active/Mobile navigation polish.md", old: false, content: "---\ntitle: Mobile navigation polish\n---\nTighten tab bar spacing.\n" },
    Doc { key: "onboarding", path: "projects/active/Onboarding wizard polish.md", old: true, content: "---\ntitle: Onboarding wizard polish\n---\nFirst-run flow.\n" },
    Doc { key: "search-perf", path: "tasks/improve-search-performance.md", old: true, content: "---\ntitle: Improve search performance\n---\nProfile the index.\n" },
    Doc { key: "search-typography", path: "notes/Search typography.md", old: false, content: "# Search typography\nFont sizes for the search box.\n" },
    Doc { key: "search-noise", path: "notes/Export improvements.md", old: false, content: "# Export improvements\nA search for performance numbers was included in the meeting.\n" },
    Doc { key: "percent", path: "tasks/release-100-checklist.md", old: true, content: "---\ntitle: Release 100% checklist\n---\nShip when coverage is 100%.\n" },
    Doc { key: "percent-decoy", path: "tasks/release-1000-checklist.md", old: false, content: "---\ntitle: Release 1000 checklist\n---\nShip after 1000 installs.\n" },
    Doc { key: "underscore", path: "notes/Fix foo_bar lookup.md", old: true, content: "# Fix foo_bar lookup\nThe foo_bar key is missing.\n" },
    Doc { key: "underscore-decoy", path: "notes/Fix fooXbar lookup.md", old: false, content: "# Fix fooXbar lookup\nThe fooXbar key is renamed.\n" },
    Doc { key: "code-ident", path: "library/imports/heartbeat_run_events retention.md", old: true, content: "# heartbeat_run_events retention\nKeep 90 days.\n" },
    Doc { key: "unicode", path: "notes/Réparer navigation mobile.md", old: true, content: "# Réparer navigation mobile\nCorriger la barre.\n" },
    Doc { key: "person", path: "people/Dana Whitfield.md", old: true, content: "---\ntype: person\n---\n# Dana Whitfield\nVP Operations at Acme.\n" },
    Doc { key: "cobalt-program", path: "notes/Cobalt program.md", old: true, content: "# Cobalt program\n\n## Budget owner\nPriya Raman.\n\n## Launch\nThursday.\n" },
    Doc { key: "okr", path: "goals/Q3 OKRs.md", old: true, content: "---\ntype: goal\ntitle: Q3 OKRs\n---\nShip the platform roadmap.\n" },
    Doc { key: "okr-decoy", path: "notes/Vendor call.md", old: false, content: "# Vendor call\nTalked to Brokr about Q3 hosting.\n" },
    Doc { key: "renewal", path: "tasks/acme-renewal.md", old: true, content: "---\ntitle: Acme renewal\n---\nSend the renewal quote.\n" },
    Doc { key: "zircon", path: "notes/Zircon plan.md", old: true, content: "Unrelated body text.\n" },
];

pub(crate) struct Case {
    pub id: &'static str,
    pub q: &'static str,
    pub grades: &'static [(&'static str, u32)],
    pub first: Option<&'static str>,
    pub absent: &'static [&'static str],
}

pub(crate) const CASES: &[Case] = &[
    Case {
        id: "J1",
        q: "pricing decision acme",
        grades: &[
            ("acme-pricing", 3),
            ("acme-meeting", 2),
            ("acme-recording", 1),
        ],
        first: Some("acme-pricing"),
        absent: &["pricing-page", "pricing-faq"],
    },
    Case {
        id: "J2",
        q: "Acme pricing decision",
        grades: &[
            ("acme-pricing", 3),
            ("acme-meeting", 2),
            ("acme-recording", 1),
        ],
        first: Some("acme-pricing"),
        absent: &[],
    },
    Case {
        id: "J3",
        q: "the acme pricing",
        grades: &[
            ("acme-pricing", 3),
            ("acme-meeting", 2),
            ("acme-recording", 1),
        ],
        first: Some("acme-pricing"),
        absent: &["pricing-page"],
    },
    Case {
        id: "J4",
        q: "\"connection timeout\"",
        grades: &[("timeout-runbook", 3)],
        first: Some("timeout-runbook"),
        absent: &["timeout-scattered"],
    },
    Case {
        id: "J5",
        q: "repair \"connection timeout\"",
        grades: &[("timeout-runbook", 3)],
        first: Some("timeout-runbook"),
        absent: &["timeout-scattered"],
    },
    Case {
        id: "J6",
        q: "weekly sync",
        grades: &[("sync-meeting", 3), ("sync-recording", 3), ("standup", 1)],
        first: None,
        absent: &[],
    },
    Case {
        id: "J7",
        q: "roadmap platform",
        grades: &[
            ("platform-roadmap", 3),
            ("okr", 2),
            ("standup", 1),
            ("sync-recording", 1),
        ],
        first: Some("platform-roadmap"),
        absent: &["sync-meeting"],
    },
    Case {
        id: "J8",
        q: "api",
        grades: &[("api-auth", 3), ("mobile-api", 3), ("api-incidental", 1)],
        first: None,
        absent: &["api-decoy"],
    },
    Case {
        id: "J9",
        q: "ui",
        grades: &[("mobile-ui", 3)],
        first: Some("mobile-ui"),
        absent: &["ui-decoy"],
    },
    Case {
        id: "J10",
        q: "mibile api",
        grades: &[("mobile-api", 3)],
        first: Some("mobile-api"),
        absent: &["mobile-nav", "mobile-ui", "unicode"],
    },
    Case {
        id: "J11",
        q: "onbordng wizard",
        grades: &[("onboarding", 3)],
        first: Some("onboarding"),
        absent: &[],
    },
    Case {
        id: "J12",
        q: "serach",
        grades: &[("search-perf", 3), ("search-typography", 3)],
        first: None,
        absent: &[],
    },
    Case {
        id: "J13",
        q: "search performance",
        grades: &[("search-perf", 3), ("search-noise", 1)],
        first: Some("search-perf"),
        absent: &["search-typography"],
    },
    Case {
        id: "J14",
        q: "100%",
        grades: &[("percent", 3)],
        first: Some("percent"),
        absent: &["percent-decoy"],
    },
    Case {
        id: "J15",
        q: "foo_bar",
        grades: &[("underscore", 3)],
        first: Some("underscore"),
        absent: &["underscore-decoy"],
    },
    Case {
        id: "J16",
        q: "heartbeat_run_events",
        grades: &[("code-ident", 3)],
        first: Some("code-ident"),
        absent: &[],
    },
    Case {
        id: "J17",
        q: "réparer mobile",
        grades: &[("unicode", 3)],
        first: Some("unicode"),
        absent: &[],
    },
    Case {
        id: "J18",
        q: "Dana Whitfield",
        grades: &[("person", 3), ("acme-meeting", 2), ("acme-recording", 1)],
        first: Some("person"),
        absent: &[],
    },
    Case {
        id: "J19",
        q: "cobalt budget owner",
        grades: &[("cobalt-program", 3), ("standup", 1)],
        first: Some("cobalt-program"),
        absent: &[],
    },
    Case {
        id: "J20",
        q: "q3 okr",
        grades: &[("okr", 3)],
        first: Some("okr"),
        absent: &["okr-decoy"],
    },
    Case {
        id: "J21",
        q: "renewal",
        grades: &[("renewal", 3), ("acme-meeting", 1), ("acme-recording", 1)],
        first: Some("renewal"),
        absent: &[],
    },
    Case {
        id: "J22",
        q: "annual pricing",
        grades: &[("acme-pricing", 3), ("pricing-faq", 1)],
        first: Some("acme-pricing"),
        absent: &["pricing-page"],
    },
    Case {
        id: "J23",
        q: "\"annual pricing\" acme",
        grades: &[("acme-pricing", 3)],
        first: Some("acme-pricing"),
        absent: &[],
    },
    Case {
        id: "J24",
        q: "platform",
        grades: &[
            ("platform-roadmap", 3),
            ("okr", 1),
            ("standup", 1),
            ("sync-recording", 1),
        ],
        first: Some("platform-roadmap"),
        absent: &[],
    },
    Case {
        id: "J25",
        q: "weekyl sync",
        grades: &[("sync-meeting", 3), ("sync-recording", 3)],
        first: None,
        absent: &[],
    },
    Case {
        id: "J26",
        q: "quasarxylophone",
        grades: &[],
        first: None,
        absent: &[],
    },
    Case {
        id: "J27",
        q: "cobalt launch?",
        grades: &[("cobalt-program", 3)],
        first: Some("cobalt-program"),
        absent: &["standup"],
    },
    Case {
        id: "J28",
        q: "Acme pricing decision.md",
        grades: &[("acme-pricing", 3)],
        first: Some("acme-pricing"),
        absent: &[],
    },
    Case {
        id: "J29",
        q: "zircon plan",
        grades: &[("zircon", 3)],
        first: Some("zircon"),
        absent: &[],
    },
    Case {
        id: "J30",
        q: "standup 2026-09-10",
        grades: &[("standup", 3)],
        first: Some("standup"),
        absent: &[],
    },
    Case {
        id: "J31",
        q: "build-mobile ui",
        grades: &[("mobile-ui", 3)],
        first: Some("mobile-ui"),
        absent: &["ui-decoy"],
    },
];

/// A retrieval-only judged case (see the module doc). Keys are [`CORPUS`]
/// keys, or for R3 `alpha` / `beta` (the two projects' READMEs).
pub(crate) struct RetrievalCase {
    pub id: &'static str,
    pub q: &'static str,
    pub grades: &'static [(&'static str, u32)],
    pub absent: &'static [&'static str],
    /// What the case needs: `grown` (R1's corpus with newer filler past a
    /// scaled budget), `plain` or `scope` (R3's projects and worktree).
    /// Documents the case; each test builds its own fixture.
    #[allow(dead_code)]
    pub setup: &'static str,
}

pub(crate) const RETRIEVAL_CASES: &[RetrievalCase] = &[
    RetrievalCase {
        id: "R1",
        q: "annual pricing",
        grades: &[("acme-pricing", 3), ("pricing-faq", 1)],
        absent: &[],
        setup: "grown",
    },
    RetrievalCase {
        id: "R2",
        q: "what will the yearly subscription cost that customer",
        grades: &[("acme-pricing", 3), ("pricing-faq", 1)],
        absent: &[],
        setup: "plain",
    },
    RetrievalCase {
        id: "R3",
        q: "kestrel launch checklist",
        grades: &[("alpha", 3)],
        absent: &["beta"],
        setup: "scope",
    },
];

/// A lifecycle case (see the module doc). Keys: `old` and `new` (the two
/// learnings), or [`CORPUS`] keys.
pub(crate) struct LifecycleCase {
    pub id: &'static str,
    pub q: &'static str,
    pub grades: &'static [(&'static str, u32)],
    pub absent: &'static [&'static str],
    /// Asks about the past: the old version is kept, labelled.
    pub historical: bool,
}

pub(crate) const LIFECYCLE_CASES: &[LifecycleCase] = &[
    LifecycleCase {
        id: "L1",
        q: crate::note_lifecycle::fixture::NOW_Q,
        grades: &[("new", 3)],
        absent: &["old"],
        historical: false,
    },
    LifecycleCase {
        id: "L2",
        q: crate::note_lifecycle::fixture::HISTORY_Q,
        // Both answer it; the policy puts the correction first.
        grades: &[("new", 3), ("old", 2)],
        absent: &[],
        historical: true,
    },
];

/// R1's scaled budget and filler: 120 newer 1 KB notes against a 64 KB
/// budget, the ratio of a vault grown past the real 32 MB.
pub(crate) const GROWN_BUDGET: u64 = 64 * 1024;
const GROWN_FILLER: usize = 120;

/// [`write_corpus`] plus R1's filler: newer than every corpus note, and
/// sharing no word with any judged query.
pub(crate) fn write_grown_corpus() -> tempfile::TempDir {
    let dir = write_corpus();
    let newest = epoch("2026-10-01T00:00:00Z");
    let filler = "Lorem ipsum dolor sit amet, consectetur adipiscing elit.\n".repeat(18);
    for i in 0..GROWN_FILLER {
        let path = dir.path().join(format!("daily/{i:03} log.md"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &filler).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(
                fs::FileTimes::new()
                    .set_modified(UNIX_EPOCH + Duration::from_secs(newest + i as u64 * 60)),
            )
            .unwrap();
    }
    dir
}

/// The pre-F3 vault half of automatic retrieval, copied verbatim in effect:
/// eligible notes newest first, the byte budget charged for every one BEFORE
/// the FTS skip, two lines per note, ordered by (coverage, in_line, newest,
/// path, line), at most two passages per note, cut to 8. Returns each
/// passage's vault-relative path. Report-only baseline; never asserted on
/// except where a case documents the defect.
pub(crate) fn legacy_retrieval(
    vault: &Path,
    query: &str,
    budget: u64,
    prefilter: &dyn Fn(&[String], &[crate::types::Note]) -> crate::vault_search::TermCandidates,
) -> Vec<String> {
    let terms = crate::directory_connections::retrieval_terms(query);
    if terms.is_empty() {
        return Vec::new();
    }
    let mut notes: Vec<crate::types::Note> = crate::vault::walk_notes(vault)
        .into_iter()
        .filter(|n| crate::retrieval_broker::vault_eligible(&n.path))
        .collect();
    notes.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.path.cmp(&b.path)));
    let texts: Vec<String> = terms.iter().map(|t| t.text.clone()).collect();
    let sets = prefilter(&texts, &notes).filter(|v| v.len() == terms.len());
    // (coverage, in_line, mtime, path, line)
    let mut found: Vec<(usize, usize, String, String, usize)> = Vec::new();
    let mut scanned = 0u64;
    for note in &notes {
        if note.size > crate::vault_search::RETRIEVAL_MAX_NOTE_BYTES {
            continue;
        }
        scanned += note.size;
        if scanned > budget {
            break;
        }
        let (stem, stem_norm) = crate::vault_search::stem_names(&note.name);
        let stem_mask =
            crate::vault_search::name_mask(&terms, &[stem.as_str(), stem_norm.as_str()]);
        if let Some(sets) = &sets {
            let skip = stem_mask == 0
                && sets
                    .iter()
                    .all(|s| s.as_ref().is_some_and(|s| !s.contains(note.path.as_str())));
            if skip {
                continue;
            }
        }
        let Ok(content) = fs::read_to_string(vault.join(&note.path)) else {
            continue;
        };
        let lower = content.to_lowercase();
        if crate::vault_search::term_mask(&terms, &lower) == 0 && stem_mask == 0 {
            continue;
        }
        let (fm, _) = crate::frontmatter::parse_map(&content);
        let item = crate::vault_index::classify(&note.path, &fm);
        let title = item.title.to_lowercase();
        let mask = stem_mask | crate::vault_search::name_mask(&terms, &[title.as_str()]);
        for ev in crate::vault_search::best_lines(&terms, &content, mask, 2) {
            found.push((
                ev.coverage,
                ev.in_line,
                note.mtime.clone(),
                note.path.clone(),
                ev.line,
            ));
        }
    }
    found.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.cmp(&a.1))
            .then(b.2.cmp(&a.2))
            .then(a.3.cmp(&b.3))
            .then(a.4.cmp(&b.4))
    });
    let mut out: Vec<(String, usize)> = Vec::new();
    for (_, _, _, path, line) in found {
        if out.iter().any(|(p, l)| *p == path && *l == line)
            || out.iter().filter(|(p, _)| *p == path).count() >= 2
        {
            continue;
        }
        out.push((path, line));
        if out.len() >= 8 {
            break;
        }
    }
    out.into_iter().map(|(p, _)| p).collect()
}

/// Distinct keys in first-appearance order: the notes a passage list cites.
pub(crate) fn distinct(keys: impl IntoIterator<Item = &'static str>) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for k in keys {
        if !out.contains(&k) {
            out.push(k);
        }
    }
    out
}

fn epoch(rfc3339: &str) -> u64 {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .expect("fixture timestamp")
        .timestamp() as u64
}

/// Writes [`CORPUS`] under a `vault-` prefixed temp dir (a dot-prefixed root
/// would be pruned by `walk_notes`). Old notes are dated from 2025-03-01, new
/// ones from 2026-09-01, one minute apart by corpus index.
pub(crate) fn write_corpus() -> tempfile::TempDir {
    let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
    let old = epoch("2025-03-01T00:00:00Z");
    let new = epoch("2026-09-01T00:00:00Z");
    for (i, doc) in CORPUS.iter().enumerate() {
        let path = dir.path().join(doc.path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, doc.content).unwrap();
        let base = if doc.old { old } else { new };
        let at = UNIX_EPOCH + Duration::from_secs(base + i as u64 * 60);
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(at))
            .unwrap();
    }
    dir
}

pub(crate) fn key_of(path: &str) -> &'static str {
    CORPUS
        .iter()
        .find(|d| d.path == path)
        .map_or("?", |d| d.key)
}

pub(crate) fn keys_of(hits: &[SearchResult]) -> Vec<&'static str> {
    hits.iter().map(|h| key_of(&h.path)).collect()
}

/// Port of Paperclip's `searchQualityMetrics`: (nDCG@5, reciprocal rank). With
/// no relevant documents, nDCG is 1 iff the results are empty.
pub(crate) fn metrics(keys: &[&str], grades: &[(&str, u32)]) -> (f64, f64) {
    let grade = |key: &str| {
        grades
            .iter()
            .find(|(k, _)| *k == key)
            .map_or(0, |(_, g)| *g)
    };
    let gain = |g: u32, index: usize| (2f64.powi(g as i32) - 1.0) / ((index + 2) as f64).log2();
    // fold from +0.0, not `sum()`: an empty f64 sum is -0.0, which printed as
    // "-0.000" for every case with no hits.
    let dcg: f64 = keys
        .iter()
        .take(5)
        .enumerate()
        .map(|(i, k)| gain(grade(k), i))
        .fold(0.0, |acc, g| acc + g);
    let mut ideal_grades: Vec<u32> = grades.iter().map(|(_, g)| *g).collect();
    ideal_grades.sort_by(|a, b| b.cmp(a));
    let ideal: f64 = ideal_grades
        .iter()
        .take(5)
        .enumerate()
        .map(|(i, g)| gain(*g, i))
        .sum();
    let ndcg = if ideal == 0.0 {
        if keys.is_empty() {
            1.0
        } else {
            0.0
        }
    } else {
        dcg / ideal
    };
    let rr = keys
        .iter()
        .position(|k| grade(k) == 3)
        .map_or(0.0, |rank| 1.0 / (rank + 1) as f64);
    (ndcg, rr)
}

/// The pre-Paperclip `vault::search` loop, copied verbatim except that the
/// FTS prefilter is omitted (it never changed the old results): one
/// lowercased needle, a `limit * 2` early break over the unsorted walk, and a
/// name match (1000) that ties a body match at byte 0. Report-only baseline;
/// never asserted on.
pub(crate) fn legacy_search(
    vault: &Path,
    query: &str,
    limit: usize,
    kinds: Option<&[String]>,
) -> Vec<SearchResult> {
    if query.len() < 2 {
        return Vec::new();
    }
    let needle = query.to_lowercase();
    let notes = crate::vault::walk_notes(vault);
    let mut results: Vec<SearchResult> = Vec::new();
    for note in notes {
        if results.len() >= limit * 2 {
            break;
        }
        let name_match = note.name.to_lowercase().contains(&needle);
        let full = vault.join(&note.path);
        let content = match fs::read_to_string(&full) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let (fm, _) = crate::frontmatter::parse_map(&content);
        let kind = crate::vault_index::classify(&note.path, &fm).kind;
        if let Some(allow) = kinds {
            if !allow.iter().any(|k| k == &kind) {
                continue;
            }
        }
        let lower = content.to_lowercase();
        let body_idx = lower.find(&needle);
        if body_idx.is_some() || name_match {
            let snippet = if let Some(idx) = body_idx {
                let start = crate::vault::char_floor(&content, idx.saturating_sub(60));
                let end = crate::vault::char_ceil(
                    &content,
                    (idx + needle.len() + 100).min(content.len()),
                );
                content[start..end].to_string()
            } else {
                String::new()
            };
            let score = if name_match {
                1000
            } else {
                1000 - (body_idx.unwrap_or(0) as i64)
            };
            results.push(SearchResult {
                path: note.path,
                name: note.name,
                mtime: note.mtime,
                size: note.size,
                snippet,
                score,
                kind,
            });
        }
    }
    results.sort_by_key(|r| std::cmp::Reverse(r.score));
    results.truncate(limit);
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Note;
    use crate::vault_search::{search_with, RetrievalCandidates, TermCandidates};
    use crate::vault_search_index::SearchIndex;
    use std::cell::{Cell, RefCell};

    fn no_index(_: &[String], _: &[Note]) -> TermCandidates {
        None
    }

    /// Retrieval with no index: the linear scan.
    fn no_bm25(_: &[String], _: &[Note]) -> RetrievalCandidates {
        None
    }

    fn mean(values: &[f64]) -> f64 {
        values.iter().sum::<f64>() / values.len().max(1) as f64
    }

    #[test]
    fn judged_queries_meet_relevance_gates() {
        let dir = write_corpus();
        let v = dir.path();
        let mut after_ndcg = Vec::new();
        let mut after_rr = Vec::new();
        let mut before_ndcg = Vec::new();
        let mut before_rr = Vec::new();
        let mut failures = Vec::new();
        eprintln!(
            "{:<4} {:<28} {:>7} {:>5} | {:>7} {:>5} | results",
            "case", "query", "nDCG@5", "RR", "legacy", "RR"
        );
        for case in CASES {
            let hits = search_with(v, case.q, 50, None, &no_index);
            let keys = keys_of(&hits);
            let (ndcg, rr) = metrics(&keys, case.grades);
            let legacy = keys_of(&legacy_search(v, case.q, 50, None));
            let (legacy_ndcg, legacy_rr) = metrics(&legacy, case.grades);
            eprintln!(
                "{:<4} {:<28} {:>7.3} {:>5.2} | {:>7.3} {:>5.2} | {:?}",
                case.id, case.q, ndcg, rr, legacy_ndcg, legacy_rr, keys
            );
            if let Some(first) = case.first {
                if keys.first().copied() != Some(first) {
                    failures.push(format!(
                        "{}: first should be {first}, got {keys:?}",
                        case.id
                    ));
                }
            }
            for absent in case.absent {
                if keys.contains(absent) {
                    failures.push(format!(
                        "{}: {absent} must be absent, got {keys:?}",
                        case.id
                    ));
                }
            }
            for (key, grade) in case.grades {
                if *grade == 3 && !keys.iter().take(5).any(|k| k == key) {
                    failures.push(format!(
                        "{}: {key} missing from the top 5 of {keys:?}",
                        case.id
                    ));
                }
            }
            if case.grades.is_empty() && !keys.is_empty() {
                failures.push(format!("{}: expected no results, got {keys:?}", case.id));
            }
            after_ndcg.push(ndcg);
            before_ndcg.push(legacy_ndcg);
            // MRR only counts cases that have a grade-3 answer.
            if case.grades.iter().any(|(_, g)| *g == 3) {
                after_rr.push(rr);
                before_rr.push(legacy_rr);
            }
        }
        let (mrr, ndcg) = (mean(&after_rr), mean(&after_ndcg));
        eprintln!(
            "before (legacy_search): MRR {:.3}, mean nDCG@5 {:.3}  ->  after: MRR {:.3}, mean nDCG@5 {:.3}  ({} cases)",
            mean(&before_rr),
            mean(&before_ndcg),
            mrr,
            ndcg,
            CASES.len()
        );
        assert!(
            failures.is_empty(),
            "judged cases failed:\n{}",
            failures.join("\n")
        );
        assert!(mrr >= 0.95, "MRR {mrr:.3} < 0.95");
        assert!(ndcg >= 0.90, "mean nDCG@5 {ndcg:.3} < 0.90");
    }

    /// The semantic half for an evaluation: the bundled bge-small model (a
    /// private instance, never the process cache) over an isolated index of
    /// the vault. `None` when the model file isn't fetched (as the embedding
    /// test skips).
    struct SemanticArm {
        _db: tempfile::TempDir,
        idx: crate::vault_embed_index::EmbedIndex,
        model: crate::embed_model::OwnEmbedder,
    }

    impl SemanticArm {
        fn over(v: &Path) -> Option<SemanticArm> {
            let dir = crate::embed_model::default_model_dir();
            if !dir.join("model.safetensors").exists() {
                eprintln!(
                    "bge model not fetched at {}; semantic arm skipped",
                    dir.display()
                );
                return None;
            }
            let model = crate::embed_model::OwnEmbedder::load(&dir).expect("model");
            let db = tempfile::tempdir().unwrap();
            let mut idx =
                crate::vault_embed_index::EmbedIndex::open(&db.path().join("embed.db")).unwrap();
            assert!(idx.reconcile_add_edit(&crate::vault::walk_notes(v), v));
            idx.embed_pending_for_test(|t| model.passage(t).expect("embed"));
            Some(SemanticArm {
                _db: db,
                idx,
                model,
            })
        }

        fn answer(&self, q: &str) -> crate::retrieval_broker::SemanticAnswer {
            let qvec = self.model.query(q)?;
            Ok(self
                .idx
                .note_hits(&qvec, crate::retrieval_broker::SEMANTIC_NOTES))
        }
    }

    fn lexical_only(_: &str) -> crate::retrieval_broker::SemanticAnswer {
        Err("semantic search is off".into())
    }

    /// The notes the broker's evidence (its ranked pool cut to the budget)
    /// cites, as keys.
    fn broker_keys(
        v: &Path,
        q: &str,
        scope: &crate::retrieval_broker::Scope,
        budget: u64,
        prefilter: &dyn Fn(&[String], &[Note]) -> RetrievalCandidates,
        semantic: &dyn Fn(&str) -> crate::retrieval_broker::SemanticAnswer,
        key: &dyn Fn(&str) -> &'static str,
    ) -> Vec<&'static str> {
        let r = crate::retrieval_broker::retrieve_tuned(
            &crate::retrieval_broker::Request {
                vault: v,
                query: q,
                scope,
                kinds: None,
            },
            budget,
            prefilter,
            "test",
            semantic,
        );
        distinct(
            crate::retrieval_broker::evidence(r.passages)
                .iter()
                .map(|p| key(&p.path)),
        )
    }

    fn vault_key(v: &Path) -> impl Fn(&str) -> &'static str + '_ {
        move |abs: &str| {
            Path::new(abs)
                .strip_prefix(v)
                .ok()
                .and_then(|rel| rel.to_str())
                .map_or("?", key_of)
        }
    }

    /// Agent effectiveness F3: automatic retrieval graded on the judged set
    /// before (the pre-F3 vault half) and after (the broker, lexical; and
    /// with the semantic blend when the model is fetched), plus R1-R3. No
    /// existing case may lose nDCG@5 or RR to the broker's lexical ranking.
    #[test]
    fn retrieval_judged_before_and_after_the_broker() {
        let all = crate::retrieval_broker::Scope::all();
        let dir = write_corpus();
        let v = dir.path();
        let arm = SemanticArm::over(v);
        let semantic = |q: &str| match &arm {
            Some(a) => a.answer(q),
            None => lexical_only(q),
        };
        let key = vault_key(v);
        // (case, before, after lexical, after with semantic)
        let mut rows: Vec<(&str, (f64, f64), (f64, f64), Option<(f64, f64)>)> = Vec::new();
        let mut regressions = Vec::new();
        eprintln!(
            "{:<4} {:<44} | {:>6} {:>4} | {:>6} {:>4} | {:>6} {:>4} | after (lexical)",
            "case", "query", "before", "RR", "lex", "RR", "+sem", "RR"
        );
        let mut report = |id: &'static str,
                          q: &str,
                          grades: &[(&str, u32)],
                          before: &[&str],
                          lexical: &[&str],
                          blended: Option<Vec<&str>>| {
            let b = metrics(before, grades);
            let l = metrics(lexical, grades);
            let s = blended.as_ref().map(|k| metrics(k, grades));
            eprintln!(
                "{:<4} {:<44} | {:>6.3} {:>4.2} | {:>6.3} {:>4.2} | {:>6} {:>4} | {:?}",
                id,
                q.chars().take(44).collect::<String>(),
                b.0,
                b.1,
                l.0,
                l.1,
                s.map_or("n/a".into(), |s| format!("{:.3}", s.0)),
                s.map_or("n/a".into(), |s| format!("{:.2}", s.1)),
                lexical
            );
            rows.push((id, b, l, s));
        };
        // OB12 review: the judged cases through the index's BM25 candidates
        // and bands too, as the app runs them. (linear broker, BM25) per case.
        let jdb = tempfile::tempdir().unwrap();
        let jidx = RefCell::new(SearchIndex::open(&jdb.path().join("idx.db")).expect("index"));
        let judged_bm25 = |t: &[String], n: &[Note]| -> RetrievalCandidates {
            jidx.borrow_mut().reconcile_and_rank(n, t, v)
        };
        let mut bm25_rows: Vec<((f64, f64), (f64, f64))> = Vec::new();
        for case in CASES {
            let before = distinct(
                legacy_retrieval(v, case.q, crate::retrieval_broker::VAULT_BUDGET, &no_index)
                    .iter()
                    .map(|p| key_of(p)),
            );
            let bm25 = broker_keys(
                v,
                case.q,
                &all,
                crate::retrieval_broker::VAULT_BUDGET,
                &judged_bm25,
                &lexical_only,
                &key,
            );
            let m = metrics(&bm25, case.grades);
            eprintln!("{:<4} BM25 {:>6.3} {:>4.2} | {bm25:?}", case.id, m.0, m.1);
            let lexical = broker_keys(
                v,
                case.q,
                &all,
                crate::retrieval_broker::VAULT_BUDGET,
                &no_bm25,
                &lexical_only,
                &key,
            );
            let blended = arm.as_ref().map(|_| {
                broker_keys(
                    v,
                    case.q,
                    &all,
                    crate::retrieval_broker::VAULT_BUDGET,
                    &no_bm25,
                    &semantic,
                    &key,
                )
            });
            let (b, l) = (
                metrics(&before, case.grades),
                metrics(&lexical, case.grades),
            );
            if l.0 + 1e-9 < b.0 || l.1 + 1e-9 < b.1 {
                regressions.push(format!("{}: {b:?} -> {l:?}", case.id));
            }
            bm25_rows.push((l, m));
            report(case.id, case.q, case.grades, &before, &lexical, blended);
        }
        let existing = CASES.len();
        // The BM25 path may order a case differently from the linear scan
        // (bands rank before coverage), so it is held to the bar in the mean:
        // no lower mean nDCG@5 or MRR than the linear broker's (main's).
        let mean_of = |pick: &dyn Fn(&((f64, f64), (f64, f64))) -> f64| {
            bm25_rows.iter().map(pick).sum::<f64>() / bm25_rows.len().max(1) as f64
        };
        let (linear_ndcg, linear_rr) = (mean_of(&|r| r.0 .0), mean_of(&|r| r.0 .1));
        let (bm25_ndcg, bm25_rr) = (mean_of(&|r| r.1 .0), mean_of(&|r| r.1 .1));
        eprintln!(
            "BM25 path (J1-J31): nDCG@5 {linear_ndcg:.3} -> {bm25_ndcg:.3}, MRR {linear_rr:.3} -> {bm25_rr:.3} (linear broker -> BM25)"
        );
        assert!(
            bm25_ndcg + 1e-9 >= linear_ndcg && bm25_rr + 1e-9 >= linear_rr,
            "the BM25 path regressed the judged cases against the linear broker: nDCG@5 {linear_ndcg:.3} -> {bm25_ndcg:.3}, MRR {linear_rr:.3} -> {bm25_rr:.3}"
        );

        // R1: an older exact match behind newer filler past the budget.
        let grown = write_grown_corpus();
        let g = grown.path();
        let gkey = vault_key(g);
        let r1 = &RETRIEVAL_CASES[0];
        let before = distinct(
            legacy_retrieval(g, r1.q, GROWN_BUDGET, &no_index)
                .iter()
                .map(|p| key_of(p)),
        );
        let db = tempfile::tempdir().unwrap();
        let idx = RefCell::new(SearchIndex::open(&db.path().join("idx.db")).expect("index"));
        let with_bm25 = |t: &[String], n: &[Note]| -> RetrievalCandidates {
            idx.borrow_mut().reconcile_and_rank(n, t, g)
        };
        let lexical = broker_keys(
            g,
            r1.q,
            &all,
            GROWN_BUDGET,
            &with_bm25,
            &lexical_only,
            &gkey,
        );
        let linear = broker_keys(g, r1.q, &all, GROWN_BUDGET, &no_bm25, &lexical_only, &gkey);
        assert!(before.is_empty(), "R1 documents the defect: {before:?}");
        // OB12: the index ranks by BM25 band, the linear scan by coverage
        // alone, so their orders may differ; both find the older match.
        for (path, keys) in [("index", &lexical), ("linear", &linear)] {
            assert!(
                keys.iter().take(5).any(|k| *k == "acme-pricing"),
                "R1 {path}: {keys:?}"
            );
        }
        report(r1.id, r1.q, r1.grades, &before, &lexical, None);

        // R2: a paraphrase.
        let r2 = &RETRIEVAL_CASES[1];
        let before = distinct(
            legacy_retrieval(v, r2.q, crate::retrieval_broker::VAULT_BUDGET, &no_index)
                .iter()
                .map(|p| key_of(p)),
        );
        let lexical = broker_keys(
            v,
            r2.q,
            &all,
            crate::retrieval_broker::VAULT_BUDGET,
            &no_bm25,
            &lexical_only,
            &key,
        );
        let blended = arm.as_ref().map(|a| {
            if let Ok(hits) = a.answer(r2.q) {
                for h in hits.iter().take(8) {
                    eprintln!("  R2 semantic {:.3} {}", h.score, h.path);
                }
            }
            broker_keys(
                v,
                r2.q,
                &all,
                crate::retrieval_broker::VAULT_BUDGET,
                &no_bm25,
                &semantic,
                &key,
            )
        });
        if let Some(keys) = &blended {
            assert!(
                keys.iter().take(5).any(|k| *k == "acme-pricing"),
                "R2 with the semantic blend: {keys:?}"
            );
        }
        report(r2.id, r2.q, r2.grades, &before, &lexical, blended);

        // R3: an arbitrarily named worktree keeps its project's scope.
        let r3 = &RETRIEVAL_CASES[2];
        let t = tempfile::Builder::new()
            .prefix("ae-eval-")
            .tempdir()
            .unwrap();
        let root = t.path().canonicalize().unwrap();
        let (projects, vault) = (root.join("Projects"), root.join("Vault"));
        for (name, body) in [
            (
                "Alpha",
                "# Kestrel launch checklist\nKestrel launch checklist for Alpha.\n",
            ),
            (
                "Beta",
                "# Kestrel launch\nKestrel launch checklist, Beta's copy.\n",
            ),
        ] {
            fs::create_dir_all(projects.join(name)).unwrap();
            fs::write(projects.join(name).join("README.md"), body).unwrap();
        }
        fs::create_dir_all(&vault).unwrap();
        crate::directory_connections::connect(
            projects.to_str().unwrap(),
            vault.to_str().unwrap(),
            &root.join("config.json"),
        )
        .unwrap();
        let worktree = root.join("Worktrees/zz-try-anything");
        fs::create_dir_all(&worktree).unwrap();
        crate::build_workspaces::bind_repo_for_test(
            &vault,
            "build-eval",
            &projects.join("Alpha"),
            &worktree,
        );
        let pkey = |abs: &str| -> &'static str {
            if abs.ends_with("Alpha/README.md") {
                "alpha"
            } else if abs.ends_with("Beta/README.md") {
                "beta"
            } else {
                "?"
            }
        };
        // Before: the pre-F3 rule scoped only a `local-…` folder name.
        let legacy_scope = worktree
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| s.starts_with("local-"));
        let before = broker_keys(
            &vault,
            r3.q,
            &crate::retrieval_broker::Scope::explicit(legacy_scope),
            crate::retrieval_broker::VAULT_BUDGET,
            &no_bm25,
            &lexical_only,
            &pkey,
        );
        let scope = crate::retrieval_broker::resolve_scope(&vault, &worktree);
        let after = broker_keys(
            &vault,
            r3.q,
            &scope,
            crate::retrieval_broker::VAULT_BUDGET,
            &no_bm25,
            &lexical_only,
            &pkey,
        );
        assert!(
            before.contains(&"beta"),
            "R3 documents the defect: {before:?}"
        );
        assert_eq!(after, vec!["alpha"], "R3 keeps Alpha's scope");
        eprintln!(
            "  R3 out-of-scope passages: before {}, after {} (scope {:?}, {})",
            before.iter().filter(|k| r3.absent.contains(k)).count(),
            after.iter().filter(|k| r3.absent.contains(k)).count(),
            scope.project,
            scope.basis
        );
        report(r3.id, r3.q, r3.grades, &before, &after, None);

        let mean = |pick: &dyn Fn(&(&str, (f64, f64), (f64, f64), Option<(f64, f64)>)) -> f64,
                    range: std::ops::Range<usize>| {
            let v: Vec<f64> = rows[range].iter().map(pick).collect();
            v.iter().sum::<f64>() / v.len().max(1) as f64
        };
        for (label, range) in [
            ("existing (J1-J31)", 0..existing),
            ("new (R1-R3)", existing..rows.len()),
            ("all", 0..rows.len()),
        ] {
            eprintln!(
                "{label:<18} nDCG@5 {:.3} -> {:.3}, MRR {:.3} -> {:.3} (before -> after, lexical)",
                mean(&|r| r.1 .0, range.clone()),
                mean(&|r| r.2 .0, range.clone()),
                mean(&|r| r.1 .1, range.clone()),
                mean(&|r| r.2 .1, range.clone()),
            );
        }
        if arm.is_some() {
            let blended =
                |r: &(&str, (f64, f64), (f64, f64), Option<(f64, f64)>)| r.3.unwrap_or(r.2);
            eprintln!(
                "with the semantic blend: all nDCG@5 {:.3}, MRR {:.3}",
                mean(&|r| blended(r).0, 0..rows.len()),
                mean(&|r| blended(r).1, 0..rows.len()),
            );
        }
        assert!(
            regressions.is_empty(),
            "the broker regressed judged cases:\n{}",
            regressions.join("\n")
        );
    }

    /// Agent effectiveness F7: superseded vs current, and a historical
    /// question, graded in both routes (L1, L2). Before is the pre-wave
    /// retrieval (`legacy_retrieval`, which read `learnings/` like any
    /// note); after is the broker and recall under the one lifecycle policy.
    #[test]
    fn lifecycle_judged_in_recall_and_retrieval() {
        use crate::note_lifecycle::fixture::{superseded_learning_vault, NEW_CADENCE, OLD_CADENCE};
        let dir = write_corpus();
        let v = dir.path();
        superseded_learning_vault(v);
        let key = |rel: &str| -> &'static str {
            match rel {
                OLD_CADENCE => "old",
                NEW_CADENCE => "new",
                other => key_of(other),
            }
        };
        let all = crate::retrieval_broker::Scope::all();
        eprintln!(
            "{:<4} {:<60} | {:>6} {:>4} | {:>6} {:>4} | {:>6} {:>4} | after (retrieval)",
            "case", "query", "before", "RR", "retr", "RR", "recall", "RR"
        );
        for case in LIFECYCLE_CASES {
            let before = distinct(
                legacy_retrieval(v, case.q, crate::retrieval_broker::VAULT_BUDGET, &no_index)
                    .iter()
                    .map(|p| key(p)),
            );
            let r = crate::retrieval_broker::retrieve_with(
                &crate::retrieval_broker::Request {
                    vault: v,
                    query: case.q,
                    scope: &all,
                    kinds: None,
                },
                &no_bm25,
                "test",
                &|_: &str| Err("semantic search is off".into()),
            );
            let evidence = crate::retrieval_broker::evidence(r.passages);
            let rel_of = |abs: &str| {
                Path::new(abs)
                    .strip_prefix(v)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            };
            let after = distinct(evidence.iter().map(|p| key(&rel_of(&p.path))));
            let recall = crate::vault_learning::recall(v, case.q, &[]);
            let recalled: Vec<&'static str> = recall.items.iter().map(|i| key(&i.path)).collect();
            let (b, a, rc) = (
                metrics(&before, case.grades),
                metrics(&after, case.grades),
                metrics(&recalled, case.grades),
            );
            eprintln!(
                "{:<4} {:<60} | {:>6.3} {:>4.2} | {:>6.3} {:>4.2} | {:>6.3} {:>4.2} | {:?} / recall {:?} (before {:?})",
                case.id, case.q, b.0, b.1, a.0, a.1, rc.0, rc.1, after, recalled, before
            );
            for (route, keys) in [("retrieval", &after), ("recall", &recalled)] {
                let at = |k: &str| keys.iter().position(|x| *x == k);
                for gone in case.absent {
                    assert!(
                        at(gone).is_none(),
                        "{} {route}: {gone} in {keys:?}",
                        case.id
                    );
                }
                // The correction outranks the old statement: the old one is
                // absent (current state) or behind it (history).
                if case.historical {
                    assert!(
                        matches!((at("new"), at("old")), (Some(n), Some(o)) if n < o),
                        "{} {route}: {keys:?}",
                        case.id
                    );
                } else {
                    assert!(at("new").is_some(), "{} {route}: {keys:?}", case.id);
                }
            }
            if case.historical {
                let old = evidence
                    .iter()
                    .find(|p| p.path.ends_with(OLD_CADENCE))
                    .unwrap();
                assert!(
                    old.lifecycle
                        .as_deref()
                        .is_some_and(|l| l.starts_with("historical: superseded by")),
                    "{:?}",
                    old.lifecycle
                );
                let item = recall.items.iter().find(|i| i.path == OLD_CADENCE).unwrap();
                assert!(item
                    .lifecycle
                    .as_deref()
                    .is_some_and(|l| l.starts_with("historical")));
            } else {
                assert!(
                    before.contains(&"old"),
                    "{} documents the defect: {before:?}",
                    case.id
                );
            }
            // Recall's ordering matches: the correction first.
            assert_eq!(recalled.first(), Some(&"new"), "{} recall", case.id);
        }
    }

    #[test]
    fn judged_queries_rank_identically_with_and_without_the_prefilter() {
        let dir = write_corpus();
        let v = dir.path();
        let db = tempfile::tempdir().unwrap();
        let idx = RefCell::new(SearchIndex::open(&db.path().join("idx.db")).expect("index"));
        let narrowed = Cell::new(false);
        let with_index = |terms: &[String], notes: &[Note]| -> TermCandidates {
            let sets = idx.borrow_mut().reconcile_and_query_terms(notes, terms, v);
            if let Some(sets) = &sets {
                if sets.iter().flatten().any(|s| s.len() < notes.len()) {
                    narrowed.set(true);
                }
            }
            sets
        };
        fn project(hits: &[SearchResult]) -> Vec<(String, i64, String, String)> {
            hits.iter()
                .map(|h| (h.path.clone(), h.score, h.snippet.clone(), h.kind.clone()))
                .collect()
        }
        let kind_sets: [Option<Vec<String>>; 3] = [
            None,
            Some(vec!["task".to_string()]),
            Some(vec!["meeting".to_string(), "recording".to_string()]),
        ];
        for case in CASES {
            for kinds in &kind_sets {
                for limit in [1, 5, 50] {
                    let linear = search_with(v, case.q, limit, kinds.as_deref(), &no_index);
                    let indexed = search_with(v, case.q, limit, kinds.as_deref(), &with_index);
                    assert_eq!(
                        project(&linear),
                        project(&indexed),
                        "{} kinds {kinds:?} limit {limit}",
                        case.id
                    );
                }
            }
        }
        assert!(
            narrowed.get(),
            "the index never narrowed a term's candidates"
        );
        // Stem-only terms: FTS excludes the note for that term, yet the note
        // is still found through its name. Dropping the `!name_has` guard from
        // the skip rule would fail here.
        let notes = crate::vault::walk_notes(v);
        for (q, rel, stem_only) in [
            ("zircon plan", "notes/Zircon plan.md", "zircon"),
            (
                "standup 2026-09-10",
                "notes/Standup 2026-09-10.md",
                "2026-09-10",
            ),
            (
                "Acme pricing decision.md",
                "notes/Acme pricing decision.md",
                "decision.md",
            ),
            (
                "build-mobile ui",
                "tasks/build-mobile-ui.md",
                "build-mobile",
            ),
        ] {
            let sets = idx
                .borrow_mut()
                .reconcile_and_query_terms(&notes, &[stem_only.to_string()], v)
                .expect("candidate query");
            assert!(
                !sets[0].as_ref().expect("indexed term").contains(rel),
                "{stem_only:?} should not be in {rel}'s indexed body"
            );
            let hits = search_with(v, q, 50, None, &with_index);
            assert!(
                hits.iter().any(|h| h.path == rel),
                "{q:?} lost {rel}: {:?}",
                keys_of(&hits)
            );
        }
    }
}
