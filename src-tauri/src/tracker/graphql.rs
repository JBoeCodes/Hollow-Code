//! `tracker.graphql`: any Linear GraphQL request with Hollow's own API key
//! (the tracker engine's saved key), so an agent can do in Linear what an
//! admin can, settings included (a project status, a label, a workflow
//! state). Alex's go, 2026-11-11; the official Linear connector has no
//! workspace-settings tools.
//!
//! Guards, all before the request is sent:
//! - the document must hold exactly one query or mutation (no
//!   subscription), and a mutation names its fields directly (no fragment
//!   spread or inline fragment at its root, so the check reads what runs);
//! - no field anywhere in the document may be secret-named ([`secret_name`]:
//!   secret, token, password, a `pass` word, credential, authorization, a name
//!   ending in Key), whatever its alias, so an alias can't rename a secret past
//!   the masking;
//! - a mutation may not name a team other than Engineering: not in its
//!   inline arguments, not in any variable (`teamId`, `teamIds`, `team`,
//!   `...TeamId` and the like), and `teamCreate`, `teamUpdate`,
//!   `teamDelete`, `teamUnarchive` and `teamCyclesDelete` are refused
//!   unless their `id` is the AI team; a team update's `parentId` must be AI
//!   or HOL too, and its input's other ids are read or refused like any
//!   field's. `issueLabelCreate` with no allowed `teamId` (a workspace label)
//!   is refused (2026-11-13). Any root field naming Create or Upsert not in
//!   [`TEAM_CREATES`] is refused (an allowlist, R3: initiative, customer,
//!   project status or label, emoji, roadmap and the rest are workspace-wide);
//!   `templateCreate` and `customViewCreate` also need their own top-level
//!   `input.teamId` of AI or HOL (one nested deeper does not count). A
//!   `projectUpdate` that sets `teamIds` to an empty list is refused
//!   (2026-11-14, R3);
//! - **reads** (a query, and a mutation's selection set) may not name a team
//!   other than AI or HOL either ([`check_read_teams`], R3): any argument key
//!   containing `team` at any depth (`team`, `teamId`, `accessibleTeams`, ...),
//!   `team(id:)`, and a `teams(filter:)` (only `id` and `key`, inline or by
//!   variable), with a literal or a variable, must hold only allowed teams, and
//!   only the operators `id`, `key`, `eq`, `in` and `some` (a `neq`, `or` or
//!   `name` beside them would select other teams). A query variable whose
//!   default is a string, object or list literal must be passed explicitly.
//!   A read naming no team, or one acting on an issue by id, is not
//!   team-filtered (an unfiltered `issues` still returns every team the key
//!   sees; recorded);
//! - a mutation that acts on an existing issue, comment, attachment or
//!   workflow state (its `id`, `ids`, or an `issueId`, `issueIds`,
//!   `relatedIssueId`, `commentId` or `attachmentId` anywhere in its arguments
//!   or variables; an issue mutation's `parentId`) has that entity's team read
//!   first with one `HollowTeamGuard` query, and anything not in the AI team,
//!   or not found, is refused (2026-11-12). Projects, project milestones,
//!   labels and cycles (their `id`, or a `projectId`, `projectMilestoneId`,
//!   `cycleId`, `labelId(s)` or a label's `parentId`) are read the same way:
//!   a project spanning any other team, or a workspace label, is refused
//!   (2026-11-13), an issue's own `projectId` and `projectMilestoneId`
//!   included. Any other `id`, `ids` or `...Id`/`...Ids` key it can't read
//!   (a document, a customer, a reaction's `projectUpdateId`) is refused before
//!   any card, since an approval can't override the team rule; bar
//!   [`ISSUE_ATTRIBUTE_IDS`] on an issue whose team is known,
//!   [`PROJECT_ATTRIBUTE_IDS`] on a project whose teams are, and the people in
//!   [`PERSON_IDS`];
//! - `viewer { organization { id urlKey name } }` must name Acme
//!   Mobility's id ([`ORG_ID`]), asked again before every call (one cheap
//!   request; a swapped key can't slip through a cache). Only the id is
//!   pinned (#1301, 2026-11-13): any organization can call itself Acme
//!   Mobility, and an admin can change its urlKey, after which another
//!   organization could take the old one, so neither the name nor the urlKey
//!   is accepted on its own. Both are shown in a refusal;
//! - a mutation named Delete, Archive, Remove, Trash, Retire, Suspend, Demote,
//!   Merge, Rotate, Revoke or Disconnect (or setting `trashed`), one that grants
//!   access or reaches outside the organization (a root field starting with
//!   organization, user, webhook, apiKey or oauth, or containing Invite,
//!   Integration or Membership) ask on a card in every mode ([`asks_always`], used by `app_actions::asks_in_mode`).
//!   Every other call that passes these guards asks no card (T1, 2026-11-15: the
//!   action is Write tier and its card builder returns none).
//!
//! The key goes only into the adapter's `Authorization` header. The result
//! and every audit row hold no key (its text is removed, `lin_*` tokens and
//! secret-named fields are masked). Each call, refused or sent, is one
//! `tracker_write` audit row (action `graphql`) with the query text (secret
//! string literals masked, 2,000 characters), the operation, its root fields
//! and the scrubbed variables. A rate-limited reply, or one that
//! reports no requests left, closes a per-endpoint window until Linear's
//! reset; calls inside it are refused without a request.

#[cfg(test)]
use super::linear::TEAM_ID;
use super::linear::{allowed_team, team_allowed, LinearAdapter, PERSONAL_TEAM_ID, TEAM_ONLY};
use super::TrackerError;
use chrono::{DateTime, Duration, Utc};
use once_cell::sync::Lazy;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

/// The organization the key must belong to, by name (shown on cards and
/// in refusals; not what is checked).
pub(crate) const ORG_NAME: &str = "Example Org";
/// Example Org's Linear organization id, the guard's only accepted pin
/// (read from Linear 2026-11-13; its urlKey then was `example-org`).
pub(crate) const ORG_ID: &str = "00000000-0000-4000-8000-000000000001";
const MAX_QUERY: usize = 100_000;
const MAX_VARIABLES: usize = 200_000;
pub(super) const ORG_QUERY: &str =
    "query HollowOrgGuard { viewer { organization { id urlKey name } } }";
/// How long a rate-limit window lasts when Linear names no reset.
const DEFAULT_WINDOW: i64 = 60;
/// Words that make a mutation ask on a card in every mode.
const WORDS: &[&str] = &[
    "delete",
    "archive",
    "remove",
    "trash",
    "retire",
    "suspend",
    "demote",
    "merge",
    "rotate",
    "revoke",
    "disconnect",
];
const MASK: &str = "••••";
/// Team mutations that take the team's id as `id`.
const TEAM_BY_ID: &[&str] = &[
    "teamUpdate",
    "teamDelete",
    "teamUnarchive",
    "teamCyclesDelete",
];
/// Root-field prefixes (lowercased) of mutations that grant access or reach
/// outside the organization: they ask on a card in every mode.
const ACCESS_PREFIXES: &[&str] = &["organization", "user", "webhook", "apikey", "oauth"];
/// Words (lowercased) that do the same anywhere in a root field's name.
const ACCESS_WORDS: &[&str] = &["invite", "integration", "membership"];
/// Mutations whose `id` (or, for `issueBatchUpdate`, `ids`) is an issue.
const ISSUE_BY_ID: &[&str] = &[
    "issueUpdate",
    "issueArchive",
    "issueUnarchive",
    "issueDelete",
    "issueAddLabel",
    "issueRemoveLabel",
    "issueSubscribe",
    "issueUnsubscribe",
    "issueReminder",
    "issueDescriptionUpdateFromFront",
    "issueExternalSyncDisable",
    "issueBatchUpdate",
];
/// Mutations whose `id` is a comment.
const COMMENT_BY_ID: &[&str] = &[
    "commentUpdate",
    "commentDelete",
    "commentResolve",
    "commentUnresolve",
];
/// Mutations whose `id` is an attachment.
const ATTACHMENT_BY_ID: &[&str] = &[
    "attachmentUpdate",
    "attachmentDelete",
    "attachmentSyncToSlack",
];
/// Mutations whose `id` is a workflow state.
const STATE_BY_ID: &[&str] = &["workflowStateUpdate", "workflowStateArchive"];
/// Issue mutations whose `parentId` is an issue (a label's is a label group).
const ISSUE_PARENT: &[&str] = &["issueCreate", "issueUpdate", "issueBatchUpdate"];
/// Mutations whose `id` is a project.
const PROJECT_BY_ID: &[&str] = &[
    "projectUpdate",
    "projectArchive",
    "projectUnarchive",
    "projectDelete",
];
/// Mutations whose `id` is a project milestone.
const MILESTONE_BY_ID: &[&str] = &["projectMilestoneUpdate", "projectMilestoneDelete"];
/// Mutations whose `id` is an issue label.
const LABEL_BY_ID: &[&str] = &[
    "issueLabelUpdate",
    "issueLabelDelete",
    "issueLabelRetire",
    "issueLabelRestore",
];
/// Mutations whose `id` is a cycle.
const CYCLE_BY_ID: &[&str] = &["cycleUpdate", "cycleArchive"];
/// Label mutations whose `parentId` is a label group (a label).
const LABEL_PARENT: &[&str] = &["issueLabelCreate", "issueLabelUpdate"];
/// Keys naming people, who belong to no team: they point at someone, they
/// don't write a team's data, so they need no team read.
const PERSON_IDS: &[&str] = &[
    "assigneeId",
    "delegateId",
    "leadId",
    "memberIds",
    "subscriberIds",
];
/// The most entities one call may act on by id.
const MAX_TARGETS: usize = 50;
/// The only `...Create` root fields a mutation may use (R3, inverted from a
/// denylist 2026-11-14): each is team-scoped or read through a team-checked id,
/// so the guards below see what it writes. Any other `...Create` (an
/// initiative, a customer, a project status or label, an emoji, a roadmap) makes
/// a workspace-wide object and is refused. Creates that ask on a card as access
/// ([`Operation::access`]: webhook, apiKey, oauth, invite, membership) and
/// `teamCreate` (its own refusal) are not judged here.
const TEAM_CREATES: &[&str] = &[
    "issueCreate",
    "issueBatchCreate",
    "issueLabelCreate",
    "issueRelationCreate",
    "commentCreate",
    "attachmentCreate",
    "entityExternalLinkCreate",
    "projectCreate",
    "projectMilestoneCreate",
    "projectUpdateCreate",
    "cycleCreate",
    "workflowStateCreate",
    "templateCreate",
    "customViewCreate",
];
/// Of [`TEAM_CREATES`], those that need their own top-level `input.teamId`
/// (AI or HOL): with none they would be workspace-wide.
const TEAM_SCOPABLE_CREATES: &[&str] = &["templateCreate", "customViewCreate"];
/// How much of the query text the audit row keeps.
const AUDIT_QUERY: usize = 2_000;

