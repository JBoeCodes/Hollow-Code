//! One canonical YAML-frontmatter parser/emitter for the vault.
//!
//! Historically each `vault_*` module hand-rolled its own copy of the
//! same handful of rules: strip the leading BOM, find the opening/closing
//! `---` fences, split each `key: value` line, unquote scalar values, and
//! on the write side decide whether a value needs quoting and how to
//! escape it. Six-plus near-identical copies meant the round-trip was only
//! as correct as the *weakest* copy — a real corruption risk (BE-02).
//!
//! This module is the single source of those rules. It is deliberately
//! narrow: it models frontmatter as an **ordered list of scalar key/value
//! pairs** plus the body text. That's exactly the shape the task / project
//! / workflow / calendar files use. List-valued keys (the `tags:` and
//! `syncedFromIds:` blocks) are *not* round-tripped through here — callers
//! that emit those re-derive them from structured fields on write, so the
//! parser simply skips list-item lines, matching the prior behavior.
//!
//! That makes [`parse`] + [`push_kv`] LOSSY for an existing file: list
//! items, continuation lines, empty values and any key the caller does not
//! model are gone after a parse → re-render. To edit an EXISTING file, use
//! [`patch`]: it rewrites only the named top-level keys and keeps every
//! other byte (unknown keys, lists, block scalars, comments, CRLF, BOM and,
//! unless a new body is given, the body). Re-rendering is for new files.
//!
//! Rules captured here (the union of the previous copies):
//! - BOM strip + leading-whitespace tolerance before the opening fence.
//! - Opening fence is `---`; closing fence is the first line that is
//!   exactly `---` (matched via `\n---`).
//! - CRLF tolerance: a `\r` left on the end of a line by `str::lines`
//!   on CRLF input is trimmed (`trim_end`), so Windows-authored files
//!   parse identically to Unix ones.
//! - `key: value` split on the first colon; key and value both trimmed.
//! - List-item lines (`- ` / `  - `) are skipped.
//! - Unquoting: `"…"` is unescaped in one pass (`\\`, `\"`, `\/`, `\n`,
//!   `\t`, `\r`, `\b`, `\f`, `\uXXXX`; a JSON superset, so the JSON-quoted
//!   values the retired Notion sync wrote read back exactly); `'…'` is taken verbatim
//!   minus the quotes.
//! - Quoting on emit: a value is quoted only when it would otherwise be
//!   ambiguous YAML (`:` `#` newline, leading/trailing space, or the bare
//!   words `true`/`false`/`null`); the escape is the inverse of unquote.

/// An ordered list of parsed scalar frontmatter pairs plus the body.
///
/// Order is preserved as encountered in the file. Duplicate keys keep
/// every occurrence (last-wins lookups can use [`Frontmatter::get`],
/// which returns the last match — the same effect a `BTreeMap` insert
/// loop had).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frontmatter {
    pub pairs: Vec<(String, String)>,
    pub body: String,
}

impl Frontmatter {
    /// Last value for `key`, or `None`. Last-wins mirrors the old
    /// `map.insert` loops where a later line overwrote an earlier one.
    /// Part of the ordered-pairs public API; current callers go through
    /// [`parse_map`], so this is exercised by tests for now.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.pairs
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// Parse a markdown document into its frontmatter pairs + body.
///
/// Tolerant by design: a document with no opening/closing fence is
/// returned as `{ pairs: [], body: <whole input> }` so callers can treat
/// "no frontmatter" as "all body" without a separate branch.
pub fn parse(content: &str) -> Frontmatter {
    let stripped = content.trim_start_matches('\u{feff}');
    let trimmed = stripped.trim_start();
    let Some(rest) = trimmed.strip_prefix("---") else {
        return Frontmatter {
            pairs: Vec::new(),
            body: content.to_string(),
        };
    };
    let rest = rest.trim_start_matches('\n');
    let Some(end_idx) = rest.find("\n---") else {
        return Frontmatter {
            pairs: Vec::new(),
            body: content.to_string(),
        };
    };
    let yaml_block = &rest[..end_idx];
    let body = rest[end_idx..]
        .trim_start_matches("\n---")
        .trim_start_matches('\n');

    let mut pairs: Vec<(String, String)> = Vec::new();
    for raw_line in yaml_block.lines() {
        // `trim_end` drops a trailing `\r` from CRLF input as well as any
        // trailing spaces, matching the prior per-module loops.
        let line = raw_line.trim_end();
        let lt = line.trim_start();
        if line.is_empty() || lt.starts_with("- ") {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_string();
            if key.is_empty() {
                continue;
            }
            let value = unquote(v.trim());
            pairs.push((key, value));
        }
    }

    Frontmatter {
        pairs,
        body: body.to_string(),
    }
}

/// Parse and return just the (last-wins) key→value map, plus body. A
/// convenience for callers that previously held a `BTreeMap` and only ever
/// did last-wins `.get()` lookups.
pub fn parse_map(content: &str) -> (std::collections::BTreeMap<String, String>, String) {
    let fm = parse(content);
    let mut map = std::collections::BTreeMap::new();
    for (k, v) in fm.pairs {
        map.insert(k, v);
    }
    (map, fm.body)
}

/// Reverse of the quoting applied by [`quote`], and of the JSON-string
/// quoting the retired Notion sync wrote. A double-quoted scalar is unescaped in ONE
/// left-to-right pass: `\\`, `\"`, `\/`, `\n`, `\t`, `\r`, `\b`, `\f` and
/// `\uXXXX` (surrogate pairs combined) decode; any other backslash sequence
/// is kept verbatim. A single-quoted scalar is returned verbatim sans quotes;
/// anything else is returned as-is.
///
/// One pass matters: chained `replace` calls cannot invert [`quote`]. They
/// read `quote("C:\new")` (`"C:\\new"`) back as `C:\` + newline + `ew`, and
/// left JSON's `\t` literal, so a Notion value with a tab or a backslash
/// came back altered and the next sync pushed the altered text.
pub fn unquote(v: &str) -> String {
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        return unescape_double_quoted(&v[1..v.len() - 1]);
    }
    if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
        return v[1..v.len() - 1].to_string();
    }
    v.to_string()
}

