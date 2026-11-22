//! Save-time re-validation of a `hollow-canvas` fence (Canvas C3 foundations).
//!
//! The chat parses the fence in `src/canvas/fence.ts` for its skeleton; the
//! canvas store (a later C3 PR) calls [`validate`] before it writes anything,
//! so a body the frontend accepted is never trusted on its word. Both sides
//! implement the same rules and are tested against one case file,
//! `src/canvas/fence.golden.json` (limits, reason texts, valid, streaming and
//! malformed cases). Change a rule in both files and the golden together.
//!
//! The rules, in short (the TypeScript module doc and `docs/history/CANVAS_C3.md`
//! carry the full list): the body is at most 128 KB of UTF-8 and is exactly
//! one `<hc-canvas id title [ask] [height]>…</hc-canvas>` with only
//! HTML whitespace around it. `id` is 1–64 lowercase letters, digits or
//! hyphens ([`valid_canvas_id`]); `title` is 1–180 characters,
//! not blank, no control characters but tab and newline; `ask` is a bare
//! boolean; `height` is 80–1600 whole pixels. Unknown root attributes are
//! ignored; a repeated one is refused. Comments, quoted attribute values and
//! raw-text elements inside the root are skipped as the HTML tokenizer skips
//! them, and a nested `<hc-canvas` is refused.
//!
//! All syntax is ASCII, so the scanner walks bytes and only ever slices at
//! ASCII delimiters (always char boundaries).

use std::fmt;

pub const MAX_BODY_BYTES: usize = 128 * 1024;
pub const MAX_ROOT_TAG_BYTES: usize = 4096;
pub const MAX_ROOT_ATTRIBUTES: usize = 32;
pub const MAX_TITLE_CHARS: usize = 180;
pub const MIN_HEIGHT: u32 = 80;
pub const MAX_HEIGHT: u32 = 1600;
const ROOT: &str = "hc-canvas";

/// Why a fence body is not a canvas. [`CanvasError::code`] and
/// [`CanvasError::reason`] match the golden's `reasons` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasError {
    TooLarge,
    NoRoot,
    BadTag,
    DuplicateAttribute,
    BadId,
    BadTitle,
    BadAsk,
    BadHeight,
    Nested,
    Unclosed,
    TwoRoots,
    AfterRoot,
}

impl CanvasError {
    #[cfg(test)]
    pub const ALL: [CanvasError; 12] = [
        CanvasError::TooLarge,
        CanvasError::NoRoot,
        CanvasError::BadTag,
        CanvasError::DuplicateAttribute,
        CanvasError::BadId,
        CanvasError::BadTitle,
        CanvasError::BadAsk,
        CanvasError::BadHeight,
        CanvasError::Nested,
        CanvasError::Unclosed,
        CanvasError::TwoRoots,
        CanvasError::AfterRoot,
    ];

    #[cfg(test)]
    pub fn code(self) -> &'static str {
        match self {
            CanvasError::TooLarge => "too-large",
            CanvasError::NoRoot => "no-root",
            CanvasError::BadTag => "bad-tag",
            CanvasError::DuplicateAttribute => "duplicate-attribute",
            CanvasError::BadId => "bad-id",
            CanvasError::BadTitle => "bad-title",
            CanvasError::BadAsk => "bad-ask",
            CanvasError::BadHeight => "bad-height",
            CanvasError::Nested => "nested",
            CanvasError::Unclosed => "unclosed",
            CanvasError::TwoRoots => "two-roots",
            CanvasError::AfterRoot => "after-root",
        }
    }

    pub fn reason(self) -> &'static str {
        match self {
            CanvasError::TooLarge => "The canvas is larger than 128 KB.",
            CanvasError::NoRoot => "A canvas must start with <hc-canvas id=\"…\" title=\"…\">.",
            CanvasError::BadTag => "The <hc-canvas> tag is malformed.",
            CanvasError::DuplicateAttribute => "An <hc-canvas> attribute appears twice.",
            CanvasError::BadId => {
                "The canvas id must be 1–64 lowercase letters, digits or hyphens."
            }
            CanvasError::BadTitle => "The canvas needs a title of 1–180 characters of plain text.",
            CanvasError::BadAsk => "ask takes no value: write <hc-canvas ask …>.",
            CanvasError::BadHeight => "height must be whole pixels from 80 to 1600.",
            CanvasError::Nested => "A canvas cannot contain another <hc-canvas>.",
            CanvasError::Unclosed => "The canvas ended before </hc-canvas>.",
            CanvasError::TwoRoots => "A fence holds one canvas; this one has two.",
            CanvasError::AfterRoot => "Nothing may follow </hc-canvas> in the fence.",
        }
    }
}

