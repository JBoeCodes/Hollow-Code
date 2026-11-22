//! The permission-mode contract, as the agent is told it (agent
//! effectiveness F6; owner decisions 2026-10-02 and Q1 2026-10-06).
//!
//! Every sentence Hollow puts in front of an agent about asking, cards and
//! preapproval is generated here, per permission mode and agent capability:
//! the adapters' policy ([`policy`], after `agent::WORKSPACE_POLICY`), the
//! `<hollow-connectors>` note ([`connector_sentence`], [`external_confirm`]),
//! the Hollow app
//! guidance ([`app_cards`]) and computer control's tier-2 wording
//! ([`computer_tier2`], [`computer_brief`]).
//!
//! Only the words live here. What a mode allows is decided where it always
//! was, and the tests pin each sentence to it: `app_actions::asks_in_mode`
//! (Hollow app cards, [`app_actions::AUTO_ASK_CLASSES`] in auto),
//! `computer_mcp::server_entry` (tier 2 asks outside bypass),
//! [`connector_preapproved`] (library connectors, called by the command
//! builders) and `connectors::eligible` (none attach in plan).
//!
//! Q1: bypass shows no Hollow approval or confirm cards and carries no
//! Hollow-internal confirm wording (question cards and task suggestions are
//! not approvals: they reach the person in every mode), but in every mode
//! the agent still asks the person before a
//! shared connector sends, publishes, deletes or spends anything outside
//! this Mac ([`EXTERNAL_CONFIRM`]; an automation, which no one can answer,
//! only when its own prompt asks for exactly that).

use crate::app_actions;

/// A canonical permission mode (`app_mcp::PERMISSION_MODES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Plan,
    Default,
    AcceptEdits,
    Auto,
    Bypass,
}

impl Mode {
    #[cfg(test)]
    pub const ALL: [Mode; 5] = [
        Mode::Plan,
        Mode::Default,
        Mode::AcceptEdits,
        Mode::Auto,
        Mode::Bypass,
    ];

    /// `perm` read like every adapter reads it: empty or unknown is
    /// `acceptEdits` (`app_mcp::normalize_perm`).
    pub fn of(perm: &str) -> Mode {
        match crate::app_mcp::normalize_perm(perm) {
            "plan" => Mode::Plan,
            "default" => Mode::Default,
            "auto" => Mode::Auto,
            "bypassPermissions" => Mode::Bypass,
            _ => Mode::AcceptEdits,
        }
    }

    pub fn perm(self) -> &'static str {
        match self {
            Mode::Plan => "plan",
            Mode::Default => "default",
            Mode::AcceptEdits => "acceptEdits",
            Mode::Auto => "auto",
            Mode::Bypass => "bypassPermissions",
        }
    }

    /// The mode's name as the person sees it in the picker.
    fn label(self) -> &'static str {
        match self {
            Mode::Plan => "plan",
            Mode::Default => "default",
            Mode::AcceptEdits => "accept edits",
            Mode::Auto => "auto",
            Mode::Bypass => "bypass",
        }
    }
}

/// Which of Hollow's own approval cards a chat in this mode shows
/// (`app_actions::asks_in_mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cards {
    /// Every confirm and external action.
    Every,
    /// Only [`AUTO_ASK_EXTRAS`] and [`app_actions::AUTO_ASK_CLASSES`].
    Sensitive,
    None,
}

pub fn cards(mode: Mode) -> Cards {
    match mode {
        Mode::Bypass => Cards::None,
        Mode::Auto => Cards::Sensitive,
        _ => Cards::Every,
    }
}

/// What still asks in auto besides [`app_actions::AUTO_ASK_CLASSES`], one
/// per branch of `app_actions::asks_in_mode`: an external action, a
/// sensitive `settings.change` key, and `sessions.start` with a stronger
/// mode.
pub const AUTO_ASK_EXTRAS: [&str; 3] = [
    "anything that leaves this Mac",
    "a sensitive setting",
    "a session started in a stronger mode than this chat",
];

/// Q1: kept in every mode, in the `<hollow-connectors>` note of a chat.
pub const EXTERNAL_CONFIRM: &str = "Whatever the mode, before a shared connector sends, publishes, deletes or spends anything outside this Mac, ask the person and wait for their yes.";

