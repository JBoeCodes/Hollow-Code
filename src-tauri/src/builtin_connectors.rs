//! Connectors built into Hollow: Microsoft 365 on Hollow's own sign-in,
//! instead of a claude.ai account's, and Plaud's own MCP server (overnight
//! 2026-10-13 S8). Hollow's own Notion connector (`hollow_notion`) was
//! removed on 2026-11-10; Notion's official connector (`notion-official`,
//! a library row) reads the archive instead.
//!
//! A claude.ai connector is signed in per claude.ai account and organization,
//! and a run that uses one must go out on Claude Code's own login
//! (`connectors.rs`), so it breaks on every other Claude account and in
//! Codex. Microsoft 365 runs as a hidden `hollow` subcommand (`m365-mcp`)
//! with Hollow's Microsoft sign-in (`m365_auth.rs`). One sign-in serves
//! every agent and every account.
//!
//! They live in the connectors library (`Transport::Builtin`, ids starting
//! `hollow_`, which users can't take) with the usual switches. A built-in is
//! attached only while its sign-in is ready, and while it is, the claude.ai
//! connector for the same service ([`Builtin::supersedes_host`]) is left out,
//! so agents see one Microsoft 365 and the run can use any account. A
//! library row for a built-in this build doesn't have (an older
//! `hollow_notion`) is dropped from the library view and never attached.
//!
//! Plaud is the one built-in that isn't a `hollow` subcommand
//! ([`Builtin::program`]): it runs Plaud's installed `plaud-mcp`, which keeps
//! its own sign-in (`plaud_mcp_auth.rs`, not the CLI sign-in Recordings use).
//! A Plaud the person added themselves wins: while one is in the library
//! ([`is_own`]), the built-in stands aside ([`aside`]) and is never attached.
use serde::Serialize;

pub struct Builtin {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// The hidden `hollow` subcommand that serves it.
    pub subcommand: &'static str,
    /// A claude.ai connector at this host is skipped while this one is ready.
    pub supersedes_host: Option<&'static str>,
    /// An installed program it runs instead of a `hollow` subcommand, found
    /// on the tool PATH a local connector gets.
    pub program: Option<&'static str>,
    /// Its tools no agent gets (Claude `--disallowed-tools`, Codex
    /// `disabled_tools`): Plaud's `login` / `logout`, which open a browser
    /// sign-in or revoke the grant. Those are the person's, in Settings.
    pub hidden_tools: &'static [&'static str],
    /// The default "run without asking" for a fresh row.
    pub auto_approve: bool,
}

/// Supersedes claude.ai's Microsoft 365 while signed in (five asks B2,
/// 2026-10-10): calendar and files reach every account and Codex. claude.ai's
/// also has mail, which Hollow deliberately doesn't; it was switched off in
/// Alex's library when this changed, so nothing in use is lost.
pub const M365: Builtin = Builtin {
    id: "hollow_m365",
    name: "Microsoft 365",
    description: "Microsoft 365 calendar (Outlook events) and OneDrive / SharePoint files: search, read documents. Read-only; no mail or Teams.",
    subcommand: "m365-mcp",
    supersedes_host: Some("microsoft365.mcp.claude.com"),
    program: None,
    hidden_tools: &[],
    auto_approve: true,
};

/// Plaud recordings for every agent and account (overnight S8), through
/// Plaud's own MCP server and its own sign-in.
pub const PLAUD: Builtin = Builtin {
    id: "hollow_plaud",
    name: "Plaud",
    description: "Plaud recordings: list and find them, read transcripts, summaries and notes. Runs Plaud's own MCP server (plaud-mcp) on its own sign-in.",
    subcommand: "",
    // Plaud's installer also connects its remote server on claude.ai.
    supersedes_host: Some("mcp.plaud.ai"),
    program: Some(crate::plaud_mcp_auth::PROGRAM),
    hidden_tools: &["login", "logout"],
    // Its sign-in tools are hidden, so what agents get only reads. Headless
    // runs can't ask, so `false` would refuse every Plaud call in the default
    // mode (S8 re-review); like the other built-ins, it runs without asking.
    auto_approve: true,
};