impl fmt::Display for CanvasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason())
    }
}

/// The root element's attributes the host reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanvasRoot {
    pub id: String,
    pub title: String,
    pub ask: bool,
    pub height: Option<u32>,
}

/// A fence body's state, as `parseCanvasFence` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanvasParse<'a> {
    /// The root start tag hasn't closed yet.
    Pending,
    /// The start tag closed and the fence is still streaming.
    Drawing(CanvasRoot),
    /// The fence ended and the canvas is well formed. `inner` is the HTML
    /// between the root's start and end tags.
    Ready {
        root: CanvasRoot,
        inner: &'a str,
    },
    Invalid(CanvasError),
}

/// A complete, valid canvas: what the store may save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidCanvas<'a> {
    pub root: CanvasRoot,
    pub inner: &'a str,
}

/// A canvas id: 1–64 lowercase ASCII letters, digits or hyphens. It was the
/// whiteboards' id rule; it outlived them (2026-10-07). The TypeScript copy
/// is `CANVAS_ID` in `src/canvas/fence.ts`.
pub(crate) fn valid_canvas_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Re-validate a finished fence body before saving it.
pub fn validate(body: &str) -> Result<ValidCanvas<'_>, CanvasError> {
    match parse(body, true) {
        CanvasParse::Ready { root, inner } => Ok(ValidCanvas { root, inner }),
        CanvasParse::Invalid(e) => Err(e),
        // A closed fence is never pending or drawing.
        CanvasParse::Pending | CanvasParse::Drawing(_) => Err(CanvasError::Unclosed),
    }
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0c | b'\r')
}

/// While streaming, drop a last line that may be the closing fence arriving,
/// with the newline before it: one made only of backticks or tildes, or still
/// empty or only spaces and tabs (the newline after the root has arrived, the
/// fence characters not yet). What is left reads as the body will once the
/// fence closes (the size limit applies to it).
fn without_typing_fence(body: &str) -> &str {
    let nl = body.rfind('\n');
    let start = nl.map_or(0, |i| i + 1);
    let last = body[start..].trim_start_matches([' ', '\t']);
    // An empty `last` is the fence line just starting: `all` is true for it.
    let fence = last.bytes().all(|b| b == b'`') || last.bytes().all(|b| b == b'~');
    if fence {
        &body[..nl.unwrap_or(0)]
    } else {
        body
    }
}

fn is_control(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{8}' | '\u{b}'..='\u{1f}' | '\u{7f}')
}

