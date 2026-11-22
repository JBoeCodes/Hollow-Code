//! Multi-term lexical vault search: Hollow's port of Paperclip's task-search
//! contract (`server/src/services/task-search.ts` and `doc/SEARCH.md` at
//! paperclip `efce935`), adapted to plain Markdown files. `vault::search` is a
//! one-line delegate to [`search`], so the Tauri command, the `search_vault` MCP
//! tool and every caller share this one implementation.
//!
//! GRAMMAR ([`Query::parse`])
//! - The first 200 characters are trimmed, whitespace-collapsed and lowercased.
//!   `"double quotes"` keep an exact phrase as one term.
//! - Unquoted terms lose leading/trailing sentence punctuation (`launch?`,
//!   `acme,`, `(api)`). Internal punctuation and `%` stay literal (`foo_bar`,
//!   `100%`, `node.js`, `decision.md`).
//! - Grammatical filler (a an the and of to for in on with) and bare
//!   punctuation are dropped unless nothing else remains. Quoted filler is
//!   always kept. Repeats collapse, and a quoted repeat wins. At most 8 terms.
//! - EVERY term must match, in any order, in the note's file name or its body.
//! - A 1-3 letter term written in a spaced script (Latin, Greek, Cyrillic) must
//!   begin a word: `ui` never matches "build" and `api` never matches
//!   "Capistrano". Scripts written without spaces (CJK, kana, hangul) keep plain
//!   substring semantics, and their letters count as a word boundary. Every
//!   other term is a plain substring of the lowercased text.
//!
//! ADMISSION reads only the file name (the stem, the `-`/`_`-normalized stem,
//! and the full name for terms that contain a `.`) and `lower(content)`, which
//! is exactly the text the FTS5 prefilter indexes. That is what keeps the
//! prefilter a pure optimization; the proof is in `vault_search_index`. A
//! derived frontmatter title ranks but never admits: `frontmatter::unquote`
//! decodes escapes, so a title is not always a substring of the content.
//!
//! RANKING uses disjoint bands, and bonuses (at most `MAX_BONUS` < 1000) only
//! order notes inside a band: exact title (6000) > title phrase (5000) > every
//! term in the title (4000) > every term in the title or a heading (3000) >
//! body (2000) > file-name typo recovery (1000). Ties break by mtime (newest
//! first), then path, so the order is total and independent of the directory
//! walk order. There is no early exit: every prefilter candidate is scored
//! before truncation.
//!
//! TYPO RECOVERY runs only when the literal pass found nothing (after the kind
//! filter), never when a term is quoted, and only for unquoted 4-255 letter
//! terms: 2 edits when the shorter word has at least 6 letters, 1 edit at 5,
//! otherwise exact. It matches FILE-NAME stems, not frontmatter titles. Hollow's
//! writers slug file names from titles, and matching titles would mean reading
//! every note's frontmatter on each zero-hit keystroke.
//!
//! The module also owns the line scorer shared by both halves of the host
//! retrieval broker (`retrieval_broker`: [`best_lines`] and the vault scan
//! [`retrieval_scan_with`]), so vault and project evidence carry the same
//! coverage numbers by construction.

use crate::types::{Note, SearchResult};
use once_cell::sync::Lazy;
use regex::Regex;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

/// Characters of the raw query that are considered at all.
pub const MAX_QUERY_CHARS: usize = 200;
/// Terms kept after filler removal and de-duplication.
pub const MAX_TERMS: usize = 8;
/// Paperclip's deliberately small filler list. Question words stay meaningful.
const FILLER: [&str; 10] = [
    "a", "an", "the", "and", "of", "to", "for", "in", "on", "with",
];
/// Trimmed from both ends of UNQUOTED terms only.
const EDGE_PUNCT: &[char] = &[
    '?', '!', '.', ',', ';', ':', '(', ')', '[', ']', '{', '}', '<', '>', '"', '\'', '`',
    '\u{201c}', '\u{201d}', '\u{2018}', '\u{2019}', '\u{ab}', '\u{bb}', '\u{2026}',
];

pub(crate) const BAND_EXACT_TITLE: i64 = 6000;
pub(crate) const BAND_TITLE_PHRASE: i64 = 5000;
pub(crate) const BAND_TITLE_ALL: i64 = 4000;
pub(crate) const BAND_HEADING_ALL: i64 = 3000;
pub(crate) const BAND_BODY_ALL: i64 = 2000;
pub(crate) const BAND_TITLE_TYPO: i64 = 1000;

/// Per term that matches a title as a whole word.
const WORD_BONUS: i64 = 10;
/// A title starts with the whole phrase.
const STARTS_BONUS: i64 = 30;
/// A multi-term phrase occurs contiguously in the note.
const BODY_PHRASE_BONUS: i64 = 100;
/// Per distinct term on the best single line.
const LINE_COVERAGE_BONUS: i64 = 20;
/// Bonuses can never lift a note into the next band.
const MAX_BONUS: i64 = MAX_TERMS as i64 * WORD_BONUS
    + STARTS_BONUS
    + BODY_PHRASE_BONUS
    + MAX_TERMS as i64 * LINE_COVERAGE_BONUS;
const _: () = assert!(MAX_BONUS < 1000);

/// Retrieval skips (and counts) vault notes larger than this. It mirrors
/// `directory_connections`' 1 MB text-reader limit for project files.
pub(crate) const RETRIEVAL_MAX_NOTE_BYTES: u64 = 1_048_576;
/// The hard bound on bytes one retrieval pass reads, matching or not. With
/// an FTS index only candidate notes are read, so this binds only a linear
/// scan (no index in this process, or a failed one) of a very large vault.
pub(crate) const RETRIEVAL_READ_BYTES: u64 = 256 * 1024 * 1024;
/// BM25 bands (OB12): a note's band is how many times the best score
/// divides by this before it reaches the note's (`floor(log(best / score))`
/// in this base), so band 0 is within a factor of 1.25 of the best. Line
/// coverage orders the notes inside a band.
const BAND_RATIO: f64 = 1.25;
/// The band of a candidate the index didn't rank (a name match, a newest
/// note, a mid-word `topup` match); ranked bands stop just short of it. An
/// `extra` note (one the ranking can't speak for) is band 0 instead.
const UNRANKED_BAND: usize = 32;
/// The newest eligible notes read whatever their rank.
const RECENT_CANDIDATES: usize = 20;
/// Notes whose stem names a term read whatever their rank, most names and
/// then newest first.
const NAME_CANDIDATES: usize = 50;

/// A size as the notices say it: whole MB, else bytes.
fn size_words(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{} MB", bytes / (1024 * 1024))
    } else {
        format!("{bytes} bytes")
    }
}

/// The notice when the matching notes outgrow the scoring budget.
pub(crate) fn retrieval_budget_notice(budget: u64) -> String {
    format!(
        "Vault retrieval scored {} of matching notes, newest first, and stopped; use search_vault for a targeted query.",
        size_words(budget)
    )
}

/// The notice when a pass reads its hard limit before finishing.
pub(crate) fn retrieval_read_notice(limit: u64) -> String {
    format!(
        "Vault retrieval read its {} limit before finishing (no search index in this process), so older notes may be missing; use search_vault for a targeted query.",
        size_words(limit)
    )
}

/// Per-term FTS5 candidate sets, index-aligned with the query's terms. The
/// outer `None` means "no index" (scan everything); an inner `None` means that
/// term is unknown to the index (shorter than 3 characters).
pub type TermCandidates = Option<Vec<Option<HashSet<String>>>>;

/// Retrieval's candidates from the FTS index (OB12,
/// `vault_search_index::ranked_candidates`). `None` means no index: every
/// eligible note is read, in one band (the linear scan).
pub type RetrievalCandidates = Option<RankedCandidates>;

/// The notes the word index ranks for a retrieval, and the ones it can't.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RankedCandidates {
    /// Eligible notes, best first, with their BM25 score (higher is better):
    /// at most `vault_search_index::BM25_CANDIDATES`.
    pub ranked: Vec<(String, f64)>,
    /// Notes the ranking can't speak for, read in the best band: trigram
    /// matches of a term that isn't one word, and (read-only) notes changed
    /// since the index saw them.
    pub extra: HashSet<String>,
    /// When fewer than `BM25_CANDIDATES` rank, the trigram matches of every
    /// word term (a term inside a longer word, which the word index can't
    /// see): read, behind every ranked note. At most the rest of
    /// `BM25_CANDIDATES` and 4 MB, best trigram BM25 first (RF-737).
    pub topup: HashSet<String>,
}

static TERM_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#""([^"]+)"|([^\s"]+)"#).unwrap());

/// A letter of a script written with spaces between words. Only these get
/// the short-term word-start rule.
fn spaced_letter(c: char) -> bool {
    c.is_alphabetic()
        && (c.is_ascii_alphabetic()
            || matches!(
                c as u32,
                0x00C0..=0x024F | 0x1E00..=0x1EFF | 0x0370..=0x03FF | 0x1F00..=0x1FFF | 0x0400..=0x052F
            ))
}

