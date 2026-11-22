//! Gemini's Google Takeout export (H5b): "My Activity" for Gemini Apps, a zip
//! or the extracted folder, holding `.../Gemini Apps/MyActivity.json` (a JSON
//! array with one record per prompt). Attachments are listed by name only
//! (N28).
//!
//! **No conversation id.** Takeout has one record per prompt, so [`stage`]
//! reads every record (streaming, one at a time), sorts them by time and
//! starts a new conversation after more than [`GAP_MINUTES`] minutes without
//! a prompt (N30). Each group is written as one staged JSON file the shared
//! inspect, preview and commit code reads like any other ([`read`]); its id
//! is a hash of the first record's time and prompt, so the same chat dedupes
//! on re-import and a group that later grew updates in place. Every group is
//! labelled [`GROUPING`] and lands in Quick chats (there is no folder).
//!
//! **A record** is kept when its `header` names Gemini Apps (or Bard) and its
//! `title` starts with "Prompted"; other activity (feedback, "Used ...") is
//! ignored. The response is HTML: [`html_text`] turns it into text with a
//! small scanner (tags dropped, `script`/`style` content dropped, block tags
//! become line breaks, entities decoded once); nothing is rendered or
//! executed. A record that isn't an object, has no usable time, or has
//! neither a prompt nor an attachment is skipped and counted in
//! [`archive::Staged::malformed`].
//!
//! **HTML export.** Takeout can also write `MyActivity.html`, whose times are
//! locale text with zone abbreviations that can't be read back reliably, so
//! an export holding only HTML reports "not supported" rather than guess.

use super::archive::{self, Export, Staged, Streamed};
use super::model::{Builder, Parsed, MESSAGE_MAX_CHARS};
use super::walker::Entry;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// A new conversation starts after more than this many minutes without a
/// prompt.
pub(crate) const GAP_MINUTES: i64 = 30;
/// The label on every grouped conversation.
pub(crate) const GROUPING: &str = "Grouped by time";
/// A staged group file bigger than this isn't read.
const MAX_GROUP_FILE: u64 = 256 * 1024 * 1024;
/// Text kept across all records while grouping (a Takeout of years of
/// activity can be large); past it the rest isn't read.
const MAX_HELD_BYTES: usize = 512 * 1024 * 1024;
/// Records kept while grouping.
const MAX_RECORDS: usize = 1_000_000;
/// Attachment names kept per record.
const MAX_FILES: usize = 50;

/// Whether an export file name is a Gemini Apps activity JSON: a
/// `MyActivity.json` inside a folder named for Gemini Apps (or its old name,
/// Bard), at most six folders deep.
pub(crate) fn is_shard(name: &str) -> bool {
    let parts: Vec<&str> = name.split('/').collect();
    if parts.is_empty() || parts.len() > 7 {
        return false;
    }
    let file = parts[parts.len() - 1].to_ascii_lowercase();
    if file != "myactivity.json" {
        return false;
    }
    // Picked inside the Gemini Apps folder, the file has no parent name: the
    // records' own header then decides.
    parts.len() == 1 || {
        let folder = parts[parts.len() - 2].to_ascii_lowercase();
        folder.contains("gemini") || folder.contains("bard")
    }
}

/// A staged group file, as the walker lists it.
pub(crate) fn is_transcript(entry: &Entry) -> bool {
    super::chatgpt::is_transcript(entry)
}

/// One prompt and its response, as kept while grouping.
struct Record {
    time: DateTime<Utc>,
    prompt: String,
    response: String,
    files: Vec<String>,
}

impl Record {
    fn weight(&self) -> usize {
        self.prompt.len() + self.response.len() + self.files.iter().map(String::len).sum::<usize>()
    }
}

fn clean(text: &str, max_chars: usize) -> String {
    let kept: String = text
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .take(max_chars)
        .collect();
    kept.trim().to_string()
}

/// The prompt behind a `Prompted <text>` title (the space after the word is
/// often a non-breaking one).
fn prompt_of(title: &str) -> Option<&str> {
    let rest = title.trim_start().strip_prefix("Prompted")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(rest.trim())
}