/// The same rule for an automation, which no one can answer: it keeps its
/// boundary's rule (`workflows::WORKFLOW_BOUNDARY`), its own prompt being
/// the person's word.
pub const EXTERNAL_CONFIRM_AUTOMATION: &str = "This automation can't ask anyone, so through a shared connector it sends, publishes, deletes or spends anything outside this Mac only when its own prompt explicitly asks for exactly that.";

/// [`EXTERNAL_CONFIRM`], or its automation form.
pub fn external_confirm(automation: bool) -> &'static str {
    if automation {
        EXTERNAL_CONFIRM_AUTOMATION
    } else {
        EXTERNAL_CONFIRM
    }
}

/// Whether a shared library connector's tools run without a prompt:
/// always in `bypassPermissions`; in `acceptEdits`/`default` when the user
/// marked it auto-approve (a headless run would otherwise deny its tools);
/// never in `plan`, and never in `auto`. An auto run still attaches the
/// connector, but each call goes to the agent's own permission layer
/// (Claude's auto-mode classifier, Codex's automatic reviewer): a library
/// connector can reach off this Mac, which auto must not wave through
/// (review C, merged plan slice 2). The command builders call this, so the
/// note's claims ([`connector_sentence`]) and the preapproval are one rule.
pub fn connector_preapproved(auto_approve: bool, perm: &str) -> bool {
    match perm {
        "bypassPermissions" => true,
        "auto" | "plan" => false,
        _ => auto_approve,
    }
}

/// The permission paragraph every adapter appends after
/// `agent::WORKSPACE_POLICY`. `owned_mcp` is whether Hollow can attach its
/// own servers (app tools and their approval cards, question cards, shared
/// connectors) to the agent (`agent::capabilities`). Without them a chat
/// still posts question cards through a fenced block
/// (`interaction_schema::GUIDANCE_FENCED`), so that text names only what
/// can't attach: app tools, approval cards, shared connectors. True of every surface (chat, automation,
/// teammate, heartbeat), so it claims nothing about cards outside bypass:
/// where Hollow shows them is the chat's own guidance ([`app_cards`]). The
/// external confirmation ([`EXTERNAL_CONFIRM`]) rides in the
/// `<hollow-connectors>` note, which is there exactly when a run has shared
/// connectors (none attach in plan or without owned MCP,
/// `connectors::eligible`).
///
/// OB02 (overnight 10-17): every Hollow run is headless (Claude's `-p`), so
/// in default and accept edits a command or read the CLI would need
/// approval for is refused, never asked; the old "your own permission
/// check stops for it" read as a prompt that never comes. Those two modes
/// say so and give the forms that pass ([`HEADLESS_FORMS`]); auto, bypass
/// and plan are unchanged.
pub fn policy(perm: &str, owned_mcp: bool) -> String {
    const POINTER: &str = "only a shared connector's effects outside this Mac need you to ask yourself (the <hollow-connectors> note says how).";
    const NO_HOLLOW: &str = "Hollow can't attach its app tools, approval cards or shared connectors to this agent, so it reaches no external service through Hollow";
    let mode = Mode::of(perm);
    let label = mode.label();
    match (mode, owned_mcp) {
        (Mode::Plan, _) => "Permission mode: plan. Read and plan only: change nothing, and describe the changes you would make.".to_string(),
        (Mode::Bypass, true) => format!("Permission mode: bypass. The person approved this run's work in Hollow and on this Mac up front, and Hollow shows no approval cards, so don't stop to ask; {POINTER}"),
        (Mode::Bypass, false) => format!("Permission mode: bypass. {NO_HOLLOW}; the person approved this run's work on this Mac up front, so don't stop to ask."),
        (Mode::Auto, true) => format!("Permission mode: {label}. Where this mode needs the person's approval, Hollow or your own permission check stops for it, so you needn't ask first; {POINTER}"),
        (Mode::Auto, false) => format!("Permission mode: {label}. {NO_HOLLOW}; what this mode doesn't allow, your CLI refuses."),
        (_, true) => format!("Permission mode: {label}. What your CLI must approve is refused, not asked; {POINTER} {HEADLESS_FORMS}"),
        (_, false) => format!("Permission mode: {label}. {NO_HOLLOW}; what your CLI must approve is refused, not asked. {HEADLESS_FORMS}"),
    }
}

