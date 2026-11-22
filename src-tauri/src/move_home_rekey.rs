//! Move Hollow home: the pure re-key functions (no file system).
//!
//! - [`stem_for`]: the per-vault store stem of a path as given (no
//!   canonicalize; `session_store::vault_file_stem` canonicalizes first and
//!   then computes exactly this).
//! - [`workspace_id`]: a connected project's `local-<24 hex>` id
//!   (`directory_connections::project`).
//! - [`claude_dir_name`]: Claude Code's per-project folder name for a cwd.
//! - [`rewrite_paths`]: single-pass, segment-aware, longest-prefix path
//!   rewrite. A mapping matches only at a path boundary on both sides, so
//!   `/x/hollow-vault2` and `/x/hollow-vault.bak` never match `/x/hollow-vault`,
//!   and output is never rescanned.
//! - [`rewrite_ids`]: exact `local-<24 hex>` token rewrite.
//! - [`rewrite_json_strings`]: a byte-preserving JSON string rewriter (only
//!   the string tokens that change are re-encoded).
//! - [`rewrite_codex_headers`]: `[projects."<path>"]` header lines.

use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// `{name, non-alphanumerics to "-"}-{sha256(path)[..16]}`, of `path` exactly
/// as given.
pub fn stem_for(path: &Path) -> String {
    let hash = format!("{:x}", Sha256::digest(path.to_string_lossy().as_bytes()));
    let name: String = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "vault".to_string())
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    format!("{name}-{}", &hash[..16])
}

/// `local-` + the first 24 hex of sha256(path string).
pub fn workspace_id(path: &Path) -> String {
    let hash = format!("{:x}", Sha256::digest(path.to_string_lossy().as_bytes()));
    format!("local-{}", &hash[..24])
}

/// Whether `s` is a workspace id Hollow derives from a path.
pub fn is_local_id(s: &str) -> bool {
    s.len() == 30
        && s.starts_with("local-")
        && s[6..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Claude Code's project folder name for `cwd`: every character that is not
/// an ASCII letter or digit becomes `-` (one per UTF-16 unit, as Claude's
/// JavaScript replace does).
pub fn claude_dir_name(cwd: &str) -> String {
    let mut out = String::with_capacity(cwd.len());
    for c in cwd.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            for _ in 0..c.len_utf16() {
                out.push('-');
            }
        }
    }
    out
}

/// A character that may continue a path name (so a match can't start or end
/// next to it).
fn name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | '~' | '$' | '+' | '@' | '%')
}

/// What may follow a matched path in free text: the end, `/`, a newline or
/// tab, a quote or backtick, or one of `,;:)]}<>|&\` (a `PATH`-like list,
/// a plist `<string>`, a script's `&&`); or a `.` that ends a
/// sentence (then the end or whitespace). Never a space: Finder names copies
/// `projects 2` and `hollow-vault copy`, so a space can continue a name.
fn ends_path(rest: &str) -> bool {
    let mut chars = rest.chars();
    match chars.next() {
        None => true,
        Some('/' | '\n' | '\r' | '\t') => true,
        Some(
            '"' | '\'' | '`' | ',' | ';' | ':' | ')' | ']' | '}' | '<' | '>' | '|' | '&' | '\\',
        ) => true,
        Some('.') => match chars.next() {
            None => true,
            Some(n) => n.is_whitespace() || matches!(n, '"' | '\'' | ')' | '`'),
        },
        _ => false,
    }
}

/// A whole value that is a path (a database path column, a JSON key or
/// path value, a transcript cwd, a Codex header, a git pointer): it matches a
/// mapping only when it equals the old path or continues it with `/`.
/// Longest mapping first. `None` when nothing matches.
pub fn rewrite_whole(value: &str, mappings: &[(String, String)]) -> Option<String> {
    mappings.iter().find_map(|(old, new)| {
        if value == old {
            Some(new.clone())
        } else {
            value
                .strip_prefix(old.as_str())
                .filter(|rest| rest.starts_with('/'))
                .map(|rest| format!("{new}{rest}"))
        }
    })
}

/// [`rewrite_whole`], or the value unchanged.
pub fn whole_or_same(value: &str, mappings: &[(String, String)]) -> String {
    rewrite_whole(value, mappings).unwrap_or_else(|| value.to_string())
}

