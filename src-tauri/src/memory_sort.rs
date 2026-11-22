//! Sorting Claude Code's auto-memory entries by the four-question test (PX6).
//!
//! Claude Code keeps per-project auto-memory files under
//! `~/.claude/projects/<slug>/memory/*.md`. The context scan (CX19) finds them
//! as Memory files; this module proposes, for each one, the layer it belongs
//! to and one sentence naming the rule that fired. The review card (CX20)
//! shows both, and the person's click decides.
//!
//! **Rules.**
//! - Pure and read only: [`classify`] takes the path, the text and a date
//!   and returns a [`MemorySort`]. No file, no network, no model call. The
//!   scan calls it on the text it has already read.
//! - Deterministic keyword and pattern rules, asked in this order:
//!   1. Stale, first yes wins: `index` (a `MEMORY.md` file is the folder's
//!      index, not an entry), `empty` (almost no words), `superseded` (a
//!      `status:` or `superseded_by:` field, or a phrase like "superseded
//!      by"), `dated-old` (its newest date is over [`STALE_DAYS`] old and it
//!      describes a passing state).
//!   2. Otherwise each layer's signs are counted: Home (paths on this Mac,
//!      named machines, machine tools, disks and runners), Personal (the
//!      person's preferences; a `type: user` or `type: feedback` field) and
//!      Project (a codebase's files, its branches and merges, its gates and
//!      tests; a `type: project` field). The most signs win; a tie goes to the
//!      four-question order (Home, Personal, Project), the narrowest layer.
//!   3. No sign at all: Personal, rule `default`.
//! - A reason names rules and fixed words only, never a line of the file
//!   (a date it found is the one exception).

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// The layer a memory entry is proposed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SortLayer {
    /// Facts about this machine or another named machine.
    Home,
    /// Facts about one codebase.
    Project,
    /// The person's own preferences and ways of working.
    Personal,
    /// Superseded, dated and old, empty, or an index.
    Stale,
}

/// A proposed layer, the rule that decided it and why, in one sentence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MemorySort {
    pub layer: SortLayer,
    /// The deciding rule's id (`index`, `home-path`, `default` ...).
    pub rule: String,
    pub reason: String,
}

/// A dated entry older than this many days, describing a passing state, is
/// stale.
pub(crate) const STALE_DAYS: i64 = 60;
/// Fewer letters and digits than this (body and description together) is an
/// empty entry.
const MIN_WORD_CHARS: usize = 24;
/// The most signs one rule adds, so one long list can't outvote the rest.
const RULE_CAP: usize = 3;
/// What a `type:` field adds to its layer.
const TYPE_WEIGHT: usize = 2;

/// The project slug of an auto-memory file's path below `~/.claude`
/// (`projects/<slug>/memory/<file>.md`), else `None`.
pub(crate) fn auto_memory_slug(rel_path: &str) -> Option<&str> {
    let parts: Vec<&str> = rel_path.split('/').collect();
    match parts.as_slice() {
        ["projects", slug, "memory", rest @ ..]
            if !slug.is_empty()
                && !rest.is_empty()
                && rest.iter().all(|p| !p.is_empty())
                && rest
                    .last()
                    .is_some_and(|f| f.to_ascii_lowercase().ends_with(".md")) =>
        {
            Some(slug)
        }
        _ => None,
    }
}

/// How a rule looks for its words.
#[derive(Clone, Copy)]
enum Match {
    /// Anywhere in the lowercased text.
    Within,
    /// As whole words (letters and digits on neither side).
    Word,
    /// A token (split on spaces and quotes) that starts with it.
    TokenStart,
    /// A token that ends with it (trailing punctuation dropped).
    TokenEnd,
}

/// One counting rule: its id, its layer, what a hit says and its words.
struct Rule {
    id: &'static str,
    layer: SortLayer,
    says: &'static str,
    how: Match,
    words: &'static [&'static str],
}

