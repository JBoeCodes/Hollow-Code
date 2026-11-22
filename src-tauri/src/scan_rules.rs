//! The shared context-scan rules (context layers CX1).
//!
//! `vendor/ctx-scan/scan-rules.json` is a vendored copy of the one rules list
//! `example-org/org-context` keeps in `tools/ctx-scan/`: secrets,
//! one computer's facts, one person's facts, employee data, data files and the
//! size limit, each with a severity per layer (home, personal, org, project,
//! vault). The standalone Python `ctx-scan` reads the same file, and both test
//! suites read the same fixture corpus (`fixtures/expected.json`,
//! `fixtures/cases.json`), so the two scanners can't drift.
//!
//! [`memory_guard`](crate::memory_guard) takes its stored-secret shapes from
//! here; the vault repo's first-commit scan (two-Macs V0/V1) and the per-layer
//! write rules and leak checks (CX7, CX10) are meant to call [`scan_text`],
//! [`scan_path`] and [`scan_file`] rather than keep lists of their own. The
//! context sync engine (`sync_engine`, CX6a) scans each staged blob with
//! [`scan_bytes`] before it commits.
//!
//! A finding names the rule, line and column, never the matched text.

// `scan_path`, `scan_file` and the per-layer API are for the vault repo's
// first-commit scan (V1), the write rules (CX7) and the leak checks (CX10);
// until those land only memory_guard and the tests call into this module.
#![allow(dead_code)]

use once_cell::sync::Lazy;
use regex::{Captures, Regex};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// The vendored rules file, built into the binary.
pub(crate) const RULES_JSON: &str = include_str!("../../vendor/ctx-scan/scan-rules.json");

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Severity {
    Warn,
    Block,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Line,
    Path,
    Size,
    PersonalTerms,
    Unreadable,
}

#[derive(Deserialize, Default)]
struct ValueChecks {
    #[serde(default)]
    letter_and_digit: bool,
    #[serde(default)]
    not_placeholder: bool,
    #[serde(default)]
    min_entropy: Option<f64>,
    #[serde(default)]
    not_reference: bool,
}

#[derive(Deserialize)]
struct RawRule {
    id: String,
    group: String,
    target: String,
    #[serde(default)]
    pattern: Option<String>,
    #[serde(default)]
    exclude: Option<String>,
    #[serde(default)]
    message: String,
    severity: BTreeMap<String, String>,
    #[serde(default)]
    value_group: Option<usize>,
    #[serde(default)]
    value_checks: Option<ValueChecks>,
}

#[derive(Deserialize)]
struct RawRules {
    schema_version: u32,
    max_file_bytes: u64,
    layers: Vec<String>,
    #[serde(default)]
    placeholder_words: Vec<String>,
    rules: Vec<RawRule>,
}

pub(crate) struct Rule {
    pub id: String,
    pub group: String,
    pub target: Target,
    pub message: String,
    regex: Option<Regex>,
    exclude: Option<Regex>,
    severity: BTreeMap<String, Severity>,
    value_group: Option<usize>,
    checks: ValueChecks,
}

impl Rule {
    /// This rule's severity in `layer`, or `None` when it is off there.
    pub fn severity_for(&self, layer: &str) -> Option<Severity> {
        self.severity
            .get(layer)
            .or_else(|| self.severity.get("*"))
            .copied()
    }

    fn named(&self, name: &str) -> bool {
        name == self.id || name == self.group || name == "*"
    }
}

pub(crate) struct Rules {
    pub max_file_bytes: u64,
    pub layers: Vec<String>,
    placeholder_words: Vec<String>,
    pub rules: Vec<Rule>,
}

/// One finding. `line` and `column` are 1-based (column in characters);
/// both are 0 for a path, size or unreadable finding. `offset` is the byte
/// offset of the match in the scanned text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Finding {
    pub rule: &'static str,
    pub group: &'static str,
    pub severity: Severity,
    pub line: usize,
    pub column: usize,
    pub offset: usize,
}

/// `context-scan: allow <rule> <reason>` on a line or the line above it.
static ALLOW: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"context-scan:\s{0,3}allow\s{1,3}([A-Za-z0-9_.*-]{1,80})\s{1,3}(\S.{0,300})")
        .expect("allow pattern")
});