// ── Lexer and parser ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Name(String),
    Var(String),
    Str(String),
    Punct(char),
    Spread,
    /// A number, kept as written.
    Other(String),
}

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            c if c.is_whitespace() || c == ',' || c == '\u{feff}' => i += 1,
            '#' => {
                while i < chars.len() && chars[i] != '\n' && chars[i] != '\r' {
                    i += 1;
                }
            }
            '"' if chars[i..].starts_with(&['"', '"', '"']) => {
                i += 3;
                let mut text = String::new();
                loop {
                    if i >= chars.len() {
                        return Err("the query has an unterminated block string".into());
                    }
                    if chars[i..].starts_with(&['\\', '"', '"', '"']) {
                        text.push_str("\"\"\"");
                        i += 4;
                    } else if chars[i..].starts_with(&['"', '"', '"']) {
                        i += 3;
                        break;
                    } else {
                        text.push(chars[i]);
                        i += 1;
                    }
                }
                out.push(Tok::Str(text));
            }
            '"' => {
                i += 1;
                let mut text = String::new();
                loop {
                    let Some(&d) = chars.get(i) else {
                        return Err("the query has an unterminated string".into());
                    };
                    i += 1;
                    match d {
                        '"' => break,
                        '\n' | '\r' => return Err("the query has an unterminated string".into()),
                        '\\' => {
                            let Some(&e) = chars.get(i) else {
                                return Err("the query has an unterminated string".into());
                            };
                            i += 1;
                            match e {
                                'n' => text.push('\n'),
                                't' => text.push('\t'),
                                'r' => text.push('\r'),
                                'b' => text.push('\u{8}'),
                                'f' => text.push('\u{c}'),
                                'u' => {
                                    let hex: String = chars.iter().skip(i).take(4).collect();
                                    let code = u32::from_str_radix(&hex, 16).map_err(|_| {
                                        "the query has a bad \\u escape".to_string()
                                    })?;
                                    text.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                                    i += 4;
                                }
                                other => text.push(other),
                            }
                        }
                        other => text.push(other),
                    }
                }
                out.push(Tok::Str(text));
            }
            '.' => {
                if chars[i..].starts_with(&['.', '.', '.']) {
                    out.push(Tok::Spread);
                    i += 3;
                } else {
                    return Err("the query has a stray `.`".into());
                }
            }
            '$' => {
                i += 1;
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                if start == i {
                    return Err("the query has a `$` with no variable name".into());
                }
                out.push(Tok::Var(chars[start..i].iter().collect()));
            }
            '{' | '}' | '(' | ')' | '[' | ']' | ':' | '=' | '!' | '@' | '&' | '|' => {
                out.push(Tok::Punct(c));
                i += 1;
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                out.push(Tok::Name(chars[start..i].iter().collect()));
            }
            c if c.is_ascii_digit() || c == '-' => {
                let start = i;
                while i < chars.len()
                    && (chars[i].is_ascii_alphanumeric()
                        || matches!(chars[i], '.' | '+' | '-' | '_'))
                {
                    i += 1;
                }
                out.push(Tok::Other(chars[start..i].iter().collect()));
            }
            other => return Err(format!("the query has an unexpected character `{other}`")),
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpKind {
    Query,
    Mutation,
    Subscription,
}

impl OpKind {
    fn word(self) -> &'static str {
        match self {
            OpKind::Query => "query",
            OpKind::Mutation => "mutation",
            OpKind::Subscription => "subscription",
        }
    }
}

#[derive(Debug, Clone)]
struct RootField {
    name: String,
    args: Vec<Tok>,
}

#[derive(Debug, Clone)]
pub(crate) struct Operation {
    kind: OpKind,
    name: Option<String>,
    fields: Vec<RootField>,
    has_spread: bool,
    /// A variable definition has a default (`$t: ID = "…"`), which the team
    /// guard can't see in `variables`.
    has_defaults: bool,
    /// Variables whose default holds a string or an object or list literal.
    risky_defaults: Vec<String>,
    /// Every field with arguments anywhere in the document (nested and in
    /// fragments), for the read team guard; the root fields are among them.
    nested: Vec<RootField>,
    /// The first secret-named field selected anywhere in the document
    /// (fragments included), by its real name, not its alias.
    secret_field: Option<String>,
}

struct Cursor<'a> {
    toks: &'a [Tok],
    i: usize,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i)
    }

    fn is(&self, c: char) -> bool {
        self.peek() == Some(&Tok::Punct(c))
    }

    /// From an opening bracket through its match; the tokens between.
    fn balanced(&mut self, open: char, close: char) -> Result<Vec<Tok>, String> {
        if !self.is(open) {
            return Err(format!("the query is malformed: expected `{open}`"));
        }
        self.i += 1;
        let start = self.i;
        let mut depth = 1;
        while let Some(tok) = self.toks.get(self.i) {
            match tok {
                Tok::Punct(c) if *c == open => depth += 1,
                Tok::Punct(c) if *c == close => {
                    depth -= 1;
                    if depth == 0 {
                        let inner = self.toks[start..self.i].to_vec();
                        self.i += 1;
                        return Ok(inner);
                    }
                }
                _ => {}
            }
            self.i += 1;
        }
        Err(format!("the query is malformed: `{open}` is never closed"))
    }

    fn directives(&mut self) -> Result<(), String> {
        while self.is('@') {
            self.i += 1;
            if !matches!(self.peek(), Some(Tok::Name(_))) {
                return Err("the query is malformed: a directive has no name".into());
            }
            self.i += 1;
            if self.is('(') {
                self.balanced('(', ')')?;
            }
        }
        Ok(())
    }

    /// The root fields of the selection set at the cursor.
    fn selection(&mut self, op: &mut Operation) -> Result<(), String> {
        if !self.is('{') {
            return Err("the query is malformed: expected `{`".into());
        }
        self.i += 1;
        loop {
            match self.peek().cloned() {
                None => return Err("the query is malformed: `{` is never closed".into()),
                Some(Tok::Punct('}')) => {
                    self.i += 1;
                    return Ok(());
                }
                Some(Tok::Spread) => {
                    op.has_spread = true;
                    self.i += 1;
                    if self.peek() == Some(&Tok::Name("on".into())) {
                        self.i += 2;
                    } else {
                        self.i += 1;
                    }
                    self.directives()?;
                    if self.is('{') {
                        self.balanced('{', '}')?;
                    }
                }
                Some(Tok::Name(first)) => {
                    self.i += 1;
                    let mut name = first;
                    if self.is(':') {
                        self.i += 1;
                        let Some(Tok::Name(real)) = self.peek().cloned() else {
                            return Err("the query is malformed: an alias has no field".into());
                        };
                        self.i += 1;
                        name = real;
                    }
                    let args = if self.is('(') {
                        self.balanced('(', ')')?
                    } else {
                        Vec::new()
                    };
                    self.directives()?;
                    if self.is('{') {
                        self.balanced('{', '}')?;
                    }
                    op.fields.push(RootField { name, args });
                }
                Some(_) => return Err("the query is malformed inside a selection".into()),
            }
        }
    }
}