/// A character that continues a word for the word-start rule. Punctuation,
/// whitespace, `_` and letters of unspaced scripts are boundaries.
pub(crate) fn joins_word(c: char) -> bool {
    c.is_numeric() || spaced_letter(c)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Term {
    pub text: String,
    pub quoted: bool,
}

impl Term {
    /// 1-3 spaced-script letters: must match at the start of a word.
    pub fn word_start(&self) -> bool {
        let n = self.text.chars().count();
        (1..=3).contains(&n) && self.text.chars().all(spaced_letter)
    }

    /// Eligible for typo recovery: unquoted, 4-255 letters.
    fn fuzzable(&self) -> bool {
        let n = self.text.chars().count();
        !self.quoted && (4..=255).contains(&n) && self.text.chars().all(char::is_alphabetic)
    }

    /// Long enough for the trigram FTS5 index.
    pub fn indexable(&self) -> bool {
        self.text.chars().count() >= 3
    }

    fn has_letter(&self) -> bool {
        self.text.chars().any(char::is_alphabetic)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub terms: Vec<Term>,
    /// The terms joined by single spaces.
    pub phrase: String,
}

impl Query {
    pub fn parse(raw: &str) -> Query {
        let truncated: String = raw.chars().take(MAX_QUERY_CHARS).collect();
        let normalized = truncated
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let parsed: Vec<Term> = TERM_RE
            .captures_iter(&normalized)
            .filter_map(|c| {
                if let Some(quoted) = c.get(1) {
                    Some(Term {
                        text: quoted.as_str().to_string(),
                        quoted: true,
                    })
                } else {
                    c.get(2).map(|m| Term {
                        text: m.as_str().to_string(),
                        quoted: false,
                    })
                }
            })
            .collect();
        let meaningful: Vec<Term> = parsed
            .iter()
            .filter_map(|t| {
                if t.quoted {
                    return Some(t.clone());
                }
                let text = t.text.trim_matches(EDGE_PUNCT);
                let keep = !text.is_empty()
                    && !FILLER.contains(&text)
                    && text.chars().any(char::is_alphanumeric);
                keep.then(|| Term {
                    text: text.to_string(),
                    quoted: false,
                })
            })
            .collect();
        let source = if meaningful.is_empty() {
            parsed
        } else {
            meaningful
        };
        let mut terms: Vec<Term> = Vec::new();
        for term in source {
            match terms.iter_mut().find(|t| t.text == term.text) {
                Some(existing) => existing.quoted |= term.quoted,
                None => terms.push(term),
            }
        }
        terms.truncate(MAX_TERMS);
        let phrase = terms
            .iter()
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        Query { terms, phrase }
    }
}

/// The first acceptable occurrence of `t` in already-lowercased text. A
/// word-start term skips occurrences that continue a word, so every
/// occurrence has to be considered, not only the first.
pub fn find_term(hay_lower: &str, t: &Term) -> Option<usize> {
    if t.text.is_empty() {
        return None;
    }
    let word_start = t.word_start();
    hay_lower
        .match_indices(t.text.as_str())
        .map(|(i, _)| i)
        .find(|&i| {
            !word_start
                || hay_lower[..i]
                    .chars()
                    .next_back()
                    .is_none_or(|p| !joins_word(p))
        })
}

pub fn has_term(hay_lower: &str, t: &Term) -> bool {
    find_term(hay_lower, t).is_some()
}

/// Some occurrence of `t` has a word boundary on both sides.
fn whole_word(hay_lower: &str, t: &Term) -> bool {
    if t.text.is_empty() {
        return false;
    }
    hay_lower.match_indices(t.text.as_str()).any(|(i, m)| {
        let before = hay_lower[..i].chars().next_back();
        let after = hay_lower[i + m.len()..].chars().next();
        before.is_none_or(|c| !joins_word(c)) && after.is_none_or(|c| !joins_word(c))
    })
}

/// Lowercase, `-`/`_` → space, whitespace collapsed.
fn normalize_title(s: &str) -> String {
    s.to_lowercase()
        .replace(['-', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A file name without its `.md` / `.html` / `.htm` extension.
pub(crate) fn stem(name: &str) -> &str {
    for ext in [".md", ".html", ".htm"] {
        if name.len() >= ext.len() {
            let split = name.len() - ext.len();
            if name
                .get(split..)
                .is_some_and(|s| s.eq_ignore_ascii_case(ext))
            {
                return &name[..split];
            }
        }
    }
    name
}

/// Markdown ATX heading lines of a body, lowercased and joined by `\n`.
fn headings_lower(body: &str) -> String {
    body.lines()
        .filter(|line| {
            let t = line.trim_start();
            let hashes = t.chars().take_while(|c| *c == '#').count();
            (1..=6).contains(&hashes)
                && (t[hashes..].starts_with(' ') || t[hashes..].starts_with('\t'))
        })
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Byte offset where the body starts. Exact because `frontmatter::parse`
/// always returns the body as a suffix of the content.
fn frontmatter_end(content: &str) -> usize {
    content.len() - crate::frontmatter::parse(content).body.len()
}

/// Each line of `content` without its terminator, with its byte offset. Line
/// numbering matches `str::lines`.
fn lines_with_offsets(content: &str) -> impl Iterator<Item = (usize, &str)> + '_ {
    let mut offset = 0usize;
    content.split_inclusive('\n').map(move |raw| {
        let start = offset;
        offset += raw.len();
        let line = match raw.strip_suffix('\n') {
            Some(l) => l.strip_suffix('\r').unwrap_or(l),
            None => raw,
        };
        (start, line)
    })
}

/// Paperclip's bounded edit distance: 2 edits when the shorter word has at
/// least 6 letters, 1 at 5, otherwise exact. Both words must be 4-255 letters
/// long and differ in length by at most the allowed edits.
fn within_edits(term: &str, word: &str) -> bool {
    let a: Vec<char> = term.chars().collect();
    let b: Vec<char> = word.chars().collect();
    if !(4..=255).contains(&a.len()) || !(4..=255).contains(&b.len()) {
        return false;
    }
    let shorter = a.len().min(b.len());
    let edits = if shorter >= 6 {
        2
    } else if shorter >= 5 {
        1
    } else {
        0
    };
    if a.len().abs_diff(b.len()) > edits {
        return false;
    }
    if edits == 0 {
        return a == b;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        let mut row_min = cur[0];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            row_min = row_min.min(cur[j]);
        }
        if row_min > edits {
            return false;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()] <= edits
}

/// Bit `i` is set iff term `i` occurs in `hay_lower`. At most 32 terms count;
/// callers pass at most 16.
pub(crate) fn term_mask(terms: &[Term], hay_lower: &str) -> u32 {
    let mut mask = 0u32;
    for (i, t) in terms.iter().enumerate().take(32) {
        if has_term(hay_lower, t) {
            mask |= 1 << i;
        }
    }
    mask
}

/// Like [`term_mask`] over several already-lowercased names, but only for
/// terms that contain a letter. A bare number such as `2026` must not
/// name-match every date-slugged calendar or recording file.
pub(crate) fn name_mask(terms: &[Term], names: &[&str]) -> u32 {
    let mut mask = 0u32;
    for (i, t) in terms.iter().enumerate().take(32) {
        if t.has_letter() && names.iter().any(|n| has_term(n, t)) {
            mask |= 1 << i;
        }
    }
    mask
}

/// The name variants that may admit a note. Admission, the FTS skip rule, typo
/// recovery and the title band all go through [`NoteNames::has`].
struct NoteNames {
    name_lower: String,
    stem_raw: String,
    stem_norm: String,
}

impl NoteNames {
    fn new(name: &str) -> NoteNames {
        let stem_raw = stem(name).to_lowercase();
        let stem_norm = normalize_title(&stem_raw);
        NoteNames {
            name_lower: name.to_lowercase(),
            stem_raw,
            stem_norm,
        }
    }

    /// A term without a `.` can only hit the full name inside the extension
    /// (`md`, `htm`, `html`), which is noise, so only dotted terms look there.
    fn has(&self, t: &Term) -> bool {
        has_term(&self.stem_raw, t)
            || has_term(&self.stem_norm, t)
            || (t.text.contains('.') && has_term(&self.name_lower, t))
    }
}

/// Whether a request's `terms` name a note (HF05, the oversized notice's
/// rule): a term with a letter in its stem or, dotted, in its full name.
fn request_names(names: &NoteNames, terms: &[Term]) -> bool {
    terms.iter().any(|t| t.has_letter() && names.has(t))
}

/// [`request_names`] for note file `name`: whether a request with `terms`
/// would hear that an oversized note of that name was skipped (OF17's
/// replay probe).
#[cfg(test)]
pub(crate) fn oversized_notice_for(name: &str, terms: &[Term]) -> bool {
    request_names(&NoteNames::new(name), terms)
}

/// A note file name's lowercased stem and its `-`/`_`-normalized form, the
/// two names retrieval matches terms against (evaluation copies use it).
#[cfg(test)]
pub(crate) fn stem_names(name: &str) -> (String, String) {
    let n = NoteNames::new(name);
    (n.stem_raw, n.stem_norm)
}

/// Keyword search over the vault. See the module doc for the grammar and the
/// ranking. `kinds` applies before the cap.
pub fn search(
    vault: &Path,
    query: &str,
    limit: usize,
    kinds: Option<&[String]>,
) -> Vec<SearchResult> {
    search_with(
        vault,
        query,
        limit,
        kinds,
        &|terms: &[String], notes: &[Note]| {
            crate::vault_search_index::term_candidates(vault, terms, notes)
        },
    )
}

/// [`search`] with an explicit prefilter, so tests can compare the linear
/// scan with a local index without touching the process-global one.
pub(crate) fn search_with(
    vault: &Path,
    query: &str,
    limit: usize,
    kinds: Option<&[String]>,
    prefilter: &dyn Fn(&[String], &[Note]) -> TermCandidates,
) -> Vec<SearchResult> {
    if query.len() < 2 {
        return Vec::new();
    }
    let q = Query::parse(query);
    if q.terms.is_empty() {
        return Vec::new();
    }
    let notes = crate::vault::walk_notes(vault);
    let texts: Vec<String> = q.terms.iter().map(|t| t.text.clone()).collect();
    let sets = prefilter(&texts, &notes).filter(|v| v.len() == q.terms.len());
    let mut hits: Vec<SearchResult> = Vec::new();
    for note in &notes {
        let names = NoteNames::new(&note.name);
        if let Some(sets) = &sets {
            // A term that is in neither the index's candidates nor the name can
            // never be satisfied, so the note is skipped without a read.
            let provably_absent = q.terms.iter().zip(sets.iter()).any(|(t, set)| {
                t.indexable()
                    && matches!(set, Some(s) if !s.contains(note.path.as_str()))
                    && !names.has(t)
            });
            if provably_absent {
                continue;
            }
        }
        let content = match fs::read_to_string(vault.join(&note.path)) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let lower = content.to_lowercase();
        if !q.terms.iter().all(|t| names.has(t) || has_term(&lower, t)) {
            continue;
        }
        // Classify only admitted notes; `parse_map` returns the body as a
        // suffix of the content, so its length locates the frontmatter end.
        let (fm, body) = crate::frontmatter::parse_map(&content);
        let fm_end = content.len() - body.len();
        let item = crate::vault_index::classify(&note.path, &fm);
        if let Some(allow) = kinds {
            if !allow.iter().any(|k| k == &item.kind) {
                continue;
            }
        }
        let line = best_search_line(&q.terms, &content, fm_end);
        let score = score_admitted(
            &q,
            &names,
            &item.title,
            &body,
            &lower,
            line.as_ref().map_or(0, |l| l.count),
        );
        let snippet = match line {
            Some(l) => {
                let start = crate::vault::char_floor(&content, l.offset.saturating_sub(60));
                let end = crate::vault::char_ceil(
                    &content,
                    (l.offset + l.term_len + 100).min(content.len()),
                );
                content[start..end].to_string()
            }
            None => String::new(),
        };
        hits.push(SearchResult {
            path: note.path.clone(),
            name: note.name.clone(),
            mtime: note.mtime.clone(),
            size: note.size,
            snippet,
            score,
            kind: item.kind,
        });
    }
    if hits.is_empty() {
        hits = title_typo_hits(vault, &notes, &q, kinds);
    }
    sort_results(&mut hits);
    hits.truncate(limit);
    hits
}

/// Score desc, then mtime desc, then path asc: a total order.
fn sort_results(hits: &mut [SearchResult]) {
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| b.mtime.cmp(&a.mtime))
            .then_with(|| a.path.cmp(&b.path))
    });
}

/// The line that best explains a match, for the snippet and the line bonus.
struct SearchLine {
    /// Byte offset of the line's first matched term (approximate when
    /// lowercasing changes byte lengths; callers snap to char boundaries).
    offset: usize,
    term_len: usize,
    /// Distinct terms on the line.
    count: usize,
}

/// Most distinct terms wins; ties prefer body lines over frontmatter, then the
/// earlier line.
fn best_search_line(terms: &[Term], content: &str, fm_end: usize) -> Option<SearchLine> {
    let mut best: Option<(bool, SearchLine)> = None;
    for (start, line) in lines_with_offsets(content) {
        let lower = line.to_lowercase();
        let mut count = 0usize;
        let mut first: Option<(usize, usize)> = None;
        for t in terms {
            if let Some(i) = find_term(&lower, t) {
                count += 1;
                if first.is_none_or(|(fi, _)| i < fi) {
                    first = Some((i, t.text.len()));
                }
            }
        }
        let Some((i, term_len)) = first else {
            continue;
        };
        let in_body = start >= fm_end;
        let better = match &best {
            None => true,
            Some((best_in_body, b)) => {
                count > b.count || (count == b.count && in_body && !best_in_body)
            }
        };
        if better {
            best = Some((
                in_body,
                SearchLine {
                    offset: start + i,
                    term_len,
                    count,
                },
            ));
        }
    }
    best.map(|(_, line)| line)
}

fn score_admitted(
    q: &Query,
    names: &NoteNames,
    derived_title: &str,
    body: &str,
    lower: &str,
    best_line_count: usize,
) -> i64 {
    let derived_lower = derived_title.to_lowercase();
    let derived_norm = normalize_title(&derived_lower);
    let mut titles: Vec<&str> = Vec::with_capacity(4);
    for t in [
        names.stem_raw.as_str(),
        names.stem_norm.as_str(),
        derived_lower.as_str(),
        derived_norm.as_str(),
    ] {
        if !t.is_empty() && !titles.contains(&t) {
            titles.push(t);
        }
    }
    let phrase = q.phrase.as_str();
    let title_has = |t: &Term| {
        titles.iter().any(|x| has_term(x, t))
            || (t.text.contains('.') && has_term(&names.name_lower, t))
    };
    let all_title = q.terms.iter().all(&title_has);
    let band = if titles.contains(&phrase) || names.name_lower == phrase {
        BAND_EXACT_TITLE
    } else if all_title
        && (titles.iter().any(|x| x.contains(phrase)) || names.name_lower.contains(phrase))
    {
        BAND_TITLE_PHRASE
    } else if all_title {
        BAND_TITLE_ALL
    } else {
        let headings = headings_lower(body);
        if q.terms
            .iter()
            .all(|t| title_has(t) || has_term(&headings, t))
        {
            BAND_HEADING_ALL
        } else {
            BAND_BODY_ALL
        }
    };
    let word_hits = q
        .terms
        .iter()
        .filter(|t| titles.iter().any(|x| whole_word(x, t)))
        .count() as i64;
    let mut bonus = (word_hits * WORD_BONUS).min(MAX_TERMS as i64 * WORD_BONUS);
    if titles.iter().any(|x| x.starts_with(phrase)) {
        bonus += STARTS_BONUS;
    }
    if q.terms.len() >= 2 && lower.contains(phrase) {
        bonus += BODY_PHRASE_BONUS;
    }
    bonus +=
        (best_line_count as i64 * LINE_COVERAGE_BONUS).min(MAX_TERMS as i64 * LINE_COVERAGE_BONUS);
    band + bonus
}

/// Zero-literal-hit recovery against file-name stems. Never consults FTS:
/// it reads nothing until a stem matches, so it stays outside the parity
/// argument.
fn title_typo_hits(
    vault: &Path,
    notes: &[Note],
    q: &Query,
    kinds: Option<&[String]>,
) -> Vec<SearchResult> {
    if q.terms.iter().any(|t| t.quoted) || !q.terms.iter().any(Term::fuzzable) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for note in notes {
        let names = NoteNames::new(&note.name);
        let words: Vec<&str> = names
            .stem_norm
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect();
        let keep = q.terms.iter().all(|t| {
            names.has(t) || (t.fuzzable() && words.iter().any(|w| within_edits(&t.text, w)))
        });
        if !keep {
            continue;
        }
        let content = match fs::read_to_string(vault.join(&note.path)) {
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
        let word_hits = q
            .terms
            .iter()
            .filter(|t| whole_word(&names.stem_raw, t) || whole_word(&names.stem_norm, t))
            .count() as i64;
        out.push(SearchResult {
            path: note.path.clone(),
            name: note.name.clone(),
            mtime: note.mtime.clone(),
            size: note.size,
            snippet: String::new(),
            score: BAND_TITLE_TYPO + WORD_BONUS * word_hits,
            kind,
        });
    }
    out
}

// ── Retrieval evidence (shared by both halves of directory retrieval) ─────────

/// One cited line. `coverage` = distinct terms in the line plus terms in the
/// file's name; `in_line` = distinct terms in the line alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LineEvidence {
    /// 1-based, numbered like `str::lines`.
    pub line: usize,
    pub excerpt: String,
    pub coverage: usize,
    pub in_line: usize,
}

fn excerpt(line: &str) -> String {
    line.chars().take(900).collect()
}

/// The part of a frontmatter line that may match a term: the value after a
/// `key:`, or the whole line when it has no key (a list item, a fence).
/// Matching the key too would let a prompt word such as "status", "project" or
/// "due" match that key in every note that has it.
fn frontmatter_value(line: &str) -> &str {
    match line.trim_start().split_once(':') {
        Some((key, value)) if !key.is_empty() && !key.contains(char::is_whitespace) => value,
        _ => line,
    }
}

/// The best `max` lines of `content` for `terms`, ordered by (coverage desc,
/// in_line desc, body before frontmatter, line asc). A line is a candidate only
/// if it matches a term on its own, so blank lines and `---` fences never
/// become evidence. A frontmatter line matches on its value only, so a key
/// such as `status:` or `project:` never matches a prompt word. `name_mask`
/// only boosts candidate lines. A file with no matching line but a name match
/// yields exactly one name-only passage (in_line 0) at its first non-blank body
/// line. One file's worth of [`LineScorer::best_lines`], which retrieval
/// builds once per request (test-only: the evaluation and the unit tests).
#[cfg(test)]
pub(crate) fn best_lines(
    terms: &[Term],
    content: &str,
    name_mask: u32,
    max: usize,
) -> Vec<LineEvidence> {
    LineScorer::new(terms).best_lines(content, name_mask, max)
}

/// [`best_lines`] for many files with one set of terms (run 15 S1: retrieval
/// scores ~32 MB of connected files on every chat turn). Each term's
/// substring finder is built once, and a file's body is lowercased once and
/// searched once per term, instead of each line being lowercased and
/// searched once per term. The lines and their masks are exactly
/// [`best_lines_by_line`]'s: a term never holds a line break
/// (`retrieval_words` keeps alphanumeric runs), lowercasing never adds or
/// removes one, a line break before an occurrence is a word boundary just as
/// a line's start is, and occurrences are found left to right without
/// overlap within each line either way. Frontmatter lines are still matched
/// one by one, on their value only.
pub(crate) struct LineScorer<'t> {
    terms: &'t [Term],
    finders: Vec<Option<memchr::memmem::Finder<'t>>>,
    /// A term holding a line break (never one of `retrieval_words`) could
    /// match across lines in a whole-body search: then every call answers
    /// line by line, as before.
    by_line: bool,
}

#[cfg(test)]
thread_local! {
    /// `turn_prep_probe`'s "before": every [`LineScorer`] answers with the
    /// line-by-line reference instead, so one run can time both and show the
    /// evidence is the same.
    pub(crate) static BY_LINE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

impl<'t> LineScorer<'t> {
    pub(crate) fn new(terms: &'t [Term]) -> Self {
        let finders = terms
            .iter()
            .take(32)
            .map(|t| (!t.text.is_empty()).then(|| memchr::memmem::Finder::new(t.text.as_bytes())))
            .collect();
        let by_line = terms.iter().any(|t| t.text.contains(['\n', '\r']));
        LineScorer {
            terms,
            finders,
            by_line,
        }
    }

    /// Whether this call answers with the line-by-line reference.
    fn by_line(&self) -> bool {
        #[cfg(test)]
        if BY_LINE.with(|c| c.get()) {
            return true;
        }
        self.by_line
    }

    /// [`term_mask`] over already-lowercased text, with the prepared finders.
    pub(crate) fn term_mask(&self, hay_lower: &str) -> u32 {
        if self.by_line() {
            return term_mask(self.terms, hay_lower);
        }
        let mut mask = 0u32;
        for (i, (t, finder)) in self.terms.iter().zip(&self.finders).enumerate() {
            let Some(finder) = finder else { continue };
            let word_start = t.word_start();
            let found = finder.find_iter(hay_lower.as_bytes()).any(|at| {
                !word_start
                    || hay_lower[..at]
                        .chars()
                        .next_back()
                        .is_none_or(|p| !joins_word(p))
            });
            if found {
                mask |= 1 << i;
            }
        }
        mask
    }

    /// [`best_lines`] for one file.
    pub(crate) fn best_lines(
        &self,
        content: &str,
        name_mask: u32,
        max: usize,
    ) -> Vec<LineEvidence> {
        if self.by_line() {
            return best_lines_by_line(self.terms, content, name_mask, max);
        }
        let terms = self.terms;
        let fm_end = frontmatter_end(content);
        // (coverage, in_line, in_frontmatter, line number, text)
        let mut found: Vec<(usize, usize, bool, usize, &str)> = Vec::new();
        let mut first_frontmatter: Option<(usize, &str)> = None;
        // Frontmatter lines (those starting before fm_end), one by one; the
        // body starts at the first line starting at or past it.
        let (mut body_start, mut body_line) = (content.len(), 0usize);
        for (index, (start, line)) in lines_with_offsets(content).enumerate() {
            if start >= fm_end {
                (body_start, body_line) = (start, index);
                break;
            }
            body_line = index + 1;
            let mask = term_mask(terms, &frontmatter_value(line).to_lowercase());
            if mask != 0 {
                found.push((
                    (mask | name_mask).count_ones() as usize,
                    mask.count_ones() as usize,
                    true,
                    index + 1,
                    line,
                ));
            } else if name_mask != 0
                && !line.trim().is_empty()
                && line.trim() != "---"
                && first_frontmatter.is_none()
            {
                first_frontmatter = Some((index + 1, line));
            }
        }
        // The body: lowercased once, each term's occurrences mapped to the
        // line they sit on by the line breaks before them.
        let body = &content[body_start..];
        let lower = body.to_lowercase();
        let hay = lower.as_bytes();
        let mut masks: std::collections::BTreeMap<usize, u32> = std::collections::BTreeMap::new();
        let mut breaks: Option<Vec<usize>> = None;
        for (i, (t, finder)) in terms.iter().zip(&self.finders).enumerate() {
            let Some(finder) = finder else { continue };
            let word_start = t.word_start();
            let step = t.text.len().max(1);
            // Left to right without overlap, as `match_indices` and
            // `find_iter` go; once a line holds the term, the rest of that
            // line is skipped (its mask can't change), just as the reference
            // stops at a line's first accepted occurrence (review S1 F1: a
            // line repeating a term must not cost one step per repeat).
            let mut pos = 0usize;
            while let Some(found) = finder.find(&hay[pos..]) {
                let at = pos + found;
                // A match of a whole UTF-8 needle starts on a char boundary.
                if word_start && lower[..at].chars().next_back().is_some_and(joins_word) {
                    pos = at + step;
                    continue;
                }
                let breaks = breaks
                    .get_or_insert_with(|| memchr::memchr_iter(b'\n', hay).collect::<Vec<_>>());
                let line = breaks.partition_point(|&n| n < at);
                *masks.entry(line).or_insert(0) |= 1 << i;
                match breaks.get(line) {
                    Some(&end) => pos = end + 1,
                    None => break,
                }
            }
        }
        // Matched body lines; with none, the first non-blank body line is
        // the name-only passage's place (used only when nothing matched).
        let mut first_body: Option<(usize, &str)> = None;
        match masks.keys().next_back().copied() {
            Some(last) => {
                for (index, (_, line)) in lines_with_offsets(body).enumerate().take(last + 1) {
                    if let Some(&mask) = masks.get(&index) {
                        found.push((
                            (mask | name_mask).count_ones() as usize,
                            mask.count_ones() as usize,
                            false,
                            body_line + index + 1,
                            line,
                        ));
                    }
                }
            }
            None if name_mask != 0 && found.is_empty() => {
                first_body = lines_with_offsets(body)
                    .enumerate()
                    .find(|(_, (_, line))| !line.trim().is_empty())
                    .map(|(index, (_, line))| (body_line + index + 1, line));
            }
            None => {}
        }
        ranked_evidence(found, first_body, first_frontmatter, name_mask, max)
    }
}

/// The reference [`best_lines`]: every line lowercased and searched for every
/// term on its own. [`LineScorer`] must give exactly its answer, and uses it
/// for a term holding a line break.
pub(crate) fn best_lines_by_line(
    terms: &[Term],
    content: &str,
    name_mask: u32,
    max: usize,
) -> Vec<LineEvidence> {
    let fm_end = frontmatter_end(content);
    // (coverage, in_line, in_frontmatter, line number, text)
    let mut found: Vec<(usize, usize, bool, usize, &str)> = Vec::new();
    let mut first_body: Option<(usize, &str)> = None;
    let mut first_frontmatter: Option<(usize, &str)> = None;
    for (index, (start, line)) in lines_with_offsets(content).enumerate() {
        let matchable = if start < fm_end {
            frontmatter_value(line)
        } else {
            line
        };
        let mask = term_mask(terms, &matchable.to_lowercase());
        if mask != 0 {
            found.push((
                (mask | name_mask).count_ones() as usize,
                mask.count_ones() as usize,
                start < fm_end,
                index + 1,
                line,
            ));
        } else if name_mask != 0 && !line.trim().is_empty() {
            if start >= fm_end {
                if first_body.is_none() {
                    first_body = Some((index + 1, line));
                }
            } else if line.trim() != "---" && first_frontmatter.is_none() {
                first_frontmatter = Some((index + 1, line));
            }
        }
    }
    ranked_evidence(found, first_body, first_frontmatter, name_mask, max)
}

/// The matched lines best first, cut to `max`; with none, the name-only
/// passage (first non-blank body line, else frontmatter line) when the name
/// matched.
fn ranked_evidence(
    mut found: Vec<(usize, usize, bool, usize, &str)>,
    first_body: Option<(usize, &str)>,
    first_frontmatter: Option<(usize, &str)>,
    name_mask: u32,
    max: usize,
) -> Vec<LineEvidence> {
    if found.is_empty() {
        if name_mask == 0 {
            return Vec::new();
        }
        return first_body
            .or(first_frontmatter)
            .map(|(line, text)| LineEvidence {
                line,
                excerpt: excerpt(text),
                coverage: name_mask.count_ones() as usize,
                in_line: 0,
            })
            .into_iter()
            .collect();
    }
    found.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.cmp(&a.1))
            .then(a.2.cmp(&b.2))
            .then(a.3.cmp(&b.3))
    });
    found.truncate(max);
    found
        .into_iter()
        .map(|(coverage, in_line, _, line, text)| LineEvidence {
            line,
            excerpt: excerpt(text),
            coverage,
            in_line,
        })
        .collect()
}