/// The forms a headless default or accept-edits run's commands pass in
/// (OB02): of 201 accept-edits Bash calls 91 were refused, 40 of them
/// `cd`-first or `$VAR` expansion, most of the rest pipes or compounds.
/// Every native session of such a chat carries it (in each mode line it is
/// sent, S02), so it is kept short (the heavy G2 turn's workspace layer
/// pays for each byte).
pub const HEADLESS_FORMS: &str = "One command per call: no `cd X &&` (use git -C), pipes into interpreters or $VARS (`printenv HOLLOW_SCRATCH_DIR` gives the scratch path). git, gh and gcloud logins are fine; suggest changes outside the workspace.";

/// The `<hollow-connectors>` note's preapproval sentence for the attached
/// connectors (`(name, auto_approve)`), from [`connector_preapproved`].
pub fn connector_sentence(perm: &str, attached: &[(&str, bool)]) -> String {
    let mode = Mode::of(perm);
    let preapproved: Vec<&str> = attached
        .iter()
        .filter(|(_, auto)| connector_preapproved(*auto, mode.perm()))
        .map(|(name, _)| *name)
        .collect();
    match mode {
        Mode::Bypass => "In bypass they all run without a prompt.".into(),
        Mode::Auto => {
            "Auto preapproves none of them: each call goes through your own permission check."
                .into()
        }
        Mode::Plan => "Plan mode preapproves none of them.".into(),
        Mode::Default | Mode::AcceptEdits => {
            if preapproved.len() == attached.len() {
                "Each is marked auto-approve, so they run without a prompt.".into()
            } else if preapproved.is_empty() {
                "None is marked auto-approve, and this run can't prompt, so their calls are refused."
                    .into()
            } else {
                format!(
                    "Only {} {} marked auto-approve and {} without a prompt; this run can't prompt for the others, so their calls are refused.",
                    preapproved.join(", "),
                    if preapproved.len() == 1 { "is" } else { "are" },
                    if preapproved.len() == 1 { "runs" } else { "run" },
                )
            }
        }
    }
}

/// The auto classes, joined with semicolons (a label may hold a comma), in
/// the order the agent reads them.
fn auto_list() -> String {
    let labels: Vec<&str> = AUTO_ASK_EXTRAS[..1]
        .iter()
        .copied()
        .chain(
            app_actions::AUTO_ASK_CLASSES
                .iter()
                .map(|class| class.label),
        )
        .chain(AUTO_ASK_EXTRAS[1..].iter().copied())
        .collect();
    let (last, rest) = labels.split_last().expect("classes");
    format!("{}; and {last}", rest.join("; "))
}

/// The Hollow app guidance's card sentences for a run that may change
/// things (a read-only run gets none). `automation`: an automation's run,
/// which no one can answer (`app_broker::execute`). Path-free, like the
/// rest of the guidance.
pub fn app_cards(perm: &str, automation: bool) -> String {
    if automation {
        return "This automation runs unattended, so nothing can ask the user: a confirm action, or a change whose card would ask, runs only when this automation's allowed tools name it, and an action that leaves this Mac never runs.".into();
    }
    match cards(Mode::of(perm)) {
        Cards::None => "In this bypass chat Hollow shows no approval or confirm cards, except gardener.apply, records.records_supersede_decision, records.records_set_decision_status, friction.draft_fix, build.install_built_app, build.publish_main, github.triage, build.approve_setup_hook, personas.accept, settings.cfo_approve_extra_usage, skills.approve_repo_skill, skills.set_repo_owner and workspaces.edit_standing_orders, which always ask, as do destructive or access-granting tracker.graphql mutations and the apply of workspaces.archive_shared_docs (Archive older entries): every other action, deletes and build_publish included, runs at once, so say what you did.".into(),
        Cards::Sensitive => format!(
            "In this auto chat most actions run without a card; Hollow still asks the user on one for: {}. A card for leaving this Mac names the destination. Say why in your reply before you call one of those; if the user declines, nothing changed.",
            auto_list()
        ),
        Cards::Every => "Confirm and external actions first ask the user on a card in the chat: deleting, spending, secrets and accounts, what agents may do (among them save_automation, run_automation, build_commit and a confirm-tier setting), and anything that leaves this Mac, such as build_publish, whose card names its destination. Say why in your reply before you call one; if the user declines, nothing changed.".into(),
    }
}