/// The one operation in `src`. More or fewer than one is refused.
pub(crate) fn parse(src: &str) -> Result<Operation, String> {
    let toks = lex(src)?;
    let mut cur = Cursor { toks: &toks, i: 0 };
    let mut found: Option<Operation> = None;
    while let Some(tok) = cur.peek().cloned() {
        let mut op = Operation {
            kind: OpKind::Query,
            name: None,
            fields: Vec::new(),
            has_spread: false,
            has_defaults: false,
            risky_defaults: Vec::new(),
            nested: Vec::new(),
            secret_field: None,
        };
        match tok {
            Tok::Punct('{') => {}
            Tok::Name(word) if word == "fragment" => {
                cur.i += 1;
                while !cur.is('{') && !cur.is('@') && cur.peek().is_some() {
                    cur.i += 1;
                }
                cur.directives()?;
                cur.balanced('{', '}')?;
                continue;
            }
            Tok::Name(word) if matches!(word.as_str(), "query" | "mutation" | "subscription") => {
                op.kind = match word.as_str() {
                    "mutation" => OpKind::Mutation,
                    "subscription" => OpKind::Subscription,
                    _ => OpKind::Query,
                };
                cur.i += 1;
                if let Some(Tok::Name(name)) = cur.peek().cloned() {
                    op.name = Some(name);
                    cur.i += 1;
                }
                if cur.is('(') {
                    let defs = cur.balanced('(', ')')?;
                    op.has_defaults = defs.contains(&Tok::Punct('='));
                    let (mut cur_var, mut in_default) = (None::<String>, false);
                    for tok in &defs {
                        match tok {
                            Tok::Var(v) => (cur_var, in_default) = (Some(v.clone()), false),
                            Tok::Punct('=') => in_default = true,
                            Tok::Str(_) | Tok::Punct('{' | '[') if in_default => {
                                if let Some(v) = cur_var.take() {
                                    op.risky_defaults.push(v);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                cur.directives()?;
            }
            _ => return Err("the query must hold one query or mutation".into()),
        }
        cur.selection(&mut op)?;
        if found.replace(op).is_some() {
            return Err("send one operation per call (this query holds more than one)".into());
        }
    }
    let mut op: Operation = found.ok_or("the query holds no operation")?;
    op.secret_field = selected_secret(&toks);
    op.nested = argument_fields(&toks);
    Ok(op)
}

/// Every `name(...)` in the document: a field with arguments at any depth
/// (an operation's variable definitions and a directive's arguments come out
/// too; they hold no team-named key, so they pass).
fn argument_fields(toks: &[Tok]) -> Vec<RootField> {
    let mut out = Vec::new();
    for (n, tok) in toks.iter().enumerate() {
        let (Tok::Name(name), Some(Tok::Punct('('))) = (tok, toks.get(n + 1)) else {
            continue;
        };
        let mut depth = 1;
        let mut args = Vec::new();
        for t in &toks[n + 2..] {
            match t {
                Tok::Punct('(') => depth += 1,
                Tok::Punct(')') => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                break;
            }
            args.push(t.clone());
        }
        out.push(RootField {
            name: name.clone(),
            args,
        });
    }
    out
}

/// The first secret-named field any selection set in the document picks,
/// by its real name: an alias (`s: secret`) names `secret`. A field is a name
/// inside braces and outside arguments that is not an alias, a directive, a
/// fragment's name or a type condition.
fn selected_secret(toks: &[Tok]) -> Option<String> {
    let (mut braces, mut parens) = (0i32, 0i32);
    for (n, tok) in toks.iter().enumerate() {
        match tok {
            Tok::Punct('(') => parens += 1,
            Tok::Punct(')') => parens -= 1,
            Tok::Punct('{') if parens == 0 => braces += 1,
            Tok::Punct('}') if parens == 0 => braces -= 1,
            Tok::Name(name) if parens == 0 && braces > 0 => {
                let before = n.checked_sub(1).and_then(|b| toks.get(b));
                let skip = matches!(before, Some(Tok::Punct('@') | Tok::Spread))
                    || (name == "on" && before == Some(&Tok::Spread))
                    || (matches!(before, Some(Tok::Name(on)) if on == "on")
                        && n >= 2
                        && toks.get(n - 2) == Some(&Tok::Spread))
                    || toks.get(n + 1) == Some(&Tok::Punct(':'));
                if !skip && secret_name(name) {
                    return Some(name.clone());
                }
            }
            _ => {}
        }
    }
    None
}

// ── Guards ──────────────────────────────────────────────────────────────────

/// A field listing teams, whose `filter` is held to ids and keys.
fn teams_field(name: &str) -> bool {
    matches!(name, "teams" | "administrableTeams")
}

/// A root field that makes an object: its name holds `Create` or `Upsert`
/// (any case), so `customerUpsert` is judged like `customerCreate`.
fn makes_something(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("create") || lower.contains("upsert")
}

/// A root field that grants access or reaches outside the organization.
fn access_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    ACCESS_PREFIXES.iter().any(|p| lower.starts_with(p))
        || ACCESS_WORDS.iter().any(|w| lower.contains(w))
}

impl Operation {
    /// A mutation that deletes, archives, removes or otherwise takes
    /// something away or changes who has access: Delete, Archive, Remove,
    /// Trash, Retire, Suspend, Demote or Merge in its operation name or a root
    /// field, or `trashed` / `archivedAt` set in its inline arguments or its
    /// variables (`issueUpdate(input: {trashed: true})`).
    fn destructive(&self, variables: &Value) -> bool {
        let hit = |text: &str| {
            let lower = text.to_ascii_lowercase();
            WORDS.iter().any(|w| lower.contains(w))
        };
        let flag = |key: &str| matches!(key, "trashed" | "archivedAt");
        self.kind == OpKind::Mutation
            && (self.name.as_deref().is_some_and(hit)
                || self.fields.iter().any(|f| {
                    hit(&f.name) || f.args.iter().any(|t| matches!(t, Tok::Name(n) if flag(n)))
                })
                || names_key(variables, &flag))
    }

    /// A mutation that grants access or reaches outside the organization: a
    /// root field starting with organization, user, webhook, apiKey or oauth
    /// (`organizationInviteCreate`, `userPromoteAdmin`, `webhookCreate`, which
    /// with `allPublicTeams` would stream every public team out), or
    /// containing Invite, Integration or Membership.
    fn access(&self) -> bool {
        self.kind == OpKind::Mutation && self.fields.iter().any(|f| access_name(&f.name))
    }

    /// Whether this call asks on a card in every mode. A by-id target whose
    /// team Hollow can't read is not a reason: [`preflight`] refuses it, and a
    /// card can't approve it past the team rule.
    fn asks(&self, variables: &Value) -> bool {
        self.destructive(variables) || self.access()
    }

    /// What the audit row and the card call it: the operation's name, else
    /// its root fields.
    fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            self.fields
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
    }
}

/// Whether this call asks on a card in every mode: a destructive mutation
/// ([`Operation::destructive`]) or one that grants access or reaches outside
/// the organization ([`Operation::access`]). One acting by id on something
/// whose team Hollow can't read ([`targets`]) doesn't ask: it is refused
/// before any card, and every target it can read has its team confirmed
/// before sending, whatever the card said. A query that doesn't parse is
/// refused when it runs, so it doesn't ask.
pub(crate) fn asks_always(query: &str, variables: &Value) -> bool {
    parse(query).is_ok_and(|op| op.asks(variables))
}

/// Whether any object in `value` has a key `flag` accepts.
fn names_key(value: &Value, flag: &dyn Fn(&str) -> bool) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(k, v)| flag(k) || names_key(v, flag)),
        Value::Array(items) => items.iter().any(|v| names_key(v, flag)),
        _ => false,
    }
}

fn team_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    matches!(lower.as_str(), "team" | "teams" | "teamkey" | "teamkeys")
        || lower.ends_with("teamid")
        || lower.ends_with("teamids")
}

/// A team id or key of one of the two teams Hollow may touch (AI and HOL).
fn ours(text: &str) -> bool {
    allowed_team(text)
        .is_some_and(|id| PERSONAL_ALLOWED.with(|p| p.get()) || id != PERSONAL_TEAM_ID)
}

thread_local! {
    /// Whether the check running on this thread may name the personal team
    /// (set by [`preflight_with`] around its synchronous checks; closed by
    /// default, so any check run outside it refuses the personal team).
    static PERSONAL_ALLOWED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A value that names a team: null, our team's id or key, a list of them,
/// or an object whose `id`/`key` do.
fn team_value(value: &Value) -> Result<(), String> {
    match value {
        Value::Null => Ok(()),
        // A flag (`includeSubTeams: true`) names no team.
        Value::Bool(_) => Ok(()),
        Value::String(text) if ours(text) => Ok(()),
        Value::Array(items) => items.iter().try_for_each(team_value),
        Value::Object(map) => {
            // Only these keys: an `or`, `neq` or `name` beside them could
            // select other teams.
            if map.is_empty()
                || !map
                    .keys()
                    .all(|k| matches!(k.as_str(), "id" | "key" | "eq" | "in" | "some"))
            {
                return Err(TEAM_ONLY.into());
            }
            map.values().try_for_each(team_value)
        }
        _ => Err(TEAM_ONLY.into()),
    }
}

/// A query reads any key naming a team, accessibleTeams included (R3).
fn team_read_key(key: &str) -> bool {
    team_key(key) || key.to_ascii_lowercase().contains("team")
}

fn walk_variables_by(value: &Value, named: &dyn Fn(&str) -> bool) -> Result<(), String> {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                if named(key) {
                    team_value(inner)?;
                }
                walk_variables_by(inner, named)?;
            }
            Ok(())
        }
        Value::Array(items) => items.iter().try_for_each(|v| walk_variables_by(v, named)),
        _ => Ok(()),
    }
}