/// One vault passage for directory retrieval, keyed by vault-relative path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LinePassage {
    pub rel: String,
    pub line: usize,
    pub excerpt: String,
    pub kind: String,
    pub coverage: usize,
    pub in_line: usize,
    /// The note's mtime (the walk's RFC 3339 string). Retrieval breaks equal
    /// scores newest first with it, so a recurring meeting's latest note is
    /// cited instead of its lexically first (oldest date-slugged) sibling.
    pub mtime: String,
    /// The note's lifecycle (status, supersession, approval, date), read
    /// from the frontmatter this pass already parses (agent effectiveness
    /// F7, `note_lifecycle::of`). `None` for any note but a learning.
    pub note: Option<crate::note_lifecycle::Lifecycle>,
    /// The note's BM25 band (OB12, [`bm25_band`]; 0 is best), which the
    /// broker ranks before line coverage. 0 for every note of a linear scan.
    pub band: usize,
}

/// What one vault retrieval pass did, for the run's context manifest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ScanStats {
    /// Eligible notes (after `include`).
    pub eligible: usize,
    /// Notes chosen to be read: the index's candidates (OB12), or on a
    /// linear scan every eligible note under 1 MB.
    pub candidates: usize,
    /// Notes actually read.
    pub read: usize,
    /// Notes that matched a term (and the kind filter) and were scored.
    pub matched: usize,
    pub bytes_read: u64,
    /// Bytes charged to the scoring budget: matching notes only.
    pub bytes_charged: u64,
    /// The FTS index chose the candidates (BM25, OB12); `false`: a linear
    /// scan.
    pub indexed: bool,
    /// `budget` or `read-limit` when the pass stopped early.
    pub stopped: Option<&'static str>,
    /// Notes over 1 MB the request names (a term in the file's stem, or a
    /// dotted term in its name; never a folder), skipped and noticed. Others
    /// are skipped silently: they say nothing about this request (HF05).
    pub oversized: usize,
}

/// Bounds for one vault retrieval pass.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ScanLimits {
    /// Bytes of MATCHING notes scored, newest first.
    pub budget: u64,
    /// Bytes read in all, matching or not.
    pub read: u64,
    /// Best lines kept per note.
    pub lines: usize,
}

impl ScanLimits {
    #[cfg(test)]
    pub(crate) fn with_budget(budget: u64) -> Self {
        ScanLimits {
            budget,
            read: RETRIEVAL_READ_BYTES.max(budget),
            lines: 2,
        }
    }
}

/// The vault half of directory retrieval with the default limits for
/// `byte_budget`: up to two [`best_lines`] per matching note, OR-style
/// coverage over `terms`. See [`retrieval_scan_with`] (the broker's call).
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) fn retrieval_passages_with(
    vault: &Path,
    terms: &[Term],
    include: &dyn Fn(&str) -> bool,
    kinds: Option<&[String]>,
    byte_budget: u64,
    notices: &mut Vec<String>,
    prefilter: &dyn Fn(&[String], &[Note]) -> RetrievalCandidates,
) -> Vec<LinePassage> {
    let (passages, _) = retrieval_scan_with(
        vault,
        terms,
        include,
        kinds,
        ScanLimits::with_budget(byte_budget),
        notices,
        prefilter,
    );
    passages
}

/// A note's BM25 band (OB12): 0 for the best `score`, then one more each
/// time the score divides by [`BAND_RATIO`], capped just before
/// [`UNRANKED_BAND`].
fn bm25_band(score: f64, best: f64) -> usize {
    if best <= 0.0 || score >= best {
        return 0;
    }
    if score <= 0.0 {
        return UNRANKED_BAND - 1;
    }
    let band = (best / score).log(BAND_RATIO).floor();
    (band as usize).min(UNRANKED_BAND - 1)
}