/// Whitespace for the "not blank" rule; the same list as `fence.ts`.
fn is_blank_char(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{85}' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// Decode character references in a known attribute's value: the five XML
/// names and numeric references (≤7 decimal or ≤6 hex digits, with `;`).
/// Other `&name;` stays literal. `None` when a numeric reference is malformed.
pub fn decode_attribute(v: &str) -> Option<String> {
    if !v.contains('&') {
        return Some(v.to_string());
    }
    let mut out = String::with_capacity(v.len());
    let mut rest = v;
    while let Some(a) = rest.find('&') {
        out.push_str(&rest[..a]);
        let tail = &rest[a..];
        if let Some(num) = tail.strip_prefix("&#") {
            let (hex, digits) = match num.strip_prefix(['x', 'X']) {
                Some(h) => (true, h),
                None => (false, num),
            };
            let len = digits
                .bytes()
                .take_while(|b| {
                    if hex {
                        b.is_ascii_hexdigit()
                    } else {
                        b.is_ascii_digit()
                    }
                })
                .count();
            let max = if hex { 6 } else { 7 };
            if len == 0 || len > max || digits.as_bytes().get(len) != Some(&b';') {
                return None;
            }
            let cp = u32::from_str_radix(&digits[..len], if hex { 16 } else { 10 }).ok()?;
            if cp == 0 {
                return None;
            }
            out.push(char::from_u32(cp)?);
            let used = 2 + usize::from(hex) + len + 1;
            rest = &tail[used..];
            continue;
        }
        let named = [
            ("&amp;", '&'),
            ("&lt;", '<'),
            ("&gt;", '>'),
            ("&quot;", '"'),
            ("&apos;", '\''),
        ]
        .into_iter()
        .find(|(name, _)| tail.starts_with(name));
        match named {
            Some((name, ch)) => {
                out.push(ch);
                rest = &tail[name.len()..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    Some(out)
}

enum TagScan {
    Incomplete,
    Bad,
    Ok {
        attrs: Vec<(String, String)>,
        end: usize,
    },
}

/// Strict scan of the root start tag's attributes, from just after `<hc-canvas`.
fn scan_root_tag(t: &str, from: usize) -> TagScan {
    let b = t.as_bytes();
    let n = b.len();
    let mut attrs = Vec::new();
    let mut k = from;
    loop {
        while k < n && is_ws(b[k]) {
            k += 1;
        }
        if k >= n {
            return TagScan::Incomplete;
        }
        match b[k] {
            b'>' => return TagScan::Ok { attrs, end: k + 1 },
            b'/' | b'=' | b'"' | b'\'' | b'<' => return TagScan::Bad,
            _ => {}
        }
        let start = k;
        while k < n && !is_ws(b[k]) && !matches!(b[k], b'/' | b'>' | b'=') {
            if matches!(b[k], b'"' | b'\'' | b'<') {
                return TagScan::Bad;
            }
            k += 1;
        }
        if k >= n {
            return TagScan::Incomplete;
        }
        let name = t[start..k].to_ascii_lowercase();
        let mut value = String::new();
        let mut j = k;
        while j < n && is_ws(b[j]) {
            j += 1;
        }
        if j >= n {
            return TagScan::Incomplete;
        }
        if b[j] == b'=' {
            j += 1;
            while j < n && is_ws(b[j]) {
                j += 1;
            }
            if j >= n {
                return TagScan::Incomplete;
            }
            let q = b[j];
            if q == b'"' || q == b'\'' {
                let Some(e) = t[j + 1..].find(q as char).map(|e| e + j + 1) else {
                    return TagScan::Incomplete;
                };
                value = t[j + 1..e].to_string();
                k = e + 1;
                if k < n && !is_ws(b[k]) && b[k] != b'>' {
                    return TagScan::Bad;
                }
            } else if q == b'>' {
                return TagScan::Bad;
            } else {
                let s = j;
                while j < n && !is_ws(b[j]) && b[j] != b'>' {
                    if matches!(b[j], b'"' | b'\'' | b'<' | b'=' | b'`') {
                        return TagScan::Bad;
                    }
                    j += 1;
                }
                if j >= n {
                    return TagScan::Incomplete;
                }
                value = t[s..j].to_string();
                k = j;
            }
        }
        attrs.push((name, value));
        if attrs.len() > MAX_ROOT_ATTRIBUTES {
            return TagScan::Bad;
        }
    }
}

fn root_from(attrs: &[(String, String)]) -> Result<CanvasRoot, CanvasError> {
    for (i, (name, _)) in attrs.iter().enumerate() {
        if attrs[..i].iter().any(|(seen, _)| seen == name) {
            return Err(CanvasError::DuplicateAttribute);
        }
    }
    let get = |name: &str| {
        attrs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    };
    let id = get("id")
        .and_then(decode_attribute)
        .filter(|id| valid_canvas_id(id))
        .ok_or(CanvasError::BadId)?;
    let title = get("title")
        .and_then(decode_attribute)
        .filter(|t| {
            !t.chars().all(is_blank_char)
                && t.chars().count() <= MAX_TITLE_CHARS
                && !t.chars().any(is_control)
        })
        .ok_or(CanvasError::BadTitle)?;
    let ask = match get("ask") {
        None => false,
        Some(raw) => match decode_attribute(raw) {
            Some(v) if v.is_empty() || v.eq_ignore_ascii_case("ask") => true,
            _ => return Err(CanvasError::BadAsk),
        },
    };
    let height = match get("height") {
        None => None,
        Some(raw) => {
            let v = decode_attribute(raw).ok_or(CanvasError::BadHeight)?;
            let digits = v.strip_suffix("px").unwrap_or(&v);
            let px = (1..=5)
                .contains(&digits.len())
                .then_some(digits)
                .filter(|d| d.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|d| d.parse::<u32>().ok())
                .filter(|px| (MIN_HEIGHT..=MAX_HEIGHT).contains(px))
                .ok_or(CanvasError::BadHeight)?;
            Some(px)
        }
    };
    Ok(CanvasRoot {
        id,
        title,
        ask,
        height,
    })
}

/// Read a tag name from `from` (ASCII-lowered) up to whitespace, `/`, `>` or the end.
fn tag_name(t: &str, from: usize) -> (String, usize) {
    let b = t.as_bytes();
    let mut k = from;
    while k < b.len() && !is_ws(b[k]) && b[k] != b'/' && b[k] != b'>' {
        k += 1;
    }
    (t[from..k].to_ascii_lowercase(), k)
}

fn find_from(t: &str, from: usize, pat: &str) -> Option<usize> {
    t[from..].find(pat).map(|i| i + from)
}

/// Skip the rest of a non-root start tag, honouring quoted attribute values.
fn skip_tag(t: &str, from: usize) -> Option<usize> {
    let b = t.as_bytes();
    let n = b.len();
    let mut k = from;
    while k < n {
        let ch = b[k];
        if ch == b'>' {
            return Some(k + 1);
        }
        k += 1;
        if ch == b'=' {
            while k < n && is_ws(b[k]) {
                k += 1;
            }
            if k < n && (b[k] == b'"' || b[k] == b'\'') {
                let q = if b[k] == b'"' { "\"" } else { "'" };
                k = find_from(t, k + 1, q)? + 1;
            }
        }
    }
    None
}

const RAW_TEXT: [&str; 9] = [
    "script", "style", "textarea", "title", "xmp", "iframe", "noembed", "noframes", "noscript",
];

/// Index of the `</name` that ends a raw-text element, or `None` if it hasn't arrived.
fn raw_text_end(t: &str, from: usize, name: &str) -> Option<usize> {
    let b = t.as_bytes();
    let mut k = from;
    loop {
        let at = find_from(t, k, "</")?;
        let after = at + 2 + name.len();
        if after >= b.len() {
            return None;
        }
        if b[at + 2..after].eq_ignore_ascii_case(name.as_bytes())
            && (is_ws(b[after]) || b[after] == b'/' || b[after] == b'>')
        {
            return Some(at);
        }
        k = at + 2;
    }
}

enum ContentScan {
    Incomplete,
    Error(CanvasError),
    Closed { inner_end: usize, after: usize },
}

/// Find the root's `</hc-canvas>`, from the end of its start tag.
fn scan_content(t: &str, from: usize) -> ContentScan {
    let b = t.as_bytes();
    let n = b.len();
    let mut k = from;
    loop {
        let Some(lt) = find_from(t, k, "<") else {
            return ContentScan::Incomplete;
        };
        if lt + 1 >= n {
            return ContentScan::Incomplete;
        }
        if t[lt..].starts_with("<!--") {
            match find_from(t, lt + 4, "-->") {
                Some(e) => {
                    k = e + 3;
                    continue;
                }
                None => return ContentScan::Incomplete,
            }
        }
        let c = b[lt + 1];
        if c == b'!' || c == b'?' {
            match find_from(t, lt + 2, ">") {
                Some(e) => {
                    k = e + 1;
                    continue;
                }
                None => return ContentScan::Incomplete,
            }
        }
        if c == b'/' {
            let Some(&d) = b.get(lt + 2) else {
                return ContentScan::Incomplete;
            };
            if d.is_ascii_alphabetic() {
                let (name, p) = tag_name(t, lt + 2);
                if p >= n {
                    return ContentScan::Incomplete;
                }
                if name == ROOT {
                    let mut q = p;
                    while q < n && is_ws(b[q]) {
                        q += 1;
                    }
                    if q >= n {
                        return ContentScan::Incomplete;
                    }
                    if b[q] != b'>' {
                        return ContentScan::Error(CanvasError::BadTag);
                    }
                    return ContentScan::Closed {
                        inner_end: lt,
                        after: q + 1,
                    };
                }
            }
            // Any other end tag, `</>` or a bogus comment: skip to its `>`.
            match find_from(t, lt + 2, ">") {
                Some(e) => {
                    k = e + 1;
                    continue;
                }
                None => return ContentScan::Incomplete,
            }
        }
        if c.is_ascii_alphabetic() {
            let (name, p) = tag_name(t, lt + 1);
            if p >= n {
                return ContentScan::Incomplete;
            }
            if name == ROOT {
                return ContentScan::Error(CanvasError::Nested);
            }
            let Some(end) = skip_tag(t, p) else {
                return ContentScan::Incomplete;
            };
            if name == "plaintext" {
                return ContentScan::Incomplete;
            }
            if RAW_TEXT.contains(&name.as_str()) {
                match raw_text_end(t, end, &name) {
                    Some(close) => {
                        k = close;
                        continue;
                    }
                    None => return ContentScan::Incomplete,
                }
            }
            k = end;
            continue;
        }
        k = lt + 1;
    }
}

enum RootOpen {
    Yes,
    Partial,
    No,
}

/// Does `t` at `at` start with `<hc-canvas` (ASCII case-insensitive)?
fn root_open_at(t: &str, at: usize) -> RootOpen {
    let b = t.as_bytes();
    for (i, want) in "<hc-canvas".bytes().enumerate() {
        match b.get(at + i) {
            None => return RootOpen::Partial,
            Some(got) if got.to_ascii_lowercase() == want => {}
            Some(_) => return RootOpen::No,
        }
    }
    RootOpen::Yes
}

/// The root's `texture` attribute, entities decoded, for a frame body
/// (`canvasFrameBody` in `src/canvas/fence.ts`; the phone's canvases,
/// v1.2). `None` without one, or for a body whose root tag doesn't scan.
pub(crate) fn root_texture(body: &str) -> Option<String> {
    let b = body.as_bytes();
    let mut i = 0;
    while i < b.len() && is_ws(b[i]) {
        i += 1;
    }
    if !matches!(root_open_at(body, i), RootOpen::Yes) {
        return None;
    }
    match scan_root_tag(body, i + 1 + ROOT.len()) {
        TagScan::Ok { attrs, .. } => attrs
            .iter()
            .find(|(name, _)| name == "texture")
            .and_then(|(_, value)| decode_attribute(value)),
        _ => None,
    }
}

/// Classify a fence body. `fence_closed` is whether the closing fence line
/// has arrived. Mirrors `parseCanvasFence` in `src/canvas/fence.ts`.
pub fn parse(body: &str, fence_closed: bool) -> CanvasParse<'_> {
    use CanvasParse::{Drawing, Invalid, Pending, Ready};
    // Measured after the typing-fence strip: the closing fence line is not
    // part of the body.
    let t = if fence_closed {
        body
    } else {
        without_typing_fence(body)
    };
    if t.len() > MAX_BODY_BYTES {
        return Invalid(CanvasError::TooLarge);
    }
    let b = t.as_bytes();
    let n = b.len();
    let unfinished = |e: CanvasError| if fence_closed { Invalid(e) } else { Pending };
    let mut i = 0;
    while i < n && is_ws(b[i]) {
        i += 1;
    }
    if i >= n {
        return unfinished(CanvasError::NoRoot);
    }
    match root_open_at(t, i) {
        RootOpen::No => return Invalid(CanvasError::NoRoot),
        RootOpen::Partial => return unfinished(CanvasError::NoRoot),
        RootOpen::Yes => {}
    }
    let p = i + 1 + ROOT.len();
    if p >= n {
        return unfinished(CanvasError::Unclosed);
    }
    if b[p] == b'/' {
        return Invalid(CanvasError::BadTag);
    }
    if !is_ws(b[p]) && b[p] != b'>' {
        return Invalid(CanvasError::NoRoot);
    }

    let tag = scan_root_tag(t, p);
    let tag_end = match &tag {
        TagScan::Bad => return Invalid(CanvasError::BadTag),
        TagScan::Ok { end, .. } => *end,
        TagScan::Incomplete => n,
    };
    if tag_end - i > MAX_ROOT_TAG_BYTES {
        return Invalid(CanvasError::BadTag);
    }
    let TagScan::Ok { attrs, end } = tag else {
        return unfinished(CanvasError::Unclosed);
    };
    let root = match root_from(&attrs) {
        Ok(root) => root,
        Err(e) => return Invalid(e),
    };

    match scan_content(t, end) {
        ContentScan::Error(e) => Invalid(e),
        ContentScan::Incomplete => {
            if fence_closed {
                Invalid(CanvasError::Unclosed)
            } else {
                Drawing(root)
            }
        }
        ContentScan::Closed { inner_end, after } => {
            let mut k = after;
            while k < n && is_ws(b[k]) {
                k += 1;
            }
            if k < n {
                let next = b.get(k + 1 + ROOT.len());
                let two_roots = matches!(root_open_at(t, k), RootOpen::Yes)
                    && next.is_none_or(|&c| is_ws(c) || c == b'>' || c == b'/');
                return Invalid(if two_roots {
                    CanvasError::TwoRoots
                } else {
                    CanvasError::AfterRoot
                });
            }
            if !fence_closed {
                return Drawing(root);
            }
            Ready {
                root,
                inner: &t[end..inner_end],
            }
        }
    }
}

/// The `hollow-canvas` fences in one assistant reply, for the turn's
/// receipt (Canvas C3 wave 2 C4): how many and how many UTF-8 bytes their
/// bodies hold, valid or not, so a reply's canvas cost is visible.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReplyCanvases {
    pub count: u32,
    pub bytes: u64,
}

/// A fence opening line as `src/chatSegments.ts` reads it: optional spaces
/// or tabs, three or more backticks or tildes, then an optional language
/// (`[A-Za-z0-9_.+#-]`, lower-cased), nothing else. Returns the fence
/// character, its run length and the language. Also `cite_check::extract`'s
/// fence skip (repo features R5).
pub(crate) fn fence_open(line: &str) -> Option<(u8, usize, String)> {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    let ch = *b.get(i)?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let run = b[i..].iter().take_while(|&&c| c == ch).count();
    if run < 3 {
        return None;
    }
    i += run;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    let start = i;
    while i < b.len() && (b[i].is_ascii_alphanumeric() || b"_.+#-".contains(&b[i])) {
        i += 1;
    }
    let lang = line[start..i].to_ascii_lowercase();
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    (i == b.len()).then_some((ch, run, lang))
}

/// Whether `line` closes a fence of `run` × `ch`: optional spaces or tabs,
/// at least `run` of `ch`, optional spaces or tabs.
pub(crate) fn fence_close(line: &str, ch: u8, run: usize) -> bool {
    let t = line.trim_matches(|c| c == ' ' || c == '\t');
    t.len() >= run && t.bytes().all(|c| c == ch)
}

/// One canvas fence in a reply: its body (the lines between the fence
/// lines, joined by the newlines between them) and whether its closing
/// fence line arrived.
struct ReplyFence<'a> {
    lines: &'a [&'a str],
    closed: bool,
}

impl ReplyFence<'_> {
    fn bytes(&self) -> u64 {
        self.lines.iter().map(|l| l.len() as u64).sum::<u64>()
            + self.lines.len().saturating_sub(1) as u64
    }
}