/// The value tokens right after a `:`: one literal or variable, or a flat list.
fn literal_values(args: &[Tok], at: usize, variables: &Value) -> Result<(), String> {
    let one = |tok: &Tok| -> Result<(), String> {
        match tok {
            Tok::Str(text) if ours(text) => Ok(()),
            Tok::Var(name) => team_value(variables.get(name).unwrap_or(&Value::Null)),
            Tok::Name(word) if word == "null" => Ok(()),
            _ => Err(TEAM_ONLY.into()),
        }
    };
    match args.get(at) {
        Some(Tok::Punct('[')) => {
            for tok in &args[at + 1..] {
                if tok == &Tok::Punct(']') {
                    return Ok(());
                }
                one(tok)?;
            }
            Err(TEAM_ONLY.into())
        }
        Some(Tok::Punct('{')) => Err(TEAM_ONLY.into()),
        Some(tok) => one(tok),
        None => Err(TEAM_ONLY.into()),
    }
}

/// A document's team guard for reads (R3): see the module docs. Every
/// argument list in the document is read, so a nested `teams(filter: ...)` is
/// held too, and a mutation's selection set is held like a query's.
fn check_read_teams(op: &Operation, variables: &Value, vars: bool) -> Result<(), String> {
    let refuse =
        || format!("a read may not name a team other than Engineering or Personal. {TEAM_ONLY}");
    if vars {
        walk_variables_by(variables, &team_read_key).map_err(|_| refuse())?;
        // A default that would stand in for a missing variable can carry a
        // team the guard never sees.
        if let Some(name) = op
            .risky_defaults
            .iter()
            .find(|n| variables.get(n.as_str()).is_none())
        {
            return Err(format!(
                "variable ${name} has a default that holds a literal; pass the value in variables. {TEAM_ONLY}"
            ));
        }
    }
    for field in &op.nested {
        for (n, tok) in field.args.iter().enumerate() {
            let Tok::Name(key) = tok else { continue };
            if field.args.get(n + 1) != Some(&Tok::Punct(':')) {
                continue;
            }
            let by_field = match field.name.as_str() {
                "team" => key == "id",
                n if teams_field(n) => matches!(key.as_str(), "id" | "key"),
                _ => false,
            };
            if team_read_key(key) || by_field {
                read_value(&field.args, n + 2, variables).map_err(|_| refuse())?;
            }
            // `teams(filter:)`: only id and key, with eq or in, by literal or variable.
            if teams_field(&field.name) && key == "filter" {
                match field.args.get(n + 2) {
                    Some(Tok::Var(v)) => match variables.get(v) {
                        None | Some(Value::Null) => {}
                        Some(f @ Value::Object(m)) => {
                            if !m.keys().all(|k| matches!(k.as_str(), "id" | "key")) {
                                return Err(refuse());
                            }
                            team_value(f).map_err(|_| refuse())?;
                        }
                        Some(_) => return Err(refuse()),
                    },
                    Some(Tok::Punct('{')) => {
                        read_value(&field.args, n + 2, variables).map_err(|_| refuse())?
                    }
                    _ => return Err(refuse()),
                }
            }
        }
    }
    Ok(())
}

/// A team-named key's value in a document: literals and variables naming
/// allowed teams, a list of them, or an object using only `id`, `key`, `eq`,
/// `in` and `some` over them.
fn read_value(args: &[Tok], at: usize, variables: &Value) -> Result<(), String> {
    fn tokens(inner: &[Tok], variables: &Value) -> Result<(), String> {
        for tok in inner {
            match tok {
                Tok::Str(text) if ours(text) => {}
                Tok::Var(name) => team_value(variables.get(name).unwrap_or(&Value::Null))?,
                Tok::Name(w)
                    if matches!(
                        w.as_str(),
                        "id" | "key" | "eq" | "in" | "some" | "null" | "true" | "false"
                    ) => {}
                Tok::Punct('{' | '}' | '[' | ']' | ':') => {}
                _ => return Err(TEAM_ONLY.into()),
            }
        }
        Ok(())
    }
    let (open, close) = match args.get(at) {
        Some(Tok::Punct('[')) => ('[', ']'),
        Some(Tok::Punct('{')) => ('{', '}'),
        Some(_) => return tokens(&args[at..=at], variables),
        None => return Err(TEAM_ONLY.into()),
    };
    let mut depth = 0;
    for (n, tok) in args[at..].iter().enumerate() {
        match tok {
            Tok::Punct(c) if *c == open => depth += 1,
            Tok::Punct(c) if *c == close => {
                depth -= 1;
                if depth == 0 {
                    return tokens(&args[at..=at + n], variables);
                }
            }
            _ => {}
        }
    }
    Err(TEAM_ONLY.into())
}

/// Whether the field's own top-level `input.teamId` (inline, or in the
/// variable that is the whole input, or a variable set as the `teamId`) is an
/// allowed team. A `teamId` nested deeper (`templateData`, `filters`) does not
/// count.
fn input_team_ok(field: &RootField, variables: &Value) -> bool {
    let Some(at) = top_level(&field.args, "input") else {
        return false;
    };
    let text_ok = |v: Option<&Value>| v.and_then(Value::as_str).is_some_and(ours);
    match field.args.get(at) {
        Some(Tok::Var(v)) => text_ok(variables.get(v).and_then(|i| i.get("teamId"))),
        Some(Tok::Punct('{')) => {
            let mut depth = 0;
            for (n, tok) in field.args.iter().enumerate().skip(at) {
                match tok {
                    Tok::Punct('{' | '[') => depth += 1,
                    Tok::Punct('}' | ']') => depth -= 1,
                    Tok::Name(k)
                        if depth == 1
                            && k == "teamId"
                            && field.args.get(n + 1) == Some(&Tok::Punct(':')) =>
                    {
                        return match field.args.get(n + 2) {
                            Some(Tok::Str(t)) => ours(t),
                            Some(Tok::Var(v)) => text_ok(variables.get(v)),
                            _ => false,
                        };
                    }
                    _ => {}
                }
                if depth == 0 {
                    break;
                }
            }
            false
        }
        _ => false,
    }
}

/// Whether a `teamIds` this field sets (inline, or at any depth of a variable
/// it uses) is an empty list or null: on a project update that would take
/// the project out of every team.
fn clears_team_ids(field: &RootField, variables: &Value) -> bool {
    fn cleared(value: &Value) -> bool {
        match value {
            Value::Object(map) => map.iter().any(|(k, v)| {
                let empty = v.is_null() || v.as_array().is_some_and(Vec::is_empty);
                (k == "teamIds" && empty) || cleared(v)
            }),
            Value::Array(items) => items.iter().any(cleared),
            _ => false,
        }
    }
    field.args.iter().enumerate().any(|(n, tok)| match tok {
        Tok::Name(key) if key == "teamIds" && field.args.get(n + 1) == Some(&Tok::Punct(':')) => {
            match field.args.get(n + 2) {
                Some(Tok::Punct('[')) => field.args.get(n + 3) == Some(&Tok::Punct(']')),
                Some(Tok::Name(w)) => w == "null",
                Some(Tok::Var(v)) => variables
                    .get(v)
                    .is_none_or(|x| x.is_null() || x.as_array().is_some_and(Vec::is_empty)),
                _ => false,
            }
        }
        Tok::Var(v) => variables.get(v).is_some_and(cleared),
        _ => false,
    })
}

fn check_teams(op: &Operation, variables: &Value) -> Result<(), String> {
    if op.kind != OpKind::Mutation {
        return check_read_teams(op, variables, true);
    }
    walk_variables_by(variables, &team_read_key)?;
    check_read_teams(op, variables, false)?;
    for field in &op.fields {
        if field.name == "teamCreate" {
            return Err(format!("teamCreate is refused. {TEAM_ONLY}"));
        }
        if TEAM_BY_ID.contains(&field.name.as_str()) {
            let id_at = top_level(&field.args, "id");
            let ok = id_at.is_some_and(|at| match field.args.get(at) {
                Some(Tok::Str(text)) => ours(text) && team_allowed(text),
                Some(Tok::Var(name)) => variables
                    .get(name)
                    .and_then(Value::as_str)
                    .is_some_and(|t| ours(t) && team_allowed(t)),
                _ => false,
            });
            if !ok {
                return Err(format!(
                    "{} needs the id of team Engineering or Personal. {TEAM_ONLY}",
                    field.name
                ));
            }
            team_parent(field, variables)?;
        }
        if field.name == "issueLabelCreate" && !names_team(field, variables) {
            return Err(format!(
                "issueLabelCreate with no teamId of team Engineering or Personal would create a workspace label (no team, shared across teams), so it is refused. {TEAM_ONLY}"
            ));
        }
        for (n, tok) in field.args.iter().enumerate() {
            if let Tok::Name(key) = tok {
                if team_key(key) && field.args.get(n + 1) == Some(&Tok::Punct(':')) {
                    literal_values(&field.args, n + 2, variables)?;
                }
            }
        }
        let name = field.name.as_str();
        let creates = makes_something(name) && name != "teamCreate" && !access_name(name);
        if (creates && !TEAM_CREATES.contains(&name))
            || (TEAM_SCOPABLE_CREATES.contains(&name) && !input_team_ok(field, variables))
        {
            return Err(format!(
                "{name} would create a workspace-wide object (no team, shared across teams), so it is refused. {TEAM_ONLY}"
            ));
        }
        if name == "projectUpdate" && clears_team_ids(field, variables) {
            return Err(format!(
                "projectUpdate with an empty teamIds would take the project out of every team, so it is refused. {TEAM_ONLY}"
            ));
        }
    }
    Ok(())
}

