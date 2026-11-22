//! Test-only helpers for the source-text tests that pin how a function is
//! wired (`include_str!` plus a needle). The crate is rustfmt-gated in CI,
//! and rustfmt moves line breaks and adds trailing commas whenever a call
//! grows or shrinks, so a needle written on one line must not depend on
//! layout. These match a needle against source the way a reader would:
//! whitespace may appear (or not) between tokens, a run of whitespace in the
//! needle matches any run in the source, and a trailing comma may precede a
//! closing bracket. Identifiers stay intact: `foo bar` never matches `foobar`.

use regex::Regex;

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The layout-insensitive pattern for `needle`.
pub(crate) fn pattern(needle: &str) -> Regex {
    let mut out = String::new();
    let mut prev: Option<char> = None;
    let mut gap = false;
    for c in needle.chars() {
        if c.is_whitespace() {
            gap = prev.is_some();
            continue;
        }
        if let Some(p) = prev {
            if is_word(p) && is_word(c) {
                if gap {
                    out.push_str(r"\s+");
                }
            } else {
                out.push_str(r"\s*");
            }
        }
        if matches!(c, ')' | ']' | '}') {
            out.push_str(r"(?:,\s*)?");
        }
        out.push_str(&regex::escape(&c.to_string()));
        prev = Some(c);
        gap = false;
    }
    Regex::new(&out).expect("a source-pin pattern")
}

/// Byte offset of the first layout-insensitive match of `needle` in `hay`.
pub(crate) fn find(hay: &str, needle: &str) -> Option<usize> {
    pattern(needle).find(hay).map(|m| m.start())
}

/// Whether `hay` contains `needle`, ignoring layout.
pub(crate) fn contains(hay: &str, needle: &str) -> bool {
    pattern(needle).is_match(hay)
}

/// How many non-overlapping layout-insensitive matches of `needle` `hay` has.
pub(crate) fn count(hay: &str, needle: &str) -> usize {
    pattern(needle).find_iter(hay).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_one_line_needle_matches_rustfmt_wrapped_calls() {
        let wrapped =
            "let (a, b) = crate::m::f(\n        agent,\n        &perm,\n        grant,\n    );";
        assert!(contains(wrapped, "crate::m::f(agent, &perm, grant)"));
        assert!(contains(wrapped, "let (a, b) = crate::m::f("));
        assert_eq!(find(wrapped, "f(agent"), wrapped.find("f(\n"));
        assert_eq!(count("g(x)\ng(\n    x,\n)", "g(x)"), 2);
    }

    #[test]
    fn identifiers_and_tokens_stay_intact() {
        assert!(!contains("foobar(x)", "foo bar(x)"));
        assert!(!contains("foo bar(x)", "foobar(x)"));
        assert!(!contains("f(x, y)", "f(x)"));
        assert!(contains("a.b", "a . b"));
    }
}