/// An allow on the line above counts only when that line holds nothing else,
/// so an allowed secret can't also wave through a different one below it.
static ALLOW_ONLY_LINE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"^\s{0,40}(<!--|#|//|--|;|/\*|\*)?\s{0,3}context-scan:\s{0,3}allow\s{1,3}([A-Za-z0-9_.*-]{1,80})\s{1,3}(\S.{0,300})$",
    )
    .expect("allow-only pattern")
});

/// The vendored rules, parsed once. The tests prove they parse.
pub(crate) static RULES: Lazy<Rules> =
    Lazy::new(|| Rules::parse(RULES_JSON).expect("vendored scan-rules.json parses"));

impl Rules {
    pub fn parse(json: &str) -> Result<Rules, String> {
        let raw: RawRules = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if raw.schema_version != 1 {
            return Err(format!("unsupported schema_version {}", raw.schema_version));
        }
        let mut rules = Vec::with_capacity(raw.rules.len());
        for r in raw.rules {
            let target = match r.target.as_str() {
                "line" => Target::Line,
                "path" => Target::Path,
                "size" => Target::Size,
                "personal_terms" => Target::PersonalTerms,
                "unreadable" => Target::Unreadable,
                other => return Err(format!("rule {}: unknown target {other}", r.id)),
            };
            let compile = |p: &Option<String>| -> Result<Option<Regex>, String> {
                p.as_deref()
                    .map(|p| Regex::new(p).map_err(|e| format!("rule {}: {e}", r.id)))
                    .transpose()
            };
            let regex = compile(&r.pattern)?;
            if matches!(target, Target::Line | Target::Path) && regex.is_none() {
                return Err(format!("rule {} needs a pattern", r.id));
            }
            let exclude = compile(&r.exclude)?;
            let mut severity = BTreeMap::new();
            for (layer, s) in r.severity {
                let s = match s.as_str() {
                    "warn" => Severity::Warn,
                    "block" => Severity::Block,
                    other => return Err(format!("rule {}: severity {other}", r.id)),
                };
                severity.insert(layer, s);
            }
            rules.push(Rule {
                id: r.id,
                group: r.group,
                target,
                message: r.message,
                regex,
                exclude,
                severity,
                value_group: r.value_group,
                checks: r.value_checks.unwrap_or_default(),
            });
        }
        Ok(Rules {
            max_file_bytes: raw.max_file_bytes,
            layers: raw.layers,
            placeholder_words: raw
                .placeholder_words
                .into_iter()
                .map(|w| w.to_lowercase())
                .collect(),
            rules,
        })
    }

    pub fn get(&self, id: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.id == id)
    }

    fn value_passes(&self, rule: &Rule, caps: &Captures) -> bool {
        let Some(group) = rule.value_group else {
            return true;
        };
        let value = caps.get(group).map(|m| m.as_str()).unwrap_or("");
        let checks = &rule.checks;
        if checks.letter_and_digit
            && !(value.chars().any(|c| c.is_numeric()) && value.chars().any(|c| c.is_alphabetic()))
        {
            return false;
        }
        if checks.not_placeholder {
            let low = value.to_lowercase();
            if self
                .placeholder_words
                .iter()
                .any(|w| low.contains(w.as_str()))
            {
                return false;
            }
        }
        if let Some(min) = checks.min_entropy {
            if entropy(value) < min {
                return false;
            }
        }
        if checks.not_reference && is_reference(value) {
            return false;
        }
        true
    }
}

/// A value that points somewhere (a path, URL, e-mail address or environment
/// lookup) rather than holding a secret. Same test as `ctx-scan`'s.
pub(crate) fn is_reference(value: &str) -> bool {
    if value.starts_with(['/', '~', '$', '.', '{', '%', '<']) || value.contains("://") {
        return true;
    }
    let low = value.to_lowercase();
    if ["process.env", "os.environ", "env.", "import.meta"]
        .iter()
        .any(|p| low.starts_with(p))
    {
        return true;
    }
    if let Some(at) = value.rfind('@').filter(|&at| at > 0) {
        let host = &value[at + 1..];
        let tld = host.rsplit('.').next().unwrap_or("");
        if host.contains('.')
            && host
                .chars()
                .all(|c| c.is_alphanumeric() || c == '.' || c == '-')
            && !tld.is_empty()
            && tld.chars().all(|c| c.is_alphabetic())
        {
            return true;
        }
    }
    false
}