pub const ALL: [&Builtin; 2] = [&M365, &PLAUD];

/// The tools of the attached server `name` no agent gets.
pub fn hidden_tools(name: &str) -> &'static [&'static str] {
    get(name).map(|b| b.hidden_tools).unwrap_or(&[])
}

/// Whether a connector the person added (`name`, `command`, `args`, `url`)
/// is their own copy of built-in `b`, which then stands aside. Only Plaud has
/// one: a command or argument that is `plaud-mcp`, its npm package
/// (`@plaud-ai/mcp`, `@plaud-ai/mcp@1.2`) or a file inside it
/// (`node …/@plaud-ai/mcp/dist/index.js`, the package's own `.mcp.json`); a
/// remote at `plaud.ai` or `*.plaud.ai`; or one named Plaud.
pub fn is_own(b: &Builtin, name: &str, command: &str, args: &[String], url: &str) -> bool {
    let Some(program) = b.program else {
        return false;
    };
    let package = crate::plaud_mcp_auth::PACKAGE;
    let runs = |word: &str| {
        word.rsplit('/').next() == Some(program)
            || word == package
            || word.ends_with(&format!("/{package}"))
            || word.starts_with(&format!("{package}@"))
            || word.contains(&format!("{package}/"))
    };
    let host = reqwest::Url::parse(url.trim())
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase));
    runs(command)
        || args.iter().any(|a| runs(a))
        || host.is_some_and(|h| h == "plaud.ai" || h.ends_with(".plaud.ai"))
        || name.trim().eq_ignore_ascii_case(b.name)
}

/// The state of a built-in standing aside for the person's own `own_name`
/// connector (reads nothing).
pub fn aside(id: &str, own_name: &str) -> Option<BuiltinState> {
    let b = get(id)?;
    Some(BuiltinState {
        ready: false,
        kind: kind(id)?,
        line: format!(
            "Stands aside: your own {} connector “{own_name}” is in use, so agents get that one. Remove yours from the library to use this one instead.",
            b.name
        ),
        program_missing: false,
        own: Some(own_name.to_string()),
    })
}

fn kind(id: &str) -> Option<&'static str> {
    match id {
        "hollow_m365" => Some("m365"),
        "hollow_plaud" => Some("plaud"),
        _ => None,
    }
}

pub fn get(id: &str) -> Option<&'static Builtin> {
    ALL.into_iter().find(|b| b.id == id)
}

/// Whether a built-in connector's sign-in is in place (no network). Reads
/// Hollow's Keychain item for Microsoft 365, so the GUI calls this, never a
/// test.
pub fn ready(id: &str) -> bool {
    match id {
        "hollow_m365" => crate::m365_auth::signed_in(),
        "hollow_plaud" => crate::plaud_mcp_auth::ready(),
        _ => false,
    }
}

/// What Settings shows for a built-in connector.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinState {
    pub ready: bool,
    /// `m365` or `plaud`: which sign-in the UI offers.
    pub kind: &'static str,
    /// One line on what it runs as, or what's missing.
    pub line: String,
    /// The program it runs isn't installed (Plaud).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub program_missing: bool,
    /// The name of the person's own connector it stands aside for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own: Option<String>,
}

pub fn state(id: &str) -> Option<BuiltinState> {
    let ready = ready(id);
    let simple = |kind, line: String| BuiltinState {
        ready,
        kind,
        line,
        program_missing: false,
        own: None,
    };
    Some(match id {
        "hollow_m365" => {
            let account = crate::m365_auth::load()
                .map(|t| t.account)
                .filter(|a| !a.is_empty());
            simple(
                "m365",
                match (ready, account, crate::m365_auth::stored_problem()) {
                    (true, Some(a), _) => format!("Signed in to Microsoft as {a}. Works on every Claude account and in Codex."),
                    (true, None, _) => "Signed in to Microsoft. Works on every Claude account and in Codex.".into(),
                    // A refused grant (five asks B3): say so, not "sign in once".
                    (false, _, Some(problem)) => format!("The Microsoft sign-in stopped working: {problem}"),
                    (false, _, None) => "Sign in to Microsoft once, here in Hollow.".into(),
                },
            )
        }
        "hollow_plaud" => {
            let missing = !crate::plaud_mcp_auth::installed();
            BuiltinState {
                program_missing: missing,
                ..simple("plaud", plaud_line(ready, missing))
            }
        }
        _ => return None,
    })
}