/// Every canvas fence in `text`, in order, exactly as `parseSegments`
/// splits a reply (`src/canvas/replyCanvases.golden.json` pins the two
/// together): a fence left open runs to the end, and an empty open fence at
/// the very end whose language is still being typed (`hollow-canv`) is a
/// canvas too.
fn reply_fences<'a>(lines: &'a [&'a str]) -> Vec<ReplyFence<'a>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some((ch, run, lang)) = fence_open(lines[i]) else {
            i += 1;
            continue;
        };
        let mut j = i + 1;
        let mut closed = false;
        while j < lines.len() {
            if fence_close(lines[j], ch, run) {
                closed = true;
                break;
            }
            j += 1;
        }
        let typing = !closed
            && j == i + 1
            && lang.starts_with("hollow-c")
            && CANVAS_FENCE.starts_with(lang.as_str());
        if lang == CANVAS_FENCE || typing {
            out.push(ReplyFence {
                lines: &lines[i + 1..j],
                closed,
            });
        }
        i = if closed { j + 1 } else { j };
    }
    out
}

/// Count the canvas fences in `text` and the UTF-8 bytes their bodies hold,
/// valid or not ([`reply_fences`]).
pub fn reply_canvases(text: &str) -> ReplyCanvases {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = ReplyCanvases::default();
    for fence in reply_fences(&lines) {
        out.count += 1;
        out.bytes += fence.bytes();
    }
    out
}