/// A team mutation's `parentId`, inline or at any depth of a variable it
/// uses: the team it would sit under, which must be AI or HOL itself, by id
/// (2026-11-13). Null or anything else is refused.
fn team_parent(field: &RootField, variables: &Value) -> Result<(), String> {
    fn allowed(value: Option<&Value>) -> bool {
        value
            .and_then(Value::as_str)
            .is_some_and(|t| ours(t) && team_allowed(t))
    }
    fn parents_ok(value: &Value) -> bool {
        match value {
            Value::Object(map) => map
                .iter()
                .all(|(k, v)| (k != "parentId" || allowed(Some(v))) && parents_ok(v)),
            Value::Array(items) => items.iter().all(parents_ok),
            _ => true,
        }
    }
    let ok = field.args.iter().enumerate().all(|(n, tok)| match tok {
        Tok::Name(key) if key == "parentId" && field.args.get(n + 1) == Some(&Tok::Punct(':')) => {
            match field.args.get(n + 2) {
                Some(Tok::Str(text)) => ours(text) && team_allowed(text),
                Some(Tok::Var(var)) => allowed(variables.get(var)),
                _ => false,
            }
        }
        Tok::Var(var) => variables.get(var).is_none_or(parents_ok),
        _ => true,
    });
    if ok {
        Ok(())
    } else {
        Err(format!(
            "{}'s parentId must be the id of team Engineering or Personal. {TEAM_ONLY}",
            field.name
        ))
    }
}

/// Where the value of the field's own (depth 0) argument `name` starts.
fn top_level(args: &[Tok], name: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut at = None;
    for (n, tok) in args.iter().enumerate() {
        match tok {
            Tok::Punct('{' | '[' | '(') => depth += 1,
            Tok::Punct('}' | ']' | ')') => depth -= 1,
            Tok::Name(word)
                if depth == 0 && word == name && args.get(n + 1) == Some(&Tok::Punct(':')) =>
            {
                at = Some(n + 2);
            }
            _ => {}
        }
    }
    at
}

/// What kind of entity an id names, so its team can be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entity {
    Issue,
    Comment,
    Attachment,
    State,
    /// In one or more teams, every one of which must be allowed.
    Project,
    /// Read through its project.
    Milestone,
    /// A team's label; a workspace label (no team) is shared, so refused.
    Label,
    Cycle,
}

impl Entity {
    fn word(self) -> &'static str {
        match self {
            Entity::Issue => "issue",
            Entity::Comment => "comment",
            Entity::Attachment => "attachment",
            Entity::State => "workflow state",
            Entity::Project => "project",
            Entity::Milestone => "project milestone",
            Entity::Label => "label",
            Entity::Cycle => "cycle",
        }
    }

    /// The read field and the selection that reaches its team(s).
    fn read(self) -> (&'static str, &'static str) {
        const TEAMS: &str = "teams { nodes { id key } pageInfo { hasNextPage } }";
        match self {
            Entity::Issue => ("issue", "team { id key }"),
            Entity::Comment => ("comment", "issue { team { id key } }"),
            Entity::Attachment => ("attachment", "issue { team { id key } }"),
            Entity::State => ("workflowState", "team { id key }"),
            Entity::Project => ("project", TEAMS),
            Entity::Milestone => (
                "projectMilestone",
                "project { teams { nodes { id key } pageInfo { hasNextPage } } }",
            ),
            Entity::Label => ("issueLabel", "team { id key }"),
            Entity::Cycle => ("cycle", "team { id key }"),
        }
    }

    /// The teams of the entity read into `node` (the guard's `tN`): `None`
    /// when it wasn't found or its teams can't be read in full; an empty list
    /// when it exists in no team (a workspace label).
    fn teams(self, node: Option<&Value>) -> Option<Vec<&Value>> {
        fn found(v: Option<&Value>) -> Option<&Value> {
            v.filter(|v| v.is_object())
        }
        fn one(owner: &Value) -> Option<Vec<&Value>> {
            match owner.get("team")? {
                Value::Null => Some(Vec::new()),
                team => Some(vec![team]),
            }
        }
        fn many(owner: &Value) -> Option<Vec<&Value>> {
            let teams = owner.get("teams")?;
            if teams.pointer("/pageInfo/hasNextPage") != Some(&Value::Bool(false)) {
                return None;
            }
            Some(teams.get("nodes")?.as_array()?.iter().collect())
        }
        let node = found(node)?;
        match self {
            Entity::Issue | Entity::State | Entity::Label | Entity::Cycle => one(node),
            Entity::Comment | Entity::Attachment => one(found(node.get("issue"))?),
            Entity::Project => many(node),
            Entity::Milestone => many(found(node.get("project"))?),
        }
    }
}

/// An existing entity a mutation acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    pub kind: Entity,
    pub id: String,
}

/// A mutation's by-id targets: the ones Hollow reads the team of before
/// sending, and the root fields that act by id on something it can't (the
/// call is refused).
#[derive(Debug, Default)]
struct Targets {
    found: Vec<Target>,
    unresolved: Vec<String>,
}

impl Targets {
    fn push(&mut self, kind: Entity, id: String) {
        let target = Target { kind, id };
        if !self.found.contains(&target) {
            self.found.push(target);
        }
    }

    fn unresolved(&mut self, field: &str) {
        if !self.unresolved.iter().any(|f| f == field) {
            self.unresolved.push(field.to_string());
        }
    }
}

/// What the field's own `id` (or `ids`) names, when Hollow can read its team.
fn by_id_kind(field: &str, arg: &str) -> Option<Entity> {
    match arg {
        "ids" => (field == "issueBatchUpdate").then_some(Entity::Issue),
        _ if field == "issueBatchUpdate" => None,
        _ if ISSUE_BY_ID.contains(&field) => Some(Entity::Issue),
        _ if COMMENT_BY_ID.contains(&field) => Some(Entity::Comment),
        _ if ATTACHMENT_BY_ID.contains(&field) => Some(Entity::Attachment),
        _ if STATE_BY_ID.contains(&field) => Some(Entity::State),
        _ if PROJECT_BY_ID.contains(&field) => Some(Entity::Project),
        _ if MILESTONE_BY_ID.contains(&field) => Some(Entity::Milestone),
        _ if LABEL_BY_ID.contains(&field) => Some(Entity::Label),
        _ if CYCLE_BY_ID.contains(&field) => Some(Entity::Cycle),
        _ => None,
    }
}

/// What an argument or input key names, anywhere in a field's arguments.
fn ref_kind(field: &str, key: &str) -> Option<Entity> {
    match key {
        "issueId" | "issueIds" | "relatedIssueId" => Some(Entity::Issue),
        "parentId" if ISSUE_PARENT.contains(&field) => Some(Entity::Issue),
        // A reply's parent is a comment, read through its issue.
        "parentId" if matches!(field, "commentCreate" | "commentUpdate") => Some(Entity::Comment),
        "commentId" | "sourceCommentId" => Some(Entity::Comment),
        "stateId" => Some(Entity::State),
        "attachmentId" => Some(Entity::Attachment),
        _ => None,
    }
}

/// What a key that would otherwise be unchecked names, when Hollow can read
/// its team: a project, milestone, label or cycle. Only asked of a key
/// [`unchecked_id`] holds, so an AI issue's own attribute ids still need no
/// read (a workspace label stays usable on it).
fn owned_kind(field: &str, key: &str) -> Option<Entity> {
    match key {
        "projectId" => Some(Entity::Project),
        "projectMilestoneId" => Some(Entity::Milestone),
        "cycleId" => Some(Entity::Cycle),
        "labelId" | "labelIds" | "addedLabelIds" | "removedLabelIds" => Some(Entity::Label),
        "parentId" if LABEL_PARENT.contains(&field) => Some(Entity::Label),
        _ => None,
    }
}

/// The ids a JSON value holds: a string, a list of strings, or null (none).
fn json_ids(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::Null => Some(Vec::new()),
        Value::String(text) => Some(vec![text.clone()]),
        Value::Array(items) => items
            .iter()
            .map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => None,
    }
}

/// The ids at `at` in a field's arguments: a string, a variable, null or a
/// flat list of them. `None` when Hollow can't read them.
fn arg_ids(args: &[Tok], at: usize, variables: &Value) -> Option<Vec<String>> {
    let one = |tok: &Tok| -> Option<Vec<String>> {
        match tok {
            Tok::Str(text) => Some(vec![text.clone()]),
            Tok::Var(name) => json_ids(variables.get(name).unwrap_or(&Value::Null)),
            Tok::Name(word) if word == "null" => Some(Vec::new()),
            _ => None,
        }
    };
    match args.get(at)? {
        Tok::Punct('[') => {
            let mut out = Vec::new();
            for tok in &args[at + 1..] {
                if tok == &Tok::Punct(']') {
                    return Some(out);
                }
                out.extend(one(tok)?);
            }
            None
        }
        tok => one(tok),
    }
}