/// Four hex digits at the start of `s` as a UTF-16 code unit.
fn hex4(s: &str) -> Option<u16> {
    let digits = s.get(..4)?;
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u16::from_str_radix(digits, 16).ok()
}

fn unescape_double_quoted(inner: &str) -> String {
    let mut out = String::with_capacity(inner.len());
    let mut rest = inner;
    while let Some(pos) = rest.find('\\') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 1..];
        let Some(c) = after.chars().next() else {
            // A trailing lone backslash stays as written.
            out.push('\\');
            rest = after;
            break;
        };
        let simple = match c {
            '\\' => Some('\\'),
            '"' => Some('"'),
            '/' => Some('/'),
            'n' => Some('\n'),
            't' => Some('\t'),
            'r' => Some('\r'),
            'b' => Some('\u{8}'),
            'f' => Some('\u{c}'),
            _ => None,
        };
        if let Some(decoded) = simple {
            out.push(decoded);
            rest = &after[1..];
            continue;
        }
        if c == 'u' {
            if let Some(high) = hex4(&after[1..]) {
                // A high surrogate combines with a following `\uXXXX` low one.
                let low = after[5..]
                    .strip_prefix("\\u")
                    .and_then(hex4)
                    .filter(|low| (0xDC00..0xE000).contains(low));
                let (code, used) = match low {
                    Some(low) if (0xD800..0xDC00).contains(&high) => (
                        0x10000 + ((u32::from(high) - 0xD800) << 10) + (u32::from(low) - 0xDC00),
                        11,
                    ),
                    _ => (u32::from(high), 5),
                };
                if let Some(ch) = char::from_u32(code) {
                    out.push(ch);
                    rest = &after[used..];
                    continue;
                }
            }
        }
        // Not an escape this parser knows: keep the backslash and the character.
        out.push('\\');
        out.push(c);
        rest = &after[c.len_utf8()..];
    }
    out.push_str(rest);
    out
}

/// True when `v` must be double-quoted to round-trip safely as a YAML
/// scalar. Kept intentionally conservative — over-quoting is harmless,
/// under-quoting corrupts. This is the union of the predicates the
/// email / chat / calendar writers used to hand-roll (AQ-05); every
/// frontmatter emitter must use this single definition.
pub fn needs_quoting(v: &str) -> bool {
    // Leading YAML indicator characters turn a bare scalar into a list
    // item, anchor, alias, tag, block scalar, flow collection, or quote.
    let starts_special = matches!(
        v.as_bytes().first(),
        Some(b'-' | b'&' | b'*' | b'!' | b'|' | b'>' | b'[' | b'{' | b'\'' | b'"' | b'@' | b'`')
    );
    starts_special
        || v.contains(':')
        || v.contains('#')
        || v.contains('"')
        || v.contains('\n')
        || v.starts_with(' ')
        || v.ends_with(' ')
        || v == "true"
        || v == "false"
        || v == "null"
}

/// Double-quote and escape a scalar value. Inverse of [`unquote`].
pub fn quote(v: &str) -> String {
    let escaped = v
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    format!("\"{escaped}\"")
}