/// Computer control's tier-2 verbs (`browser_current_tab`,
/// `browser_list_tabs`, `quit_app`): they ask outside bypass
/// (`computer_mcp::ENV_BYPASS`). `bypass` is that variable's value.
pub fn computer_tier2(bypass: bool) -> &'static str {
    if bypass {
        "Tier-2 action: in this bypass chat it runs at once, with no card."
    } else {
        "Tier-2 action: the user must allow it first (a card in the chat, or a native Allow/Deny dialog); if they deny, accept it and move on."
    }
}

/// The `<hollow-mode>` line's sentence for those verbs ([`turn_line`]).
pub fn computer_brief(perm: &str) -> &'static str {
    if Mode::of(perm) == Mode::Bypass {
        "browser tab reads and quit_app run at once in this bypass chat"
    } else {
        "browser tab reads and quit_app run only after the user allows them on a card"
    }
}

/// The see-and-act desktop tools' line (Cua CU2), after [`computer_brief`]:
/// what each mode does with them (design section 4). In bypass the
/// outward and password-field cards still show; nothing else does.
pub fn computer_screen_brief(perm: &str) -> &'static str {
    match Mode::of(perm) {
        Mode::Plan => "see-and-act desktop tools do not run in plan mode",
        Mode::Default => "see-and-act desktop tools (the sandbox Mac) ask once per chat, and moving data between apps asks each time",
        Mode::AcceptEdits | Mode::Auto => "see-and-act desktop tools run on the sandbox Mac, and moving data between apps asks each time",
        Mode::Bypass => "see-and-act desktop tools run on the sandbox Mac with no card, but a send, post, pay or delete click and typing into a password field still ask",
    }
}

/// What a chat's configured half (the appended prompt and the policy
/// after it) says about the mode in every mode that acts (HB01, evening
/// 10-16): only where to find it. That text is in the native session's
/// configuration fingerprint and is Claude's system prompt, so a mode in it
/// started a fresh session (and a fresh prompt cache) every time a
/// Main-agent wake (acceptEdits) and the person's next turn (bypass)
/// alternated. The mode itself is the turn's [`turn_line`], which a resumed
/// turn leaves out when its session already holds that exact line (S02),
/// so this says "the latest", not "each turn's". Plan keeps its own
/// paragraph: it reads only, and a plan turn starts its own session.
pub const MODE_POINTER: &str = "Permission mode: the latest <hollow-mode> line in this conversation names the mode and what it means here. Hollow sends a new one when the mode changes; the latest applies.";

/// The chat policy paragraph after `agent::WORKSPACE_POLICY`: plan's own
/// [`policy`], else [`MODE_POINTER`].
pub fn chat_policy(perm: &str, owned_mcp: bool) -> String {
    match Mode::of(perm) {
        Mode::Plan => policy(perm, owned_mcp),
        _ => MODE_POINTER.to_string(),
    }
}

/// The Hollow app guidance's card sentence in a chat (HB01): where the
/// cards are said, since which ones show depends on the mode.
pub const APP_CARDS_POINTER: &str = "Which of these show the user a card first depends on the permission mode; the latest <hollow-mode> line says.";

/// The chat brief's computer line for the tier-2 verbs (HB01).
pub const COMPUTER_POINTER: &str =
    "browser tab reads and quit_app as the latest <hollow-mode> line says";

/// What a headless run reads without approval (OB02), for [`turn_line`]:
/// its readable roots (the working directory, the scratch folder when it
/// has one, and each other `--add-dir` folder, `extra`), and whether
/// Hollow's readers for what lies outside them are attached: `read_session`
/// (`hollow_app`) for another chat's answer, `read_source` (`hollow_vault`)
/// for a file Hollow attached or cited. Only for an agent whose reads stop
/// at its roots (`capability_contract::reads_only_its_roots`): of 68
/// Main-agent wakes, 27 hit a refusal, and 27 refused reads were another
/// workspace's files. The working directory and scratch folder are named,
/// not spelled out: the brief already gives the working directory's path
/// and `$HOLLOW_SCRATCH_DIR`, each production path is about 90 bytes, and
/// every native session gets the line (the heavy G2 turn's workspace layer
/// paid for it).
pub struct Reads<'a> {
    pub scratch: bool,
    pub extra: &'a [std::path::PathBuf],
    pub app: bool,
    pub vault: bool,
}