/// Retrieval's candidates by path, with their bands (OB12): each note the
/// index ranked in its [`bm25_band`]; `extra` (notes the ranking can't speak
/// for: a just-edited note, a CJK match) in band 0, so coverage decides and
/// an exact match isn't buried; then, in [`UNRANKED_BAND`], up to
/// [`NAME_CANDIDATES`] notes whose stem names a term (most names, then
/// newest), the [`RECENT_CANDIDATES`] newest notes and `topup`. `notes` run
/// newest first.
fn candidate_bands(
    terms: &[Term],
    notes: &[Note],
    candidates: RankedCandidates,
) -> HashMap<String, usize> {
    let best = candidates.ranked.first().map_or(0.0, |(_, s)| *s);
    let mut bands: HashMap<String, usize> = HashMap::new();
    for (path, score) in candidates.ranked {
        bands.entry(path).or_insert(bm25_band(score, best));
    }
    for path in candidates.extra {
        // A stale note's rank is its old text's; band 0 overrides it.
        bands.insert(path, 0);
    }
    // (stem terms, walk index), most terms first, then newest.
    let mut named: Vec<(u32, usize)> = Vec::new();
    for (i, n) in notes.iter().enumerate() {
        let names = NoteNames::new(&n.name);
        let mask = name_mask(terms, &[names.stem_raw.as_str(), names.stem_norm.as_str()]);
        if mask != 0 {
            named.push((mask.count_ones(), i));
        }
    }
    named.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let unranked = named
        .iter()
        .take(NAME_CANDIDATES)
        .map(|&(_, i)| notes[i].path.clone())
        .chain(notes.iter().take(RECENT_CANDIDATES).map(|n| n.path.clone()))
        .chain(candidates.topup);
    for path in unranked {
        bands.entry(path).or_insert(UNRANKED_BAND);
    }
    bands
}