const RULES: &[Rule] = &[
    Rule {
        id: "home-path",
        layer: SortLayer::Home,
        says: "names paths on this Mac",
        how: Match::TokenStart,
        words: &[
            "~/",
            "/users/",
            "/opt/homebrew",
            "/volumes/",
            "$home/",
            "/library/",
        ],
    },
    Rule {
        id: "home-folder",
        layer: SortLayer::Home,
        says: "names this Mac's own folders",
        how: Match::Within,
        words: &["library/caches", "application support", "launchagents"],
    },
    Rule {
        id: "named-machine",
        layer: SortLayer::Home,
        says: "names this Mac or another machine",
        how: Match::Word,
        words: &[
            "this mac",
            "this machine",
            "macbook",
            "mac mini",
            "the mini",
            "ssh mini",
            "tailnet",
            "tailscale",
            "hostname",
            "nixos",
            "homelab",
        ],
    },
    Rule {
        id: "machine-tool",
        layer: SortLayer::Home,
        says: "names tools installed on a machine",
        how: Match::Word,
        words: &[
            "homebrew",
            "brew install",
            "brew upgrade",
            "launchd",
            "launchctl",
            "xcode",
            "pmset",
            "keychain",
            "full disk access",
            "codesign",
        ],
    },
    Rule {
        id: "disk-runner",
        layer: SortLayer::Home,
        says: "names disks, cores, memory or runners",
        how: Match::Word,
        words: &[
            "ssd",
            "disk",
            "free space",
            "apfs",
            "time machine",
            "memory pressure",
            "cores",
            "ram",
            "runner",
            "hollow-slot",
            "cargo_target_dir",
        ],
    },
    Rule {
        id: "preference",
        layer: SortLayer::Personal,
        says: "states how the person likes work done",
        how: Match::Word,
        words: &[
            "prefers",
            "prefer",
            "preference",
            "likes",
            "dislikes",
            "wants",
            "doesn't want",
            "does not want",
            "asked me",
            "his call",
            "her call",
            "their call",
            "the user",
            "user's",
            "always ask",
            "never ask",
            "don't ask",
            "ways of working",
        ],
    },
    Rule {
        id: "code-path",
        layer: SortLayer::Project,
        says: "names a codebase's files",
        how: Match::TokenEnd,
        words: &[
            ".rs", ".ts", ".tsx", ".mjs", ".js", ".css", ".toml", ".json", ".py", ".go", ".swift",
        ],
    },
    Rule {
        id: "code-folder",
        layer: SortLayer::Project,
        says: "names a codebase's folders",
        how: Match::TokenStart,
        words: &[
            "src/",
            "src-tauri/",
            "scripts/",
            "docs/",
            "tests/",
            "crates/",
        ],
    },
    Rule {
        id: "pr-merge",
        layer: SortLayer::Project,
        says: "names branches, pull requests or merges",
        how: Match::Word,
        words: &[
            "pr",
            "prs",
            "pull request",
            "merge",
            "merged",
            "land.sh",
            "branch",
            "rebase",
            "main",
            "worktree",
            "commit",
        ],
    },
    Rule {
        id: "gate-test",
        layer: SortLayer::Project,
        says: "names a project's gates or tests",
        how: Match::Word,
        words: &[
            "gate",
            "cargo test",
            "clippy",
            "vitest",
            "tsc",
            "npm test",
            "playwright",
            "eslint",
            "fixture",
            "ci",
            "smoke",
        ],
    },
];

/// Phrases that mark an entry as replaced.
const SUPERSEDED: &[&str] = &[
    "superseded by",
    "superseded:",
    "no longer true",
    "no longer applies",
    "no longer the case",
    "no longer needed",
    "is obsolete",
    "now obsolete",
    "this is outdated",
    "(outdated)",
    "[outdated]",
];

/// `status:` values that mark an entry as replaced.
const STALE_STATUS: &[&str] = &[
    "superseded",
    "obsolete",
    "outdated",
    "archived",
    "stale",
    "retired",
];

/// Words for a state that passes (with an old date, the entry is stale).
const PASSING: &[&str] = &[
    "currently",
    "for now",
    "right now",
    "today",
    "this week",
    "tonight",
    "in progress",
    "in flight",
    "not yet merged",
    "not merged yet",
    "waiting on",
    "blocked on",
    "temporarily",
    "temporary",
    "at the moment",
];

/// A memory file split into its frontmatter fields and its body.
struct Parsed<'a> {
    fields: Vec<(String, String)>,
    body: &'a str,
}

impl Parsed<'_> {
    fn field(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .filter(|v| !v.is_empty())
    }
}

/// Frontmatter between a first `---` line and the next; otherwise all body.
fn parse(text: &str) -> Parsed<'_> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let all = Parsed {
        fields: Vec::new(),
        body: text,
    };
    let mut lines = text.split_inclusive('\n');
    if lines.next().map(str::trim_end) != Some("---") {
        return all;
    }
    let mut at = text.find('\n').map_or(text.len(), |i| i + 1);
    let mut fields = Vec::new();
    for line in lines {
        at += line.len();
        let t = line.trim_end();
        if t == "---" {
            return Parsed {
                fields,
                body: &text[at..],
            };
        }
        if let Some((k, v)) = t.split_once(':') {
            let v = v.trim().trim_matches(|c| c == '"' || c == '\'').trim();
            fields.push((k.trim().to_ascii_lowercase(), v.to_string()));
        }
    }
    all
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `needle` in `hay` with no letter, digit or `_` on either side.
fn has_word(hay: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(i) = hay[from..].find(needle) {
        let start = from + i;
        let end = start + needle.len();
        let before = hay[..start].chars().next_back();
        let after = hay[end..].chars().next();
        if !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char) {
            return true;
        }
        from = start + needle.len().max(1);
        while !hay.is_char_boundary(from) {
            from += 1;
        }
    }
    false
}

