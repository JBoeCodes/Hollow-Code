//! Section-aware note chunker for the semantic (embedding) index.
//!
//! Pure + dependency-free so it's unit-tested in isolation (the same
//! discipline as `quick_add` / `trigger_evaluator`). The embedding index
//! (a later phase) feeds each note's frontmatter-stripped body through
//! `chunk_note` and embeds the resulting `Chunk.text`; `byte_start..byte_end`
//! index back into the SAME body string passed in, so a retrieved chunk can
//! cite an exact, verbatim span of the note.
//!
//! Strategy (from the embeddings design pass): split on markdown headings
//! and chat-mirror message terminators (`<!-- … -->`), then pack each
//! resulting section to a soft char budget with overlap, falling back to a
//! sliding window for over-long or heading-less sections. A one-line context
//! header (`title · kind · heading`) is prepended to each chunk's *embedded*
//! text so an isolated paragraph still carries who/what it's about — a known
//! recall win for bge-class models — WITHOUT polluting the cited byte span.
//!
//! The budget is **char-based** here on purpose: deterministic and
//! dependency-free, so this module never pulls in the model's tokenizer. At
//! ~4 chars/token for English, the default `max_chars` of 1000 (~250 tokens)
//! sits well under bge-small's 512-token window even after the context
//! header; the index layer owns the budget and can tighten it once the real
//! tokenizer is in hand.

/// One embeddable unit of a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// 0-based position of this chunk within the note.
    pub ord: usize,
    /// Nearest enclosing markdown heading (`None` = preamble / heading-less).
    pub heading: Option<String>,
    /// Byte span into the body passed to `chunk_note` — the citable text.
    /// Always lands on UTF-8 char boundaries. For a frontmatter chunk
    /// (`in_frontmatter`), a span of the whole note's frontmatter block
    /// instead, so its citation names a frontmatter line, never a body line.
    pub byte_start: usize,
    pub byte_end: usize,
    /// The text to embed: a context header line + the span's trimmed text.
    pub text: String,
    /// A chunk synthesized from the note's frontmatter fields by the index
    /// layer (`vault_embed_index`); `chunk_note` never makes one.
    pub in_frontmatter: bool,
}

/// Inputs that color the embedded text and bound chunk size.
pub struct ChunkOpts<'a> {
    /// Note title, for the context header (empty = omit).
    pub title: &'a str,
    /// Vault kind label, e.g. "task" / "email", for the context header.
    pub kind: &'a str,
    /// Soft max chunk size in chars; a section longer than this is windowed.
    pub max_chars: usize,
    /// Char overlap between adjacent windows of one long section.
    pub overlap_chars: usize,
}

impl Default for ChunkOpts<'_> {
    fn default() -> Self {
        ChunkOpts {
            title: "",
            kind: "",
            max_chars: 1000,
            overlap_chars: 150,
        }
    }
}

/// Split `body` (already frontmatter-stripped; HTML-stripped for `.html`
/// notes) into embeddable chunks. Empty/whitespace-only spans are skipped,
/// so an empty body yields no chunks.
pub fn chunk_note(body: &str, opts: &ChunkOpts) -> Vec<Chunk> {
    let mut out = Vec::new();
    for sec in split_sections(body) {
        let sec_text = &body[sec.start..sec.end];
        if sec_text.trim().is_empty() {
            continue;
        }
        for (s, e) in window_spans(sec_text, opts.max_chars, opts.overlap_chars) {
            let abs_start = sec.start + s;
            let abs_end = sec.start + e;
            let slice = &body[abs_start..abs_end];
            if slice.trim().is_empty() {
                continue;
            }
            out.push(Chunk {
                ord: out.len(),
                heading: sec.heading.clone(),
                byte_start: abs_start,
                byte_end: abs_end,
                text: compose_text(opts, sec.heading.as_deref(), slice),
                in_frontmatter: false,
            });
        }
    }
    out
}

