//! A durable child task's own status line (repo ideas 2026-10-27 #7, after
//! openbot's teammate replies): the child task packet asks every child to end
//! its answer with one line, `Done`, `Partial: what is left` or `Blocked:
//! what would unblock it` (`delegation::CHILD_TASK_HEADER`), and this reads
//! it back, so the Main agent page and a parent's child-finished note can
//! say "Blocked: needs Alex's choice of X" without the transcript.
//!
//! Only the answer's last line counts (a trailing fenced block, such as the
//! composer's `hollow-next` suggestion, is skipped), optionally after
//! `Status:` and list or emphasis marks. A missing or unreadable line is
//! [`Outcome::Unknown`]: never done. Pure, so any surface that shows a
//! child (#1047's task cards included) can reuse [`parse`].

use serde::Serialize;

/// The longest detail kept after `Partial:` or `Blocked:` (characters).
pub const DETAIL_CHARS: usize = 200;

/// What a child said of its own work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", content = "detail", rename_all = "camelCase")]
pub enum Outcome {
    Done,
    Partial(String),
    Blocked(String),
    /// No status line: unknown, never done.
    Unknown,
}

impl Outcome {
    /// One line for a note: `Done`, `Partial: …`, `Blocked: …` or
    /// `no status line`.
    pub fn words(&self) -> String {
        let with = |word: &str, detail: &str| {
            if detail.is_empty() {
                word.to_string()
            } else {
                format!("{word}: {detail}")
            }
        };
        match self {
            Outcome::Done => "Done".into(),
            Outcome::Partial(d) => with("Partial", d),
            Outcome::Blocked(d) => with("Blocked", d),
            Outcome::Unknown => "no status line".into(),
        }
    }
}

/// The status line of `answer`, or [`Outcome::Unknown`]: its last line
/// outside fenced blocks, tracked from the top (a fence closes only with a
/// run of the same mark at least as long), so a trailing `hollow-next`
/// block is skipped and an answer that ends inside an unclosed fence has no
/// status (its end is code, not the child's word).
pub fn parse(answer: &str) -> Outcome {
    let mut fence: Option<(char, usize)> = None;
    let mut last: Option<&str> = None;
    for line in answer.lines() {
        let t = line.trim_start();
        let mark = t.chars().next().filter(|c| *c == '`' || *c == '~');
        let run = mark.map_or(0, |m| t.chars().take_while(|c| *c == m).count());
        match (fence, mark) {
            (None, Some(m)) if run >= 3 => fence = Some((m, run)),
            (Some((open, len)), Some(m))
                if m == open && run >= len && t[run..].trim().is_empty() =>
            {
                fence = None
            }
            (None, _) if !line.trim().is_empty() => last = Some(line),
            _ => {}
        }
    }
    match (fence, last) {
        (None, Some(line)) => read_line(line),
        _ => Outcome::Unknown,
    }
}

/// One line as a status, or Unknown.
fn read_line(line: &str) -> Outcome {
    let s = line.trim();
    // A quoted line is someone else's words, not the child's status.
    if s.starts_with('>') {
        return Outcome::Unknown;
    }
    // List marks before it; emphasis around it or its words.
    let s = s.trim_start_matches(['-', '•', ' ']).trim();
    let s = s.trim_start_matches("* ").trim();
    let s = s.replace("**", "").replace("__", "");
    let mut s = s.trim().trim_matches(['*', '_']).trim();
    if let Some(rest) = strip_word(s, "status") {
        s = rest.trim_start_matches([':', '-', '—', ' ']).trim();
    }
    for (word, make) in [
        ("done", (|_| Outcome::Done) as fn(String) -> Outcome),
        ("partial", Outcome::Partial),
        ("blocked", Outcome::Blocked),
    ] {
        let Some(rest) = strip_word(s, word) else {
            continue;
        };
        let rest = rest.trim_start();
        // "Done", "Done." or "Done: …"; prose that merely starts with the
        // word ("Done reviewing…") is not a status line.
        let detail = if rest.is_empty() || rest == "." {
            String::new()
        } else if let Some(after) = rest.strip_prefix([':', '—', '–', '-']) {
            after.trim().trim_end_matches('.').trim().to_string()
        } else {
            return Outcome::Unknown;
        };
        let detail: String = detail
            .chars()
            .filter(|c| !c.is_control())
            .take(DETAIL_CHARS)
            .collect();
        return make(detail);
    }
    Outcome::Unknown
}