/// Append a `key: value` line to `lines`, quoting the value when
/// [`needs_quoting`] says so. Empty values are skipped entirely — the
/// canonical emit shape omits empty scalar keys rather than writing
/// `key:` with a blank value.
pub fn push_kv(lines: &mut Vec<String>, key: &str, value: &str) {
    if value.is_empty() {
        return;
    }
    if needs_quoting(value) {
        lines.push(format!("{key}: {}", quote(value)));
    } else {
        lines.push(format!("{key}: {value}"));
    }
}

/// One edit [`patch`] applies to a top-level frontmatter key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyEdit {
    /// Write `key: value`, quoted exactly as [`push_kv`] quotes. Unlike
    /// `push_kv`, an empty value is written as `key: ""` rather than
    /// dropped, so a present-but-empty key stays present.
    Set(String),
    /// Write `key: "value"`, always double-quoted with JSON string escapes
    /// (`serde_json::to_string`), so every control character is escaped and
    /// [`unquote`] reads back exactly `value`. Synced remote text round-trips
    /// this way, and files the retired Notion sync wrote keep their bytes.
    Quoted(String),
    /// Delete the key together with its continuation lines.
    Remove,
    /// Replace the key's whole span with these literal lines (the derived
    /// `tags:` list, for example). The first line should be the `key:` line.
    Block(Vec<String>),
}

/// Byte range of the frontmatter block (between the fences), located exactly
/// as [`parse`] finds it: BOM and leading whitespace tolerated, the first
/// `\n---` closes. Content `parse` would not read as frontmatter is an error.
pub(crate) fn block_bounds(content: &str) -> Result<(usize, usize), String> {
    let stripped = content.trim_start_matches('\u{feff}');
    let trimmed = stripped.trim_start();
    let Some(rest) = trimmed.strip_prefix("---") else {
        return Err("Expected Markdown frontmatter; file preserved".into());
    };
    let after_fence = rest.trim_start_matches('\n');
    let Some(end_idx) = after_fence.find("\n---") else {
        return Err("Unclosed Markdown frontmatter; file preserved".into());
    };
    // `after_fence` is a suffix of `content`, so its offset is a byte index.
    let block_start = content.len() - after_fence.len();
    Ok((block_start, block_start + end_idx))
}

/// The items of a top-level YAML block list: a `key:` line with no value,
/// followed by `- item` lines (the shape Obsidian writes for a List
/// property). [`parse`] skips list items, so this is how a caller reads them.
/// Items are unquoted like scalars; empty items are dropped. Last top-level
/// occurrence wins, as in [`parse_map`]. `None` when the key is absent, has a
/// scalar value, or the document has no frontmatter.
pub fn list_items(content: &str, key: &str) -> Option<Vec<String>> {
    let (start, end) = block_bounds(content).ok()?;
    let lines: Vec<&str> = content[start..end].split('\n').collect();
    let mut found = None;
    let mut i = 0;
    while i < lines.len() {
        if top_level_key(lines[i]) != Some(key) {
            i += 1;
            continue;
        }
        let value = lines[i].split_once(':').map_or("", |(_, v)| v.trim());
        let mut j = i + 1;
        while j < lines.len() && is_continuation(lines[j]) {
            j += 1;
        }
        found = value.is_empty().then(|| {
            lines[i + 1..j]
                .iter()
                .filter_map(|line| {
                    let t = line.trim();
                    let item = t.strip_prefix("- ").or((t == "-").then_some(""))?;
                    Some(unquote(item.trim()))
                })
                .filter(|item| !item.is_empty())
                .collect()
        });
        i = j;
    }
    found
}

/// True when the last top-level `key:` line opens a YAML block scalar (`|`
/// or `>`, with optional chomping/indentation indicators and a trailing
/// comment). [`parse`] reads such a key as the bare indicator, not the text
/// below it, so a caller that would act on the value (a sync would send a
/// literal `|`) must refuse instead. False without frontmatter.
pub fn is_block_scalar(content: &str, key: &str) -> bool {
    let Ok((start, end)) = block_bounds(content) else {
        return false;
    };
    content[start..end]
        .split('\n')
        .rfind(|line| top_level_key(line) == Some(key))
        .and_then(|line| line.split_once(':'))
        .is_some_and(|(_, value)| {
            let value = value.trim();
            let value = value
                .split_once(" #")
                .map_or(value, |(head, _)| head)
                .trim();
            let mut chars = value.chars();
            matches!(chars.next(), Some('|' | '>'))
                && chars.all(|c| matches!(c, '+' | '-' | '1'..='9'))
        })
}

