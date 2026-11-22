//! Program-checked quotes on records (EV1, 2026-11-16).
//!
//! A decision or a person fact ends `Evidence: “quote”` and its anchor, and the
//! anchor is a link back to where it was said. Extraction already drops a
//! proposal whose quote isn't in the segment it cites (`extract`), but nothing
//! looked again when the person **accepted** it: a transcript can be re-synced
//! or a turn change between the extraction and the click. So [`check`] runs at
//! accept time (`records::accept_proposal`, before anything is claimed or
//! written): for an anchor Hollow can read, the quote must appear verbatim in
//! the text there (whitespace and case folded), else the accept is refused with
//! the nearest real line shown and the item stays pending (the card can't edit
//! a quote or an anchor, so the way on is to reject it and have it proposed
//! again from the source). This catches drift since extraction. It does not
//! cover an agent's `records::propose_decision`, which carries one free-text
//! string as both quote and anchor, so there is nothing separate to check.
//!
//! Anchors it reads: `<session>#turn<N>` (a conversation turn, with or
//! without a `chat:` prefix: the user message and the agent's reply of turn N)
//! and `<note>.md@<MM:SS>` (the transcript line of a kept recording). Any other
//! anchor (a file path, `plaud:<id>@…`, an agent's free text), a conversation
//! or transcript that can't be read now, and a quote that is its own anchor are
//! not checked: they pass, and [`Quote::Unchecked`] says why. A quote that
//! can't be checked is not a quote that was checked.

use crate::extract::{self, Source};
use std::path::Path;

/// What [`check`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Quote {
    /// The quote is in the text at the anchor.
    Verified,
    /// Not checked, with the reason; the accept goes on.
    Unchecked(&'static str),
    /// The anchor was read and the quote isn't in it: the nearest real line
    /// there, when any shares a word with the quote.
    Miss { nearest: Option<String> },
}

/// An anchor Hollow can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Anchor {
    /// Turn `turn` (1-based, user messages counted) of conversation `key`.
    Turn { key: String, turn: usize },
    /// The transcript line starting at `at` in the vault note `path`.
    Transcript { path: String, at: String },
}

/// The anchor `text` names, or `None` for anything Hollow can't read.
pub(crate) fn parse_anchor(text: &str) -> Option<Anchor> {
    let text = text.trim();
    let bare = text.strip_prefix("chat:").unwrap_or(text);
    if let Some((key, turn)) = bare.rsplit_once("#turn") {
        let turn: usize = turn.parse().ok().filter(|n| *n >= 1)?;
        return crate::agent_sessions::is_safe_session_key(key).then(|| Anchor::Turn {
            key: key.to_string(),
            turn,
        });
    }
    let (path, at) = text.rsplit_once('@')?;
    let path = path.trim().trim_start_matches("vault:");
    let at = at.trim();
    let timed =
        !at.is_empty() && at.chars().all(|c| c.is_ascii_digit() || c == ':') && at.contains(':');
    (path.ends_with(".md") && !path.starts_with('/') && !path.contains("..") && timed).then(|| {
        Anchor::Transcript {
            path: path.to_string(),
            at: at.to_string(),
        }
    })
}

/// Whitespace collapsed and lowercase, how extraction compares a quote.
fn squash(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// `text` as the quote comparison sees it: whitespace collapsed, lowercase.
pub(crate) fn fold(text: &str) -> String {
    squash(text)
}

/// Whether `quote` appears in `text` the way [`check`] compares (whitespace
/// and case folded). The comparison memory provenance (ND7) makes against the
/// person's own words.
pub(crate) fn contains_quote(text: &str, quote: &str) -> bool {
    let wanted = squash(quote);
    !wanted.is_empty() && squash(text).contains(&wanted)
}

fn words(text: &str) -> std::collections::BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 1)
        .map(str::to_lowercase)
        .collect()
}

/// The line of `texts` that shares the most words with `quote`, cut for
/// display; `None` when none shares any.
pub(crate) fn nearest_line(quote: &str, texts: &[&str]) -> Option<String> {
    let wanted = words(quote);
    if wanted.is_empty() {
        return None;
    }
    let mut best: Option<(usize, &str)> = None;
    for line in texts.iter().flat_map(|t| t.lines()) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let score = words(line).intersection(&wanted).count();
        if score > 0 && best.is_none_or(|(s, _)| score > s) {
            best = Some((score, line));
        }
    }
    best.map(|(_, line)| {
        let line: String = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.chars().count() > 200 {
            format!("{}…", line.chars().take(199).collect::<String>())
        } else {
            line
        }
    })
}