impl Reads<'_> {
    fn sentence(&self) -> String {
        let mut roots = vec!["the working directory".to_string()];
        if self.scratch {
            roots.push("scratch folder".to_string());
        }
        roots.extend(self.extra.iter().map(|dir| dir.display().to_string()));
        let (last, rest) = roots.split_last().expect("the working directory");
        let mut s = if rest.is_empty() {
            format!("Reads stop at {last}")
        } else {
            format!("Reads stop at {} and {last}", rest.join(", "))
        };
        s.push_str(match (self.app, self.vault) {
            (true, true) => "; read_session reads other chats, read_source cited notes",
            (true, false) => "; read_session reads other chats",
            (false, true) => "; read_source reads cited notes",
            (false, false) => "",
        });
        s.push('.');
        s
    }
}

/// A chat turn's `<hollow-mode>` line (HB01), in its message beside the
/// clock: the mode's [`policy`], then, for what is attached, the Hollow app
/// card sentence ([`app_cards`]; `app_changes`: the app tools may change
/// things) and computer control's tier-2 sentence ([`computer_brief`]).
/// The mode that applies is always the latest line in the conversation: a
/// resumed turn whose native session holds this exact line leaves it out,
/// a changed mode's line is sent, and every `stable_layers::REFRESH_TURNS`
/// resumed turns it is sent again, for the agent's own context compaction
/// (S02).
/// In default and accept edits, `reads` (OB02) adds where this run can read
/// right after the policy; the turn's line is in its message, not the
/// fingerprinted configured half, so it may name a build workspace's folder.
pub fn turn_line(
    perm: &str,
    owned_mcp: bool,
    app_changes: bool,
    computer: bool,
    reads: Option<&Reads<'_>>,
) -> String {
    let mode = Mode::of(perm);
    let mut line = policy(perm, owned_mcp);
    if let Some(reads) = reads.filter(|_| matches!(mode, Mode::Default | Mode::AcceptEdits)) {
        line.push(' ');
        line.push_str(&reads.sentence());
    }
    if app_changes {
        line.push(' ');
        line.push_str(&app_cards(perm, false));
    }
    if computer {
        line.push_str(" Computer control: ");
        line.push_str(computer_brief(perm));
        line.push_str("; ");
        line.push_str(computer_screen_brief(perm));
        line.push('.');
    }
    format!("<hollow-mode>{line} This replaces any mode an earlier turn named.</hollow-mode>\n\n")
}