/// The bodies of the reply's closed canvas fences, in order, byte for byte
/// as the transcript's `ChatCanvas` receives them before its stray-tag
/// repair (`written`). The turn's end saves these (question cards,
/// 2026-10-14), so an ask-canvas no window watched stream is still saved.
pub fn reply_canvas_bodies(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.split('\n').collect();
    reply_fences(&lines)
        .into_iter()
        .filter(|f| f.closed)
        .map(|f| f.lines.join("\n"))
        .collect()
}

/// The canvas fence language.
pub const CANVAS_FENCE: &str = "hollow-canvas";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const GOLDEN: &str = include_str!("../../src/canvas/fence.golden.json");
    const REPLY_GOLDEN: &str = include_str!("../../src/canvas/replyCanvases.golden.json");

    /// The receipt's canvas count and bytes per reply match the chat's own
    /// segmenter (`parseSegments`, pinned by the same golden in Vitest).
    #[test]
    fn reply_canvases_match_the_golden() {
        let golden: Value = serde_json::from_str(REPLY_GOLDEN).expect("golden parses");
        let cases = golden["cases"].as_array().unwrap();
        assert!(cases.len() >= 10);
        for case in cases {
            let text = case["text"].as_str().unwrap();
            let got = reply_canvases(text);
            assert_eq!(
                (u64::from(got.count), got.bytes),
                (
                    case["count"].as_u64().unwrap(),
                    case["bytes"].as_u64().unwrap()
                ),
                "{}",
                case["name"]
            );
        }
    }

    fn golden() -> Value {
        serde_json::from_str(GOLDEN).expect("fence.golden.json parses")
    }

    fn body_of(v: &Value) -> String {
        match v {
            Value::String(s) => s.clone(),
            Value::Object(o) => {
                let s = |k: &str| o[k].as_str().unwrap().to_string();
                s("prefix")
                    + &s("fill").repeat(o["times"].as_u64().unwrap() as usize)
                    + &s("suffix")
            }
            other => panic!("bad body {other}"),
        }
    }

    /// The parse result in the golden's flat shape (`inner` only when asked).
    fn flat(p: &CanvasParse<'_>, with_inner: bool) -> Value {
        let root = |state: &str, r: &CanvasRoot| {
            serde_json::json!({
                "state": state, "id": r.id, "title": r.title, "ask": r.ask, "height": r.height,
            })
        };
        match p {
            CanvasParse::Pending => serde_json::json!({ "state": "pending" }),
            CanvasParse::Invalid(e) => serde_json::json!({ "state": "invalid", "code": e.code() }),
            CanvasParse::Drawing(r) => root("drawing", r),
            CanvasParse::Ready { root: r, inner } => {
                let mut v = root("ready", r);
                if with_inner {
                    v["inner"] = Value::String((*inner).to_string());
                }
                v
            }
        }
    }

    #[test]
    fn limits_and_reasons_match_the_golden() {
        let g = golden();
        let l = &g["limits"];
        assert_eq!(l["bodyBytes"], MAX_BODY_BYTES);
        assert_eq!(l["rootTagBytes"], MAX_ROOT_TAG_BYTES);
        assert_eq!(l["rootAttributes"], MAX_ROOT_ATTRIBUTES);
        assert_eq!(l["titleChars"], MAX_TITLE_CHARS);
        assert_eq!(l["minHeight"], MIN_HEIGHT);
        assert_eq!(l["maxHeight"], MAX_HEIGHT);
        let reasons = g["reasons"].as_object().unwrap();
        assert_eq!(reasons.len(), CanvasError::ALL.len());
        for e in CanvasError::ALL {
            assert_eq!(reasons[e.code()], e.reason(), "{}", e.code());
        }
    }

    #[test]
    fn every_golden_case_parses_as_pinned() {
        let g = golden();
        let cases = g["cases"].as_array().unwrap();
        assert!(cases.len() > 100, "the golden lost its cases");
        let mut failures = Vec::new();
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let body = body_of(&case["body"]);
            let closed = case["closed"].as_bool().unwrap();
            let want = &case["expect"];
            let got = flat(&parse(&body, closed), want.get("inner").is_some());
            if &got != want {
                failures.push(format!("{name}: want {want}, got {got}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn a_well_formed_canvas_never_looks_broken_while_it_streams() {
        let g = golden();
        for case in g["cases"].as_array().unwrap() {
            let (Some(body), "ready") = (
                case["body"].as_str(),
                case["expect"]["state"].as_str().unwrap(),
            ) else {
                continue;
            };
            let name = case["name"].as_str().unwrap();
            let mut drawing = false;
            let cuts = body.char_indices().map(|(i, _)| i).chain([body.len()]);
            for n in cuts {
                match parse(&body[..n], false) {
                    CanvasParse::Pending => assert!(!drawing, "{name} went back to pending at {n}"),
                    CanvasParse::Drawing(root) => {
                        drawing = true;
                        assert_eq!(
                            root.id,
                            case["expect"]["id"].as_str().unwrap(),
                            "{name} at {n}"
                        );
                        assert_eq!(
                            root.title,
                            case["expect"]["title"].as_str().unwrap(),
                            "{name} at {n}"
                        );
                    }
                    other => panic!("{name} at {n}: {other:?}"),
                }
            }
            assert!(drawing, "{name}");
        }
    }

    #[test]
    fn validate_is_the_closed_fence_parse() {
        let ok = validate("<hc-canvas id=\"a\" title=\"A\" ask><p>x</p></hc-canvas>\n").unwrap();
        assert_eq!(ok.root.id, "a");
        assert!(ok.root.ask);
        assert_eq!(ok.inner, "<p>x</p>");
        assert_eq!(
            validate("<hc-canvas id=\"a\" title=\"A\"><p>").unwrap_err(),
            CanvasError::Unclosed
        );
        assert_eq!(
            validate("<script></script><hc-canvas id=\"a\" title=\"A\"></hc-canvas>")
                .unwrap_err()
                .to_string(),
            CanvasError::NoRoot.reason()
        );
    }

    #[test]
    fn decodes_only_the_documented_references() {
        assert_eq!(decode_attribute("a &amp; b").as_deref(), Some("a & b"));
        assert_eq!(
            decode_attribute("&nbsp;&copy;&").as_deref(),
            Some("&nbsp;&copy;&")
        );
        assert_eq!(
            decode_attribute("&#x1F680;&#128640;").as_deref(),
            Some("🚀🚀")
        );
        assert_eq!(decode_attribute("&#;"), None);
        assert_eq!(decode_attribute("&#x;"), None);
    }
}