/// Attribute ids an issue mutation may set without a team read of their own,
/// once the issue's team is known to be AI (its `id` read by the team guard,
/// or `issueCreate`'s `teamId`, which [`check_teams`] holds to AI). Linear
/// itself refuses most of them across teams; the rest only point at
/// something, they don't change it:
/// - `assigneeId`, `delegateId`, `subscriberIds`: people, not a team's data;
/// - `labelIds`, `addedLabelIds`, `removedLabelIds`, `labelId`
///   (`issueAddLabel`/`issueRemoveLabel`'s one label): labels are the team's
///   or the workspace's;
/// - `cycleId`: puts the AI issue in a cycle, which must be the team's;
/// - `templateId`: a template the new issue is filled from.
///
/// Not here: `stateId` and `sourceCommentId` (R3, 2026-11-14): a workflow
/// state and the comment an issue is made from each have their team read
/// like any target, so a state or comment of LP or PRO is refused here, not
/// left to Linear; and an issue's `projectId` and `projectMilestoneId` have their
/// project's teams read like any target (2026-11-13), since Linear may add
/// the issue's team to that project, writing another team's project.
const ISSUE_ATTRIBUTE_IDS: &[&str] = &[
    "assigneeId",
    "delegateId",
    "labelIds",
    "addedLabelIds",
    "removedLabelIds",
    "labelId",
    "cycleId",
    "subscriberIds",
    "templateId",
];

/// Attribute ids a project create or update may set without a team read,
/// once the project's own teams are known to be AI or HOL (its `id` read by
/// the team guard, or `projectCreate`'s `teamIds` naming them as concrete
/// strings, which [`check_teams`] holds to AI and HOL): a project status and
/// project labels are workspace-level settings that only change the
/// confirmed project.
const PROJECT_ATTRIBUTE_IDS: &[&str] = &["statusId", "labelIds"];

/// A key naming some other entity by id (`...Id`, `...Ids`, or a bare `id`
/// or `ids` nested in an input, where it isn't the field's own target) that
/// the team guard doesn't read as an issue, comment or attachment and that
/// names no person ([`PERSON_IDS`]): its team is read when [`owned_kind`]
/// knows it, else the call is refused, unless it is one of `attrs` (the
/// [`ISSUE_ATTRIBUTE_IDS`] of an issue, or the [`PROJECT_ATTRIBUTE_IDS`] of a
/// project, whose team is known) or a team mutation's `parentId`, which
/// [`check_teams`] holds to AI or HOL.
fn unchecked_id(field: &str, key: &str, attrs: &[&str]) -> bool {
    (matches!(key, "id" | "ids") || key.ends_with("Id") || key.ends_with("Ids"))
        && !team_key(key)
        && !PERSON_IDS.contains(&key)
        && ref_kind(field, key).is_none()
        && !attrs.contains(&key)
        && !(key == "parentId" && TEAM_BY_ID.contains(&field))
}

/// Reference keys inside a variable's value, at any depth.
fn var_refs(field: &str, value: &Value, attrs: &[&str], out: &mut Targets) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                if let Some(kind) = ref_kind(field, key) {
                    match json_ids(inner) {
                        Some(ids) => ids.into_iter().for_each(|id| out.push(kind, id)),
                        None => out.unresolved(field),
                    }
                } else if unchecked_id(field, key, attrs) {
                    match (owned_kind(field, key), json_ids(inner)) {
                        (Some(kind), Some(ids)) => {
                            ids.into_iter().for_each(|id| out.push(kind, id))
                        }
                        _ => out.unresolved(field),
                    }
                }
                var_refs(field, inner, attrs, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|v| var_refs(field, v, attrs, out)),
        _ => {}
    }
}

/// Whether the field sets a team key to a concrete string that is an allowed
/// team, or a list holding one (inline, through a variable, or inside a
/// variable it uses). A null or missing team doesn't count: it establishes no
/// team at all. [`check_teams`] has already refused any other team.
fn names_team(field: &RootField, variables: &Value) -> bool {
    field.args.iter().enumerate().any(|(n, tok)| match tok {
        Tok::Name(key) if team_key(key) && field.args.get(n + 1) == Some(&Tok::Punct(':')) => {
            match field.args.get(n + 2) {
                Some(Tok::Str(text)) => ours(text),
                Some(Tok::Var(var)) => variables.get(var).is_some_and(concrete_team),
                Some(Tok::Punct('[')) => field.args[n + 3..]
                    .iter()
                    .take_while(|t| **t != Tok::Punct(']'))
                    .any(|t| match t {
                        Tok::Str(text) => ours(text),
                        Tok::Var(var) => variables.get(var).is_some_and(concrete_team),
                        _ => false,
                    }),
                _ => false,
            }
        }
        Tok::Var(var) => variables.get(var).is_some_and(ai_team_in),
        _ => false,
    })
}

/// An allowed team's id or key as a string, or a list holding one.
fn concrete_team(value: &Value) -> bool {
    match value {
        Value::String(text) => ours(text),
        Value::Array(items) => items.iter().any(|v| v.as_str().is_some_and(ours)),
        _ => false,
    }
}

/// A team key anywhere in `value` set to an allowed team ([`concrete_team`]).
fn ai_team_in(value: &Value) -> bool {
    match value {
        Value::Object(map) => map
            .iter()
            .any(|(k, v)| (team_key(k) && concrete_team(v)) || ai_team_in(v)),
        Value::Array(items) => items.iter().any(ai_team_in),
        _ => false,
    }
}

/// A mutation's by-id targets ([`Targets`]); none for a query. Any other
/// `...Id` or `...Ids` key it can't check makes the call refused
/// ([`unchecked_id`]).
fn targets(op: &Operation, variables: &Value) -> Targets {
    let mut out = Targets::default();
    if op.kind != OpKind::Mutation {
        return out;
    }
    for field in &op.fields {
        let name = field.name.as_str();
        if name == "teamCreate" {
            continue;
        }
        // A team mutation's own `id` is held to AI or HOL by [`check_teams`];
        // its input's ids are read below like any other field's (2026-11-13).
        let team_field = TEAM_BY_ID.contains(&name);
        let mut root = None;
        for arg in ["id", "ids"] {
            if team_field && arg == "id" {
                continue;
            }
            let Some(at) = top_level(&field.args, arg) else {
                continue;
            };
            match (by_id_kind(name, arg), arg_ids(&field.args, at, variables)) {
                (Some(kind), Some(ids)) if !ids.is_empty() => {
                    root = Some(kind);
                    ids.into_iter().for_each(|id| out.push(kind, id));
                }
                _ => out.unresolved(name),
            }
        }
        let resolved = !out.unresolved.iter().any(|f| f == name);
        let issue_ok = (resolved && root == Some(Entity::Issue))
            || (name == "issueCreate" && names_team(field, variables));
        let project_ok = (resolved && name == "projectUpdate" && root == Some(Entity::Project))
            || (name == "projectCreate" && names_team(field, variables));
        let attrs: &[&str] = if issue_ok {
            ISSUE_ATTRIBUTE_IDS
        } else if project_ok {
            PROJECT_ATTRIBUTE_IDS
        } else {
            &[]
        };
        let mut depth = 0i32;
        for (n, tok) in field.args.iter().enumerate() {
            match tok {
                Tok::Punct('{' | '[' | '(') => depth += 1,
                Tok::Punct('}' | ']' | ')') => depth -= 1,
                // The root `id` and `ids` were read above (a team's by
                // [`check_teams`]); a nested one is checked below.
                Tok::Name(key) if depth == 0 && matches!(key.as_str(), "id" | "ids") => {}
                Tok::Name(key) if field.args.get(n + 1) == Some(&Tok::Punct(':')) => {
                    if let Some(kind) = ref_kind(name, key) {
                        match arg_ids(&field.args, n + 2, variables) {
                            Some(ids) => ids.into_iter().for_each(|id| out.push(kind, id)),
                            None => out.unresolved(name),
                        }
                    } else if unchecked_id(name, key, attrs) {
                        match (
                            owned_kind(name, key),
                            arg_ids(&field.args, n + 2, variables),
                        ) {
                            (Some(kind), Some(ids)) => {
                                ids.into_iter().for_each(|id| out.push(kind, id))
                            }
                            _ => out.unresolved(name),
                        }
                    }
                }
                Tok::Var(var) => var_refs(
                    name,
                    variables.get(var).unwrap_or(&Value::Null),
                    attrs,
                    &mut out,
                ),
                _ => {}
            }
        }
    }
    out
}

// ── Scrubbing ───────────────────────────────────────────────────────────────

/// A field or key that holds a secret: one containing secret, token,
/// password, passphrase, credential, authorization or apiKey; one with a
/// camelCase or snake_case word starting with `pass` (pass, passkey, passcode;
/// not bypass); or one ending in Key or Keys (privateKey, signingKey,
/// webhookKeys), but not a bare `key` (a team's key, `ENG`), `urlKey` or
/// `teamKey(s)`.
fn secret_name(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    let contains = [
        "secret",
        "token",
        "password",
        "passphrase",
        "apikey",
        "api_key",
        "authorization",
        "credential",
    ]
    .iter()
    .any(|w| lower.contains(w));
    let stem = lower.strip_suffix('s').unwrap_or(&lower);
    // Ids, not secrets: an organization's urlKey, a team's key.
    let named_id = matches!(stem, "urlkey" | "teamkey");
    let ends_key = stem.len() > 3 && stem.ends_with("key") && !named_id;
    let pass_word = key
        .split(['_', '-'])
        .flat_map(camel_words)
        .any(|w| w.to_ascii_lowercase().starts_with("pass"));
    contains || ends_key || pass_word
}