/// A contiguous slice of the body sharing one heading context.
struct Section {
    heading: Option<String>,
    start: usize,
    end: usize,
}

/// Cut the body into sections. A markdown heading line STARTS a new section
/// (and the heading is carried on its content); a chat-mirror comment
/// terminator (`<!-- … -->` on its own line) ENDS the current section but
/// keeps the heading, so each message run in a day-grouped chat mirror
/// becomes its own section.
fn split_sections(body: &str) -> Vec<Section> {
    let mut sections = Vec::new();
    let mut heading: Option<String> = None;
    let mut start = 0usize;
    let mut pos = 0usize;

    for line in body.split_inclusive('\n') {
        let line_start = pos;
        pos += line.len();
        let trimmed = line.trim();
        if let Some(h) = heading_text(trimmed) {
            // Flush everything before this heading under the old heading.
            if line_start > start {
                sections.push(Section {
                    heading: heading.clone(),
                    start,
                    end: line_start,
                });
            }
            heading = Some(h);
            start = line_start;
        } else if is_comment_terminator(trimmed) {
            // Close the current run *after* the terminator; same heading.
            if pos > start {
                sections.push(Section {
                    heading: heading.clone(),
                    start,
                    end: pos,
                });
            }
            start = pos;
        }
    }
    if pos > start {
        sections.push(Section {
            heading: heading.clone(),
            start,
            end: pos,
        });
    }
    sections
}

/// `Some(text)` if `trimmed` is an ATX heading (`#`..`######` + space),
/// else `None`. A `#tag` with no following space is not a heading.
fn heading_text(trimmed: &str) -> Option<String> {
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&hashes) {
        let rest = &trimmed[hashes..]; // '#' is ASCII, so byte == char here
        if rest.is_empty() || rest.starts_with(' ') {
            return Some(rest.trim().to_string());
        }
    }
    None
}

/// A line that is solely an HTML comment, e.g. the `<!-- mid -->` message
/// terminators the Teams/Slack vault mirrors write.
fn is_comment_terminator(trimmed: &str) -> bool {
    trimmed.len() >= 7 && trimmed.starts_with("<!--") && trimmed.ends_with("-->")
}

/// Byte-span windows over `text`, each ≤ `max_chars` chars with `overlap`
/// chars shared between neighbours. Boundaries land on char starts (UTF-8
/// safe). A section that already fits is returned as a single span.
fn window_spans(text: &str, max_chars: usize, overlap: usize) -> Vec<(usize, usize)> {
    // Byte offset of every char start, plus the end — so [offs[a], offs[b])
    // is always a valid slice for char indices a ≤ b.
    let mut offs: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    offs.push(text.len());
    let n = offs.len() - 1; // char count
    let max_chars = max_chars.max(1);
    if n <= max_chars {
        return vec![(0, text.len())];
    }
    let step = max_chars.saturating_sub(overlap).max(1);
    let mut spans = Vec::new();
    let mut a = 0usize;
    loop {
        let b = (a + max_chars).min(n);
        spans.push((offs[a], offs[b]));
        if b >= n {
            break;
        }
        a += step;
    }
    spans
}