fn tokens(hay: &str) -> impl Iterator<Item = &str> {
    hay.split(|c: char| c.is_whitespace() || "`'\"()[]{}<>,;*|".contains(c))
        .filter(|t| !t.is_empty())
}

fn rule_hits(rule: &Rule, hay: &str) -> usize {
    rule.words
        .iter()
        .filter(|w| match rule.how {
            Match::Within => hay.contains(**w),
            Match::Word => has_word(hay, w),
            Match::TokenStart => tokens(hay).any(|t| t.starts_with(**w)),
            Match::TokenEnd => tokens(hay).any(|t| {
                let t = t.trim_end_matches(['.', ':', '!', '?']);
                t.len() > w.len() && t.ends_with(**w)
            }),
        })
        .count()
        .min(RULE_CAP)
}

/// Every `YYYY-MM-DD` date in `text`.
fn dates(text: &str) -> Vec<NaiveDate> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    if b.len() < 10 {
        return out;
    }
    for i in 0..=b.len() - 10 {
        let w = &b[i..i + 10];
        let shape = w.iter().enumerate().all(|(j, c)| match j {
            4 | 7 => *c == b'-',
            _ => c.is_ascii_digit(),
        });
        let edge_ok = (i == 0 || !b[i - 1].is_ascii_digit())
            && b.get(i + 10).is_none_or(|c| !c.is_ascii_digit());
        if shape && edge_ok {
            // The window is ASCII digits and dashes, so this slice is valid.
            if let Ok(d) = NaiveDate::parse_from_str(&text[i..i + 10], "%Y-%m-%d") {
                out.push(d);
            }
        }
    }
    out
}

fn layer_name(layer: SortLayer) -> &'static str {
    match layer {
        SortLayer::Home => "Home",
        SortLayer::Project => "Project",
        SortLayer::Personal => "Personal",
        SortLayer::Stale => "Stale",
    }
}

fn sorted(layer: SortLayer, rule: &str, reason: String) -> MemorySort {
    MemorySort {
        layer,
        rule: rule.to_string(),
        reason,
    }
}