/// Checks `quote` against what the vault and the session store hold at
/// `anchor`.
pub(crate) fn check(vault: &Path, anchor: &str, quote: &str) -> Quote {
    let quote = quote.trim();
    if quote.is_empty() || squash(quote) == squash(anchor) {
        return Quote::Unchecked("it quotes nothing but its anchor");
    }
    let Some(parsed) = parse_anchor(anchor) else {
        return Quote::Unchecked("its anchor isn't a conversation turn or a transcript line");
    };
    let (source, anchor_is): (Source, Box<dyn Fn(&str) -> bool>) = match &parsed {
        Anchor::Turn { key, turn } => {
            let wanted = format!("{key}#turn{turn}");
            (
                Source::Chat {
                    session_key: key.clone(),
                    from_turn: Some(*turn),
                    to_turn: Some(*turn),
                },
                Box::new(move |a: &str| a == wanted),
            )
        }
        Anchor::Transcript { path, at } => {
            let wanted = format!("{path}@{at}");
            (
                Source::Recording { path: path.clone() },
                Box::new(move |a: &str| a == wanted),
            )
        }
    };
    let Ok(loaded) = extract::load(vault, &source) else {
        return Quote::Unchecked("the conversation or transcript can't be read now");
    };
    let texts: Vec<&str> = loaded
        .segments
        .iter()
        .filter(|s| anchor_is(&s.anchor))
        .map(|s| s.text.as_str())
        .collect();
    let wanted = squash(quote);
    if texts.iter().any(|t| squash(t).contains(&wanted)) {
        return Quote::Verified;
    }
    Quote::Miss {
        nearest: nearest_line(quote, &texts),
    }
}