/// A JSON string: a path value (it starts with `/` and holds no whitespace)
/// is matched whole, any other string as free text.
pub fn rewrite_json_value(value: &str, mappings: &[(String, String)]) -> Option<String> {
    if value.starts_with('/') && !value.chars().any(char::is_whitespace) {
        rewrite_whole(value, mappings)
    } else {
        let (new, n) = rewrite_paths(value, mappings);
        (n > 0).then_some(new)
    }
}

/// A git pointer file (`gitdir: <path>` in a worktree's `.git`, or a bare
/// path in `.git/worktrees/<id>/gitdir`): the path is matched whole.
pub fn rewrite_git_pointer(text: &str, mappings: &[(String, String)]) -> (String, usize) {
    let mut count = 0;
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let (prefix, path) = match body.strip_prefix("gitdir:") {
            Some(rest) => {
                let trimmed = rest.trim_start();
                (&body[..body.len() - trimmed.len()], trimmed)
            }
            None => ("", body),
        };
        match rewrite_whole(path, mappings) {
            Some(new) => {
                count += 1;
                out.push_str(prefix);
                out.push_str(&new);
                out.push_str(&line[body.len()..]);
            }
            None => out.push_str(line),
        }
    }
    (out, count)
}

/// Mappings sorted for matching: longest old first, then by text, empty and
/// identity mappings dropped, and trailing `/` trimmed.
pub fn sorted_mappings(mappings: &[(String, String)]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = mappings
        .iter()
        .map(|(a, b)| (trim_slash(a), trim_slash(b)))
        .filter(|(a, b)| !a.is_empty() && a != "/" && a != b)
        .collect();
    out.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

fn trim_slash(s: &str) -> String {
    let t = s.trim_end_matches('/');
    if t.is_empty() {
        s.to_string()
    } else {
        t.to_string()
    }
}

/// Rewrite every path in `text` that starts with a mapping's old path, at a
/// path boundary on both sides, longest mapping first; one left-to-right
/// pass, so a replacement is never matched again. Returns the text and the
/// number of replacements. `mappings` should come from [`sorted_mappings`].
pub fn rewrite_paths(text: &str, mappings: &[(String, String)]) -> (String, usize) {
    if mappings.is_empty() || text.is_empty() {
        return (text.to_string(), 0);
    }
    let mut out = String::with_capacity(text.len());
    let mut count = 0;
    let mut i = 0;
    let mut prev: Option<char> = None;
    while i < text.len() {
        let rest = &text[i..];
        if prev.is_none_or(|p| !name_char(p)) {
            if let Some((old, new)) = mappings
                .iter()
                .find(|(old, _)| rest.starts_with(old.as_str()) && ends_path(&rest[old.len()..]))
            {
                out.push_str(new);
                count += 1;
                i += old.len();
                prev = old.chars().last();
                continue;
            }
        }
        let c = rest.chars().next().unwrap_or('\0');
        out.push(c);
        prev = Some(c);
        i += c.len_utf8().max(1);
    }
    (out, count)
}

/// Rewrite exact `local-<24 hex>` tokens found in `ids`. A token must not
/// continue a longer name on either side.
pub fn rewrite_ids(text: &str, ids: &BTreeMap<String, String>) -> (String, usize) {
    if ids.is_empty() || !text.contains("local-") {
        return (text.to_string(), 0);
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut count = 0;
    let mut last = 0;
    let mut search = 0;
    while let Some(off) = text[search..].find("local-") {
        let at = search + off;
        let end = at + 30;
        let before_ok = at == 0 || {
            let b = bytes[at - 1];
            !(b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        };
        let after_ok = end == bytes.len()
            || (end < bytes.len() && {
                let b = bytes[end];
                !(b.is_ascii_alphanumeric() || b == b'_')
            });
        if before_ok && after_ok && end <= bytes.len() && text.is_char_boundary(end) {
            if let Some(new) = ids.get(&text[at..end]) {
                out.push_str(&text[last..at]);
                out.push_str(new);
                count += 1;
                last = end;
                search = end;
                continue;
            }
        }
        search = at + 6;
    }
    out.push_str(&text[last..]);
    (out, count)
}

/// Paths then ids, in one function: what a text store gets.
pub fn rewrite_text(
    text: &str,
    mappings: &[(String, String)],
    ids: &BTreeMap<String, String>,
) -> (String, usize) {
    let (a, n) = rewrite_paths(text, mappings);
    let (b, m) = rewrite_ids(&a, ids);
    (b, n + m)
}

/// Where a JSON string token sits: whether it is an object key, and the keys
/// of the objects around it (outermost first; array positions are `[]`).
pub struct JsonSpot<'a> {
    pub is_key: bool,
    pub path: &'a [String],
}

#[derive(Clone)]
enum Frame {
    Object { expect_key: bool, key: String },
    Array,
}

/// Decode a JSON string body (without quotes).
fn json_unescape(raw: &str) -> Option<String> {
    if !raw.contains('\\') {
        return Some(raw.to_string());
    }
    serde_json::from_str::<String>(&format!("\"{raw}\"")).ok()
}

/// Rewrite JSON string tokens in place: `f` gets each decoded string and
/// returns its replacement, or `None` to keep it. Untouched bytes stay as
/// they were (formatting, escapes, key order). Returns the text and the
/// number of tokens changed; `None` when the text isn't well-formed enough
/// to walk (an unterminated string).
pub fn rewrite_json_strings(
    text: &str,
    f: &mut dyn FnMut(&str, &JsonSpot<'_>) -> Option<String>,
) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut stack: Vec<Frame> = Vec::new();
    let mut count = 0;
    let mut i = 0;
    let mut last = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let start = i + 1;
                let mut j = start;
                while j < bytes.len() && bytes[j] != b'"' {
                    if bytes[j] == b'\\' {
                        j += 1;
                    }
                    j += 1;
                }
                if j >= bytes.len() {
                    return None;
                }
                let raw = &text[start..j];
                let is_key = matches!(
                    stack.last(),
                    Some(Frame::Object {
                        expect_key: true,
                        ..
                    })
                );
                let decoded = json_unescape(raw)?;
                let path: Vec<String> = stack
                    .iter()
                    .map(|fr| match fr {
                        Frame::Object { key, .. } => key.clone(),
                        Frame::Array => "[]".to_string(),
                    })
                    .collect();
                // The path of a key excludes the key itself; a value's path
                // ends with its key.
                let spot = JsonSpot {
                    is_key,
                    path: if is_key {
                        &path[..path.len().saturating_sub(1)]
                    } else {
                        &path
                    },
                };
                if let Some(new) = f(&decoded, &spot) {
                    if new != decoded {
                        out.push_str(&text[last..i]);
                        out.push_str(&serde_json::to_string(&new).ok()?);
                        last = j + 1;
                        count += 1;
                    }
                }
                if is_key {
                    if let Some(Frame::Object { expect_key, key }) = stack.last_mut() {
                        *expect_key = false;
                        *key = decoded;
                    }
                }
                i = j + 1;
                continue;
            }
            b'{' => stack.push(Frame::Object {
                expect_key: true,
                key: String::new(),
            }),
            b'[' => stack.push(Frame::Array),
            b'}' | b']' => {
                stack.pop();
            }
            b',' => {
                if let Some(Frame::Object { expect_key, .. }) = stack.last_mut() {
                    *expect_key = true;
                }
            }
            _ => {}
        }
        // An object's frame keeps its open key for nested values; when the
        // key's value is a container, push happens above with the parent's
        // key already recorded.
        i += 1;
    }
    out.push_str(&text[last..]);
    Some((out, count))
}