/// A line that starts a top-level key: no leading whitespace, not a comment
/// or list item, not blank, and containing a colon. Returns the trimmed key.
pub(crate) fn top_level_key(line: &str) -> Option<&str> {
    if line.starts_with(|c: char| c.is_whitespace() || c == '#' || c == '-') {
        return None;
    }
    if line.trim_end().is_empty() {
        return None;
    }
    let (key, _) = line.split_once(':')?;
    let key = key.trim();
    (!key.is_empty()).then_some(key)
}

/// A line that belongs to the preceding key: indented, a list item, or
/// blank. (Trailing blank lines are handed back to the stream by `patch`.)
fn is_continuation(line: &str) -> bool {
    let t = line.trim_end();
    t.is_empty()
        || line.starts_with(|c: char| c.is_whitespace())
        || line.starts_with("- ")
        || t == "-"
}

fn edit_for<'e>(edits: &'e [(&str, KeyEdit)], key: &str) -> Option<&'e KeyEdit> {
    edits.iter().find(|(k, _)| *k == key).map(|(_, e)| e)
}

fn render_edit(key: &str, edit: &KeyEdit, cr: &str) -> Vec<String> {
    match edit {
        KeyEdit::Set(value) => {
            let line = if value.is_empty() {
                format!("{key}: \"\"")
            } else if needs_quoting(value) {
                format!("{key}: {}", quote(value))
            } else {
                format!("{key}: {value}")
            };
            vec![format!("{line}{cr}")]
        }
        KeyEdit::Quoted(value) => {
            let quoted = serde_json::to_string(value).expect("a string always serializes");
            vec![format!("{key}: {quoted}{cr}")]
        }
        KeyEdit::Remove => Vec::new(),
        KeyEdit::Block(lines) => lines.iter().map(|l| format!("{l}{cr}")).collect(),
    }
}