/// Prepend `title · kind · heading` to the (trimmed) span text so an
/// isolated chunk is self-describing. All parts are optional; an entirely
/// empty header yields just the trimmed text.
fn compose_text(opts: &ChunkOpts, heading: Option<&str>, slice: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if !opts.title.is_empty() {
        parts.push(opts.title);
    }
    if !opts.kind.is_empty() {
        parts.push(opts.kind);
    }
    let h = heading.map(str::trim).unwrap_or("");
    if !h.is_empty() {
        parts.push(h);
    }
    let body = slice.trim();
    if parts.is_empty() {
        body.to_string()
    } else {
        format!("{}\n\n{}", parts.join(" · "), body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> ChunkOpts<'static> {
        ChunkOpts {
            title: "Note",
            kind: "note",
            max_chars: 1000,
            overlap_chars: 150,
        }
    }

    #[test]
    fn splits_on_headings_with_preamble() {
        let body = "intro line\n\n## Alpha\nalpha body\n\n## Beta\nbeta body\n";
        let chunks = chunk_note(body, &opts());
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].heading, None);
        assert_eq!(chunks[1].heading.as_deref(), Some("Alpha"));
        assert_eq!(chunks[2].heading.as_deref(), Some("Beta"));
        assert_eq!(chunks[0].ord, 0);
        assert_eq!(chunks[2].ord, 2);
        // Span reconstructs the original text exactly.
        assert_eq!(
            &body[chunks[1].byte_start..chunks[1].byte_end],
            "## Alpha\nalpha body\n\n"
        );
        // Context header is prepended to embedded text but not the span.
        assert!(chunks[1].text.starts_with("Note · note · Alpha\n\n"));
        assert!(chunks[1].text.contains("alpha body"));
    }

    #[test]
    fn hashtag_without_space_is_not_a_heading() {
        let body = "see #urgent and #later for context";
        let chunks = chunk_note(body, &opts());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].heading, None);
    }

    #[test]
    fn windows_long_heading_less_body_with_overlap() {
        let body = "x".repeat(2500);
        let o = ChunkOpts {
            title: "",
            kind: "",
            max_chars: 1000,
            overlap_chars: 200,
        };
        let chunks = chunk_note(&body, &o);
        // step = 800 → windows start at 0, 800, 1600; the 1600 window already
        // reaches 2500 (≤ max_chars away) and stops → 3 windows.
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].byte_start, 0);
        assert_eq!(chunks[0].byte_end, 1000);
        // Overlap: window 1 starts 200 chars before window 0 ended.
        assert_eq!(chunks[1].byte_start, 800);
        // Last window reaches the end.
        assert_eq!(chunks.last().unwrap().byte_end, 2500);
        // No context header when title/kind/heading all empty.
        assert!(chunks[0].text.starts_with("xxxx"));
    }

    #[test]
    fn windows_respect_utf8_char_boundaries() {
        // Multi-byte chars: each "é" is 2 bytes, "你" is 3. A naive byte
        // window would split mid-codepoint and panic on slice.
        let body = "é你".repeat(800); // 1600 chars, 4000 bytes
        let o = ChunkOpts {
            title: "",
            kind: "",
            max_chars: 500,
            overlap_chars: 50,
        };
        let chunks = chunk_note(&body, &o);
        assert!(chunks.len() > 1);
        for c in &chunks {
            // Slicing on these offsets must not panic → boundaries are valid.
            let _ = &body[c.byte_start..c.byte_end];
            assert!(body.is_char_boundary(c.byte_start));
            assert!(body.is_char_boundary(c.byte_end));
        }
    }

    #[test]
    fn chat_mirror_terminators_split_message_runs() {
        let body = "## 2026-06-23\nfirst message\n<!-- m1 -->\nsecond message\n<!-- m2 -->\n";
        let chunks = chunk_note(body, &opts());
        // Two message runs under the same date heading.
        assert_eq!(chunks.len(), 2);
        assert!(chunks
            .iter()
            .all(|c| c.heading.as_deref() == Some("2026-06-23")));
        assert!(chunks[0].text.contains("first message"));
        assert!(chunks[1].text.contains("second message"));
    }

    #[test]
    fn empty_body_yields_no_chunks() {
        assert!(chunk_note("", &opts()).is_empty());
        assert!(chunk_note("   \n\n  \n", &opts()).is_empty());
    }

    #[test]
    fn heading_at_start_has_no_empty_preamble() {
        let body = "## Only\nbody text\n";
        let chunks = chunk_note(body, &opts());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].heading.as_deref(), Some("Only"));
        assert_eq!(chunks[0].byte_start, 0);
    }
}