/// The JSON store rewrite for Hollow's own files: a string equal to the old
/// stem becomes the new stem; any other string gets [`rewrite_text`].
pub fn rewrite_store_json(
    text: &str,
    old_stem: &str,
    new_stem: &str,
    mappings: &[(String, String)],
    ids: &BTreeMap<String, String>,
) -> Option<(String, usize)> {
    rewrite_json_strings(text, &mut |s, _| {
        if !old_stem.is_empty() && s == old_stem && old_stem != new_stem {
            return Some(new_stem.to_string());
        }
        let (paths, changed) = match rewrite_json_value(s, mappings) {
            Some(p) => (p, true),
            None => (s.to_string(), false),
        };
        let (new, n) = rewrite_ids(&paths, ids);
        (changed || n > 0).then_some(new)
    })
}

/// `~/.claude.json`: rewrite the keys of the top-level `projects` object.
/// A key whose new name already exists is kept as it is and returned in the
/// second list (reported, never overwritten). Returns (text, changed,
/// kept-because-taken).
pub fn rewrite_claude_json(
    text: &str,
    mappings: &[(String, String)],
) -> Option<(String, usize, Vec<String>)> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let existing: BTreeSet<String> = value
        .get("projects")
        .and_then(|p| p.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    let mut taken = Vec::new();
    let mut claimed: BTreeSet<String> = BTreeSet::new();
    let (out, n) = rewrite_json_strings(text, &mut |s, spot| {
        if !(spot.is_key && spot.path.len() == 1 && spot.path[0] == "projects") {
            return None;
        }
        let new = rewrite_whole(s, mappings)?;
        if existing.contains(&new) || !claimed.insert(new.clone()) {
            taken.push(s.to_string());
            return None;
        }
        Some(new)
    })?;
    Some((out, n, taken))
}