/// Rewrite only the named top-level keys of an existing document and keep
/// every other byte.
///
/// The frontmatter block is located exactly as [`parse`] finds it (BOM and
/// leading whitespace tolerated, first `\n---` closes), so a file `parse`
/// reads as frontmatter is the file this edits; anything else is refused
/// and the input is left alone. A key's span is its line plus the indented,
/// list-item and blank lines that follow (so a YAML list or block scalar is
/// replaced whole); trailing blank lines stay in place. For each edited key
/// the FIRST occurrence's span is replaced and later duplicates are dropped,
/// so last-wins readers see the edit. `Set`/`Quoted` keys that are absent are
/// inserted, in `edits` order, just before the first top-level `tags` line
/// (or at the end of the block); an absent `Block` is appended last; an
/// absent `Remove` is a no-op. Emitted lines take the block's CRLF style.
/// A `Set`/`Quoted`/`Remove` key that [`parse`] would still read from an INDENTED
/// line (inside a block scalar or nested map) is refused, because the edit
/// would not be what readers see.
///
/// `body: None` keeps the closing fence and body byte-for-byte.
/// `body: Some(b)` replaces them with the canonical render shape: the fence,
/// then a blank line and `b` trimmed when `b` is non-blank.
pub fn patch(
    content: &str,
    edits: &[(&str, KeyEdit)],
    body: Option<&str>,
) -> Result<String, String> {
    let (block_start, block_end) = block_bounds(content)?;
    let block = &content[block_start..block_end];
    // Include the newline that ends the last line, so a one-line CRLF block
    // is still recognised.
    let cr = if content[block_start..=block_end].contains("\r\n") {
        "\r"
    } else {
        ""
    };
    let lines: Vec<&str> = block.split('\n').collect();

    // Segment the block into loose lines and key spans.
    let mut segments: Vec<(Option<&str>, usize, usize)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        match top_level_key(lines[i]) {
            Some(key) => {
                let mut end = i + 1;
                while end < lines.len() && is_continuation(lines[end]) {
                    end += 1;
                }
                while end > i + 1 && lines[end - 1].trim().is_empty() {
                    end -= 1;
                }
                segments.push((Some(key), i, end));
                i = end;
            }
            None => {
                segments.push((None, i, i + 1));
                i += 1;
            }
        }
    }

    let mut out: Vec<String> = Vec::with_capacity(lines.len() + edits.len());
    let mut handled: Vec<&str> = Vec::new();
    let mut tags_at: Option<usize> = None;
    for (key, start, end) in segments {
        let Some(key) = key else {
            out.push(lines[start].to_string());
            continue;
        };
        match edit_for(edits, key) {
            None => {
                if key == "tags" && tags_at.is_none() {
                    tags_at = Some(out.len());
                }
                out.extend(lines[start..end].iter().map(|l| l.to_string()));
            }
            Some(edit) => {
                if handled.contains(&key) {
                    // A later duplicate: drop it so the edit is what readers see.
                    continue;
                }
                handled.push(key);
                let rendered = render_edit(key, edit, cr);
                if key == "tags" && tags_at.is_none() && !rendered.is_empty() {
                    tags_at = Some(out.len());
                }
                out.extend(rendered);
            }
        }
    }

    let mut inserts: Vec<String> = Vec::new();
    let mut blocks: Vec<String> = Vec::new();
    for (key, edit) in edits {
        let key: &str = key;
        if handled.contains(&key) {
            continue;
        }
        handled.push(key);
        match edit {
            KeyEdit::Set(_) | KeyEdit::Quoted(_) => inserts.extend(render_edit(key, edit, cr)),
            KeyEdit::Block(_) => blocks.extend(render_edit(key, edit, cr)),
            KeyEdit::Remove => {}
        }
    }
    let tail = out.split_off(tags_at.unwrap_or(out.len()));
    out.extend(inserts);
    out.extend(tail);
    out.extend(blocks);

    // `parse` also reads an INDENTED `key: value` line (inside a block scalar
    // or nested map) as a key, last-wins, while this patcher only rewrites
    // top-level lines. Such a line would shadow the edit: the write would
    // report success yet read back unchanged. Refuse instead.
    for (key, edit) in edits {
        if matches!(edit, KeyEdit::Block(_)) {
            continue;
        }
        let shadowed = out.iter().any(|line| {
            let lt = line.trim_start();
            top_level_key(line).is_none()
                && !lt.starts_with("- ")
                && lt.split_once(':').is_some_and(|(k, _)| k.trim() == *key)
        });
        if shadowed {
            return Err(format!(
                "`{key}` also appears as an indented line inside another frontmatter key, which readers would see instead of this edit; file preserved. Move or rename that line, then retry."
            ));
        }
    }

    let head = &content[..block_start];
    let patched = out.join("\n");
    Ok(match body {
        None => format!("{head}{patched}{}", &content[block_end..]),
        Some(b) => {
            let b = b.trim();
            if b.is_empty() {
                format!("{head}{patched}\n---\n")
            } else {
                format!("{head}{patched}\n---\n\n{b}\n")
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_quoting_covers_the_union_of_writer_predicates() {
        // The cases the four pre-AQ-05 writers disagreed on.
        for v in [
            "a: b",
            "tag #x",
            "line\nbreak",
            " leading",
            "trailing ",
            "true",
            "false",
            "null",
            "- list",
            "&anchor",
            "*alias",
            "!tag",
            "|block",
            ">fold",
            "[flow",
            "{flow",
            "'quoted",
            "\"quoted",
            "@reserved",
            "`tick",
            "has \" inside",
        ] {
            assert!(needs_quoting(v), "must quote: {v:?}");
        }
        for v in ["plain", "Hello World", "P1", "2026-06-10", "a.b (c)"] {
            assert!(!needs_quoting(v), "must stay bare: {v:?}");
        }
    }

    #[test]
    fn quote_unquote_round_trips_hostile_scalars() {
        for v in [
            "Re: numbers \"Q2\" #final",
            "- starts like a list: with colon",
            "back\\slash and \"quotes\"",
        ] {
            assert_eq!(unquote(&quote(v)), v);
        }
    }

    #[test]
    fn parse_basic_pairs() {
        let fm = parse("---\ntitle: Hello\nstatus: active\n---\n\nbody text");
        assert_eq!(fm.get("title"), Some("Hello"));
        assert_eq!(fm.get("status"), Some("active"));
        assert_eq!(fm.body.trim(), "body text");
    }

    #[test]
    fn parse_no_frontmatter_is_all_body() {
        let fm = parse("just a plain note\nwith lines");
        assert!(fm.pairs.is_empty());
        assert_eq!(fm.body, "just a plain note\nwith lines");
    }

    #[test]
    fn parse_unterminated_fence_is_all_body() {
        let input = "---\ntitle: x\nno closing fence here";
        let fm = parse(input);
        assert!(fm.pairs.is_empty());
        assert_eq!(fm.body, input);
    }

    #[test]
    fn parse_strips_bom() {
        let fm = parse("\u{feff}---\ntitle: WithBom\n---\nbody");
        assert_eq!(fm.get("title"), Some("WithBom"));
    }

    #[test]
    fn parse_tolerates_crlf() {
        let fm = parse("---\r\ntitle: Win\r\nstatus: ok\r\n---\r\n\r\nbody");
        // The trailing \r must not survive on the value.
        assert_eq!(fm.get("title"), Some("Win"));
        assert_eq!(fm.get("status"), Some("ok"));
    }

    #[test]
    fn parse_unquotes_colon_value() {
        let fm = parse("---\ntitle: \"With: colon\"\nstatus: active\n---\n\nbody");
        assert_eq!(fm.get("title"), Some("With: colon"));
        assert_eq!(fm.get("status"), Some("active"));
        assert_eq!(fm.body.trim(), "body");
    }

    #[test]
    fn parse_skips_list_items() {
        let fm = parse("---\ntitle: T\ntags:\n  - tasks\n  - status/done\n---\nbody");
        assert_eq!(fm.get("title"), Some("T"));
        // `tags:` parses to an empty value; the list items are skipped, so
        // no `tasks`/`status` keys leak in.
        assert_eq!(fm.pairs.iter().filter(|(k, _)| k == "tags").count(), 1);
        assert_eq!(fm.get("tags"), Some(""));
    }

    #[test]
    fn parse_preserves_order_and_get_is_last_wins() {
        let fm = parse("---\na: 1\nb: 2\na: 3\n---\n");
        assert_eq!(
            fm.pairs,
            vec![
                ("a".to_string(), "1".to_string()),
                ("b".to_string(), "2".to_string()),
                ("a".to_string(), "3".to_string()),
            ]
        );
        // last-wins, matching the old BTreeMap insert loop
        assert_eq!(fm.get("a"), Some("3"));
    }

    #[test]
    fn parse_map_last_wins() {
        let (map, body) = parse_map("---\na: 1\na: 3\nb: 2\n---\nrest");
        assert_eq!(map.get("a"), Some(&"3".to_string()));
        assert_eq!(map.get("b"), Some(&"2".to_string()));
        assert_eq!(body, "rest");
    }

    #[test]
    fn quote_unquote_round_trip() {
        for v in [
            "plain",
            "has: colon",
            "has # hash",
            "line\nbreak",
            " leading",
            "trailing ",
            "true",
            "false",
            "null",
            "quote\"inside",
            "back\\slash",
        ] {
            if needs_quoting(v) {
                let q = quote(v);
                assert_eq!(unquote(&q), v, "round-trip failed for {v:?}");
            } else {
                // Bare values unquote to themselves.
                assert_eq!(unquote(v), v);
            }
        }
    }

    #[test]
    fn needs_quoting_flags_ambiguous_only() {
        assert!(!needs_quoting("simple"));
        assert!(!needs_quoting("2026-06-01"));
        assert!(!needs_quoting("In progress")); // spaces in the middle are fine
        assert!(needs_quoting("with: colon"));
        assert!(needs_quoting("with # hash"));
        assert!(needs_quoting(" leading"));
        assert!(needs_quoting("trailing "));
        assert!(needs_quoting("true"));
    }

    #[test]
    fn unquote_single_quotes_verbatim() {
        assert_eq!(unquote("'hello world'"), "hello world");
        // single-quoted does NOT interpret backslash escapes
        assert_eq!(unquote("'a\\nb'"), "a\\nb");
    }

    #[test]
    fn push_kv_skips_empty_and_quotes_when_needed() {
        let mut lines = Vec::new();
        push_kv(&mut lines, "empty", "");
        push_kv(&mut lines, "plain", "value");
        push_kv(&mut lines, "colon", "a: b");
        assert_eq!(
            lines,
            vec!["plain: value".to_string(), "colon: \"a: b\"".to_string()]
        );
    }

    #[test]
    fn body_excludes_closing_fence_and_leading_blank() {
        let fm = parse("---\nk: v\n---\n\n# Heading\n\ntext");
        assert_eq!(fm.body, "# Heading\n\ntext");
        assert!(!fm.body.starts_with("---"));
    }

    fn set(v: &str) -> KeyEdit {
        KeyEdit::Set(v.into())
    }

    #[test]
    fn patch_changes_only_the_named_key_and_keeps_every_other_byte() {
        let input = "---\ntitle: \"T\"\nstatus: Old\naliases:\n  - One\n  - Two\ncustom: keep me\n# comment\nnotes: |\n  first\n\n  second\nlast: 1\n---\n\nBody stays.\n\n- [ ] step\n";
        let out = patch(input, &[("status", set("New"))], None).unwrap();
        assert_eq!(out, input.replacen("status: Old", "status: New", 1));
    }

    #[test]
    fn patch_replaces_a_key_together_with_its_list_or_block_continuation() {
        let input = "---\nlabels:\n  - a\n  - b\nnext: 1\n---\nbody";
        let out = patch(input, &[("labels", set("x"))], None).unwrap();
        assert_eq!(out, "---\nlabels: x\nnext: 1\n---\nbody");

        let input = "---\ndescription: |\n  line one\n\n  line two\n\nfollowing: kept\n---\n";
        let out = patch(input, &[("description", set("short"))], None).unwrap();
        // The block scalar's indented lines go with it; the blank line after
        // the span and the following key survive.
        assert_eq!(out, "---\ndescription: short\n\nfollowing: kept\n---\n");
    }

    #[test]
    fn patch_removes_keys_and_later_duplicates_so_last_wins_readers_see_the_edit() {
        let input = "---\nstatus: A\nother: x\nstatus: B\n---\n";
        let out = patch(input, &[("status", set("C"))], None).unwrap();
        assert_eq!(out, "---\nstatus: C\nother: x\n---\n");
        assert_eq!(
            parse_map(&out).0.get("status").map(String::as_str),
            Some("C")
        );

        let input = "---\ntitle: T\nrange:\n  - a\n  - b\nkeep: y\n---\n";
        let out = patch(input, &[("range", KeyEdit::Remove)], None).unwrap();
        assert_eq!(out, "---\ntitle: T\nkeep: y\n---\n");
        let unchanged = patch(input, &[("absent", KeyEdit::Remove)], None).unwrap();
        assert_eq!(unchanged, input);
    }

    #[test]
    fn patch_inserts_missing_keys_before_tags_and_appends_missing_blocks() {
        let input = "---\ntitle: T\ntags:\n  - tasks\n---\n";
        let out = patch(input, &[("priority", set("P1"))], None).unwrap();
        assert_eq!(out, "---\ntitle: T\npriority: P1\ntags:\n  - tasks\n---\n");

        let input = "---\ntitle: T\n---\n";
        let block = KeyEdit::Block(vec!["tags:".into(), "  - tasks".into()]);
        let out = patch(
            input,
            &[
                ("tags", block),
                ("priority", set("P2")),
                ("due", set("2026-07-01")),
            ],
            None,
        )
        .unwrap();
        assert_eq!(
            out,
            "---\ntitle: T\npriority: P2\ndue: 2026-07-01\ntags:\n  - tasks\n---\n"
        );
    }

    #[test]
    fn patch_preserves_crlf_and_bom_and_quotes_like_push_kv() {
        let input =
            "\u{feff}---\r\ntitle: T\r\nstatus: Old\r\nlist:\r\n  - a\r\n---\r\n\r\nbody\r\n";
        let out = patch(input, &[("status", set("New")), ("added", set("x"))], None).unwrap();
        assert_eq!(
            out,
            "\u{feff}---\r\ntitle: T\r\nstatus: New\r\nlist:\r\n  - a\r\nadded: x\r\n---\r\n\r\nbody\r\n"
        );

        let out = patch(
            "---\ntitle: T\n---\n",
            &[("k", set("a: b")), ("empty", set(""))],
            None,
        )
        .unwrap();
        assert_eq!(out, "---\ntitle: T\nk: \"a: b\"\nempty: \"\"\n---\n");
        let (map, _) = parse_map(&out);
        assert_eq!(map.get("k").map(String::as_str), Some("a: b"));
        assert_eq!(map.get("empty").map(String::as_str), Some(""));
    }

    #[test]
    fn patch_keeps_body_bytes_unless_a_new_body_is_given() {
        let input = "---\ntitle: T\n---\n\n  indented body  \n\n\n";
        let kept = patch(input, &[("title", set("U"))], None).unwrap();
        assert_eq!(kept, "---\ntitle: U\n---\n\n  indented body  \n\n\n");
        let replaced = patch(input, &[], Some("x")).unwrap();
        assert_eq!(replaced, "---\ntitle: T\n---\n\nx\n");
        let emptied = patch(input, &[], Some("  ")).unwrap();
        assert_eq!(emptied, "---\ntitle: T\n---\n");
    }

    #[test]
    fn patch_refuses_content_parse_would_not_read_as_frontmatter() {
        assert!(patch("no frontmatter here", &[("k", set("v"))], None).is_err());
        assert!(patch("---\ntitle: never closed\n", &[("k", set("v"))], None).is_err());
    }

    #[test]
    fn patch_refuses_an_edit_an_indented_line_would_shadow() {
        // `parse` reads the indented `status:` inside the block scalar last,
        // so rewriting only the top-level line would be a silent no-op.
        let input = "---\nstatus: Not started\nnotes: |\n  status: waiting on legal\n---\n";
        assert_eq!(
            parse_map(input).0.get("status").map(String::as_str),
            Some("waiting on legal")
        );
        let error = patch(input, &[("status", set("Delivered"))], None).unwrap_err();
        assert!(error.contains("`status`"), "{error}");
        assert!(patch(input, &[("status", KeyEdit::Remove)], None).is_err());
        // Replacing the block that holds the indented line clears the shadow,
        // and edits to other keys are unaffected.
        let out = patch(
            input,
            &[("notes", set("cleared")), ("status", set("Delivered"))],
            None,
        )
        .unwrap();
        assert_eq!(
            parse_map(&out).0.get("status").map(String::as_str),
            Some("Delivered")
        );
        assert!(patch(input, &[("other", set("x"))], None).is_ok());
    }

    #[test]
    fn quoted_edit_always_json_quotes_and_reads_back_exactly() {
        let input = "---\ntitle: old\ntags:\n  - a\n---\nBody\n";
        let value = "tab\there \"q\" \\ \u{1}";
        let out = patch(
            input,
            &[
                ("title", KeyEdit::Quoted("plain".into())),
                ("note", KeyEdit::Quoted(value.into())),
            ],
            None,
        )
        .unwrap();
        assert_eq!(
            out,
            format!(
                "---\ntitle: \"plain\"\nnote: {}\ntags:\n  - a\n---\nBody\n",
                serde_json::to_string(value).unwrap()
            )
        );
        assert_eq!(parse_map(&out).0["note"], value);
        let shadowed = "---\nstatus: a\nnotes: |\n  status: b\n---\n";
        assert!(patch(shadowed, &[("status", KeyEdit::Quoted("c".into()))], None).is_err());
    }

    #[test]
    fn is_block_scalar_reads_the_raw_indicator_not_a_quoted_pipe() {
        let doc =
            "---\ndescription: |-\n  text\nfolded: > # note\nquoted: \"|\"\ntitle: plain\n---\n";
        assert!(is_block_scalar(doc, "description"));
        assert!(is_block_scalar(doc, "folded"));
        assert!(!is_block_scalar(doc, "quoted"));
        assert!(!is_block_scalar(doc, "title"));
        assert!(!is_block_scalar(doc, "absent"));
        assert!(!is_block_scalar("no frontmatter", "description"));
    }

    #[test]
    fn unquote_inverts_quote_for_backslash_sequences() {
        for v in [
            "C:\\new folder",
            "a\\\\nb",
            "two\nlines",
            "tab\\t kept",
            "end\\",
            "\"q\" \\ \"q\"",
        ] {
            assert_eq!(unquote(&quote(v)), v, "quote round trip for {v:?}");
        }
        // The chained-replace parser read this back as `C:\` + newline + `ew`.
        assert_eq!(unquote("\"C:\\\\new\""), "C:\\new");
    }

    #[test]
    fn unquote_reads_json_quoted_values_exactly() {
        // The retired Notion sync wrote mapped values with serde_json::to_string.
        for v in [
            "tab\there",
            "cr\rlf\n",
            "C:\\new and \\t",
            "\"quoted\" / slash",
            "ctrl\u{1}\u{8}\u{c}",
            "emoji 😀 and é",
        ] {
            let json = serde_json::to_string(v).unwrap();
            assert_eq!(unquote(&json), v, "JSON round trip for {json}");
        }
        assert_eq!(unquote("\"\\ud83d\\ude00 \\u00e9\\/\""), "😀 é/");
    }

    #[test]
    fn unquote_keeps_unknown_or_broken_escapes_verbatim() {
        assert_eq!(unquote("\"a\\qb\""), "a\\qb");
        assert_eq!(unquote("\"bad \\u12 hex\""), "bad \\u12 hex");
        assert_eq!(unquote("\"lone \\ud800 high\""), "lone \\ud800 high");
        assert_eq!(unquote("\"trailing\\\""), "trailing\\");
        assert_eq!(unquote("\"üñï\\çødé\""), "üñï\\çødé");
    }

    #[test]
    fn list_items_reads_a_block_list_and_ignores_scalars() {
        let doc = "---\ntitle: T\nlabels:\n  - ui\n  - \"back: end\"\n  -\nnext: 1\n---\nbody";
        assert_eq!(
            list_items(doc, "labels"),
            Some(vec!["ui".to_string(), "back: end".to_string()])
        );
        assert_eq!(list_items(doc, "title"), None);
        assert_eq!(list_items(doc, "absent"), None);
        assert_eq!(list_items("---\nlabels: a, b\n---\n", "labels"), None);
        assert_eq!(list_items("---\nlabels:\n---\n", "labels"), Some(vec![]));
        assert_eq!(list_items("no frontmatter", "labels"), None);
        // Last top-level occurrence wins, as in parse_map.
        assert_eq!(
            list_items("---\nlabels:\n  - a\nlabels: b\n---\n", "labels"),
            None
        );
    }
}
