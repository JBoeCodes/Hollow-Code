//! The suggested next message (2026-10-01): every chat turn's appended prompt
//! asks the agent to end its reply with one fenced `hollow-next` block, one
//! to three lines, each a message the user is likely to send next. The
//! frontend hides the block and offers its first line as the empty
//! composer's placeholder (`src/shipActions.ts`); sending it is still the
//! user's keypress, so nothing here acts on its own.
//!
//! Text, not a tool, so it works for every chat provider the same way (the
//! agent already holds the request, the workspace brief and plan, and what
//! it just did). The stored transcript keeps the block, so a reopened
//! conversation still offers it; [`strip`] removes it from text that leaves
//! the chat (vault session records, learning extraction).
//!
//! The same convention hides an ask-canvas's answer (Canvas C4 A2): the
//! composer text a canvas drafts ends with a fenced [`CANVAS_ANSWER_LANG`]
//! block holding the exact answers as JSON. The person sends it; the agent's
//! prompt keeps the block, the chat hides it (`src/canvas/askAnswer.ts`), and
//! [`strip`] removes it here too.

/// The fence language of the suggestion block.
pub const FENCE_LANG: &str = "hollow-next";

/// The fence language of an ask-canvas's answers in a user message
/// (`ANSWER_FENCE` in `src/canvas/askAnswer.ts`).
pub const CANVAS_ANSWER_LANG: &str = "hollow-canvas-answer";

/// Every fence Hollow hides from text that leaves the chat.
const HIDDEN_LANGS: [&str; 2] = [FENCE_LANG, CANVAS_ANSWER_LANG];

/// Appended (byte-stable, path-free) to every chat turn's prompt.
pub const GUIDANCE: &str = "\
# Suggested next message (Hollow composer)