/// Every string in a JSON file through [`rewrite_json_value`] (Gemini's
/// projects, Claude desktop's trusted folders, Obsidian's vault list).
pub fn rewrite_json_paths(text: &str, mappings: &[(String, String)]) -> Option<(String, usize)> {
    rewrite_json_strings(text, &mut |s, _| rewrite_json_value(s, mappings))
}

/// `~/.codex/config.toml`: rewrite `[projects."<path>"]` header lines in
/// place. A header whose new name is already a header is kept and reported.
/// Returns (text, changed, kept-because-taken).
pub fn rewrite_codex_headers(
    text: &str,
    mappings: &[(String, String)],
) -> (String, usize, Vec<String>) {
    let header_path = |line: &str| -> Option<String> {
        let t = line.trim();
        let inner = t.strip_prefix("[projects.\"")?.strip_suffix("\"]")?;
        (!inner.contains('"')).then(|| inner.to_string())
    };
    let existing: BTreeSet<String> = text.lines().filter_map(header_path).collect();
    let mut claimed = BTreeSet::new();
    let mut taken = Vec::new();
    let mut count = 0;
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        if let Some(path) = header_path(body) {
            if let Some(new) = rewrite_whole(&path, mappings) {
                if existing.contains(&new) || !claimed.insert(new.clone()) {
                    taken.push(path);
                } else {
                    let indent = &body[..body.len() - body.trim_start().len()];
                    out.push_str(indent);
                    out.push_str(&format!("[projects.\"{new}\"]"));
                    out.push_str(&line[body.len()..]);
                    count += 1;
                    continue;
                }
            }
        }
        out.push_str(line);
    }
    (out, count, taken)
}

/// The `$HOME/` and `~/` forms of each mapping under `home`, for scripts.
pub fn home_forms(mappings: &[(String, String)], home: &Path) -> Vec<(String, String)> {
    let home = home.to_string_lossy().trim_end_matches('/').to_string();
    let mut out = mappings.to_vec();
    for (old, new) in mappings {
        let (Some(o), Some(n)) = (
            old.strip_prefix(&format!("{home}/")),
            new.strip_prefix(&format!("{home}/")),
        ) else {
            continue;
        };
        out.push((format!("$HOME/{o}"), format!("$HOME/{n}")));
        out.push((format!("${{HOME}}/{o}"), format!("${{HOME}}/{n}")));
        out.push((format!("~/{o}"), format!("~/{n}")));
    }
    sorted_mappings(&out)
}

/// The olds of `mappings` that occur in `text` (at a boundary).
pub fn mentions(text: &str, mappings: &[(String, String)]) -> Vec<String> {
    let mut found = BTreeSet::new();
    for (old, new) in mappings {
        let one = [(old.clone(), new.clone())];
        if rewrite_paths(text, &one).1 > 0 {
            found.insert(old.clone());
        }
    }
    found.into_iter().collect()
}

/// `name` with `.moved-<n>` before its extension (`notes.md` to
/// `notes.moved-1.md`; `README` to `README.moved-1`).
pub fn moved_name(name: &str, n: usize) -> String {
    match name.rfind('.') {
        Some(dot) if dot > 0 => format!("{}.moved-{n}{}", &name[..dot], &name[dot..]),
        _ => format!("{name}.moved-{n}"),
    }
}