/// One vault retrieval pass. Notes are scanned newest first. With the FTS
/// index (OB12) only its candidates are read ([`candidate_bands`]: BM25's
/// best 200, name matches, the newest 20 and what the index can't vouch for),
/// each passage carrying its note's BM25 band; without one (a cold index, or
/// another process with no database) every eligible note is read, all in
/// band 0, the linear scan that is the fallback and the test oracle; a kinds
/// filter scans linearly too. A note
/// that is read is scored by the same code on either path, so its passages
/// are the same. It is charged to `limits.budget` only once it matches a term
/// (and the kind filter), so newer notes that never mention the query can't
/// starve an older exact match (agent effectiveness F3; until 2026-10-06
/// every eligible note was charged before the skip). `limits.read` bounds
/// the bytes read in all, which only a very large linear scan reaches, and
/// its notice says so. Size, budget and read notices go to `notices`: they
/// describe the vault, not the query.
#[allow(clippy::too_many_arguments)]
pub(crate) fn retrieval_scan_with(
    vault: &Path,
    terms: &[Term],
    include: &dyn Fn(&str) -> bool,
    kinds: Option<&[String]>,
    limits: ScanLimits,
    notices: &mut Vec<String>,
    prefilter: &dyn Fn(&[String], &[Note]) -> RetrievalCandidates,
) -> (Vec<LinePassage>, ScanStats) {
    let mut stats = ScanStats::default();
    if terms.is_empty() {
        return (Vec::new(), stats);
    }
    let mut notes: Vec<Note> = crate::vault::walk_notes(vault)
        .into_iter()
        .filter(|n| include(&n.path))
        .collect();
    notes.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.path.cmp(&b.path)));
    stats.eligible = notes.len();
    let texts: Vec<String> = terms.iter().map(|t| t.text.clone()).collect();
    // A note's kind comes from its frontmatter (`vault_index::classify`), so
    // it can't be told before BM25's top-200 cut: with a kinds filter the
    // scan stays linear, as before OB12, and no matching note is cut.
    // `extra` holding every eligible note (a read-only reader while the
    // index rebuilds) reads them all in band 0: a linear scan (RF-737).
    let mut covers_all = false;
    let bands = match kinds {
        Some(_) => None,
        None => prefilter(&texts, &notes).map(|c| {
            covers_all = !notes.is_empty() && notes.iter().all(|n| c.extra.contains(&n.path));
            candidate_bands(terms, &notes, c)
        }),
    };
    stats.indexed = bands.is_some() && !covers_all;
    let scorer = LineScorer::new(terms);
    let mut out = Vec::new();
    for note in &notes {
        let names = NoteNames::new(&note.name);
        if note.size > RETRIEVAL_MAX_NOTE_BYTES {
            // Noticed only when the request names its file (HF05): one
            // unrelated 1.8 MB `preview.html` put the notice in every
            // retrieval prompt. Its stem or, for a dotted term such as
            // `preview.html`, its full name; folders never count (`notes`,
            // `outputs` are schema words), and neither do bare numbers.
            if request_names(&names, terms) {
                stats.oversized += 1;
            }
            continue;
        }
        let stem_mask = name_mask(terms, &[names.stem_raw.as_str(), names.stem_norm.as_str()]);
        let band = match &bands {
            Some(bands) => match bands.get(note.path.as_str()) {
                Some(&band) => band,
                None => continue, // not a candidate: never read
            },
            None => 0,
        };
        stats.candidates += 1;
        if stats.bytes_read + note.size > limits.read {
            stats.stopped = Some("read-limit");
            notices.push(retrieval_read_notice(limits.read));
            break;
        }
        stats.bytes_read += note.size;
        stats.read += 1;
        let content = match fs::read_to_string(vault.join(&note.path)) {
            Ok(c) => c,
            Err(_) => continue,
        };
        // An HTML note is matched and cited by its text, never its markup
        // (HF05): tags are stripped in place, so line N is still the file's
        // line N and a citation (or `read_source`) lands on it.
        let text: Cow<str> = if crate::vault_embed_index::is_html_path(&note.path) {
            Cow::Owned(crate::vault_embed_index::strip_html_lines(&content))
        } else {
            Cow::Borrowed(&content)
        };
        let lower = text.to_lowercase();
        if scorer.term_mask(&lower) == 0 && stem_mask == 0 {
            continue;
        }
        let (fm, body) = crate::frontmatter::parse_map(&content);
        let item = crate::vault_index::classify(&note.path, &fm);
        if let Some(allow) = kinds {
            if !allow.iter().any(|k| k == &item.kind) {
                continue;
            }
        }
        let lifecycle = crate::note_lifecycle::of(&note.path, &fm, &body);
        if stats.bytes_charged + note.size > limits.budget {
            stats.stopped = Some("budget");
            notices.push(retrieval_budget_notice(limits.budget));
            break;
        }
        stats.bytes_charged += note.size;
        stats.matched += 1;
        // The derived title is added only for an already-admitted note; the
        // folder never counts (tasks/, notes/, calendar/ are schema words).
        let title_lower = item.title.to_lowercase();
        let mask = stem_mask | name_mask(terms, &[title_lower.as_str()]);
        for ev in scorer.best_lines(&text, mask, limits.lines) {
            out.push(LinePassage {
                rel: note.path.clone(),
                line: ev.line,
                excerpt: ev.excerpt,
                kind: item.kind.clone(),
                coverage: ev.coverage,
                in_line: ev.in_line,
                mtime: note.mtime.clone(),
                note: lifecycle.clone(),
                band,
            });
        }
    }
    if stats.oversized > 0 {
        notices.push(if stats.oversized == 1 {
            "1 vault note over 1 MB was not scanned for retrieval.".to_string()
        } else {
            format!(
                "{} vault notes over 1 MB were not scanned for retrieval.",
                stats.oversized
            )
        });
    }
    (out, stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::time::{Duration, Instant, UNIX_EPOCH};

    fn terms(texts: &[&str]) -> Vec<Term> {
        texts
            .iter()
            .map(|t| Term {
                text: t.to_string(),
                quoted: false,
            })
            .collect()
    }

    fn term(text: &str) -> Term {
        Term {
            text: text.to_string(),
            quoted: false,
        }
    }

    fn texts(q: &Query) -> Vec<&str> {
        q.terms.iter().map(|t| t.text.as_str()).collect()
    }

    /// A non-dot prefix: `walk_notes` prunes a dot-prefixed ROOT, so a
    /// `TempDir::new()` (`.tmpXXXX`) vault would be silently empty.
    fn vault() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("vault-").tempdir().unwrap()
    }

    fn put(v: &Path, rel: &str, content: &str) {
        let p = v.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    fn set_mtime(path: &Path, unix_secs: u64) {
        let t = UNIX_EPOCH + Duration::from_secs(unix_secs);
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(t))
            .unwrap();
    }

    fn no_index(_: &[String], _: &[Note]) -> TermCandidates {
        None
    }

    /// Retrieval with no index: the linear scan.
    fn no_bm25(_: &[String], _: &[Note]) -> RetrievalCandidates {
        None
    }

    fn paths(hits: &[SearchResult]) -> Vec<&str> {
        hits.iter().map(|h| h.path.as_str()).collect()
    }

    fn search_linear(
        v: &Path,
        q: &str,
        limit: usize,
        kinds: Option<&[String]>,
    ) -> Vec<SearchResult> {
        search_with(v, q, limit, kinds, &no_index)
    }

    fn kinds(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    // ── grammar ──────────────────────────────────────────────────────────────

    #[test]
    fn parse_keeps_short_domain_terms_and_quoted_filler() {
        let q = Query::parse(r#"the API is not "in the UI""#);
        assert_eq!(texts(&q), vec!["api", "is", "not", "in the ui"]);
        assert!(q.terms[3].quoted);
        assert!(!q.terms[0].quoted);
        assert_eq!(q.phrase, "api is not in the ui");
    }

    #[test]
    fn parse_all_filler_query_keeps_its_terms() {
        assert_eq!(texts(&Query::parse("the and")), vec!["the", "and"]);
    }

    #[test]
    fn parse_repeated_term_keeps_strictest_intent() {
        assert_eq!(
            Query::parse(r#"callback "callback""#).terms,
            vec![Term {
                text: "callback".into(),
                quoted: true
            }]
        );
    }

    #[test]
    fn parse_caps_terms_and_query_length() {
        let q = Query::parse("one two three four five six seven eight nine ten eleven twelve");
        assert_eq!(
            texts(&q),
            vec!["one", "two", "three", "four", "five", "six", "seven", "eight"]
        );
        let long = "é".repeat(250) + " " + &"ü".repeat(250);
        let q = Query::parse(&long);
        assert_eq!(q.terms.len(), 1);
        assert_eq!(q.terms[0].text.chars().count(), MAX_QUERY_CHARS);
    }

    #[test]
    fn parse_trims_edge_punctuation_and_drops_bare_punctuation() {
        assert_eq!(
            texts(&Query::parse("When is the Cobalt launch?")),
            vec!["when", "is", "cobalt", "launch"]
        );
        assert_eq!(
            texts(&Query::parse("acme, pricing")),
            vec!["acme", "pricing"]
        );
        assert_eq!(
            texts(&Query::parse("acme - pricing")),
            vec!["acme", "pricing"]
        );
        assert_eq!(texts(&Query::parse("(api)")), vec!["api"]);
        assert_eq!(
            texts(&Query::parse("100% foo_bar node.js decision.md")),
            vec!["100%", "foo_bar", "node.js", "decision.md"]
        );
        assert_eq!(texts(&Query::parse("&&")), vec!["&&"]);
        assert_eq!(texts(&Query::parse("??")), vec!["??"]);
        let quoted = Query::parse(r#""launch?""#);
        assert_eq!(texts(&quoted), vec!["launch?"]);
        assert!(quoted.terms[0].quoted);
        assert!(Query::parse(r#""""#).terms.is_empty());
    }

    // ── matching ─────────────────────────────────────────────────────────────

    #[test]
    fn short_alphabetic_terms_must_begin_a_word() {
        assert!(!has_term("build release", &term("ui")));
        assert!(!has_term("capistrano", &term("api")));
        assert!(!has_term("rapid", &term("api")));
        assert!(!has_term("brokr", &term("okr")));
        // The first "ui" (inside "build") is rejected; the second is a word.
        assert!(has_term("build mobile ui", &term("ui")));
        assert!(has_term("the api", &term("api")));
        assert!(has_term("_ui", &term("ui")));
        // Not all letters: a plain substring.
        assert!(has_term("fooq3", &term("q3")));
    }

    #[test]
    fn word_start_rule_is_limited_to_spaced_scripts() {
        assert!(has_term("来週の会議", &term("会議")));
        assert!(has_term("会議のapi", &term("api")));
        assert!(!has_term("éapi", &term("api")));
    }

    #[test]
    fn longer_terms_match_as_typeahead_substrings() {
        assert!(has_term("pricing decision", &term("pric")));
        assert!(has_term("weekly-sync", &term("eekl")));
    }

    #[test]
    fn typo_distance_follows_paperclip_thresholds() {
        for (a, b) in [
            ("serach", "search"),
            ("mibile", "mobile"),
            ("onbordng", "onboarding"),
            ("seach", "search"),
            ("weekyl", "weekly"),
        ] {
            assert!(within_edits(a, b), "{a} ~ {b}");
        }
        assert!(!within_edits("fooo", "food"));
        assert!(!within_edits("quasarxylophone", "authentication"));
    }

    // ── ranking ──────────────────────────────────────────────────────────────

    #[test]
    fn band_bonuses_cannot_cross_a_band() {
        const { assert!(MAX_BONUS < 1000) };
        let dir = vault();
        let v = dir.path();
        // Exact title with no body help vs a body note that earns every bonus.
        put(v, "notes/Cobalt launch.md", "Unrelated.\n");
        put(
            v,
            "notes/busy.md",
            "# Cobalt launch\ncobalt launch cobalt launch\n",
        );
        let hits = search_linear(v, "cobalt launch", 10, None);
        assert_eq!(paths(&hits)[0], "notes/Cobalt launch.md");
        assert!(hits[0].score >= BAND_EXACT_TITLE);
        // Adjacent bands: every term in the title (4000) vs title + heading
        // (3000) carrying the phrase, line and word bonuses.
        let dir = vault();
        let v = dir.path();
        put(v, "notes/Launch cobalt.md", "Unrelated.\n");
        put(
            v,
            "notes/Cobalt.md",
            "# Launch\ncobalt launch cobalt launch\n",
        );
        let hits = search_linear(v, "cobalt launch", 10, None);
        assert_eq!(
            paths(&hits),
            vec!["notes/Launch cobalt.md", "notes/Cobalt.md"]
        );
        assert!(hits[0].score >= BAND_TITLE_ALL && hits[0].score < BAND_TITLE_PHRASE);
        assert!(hits[1].score >= BAND_HEADING_ALL && hits[1].score < BAND_TITLE_ALL);
    }

    #[test]
    fn title_phrase_outranks_title_with_every_term_apart() {
        let dir = vault();
        let v = dir.path();
        // The phrase note is OLDER, so only its band can put it first.
        put(v, "notes/Cobalt launch plan.md", "Unrelated.\n");
        set_mtime(&v.join("notes/Cobalt launch plan.md"), 1_600_000_000);
        put(v, "notes/Launch the cobalt.md", "Unrelated.\n");
        set_mtime(&v.join("notes/Launch the cobalt.md"), 1_700_000_000);
        let hits = search_linear(v, "cobalt launch", 10, None);
        assert_eq!(
            paths(&hits),
            vec!["notes/Cobalt launch plan.md", "notes/Launch the cobalt.md"]
        );
        assert!(
            hits[0].score >= BAND_TITLE_PHRASE && hits[0].score < BAND_EXACT_TITLE,
            "{}",
            hits[0].score
        );
        assert!(
            hits[1].score >= BAND_TITLE_ALL && hits[1].score < BAND_TITLE_PHRASE,
            "{}",
            hits[1].score
        );
    }

    #[test]
    fn reordered_words_match_across_title_and_body() {
        let dir = vault();
        let v = dir.path();
        put(v, "notes/Acme pricing decision.md", "Draft.\n");
        put(
            v,
            "notes/other.md",
            "Acme discussed the pricing decision.\n",
        );
        let hits = search_linear(v, "pricing decision acme", 10, None);
        assert_eq!(
            paths(&hits),
            vec!["notes/Acme pricing decision.md", "notes/other.md"]
        );
    }

    #[test]
    fn all_terms_are_required() {
        let dir = vault();
        let v = dir.path();
        put(v, "notes/Cobalt.md", "Cobalt plan.\n");
        put(v, "notes/other.md", "Cobalt and friends.\n");
        assert!(search_linear(v, "cobalt zebra", 10, None).is_empty());
    }

    #[test]
    fn quoted_terms_never_typo_match() {
        let dir = vault();
        let v = dir.path();
        put(
            v,
            "projects/active/Onboarding wizard polish.md",
            "---\ntitle: Onboarding wizard polish\n---\nFirst-run flow.\n",
        );
        assert!(search_linear(v, r#""onbordng" wizard"#, 10, None).is_empty());
        // Unquoted, the same query recovers the title.
        assert_eq!(
            paths(&search_linear(v, "onbordng wizard", 10, None)),
            vec!["projects/active/Onboarding wizard polish.md"]
        );
    }

    #[test]
    fn typo_recovery_runs_only_without_literal_hits() {
        let dir = vault();
        let v = dir.path();
        put(v, "notes/Misspelled.md", "The wizard has a serach box.\n");
        put(
            v,
            "tasks/improve-search-performance.md",
            "---\ntitle: Improve search performance\n---\nProfile the index.\n",
        );
        put(v, "notes/Search wizard.md", "Wizard notes.\n");
        let hits = search_linear(v, "serach wizard", 10, None);
        assert_eq!(paths(&hits), vec!["notes/Misspelled.md"]);
    }

    #[test]
    fn typo_recovery_honors_kinds_filter() {
        let dir = vault();
        let v = dir.path();
        put(
            v,
            "projects/active/Mobile API.md",
            "---\ntitle: Mobile API\n---\nBuild the mobile API for the app.\n",
        );
        put(
            v,
            "tasks/fix-api-authentication.md",
            "---\ntitle: Fix API authentication\n---\nTokens expire early.\n",
        );
        let tasks = kinds(&["task"]);
        assert!(search_linear(v, "mibile api", 10, Some(tasks.as_slice())).is_empty());
        let projects = kinds(&["project"]);
        let hits = search_linear(v, "mibile api", 10, Some(projects.as_slice()));
        assert_eq!(paths(&hits), vec!["projects/active/Mobile API.md"]);
        assert!(hits[0].score >= BAND_TITLE_TYPO && hits[0].score < BAND_BODY_ALL);
    }

    #[test]
    fn kinds_filter_applies_before_the_cap() {
        let dir = vault();
        let v = dir.path();
        for i in 0..60 {
            put(
                v,
                &format!("notes/Cobalt {i:02}.md"),
                "# Cobalt\nCobalt cobalt.\n",
            );
        }
        put(
            v,
            "calendar/2026-09-01-0900-sync.md",
            "---\ntype: meeting\nsubject: Sync\n---\nWe mentioned cobalt once.\n",
        );
        let meeting = kinds(&["meeting"]);
        let hits = search_linear(v, "cobalt", 5, Some(meeting.as_slice()));
        assert_eq!(paths(&hits), vec!["calendar/2026-09-01-0900-sync.md"]);
    }

    #[test]
    fn old_strong_title_match_beats_many_recent_body_mentions() {
        let dir = vault();
        let v = dir.path();
        put(v, "notes/Cobalt.md", "# Cobalt\nProgram notes.\n");
        set_mtime(&v.join("notes/Cobalt.md"), 1_600_000_000);
        for i in 0..300 {
            let rel = format!("notes/update-{i:03}.md");
            put(v, &rel, "status: cobalt pending\n");
            set_mtime(&v.join(&rel), 1_700_000_000 + i);
        }
        let hits = search_linear(v, "cobalt", 1, None);
        assert_eq!(paths(&hits), vec!["notes/Cobalt.md"]);
    }

    #[test]
    fn snippet_centers_on_the_best_covered_body_line() {
        let dir = vault();
        let v = dir.path();
        put(
            v,
            "notes/review.md",
            "---\nsubject: Pricing review\n---\nIntro line.\nThe pricing decision for Acme is final.\n",
        );
        let hits = search_linear(v, "acme pricing decision", 10, None);
        assert_eq!(hits.len(), 1);
        assert!(
            hits[0].snippet.contains("decision for Acme"),
            "snippet: {:?}",
            hits[0].snippet
        );
    }

    #[test]
    fn title_only_match_has_empty_snippet() {
        let dir = vault();
        let v = dir.path();
        put(v, "notes/Zircon plan.md", "Unrelated.\n");
        let hits = search_linear(v, "zircon plan", 10, None);
        assert_eq!(paths(&hits), vec!["notes/Zircon plan.md"]);
        assert_eq!(hits[0].snippet, "");
    }

    #[test]
    fn filename_with_extension_matches_by_name_but_extension_alone_does_not() {
        let dir = vault();
        let v = dir.path();
        put(v, "notes/Acme pricing decision.md", "Draft only.\n");
        put(v, "notes/other.md", "Nothing here.\n");
        let hits = search_linear(v, "acme pricing decision.md", 10, None);
        assert_eq!(paths(&hits), vec!["notes/Acme pricing decision.md"]);
        assert!(hits[0].score >= BAND_EXACT_TITLE);
        // Before, "md" matched every note through its extension.
        assert!(search_linear(v, "md", 10, None).is_empty());
    }

    #[test]
    fn unreadable_notes_are_skipped_not_fatal() {
        let dir = vault();
        let v = dir.path();
        put(v, "notes/good.md", "Cobalt owner.\n");
        fs::write(v.join("notes/bad.md"), [0xff, 0xfe, b'c']).unwrap();
        assert_eq!(
            paths(&search_linear(v, "cobalt", 10, None)),
            vec!["notes/good.md"]
        );
    }

    // ── retrieval evidence ───────────────────────────────────────────────────

    /// Run 15 S1: the scorer that searches a body once per term answers
    /// exactly as the line-by-line reference, over generated notes full of
    /// the edge cases: frontmatter (matched on values only), CRLF, a lone or
    /// doubled CR, blank and whitespace lines, `---` fences, no trailing
    /// newline, lowercasing that changes length or depends on context (`İ`,
    /// `Σ` at a word's end, `ẞ`), unspaced scripts, 3-letter word-start terms,
    /// overlapping repeats (`aaaa` for `aaa`), name masks and cut-offs. The
    /// whole-text mask agrees with `term_mask` too, and a term holding a line
    /// break falls back to the reference.
    #[test]
    fn line_scorer_matches_the_line_by_line_reference() {
        let words = [
            "cobalt",
            "Cobalt",
            "COBALT",
            "cob",
            "Cob",
            "xcob",
            "cobcob",
            "alt",
            "aaaa",
            "aaa",
            "plan",
            "PLAN-review",
            "review",
            "ΣΑΣ",
            "ΌΣΟΣ",
            "σας",
            "İstanbul",
            "istanbul",
            "ıi",
            "日本語",
            "語plan",
            "naïve",
            "ÜBER",
            "über",
            "ẞ",
            "ß",
            "ss",
            "ﬁle",
            "file",
            "café",
            "2026",
            "q3",
            "x",
            "status:",
            "key:",
            "value",
            "-",
            "_",
            "'",
            "’",
            "é",
            "Ǆ",
            "ǆ",
        ];
        let seps = [" ", " ", " ", "  ", "\t", "-", "_", "/", ".", "'", "’", "·"];
        let term_pool = [
            "cobalt",
            "cob",
            "alt",
            "aaa",
            "plan",
            "review",
            "σας",
            "όσος",
            "i̇stanbul",
            "istanbul",
            "日本語",
            "naïve",
            "über",
            "ss",
            "ß",
            "file",
            "café",
            "2026",
            "q3",
            "value",
            "ǆ",
            "status",
            "語",
            "zzz",
        ];
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n.max(1) as u64) as usize
        };
        let line_of = |next: &mut dyn FnMut(usize) -> usize| {
            let n = next(7);
            let mut line = String::new();
            for i in 0..n {
                if i > 0 {
                    line.push_str(seps[next(seps.len())]);
                }
                line.push_str(words[next(words.len())]);
            }
            line
        };
        for case in 0..3000 {
            let mut content = String::new();
            if next(3) == 0 {
                content.push_str("---\n");
                for _ in 0..next(4) {
                    let key = ["title", "status", "tags", "- item", "key"][next(5)];
                    content.push_str(&format!("{key}: {}\n", line_of(&mut next)));
                }
                if next(5) != 0 {
                    content.push_str("---\n");
                }
            }
            let lines = next(9);
            for i in 0..lines {
                content.push_str(&match next(6) {
                    0 => String::new(),
                    1 => "   ".to_string(),
                    2 => "---".to_string(),
                    _ => line_of(&mut next),
                });
                let last = i + 1 == lines;
                content.push_str(match next(8) {
                    0 => "\r\n",
                    1 => "\r",
                    2 => "\r\r\n",
                    3 if last => "",
                    _ => "\n",
                });
            }
            let mut terms: Vec<Term> = (0..1 + next(6))
                .map(|_| term(term_pool[next(term_pool.len())]))
                .collect();
            terms.dedup();
            let name_mask = if next(3) == 0 { next(8) as u32 } else { 0 };
            let max = 1 + next(4);
            let scorer = LineScorer::new(&terms);
            assert_eq!(
                scorer.best_lines(&content, name_mask, max),
                best_lines_by_line(&terms, &content, name_mask, max),
                "case {case}: {content:?} terms {:?} mask {name_mask} max {max}",
                terms.iter().map(|t| t.text.as_str()).collect::<Vec<_>>()
            );
            let lower = content.to_lowercase();
            assert_eq!(
                scorer.term_mask(&lower),
                term_mask(&terms, &lower),
                "case {case}"
            );
        }
        // Dense repeats on one line and across lines (review S1 F1): a line
        // is left at its first accepted occurrence, a rejected word-start
        // match moves on without overlap, and the answer stays the same.
        let dense = vec![term("cobalt"), term("cob"), term("aaa")];
        let scorer = LineScorer::new(&dense);
        for content in [
            "cobalt ".repeat(20_000),
            format!(
                "x{}\n{}",
                "cobcob".repeat(5_000),
                "aaaa xaaa aaa\n".repeat(2_000)
            ),
            "xcobalt cob\n".repeat(3_000),
            "xcob ".repeat(4_000) + "cob",
        ] {
            assert_eq!(
                scorer.best_lines(&content, 0, 3),
                best_lines_by_line(&dense, &content, 0, 3)
            );
        }
        // A term with a line break never matches across lines.
        let t = vec![term("a\nb")];
        let scorer = LineScorer::new(&t);
        assert!(scorer.best_lines("a\nb\n", 0, 2).is_empty());
        assert_eq!(
            scorer.best_lines("x a\nb y\n", 1, 2),
            best_lines_by_line(&t, "x a\nb y\n", 1, 2)
        );
    }

    #[test]
    fn best_lines_never_emits_unmatched_lines() {
        let t = terms(&["cobalt", "launch", "review", "friday"]);
        let content = "---\ntitle: Cobalt launch review\nstatus: Not started\n---\nThe launch review moved to Friday.\n";
        let lines = best_lines(&t, content, 0b0111, 2);
        assert_eq!(
            lines,
            vec![
                LineEvidence {
                    line: 5,
                    excerpt: "The launch review moved to Friday.".into(),
                    coverage: 4,
                    in_line: 3
                },
                LineEvidence {
                    line: 2,
                    excerpt: "title: Cobalt launch review".into(),
                    coverage: 3,
                    in_line: 3
                },
            ]
        );
        // No matching line, but a name match: one passage at the first body line.
        let name_only = best_lines(
            &terms(&["zircon"]),
            "---\ntitle: x\n---\n\nSome body.\n",
            1,
            2,
        );
        assert_eq!(
            name_only,
            vec![LineEvidence {
                line: 5,
                excerpt: "Some body.".into(),
                coverage: 1,
                in_line: 0
            }]
        );
        // Neither: nothing.
        assert!(best_lines(&terms(&["zircon"]), "Some body.\n", 0, 2).is_empty());
    }

    #[test]
    fn frontmatter_lines_match_on_their_value_not_their_key() {
        // "What's the status of the Cobalt project?" → status, cobalt, project.
        let t = terms(&["status", "cobalt", "project"]);
        let plan =
            "---\ntitle: Cobalt plan\nstatus: Active\nproject: Cobalt\n---\nDelivery Thursday.\n";
        let lines = best_lines(&t, plan, 0b010, 2);
        assert_eq!(
            lines.iter().map(|l| l.line).collect::<Vec<_>>(),
            vec![2, 4],
            "{lines:?}"
        );
        assert!(lines.iter().all(|l| !l.excerpt.starts_with("status:")));
        // An unrelated note's keys alone are not evidence.
        let other = "---\ntitle: Groceries\nstatus: Not started\nproject: Home\n---\nMilk.\n";
        assert!(best_lines(&t, other, 0, 2).is_empty());
        // A value can still match, and a body line never loses its key-like text.
        let body = "---\ntitle: x\n---\nstatus: the cobalt review slipped\n";
        assert_eq!(best_lines(&t, body, 0, 2)[0].in_line, 2);
    }

    #[test]
    fn retrieval_name_coverage_ignores_folders_and_numbers() {
        let dir = vault();
        let v = dir.path();
        put(
            v,
            "tasks/a.md",
            "---\ntitle: Cobalt due\n---\nCobalt due Friday.\n",
        );
        put(v, "tasks/b.md", "Groceries and errands.\n");
        put(
            v,
            "calendar/2026-09-01-0900-sync.md",
            "---\ntype: meeting\nsubject: Sync\n---\nNotes pending.\n",
        );
        let t = crate::directory_connections::retrieval_terms("what tasks mention cobalt 2026");
        assert_eq!(
            t.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(),
            vec!["tasks", "mention", "cobalt", "2026"]
        );
        let mut warnings = Vec::new();
        let passages = retrieval_passages_with(
            v,
            &t,
            &|_: &str| true,
            None,
            32 * 1024 * 1024,
            &mut warnings,
            &no_bm25,
        );
        assert!(!passages.is_empty());
        assert!(
            passages.iter().all(|p| p.rel == "tasks/a.md"),
            "{passages:?}"
        );
        assert!(passages
            .iter()
            .all(|p| !p.excerpt.trim().is_empty() && p.excerpt.trim() != "---"));
        assert!(warnings.is_empty());
    }

    #[test]
    fn retrieval_skips_oversized_notes_without_stopping() {
        let dir = vault();
        let v = dir.path();
        let big = format!("Cobalt export\n{}", "x".repeat(2 * 1024 * 1024));
        put(v, "notes/export.md", &big);
        set_mtime(&v.join("notes/export.md"), 1_800_000_000);
        put(v, "notes/owner.md", "Cobalt owner is Dana.\n");
        set_mtime(&v.join("notes/owner.md"), 1_600_000_000);
        let mut warnings = Vec::new();
        let passages = retrieval_passages_with(
            v,
            &terms(&["cobalt", "owner"]),
            &|_: &str| true,
            None,
            32 * 1024 * 1024,
            &mut warnings,
            &no_bm25,
        );
        assert_eq!(passages.len(), 1);
        assert_eq!(passages[0].rel, "notes/owner.md");
        assert_eq!(passages[0].line, 1);
        // HF05: the request doesn't name `export`, so it hears nothing of it;
        // naming only its folder doesn't count either.
        assert!(warnings.is_empty(), "{warnings:?}");
        let passages = retrieval_passages_with(
            v,
            &terms(&["cobalt", "notes"]),
            &|_: &str| true,
            None,
            32 * 1024 * 1024,
            &mut warnings,
            &no_bm25,
        );
        assert_eq!(passages.len(), 1);
        assert!(warnings.is_empty(), "{warnings:?}");
        // A dotted term names the file too.
        let passages = retrieval_passages_with(
            v,
            &terms(&["cobalt", "export.md"]),
            &|_: &str| true,
            None,
            32 * 1024 * 1024,
            &mut warnings,
            &no_bm25,
        );
        assert_eq!(passages.len(), 1);
        assert_eq!(
            warnings,
            vec!["1 vault note over 1 MB was not scanned for retrieval.".to_string()]
        );
    }

    /// OB12: the index chooses which notes are read, never how one is
    /// scored. Every note it reads gets exactly the linear scan's passages,
    /// it leaves notes unread, and a stem-only note still comes in through
    /// its name. (Until OB12 the per-term trigram sets made both paths read
    /// the same notes; BM25 candidates leave out notes past the top 200 by
    /// design, so only the linear scan is the oracle for what matches.)
    #[test]
    fn bm25_candidates_are_scored_as_the_linear_scan_scores_them() {
        let dir = crate::vault_search_eval::write_corpus();
        let v = dir.path();
        let db = tempfile::tempdir().unwrap();
        let idx = RefCell::new(
            crate::vault_search_index::SearchIndex::open(&db.path().join("idx.db")).expect("index"),
        );
        let with_bm25 = |t: &[String], n: &[Note]| -> RetrievalCandidates {
            idx.borrow_mut().reconcile_and_rank(n, t, v)
        };
        let prompts = [
            "Please summarize what we decided about Acme pricing, the renewal quote, the quarterly review with Dana Whitfield and Priya Raman, the enterprise tier discount and the October seat price",
            "Where does the platform roadmap stand after the weekly sync, and did the standup mention the Cobalt budget owner, finance, the launch Thursday, onboarding wizard polish or mobile navigation?",
            "Find the API authentication fix, the mobile UI build, the release 100% checklist, foo_bar lookup, heartbeat_run_events retention, search performance and typography, export improvements, connection timeout runbook",
            "zircon plan briefing",
        ];
        fn of(p: &[LinePassage], rel: &str) -> Vec<(usize, usize, usize)> {
            p.iter()
                .filter(|p| p.rel == rel)
                .map(|p| (p.line, p.coverage, p.in_line))
                .collect()
        }
        let limits = ScanLimits::with_budget(32 * 1024 * 1024);
        let mut narrowed = false;
        let mut budget_hit = false;
        for prompt in prompts {
            let t = crate::directory_connections::retrieval_terms(prompt);
            assert!(!t.is_empty());
            let (mut wa, mut wb) = (Vec::new(), Vec::new());
            let all = |_: &str| true;
            let (a, linear) = retrieval_scan_with(v, &t, &all, None, limits, &mut wa, &no_bm25);
            let (b, ranked) = retrieval_scan_with(v, &t, &all, None, limits, &mut wb, &with_bm25);
            assert!(ranked.indexed && !linear.indexed, "{prompt:?}");
            assert_eq!(linear.read, linear.eligible, "{prompt:?}");
            assert_eq!(ranked.read, ranked.candidates, "{prompt:?}");
            let cited: HashSet<&str> = b.iter().map(|p| p.rel.as_str()).collect();
            for rel in cited {
                assert_eq!(of(&a, rel), of(&b, rel), "prompt {prompt:?} note {rel}");
            }
            assert_eq!(wa, wb, "{prompt:?}");
            narrowed |= ranked.read < linear.read;
            // Only matching notes are charged (F3), so a small budget
            // stops the broad prompts, not the one-note one.
            let mut w = Vec::new();
            retrieval_passages_with(v, &t, &all, None, 600, &mut w, &no_bm25);
            budget_hit |= w.iter().any(|w| *w == retrieval_budget_notice(600));
        }
        assert!(narrowed, "the index never left a note unread");
        assert!(budget_hit, "a 600-byte budget never stopped a scan");
        // The stem-only note comes in with the index, via its name.
        let t = crate::directory_connections::retrieval_terms("zircon plan briefing");
        let mut w = Vec::new();
        let b = retrieval_passages_with(
            v,
            &t,
            &|_: &str| true,
            None,
            32 * 1024 * 1024,
            &mut w,
            &with_bm25,
        );
        assert!(b
            .iter()
            .any(|p| p.rel == "notes/Zircon plan.md" && p.in_line == 0));
    }

    /// Agent effectiveness F3 (red-first): the byte budget is charged only
    /// for notes the query can match, after the FTS skip, so a vault grown
    /// with newer notes that never mention the term can't starve an older
    /// exact match. Until 2026-10-06 every eligible note was charged before
    /// the skip, so 40 newer 2 KB notes spent a 16 KB budget and the older
    /// note was never read, with or without the index.
    #[test]
    fn an_older_exact_match_survives_a_vault_grown_with_newer_notes() {
        let dir = vault();
        let v = dir.path();
        put(
            v,
            "notes/Kestrel decision.md",
            "# Kestrel decision\nWe chose the quillfeather supplier for the Kestrel launch.\n",
        );
        set_mtime(&v.join("notes/Kestrel decision.md"), 1_600_000_000);
        let filler = "Weekly status: nothing notable this week.\n".repeat(50);
        for i in 0..40u64 {
            let rel = format!("daily/2026-09-{:02} log {i}.md", 1 + i % 28);
            put(v, &rel, &filler);
            set_mtime(&v.join(&rel), 1_700_000_000 + i * 60);
        }
        let db = tempfile::tempdir().unwrap();
        let idx = RefCell::new(
            crate::vault_search_index::SearchIndex::open(&db.path().join("idx.db")).expect("index"),
        );
        let with_bm25 = |t: &[String], n: &[Note]| -> RetrievalCandidates {
            idx.borrow_mut().reconcile_and_rank(n, t, v)
        };
        let t = crate::directory_connections::retrieval_terms("quillfeather supplier");
        for (label, prefilter) in [
            (
                "index",
                &with_bm25 as &dyn Fn(&[String], &[Note]) -> RetrievalCandidates,
            ),
            ("linear", &no_bm25),
        ] {
            let mut notices = Vec::new();
            let passages = retrieval_passages_with(
                v,
                &t,
                &|_: &str| true,
                None,
                16 * 1024,
                &mut notices,
                prefilter,
            );
            assert!(
                passages
                    .iter()
                    .any(|p| p.rel == "notes/Kestrel decision.md"),
                "{label}: the older exact match was starved: {passages:?} {notices:?}"
            );
        }
    }

    fn lexical_request<'a>(
        v: &'a Path,
        query: &'a str,
        scope: &'a crate::retrieval_broker::Scope,
    ) -> crate::retrieval_broker::Request<'a> {
        crate::retrieval_broker::Request {
            vault: v,
            query,
            scope,
            kinds: None,
        }
    }

    fn semantic_off(_: &str) -> crate::retrieval_broker::SemanticAnswer {
        Err("semantic search is off".into())
    }

    /// OB12 (red-first): with the index, retrieval reads its BM25
    /// candidates, not every note that shares a word with the request. In
    /// 1,500 notes, 1,200 name the request's words: the linear scan reads
    /// all 1,500 (before OB12 a chat turn read ~959 real notes, 7.58 MB, the
    /// same way); the index reads its best 200 and the 20 newest.
    #[test]
    fn bm25_candidates_bound_the_notes_read_when_most_notes_match() {
        let dir = vault();
        let v = dir.path();
        for i in 0..1500usize {
            let body = if i % 5 == 0 {
                format!("Lunch order {i}.\n")
            } else {
                format!("Harbour gateway rollout {i} is on track.\n")
            };
            put(v, &format!("notes/n{i:04}.md"), &body);
        }
        let db = tempfile::tempdir().unwrap();
        let idx = RefCell::new(
            crate::vault_search_index::SearchIndex::open(&db.path().join("idx.db")).expect("index"),
        );
        let with_bm25 = |t: &[String], n: &[Note]| -> RetrievalCandidates {
            idx.borrow_mut().reconcile_and_rank(n, t, v)
        };
        let all = crate::retrieval_broker::Scope::all();
        let req = lexical_request(v, "harbour gateway rollout", &all);
        let linear =
            crate::retrieval_broker::retrieve_with(&req, &no_bm25, "linear", &semantic_off);
        let ranked = crate::retrieval_broker::retrieve_with(&req, &with_bm25, "fts", &semantic_off);
        assert_eq!(linear.stats.notes_read, 1500);
        assert_eq!(linear.stats.candidate_source.as_deref(), Some("linear"));
        assert_eq!(ranked.stats.notes_eligible, 1500);
        assert_eq!(ranked.stats.candidate_source.as_deref(), Some("bm25"));
        assert_eq!(ranked.stats.candidates, Some(ranked.stats.notes_read));
        assert!(
            ranked.stats.notes_read <= 250,
            "read {}",
            ranked.stats.notes_read
        );
        assert_eq!(ranked.passages.len(), crate::retrieval_broker::POOL);
    }

    /// OB12 (red-first): a BM25 band outranks line coverage. 300 newer notes
    /// each name four of the request's common words on one line; one older
    /// note names its two rare ones. By coverage alone (the linear order)
    /// the 300 come first and the older note misses the pool; with the index
    /// it leads.
    #[test]
    fn an_older_note_with_the_rare_terms_outranks_newer_generic_matches() {
        let dir = vault();
        let v = dir.path();
        put(
            v,
            "notes/trial.md",
            "The quillfeather osprey trial ended.\n",
        );
        set_mtime(&v.join("notes/trial.md"), 1_600_000_000);
        for i in 0..300u64 {
            let rel = format!("daily/d{i:03}.md");
            put(
                v,
                &rel,
                &format!("Status update {i}: contract review is pending.\n"),
            );
            set_mtime(&v.join(&rel), 1_700_000_000 + i * 60);
        }
        let db = tempfile::tempdir().unwrap();
        let idx = RefCell::new(
            crate::vault_search_index::SearchIndex::open(&db.path().join("idx.db")).expect("index"),
        );
        let with_bm25 = |t: &[String], n: &[Note]| -> RetrievalCandidates {
            idx.borrow_mut().reconcile_and_rank(n, t, v)
        };
        let all = crate::retrieval_broker::Scope::all();
        let req = lexical_request(
            v,
            "status update on the quillfeather osprey contract review",
            &all,
        );
        let is_target =
            |p: &crate::directory_connections::Passage| p.path.ends_with("notes/trial.md");
        let linear =
            crate::retrieval_broker::retrieve_with(&req, &no_bm25, "linear", &semantic_off);
        assert!(
            !linear.passages.iter().any(is_target),
            "linear documents the defect"
        );
        let ranked = crate::retrieval_broker::retrieve_with(&req, &with_bm25, "fts", &semantic_off);
        assert!(
            ranked.passages.iter().take(8).any(is_target),
            "{:?}",
            ranked
                .passages
                .iter()
                .take(8)
                .map(|p| p.path.as_str())
                .collect::<Vec<_>>()
        );
    }

    /// RF-737 (red-first): the trigram top-up is capped at the rest of the
    /// candidate budget. Of 300 notes, 30 say `rate`, 240 `strategy` (`rate`
    /// mid-word) and 30 neither. `rate` ranks 30, so the top-up joins;
    /// uncapped it added all 240 `strategy` notes and the pass read 270 of
    /// 300, nearly the linear scan. Capped, ranked and top-up stay within
    /// `BM25_CANDIDATES`, plus at most the 20 newest.
    #[test]
    fn the_trigram_top_up_is_capped_at_the_candidate_budget() {
        let dir = vault();
        let v = dir.path();
        for i in 0..300usize {
            let body = match i % 10 {
                0 => format!("The rate {i} was agreed.\n"),
                9 => format!("Lunch order {i}.\n"),
                _ => format!("The strategy {i} is on track.\n"),
            };
            put(v, &format!("notes/n{i:03}.md"), &body);
        }
        let db = tempfile::tempdir().unwrap();
        let idx = RefCell::new(
            crate::vault_search_index::SearchIndex::open(&db.path().join("idx.db")).expect("index"),
        );
        let with_bm25 = |t: &[String], n: &[Note]| -> RetrievalCandidates {
            idx.borrow_mut().reconcile_and_rank(n, t, v)
        };
        let all = crate::retrieval_broker::Scope::all();
        let req = lexical_request(v, "rate", &all);
        let ranked = crate::retrieval_broker::retrieve_with(&req, &with_bm25, "fts", &semantic_off);
        assert_eq!(ranked.stats.notes_eligible, 300);
        assert_eq!(ranked.stats.candidate_source.as_deref(), Some("bm25"));
        let bound = crate::vault_search_index::BM25_CANDIDATES + RECENT_CANDIDATES;
        assert!(
            ranked.stats.notes_read as usize <= bound,
            "read {} of 300, bound {bound}",
            ranked.stats.notes_read
        );
        // The 30 ranked `rate` notes lead (band 0 against the top-up's 32).
        assert!(
            ranked
                .passages
                .first()
                .is_some_and(|p| p.path.ends_with("0.md")),
            "{:?}",
            ranked.passages.first().map(|p| p.path.as_str())
        );
    }

    /// OB12: with no index (none engaged here, no database for the vault),
    /// retrieval's production entry point is the linear scan, every eligible
    /// note read in one band, and ranks exactly as the oracle does.
    #[test]
    fn a_cold_index_falls_back_to_the_linear_scan() {
        let dir = crate::vault_search_eval::write_corpus();
        let v = dir.path();
        let all = crate::retrieval_broker::Scope::all();
        let req = lexical_request(v, "cobalt budget owner and the platform roadmap", &all);
        let cold = crate::retrieval_broker::retrieve_lexical(&req, "semantic search is off");
        let oracle =
            crate::retrieval_broker::retrieve_with(&req, &no_bm25, "linear", &semantic_off);
        assert_eq!(cold.stats.prefilter, "linear");
        assert_eq!(cold.stats.candidate_source.as_deref(), Some("linear"));
        assert_eq!(cold.stats.notes_read, cold.stats.notes_eligible);
        assert_eq!(cold.stats.candidates, Some(cold.stats.notes_eligible));
        let project = |r: &crate::retrieval_broker::Ranked| -> Vec<(String, usize, String)> {
            r.passages
                .iter()
                .map(|p| (p.path.clone(), p.line, p.excerpt.clone()))
                .collect()
        };
        assert!(!cold.passages.is_empty());
        assert_eq!(project(&cold), project(&oracle));
    }

    /// OB12 review: a note's kind needs its frontmatter, so with a kinds
    /// filter the scan stays linear (the index is never asked) and a note of
    /// that kind is found wherever BM25 would rank it. Without one, the
    /// (here empty) ranking decides and fewer notes are read.
    #[test]
    fn a_kinds_filter_scans_linearly() {
        let dir = crate::vault_search_eval::write_corpus();
        let v = dir.path();
        let asked = std::cell::Cell::new(false);
        let empty_ranking = |_: &[String], _: &[Note]| -> RetrievalCandidates {
            asked.set(true);
            Some(RankedCandidates::default())
        };
        let t = crate::directory_connections::retrieval_terms("mobile api build");
        let kinds = vec!["task".to_string()];
        let limits = ScanLimits::with_budget(32 * 1024 * 1024);
        let all = |_: &str| true;
        let mut w = Vec::new();
        let (passages, stats) = retrieval_scan_with(
            v,
            &t,
            &all,
            Some(kinds.as_slice()),
            limits,
            &mut w,
            &empty_ranking,
        );
        assert!(!asked.get() && !stats.indexed);
        assert_eq!(stats.read, stats.eligible);
        assert!(passages.iter().any(|p| p.rel == "tasks/build-mobile-ui.md"));
        let (_, stats) = retrieval_scan_with(v, &t, &all, None, limits, &mut w, &empty_ranking);
        // The newest 20 and the notes whose stem names a term.
        assert!(asked.get() && stats.indexed);
        assert!(stats.read < stats.eligible, "{stats:?}");
    }

    /// RF-737 review: while the index rebuilds, a read-only reader's `extra`
    /// holds every eligible note, so the pass reads them all in band 0, the
    /// linear scan, and reports that rather than BM25.
    #[test]
    fn a_ranking_that_vouches_for_no_note_reports_a_linear_scan() {
        let dir = crate::vault_search_eval::write_corpus();
        let v = dir.path();
        let all_stale = |_: &[String], n: &[Note]| -> RetrievalCandidates {
            Some(RankedCandidates {
                extra: n.iter().map(|n| n.path.clone()).collect(),
                ..Default::default()
            })
        };
        let t = crate::directory_connections::retrieval_terms("mobile api build");
        let limits = ScanLimits::with_budget(32 * 1024 * 1024);
        let all = |_: &str| true;
        let mut w = Vec::new();
        let (_, stats) = retrieval_scan_with(v, &t, &all, None, limits, &mut w, &all_stale);
        assert!(!stats.indexed, "{stats:?}");
        assert_eq!(stats.read, stats.eligible);
    }

    /// OB12 review: a note the ranking can't speak for (`extra`: changed
    /// since the index saw it, or a CJK match) is read in band 0, so coverage
    /// decides and a just-edited exact match isn't buried below every ranked
    /// note; a stale note's old rank doesn't hold it back. A mid-word top-up
    /// match comes last.
    #[test]
    fn notes_the_ranking_cannot_speak_for_join_the_best_band() {
        let notes: Vec<Note> = ["a.md", "b.md", "c.md", "d.md"]
            .iter()
            .map(|p| Note {
                path: p.to_string(),
                name: p.to_string(),
                mtime: "2026-01-01T00:00:00Z".into(),
                size: 1,
            })
            .collect();
        let candidates = RankedCandidates {
            ranked: vec![("a.md".into(), 10.0), ("b.md".into(), 1.0)],
            extra: ["b.md".to_string(), "c.md".to_string()]
                .into_iter()
                .collect(),
            topup: ["d.md".to_string()].into_iter().collect(),
        };
        let bands = candidate_bands(&terms(&["zircon"]), &notes, candidates);
        assert_eq!(bands["a.md"], 0);
        assert_eq!(bands["b.md"], 0);
        assert_eq!(bands["c.md"], 0);
        assert_eq!(bands["d.md"], UNRANKED_BAND);
    }

    // ── benches (#[ignore]; run once with --ignored --nocapture) ─────────────

    /// A deterministic 1,200-note vault (~9.7 KB per note, realistic folders).
    fn bench_vault() -> tempfile::TempDir {
        struct Lcg(u64);
        impl Lcg {
            fn next(&mut self) -> usize {
                self.0 = self
                    .0
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (self.0 >> 33) as usize
            }
        }
        const VOCAB: &[&str] = &[
            "status",
            "update",
            "review",
            "meeting",
            "team",
            "customer",
            "renewal",
            "contract",
            "invoice",
            "budget",
            "forecast",
            "hiring",
            "launch",
            "release",
            "design",
            "research",
            "interview",
            "feedback",
            "summary",
            "notes",
            "action",
            "owner",
            "deadline",
            "risk",
            "blocker",
            "decision",
            "follow",
            "draft",
            "proposal",
            "pricing",
            "support",
            "ticket",
            "incident",
            "outage",
            "deploy",
            "migration",
            "database",
            "index",
            "search",
            "mobile",
            "build",
            "guide",
            "some",
            "time",
            "home",
            "name",
            "come",
            "same",
            "theme",
            "ui",
            "api",
            "web",
            "app",
            "sync",
            "weekly",
            "daily",
            "monthly",
            "plan",
            "goal",
            "metric",
            "growth",
            "sales",
            "marketing",
            "finance",
            "legal",
            "security",
            "privacy",
            "audit",
            "vendor",
            "partner",
            "board",
            "strategy",
            "offsite",
            "workshop",
            "training",
            "onboarding",
            "platform",
            "roadmap",
            "quarterly",
            "program",
            "project",
            "milestone",
            "scope",
            "estimate",
        ];
        const FOLDERS: &[&str] = &[
            "notes",
            "tasks",
            "calendar",
            "recordings",
            "projects/active",
            "people",
            "daily",
            "meetings",
        ];
        let dir = vault();
        let v = dir.path();
        let mut rng = Lcg(0x5eed);
        for i in 0..1200usize {
            let folder = FOLDERS[i % FOLDERS.len()];
            let title: Vec<&str> = (0..3).map(|_| VOCAB[rng.next() % VOCAB.len()]).collect();
            let title = title.join(" ");
            let mut body = String::with_capacity(10_000);
            if matches!(
                folder,
                "tasks" | "calendar" | "recordings" | "projects/active"
            ) {
                body.push_str(&format!("---\ntitle: {title}\nstatus: Active\n---\n"));
            }
            body.push_str(&format!("# {title}\n\n"));
            if i % 2 == 0 {
                body.push_str("The roadmap moved again.\n");
            }
            if i % 3 == 0 {
                body.push_str("Platform owners met.\n");
            }
            if i % 5 == 0 {
                body.push_str("Budget review pending.\n");
            }
            if i % 10 == 0 {
                body.push_str("Notes from the quarterly review.\n");
            }
            if i == 777 {
                body.push_str("The zircon supplier replied.\n");
            }
            while body.len() < 9_700 {
                let words: Vec<&str> = (0..14).map(|_| VOCAB[rng.next() % VOCAB.len()]).collect();
                body.push_str(&words.join(" "));
                body.push_str(".\n");
            }
            put(v, &format!("{folder}/{title} {i:04}.md"), &body);
            set_mtime(
                &v.join(format!("{folder}/{title} {i:04}.md")),
                1_750_000_000 + i as u64 * 60,
            );
        }
        dir
    }

    fn time<F: FnMut()>(runs: usize, mut f: F) -> (Duration, Duration) {
        let mut samples: Vec<Duration> = (0..runs)
            .map(|_| {
                let t = Instant::now();
                f();
                t.elapsed()
            })
            .collect();
        samples.sort();
        let p95 = ((runs as f64) * 0.95).ceil() as usize;
        (samples[runs / 2], samples[p95.max(1) - 1])
    }

    fn ms(d: Duration) -> String {
        format!("{:.1}", d.as_secs_f64() * 1000.0)
    }

    #[test]
    #[ignore = "bench: run with --ignored --nocapture"]
    fn search_latency_on_a_1200_note_vault() {
        let dir = bench_vault();
        let v = dir.path();
        let bytes: u64 = crate::vault::walk_notes(v).iter().map(|n| n.size).sum();
        let db = tempfile::tempdir().unwrap();
        let idx = RefCell::new(
            crate::vault_search_index::SearchIndex::open(&db.path().join("idx.db")).expect("index"),
        );
        let started = Instant::now();
        let notes = crate::vault::walk_notes(v);
        assert!(idx.borrow_mut().reconcile_add_edit(&notes, v));
        let build = started.elapsed();
        let with_index = |t: &[String], n: &[Note]| -> TermCandidates {
            idx.borrow_mut().reconcile_and_query_terms(n, t, v)
        };
        eprintln!(
            "bench vault: {} notes, {:.1} MB; initial FTS build {} ms; profile: {}",
            notes.len(),
            bytes as f64 / 1_048_576.0,
            ms(build),
            if cfg!(debug_assertions) {
                "debug (unoptimized)"
            } else {
                "release"
            }
        );
        eprintln!(
            "{:<12} {:<26} {:>7} | {:>9} {:>9} | {:>9} {:>9} | {:>9} {:>9}",
            "case",
            "query",
            "hits",
            "legacy50",
            "legacy95",
            "linear50",
            "linear95",
            "index50",
            "index95"
        );
        let cases = [
            ("common", "roadmap"),
            ("rare", "zircon"),
            ("3 terms", "platform roadmap budget"),
            ("phrase", "\"quarterly review\""),
            ("typo", "roadmp"),
            ("no hit", "quasarxylophone"),
            ("2-char", "me"),
            ("2-char", "ui"),
            ("all-short", "q3 ui"),
        ];
        for (label, q) in cases {
            let hits = search_with(v, q, 50, None, &no_index).len();
            let legacy = time(10, || {
                crate::vault_search_eval::legacy_search(v, q, 50, None);
            });
            let linear = time(10, || {
                search_with(v, q, 50, None, &no_index);
            });
            let indexed = time(10, || {
                search_with(v, q, 50, None, &with_index);
            });
            eprintln!(
                "{:<12} {:<26} {:>7} | {:>9} {:>9} | {:>9} {:>9} | {:>9} {:>9}",
                label,
                q,
                hits,
                ms(legacy.0),
                ms(legacy.1),
                ms(linear.0),
                ms(linear.1),
                ms(indexed.0),
                ms(indexed.1)
            );
        }
        // search_vault in the MCP subprocess has no index: one search plus one
        // retrieval pass per call.
        let include = |rel: &str| {
            !(["me/", "sessions/", "workflows/"]
                .iter()
                .any(|p| rel.starts_with(p))
                || (rel.starts_with("workspaces/") && !rel.contains("/outputs/")))
        };
        for q in ["platform roadmap budget", "roadmap"] {
            let t = crate::directory_connections::retrieval_terms(q);
            let shaped = time(10, || {
                search_with(v, q, 20, None, &no_index);
                let mut w = Vec::new();
                retrieval_passages_with(v, &t, &include, None, 32 * 1024 * 1024, &mut w, &no_bm25);
            });
            eprintln!(
                "search_vault-shaped linear pass {q:?}: p50 {} ms, p95 {} ms",
                ms(shaped.0),
                ms(shaped.1)
            );
        }
    }

    /// Agent effectiveness F3: the pre-F3 vault half against the broker on
    /// the 1,200-note bench vault, at the real 32 MB budget and at a scaled
    /// 2 MB one (the vault "grown" past it: the only `zircon` note is #777 of
    /// 1,200, so the ~210 newest notes fill 2 MB before it). Reports latency
    /// p50/p95 and what the broker scanned. Run with `--ignored --nocapture`.
    #[test]
    #[ignore = "bench: run with --ignored --nocapture"]
    fn broker_retrieval_before_and_after_on_a_1200_note_vault() {
        let dir = bench_vault();
        let v = dir.path();
        let db = tempfile::tempdir().unwrap();
        let idx = RefCell::new(
            crate::vault_search_index::SearchIndex::open(&db.path().join("idx.db")).expect("index"),
        );
        let notes = crate::vault::walk_notes(v);
        assert!(idx.borrow_mut().reconcile_add_edit(&notes, v));
        let with_index = |t: &[String], n: &[Note]| -> TermCandidates {
            idx.borrow_mut().reconcile_and_query_terms(n, t, v)
        };
        let with_bm25 = |t: &[String], n: &[Note]| -> RetrievalCandidates {
            idx.borrow_mut().reconcile_and_rank(n, t, v)
        };
        let off = |_: &str| -> crate::retrieval_broker::SemanticAnswer { Err("off".into()) };
        let all = crate::retrieval_broker::Scope::all();
        eprintln!(
            "profile: {}",
            if cfg!(debug_assertions) {
                "debug (unoptimized)"
            } else {
                "release"
            }
        );
        let prompts = [
            ("chat, 16 terms", "Can you pull together the roadmap status, the platform owners meeting, budget review, quarterly review notes, launch risks, pricing decision, renewal contract, hiring plan and security audit follow-ups?"),
            ("rare term", "What did the zircon supplier say?"),
        ];
        for budget in [32 * 1024 * 1024u64, 2 * 1024 * 1024] {
            for (label, prompt) in prompts {
                let legacy_found =
                    crate::vault_search_eval::legacy_retrieval(v, prompt, budget, &with_index)
                        .iter()
                        .any(|p| p.contains(" 0777"));
                let legacy = time(10, || {
                    crate::vault_search_eval::legacy_retrieval(v, prompt, budget, &with_index);
                });
                for (path, prefilter) in [
                    (
                        "index",
                        &with_bm25 as &dyn Fn(&[String], &[Note]) -> RetrievalCandidates,
                    ),
                    ("linear", &no_bm25),
                ] {
                    let run = || {
                        crate::retrieval_broker::retrieve_tuned(
                            &crate::retrieval_broker::Request {
                                vault: v,
                                query: prompt,
                                scope: &all,
                                kinds: None,
                            },
                            budget,
                            prefilter,
                            path,
                            &off,
                        )
                    };
                    let r = run();
                    let found = r.passages.iter().any(|p| p.path.contains(" 0777"));
                    let t = time(10, || {
                        run();
                    });
                    eprintln!(
                        "{} MB {label:<15} legacy(index) p50 {} / p95 {} ms, zircon found {legacy_found} | broker {path:<6} p50 {} / p95 {} ms, notes eligible {} read {} matched {}, {:.1} MB read, {:.1} MB scored, pool {}, stopped {:?}, zircon in pool {found}",
                        budget / (1024 * 1024),
                        ms(legacy.0),
                        ms(legacy.1),
                        ms(t.0),
                        ms(t.1),
                        r.stats.notes_eligible,
                        r.stats.notes_read,
                        r.stats.notes_matched,
                        r.stats.bytes_read as f64 / 1_048_576.0,
                        r.stats.bytes_scored as f64 / 1_048_576.0,
                        r.stats.pool,
                        r.stats.stopped,
                    );
                }
            }
        }
    }

    /// S1 review: the broker's candidate set is every matching note within
    /// the 32 MB scoring budget, so its ranking stage must stay near-linear.
    /// 10,000 notes of about 2 KB that the 16-term chat prompt matches (all
    /// fit the budget, three lines each, so ~30,000 passages reach ranking).
    /// Reports the broker's p50/p95 with the index and linear, and the
    /// pre-F3 vault half for comparison. Run with `--ignored --nocapture`.
    #[test]
    #[ignore = "bench: run with --ignored --nocapture"]
    fn broker_retrieval_on_10k_small_matching_notes() {
        const WORDS: &[&str] = &[
            "roadmap",
            "status",
            "platform",
            "owners",
            "meeting",
            "budget",
            "review",
            "quarterly",
            "launch",
            "risks",
            "pricing",
            "decision",
            "renewal",
            "contract",
            "hiring",
            "security",
            "audit",
            "the",
            "team",
            "agreed",
            "next",
            "week",
        ];
        let dir = vault();
        let v = dir.path();
        let mut seed = 0x5eed_u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize
        };
        for i in 0..10_000usize {
            let mut body = format!("# Meeting {i:05}\n\n");
            while body.len() < 2_000 {
                let words: Vec<&str> = (0..12).map(|_| WORDS[next() % WORDS.len()]).collect();
                body.push_str(&words.join(" "));
                body.push_str(".\n");
            }
            let rel = format!("notes/meeting {i:05}.md");
            put(v, &rel, &body);
            set_mtime(&v.join(&rel), 1_750_000_000 + i as u64 * 60);
        }
        let db = tempfile::tempdir().unwrap();
        let idx = RefCell::new(
            crate::vault_search_index::SearchIndex::open(&db.path().join("idx.db")).expect("index"),
        );
        let notes = crate::vault::walk_notes(v);
        assert!(idx.borrow_mut().reconcile_add_edit(&notes, v));
        let with_index = |t: &[String], n: &[Note]| -> TermCandidates {
            idx.borrow_mut().reconcile_and_query_terms(n, t, v)
        };
        let with_bm25 = |t: &[String], n: &[Note]| -> RetrievalCandidates {
            idx.borrow_mut().reconcile_and_rank(n, t, v)
        };
        let off = |_: &str| -> crate::retrieval_broker::SemanticAnswer { Err("off".into()) };
        let all = crate::retrieval_broker::Scope::all();
        let prompt = "Can you pull together the roadmap status, the platform owners meeting, budget review, quarterly review notes, launch risks, pricing decision, renewal contract, hiring plan and security audit follow-ups?";
        eprintln!(
            "profile: {}",
            if cfg!(debug_assertions) {
                "debug (unoptimized)"
            } else {
                "release"
            }
        );
        let budget = crate::retrieval_broker::VAULT_BUDGET;
        let legacy = time(5, || {
            crate::vault_search_eval::legacy_retrieval(v, prompt, budget, &with_index);
        });
        for (path, prefilter) in [
            (
                "index",
                &with_bm25 as &dyn Fn(&[String], &[Note]) -> RetrievalCandidates,
            ),
            ("linear", &no_bm25),
        ] {
            let run = || {
                crate::retrieval_broker::retrieve_tuned(
                    &crate::retrieval_broker::Request {
                        vault: v,
                        query: prompt,
                        scope: &all,
                        kinds: None,
                    },
                    budget,
                    prefilter,
                    path,
                    &off,
                )
            };
            let r = run();
            if path == "linear" {
                // With the index only BM25's candidates are read (OB12).
                assert_eq!(r.stats.notes_matched, 10_000, "every note fits the budget");
            }
            assert_eq!(r.passages.len(), crate::retrieval_broker::POOL);
            let t = time(5, || {
                run();
            });
            eprintln!(
                "10k x 2 KB: legacy(index) p50 {} / p95 {} ms | broker {path:<6} p50 {} / p95 {} ms, notes eligible {} read {} matched {}, {:.1} MB scored, pool {}",
                ms(legacy.0),
                ms(legacy.1),
                ms(t.0),
                ms(t.1),
                r.stats.notes_eligible,
                r.stats.notes_read,
                r.stats.notes_matched,
                r.stats.bytes_scored as f64 / 1_048_576.0,
                r.stats.pool,
            );
        }
    }

    #[test]
    #[ignore = "bench: run with --ignored --nocapture"]
    fn retrieval_latency_on_a_1200_note_vault() {
        let dir = bench_vault();
        let v = dir.path();
        let db = tempfile::tempdir().unwrap();
        let idx = RefCell::new(
            crate::vault_search_index::SearchIndex::open(&db.path().join("idx.db")).expect("index"),
        );
        let notes = crate::vault::walk_notes(v);
        assert!(idx.borrow_mut().reconcile_add_edit(&notes, v));
        let with_bm25 = |t: &[String], n: &[Note]| -> RetrievalCandidates {
            idx.borrow_mut().reconcile_and_rank(n, t, v)
        };
        let include = |rel: &str| {
            !(["me/", "sessions/", "workflows/"]
                .iter()
                .any(|p| rel.starts_with(p))
                || (rel.starts_with("workspaces/") && !rel.contains("/outputs/")))
        };
        // A 16-term chat prompt, and the workflow-shaped call: workflows pass
        // their template body as the retrieval query, and the retrieval runs
        // before the agent spawns (behind the run lease once admission moves
        // there), so its cost is lease hold time.
        let prompts = [
            (
                "chat, 16 terms",
                "Can you pull together the roadmap status, the platform owners meeting, budget review, quarterly review notes, launch risks, pricing decision, renewal contract, hiring plan and security audit follow-ups?",
            ),
            (
                "workflow: morning brief",
                "Generate my morning brief for today.\n\n1. Read today's meetings from `calendar/*.md` (match on the `start` field).\n2. Read `tasks/*.md` and collect anything overdue or due today (frontmatter\n   `due`, statuses that aren't done).\n3. Review active project notes under `projects/` for current priorities\n   and blockers relevant to today's work.\n4. Write a single self-contained HTML document to `briefings/{date}.html`\n   (today's date as YYYY-MM-DD). Inline all CSS; no external assets. Keep it\n   scannable: schedule first, then tasks, then project priorities, then one\n   \"suggested focus\" line.\n",
            ),
        ];
        for (label, prompt) in prompts {
            let t = crate::directory_connections::retrieval_terms(prompt);
            let mut passages = 0usize;
            let linear = time(10, || {
                let mut w = Vec::new();
                passages = retrieval_passages_with(
                    v,
                    &t,
                    &include,
                    None,
                    32 * 1024 * 1024,
                    &mut w,
                    &no_bm25,
                )
                .len();
            });
            let indexed = time(10, || {
                let mut w = Vec::new();
                retrieval_passages_with(
                    v,
                    &t,
                    &include,
                    None,
                    32 * 1024 * 1024,
                    &mut w,
                    &with_bm25,
                );
            });
            eprintln!(
                "retrieval {label:<24} terms {:>2}, passages {:>5}: linear p50 {} / p95 {} ms, index p50 {} / p95 {} ms",
                t.len(),
                passages,
                ms(linear.0),
                ms(linear.1),
                ms(indexed.0),
                ms(indexed.1)
            );
        }
    }
}