End every reply with one fenced code block whose language is hollow-next, \
holding one to three lines: the messages the user is most likely to send you \
next, most likely first, written the way they would type them (for example \
\"Merge and install\", \"Run the native checks on the installed build\", \
\"Draft the release note\"). Base them on everything you know: the request, \
the workspace brief and plan, what you just did, and what is still open or \
waiting on the user. Keep each line under 80 characters and specific to this \
work; never a generic \"Anything else?\" and never a question to the user. \
Hollow hides the block and shows the first line as a suggestion in the \
empty composer; nothing is sent until the user sends it. Leave the block out \
when you posted a question card or an ask-canvas (the user's answer is \
the next message), or there is no sensible next step.";

/// `text` without its `hollow-next` and `hollow-canvas-answer` blocks,
/// closed or left open at the end.
pub fn strip(text: &str) -> String {
    split(text, &HIDDEN_LANGS).0
}

/// `text` without its blocks in any of `langs`, and the lines inside each,
/// joined by `\n`: exactly `splitHiddenFence` (`src/chatSegments.ts`), held
/// to the same `src/canvas/hiddenFences.golden.json` cases (Canvas C6 P1;
/// before, a line like "```js code```" opened a fence here and not there,
/// so a session record kept an answer block the chat hid). A fence line is
/// optional spaces or tabs, three or more backticks or tildes, optional
/// spaces or tabs, an info string without backticks, optional spaces or tabs.
/// Only a bare hidden language activates stripping (case-insensitively);
/// ordinary info strings and attributed reserved fences are kept whole.
/// It closes on a line of at
/// least as many of the same character. Any other fence is kept whole, so a
/// block quoted inside one stays. Text naming none of `langs` is returned
/// as it is.
pub(crate) fn split(text: &str, langs: &[&str]) -> (String, Vec<String>) {
    if !langs.iter().any(|lang| text.contains(lang)) {
        return (text.to_string(), Vec::new());
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let mut kept: Vec<&str> = Vec::new();
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some((mark, len, info)) = fence_open(lines[i]) else {
            kept.push(lines[i]);
            i += 1;
            continue;
        };
        let start = i;
        while i + 1 < lines.len() && !fence_close(lines[i + 1], mark, len) {
            i += 1;
        }
        let body_end = i;
        if i + 1 < lines.len() {
            i += 1; // the closing fence
        }
        let info = info.to_lowercase();
        if langs.contains(&info.as_str()) {
            blocks.push(lines[start + 1..=body_end].join("\n"));
        } else {
            kept.extend_from_slice(&lines[start..=i]);
        }
        i += 1;
    }
    let mut joined = kept.join("\n");
    let end = joined.trim_end().len();
    joined.truncate(end);
    (joined, blocks)
}

fn blank(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// The fence character, its run length and its full info string. Backticks
/// in the info string are refused consistently with the frontend parser.
fn fence_open(line: &str) -> Option<(char, usize, &str)> {
    let rest = line.trim_start_matches(blank);
    let mark = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let len = rest.chars().take_while(|c| *c == mark).count();
    if len < 3 {
        return None;
    }
    let info = rest[len..].trim();
    (!info.contains('`')).then_some((mark, len, info))
}

/// `^[ \t]*<mark>{len,}[ \t]*$`.
fn fence_close(line: &str, mark: char, len: usize) -> bool {
    let rest = line.strip_suffix('\r').unwrap_or(line).trim_matches(blank);
    rest.chars().count() >= len && rest.chars().all(|c| c == mark)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_a_closed_block_and_the_blank_line_before_it() {
        let text =
            "Merged PR #12.\n\n```hollow-next\nBuild and install it\nRun the smoke suite\n```\n";
        assert_eq!(strip(text), "Merged PR #12.");
    }

    #[test]
    fn strips_an_unclosed_block_at_the_end() {
        assert_eq!(strip("Done.\n```hollow-next\nShip it"), "Done.");
    }

    #[test]
    fn keeps_other_fences_and_text_after_the_block() {
        let text = "a\n```rust\nlet x = 1;\n```\n~~~hollow-next\nNext\n~~~\nb";
        assert_eq!(strip(text), "a\n```rust\nlet x = 1;\n```\nb");
    }

    #[test]
    fn leaves_text_without_a_block_alone() {
        assert_eq!(strip("Plain reply\n"), "Plain reply\n");
        assert_eq!(
            strip("mentions hollow-next inline"),
            "mentions hollow-next inline"
        );
    }

    #[test]
    fn strips_an_ask_canvas_answer_block_from_a_user_message_too() {
        // Canvas C4 A2: the composer text an ask-canvas drafts. The agent's
        // prompt keeps the JSON; the vault session record shows the lines.
        let text = "Answered canvas \"release-choices\":\n- Ship day: Friday\n\n```hollow-canvas-answer\n{\"canvasId\":\"release-choices\",\"revision\":2,\"answers\":{\"ship-day\":\"Friday\"}}\n```";
        assert_eq!(
            strip(text),
            "Answered canvas \"release-choices\":\n- Ship day: Friday"
        );
        assert_eq!(strip("Hi\n```hollow-canvas-answer\n{\"canvasId\""), "Hi");
        // Both hidden blocks in one text, and a quoted one in another fence stays.
        assert_eq!(
            strip("a\n```hollow-next\nNext\n```\n```hollow-canvas-answer\n{}\n```\nb"),
            "a\nb"
        );
        let quoted = "See:\n````md\n```hollow-canvas-answer\n{}\n```\n````";
        assert_eq!(strip(quoted), quoted);
        assert_eq!(CANVAS_ANSWER_LANG, "hollow-canvas-answer");
    }

    /// The same golden cases `splitHiddenFence` passes (Canvas C6 P1, the
    /// C4 review minor): Rust and TS agree on what opens a hidden block, so
    /// the vault session record drops exactly what the chat bubble hides.
    #[test]
    fn every_golden_case_strips_and_reads_what_the_chat_does() {
        let golden: serde_json::Value =
            serde_json::from_str(include_str!("../../src/canvas/hiddenFences.golden.json"))
                .unwrap();
        let cases = golden["cases"].as_array().unwrap();
        assert!(cases.len() > 12);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let lang = case["lang"].as_str().unwrap();
            let text = case["text"].as_str().unwrap();
            let want_text = case["expect"]["text"].as_str().unwrap();
            let want_blocks: Vec<String> = case["expect"]["blocks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|lines| {
                    let lines: Vec<&str> = lines
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|l| l.as_str().unwrap())
                        .collect();
                    lines.join("\n")
                })
                .collect();
            assert!(HIDDEN_LANGS.contains(&lang), "{name}");
            assert_eq!(strip(text), want_text, "strip: {name}");
            assert_eq!(
                crate::canvas_answers::hidden_blocks(text, lang),
                want_blocks,
                "blocks: {name}"
            );
        }
    }

    #[test]
    fn guidance_names_the_fence() {
        assert!(GUIDANCE.contains(FENCE_LANG));
    }
}