/// `privateKeyPass` as `private`, `Key`, `Pass`.
fn camel_words(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    for (n, c) in text.char_indices() {
        if n > start && c.is_ascii_uppercase() {
            out.push(&text[start..n]);
            start = n;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// The query text for the audit row, rebuilt from its tokens (comments and
/// commas dropped): every value given to a secret-named argument, input key or
/// variable (a string, a list, an object, a variable's default) masked, the
/// key and `lin_*` tokens removed, the redactor's token shapes masked, at most
/// [`AUDIT_QUERY`] characters. `None` when the document doesn't parse: then
/// no text is kept.
fn audit_query(adapter: &LinearAdapter, query: &str) -> Option<String> {
    parse(query).ok()?;
    let toks = lex(query).ok()?;
    let text = crate::claude_runner::redact(&scrub_text(adapter, &masked_text(&toks)));
    let mut out: String = text.chars().take(AUDIT_QUERY).collect();
    if text.chars().count() > AUDIT_QUERY {
        out.push('…');
    }
    Some(out)
}

/// Where the value that starts at `at` ends (one past it): a bracketed
/// list, object or argument list through its match, else one token.
fn value_end(toks: &[Tok], at: usize) -> usize {
    let (open, close) = match toks.get(at) {
        Some(Tok::Punct('[')) => ('[', ']'),
        Some(Tok::Punct('{')) => ('{', '}'),
        Some(Tok::Punct('(')) => ('(', ')'),
        Some(_) => return at + 1,
        None => return at,
    };
    let mut depth = 0;
    for (n, tok) in toks.iter().enumerate().skip(at) {
        match tok {
            Tok::Punct(c) if *c == open => depth += 1,
            Tok::Punct(c) if *c == close => {
                depth -= 1;
                if depth == 0 {
                    return n + 1;
                }
            }
            _ => {}
        }
    }
    toks.len()
}

/// The document as text, with secret-named values masked: after `name:` the
/// value, and after a secret-named `$var: Type =` its default.
fn masked_text(toks: &[Tok]) -> String {
    let mut out = String::new();
    let mut prev: Option<&Tok> = None;
    let mut i = 0;
    while i < toks.len() {
        let tok = &toks[i];
        let glue = matches!(tok, Tok::Punct(':' | ')' | ']' | '!'))
            || (tok == &Tok::Punct('(') && matches!(prev, Some(Tok::Name(_))))
            || matches!(prev, Some(Tok::Punct('(' | '[' | '@') | Tok::Spread));
        if !out.is_empty() && !glue {
            out.push(' ');
        }
        out.push_str(&match tok {
            Tok::Name(n) => n.clone(),
            Tok::Var(v) => format!("${v}"),
            Tok::Str(text) => Value::String(text.clone()).to_string(),
            Tok::Punct(c) => c.to_string(),
            Tok::Spread => "...".into(),
            Tok::Other(text) => text.clone(),
        });
        prev = Some(tok);
        i += 1;
        let secret = matches!(tok, Tok::Name(n) | Tok::Var(n) if secret_name(n));
        if !secret || toks.get(i) != Some(&Tok::Punct(':')) {
            continue;
        }
        // `name:` then the value; for a variable definition, its type, then
        // `=` and the default.
        out.push(':');
        i += 1;
        let mut at = i;
        if matches!(tok, Tok::Var(_)) {
            while matches!(
                toks.get(at),
                Some(Tok::Name(_) | Tok::Punct('[' | ']' | '!'))
            ) {
                at += 1;
            }
            if toks.get(at) != Some(&Tok::Punct('=')) {
                continue;
            }
            for t in &toks[i..=at] {
                out.push(' ');
                out.push_str(&match t {
                    Tok::Name(n) => n.clone(),
                    Tok::Punct(c) => c.to_string(),
                    _ => String::new(),
                });
            }
            at += 1;
        }
        if at < toks.len() {
            out.push_str(&format!(" \"{MASK}\""));
            i = value_end(toks, at);
            prev = Some(&Tok::Punct('"'));
        } else {
            i = at;
        }
    }
    out
}

/// Free text without Linear tokens or the key itself.
fn scrub_text(adapter: &LinearAdapter, text: &str) -> String {
    use regex::Regex;
    static TOKEN: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"\blin_(?:api|oauth|wh)_[A-Za-z0-9]{8,}").unwrap());
    let text = adapter.without_key(text);
    TOKEN.replace_all(&text, MASK).into_owned()
}

/// `value` with secret-named string fields masked and every string scrubbed.
fn scrub_value(adapter: &LinearAdapter, value: &Value, key: &str) -> Value {
    scrub_under(adapter, value, secret_name(key))
}

/// [`scrub_value`], where `secret` means an enclosing key named a secret: every
/// string below it is masked.
fn scrub_under(adapter: &LinearAdapter, value: &Value, secret: bool) -> Value {
    match value {
        Value::String(_) if secret => json!(MASK),
        Value::String(text) => json!(scrub_text(adapter, text)),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| scrub_under(adapter, v, secret))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), scrub_under(adapter, v, secret || secret_name(k))))
                .collect::<Map<_, _>>(),
        ),
        other => other.clone(),
    }
}

// ── Rate-limit window ───────────────────────────────────────────────────────

static WINDOWS: Lazy<Mutex<HashMap<String, DateTime<Utc>>>> = Lazy::new(Default::default);

fn windows() -> std::sync::MutexGuard<'static, HashMap<String, DateTime<Utc>>> {
    WINDOWS.lock().unwrap_or_else(|e| e.into_inner())
}

fn closed_until(endpoint: &str) -> Option<DateTime<Utc>> {
    windows()
        .get(endpoint)
        .copied()
        .filter(|until| *until > Utc::now())
}

fn close_window(endpoint: &str, reset: Option<&str>) {
    let until = reset
        .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
        .map(|at| at.with_timezone(&Utc))
        .filter(|at| *at > Utc::now())
        .unwrap_or_else(|| Utc::now() + Duration::seconds(DEFAULT_WINDOW));
    windows().insert(endpoint.to_string(), until);
}

fn limited(until: DateTime<Utc>) -> String {
    format!(
        "Linear's rate limit is spent; nothing was sent. Try again after {}.",
        until.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    )
}

// ── The call ────────────────────────────────────────────────────────────────

/// What a call is, before anything is sent: for the card and the audit row.
#[derive(Debug)]
pub(crate) struct Summary {
    pub kind: &'static str,
    pub label: String,
    pub fields: Vec<String>,
    pub destructive: bool,
    /// It grants access or reaches outside the organization.
    pub access: bool,
    /// The issues, comments, attachments, states, projects, milestones,
    /// labels and cycles whose team is read before the request is sent.
    pub targets: Vec<Target>,
}

/// The local checks: size, one operation, no secret-named field, the team
/// guard. Nothing is sent.
#[cfg(test)]
pub(crate) fn preflight(query: &str, variables: &Value) -> Result<Summary, String> {
    preflight_with(query, variables, true)
}

/// [`preflight`] with the personal team allowed or not (the config's choice).
pub(crate) fn preflight_with(
    query: &str,
    variables: &Value,
    personal: bool,
) -> Result<Summary, String> {
    let before = PERSONAL_ALLOWED.with(|p| p.replace(personal));
    let result = preflight_inner(query, variables);
    PERSONAL_ALLOWED.with(|p| p.set(before));
    result
}

fn preflight_inner(query: &str, variables: &Value) -> Result<Summary, String> {
    if query.trim().is_empty() {
        return Err("query is empty. Nothing was sent.".into());
    }
    if query.len() > MAX_QUERY || variables.to_string().len() > MAX_VARIABLES {
        return Err("the request is too large. Nothing was sent.".into());
    }
    if !matches!(variables, Value::Null | Value::Object(_)) {
        return Err("variables must be a JSON object. Nothing was sent.".into());
    }
    let op = parse(query).map_err(|e| format!("{e}. Nothing was sent."))?;
    if op.kind == OpKind::Subscription {
        return Err("subscriptions are not supported. Nothing was sent.".into());
    }
    if let Some(field) = &op.secret_field {
        return Err(format!(
            "the query selects `{field}`, a secret-named field; Hollow doesn't return secrets. Nothing was sent."
        ));
    }
    if op.kind == OpKind::Mutation && op.has_spread {
        return Err(
            "name a mutation's fields directly, with no fragment at its root. Nothing was sent."
                .into(),
        );
    }
    if op.kind == OpKind::Mutation && op.has_defaults {
        return Err("a mutation's variable definitions may not have defaults; pass the value in variables. Nothing was sent.".into());
    }
    check_teams(&op, variables).map_err(|e| format!("{e} Nothing was sent."))?;
    let targets = targets(&op, variables);
    if !targets.unresolved.is_empty() {
        return Err(format!(
            "Hollow can't read which team {} acts on (an id it can't resolve to a team), so it is refused, approved or not. {TEAM_ONLY} Nothing was sent.",
            targets.unresolved.join(", ")
        ));
    }
    if targets.found.len() > MAX_TARGETS {
        return Err(format!(
            "a mutation may act on at most {MAX_TARGETS} issues, comments, attachments, states, projects, milestones, labels or cycles by id. Nothing was sent."
        ));
    }
    Ok(Summary {
        kind: op.kind.word(),
        label: op.label().chars().take(120).collect(),
        fields: op.fields.iter().map(|f| f.name.clone()).collect(),
        destructive: op.destructive(variables),
        access: op.access(),
        targets: targets.found,
    })
}