/// `s` after `word` (any case) when it starts with it as a whole word.
fn strip_word<'a>(s: &'a str, word: &str) -> Option<&'a str> {
    let head = s.get(..word.len())?;
    if !head.eq_ignore_ascii_case(word) {
        return None;
    }
    let rest = &s[word.len()..];
    (!rest.chars().next().is_some_and(|c| c.is_alphanumeric())).then_some(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_lines_and_their_detail() {
        assert_eq!(parse("Merged #12.\n\nDone"), Outcome::Done);
        assert_eq!(parse("Merged.\nDone."), Outcome::Done);
        assert_eq!(parse("Status: Done"), Outcome::Done);
        assert_eq!(
            parse("Most of it.\nPartial: the docs are left."),
            Outcome::Partial("the docs are left".into())
        );
        assert_eq!(
            parse("**Status:** Blocked: needs Alex's choice of X"),
            Outcome::Blocked("needs Alex's choice of X".into())
        );
        assert_eq!(
            parse("- BLOCKED — the gate is red"),
            Outcome::Blocked("the gate is red".into())
        );
        assert_eq!(parse("Blocked"), Outcome::Blocked(String::new()));
        assert_eq!(
            Outcome::Blocked("needs X".into()).words(),
            "Blocked: needs X"
        );
        assert_eq!(Outcome::Unknown.words(), "no status line");
    }

    #[test]
    fn a_trailing_fenced_block_is_skipped() {
        let answer =
            "Fixed it.\nPartial: one test still flakes\n\n```hollow-next\nRun the gate\n```\n";
        assert_eq!(
            parse(answer),
            Outcome::Partial("one test still flakes".into())
        );
        assert_eq!(
            parse("Done\n```\nunclosed"),
            Outcome::Unknown,
            "an unclosed fence hides the end"
        );
        // Review of #1063: an unclosed sample ending in Done is code.
        assert_eq!(
            parse("Here is the log:\n```\nstep 1\nDone"),
            Outcome::Unknown
        );
        // A shorter run doesn't close a longer fence.
        assert_eq!(parse("Partial: x\n````\n```\nDone\n"), Outcome::Unknown);
        assert_eq!(
            parse("Partial: x\n````md\n```\ninner\n```\n````\n"),
            Outcome::Partial("x".into())
        );
        assert_eq!(
            parse("Blocked: y\r\n\r\n~~~\nDone\n~~~\r\n"),
            Outcome::Blocked("y".into())
        );
    }

    #[test]
    fn a_missing_or_prose_line_is_unknown_never_done() {
        for answer in [
            "",
            "   \n",
            "I merged it.",
            "Done reviewing the diff, two findings remain.",
            "Partially done, see above.",
            "The build is done",
            "Status: fine",
            "```hollow-next\nNext\n```",
        ] {
            assert_eq!(parse(answer), Outcome::Unknown, "{answer:?}");
        }
        // Only the last line counts.
        assert_eq!(parse("Done\nBut then I found a bug."), Outcome::Unknown);
        // A quote is someone else's word (review of #1063).
        assert_eq!(parse("Still blocked.\n> Done"), Outcome::Unknown);
        assert_eq!(parse("Tests pass.\n| a | b |"), Outcome::Unknown);
        // Emphasis and a hyphen separator are read.
        assert_eq!(parse("*Done*"), Outcome::Done);
        assert_eq!(
            parse("_Blocked: needs X_"),
            Outcome::Blocked("needs X".into())
        );
        assert_eq!(
            parse("Partial - docs remain"),
            Outcome::Partial("docs remain".into())
        );
    }

    #[test]
    fn details_are_bounded_and_plain() {
        let long = format!("Blocked: {}", "x".repeat(500));
        let Outcome::Blocked(d) = parse(&long) else {
            panic!()
        };
        assert_eq!(d.chars().count(), DETAIL_CHARS);
        assert_eq!(parse("Partial: a\u{7}b"), Outcome::Partial("ab".into()));
        assert_eq!(
            serde_json::to_value(Outcome::Blocked("needs X".into())).unwrap(),
            serde_json::json!({"status": "blocked", "detail": "needs X"})
        );
        assert_eq!(
            serde_json::to_value(Outcome::Unknown).unwrap(),
            serde_json::json!({"status": "unknown"})
        );
    }
}