fn is_gemini_header(record: &serde_json::Map<String, Value>) -> bool {
    let header = record
        .get("header")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    if header.is_empty() {
        // Some exports carry the product only in `products`.
        return record
            .get("products")
            .and_then(Value::as_array)
            .is_some_and(|p| {
                p.iter().filter_map(Value::as_str).any(|p| {
                    let p = p.to_ascii_lowercase();
                    p.contains("gemini") || p.contains("bard")
                })
            });
    }
    header.contains("gemini") || header.contains("bard")
}

/// Attachment names a record carries: `attachedFiles` (strings or objects
/// with a `name`) and `subtitles` entries that read "- <name>" (Takeout's
/// "Attached 2 files." followed by one line per file).
fn file_names(record: &serde_json::Map<String, Value>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |name: &str| {
        let name = clean(name, 200);
        if !name.is_empty() && out.len() < MAX_FILES {
            out.push(name);
        }
    };
    if let Some(list) = record.get("attachedFiles").and_then(Value::as_array) {
        for item in list {
            match item {
                Value::String(s) => add(s),
                Value::Object(o) => {
                    if let Some(n) = o.get("name").and_then(Value::as_str) {
                        add(n);
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(list) = record.get("subtitles").and_then(Value::as_array) {
        for item in list {
            if let Some(name) = item.get("name").and_then(Value::as_str) {
                if let Some(file) = name.trim_start().strip_prefix("- ") {
                    add(file);
                }
            }
        }
    }
    out
}

/// The response HTML: `safeHtmlItem[].html`, else `details[].html`, else a
/// string `description`.
fn response_html(record: &serde_json::Map<String, Value>) -> String {
    let from = |key: &str| -> Vec<String> {
        record
            .get(key)
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|i| i.get("html").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut parts = from("safeHtmlItem");
    if parts.is_empty() {
        parts = from("details");
    }
    if parts.is_empty() {
        if let Some(d) = record.get("description").and_then(Value::as_str) {
            parts.push(d.to_string());
        }
    }
    parts.join("\n")
}

/// One activity record, or why it isn't one: `Ok(None)` is activity that
/// isn't a prompt (ignored), `Err(())` a record that is malformed.
fn record_of(value: &Value) -> Result<Option<Record>, ()> {
    let obj = value.as_object().ok_or(())?;
    if !is_gemini_header(obj) {
        return Ok(None);
    }
    let Some(title) = obj.get("title").and_then(Value::as_str) else {
        return Err(());
    };
    let Some(prompt) = prompt_of(title) else {
        return Ok(None);
    };
    let time = obj
        .get("time")
        .and_then(Value::as_str)
        .and_then(|t| DateTime::parse_from_rfc3339(t.trim()).ok())
        .map(|t| t.with_timezone(&Utc))
        .ok_or(())?;
    let prompt = clean(prompt, MESSAGE_MAX_CHARS);
    let files = file_names(obj);
    if prompt.is_empty() && files.is_empty() {
        return Err(());
    }
    let response = clean(&html_text(&response_html(obj)), MESSAGE_MAX_CHARS);
    Ok(Some(Record {
        time,
        prompt,
        response,
        files,
    }))
}

fn time_text(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// The conversation id: a hash of the first record's time and prompt.
fn group_id(first: &Record) -> String {
    let mut h = Sha256::new();
    h.update(time_text(first.time).as_bytes());
    h.update([0]);
    h.update(first.prompt.as_bytes());
    format!("{:x}", h.finalize())[..32].to_string()
}

/// Read the activity files of `export`, group the records by time and write
/// one staged file per group into `dir` (module docs). An export with no
/// activity JSON stages nothing (`shards` 0: not supported).
pub(crate) fn stage(
    export: &mut Export,
    dir: &Path,
    cancel: &AtomicBool,
) -> Result<Staged, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("staging folder: {e}"))?;
    let mut shards: Vec<String> = export
        .items()
        .iter()
        .filter(|i| !i.nested && is_shard(&i.name))
        .map(|i| i.name.clone())
        .collect();
    shards.sort();
    let mut out = Staged::default();
    archive::tally_other_files(export, &shards, &mut out);
    let mut records: Vec<Record> = Vec::new();
    let mut held = 0usize;
    let mut seen: HashSet<(i64, u32, String)> = HashSet::new();
    for shard in &shards {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        out.shards += 1;
        let read = export.read(shard, |reader| {
            archive::for_each_element(reader, archive::MAX_ELEMENT_BYTES, |value| {
                if cancel.load(Ordering::Relaxed) {
                    return false;
                }
                match record_of(&value) {
                    Err(()) => out.malformed += 1,
                    Ok(None) => {}
                    Ok(Some(r)) => {
                        let key = (r.time.timestamp(), r.time.timestamp_subsec_nanos(), {
                            let mut h = Sha256::new();
                            h.update(r.prompt.as_bytes());
                            format!("{:x}", h.finalize())
                        });
                        if !seen.insert(key) {
                            out.duplicates += 1;
                        } else if records.len() >= MAX_RECORDS || held + r.weight() > MAX_HELD_BYTES
                        {
                            out.capped = true;
                            return false;
                        } else {
                            held += r.weight();
                            records.push(r);
                        }
                    }
                }
                true
            })
        });
        match read {
            Err(e @ (archive::ArchiveError::Ratio(_) | archive::ArchiveError::TooLarge(_))) => {
                return Err(e.to_string())
            }
            Ok(Streamed::Broken(_)) | Err(_) => out.damaged.push(shard.clone()),
            Ok(_) => {}
        }
        if out.capped {
            break;
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Ok(out);
    }
    // Stable: records at the same time keep the file's order.
    records.sort_by_key(|r| r.time);
    let gap = Duration::minutes(GAP_MINUTES);
    let mut groups: Vec<Vec<&Record>> = Vec::new();
    for r in &records {
        match groups.last_mut() {
            Some(g) if r.time - g.last().map_or(r.time, |p| p.time) <= gap => g.push(r),
            _ => groups.push(vec![r]),
        }
    }
    for group in groups {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if out.conversations >= archive::MAX_STAGED {
            out.capped = true;
            break;
        }
        let id = group_id(group[0]);
        let entries: Vec<Value> = group
            .iter()
            .map(|r| {
                json!({
                    "time": time_text(r.time),
                    "prompt": r.prompt,
                    "response": r.response,
                    "files": r.files,
                })
            })
            .collect();
        let bytes = serde_json::to_vec(&json!({
            "format": "gemini-group",
            "id": id,
            "entries": entries,
        }))
        .map_err(|e| e.to_string())?;
        std::fs::write(dir.join(archive::staged_file_name(&id)), bytes)
            .map_err(|e| format!("staging write: {e}"))?;
        out.conversations += 1;
    }
    Ok(out)
}

/// Parse one staged group (a [`stage`] file's JSON).
pub(crate) fn parse(group: &Value, cap: usize) -> Parsed {
    let mut builder = Builder::with_cap(cap);
    let obj = group.as_object();
    let id = obj
        .and_then(|o| o.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|i| !i.is_empty() && i.len() <= 64 && i.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(str::to_string);
    let entries = obj
        .filter(|o| o.get("format").and_then(Value::as_str) == Some("gemini-group"))
        .and_then(|o| o.get("entries"))
        .and_then(Value::as_array);
    let Some(entries) = entries else {
        return builder.finish();
    };
    {
        let meta = builder.meta();
        meta.recognized = id.is_some();
        meta.id = id;
        meta.grouping = Some(GROUPING.to_string());
    }
    for entry in entries {
        let Some(e) = entry.as_object() else {
            builder.meta().malformed += 1;
            continue;
        };
        let at = e.get("time").and_then(Value::as_str);
        builder.saw_time(at);
        let files: Vec<String> = e
            .get("files")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        builder.user(
            e.get("prompt").and_then(Value::as_str).unwrap_or(""),
            files,
            at,
        );
        builder.assistant(e.get("response").and_then(Value::as_str).unwrap_or(""), at);
    }
    builder.finish()
}

/// Read one staged group file.
pub(crate) fn read(path: &Path, max_bytes: usize) -> Result<Parsed, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("conversation file: {e}"))?;
    let mut bytes = Vec::new();
    file.take(MAX_GROUP_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("conversation file: {e}"))?;
    if bytes.len() as u64 > MAX_GROUP_FILE {
        return Err("That conversation is too large to read.".into());
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("conversation file: {e}"))?;
    Ok(parse(&value, max_bytes))
}

// ── HTML to text ──────────────────────────────────────────────────────

/// Tags that end a line.
fn is_block(tag: &str) -> bool {
    matches!(
        tag,
        "p" | "div"
            | "br"
            | "li"
            | "ul"
            | "ol"
            | "tr"
            | "table"
            | "pre"
            | "blockquote"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "hr"
    )
}

/// An entity's text (`amp`, `#39`, `#x1F600`), or `None` when unknown.
fn entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse::<u32>().ok()?,
        };
        return char::from_u32(code);
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "ndash" => '-',
        "mdash" => '-',
        "hellip" => '\u{2026}',
        "lsquo" | "rsquo" => '\'',
        "ldquo" | "rdquo" => '"',
        "bull" => '\u{2022}',
        "copy" => '\u{a9}',
        "times" => '\u{d7}',
        _ => return None,
    })
}

/// Safe text from an HTML fragment: no tag survives, `script` and `style`
/// bodies and comments are dropped, block tags and `<br>` become line
/// breaks, list items get a dash, and entities are decoded once (so
/// `&lt;b&gt;` stays the four characters `<b>`, never a tag). Pure string
/// scanning: no parser, nothing fetched or run. Not for rendering.
pub(crate) fn html_text(html: &str) -> String {
    let chars: Vec<char> = html.chars().collect();
    let mut out = String::with_capacity(html.len() / 2);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '<' {
            let next = chars.get(i + 1).copied().unwrap_or(' ');
            if next == '!' && chars[i..].starts_with(&['<', '!', '-', '-']) {
                // A comment: to `-->` (an unterminated one drops the rest).
                let mut j = i + 4;
                while j < chars.len() && !chars[j..].starts_with(&['-', '-', '>']) {
                    j += 1;
                }
                i = (j + 3).min(chars.len());
                continue;
            }
            if next.is_ascii_alphabetic() || next == '/' || next == '!' || next == '?' {
                // A tag: to the closing `>`, skipping quoted values.
                let mut j = i + 1;
                let mut quote: Option<char> = None;
                while j < chars.len() {
                    match (quote, chars[j]) {
                        (None, '>') => break,
                        (None, q @ ('"' | '\'')) => quote = Some(q),
                        (Some(q), d) if d == q => quote = None,
                        _ => {}
                    }
                    j += 1;
                }
                let body: String = chars[i + 1..j.min(chars.len())].iter().collect();
                let closing = body.starts_with('/');
                let name: String = body
                    .trim_start_matches('/')
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric())
                    .collect::<String>()
                    .to_ascii_lowercase();
                i = (j + 1).min(chars.len());
                if !closing && (name == "script" || name == "style") {
                    // Skip to the matching close tag (case-insensitive).
                    let close: Vec<char> = format!("</{name}").chars().collect();
                    while i < chars.len() {
                        if chars[i] == '<'
                            && chars.len() >= i + close.len()
                            && chars[i..i + close.len()]
                                .iter()
                                .zip(&close)
                                .all(|(a, b)| a.to_ascii_lowercase() == *b)
                        {
                            while i < chars.len() && chars[i] != '>' {
                                i += 1;
                            }
                            i = (i + 1).min(chars.len());
                            break;
                        }
                        i += 1;
                    }
                    continue;
                }
                if is_block(&name) {
                    if !out.ends_with('\n') && !out.is_empty() {
                        out.push('\n');
                    }
                    if !closing && name == "li" {
                        out.push_str("- ");
                    }
                }
                continue;
            }
            out.push('<');
            i += 1;
            continue;
        }
        if c == '&' {
            // `&name;` within 12 characters, else a plain ampersand.
            let end = chars[i + 1..]
                .iter()
                .take(12)
                .position(|&d| d == ';')
                .map(|p| i + 1 + p);
            if let Some(end) = end {
                let name: String = chars[i + 1..end].iter().collect();
                if let Some(ch) = entity(&name) {
                    // A control character (`&#0;`) is dropped, not kept.
                    if !ch.is_control() || ch == '\n' {
                        out.push(ch);
                    }
                    i = end + 1;
                    continue;
                }
            }
            out.push('&');
            i += 1;
            continue;
        }
        if c == '\r' {
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    // At most one blank line in a row; no trailing spaces.
    let mut lines: Vec<&str> = Vec::new();
    let mut blank = false;
    for line in out.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            if !blank && !lines.is_empty() {
                lines.push("");
            }
            blank = true;
        } else {
            lines.push(line);
            blank = false;
        }
    }
    lines.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_becomes_text_without_a_tag_surviving() {
        let html = "<p>Hello &amp; <b>welcome</b></p><ul><li>one</li><li>two &lt;3</li></ul>\
                    <script>alert('x')</script><STYLE>p{color:red}</STYLE>after<br>line<!-- hidden -->end";
        assert_eq!(
            html_text(html),
            "Hello & welcome\n- one\n- two <3\nafter\nline".to_string() + "end"
        );
    }

    #[test]
    fn entities_decode_once_and_escaped_tags_stay_text() {
        assert_eq!(
            html_text("&lt;script&gt;x&lt;/script&gt;"),
            "<script>x</script>"
        );
        // `&amp;lt;` is the text `&lt;`, not a second decode to `<`.
        assert_eq!(html_text("&amp;lt;b&amp;gt;"), "&lt;b&gt;");
        assert_eq!(
            html_text("a&nbsp;b &#39;c&#x41; &unknown; &"),
            "a b 'cA &unknown; &"
        );
        assert_eq!(html_text("&#0;&#1;ok"), "ok");
    }

    #[test]
    fn odd_html_never_panics_or_leaks_markup() {
        // Quoted `>` inside an attribute, an unterminated tag, a lone `<`.
        assert_eq!(html_text("<a href=\"x>y\">link</a>"), "link");
        assert_eq!(html_text("text <b unfinished"), "text");
        assert_eq!(html_text("1 < 2 and 3 > 2"), "1 < 2 and 3 > 2");
        assert_eq!(html_text("<script>never closed"), "");
        assert_eq!(html_text("<!-- never closed"), "");
        assert_eq!(html_text("<img src=x onerror=alert(1)>done"), "done");
        assert_eq!(html_text(""), "");
    }

    #[test]
    fn shard_names_are_gemini_activity_files_only() {
        assert!(is_shard("Takeout/My Activity/Gemini Apps/MyActivity.json"));
        assert!(is_shard("My Activity/Gemini Apps/MyActivity.json"));
        assert!(is_shard("Gemini Apps/myactivity.json"));
        assert!(is_shard("Takeout/My Activity/Bard/MyActivity.json"));
        assert!(!is_shard("Takeout/My Activity/Search/MyActivity.json"));
        assert!(!is_shard("Takeout/My Activity/Gemini Apps/MyActivity.html"));
        // The picked folder may be Gemini Apps itself: the header decides.
        assert!(is_shard("MyActivity.json"));
    }

    #[test]
    fn prompts_are_told_from_other_activity() {
        assert_eq!(prompt_of("Prompted hello"), Some("hello"));
        assert_eq!(prompt_of("Prompted\u{a0}hello"), Some("hello"));
        assert_eq!(prompt_of("Prompted"), Some(""));
        assert_eq!(prompt_of("Promptedly"), None);
        assert_eq!(prompt_of("Used Gemini Apps"), None);
    }
}