/// Hollow-internal confirm wording: never in a bypass text (Q1). The
/// tests here and the chat fixture (`chat::mode_wording_tests`) check it.
#[cfg(test)]
pub(crate) const HOLLOW_INTERNAL_CONFIRM: &[&str] = &[
    "ask the user on a card",
    "asks the user on a card",
    "behind approval",
    "must allow it first",
    "must approve",
    "if the user declines",
    "allows them on a card",
    "Confirm with the user before anything",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_actions::{asks_in_mode, Args, Tier, ACTIONS, AUTO_ASK_CLASSES};

    /// HB01: the chat policy says nothing mode-specific outside plan, and
    /// each turn's line carries what the mode means, with the bypass line
    /// free of Hollow-internal confirm wording (Q1).
    #[test]
    fn the_turn_line_carries_the_mode_and_the_chat_policy_doesnt() {
        for mode in Mode::ALL {
            let perm = mode.perm();
            let line = turn_line(perm, true, true, true, None);
            assert!(line.starts_with("<hollow-mode>"), "{line}");
            assert!(line.ends_with("</hollow-mode>\n\n"), "{line}");
            assert!(line.contains(&policy(perm, true)), "{mode:?}");
            assert!(line.contains(&app_cards(perm, false)), "{mode:?}");
            assert!(line.contains(computer_brief(perm)), "{mode:?}");
            if mode == Mode::Plan {
                assert_eq!(chat_policy(perm, true), policy(perm, true));
            } else {
                assert_eq!(chat_policy(perm, true), MODE_POINTER, "{mode:?}");
                assert_eq!(chat_policy(perm, false), MODE_POINTER, "{mode:?}");
            }
        }
        let bypass = turn_line("bypassPermissions", true, true, true, None);
        for phrase in HOLLOW_INTERNAL_CONFIRM {
            assert!(!bypass.contains(phrase), "{phrase}");
        }
        for text in [MODE_POINTER, APP_CARDS_POINTER, COMPUTER_POINTER] {
            for phrase in HOLLOW_INTERNAL_CONFIRM {
                assert!(!text.contains(phrase), "{phrase}");
            }
        }
        // Only what is attached is described.
        let bare = turn_line("auto", false, false, false, None);
        assert_eq!(
            bare,
            format!(
                "<hollow-mode>{} This replaces any mode an earlier turn named.</hollow-mode>\n\n",
                policy("auto", false)
            )
        );
    }

    /// OB02: a headless default or accept-edits turn is told that what
    /// needs approval is refused, not asked, the command forms that pass,
    /// and, for an agent whose reads stop at its roots, the roots and
    /// Hollow's readers for the rest.
    #[test]
    fn a_headless_turn_line_says_refused_and_names_the_readable_roots() {
        let extra = [std::path::PathBuf::from(
            "/Users/x/vault/workspaces/build-1",
        )];
        let reads = Reads {
            scratch: true,
            extra: &extra,
            app: true,
            vault: true,
        };
        for perm in ["acceptEdits", "default"] {
            let line = turn_line(perm, true, true, false, Some(&reads));
            for said in [
                "Reads stop at the working directory, scratch folder and /Users/x/vault/workspaces/build-1;",
                "refused, not asked",
                "read_session",
                "read_source",
                "git -C",
                "$VARS",
            ] {
                assert!(line.contains(said), "{perm}: {said} in {line}");
            }
            for owned in [true, false] {
                assert!(
                    policy(perm, owned).contains(HEADLESS_FORMS),
                    "{perm} {owned}"
                );
            }
            // Only the folders it has and the readers attached are named.
            let bare = Reads {
                scratch: false,
                extra: &[],
                app: false,
                vault: false,
            };
            let line = turn_line(perm, true, false, false, Some(&bare));
            assert!(
                line.contains(" Reads stop at the working directory. "),
                "{line}"
            );
            assert!(!line.contains("scratch folder") && !line.contains("read_session"));
            assert!(!line.contains("read_source"), "{line}");
            let chat = Reads {
                scratch: true,
                extra: &[],
                app: true,
                vault: true,
            };
            let line = turn_line(perm, true, false, false, Some(&chat));
            assert!(
                line.contains(" Reads stop at the working directory and scratch folder; read_session reads other chats, read_source cited notes. "),
                "{line}"
            );
        }
        // Auto, bypass and plan say what they said, roots or not.
        for perm in ["auto", "bypassPermissions", "plan"] {
            assert_eq!(
                turn_line(perm, true, true, true, Some(&reads)),
                turn_line(perm, true, true, true, None),
                "{perm}"
            );
        }
    }

    /// OB02 changes only default and accept edits: the bypass and auto
    /// paragraphs are byte-identical to HB01's.
    #[test]
    fn bypass_and_auto_policies_are_unchanged() {
        assert_eq!(
            policy("bypassPermissions", true),
            "Permission mode: bypass. The person approved this run's work in Hollow and on this Mac up front, and Hollow shows no approval cards, so don't stop to ask; only a shared connector's effects outside this Mac need you to ask yourself (the <hollow-connectors> note says how)."
        );
        assert_eq!(
            policy("auto", true),
            "Permission mode: auto. Where this mode needs the person's approval, Hollow or your own permission check stops for it, so you needn't ask first; only a shared connector's effects outside this Mac need you to ask yourself (the <hollow-connectors> note says how)."
        );
        assert_eq!(
            policy("auto", false),
            "Permission mode: auto. Hollow can't attach its app tools, approval cards or shared connectors to this agent, so it reaches no external service through Hollow; what this mode doesn't allow, your CLI refuses."
        );
        for perm in ["auto", "bypassPermissions", "plan"] {
            for owned in [true, false] {
                assert!(
                    !policy(perm, owned).contains("refused, not asked"),
                    "{perm}"
                );
            }
        }
    }

    #[test]
    fn modes_read_like_the_adapters_read_them() {
        for mode in Mode::ALL {
            assert_eq!(Mode::of(mode.perm()), mode);
        }
        assert_eq!(Mode::of(""), Mode::AcceptEdits);
        assert_eq!(Mode::of("nonsense"), Mode::AcceptEdits);
        assert_eq!(
            Mode::ALL.map(Mode::perm),
            crate::app_mcp::PERMISSION_MODES,
            "every mode, weakest first"
        );
    }

    /// [`cards`] is what `asks_in_mode` does: bypass asks for nothing, auto
    /// for some confirm actions and every external one, the rest for all.
    /// The always-ask list asks in every mode, and the bypass wording names
    /// each of its actions.
    #[test]
    fn cards_match_asks_in_mode() {
        let none = Args(serde_json::Map::new());
        for id in crate::app_actions::ALWAYS_ASK {
            assert!(app_cards("bypassPermissions", false).contains(id), "{id}");
        }
        let gated: Vec<_> = ACTIONS
            .iter()
            .filter(|a| matches!(a.tier, Tier::Confirm | Tier::External))
            .filter(|a| !crate::app_actions::always_asks(a.id))
            .collect();
        for mode in Mode::ALL {
            let asked = gated
                .iter()
                .filter(|a| asks_in_mode(a, &none, mode.perm()))
                .count();
            let expected = match cards(mode) {
                Cards::None => 0,
                Cards::Every => gated.len(),
                Cards::Sensitive => {
                    assert!(asked > 0 && asked < gated.len(), "{mode:?}: {asked}");
                    asked
                }
            };
            assert_eq!(asked, expected, "{mode:?}");
        }
    }

    /// The auto wording names every class and extra exactly once, and each
    /// extra is one `asks_in_mode` branch that really asks in auto.
    #[test]
    fn auto_names_exactly_the_classes_that_still_ask() {
        let text = app_cards("auto", false);
        let names: Vec<&str> = AUTO_ASK_CLASSES
            .iter()
            .map(|class| class.label)
            .chain(AUTO_ASK_EXTRAS)
            .collect();
        for name in &names {
            assert_eq!(text.matches(name).count(), 1, "{name}: {text}");
            assert!(!name.contains('/') && !name.contains(';'), "{name}");
        }
        let listed = text
            .split(" for: ")
            .nth(1)
            .and_then(|rest| rest.split(". ").next())
            .unwrap();
        let mut parts: Vec<&str> = listed
            .split("; ")
            .map(|p| p.trim_start_matches("and "))
            .collect();
        let mut expected = names.clone();
        parts.sort_unstable();
        expected.sort_unstable();
        assert_eq!(parts, expected, "nothing named that doesn't ask");

        let none = Args(serde_json::Map::new());
        let external = ACTIONS.iter().find(|a| a.tier == Tier::External).unwrap();
        assert!(
            asks_in_mode(external, &none, "auto"),
            "{}",
            AUTO_ASK_EXTRAS[0]
        );
        let secret = Args(serde_json::Map::from_iter([(
            "key".to_string(),
            serde_json::json!("jev.apiKey"),
        )]));
        let change = crate::app_actions::find("settings.change").unwrap();
        assert!(
            asks_in_mode(change, &secret, "auto"),
            "{}",
            AUTO_ASK_EXTRAS[1]
        );
        let start = crate::app_actions::find("sessions.start").unwrap();
        assert!(asks_in_mode(start, &none, "auto"), "{}", AUTO_ASK_EXTRAS[2]);
        for class in AUTO_ASK_CLASSES {
            for id in class.ids {
                let action = crate::app_actions::find(id).unwrap();
                assert!(asks_in_mode(action, &none, "auto"), "{id}");
            }
        }
    }

    /// Q1: bypass texts carry no Hollow-internal confirm wording; every
    /// mode and agent keeps the external confirmation.
    #[test]
    fn bypass_drops_hollow_confirm_wording_and_every_mode_keeps_the_external_one() {
        let bypass = [
            policy("bypassPermissions", true),
            policy("bypassPermissions", false),
            app_cards("bypassPermissions", false),
            computer_tier2(true).to_string(),
            computer_brief("bypassPermissions").to_string(),
            computer_screen_brief("bypassPermissions").to_string(),
            connector_sentence("bypassPermissions", &[("Mail", false)]),
        ];
        for text in &bypass {
            for phrase in HOLLOW_INTERNAL_CONFIRM {
                assert!(!text.contains(phrase), "{phrase:?} in {text}");
            }
        }
        for mode in Mode::ALL {
            for owned in [true, false] {
                let text = policy(mode.perm(), owned);
                assert_eq!(text.matches("Permission mode: ").count(), 1);
                // The note carries the confirmation; the policy points at
                // it wherever a connector can attach.
                let attaches = owned && mode != Mode::Plan;
                assert_eq!(
                    text.contains("<hollow-connectors>"),
                    attaches,
                    "{mode:?} {owned}"
                );
                assert!(!text.contains(EXTERNAL_CONFIRM));
            }
            let note = connector_sentence(mode.perm(), &[("Mail", false)]);
            assert!(!note.contains(EXTERNAL_CONFIRM), "the note adds it once");
        }

        // Outside bypass the cards are described where they are shown.
        for perm in ["default", "acceptEdits"] {
            assert!(app_cards(perm, false).contains("ask the user on a card"));
        }
        assert!(computer_tier2(false).contains("must allow it first"));
        assert!(computer_brief("auto").contains("allows them on a card"));
    }

    /// CU2: each mode's sentence about the see-and-act tools says what the
    /// broker does (`computer_broker::required_asks`).
    #[test]
    fn the_screen_brief_names_each_modes_cards() {
        assert!(computer_screen_brief("plan").contains("do not run"));
        assert!(computer_screen_brief("default").contains("once per chat"));
        for perm in ["acceptEdits", "auto"] {
            let t = computer_screen_brief(perm);
            assert!(t.contains("run on the sandbox") && !t.contains("once per chat"));
        }
        let b = computer_screen_brief("bypassPermissions");
        assert!(b.contains("still ask") && b.contains("password"));
    }

    /// The tier-2 wording follows `computer_mcp::server_entry`: the bypass
    /// flag is set exactly in bypass.
    #[test]
    fn computer_wording_follows_the_bypass_flag() {
        for mode in Mode::ALL {
            let bypass = mode == Mode::Bypass;
            assert_eq!(
                computer_brief(mode.perm()).contains("at once"),
                bypass,
                "{mode:?}"
            );
        }
    }

    /// An automation can't ask: its wording promises no card in any mode.
    #[test]
    fn automations_are_told_nothing_can_ask() {
        let text = app_cards("acceptEdits", true);
        assert_eq!(text, app_cards("bypassPermissions", true));
        assert!(!text.contains("on a card") && text.contains("never runs"));
    }

    /// Every preapproval claim in the note is [`connector_preapproved`]'s
    /// answer for that connector in that mode.
    #[test]
    fn connector_claims_match_the_preapproval() {
        let mixed = [("Mail", true), ("Drive", false)];
        for mode in Mode::ALL {
            let perm = mode.perm();
            for set in [&mixed[..], &mixed[..1], &mixed[1..]] {
                let text = connector_sentence(perm, set);
                let pre: Vec<&str> = set
                    .iter()
                    .filter(|(_, auto)| connector_preapproved(*auto, perm))
                    .map(|(name, _)| *name)
                    .collect();
                let says_runs =
                    text.contains("without a prompt") && !text.contains("can't prompt for");
                if pre.len() == set.len() {
                    assert!(says_runs && !text.contains("refused"), "{perm}: {text}");
                } else if pre.is_empty() {
                    assert!(
                        !text.contains("run without a prompt") && !text.contains("runs without"),
                        "{perm}: {text}"
                    );
                } else {
                    for name in &pre {
                        assert!(text.contains(&format!("Only {name} ")), "{perm}: {text}");
                    }
                    for (name, _) in set.iter().filter(|(n, _)| !pre.contains(n)) {
                        assert!(!text.contains(name), "{perm}: {text}");
                    }
                    assert!(text.contains("refused"), "{perm}: {text}");
                }
            }
        }
        assert_eq!(
            connector_sentence("auto", &mixed),
            "Auto preapproves none of them: each call goes through your own permission check."
        );
    }
}