/// Shannon entropy of `value`, in bits per character.
pub(crate) fn entropy(value: &str) -> f64 {
    let mut counts: BTreeMap<char, usize> = BTreeMap::new();
    let mut n = 0usize;
    for c in value.chars() {
        *counts.entry(c).or_default() += 1;
        n += 1;
    }
    if n == 0 {
        return 0.0;
    }
    let n = n as f64;
    -counts
        .values()
        .map(|&c| {
            let p = c as f64 / n;
            p * p.log2()
        })
        .sum::<f64>()
}

/// What to scan for and how.
#[derive(Clone, Copy)]
pub(crate) struct ScanOptions<'a> {
    /// The layer whose severities apply (`home`, `personal`, `org`,
    /// `project`, `vault`). A rule that is off in it isn't run.
    pub layer: &'a str,
    /// Only rules of these groups (`None`: every group).
    pub groups: Option<&'a [&'a str]>,
    /// Honour `context-scan: allow` lines. A repo's reviewed files may; text an
    /// agent is about to write (a memory) must not, or the text could allow
    /// its own secret.
    pub honor_allows: bool,
}

impl<'a> ScanOptions<'a> {
    pub fn layer(layer: &'a str) -> Self {
        ScanOptions {
            layer,
            groups: None,
            honor_allows: true,
        }
    }

    fn wants(&self, rule: &Rule) -> bool {
        rule.severity_for(self.layer).is_some()
            && self
                .groups
                .is_none_or(|g| g.iter().any(|g| *g == rule.group))
    }
}

/// Scan text line by line with the vendored rules. When two rules match
/// overlapping spans of one line, only the first rule (in file order) reports.
pub(crate) fn scan_text(text: &str, opts: ScanOptions) -> Vec<Finding> {
    scan_text_with(&RULES, text, opts)
}

fn scan_text_with(rules: &'static Rules, text: &str, opts: ScanOptions) -> Vec<Finding> {
    let line_rules: Vec<&'static Rule> = rules
        .rules
        .iter()
        .filter(|r| r.target == Target::Line && opts.wants(r))
        .collect();
    let mut out = Vec::new();
    let mut previous = "";
    let mut line_start = 0usize;
    for (index, raw_line) in text.split('\n').enumerate() {
        let line = raw_line.trim_end_matches('\r');
        let allows: Vec<&str> = if opts.honor_allows {
            ALLOW
                .captures_iter(line)
                .filter_map(|c| c.get(1).map(|m| m.as_str()))
                .chain(
                    ALLOW_ONLY_LINE
                        .captures(previous)
                        .and_then(|c| c.get(2).map(|m| m.as_str())),
                )
                .collect()
        } else {
            Vec::new()
        };
        let mut taken: Vec<(usize, usize)> = Vec::new();
        for rule in &line_rules {
            if allows.iter().any(|a| rule.named(a)) {
                continue;
            }
            let Some(regex) = rule.regex.as_ref() else {
                continue;
            };
            for caps in regex.captures_iter(line) {
                let whole = caps.get(0).expect("group 0");
                if !rules.value_passes(rule, &caps) {
                    continue;
                }
                let (start, end) = (whole.start(), whole.end());
                if taken.iter().any(|&(s, e)| start < e && s < end) {
                    continue;
                }
                taken.push((start, end));
                out.push(Finding {
                    rule: rule.id.as_str(),
                    group: rule.group.as_str(),
                    severity: rule.severity_for(opts.layer).expect("wanted rule"),
                    line: index + 1,
                    column: line[..start].chars().count() + 1,
                    offset: line_start + start,
                });
            }
        }
        previous = line;
        line_start += raw_line.len() + 1;
    }
    out
}

/// Path rules (secret file names, data file types, HR file names) for a
/// repo-relative path with `/` separators.
pub(crate) fn scan_path(rel: &str, opts: ScanOptions) -> Vec<Finding> {
    RULES
        .rules
        .iter()
        .filter(|r| r.target == Target::Path && opts.wants(r))
        .filter(|r| {
            r.regex.as_ref().is_some_and(|re| re.is_match(rel))
                && !r.exclude.as_ref().is_some_and(|ex| ex.is_match(rel))
        })
        .map(|r| whole_file(r, opts.layer))
        .collect()
}

