//! A table of known connectors and what must be true of them before they run
//! (CD1, from chrome-devtools-mcp): a pinned version, and every telemetry,
//! update-check and lookup opt-out preset. A connector that sends usage
//! statistics by default is a connector that sends them the day it is added;
//! the table is how Hollow notices, and the Computer control Doctor reports
//! each missing opt-out with its fix.
//!
//! Configuration only: nothing here edits a connector. Flag and variable names
//! change per release, so each entry names the version they were read for and a
//! person confirms them when they pin another. Entries are matched by the
//! command and arguments the connector runs.

use crate::connectors::Connector;

/// One opt-out: satisfied by any of these arguments, or by this variable set
/// to this value.
pub(crate) struct OptOut {
    pub what: &'static str,
    pub args: &'static [&'static str],
    pub env: Option<(&'static str, &'static str)>,
}

pub(crate) struct Known {
    pub name: &'static str,
    /// The connector runs this when its command or an argument contains it.
    pub matches: &'static str,
    /// The version the opt-outs below were read for.
    #[allow(dead_code)] // documents which release the opt-outs were read for
    pub read_for: &'static str,
    pub opt_outs: &'static [OptOut],
}

pub(crate) const KNOWN: &[Known] = &[Known {
    name: "chrome-devtools-mcp",
    matches: "chrome-devtools-mcp",
    read_for: "0.9 (its README, 2026-11)",
    opt_outs: &[
        OptOut {
            what: "usage statistics",
            args: &["--no-usage-statistics"],
            env: Some(("CHROME_DEVTOOLS_MCP_NO_USAGE_STATISTICS", "1")),
        },
        OptOut {
            what: "update checks",
            args: &[],
            env: Some(("CHROME_DEVTOOLS_MCP_NO_UPDATE_CHECKS", "1")),
        },
        OptOut {
            what: "CrUX lookups",
            args: &["--no-performance-crux"],
            env: None,
        },
    ],
}];

/// Whether an argument pins a version: `name@1.2.3` (not `@latest`, a tag or a
/// range).
fn pinned(arg: &str) -> bool {
    arg.rsplit_once('@').is_some_and(|(name, v)| {
        !name.is_empty()
            && !v.is_empty()
            && v.bytes().all(|b| {
                b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'+') || b.is_ascii_lowercase()
            })
            && v.bytes().next().is_some_and(|b| b.is_ascii_digit())
    })
}

/// What is missing from one connector, in words, for a connector in the table.
pub(crate) fn findings(c: &Connector) -> Vec<String> {
    let haystack: Vec<&str> = std::iter::once(c.command.as_str())
        .chain(c.args.iter().map(String::as_str))
        .collect();
    let Some(known) = KNOWN
        .iter()
        .find(|k| haystack.iter().any(|h| h.contains(k.matches)))
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if !haystack
        .iter()
        .any(|h| h.contains(known.matches) && pinned(h))
    {
        out.push(format!(
            "{} runs {} with no pinned version (use name@x.y.z, not @latest)",
            c.name, known.name
        ));
    }
    for o in known.opt_outs {
        let by_arg = c.args.iter().any(|a| o.args.contains(&a.as_str()));
        let by_env = o
            .env
            .is_some_and(|(k, v)| c.env.get(k).is_some_and(|x| x == v));
        if !by_arg && !by_env {
            let how = match (o.args.first(), o.env) {
                (Some(a), Some((k, v))) => format!("{a} or {k}={v}"),
                (Some(a), None) => (*a).to_string(),
                (None, Some((k, v))) => format!("{k}={v}"),
                (None, None) => "its opt-out".to_string(),
            };
            out.push(format!("{} has {} on (set {how})", c.name, o.what));
        }
    }
    out
}

/// Findings across the whole library.
pub(crate) fn library_findings(connectors: &[Connector]) -> Vec<String> {
    connectors.iter().flat_map(findings).collect()
}

#[cfg(test)]
#[path = "connector_hygiene_tests.rs"]
mod tests;