/// The refusal for a [`Quote::Miss`], in the card's words.
pub(crate) fn refusal(anchor: &str, nearest: Option<&str>) -> String {
    match nearest {
        Some(line) => format!(
            "The quote isn't at {anchor}. The nearest line there is “{line}”. Nothing was saved. Reject this proposal and have it proposed again from the source, or fix the source text it quotes."
        ),
        None => format!(
            "The quote isn't at {anchor}: no line there shares a word with it, or no line exists at that anchor. Nothing was saved. Reject this proposal and have it proposed again from the source, or fix the source text it quotes."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::tests::{source, vault_with_transcript};

    #[test]
    fn anchors_hollow_can_read_and_the_ones_it_cannot() {
        assert_eq!(
            parse_anchor("s-20261009-ab#turn4"),
            Some(Anchor::Turn {
                key: "s-20261009-ab".into(),
                turn: 4
            })
        );
        assert_eq!(
            parse_anchor("chat:s-1#turn2"),
            Some(Anchor::Turn {
                key: "s-1".into(),
                turn: 2
            })
        );
        assert_eq!(
            parse_anchor("workspaces/ops/sources/rec.md@00:05"),
            Some(Anchor::Transcript {
                path: "workspaces/ops/sources/rec.md".into(),
                at: "00:05".into()
            })
        );
        assert_eq!(
            parse_anchor("vault:notes/a.md@1:02:03"),
            Some(Anchor::Transcript {
                path: "notes/a.md".into(),
                at: "1:02:03".into()
            })
        );
        for unread in [
            "",
            "s-1#turn0",
            "s-1#turnx",
            "../x#turn2",
            "plaud:abc@00:05",
            "notes/a.md#decisions",
            "/etc/passwd.md@00:05",
            "a/../b.md@00:05",
            "notes/a.md@soon",
            "A conversation turn somewhere",
        ] {
            assert_eq!(parse_anchor(unread), None, "{unread:?}");
        }
    }

    #[test]
    fn the_nearest_line_is_the_one_sharing_the_most_words() {
        let texts = [
            "Hello there.\nThe Kestrel budget ceiling rose to 55,000 EUR\nBook the venue",
            "Nothing alike",
        ];
        assert_eq!(
            nearest_line("the kestrel budget ceiling fell to 40,000 EUR", &texts).as_deref(),
            Some("The Kestrel budget ceiling rose to 55,000 EUR")
        );
        assert_eq!(nearest_line("zebra quartz", &texts), None);
        assert_eq!(nearest_line("", &texts), None);
        let long = "word ".repeat(100);
        assert!(
            nearest_line("word", &[long.as_str()])
                .unwrap()
                .chars()
                .count()
                <= 200
        );
    }

    #[test]
    fn a_right_quote_passes_and_a_planted_wrong_one_is_refused_with_the_nearest_line() {
        let dir = vault_with_transcript();
        let Source::Recording { path } = source() else {
            unreachable!()
        };
        // The fixture's third line (00:20) says the Kestrel budget ceiling rose.
        let anchor = format!("{path}@00:20");
        let ok = check(
            dir.path(),
            &anchor,
            "the Kestrel   budget ceiling rose to 55,000 EUR",
        );
        assert_eq!(ok, Quote::Verified, "case and spacing are folded");
        let bad = check(
            dir.path(),
            &anchor,
            "the Kestrel budget ceiling fell to 40,000 EUR",
        );
        match bad {
            Quote::Miss {
                nearest: Some(line),
            } => assert!(
                line.contains("Kestrel budget ceiling rose to 55,000 EUR"),
                "{line}"
            ),
            other => panic!("{other:?}"),
        }
        // A quote that exists, but at another time, is not at this anchor.
        let elsewhere = check(dir.path(), &anchor, "I owe you the VendorSync export");
        assert!(matches!(elsewhere, Quote::Miss { .. }), "{elsewhere:?}");
        // A time with no transcript line is a miss with nothing near it.
        assert_eq!(
            check(dir.path(), &format!("{path}@09:59"), "anything said here"),
            Quote::Miss { nearest: None }
        );
    }

    #[test]
    fn what_cannot_be_checked_passes_and_says_why() {
        let dir = vault_with_transcript();
        let unchecked = |anchor: &str, quote: &str| match check(dir.path(), anchor, quote) {
            Quote::Unchecked(why) => why,
            other => panic!("{anchor}: {other:?}"),
        };
        assert!(
            unchecked("somewhere", "somewhere").contains("nothing but its anchor"),
            "an agent's free text is its own anchor"
        );
        assert!(unchecked("plaud:abc@00:05", "a quote").contains("isn't a conversation turn"));
        assert!(unchecked("s-no-such-chat#turn2", "a quote").contains("can't be read now"));
        assert!(
            unchecked("workspaces/ops/sources/missing.md@00:05", "a quote")
                .contains("can't be read now")
        );
        assert!(
            unchecked("workspaces/ops/sources/x.md@00:05", "  ").contains("nothing but its anchor")
        );
    }

    #[test]
    fn a_conversation_turn_is_read_from_the_session_store() {
        let dir = vault_with_transcript();
        let v = dir.path();
        let key = "s-quote-1";
        let ord = crate::session_store::start_turn(
            v,
            key,
            "claude",
            "/tmp",
            "run-quote-1",
            "Should we ship on Friday?",
        )
        .unwrap();
        crate::session_store::append_event(
            v,
            key,
            ord,
            "assistant",
            "We decided to ship on Friday after the gate passes.\nThe release note waits.",
        )
        .unwrap();
        crate::session_store::finish_turn(v, key, ord, "success").unwrap();
        let anchor = format!("{key}#turn1");
        assert_eq!(
            check(v, &anchor, "ship on friday AFTER the gate passes"),
            Quote::Verified
        );
        assert_eq!(
            check(v, &format!("chat:{anchor}"), "Should we ship on Friday?"),
            Quote::Verified,
            "the user's message is in the turn too"
        );
        match check(
            v,
            &anchor,
            "we decided to ship on Monday after the gate passes",
        ) {
            Quote::Miss {
                nearest: Some(line),
            } => assert!(line.starts_with("We decided to ship on Friday"), "{line}"),
            other => panic!("{other:?}"),
        }
        // Turn 2 isn't there: nothing was said at that anchor.
        assert!(matches!(
            check(v, &format!("{key}#turn2"), "ship on friday"),
            Quote::Unchecked(_) | Quote::Miss { .. }
        ));
    }

    #[test]
    fn the_refusal_names_the_anchor_the_line_and_that_nothing_was_saved() {
        let with = refusal("s-1#turn2", Some("We decided on Friday"));
        assert!(
            with.contains("s-1#turn2")
                && with.contains("“We decided on Friday”")
                && with.contains("Nothing was saved.")
        );
        assert!(
            with.contains("Reject this proposal"),
            "the card can't edit a quote, so it says what can be done"
        );
        let none = refusal("a.md@00:01", None);
        assert!(none.contains("shares a word") && none.contains("no line exists at that anchor"));
        assert!(!with.contains('\u{2014}') && !with.contains('\u{2013}'));
    }
}