fn whole_file(rule: &'static Rule, layer: &str) -> Finding {
    Finding {
        rule: rule.id.as_str(),
        group: rule.group.as_str(),
        severity: rule.severity_for(layer).expect("wanted rule"),
        line: 0,
        column: 0,
        offset: 0,
    }
}

fn target_findings(target: Target, opts: ScanOptions) -> Vec<Finding> {
    RULES
        .rules
        .iter()
        .filter(|r| r.target == target && opts.wants(r))
        .map(|r| whole_file(r, opts.layer))
        .collect()
}

/// A whole file the way `ctx-scan` checks it: path rules, the size limit (a
/// file over it is reported and only its first `max_file_bytes` are read),
/// then the text with NUL bytes dropped, so binary and UTF-16 files are read.
pub(crate) fn scan_file(path: &Path, rel: &str, opts: ScanOptions) -> Vec<Finding> {
    let mut out = scan_path(rel, opts);
    let size = match std::fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(_) => {
            out.extend(target_findings(Target::Unreadable, opts));
            return out;
        }
    };
    if size > RULES.max_file_bytes {
        out.extend(target_findings(Target::Size, opts));
    }
    let mut bytes = Vec::new();
    let read = std::fs::File::open(path).and_then(|f| {
        std::io::Read::read_to_end(
            &mut std::io::Read::take(f, RULES.max_file_bytes),
            &mut bytes,
        )
    });
    if read.is_err() {
        out.extend(target_findings(Target::Unreadable, opts));
        return out;
    }
    out.extend(scan_content(bytes, rel, opts));
    out
}

/// [`scan_file`] for content already in memory (a staged git blob, the
/// context sync engine): path rules, the size limit on `bytes.len()`, then
/// the first `max_file_bytes` as text with NUL bytes dropped.
pub(crate) fn scan_bytes(bytes: &[u8], rel: &str, opts: ScanOptions) -> Vec<Finding> {
    let mut out = scan_path(rel, opts);
    if bytes.len() as u64 > RULES.max_file_bytes {
        out.extend(target_findings(Target::Size, opts));
    }
    let head = &bytes[..bytes.len().min(RULES.max_file_bytes as usize)];
    out.extend(scan_content(head.to_vec(), rel, opts));
    out
}

/// The text part of [`scan_file`] and [`scan_bytes`].
fn scan_content(mut bytes: Vec<u8>, rel: &str, opts: ScanOptions) -> Vec<Finding> {
    bytes.retain(|&b| b != 0);
    let text = String::from_utf8_lossy(&bytes);
    let only_secrets: &[&str] = &["secret"];
    let opts = if rel.rsplit('/').next() == Some(".context-scan.yml") {
        ScanOptions {
            groups: Some(only_secrets),
            ..opts
        }
    } else {
        opts
    };
    scan_text(&text, opts)
}

/// Whether path rule `id` names `rel`, whatever the layer (its pattern
/// matches and its exclude doesn't). The vault root (`sync_vault`, CX6b)
/// keeps such files out of git instead of blocking on them.
pub(crate) fn path_rule_matches(id: &str, rel: &str) -> bool {
    RULES.get(id).is_some_and(|r| {
        r.target == Target::Path
            && r.regex.as_ref().is_some_and(|re| re.is_match(rel))
            && !r.exclude.as_ref().is_some_and(|ex| ex.is_match(rel))
    })
}