/// Propose a layer for the auto-memory entry at `rel_path` (below the Claude
/// Code folder). `text` is the file's text (`None` when the scan didn't read
/// it), `modified` its modified time in seconds and `today` the date a
/// dated entry is measured against. Pure.
pub(crate) fn classify(
    rel_path: &str,
    text: Option<&str>,
    modified: Option<i64>,
    today: NaiveDate,
) -> MemorySort {
    let name = rel_path.rsplit('/').next().unwrap_or(rel_path);
    if name.eq_ignore_ascii_case("MEMORY.md") {
        return sorted(
            SortLayer::Stale,
            "index",
            "Stale by rule index: MEMORY.md is the folder's index of the other entries, not an entry."
                .into(),
        );
    }
    let Some(text) = text else {
        return sorted(
            SortLayer::Personal,
            "unread",
            "Personal by rule unread: the scan did not read the file, so no sign points elsewhere."
                .into(),
        );
    };
    let parsed = parse(text);
    let description = parsed.field("description").unwrap_or("");
    let words = parsed
        .body
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .chain(std::iter::once(description))
        .flat_map(str::chars)
        .filter(|c| c.is_alphanumeric())
        .count();
    if words < MIN_WORD_CHARS {
        return sorted(
            SortLayer::Stale,
            "empty",
            "Stale by rule empty: the entry has almost no words.".into(),
        );
    }
    let status = parsed
        .field("status")
        .map(str::to_ascii_lowercase)
        .filter(|s| STALE_STATUS.contains(&s.as_str()));
    let replaced = parsed
        .field("superseded_by")
        .or_else(|| parsed.field("superseded-by"))
        .is_some();
    let hay = format!(
        "{}\n{}\n{}",
        parsed.field("name").unwrap_or(""),
        description,
        parsed.body
    )
    .to_lowercase();
    if let Some(s) = status {
        return sorted(
            SortLayer::Stale,
            "superseded",
            format!("Stale by rule superseded: its status field says {s}."),
        );
    }
    if replaced {
        return sorted(
            SortLayer::Stale,
            "superseded",
            "Stale by rule superseded: it names the entry that replaced it.".into(),
        );
    }
    if let Some(p) = SUPERSEDED.iter().find(|p| hay.contains(**p)) {
        return sorted(
            SortLayer::Stale,
            "superseded",
            format!("Stale by rule superseded: it says \"{p}\"."),
        );
    }
    let newest = dates(&hay).into_iter().max().or_else(|| {
        modified
            .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
            .map(|t| t.date_naive())
    });
    if let Some(d) = newest {
        let age = today.signed_duration_since(d).num_days();
        if age > STALE_DAYS {
            if let Some(p) = PASSING.iter().find(|p| has_word(&hay, p)) {
                return sorted(
                    SortLayer::Stale,
                    "dated-old",
                    format!(
                        "Stale by rule dated-old: its newest date is {d}, {age} days ago, and it describes a passing state (\"{p}\")."
                    ),
                );
            }
        }
    }

    // Count each layer's signs.
    let kind = parsed.field("type").map(str::to_ascii_lowercase);
    let mut scores: Vec<Score> = ORDER
        .iter()
        .map(|l| Score {
            layer: *l,
            total: 0,
            fired: Vec::new(),
        })
        .collect();
    for rule in RULES {
        let n = rule_hits(rule, &hay);
        if n > 0 {
            score_of(&mut scores, rule.layer).add(rule.id, rule.says, n);
        }
    }
    match kind.as_deref() {
        Some("user") => score_of(&mut scores, SortLayer::Personal).add(
            "memory-type",
            "has type user",
            TYPE_WEIGHT,
        ),
        Some("feedback") => score_of(&mut scores, SortLayer::Personal).add(
            "memory-type",
            "has type feedback",
            TYPE_WEIGHT,
        ),
        Some("project") => score_of(&mut scores, SortLayer::Project).add(
            "memory-type",
            "has type project",
            TYPE_WEIGHT,
        ),
        _ => {}
    }
    // The most signs win; on a tie the earlier layer in the four-question
    // order (Home, Personal, Project) does. `scores` is in that order, and
    // `max_by_key` keeps the last of equals, so it walks them reversed.
    let Some(best) = scores
        .iter()
        .rev()
        .max_by_key(|s| s.total)
        .filter(|s| s.total > 0)
    else {
        return sorted(
            SortLayer::Personal,
            "default",
            "Personal by rule default: no home, project or stale sign fired, so it stays with you."
                .into(),
        );
    };
    let mut fired = best.fired.clone();
    // Strongest rule first; equal ones keep the rule table's order.
    fired.sort_by_key(|f| std::cmp::Reverse(f.2));
    let ids: Vec<&str> = fired.iter().map(|f| f.0).collect();
    let says: Vec<&str> = fired.iter().map(|f| f.1).collect();
    let against = scores
        .iter()
        .rev()
        .filter(|s| s.layer != best.layer)
        .max_by_key(|s| s.total)
        .filter(|s| s.total > 0)
        .map(|s| format!("; {} had {}", layer_name(s.layer), s.total))
        .unwrap_or_default();
    let rule_word = if ids.len() == 1 { "rule" } else { "rules" };
    let total = best.total;
    sorted(
        best.layer,
        ids[0],
        format!(
            "{} by {rule_word} {}: it {} ({total} {}{against}).",
            layer_name(best.layer),
            join_and(&ids),
            join_and(&says),
            if total == 1 { "sign" } else { "signs" },
        ),
    )
}

/// The layers signs are counted for, in the four-question order.
const ORDER: [SortLayer; 3] = [SortLayer::Home, SortLayer::Personal, SortLayer::Project];

/// One layer's signs: the total and each rule that fired (id, what it
/// says, how many).
struct Score {
    layer: SortLayer,
    total: usize,
    fired: Vec<(&'static str, &'static str, usize)>,
}

impl Score {
    fn add(&mut self, id: &'static str, says: &'static str, n: usize) {
        self.total += n;
        self.fired.push((id, says, n));
    }
}

fn score_of(scores: &mut [Score], layer: SortLayer) -> &mut Score {
    scores
        .iter_mut()
        .find(|s| s.layer == layer)
        .expect("every counted layer has a score")
}

/// "a", "a and b", "a, b and c".
fn join_and(parts: &[&str]) -> String {
    match parts {
        [] => String::new(),
        [one] => (*one).to_string(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
    }
}

#[cfg(test)]
#[path = "memory_sort_tests.rs"]
mod tests;