fn failure(adapter: &LinearAdapter, endpoint: &str, error: TrackerError) -> String {
    if let TrackerError::RateLimited { reset_at } = &error {
        close_window(endpoint, reset_at.as_deref());
    }
    scrub_text(adapter, &format!("Linear refused the request: {error}"))
}

/// A guard reply that reports no requests left closes the window; the
/// call stops there.
fn spent(endpoint: &str, reply: &Value) -> Result<(), String> {
    if reply
        .pointer("/rateLimit/requestsRemaining")
        .and_then(Value::as_u64)
        == Some(0)
    {
        close_window(
            endpoint,
            reply.pointer("/rateLimit/resetAt").and_then(Value::as_str),
        );
        if let Some(until) = closed_until(endpoint) {
            return Err(limited(until));
        }
    }
    Ok(())
}

/// One read of every target's team (`HollowTeamGuard`); each must be in an
/// allowed team, by id, and a project or milestone in every one of its
/// teams. Not found, no team (a workspace label, shared across teams), a
/// team list cut short or another team: refused.
async fn confirm_targets(
    adapter: &LinearAdapter,
    endpoint: &str,
    targets: &[Target],
) -> Result<(), String> {
    let mut defs = Vec::new();
    let mut body = String::new();
    let mut vars = Map::new();
    for (n, target) in targets.iter().enumerate() {
        let (root, team) = target.kind.read();
        defs.push(format!("$t{n}: String!"));
        body.push_str(&format!(" t{n}: {root}(id: $t{n}) {{ {team} }}"));
        vars.insert(format!("t{n}"), json!(target.id));
    }
    let query = format!("query HollowTeamGuard({}) {{{body} }}", defs.join(", "));
    // The guard read failing stops the call: the request isn't sent.
    let reply = adapter
        .raw(&query, Value::Object(vars))
        .await
        .map_err(|e| format!("{} Nothing was sent.", failure(adapter, endpoint, e)))?;
    spent(endpoint, &reply)?;
    let allowed = |team: &Value| {
        team.get("id").and_then(Value::as_str).is_some_and(|id| {
            team_allowed(id) && (id != PERSONAL_TEAM_ID || adapter.personal_enabled())
        })
    };
    for (n, target) in targets.iter().enumerate() {
        let teams = target.kind.teams(reply.pointer(&format!("/data/t{n}")));
        let word = target.kind.word();
        let shown: String = scrub_text(adapter, &target.id).chars().take(80).collect();
        let Some(teams) = teams else {
            return Err(format!(
                "Hollow couldn't confirm which team {word} {shown:?} is in, so it is refused. {TEAM_ONLY} Nothing was sent."
            ));
        };
        if teams.is_empty() {
            return Err(format!(
                "{word} {shown:?} is in no team (a workspace-level one is shared across teams), so it is refused. {TEAM_ONLY} Nothing was sent."
            ));
        }
        let Some(other) = teams.iter().find(|t| !allowed(t)) else {
            continue;
        };
        let verb = if teams.len() > 1 { "spans" } else { "is in" };
        return Err(match other.get("key").and_then(Value::as_str) {
            Some(key) => format!(
                "{word} {shown:?} {verb} team {key}, which Hollow doesn't use. {TEAM_ONLY} Nothing was sent."
            ),
            None => format!(
                "Hollow couldn't confirm which team {word} {shown:?} is in, so it is refused. {TEAM_ONLY} Nothing was sent."
            ),
        });
    }
    Ok(())
}

/// The organization check, the team of each by-id target, then the request;
/// the shaped reply.
async fn send(
    adapter: &LinearAdapter,
    query: &str,
    variables: Value,
    targets: &[Target],
) -> Result<Value, String> {
    let endpoint = adapter.endpoint().to_string();
    if let Some(until) = closed_until(&endpoint) {
        return Err(limited(until));
    }
    let guard = adapter
        .raw(ORG_QUERY, json!({}))
        .await
        .map_err(|e| failure(adapter, &endpoint, e))?;
    let org = |field: &str| {
        guard
            .pointer(&format!("/data/viewer/organization/{field}"))
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("")
    };
    if org("id") != ORG_ID {
        let shown = |v: &'static str| match org(v) {
            "" => "none".to_string(),
            v => format!("{v:?}"),
        };
        return Err(format!(
            "The saved API key belongs to organization {} (id {}, urlKey {}), not {ORG_NAME:?} (id {ORG_ID}). Nothing was sent.",
            shown("name"),
            shown("id"),
            shown("urlKey"),
        ));
    }
    spent(&endpoint, &guard)?;
    if !targets.is_empty() {
        confirm_targets(adapter, &endpoint, targets).await?;
    }
    let body = adapter
        .raw(query, variables)
        .await
        .map_err(|e| failure(adapter, &endpoint, e))?;
    if body
        .pointer("/rateLimit/requestsRemaining")
        .and_then(Value::as_u64)
        == Some(0)
    {
        close_window(
            &endpoint,
            body.pointer("/rateLimit/resetAt").and_then(Value::as_str),
        );
    }
    Ok(shaped(adapter, &body))
}

/// `{data, errors, rateLimit}` as the agent reads it: secrets masked, error
/// text kept (it is how a bad field is found) but scrubbed and bounded.
fn shaped(adapter: &LinearAdapter, body: &Value) -> Value {
    let mut out = Map::new();
    out.insert(
        "data".into(),
        scrub_value(adapter, body.get("data").unwrap_or(&Value::Null), ""),
    );
    if let Some(errors) = body.get("errors").and_then(Value::as_array) {
        let errors: Vec<Value> = errors
            .iter()
            .map(|e| {
                let text = e.get("message").and_then(Value::as_str).unwrap_or("");
                json!({
                    "message": scrub_text(adapter, &text.chars().take(600).collect::<String>()),
                    "path": e.get("path").cloned().unwrap_or(Value::Null),
                    "code": e.pointer("/extensions/code").cloned().unwrap_or(Value::Null),
                })
            })
            .collect();
        out.insert("errors".into(), Value::Array(errors));
    }
    if let Some(rate) = body.get("rateLimit") {
        out.insert("rateLimit".into(), rate.clone());
    }
    Value::Object(out)
}

/// One `tracker_write` audit row for a call (action `graphql`).
fn audit(
    adapter: &LinearAdapter,
    vault: &Path,
    asked_by: String,
    summary: Option<&Summary>,
    query: &str,
    variables: &Value,
    result: &Result<Value, String>,
) {
    let session = crate::audit::is_session(&asked_by).then(|| asked_by.clone());
    let (outcome, detail) = match result {
        Ok(reply) => match reply.pointer("/errors/0/message").and_then(Value::as_str) {
            Some(first) => ("failed", Some(format!("GraphQL error: {first}"))),
            None => ("ok", None),
        },
        Err(why) => ("failed", Some(why.chars().take(300).collect::<String>())),
    };
    // The query text even for a refused call, so the row shows inline
    // arguments (an issue id, a team) the variables don't.
    let mut after = json!({ "query": audit_query(adapter, query) });
    if after["query"].is_null() {
        after["note"] = json!("the query didn't parse, so its text isn't kept");
    }
    if summary.is_none() {
        if let Ok(op) = parse(query) {
            after["fields"] = json!(op.fields.iter().map(|f| f.name.clone()).collect::<Vec<_>>());
        }
    }
    if let Some(s) = summary {
        after["kind"] = json!(s.kind);
        after["operation"] = json!(s.label);
        after["fields"] = json!(s.fields);
        after["variables"] = scrub_value(adapter, variables, "");
    }
    let after = Some(after);
    crate::audit::tracker_write(
        vault,
        crate::audit::Write {
            action: "graphql",
            session_key: session.as_deref(),
            asked_by,
            after,
            outcome,
            detail,
            ..crate::audit::Write::default()
        },
    );
}

/// Run one call with `adapter`: the guards, the request, the audit row.
pub(crate) async fn call(
    adapter: &LinearAdapter,
    vault: &Path,
    asked_by: String,
    query: &str,
    variables: Option<Value>,
) -> Result<Value, String> {
    let variables = variables.unwrap_or(Value::Null);
    let checked = preflight_with(query, &variables, adapter.personal_enabled());
    let result = match &checked {
        Ok(summary) => {
            let vars = if variables.is_null() {
                json!({})
            } else {
                variables.clone()
            };
            send(adapter, query, vars, &summary.targets).await
        }
        Err(why) => Err(why.clone()),
    };
    audit(
        adapter,
        vault,
        asked_by,
        checked.as_ref().ok(),
        query,
        &variables,
        &result,
    );
    result
}

/// `tracker.graphql`'s body: the adapter the saved settings describe.
pub(crate) async fn run(
    config: &crate::config::Config,
    query: &str,
    variables: Option<Value>,
) -> Result<Value, String> {
    crate::tracker_sync::enabled(config)?;
    let adapter = LinearAdapter::from_config(&config.tracker).map_err(|e| e.to_string())?;
    call(
        &adapter,
        &config.vault_path,
        crate::audit::asker(),
        query,
        variables,
    )
    .await
}

#[cfg(test)]
#[path = "graphql_tests.rs"]
mod tests;