/// The Plaud row's line.
fn plaud_line(ready: bool, missing: bool) -> String {
    if ready {
        "Runs Plaud's MCP server on its own Plaud sign-in, on every Claude account and in Codex."
            .into()
    } else if missing {
        format!(
            "Needs Plaud's MCP server on this Mac (`{}`), then Check again.",
            crate::plaud_mcp_auth::INSTALL_HINT
        )
    } else {
        "Sign in to Plaud once, here in Hollow. Plaud's MCP server keeps its own sign-in, separate from Recordings'.".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Overnight S8 review: every shape of a hand-added Plaud counts as
    /// theirs; unrelated look-alikes don't.
    #[test]
    fn a_hand_added_plaud_is_recognised_in_every_shape_and_look_alikes_are_not() {
        let own = |name: &str, command: &str, args: &[&str], url: &str| {
            let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            is_own(&PLAUD, name, command, &args, url)
        };
        let node_script = "/opt/homebrew/lib/node_modules/@plaud-ai/mcp/dist/index.js";
        assert!(
            own("x", "/opt/homebrew/bin/plaud-mcp", &[], ""),
            "the program"
        );
        assert!(own("x", "plaud-mcp", &[], ""), "the bare program");
        assert!(
            own("x", "npx", &["-y", "@plaud-ai/mcp"], ""),
            "npx, the package"
        );
        assert!(
            own("x", "npx", &["-y", "@plaud-ai/mcp@0.3.7"], ""),
            "npx, a version"
        );
        assert!(
            own("x", "node", &[node_script], ""),
            "node and the package's script (its .mcp.json)"
        );
        assert!(own("x", node_script, &[], ""), "the script as the command");
        assert!(
            own("Plaud", "/usr/local/bin/something", &[], ""),
            "named Plaud"
        );
        assert!(
            own("Plaud Recordings", "", &[], "https://mcp.plaud.ai/mcp"),
            "a remote at plaud.ai"
        );
        assert!(
            own("Recorder", "", &[], "https://plaud.ai/mcp"),
            "plaud.ai itself"
        );
        // Look-alikes.
        assert!(
            !own("plaud-notes", "/usr/local/bin/notes-mcp", &[], ""),
            "an unrelated stdio command"
        );
        assert!(
            !own("Recordings", "/opt/homebrew/bin/plaud", &[], ""),
            "the Plaud CLI isn't its server"
        );
        assert!(
            !own("Notes", "", &[], "https://notplaud.ai/mcp"),
            "another host ending in plaud.ai's letters"
        );
        assert!(
            !own("Notes", "", &[], "https://plaud.ai.example.com/mcp"),
            "another host"
        );
        assert!(
            !own("x", "npx", &["-y", "@plaud-ai/mcp-extra"], ""),
            "another package"
        );
        assert!(
            !is_own(&M365, "Microsoft 365", "m365-mcp", &[], ""),
            "only Plaud has one"
        );
    }

    #[test]
    fn plaud_supersedes_claudeais_plaud_and_keeps_its_sign_in_tools_from_agents() {
        assert_eq!(PLAUD.supersedes_host, Some("mcp.plaud.ai"));
        assert_eq!(hidden_tools("hollow_plaud"), ["login", "logout"]);
        assert!(hidden_tools("hollow_m365").is_empty() && hidden_tools("linear").is_empty());
        // Hollow's own Notion connector is gone (2026-11-10).
        assert!(get("hollow_notion").is_none() && ALL.len() == 2);
        let approves = |id: &str| get(id).unwrap().auto_approve;
        assert!(
            approves("hollow_plaud") && approves("hollow_m365"),
            "read-only once login/logout are hidden; `false` would refuse every call headless"
        );
    }
}