/// The extensions the `file.data_type` rule names (`csv`, `pdf`, ...), read
/// from its pattern `(?i)\.(a|b|...)$`; empty when the pattern takes another
/// shape (the vault root's generated ignore then lists none, and its test
/// says so).
pub(crate) fn data_type_extensions() -> Vec<String> {
    let Some(src) = RULES
        .get("file.data_type")
        .and_then(|r| r.regex.as_ref())
        .map(|re| re.as_str())
    else {
        return Vec::new();
    };
    src.strip_prefix("(?i)\\.(")
        .and_then(|s| s.strip_suffix(")$"))
        .filter(|s| s.chars().all(|c| c.is_ascii_alphanumeric() || c == '|'))
        .map(|s| s.split('|').map(str::to_string).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::path::PathBuf;

    fn vendor_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("vendor")
            .join("ctx-scan")
    }

    fn read_json(rel: &str) -> serde_json::Value {
        let text = std::fs::read_to_string(vendor_dir().join(rel)).expect(rel);
        serde_json::from_str(&text).expect(rel)
    }

    #[test]
    fn vendored_copy_matches_its_record() {
        let record = std::fs::read_to_string(vendor_dir().join("VENDORED.md")).unwrap();
        let line = Regex::new(r"^([0-9a-f]{64})  (\S+)$").unwrap();
        let mut checked = 0;
        for l in record.lines() {
            let Some(c) = line.captures(l) else { continue };
            let bytes = std::fs::read(vendor_dir().join(&c[2])).expect(&c[2]);
            let hash = format!("{:x}", Sha256::digest(&bytes));
            assert_eq!(
                hash, &c[1],
                "vendor/ctx-scan/{} changed: edit it in org-context and re-vendor",
                &c[2]
            );
            checked += 1;
        }
        assert!(checked >= 5, "VENDORED.md lists the vendored files");
        assert!(
            record.contains("  scan-rules.json\n"),
            "the rules file is recorded"
        );
    }

    #[test]
    fn the_vendored_rules_parse_and_name_known_layers() {
        let rules = &*RULES;
        assert!(rules.max_file_bytes >= 1024 * 1024);
        for rule in &rules.rules {
            for layer in rule.severity.keys() {
                assert!(
                    layer == "*" || rules.layers.contains(layer),
                    "{} names unknown layer {layer}",
                    rule.id
                );
            }
        }
        for id in [
            "secret.anthropic",
            "secret.github",
            "secret.aws_access_key",
            "secret.stripe",
            "secret.square",
            "secret.cloudflare",
            "secret.private_key",
            "machine.macbook",
            "pii.ssn",
            "file.too_large",
        ] {
            assert!(rules.get(id).is_some(), "{id}");
        }
    }

    fn case_text(case: &serde_json::Value, alphabet: &[char]) -> String {
        let mut out = String::new();
        for part in case["parts"].as_array().unwrap() {
            let part = part.as_str().unwrap();
            if let Some(n) = part.strip_prefix("@seq:") {
                let n: usize = n.parse().unwrap();
                out.extend((0..n).map(|i| alphabet[(i * 7 + 3) % alphabet.len()]));
            } else {
                out.push_str(part);
            }
        }
        out
    }

    #[test]
    fn shared_cases_match_the_python_scanner() {
        let data = read_json("fixtures/cases.json");
        let alphabet: Vec<char> = data["alphabet"].as_str().unwrap().chars().collect();
        let cases = data["cases"].as_array().unwrap();
        assert!(cases.len() >= 40);
        for case in cases {
            let text = case_text(case, &alphabet);
            let layer = case["layer"].as_str().unwrap();
            let mut got: Vec<&str> = scan_text(&text, ScanOptions::layer(layer))
                .iter()
                .map(|f| f.rule)
                .collect();
            got.sort();
            let mut want: Vec<&str> = case["expect"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            want.sort();
            assert_eq!(got, want, "case {}", case["name"]);
        }
    }

    #[test]
    fn shared_fixture_files_report_what_expected_json_says() {
        let expected = read_json("fixtures/expected.json");
        let layer = expected["layer"].as_str().unwrap();
        let opts = ScanOptions::layer(layer);
        let as_set = |v: &serde_json::Value| -> Vec<(u64, String, String)> {
            let mut out: Vec<_> = v
                .as_array()
                .unwrap()
                .iter()
                .map(|f| {
                    (
                        f["line"].as_u64().unwrap(),
                        f["rule"].as_str().unwrap().to_string(),
                        f["severity"].as_str().unwrap().to_string(),
                    )
                })
                .collect();
            out.sort();
            out
        };
        let got_set = |found: Vec<Finding>| -> Vec<(u64, String, String)> {
            let mut out: Vec<_> = found
                .into_iter()
                .map(|f| {
                    let sev = match f.severity {
                        Severity::Warn => "warn",
                        Severity::Block => "block",
                    };
                    (f.line as u64, f.rule.to_string(), sev.to_string())
                })
                .collect();
            out.sort();
            out
        };
        let files = expected["files"].as_object().unwrap();
        assert!(files.len() >= 6);
        for (rel, want) in files {
            let path = vendor_dir().join("fixtures").join(rel);
            assert_eq!(got_set(scan_file(&path, rel, opts)), as_set(want), "{rel}");
        }
        let tmp = tempfile::tempdir().unwrap();
        for gen in expected["generated"].as_array().unwrap() {
            let rel = gen["path"].as_str().unwrap();
            let bytes = gen["bytes"].as_u64().unwrap() as usize;
            let path = tmp.path().join("big.md");
            let line = b"plain text line for the size limit test\n";
            let mut data = line.repeat(bytes / line.len());
            data.resize(bytes, b'x');
            std::fs::write(&path, &data).unwrap();
            assert_eq!(
                got_set(scan_file(&path, rel, opts)),
                as_set(&gen["expect"]),
                "{rel}"
            );
        }
    }

    #[test]
    fn a_big_file_is_reported_and_its_first_part_read_and_nul_bytes_are_dropped() {
        let tmp = tempfile::tempdir().unwrap();
        let limit = RULES.max_file_bytes as usize;
        let key = ["sk-", "ant-api03-", &"Q7x".repeat(10)].concat();
        let early = tmp.path().join("early.md");
        let mut data = format!("KEY {key}\n").into_bytes();
        data.resize(limit + 1024, b'x');
        std::fs::write(&early, &data).unwrap();
        let late = tmp.path().join("late.md");
        let mut data = vec![b'y'; limit + 10];
        data.extend_from_slice(format!("\nKEY {key}\n").as_bytes());
        std::fs::write(&late, &data).unwrap();
        let utf16 = tmp.path().join("notes.txt");
        let wide: Vec<u8> = format!("KEY {key}\n")
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        std::fs::write(&utf16, &wide).unwrap();
        let rules_of = |p: &Path, rel: &str| -> Vec<&str> {
            let mut r: Vec<&str> = scan_file(p, rel, ScanOptions::layer("org"))
                .iter()
                .map(|f| f.rule)
                .collect();
            r.sort();
            r
        };
        assert_eq!(
            rules_of(&early, "early.md"),
            ["file.too_large", "secret.anthropic"]
        );
        assert_eq!(rules_of(&late, "late.md"), ["file.too_large"]);
        assert_eq!(rules_of(&utf16, "notes.txt"), ["secret.anthropic"]);
    }

    #[test]
    fn a_finding_carries_position_not_text() {
        let key = ["sk-", "ant-api03-", &"Q7x".repeat(10)].concat();
        let text = format!("first line\nexport KEY={key}\n");
        let found = scan_text(
            &text,
            ScanOptions {
                layer: "personal",
                groups: Some(&["secret"]),
                honor_allows: false,
            },
        );
        assert_eq!(found.len(), 1);
        let f = &found[0];
        assert_eq!((f.rule, f.line, f.column), ("secret.anthropic", 2, 12));
        assert_eq!(&text[f.offset..f.offset + 7], "sk-ant-");
        assert!(!format!("{f:?}").contains("Q7x"));
    }

    #[test]
    fn allow_lines_count_only_when_asked() {
        // A made-up value, built here so no key-shaped string sits in the source.
        let alphabet: Vec<char> = ('a'..='z').chain('0'..='9').collect();
        let value: String = (0..30)
            .map(|i| alphabet[(i * 7 + 3) % alphabet.len()])
            .collect();
        let text =
            format!("TOKEN={value}  # context-scan: allow secret.assignment documented test value");
        assert!(scan_text(&text, ScanOptions::layer("org")).is_empty());
        let strict = ScanOptions {
            layer: "org",
            groups: None,
            honor_allows: false,
        };
        assert_eq!(scan_text(&text, strict).len(), 1);
    }

    #[test]
    fn adversarial_lines_finish_fast() {
        let bad = [
            "token".repeat(200_000),
            format!("password={}", "a1".repeat(300_000)),
            format!("-----BEGIN {}", "A ".repeat(300_000)),
            "sk-ant-".repeat(100_000),
            format!("bearer {}x", " ".repeat(200_000)),
        ];
        for text in &bad {
            let start = std::time::Instant::now();
            scan_text(text, ScanOptions::layer("org"));
            assert!(start.elapsed().as_secs() < 5, "{}", &text[..20]);
        }
    }
}
