//! The result envelope of the read tools of Hollow's own MCP servers (OB1,
//! overnight 11-17): `hollow_app`'s `list_sessions` and `read_session`,
//! `hollow_tasks`' `list_tasks` and `list_projects`, and `hollow_vault`'s
//! `search_vault`.
//!
//! A result's text is held to a byte budget ([`DEFAULT_MAX_BYTES`]; the
//! caller may ask for up to [`HARD_MAX_BYTES`] with `maxBytes`). A result
//! that fits is returned byte for byte as before. One that doesn't keeps as
//! many whole list items as fit, then lists further items headers only (the
//! body is one call to the tool's own reader away), and hands back a cursor
//! that resumes at the first item not yet returned whole. Every enveloped result carries one extra text block,
//! `{"envelope":{...}}`, after the result text: `budgeted` (the text was
//! cut to the budget), `complete` (nothing left out: no next page, no
//! headers-only item, no source limit reached) and `capReasons`
//! (`byte_budget`, `row_limit`, `item_over_budget`); a cut result adds
//! `maxBytes`, `items` (`from`, `returned`, `headersOnly`, `total`),
//! `nextCursor` and a `hint` naming the next call.
//!
//! A cursor is `c1.<offset>.<hash>`: the offset into the list and the first
//! 12 hex digits of a SHA-256 of the tool's name and its other arguments
//! (keys sorted, `maxBytes` and `cursor` left out). A cursor from another
//! tool or other arguments is refused with what to do instead. It is bound
//! to the query, not to a snapshot: a list that changed between pages can
//! shift by the rows that came or went.
//!
//! The envelope block's bytes are stable (sorted keys, no times), so the
//! byte-stability test below (TC1) holds for it too.
//!
//! Refusals (BI5): every `isError` result of the three servers (and of
//! `hollow_code_context`, CC2) keeps its
//! readable sentence as the first text block and adds a last one,
//! `{"error":{"code":...,"next":...,"didYouMean":[...]}}`, with a code from
//! the closed set [`REFUSAL_CODES`] ([`classify`]).

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

/// The text budget of one result when the caller names none (about 6,000
/// tokens).
pub(crate) const DEFAULT_MAX_BYTES: usize = 24 * 1024;
/// The smallest budget `maxBytes` sets (a smaller one is raised to it).
pub(crate) const MIN_MAX_BYTES: usize = 4 * 1024;
/// The largest budget `maxBytes` sets: `app_actions::RESULT_MAX_CHARS`, the
/// cut the app broker already makes.
pub(crate) const HARD_MAX_BYTES: usize = 80_000;

/// The two arguments the envelope adds to an enveloped tool.
pub(crate) const ARG_MAX_BYTES: &str = "maxBytes";
pub(crate) const ARG_CURSOR: &str = "cursor";

const CURSOR_VERSION: &str = "c1";

/// The two schema properties an enveloped tool adds (kept short: Codex is
/// sent every listed schema on every turn).
pub(crate) fn schema_properties() -> [(&'static str, Value); 2] {
    [
        (
            ARG_MAX_BYTES,
            json!({ "type": "integer", "minimum": MIN_MAX_BYTES, "maximum": HARD_MAX_BYTES, "description": "Text budget in bytes (default 24576)." }),
        ),
        (
            ARG_CURSOR,
            json!({ "type": "string", "description": "The envelope's nextCursor, with the same other arguments." }),
        ),
    ]
}

/// Add [`schema_properties`] to a tool spec's `inputSchema.properties`.
pub(crate) fn add_schema(tool: &mut Value) {
    if let Some(props) = tool
        .pointer_mut("/inputSchema/properties")
        .and_then(Value::as_object_mut)
    {
        for (name, schema) in schema_properties() {
            props.insert(name.to_string(), schema);
        }
    }
}

/// One enveloped call's budget and page.
#[derive(Debug, Clone)]
pub(crate) struct Request {
    pub max_bytes: usize,
    /// Items of the list already returned by earlier pages.
    pub offset: usize,
    /// The query a cursor binds to (`tool` and the other arguments).
    query: String,
}

impl Request {
    /// Take `maxBytes` and `cursor` out of `args` (so the tool, and the
    /// app broker's own argument check, never see them) and check them. A
    /// near miss of either name (`max_bytes`, `next_cursor`) is refused with
    /// the right name.
    pub(crate) fn take(tool: &str, args: &mut Value) -> Result<Request, String> {
        if args.is_null() {
            *args = json!({});
        }
        let Some(map) = args.as_object_mut() else {
            return Err(format!("{tool}: arguments must be an object."));
        };
        if let Some(fix) = map.keys().find_map(|k| near_miss(k)) {
            return Err(format!("{tool}: {fix}"));
        }
        let max_bytes = match map.remove(ARG_MAX_BYTES) {
            None | Some(Value::Null) => DEFAULT_MAX_BYTES,
            Some(v) => v
                .as_u64()
                .map(|n| (n as usize).clamp(MIN_MAX_BYTES, HARD_MAX_BYTES))
                .ok_or_else(|| {
                    format!("{tool}: `maxBytes` must be a whole number of bytes ({MIN_MAX_BYTES} to {HARD_MAX_BYTES}; default {DEFAULT_MAX_BYTES}).")
                })?,
        };
        let cursor = match map.remove(ARG_CURSOR) {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if s.trim().is_empty() => None,
            Some(Value::String(s)) => Some(s),
            Some(_) => {
                return Err(format!(
                    "{tool}: `cursor` must be the string an envelope's nextCursor gave."
                ))
            }
        };
        let query = format!("{tool}\n{}", canonical(args));
        let offset = match cursor {
            None => 0,
            Some(c) => parse_cursor(tool, &c, &query)?,
        };
        Ok(Request {
            max_bytes,
            offset,
            query,
        })
    }

    /// The cursor that resumes this query at `offset`.
    pub(crate) fn cursor_at(&self, offset: usize) -> String {
        format!("{CURSOR_VERSION}.{offset}.{}", query_hash(&self.query))
    }
}

/// The right name for a near miss of `maxBytes` or `cursor`, as an error.
pub(crate) fn near_miss(key: &str) -> Option<String> {
    let folded: String = key
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase();
    let (right, what) = match folded.as_str() {
        // Only spellings of the two names themselves (`max_bytes`,
        // `MaxBytes`, `next_cursor`, `nextCursor`): a tool may one day take
        // an argument like `budget` of its own.
        "maxbytes" => (
            ARG_MAX_BYTES,
            "the text budget (whole bytes; default 24576)",
        ),
        "cursor" | "nextcursor" => (ARG_CURSOR, "the envelope's nextCursor"),
        _ => return None,
    };
    (key != right).then(|| format!("unknown argument `{key}`; did you mean `{right}` ({what})?"))
}

/// JSON with every object's keys sorted, whatever map type built it, so the
/// same arguments in any order hash alike.
pub(crate) fn canonical(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .iter()
                .map(|k| format!("{}:{}", json!(k), canonical(&map[k.as_str()])))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(canonical).collect::<Vec<_>>().join(",")
        ),
        other => other.to_string(),
    }
}

fn query_hash(query: &str) -> String {
    let digest = Sha256::digest(query.as_bytes());
    format!("{digest:x}")[..12].to_string()
}

fn parse_cursor(tool: &str, cursor: &str, query: &str) -> Result<usize, String> {
    let malformed = || {
        format!("{tool}: `cursor` `{cursor}` isn't a cursor this tool gave (they look like c1.<n>.<hash>); drop it to start from the first page.")
    };
    let mut parts = cursor.trim().split('.');
    let (Some(version), Some(offset), Some(hash), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(malformed());
    };
    if version != CURSOR_VERSION {
        return Err(malformed());
    }
    let offset: usize = offset.parse().map_err(|_| malformed())?;
    if hash != query_hash(query) {
        return Err(format!("{tool}: this cursor belongs to another query (another tool, or other arguments than the call that returned it). Repeat that call's arguments exactly with the cursor, or drop cursor to start over."));
    }
    Ok(offset)
}

/// What an enveloped result says about itself.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Envelope {
    pub budgeted: bool,
    pub cap_reasons: Vec<&'static str>,
    pub max_bytes: usize,
    pub from: usize,
    pub returned: usize,
    pub headers_only: usize,
    pub total: usize,
    pub next_cursor: Option<String>,
    pub hint: Vec<String>,
}

impl Envelope {
    pub(crate) fn complete(&self) -> bool {
        self.cap_reasons.is_empty() && self.next_cursor.is_none() && self.headers_only == 0
    }

    pub(crate) fn to_value(&self) -> Value {
        let mut out = json!({
            "budgeted": self.budgeted,
            "complete": self.complete(),
            "capReasons": self.cap_reasons,
        });
        if self.budgeted || self.from > 0 {
            out["maxBytes"] = json!(self.max_bytes);
            out["items"] = json!({
                "from": self.from,
                "returned": self.returned,
                "headersOnly": self.headers_only,
                "total": self.total,
            });
        }
        if let Some(cursor) = &self.next_cursor {
            out["nextCursor"] = json!(cursor);
        }
        if !self.hint.is_empty() {
            out["hint"] = json!(self.hint.join(" "));
        }
        out
    }

    /// The envelope as the extra text block of a tool result.
    pub(crate) fn block(&self) -> Value {
        json!({ "type": "text", "text": json!({ "envelope": self.to_value() }).to_string() })
    }
}

/// A list as [`fit`] pages it.
pub(crate) struct Listing<'a> {
    /// Items in the list.
    pub total: usize,
    /// Items have a headers-only form.
    pub has_headers: bool,
    /// The source's own limit cut the list (`row_limit`), and how to see more.
    pub row_limit: Option<&'a str>,
    /// How to read a headers-only item's body.
    pub header_hint: &'a str,
}

/// A result held to its budget: the text, and the envelope saying how.
#[derive(Debug, Clone)]
pub(crate) struct Fitted {
    pub text: String,
    pub envelope: Envelope,
}

/// Fit a list result to `req`'s budget. `whole` is the untouched text,
/// returned as is when the first page fits; `build(full, headers)` renders
/// the result with the list cut to `full` whole items then `headers`
/// headers-only ones, counted from `req.offset`. Whole items come first,
/// as many as fit, then headers-only ones; at least one item goes out, so
/// paging always moves.
pub(crate) fn fit(
    req: &Request,
    listing: &Listing<'_>,
    whole: impl FnOnce() -> String,
    build: impl Fn(usize, usize) -> String,
) -> Result<Fitted, String> {
    let budget = req.max_bytes;
    let mut envelope = Envelope {
        max_bytes: budget,
        from: req.offset,
        total: listing.total,
        ..Envelope::default()
    };
    if let Some(more) = listing.row_limit {
        envelope.cap_reasons.push("row_limit");
        envelope.hint.push(more.to_string());
    }
    if req.offset > listing.total {
        return Err(format!(
            "this cursor is past the end of the list ({} items now); drop cursor to start over.",
            listing.total
        ));
    }
    if req.offset == 0 {
        let text = whole();
        if text.len() <= budget || listing.total == 0 {
            envelope.returned = listing.total;
            if text.len() > budget {
                envelope.cap_reasons.push("item_over_budget");
            }
            return Ok(Fitted { text, envelope });
        }
    }
    let n = listing.total - req.offset;
    let fits = |full: usize, headers: usize| build(full, headers).len() <= budget;
    let mut full = largest(n, |k| fits(k, 0));
    // The first item alone is over the budget: it goes whole anyway (the
    // tool's own cut, at most HARD_MAX_BYTES, still applies), flagged, so
    // a long answer is never only a header whose reader is held to the
    // same budget.
    let over = full == 0 && n > 0;
    if over {
        full = 1;
    }
    let headers = if listing.has_headers && !over {
        largest(n - full, |k| fits(full, k))
    } else {
        0
    };
    let text = build(full, headers);
    // The next page starts at the first item not yet returned whole.
    let next = req.offset + full;
    envelope.returned = full + headers;
    envelope.headers_only = headers;
    envelope.budgeted = full < n || over;
    if envelope.budgeted {
        envelope.cap_reasons.insert(0, "byte_budget");
    }
    if over && text.len() > budget {
        envelope.cap_reasons.push("item_over_budget");
    }
    if headers > 0 {
        envelope.hint.push(listing.header_hint.to_string());
    }
    if next < listing.total {
        envelope.next_cursor = Some(req.cursor_at(next));
        envelope.hint.push(format!(
            "{} more not yet whole: call again with the same arguments and cursor (or a larger maxBytes, at most {HARD_MAX_BYTES}).",
            listing.total - next
        ));
    }
    Ok(Fitted { text, envelope })
}

/// The largest `k` in `0..=n` for which `ok(k)` holds, assuming it holds
/// for 0 and stops holding past some point (a binary search; a non-monotone
/// `ok` still yields a `k` it accepts, or 0, then climbs while the next
/// `k` is accepted too, so a renderer that switches format past a size
/// can't stop it short).
fn largest(n: usize, ok: impl Fn(usize) -> bool) -> usize {
    let (mut lo, mut hi) = (0usize, n);
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if ok(mid) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    while lo < n && ok(lo + 1) {
        lo += 1;
    }
    lo
}

/// [`fit`] for a JSON result whose list is at `pointer` (`""`: the result
/// is the list). `from_end`: the list runs oldest first and its newest items
/// matter most (a conversation), so pages run from the end and an item cut
/// to its header is an older one. `header` makes an item's headers-only form.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fit_json(
    req: &Request,
    value: &Value,
    pointer: &str,
    from_end: bool,
    header: Option<&dyn Fn(&Value) -> Value>,
    row_limit: Option<&str>,
    header_hint: &str,
    render: &dyn Fn(&Value) -> String,
) -> Result<Fitted, String> {
    let rows: Vec<Value> = value
        .pointer(pointer)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let has_list = value.pointer(pointer).is_some_and(Value::is_array);
    let mut ordered = rows;
    if from_end {
        ordered.reverse();
    }
    let offset = req.offset.min(ordered.len());
    let build = |full: usize, headers: usize| {
        let rest = &ordered[offset..];
        let mut page: Vec<Value> = rest[..full].to_vec();
        if let Some(header) = header {
            page.extend(rest[full..full + headers].iter().map(header));
        }
        if from_end {
            page.reverse();
        }
        let mut out = value.clone();
        if let Some(slot) = out.pointer_mut(pointer) {
            *slot = Value::Array(page);
        }
        render(&out)
    };
    let listing = Listing {
        total: if has_list { ordered.len() } else { 0 },
        has_headers: header.is_some(),
        row_limit,
        header_hint,
    };
    fit(req, &listing, || render(value), build)
}

/// An item's headers-only form: the named keys it has, and `headerOnly`.
pub(crate) fn header_of(row: &Value, keys: &[&str]) -> Value {
    let mut out = Map::new();
    if let Value::Object(map) = row {
        for key in keys {
            if let Some(v) = map.get(*key) {
                out.insert((*key).to_string(), v.clone());
            }
        }
    }
    out.insert("headerOnly".into(), json!(true));
    Value::Object(out)
}

// ── Refusal codes (BI5, overnight 11-17) ────────────────────────────────

/// Which owned server a refusal comes from: one code can name a different
/// next call on each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Server {
    App,
    Tasks,
    Vault,
    /// `hollow_code_context` (CC2).
    Code,
}

/// One refusal code of the closed set: what it means, and the next call on
/// `hollow_app`, `hollow_tasks`, `hollow_vault` and `hollow_code_context` in
/// that order. A next is
/// a tool name, an app action id (run it with `run_action`), `tools/list`,
/// `retry` (the same tool again, changed as the text says, or unchanged a
/// little later) or `reply` (no call fixes it: say so in your reply or ask
/// the person).
#[derive(Debug)]
pub(crate) struct RefusalCode {
    pub code: &'static str,
    /// What it means, in the source and the closed-set test (not sent).
    #[allow(dead_code)]
    pub meaning: &'static str,
    pub next: [&'static str; 4],
}

const fn code(code: &'static str, meaning: &'static str, next: [&'static str; 4]) -> RefusalCode {
    RefusalCode {
        code,
        meaning,
        next,
    }
}

/// The closed set every refusal of the four enveloped owned servers
/// resolves to.
/// The `TASK_*` and `IDEMPOTENCY_MISMATCH` codes are `vault_tasks`' own
/// error prefixes, kept as they were; `REFUSED` is the fallback for a
/// refusal no rule names (the text says why).
pub(crate) const REFUSAL_CODES: &[RefusalCode] = &[
    code(
        "UNKNOWN_ACTION",
        "no action has this id",
        ["list_actions", "tools/list", "tools/list", "tools/list"],
    ),
    code(
        "UNKNOWN_TOOL",
        "this server has no tool by this name",
        ["list_actions", "tools/list", "tools/list", "tools/list"],
    ),
    code(
        "INVALID_ARGUMENT",
        "an argument is missing, unknown or of the wrong kind",
        ["describe_action", "retry", "retry", "retry"],
    ),
    code(
        "NOT_PERMITTED",
        "this run's tier, access, depth or the person's answer on a card does not allow it",
        ["reply", "reply", "reply", "reply"],
    ),
    code(
        "NOT_FOUND",
        "the named session, workspace, task, file or skill does not exist here",
        ["retry", "list_tasks", "search_vault", "symbols"],
    ),
    code(
        "CONFLICT",
        "it changed since it was read",
        ["retry", "get_task", "retry", "retry"],
    ),
    code(
        "CURSOR_INVALID",
        "the cursor is not one this call gave, or is past the end",
        ["retry", "retry", "retry", "retry"],
    ),
    code(
        "UNAVAILABLE",
        "Hollow, its broker or the vault did not answer or is not connected",
        ["retry", "retry", "retry", "retry"],
    ),
    code(
        "INTERNAL",
        "Hollow sent an answer this server could not read",
        ["retry", "retry", "retry", "retry"],
    ),
    code(
        "REQUEST_ID_REUSED",
        "this requestId was used for a different call",
        ["sessions.request_status", "reply", "reply", "reply"],
    ),
    code(
        "REQUEST_PENDING",
        "a call with this requestId has started and not finished",
        ["sessions.request_status", "reply", "reply", "reply"],
    ),
    code(
        "TASK_CONFLICT",
        "the task changed since expectedHash was read",
        ["retry", "get_task", "retry", "retry"],
    ),
    code(
        "TASK_INVALID",
        "a task field's value is not allowed",
        ["describe_action", "retry", "retry", "retry"],
    ),
    code(
        "TASK_LINKED",
        "the task's tracker issue holds this field",
        ["retry", "get_task", "retry", "retry"],
    ),
    code(
        "TASK_CYCLE",
        "the blocker would close a loop",
        ["retry", "get_task", "retry", "retry"],
    ),
    code(
        "TASK_BLOCKER",
        "a blocker names no task",
        ["retry", "list_tasks", "retry", "retry"],
    ),
    code(
        "TASK_KEY",
        "the idempotencyKey is missing or too long",
        ["retry", "retry", "retry", "retry"],
    ),
    code(
        "IDEMPOTENCY_MISMATCH",
        "the idempotencyKey already created different work",
        ["retry", "retry", "retry", "retry"],
    ),
    code(
        "TASK_CLAIMED",
        "someone else holds the task's claim",
        ["retry", "next_claimable", "retry", "retry"],
    ),
    code(
        "REFUSED",
        "refused for the reason the text gives",
        ["reply", "reply", "reply", "reply"],
    ),
];

/// Text rules, first match wins: a lower-case phrase of the refusal, its
/// code, and a next that overrides the code's own. A refusal that starts
/// with a code of the set (`TASK_CONFLICT: ...`) takes that code first.
const REFUSAL_RULES: &[(&str, &str, Option<&str>)] = &[
    ("unknown action", "UNKNOWN_ACTION", None),
    ("unknown tool", "UNKNOWN_TOOL", None),
    ("unknown argument", "INVALID_ARGUMENT", None),
    ("takes no arguments", "INVALID_ARGUMENT", None),
    ("cursor", "CURSOR_INVALID", None),
    ("can only read it", "NOT_PERMITTED", None),
    ("can read tasks but not change", "NOT_PERMITTED", None),
    ("user declined", "NOT_PERMITTED", None),
    ("no answer on the card", "NOT_PERMITTED", None),
    ("the card for", "NOT_PERMITTED", None),
    ("needs the user's approval", "NOT_PERMITTED", None),
    ("needs the person's allow", "NOT_PERMITTED", None),
    ("levels deep", "NOT_PERMITTED", Some("sessions.handover")),
    ("this run has finished", "NOT_PERMITTED", None),
    ("refused: read_source", "NOT_PERMITTED", None),
    ("may never", "NOT_PERMITTED", None),
    ("can't run it", "NOT_PERMITTED", None),
    ("unreadable answer", "INTERNAL", None),
    ("unavailable", "UNAVAILABLE", None),
    ("receipts are full", "UNAVAILABLE", None),
    ("isn't connected", "UNAVAILABLE", None),
    ("did not answer within", "UNAVAILABLE", None),
    ("closed the connection", "UNAVAILABLE", None),
    ("could not reach", "UNAVAILABLE", None),
    ("answer failed", "UNAVAILABLE", None),
    ("changed on disk", "CONFLICT", None),
    ("changed since", "CONFLICT", None),
    ("no actions in area", "NOT_FOUND", Some("list_actions")),
    ("no workspace", "NOT_FOUND", Some("list_workspaces")),
    ("no session", "NOT_FOUND", Some("list_sessions")),
    ("is not a session key", "NOT_FOUND", Some("list_sessions")),
    ("is not a readable task", "NOT_FOUND", Some("list_tasks")),
    (
        "is not a readable project",
        "NOT_FOUND",
        Some("list_projects"),
    ),
    ("no skill named", "NOT_FOUND", Some("retry")),
    // hollow_code_context (CC2).
    ("no definition named", "NOT_FOUND", None),
    ("over the impact cap", "INVALID_ARGUMENT", Some("callers")),
    // CC4: a slice handle that is unknown, expired or from other arguments.
    ("this handle", "CURSOR_INVALID", None),
    ("over the slice cap", "INVALID_ARGUMENT", None),
    // CC5: a diff too large for `changes` and `affected_tests`.
    ("over the changes cap", "INVALID_ARGUMENT", None),
    ("no reference named", "NOT_FOUND", Some("retry")),
    ("not found", "NOT_FOUND", None),
    ("no such", "NOT_FOUND", None),
    ("does not exist", "NOT_FOUND", None),
    ("doesn't exist", "NOT_FOUND", None),
    ("is not text", "INVALID_ARGUMENT", Some("reply")),
    ("is required", "INVALID_ARGUMENT", None),
    ("must be", "INVALID_ARGUMENT", None),
    ("needs a word", "INVALID_ARGUMENT", None),
    ("only common words", "INVALID_ARGUMENT", None),
    ("at least", "INVALID_ARGUMENT", None),
    ("is past the end", "INVALID_ARGUMENT", None),
    ("is before", "INVALID_ARGUMENT", None),
];

/// A refusal's machine half: its code, the next call and up to three
/// names the text suggested.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Refusal {
    pub code: &'static str,
    pub next: &'static str,
    pub did_you_mean: Vec<String>,
}

impl Refusal {
    pub(crate) fn to_value(&self) -> Value {
        let mut error = json!({ "code": self.code, "next": self.next });
        if !self.did_you_mean.is_empty() {
            error["didYouMean"] = json!(self.did_you_mean);
        }
        json!({ "error": error })
    }

    /// The refusal as the last text block of an `isError` result, after the
    /// readable sentence (which stays the first block, as before).
    pub(crate) fn block(&self) -> Value {
        json!({ "type": "text", "text": self.to_value().to_string() })
    }
}

fn refusal_code(code: &str) -> Option<&'static RefusalCode> {
    REFUSAL_CODES.iter().find(|c| c.code == code)
}

/// The code, next call and suggestions of refusal `text` from `server`.
pub(crate) fn classify(server: Server, text: &str) -> Refusal {
    let slot = match server {
        Server::App => 0,
        Server::Tasks => 1,
        Server::Vault => 2,
        Server::Code => 3,
    };
    // A leading code (after an optional `tool:`), as vault_tasks writes them.
    let leading = text
        .split_whitespace()
        .take(2)
        .filter_map(|word| word.strip_suffix(':'))
        .find_map(refusal_code);
    let lower = text.to_lowercase();
    let (known, next) = match leading {
        Some(known) => (known, None),
        None => REFUSAL_RULES
            .iter()
            .find(|(phrase, _, _)| lower.contains(phrase))
            .and_then(|(_, code, next)| refusal_code(code).map(|known| (known, *next)))
            .unwrap_or((&REFUSAL_CODES[REFUSAL_CODES.len() - 1], None)),
    };
    Refusal {
        code: known.code,
        next: next.unwrap_or(known.next[slot]),
        did_you_mean: did_you_mean(text),
    }
}

/// The back-quoted names after "did you mean" in `text`, up to its `?`, at
/// most three (double-quoted ones when none is back-quoted, as
/// `hollow_code_context` writes them).
fn did_you_mean(text: &str) -> Vec<String> {
    let Some(at) = text
        .find("did you mean")
        .or_else(|| text.find("Did you mean"))
    else {
        return Vec::new();
    };
    let rest = &text[at..];
    let rest = rest.split('?').next().unwrap_or(rest);
    let quote = if rest.contains('`') { '`' } else { '"' };
    rest.split(quote)
        .skip(1)
        .step_by(2)
        .filter(|name| !name.is_empty())
        .take(3)
        .map(str::to_string)
        .collect()
}

/// An `isError` tool result's content: the readable `text` first (what an
/// old caller reads), then the refusal block.
pub(crate) fn refusal_content(server: Server, text: &str) -> Value {
    json!([
        { "type": "text", "text": text },
        classify(server, text).block(),
    ])
}

#[cfg(test)]
/// TC1: where two renderings of the same thing first differ, as a byte
/// offset and a short excerpt of each side; `None` when they're identical.
pub(crate) fn first_difference(a: &str, b: &str) -> Option<String> {
    if a == b {
        return None;
    }
    let at = a
        .bytes()
        .zip(b.bytes())
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()));
    let excerpt = |s: &str| {
        let start = at.saturating_sub(40);
        let bytes = &s.as_bytes()[start.min(s.len())..(at + 40).min(s.len())];
        String::from_utf8_lossy(bytes).into_owned()
    };
    Some(format!(
        "differ at byte {at} (lengths {} and {}): …{}… vs …{}…",
        a.len(),
        b.len(),
        excerpt(a),
        excerpt(b)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn req(tool: &str, args: Value) -> Result<(Request, Value), String> {
        let mut args = args;
        let r = Request::take(tool, &mut args)?;
        Ok((r, args))
    }

    fn rows(n: usize, body: usize) -> Value {
        json!({
            "sessions": (0..n)
                .map(|i| json!({ "key": format!("s-{i}"), "title": format!("t{i}"), "body": "x".repeat(body) }))
                .collect::<Vec<_>>()
        })
    }

    fn pretty(v: &Value) -> String {
        serde_json::to_string_pretty(v).unwrap()
    }

    fn head(row: &Value) -> Value {
        header_of(row, &["key", "title"])
    }

    #[test]
    fn envelope_args_are_taken_out_and_checked() {
        let (r, rest) = req("t", json!({ "limit": 3, "maxBytes": 10, "detail": true })).unwrap();
        assert_eq!(r.max_bytes, MIN_MAX_BYTES, "a small budget is raised");
        assert_eq!(rest, json!({ "limit": 3, "detail": true }));
        let (r, _) = req("t", json!({ "maxBytes": 10_000_000 })).unwrap();
        assert_eq!(
            r.max_bytes, HARD_MAX_BYTES,
            "a big one is held to the ceiling"
        );
        let (r, rest) = req("t", Value::Null).unwrap();
        assert_eq!((r.max_bytes, r.offset), (DEFAULT_MAX_BYTES, 0));
        assert_eq!(rest, json!({}));
        let err = req("t", json!({ "maxBytes": "big" })).unwrap_err();
        assert!(err.contains("whole number"), "{err}");
        let err = req("t", json!({ "max_bytes": 9000 })).unwrap_err();
        assert!(err.contains("did you mean `maxBytes`"), "{err}");
        let err = req("t", json!({ "nextCursor": "c1.3.abc" })).unwrap_err();
        assert!(err.contains("did you mean `cursor`"), "{err}");
        // Other names are the tool's business, not the envelope's.
        for own in ["budget", "maxSize", "pageToken", "page"] {
            let mut args = json!({});
            args[own] = json!(1);
            assert!(req("t", args).is_ok(), "{own}");
        }
        assert!(req("t", json!({ "cursor": 3 })).is_err());
    }

    #[test]
    fn a_cursor_is_bound_to_its_tool_and_arguments() {
        let (first, _) = req("list_sessions", json!({ "detail": true, "limit": 30 })).unwrap();
        let cursor = first.cursor_at(7);
        // The same arguments in another order, with another budget: accepted.
        let (again, _) = req(
            "list_sessions",
            json!({ "limit": 30, "cursor": cursor, "maxBytes": 50_000, "detail": true }),
        )
        .unwrap();
        assert_eq!(again.offset, 7);
        let err = req(
            "list_sessions",
            json!({ "detail": true, "limit": 12, "cursor": cursor }),
        )
        .unwrap_err();
        assert!(err.contains("another query"), "{err}");
        let err = req(
            "read_session",
            json!({ "detail": true, "limit": 30, "cursor": cursor }),
        )
        .unwrap_err();
        assert!(err.contains("another query"), "{err}");
        for bad in ["7", "c2.7.abc", "c1.x.abc", "c1.1.2.3"] {
            let err = req("list_sessions", json!({ "cursor": bad })).unwrap_err();
            assert!(err.contains("isn't a cursor"), "{bad}: {err}");
        }
    }

    #[test]
    fn a_result_under_its_budget_is_byte_identical() {
        let value = rows(3, 100);
        let (r, _) = req("t", json!({})).unwrap();
        let fitted = fit_json(
            &r,
            &value,
            "/sessions",
            false,
            Some(&head),
            None,
            "h",
            &pretty,
        )
        .unwrap();
        assert_eq!(fitted.text, pretty(&value));
        assert!(!fitted.envelope.budgeted);
        assert!(fitted.envelope.complete());
        assert_eq!(
            fitted.envelope.block()["text"],
            json!(r#"{"envelope":{"budgeted":false,"capReasons":[],"complete":true}}"#)
        );
    }

    #[test]
    fn a_long_list_pages_under_its_budget_and_every_item_comes_back_once() {
        let value = rows(60, 2_000);
        let mut cursor: Option<String> = None;
        let mut seen: Vec<String> = Vec::new();
        for _ in 0..60 {
            let mut args = json!({ "detail": true });
            if let Some(c) = &cursor {
                args["cursor"] = json!(c);
            }
            let (r, _) = req("list_sessions", args).unwrap();
            let fitted =
                fit_json(&r, &value, "/sessions", false, None, None, "h", &pretty).unwrap();
            assert!(
                fitted.text.len() <= DEFAULT_MAX_BYTES,
                "{}",
                fitted.text.len()
            );
            let page: Value = serde_json::from_str(&fitted.text).unwrap();
            for row in page["sessions"].as_array().unwrap() {
                seen.push(row["key"].as_str().unwrap().to_string());
            }
            cursor = fitted.envelope.next_cursor.clone();
            if cursor.is_none() {
                break;
            }
            assert!(fitted.envelope.budgeted);
            assert_eq!(fitted.envelope.cap_reasons, vec!["byte_budget"]);
            assert!(!fitted.envelope.complete());
        }
        let want: Vec<String> = (0..60).map(|i| format!("s-{i}")).collect();
        assert_eq!(seen, want);
    }

    #[test]
    fn items_past_the_whole_ones_go_out_headers_only() {
        let value = rows(40, 3_000);
        let (r, _) = req("t", json!({})).unwrap();
        let fitted = fit_json(
            &r,
            &value,
            "/sessions",
            false,
            Some(&head),
            None,
            "read it with x",
            &pretty,
        )
        .unwrap();
        let page: Value = serde_json::from_str(&fitted.text).unwrap();
        let list = page["sessions"].as_array().unwrap();
        assert!(fitted.envelope.headers_only > 0);
        assert!(list.last().unwrap()["headerOnly"] == json!(true));
        assert!(list[0].get("body").is_some(), "whole items first");
        assert!(fitted.envelope.to_value()["hint"]
            .as_str()
            .unwrap()
            .contains("read it with x"));
        assert!(fitted.text.len() <= DEFAULT_MAX_BYTES);
        // Paging with the headers on still returns every item whole once.
        let mut whole: Vec<String> = Vec::new();
        let mut cursor = fitted.envelope.next_cursor.clone();
        whole.extend(
            list.iter()
                .filter(|r| r.get("body").is_some())
                .map(|r| r["key"].as_str().unwrap().to_string()),
        );
        while let Some(c) = cursor {
            let (r, _) = req("t", json!({ "cursor": c })).unwrap();
            let page = fit_json(
                &r,
                &value,
                "/sessions",
                false,
                Some(&head),
                None,
                "h",
                &pretty,
            )
            .unwrap();
            let parsed: Value = serde_json::from_str(&page.text).unwrap();
            whole.extend(
                parsed["sessions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|r| r.get("body").is_some())
                    .map(|r| r["key"].as_str().unwrap().to_string()),
            );
            cursor = page.envelope.next_cursor;
        }
        assert_eq!(whole, (0..40).map(|i| format!("s-{i}")).collect::<Vec<_>>());
    }

    #[test]
    fn from_end_keeps_the_newest_items_whole() {
        let value = json!({ "turns": (0..10).map(|i| json!({ "id": format!("m-{i}"), "text": "y".repeat(5_000) })).collect::<Vec<_>>() });
        let h = |row: &Value| header_of(row, &["id"]);
        let (r, _) = req("read_session", json!({ "key": "k" })).unwrap();
        let fitted = fit_json(&r, &value, "/turns", true, Some(&h), None, "h", &pretty).unwrap();
        let page: Value = serde_json::from_str(&fitted.text).unwrap();
        let list = page["turns"].as_array().unwrap();
        assert_eq!(
            list.last().unwrap()["id"],
            json!("m-9"),
            "oldest first, as before"
        );
        assert!(
            list.last().unwrap().get("text").is_some(),
            "the newest is whole"
        );
        assert_eq!(list[0]["headerOnly"], json!(true), "older ones are headers");
        assert!(fitted.text.len() <= DEFAULT_MAX_BYTES);
    }

    #[test]
    fn one_item_over_the_budget_still_goes_out_and_says_so() {
        let value = rows(2, 30_000);
        let (r, _) = req("t", json!({})).unwrap();
        let fitted = fit_json(&r, &value, "/sessions", false, None, None, "h", &pretty).unwrap();
        assert_eq!(fitted.envelope.returned, 1);
        assert!(fitted.envelope.cap_reasons.contains(&"item_over_budget"));
        assert!(fitted.envelope.next_cursor.is_some());
    }

    #[test]
    fn a_source_limit_is_a_row_limit_even_when_the_text_fits() {
        let value = rows(2, 10);
        let (r, _) = req("t", json!({})).unwrap();
        let fitted = fit_json(
            &r,
            &value,
            "/sessions",
            false,
            None,
            Some("raise limit"),
            "h",
            &pretty,
        )
        .unwrap();
        assert_eq!(fitted.text, pretty(&value));
        assert_eq!(fitted.envelope.cap_reasons, vec!["row_limit"]);
        assert!(!fitted.envelope.complete());
        assert!(!fitted.envelope.budgeted);
    }

    #[test]
    fn a_cursor_past_the_end_is_refused() {
        let value = rows(2, 10);
        let (first, _) = req("t", json!({})).unwrap();
        let (r, _) = req("t", json!({ "cursor": first.cursor_at(5) })).unwrap();
        let err = fit_json(&r, &value, "/sessions", false, None, None, "h", &pretty).unwrap_err();
        assert!(err.contains("past the end"), "{err}");
    }

    // ── TC1: byte-stable tool lists and guidance ───────────────────────────

    /// Every owned server's tools/list answer for each access level, and
    /// the static guidance text the run prompt carries, as one string. Two
    /// fresh processes must print the same bytes.
    pub(crate) fn contract_snapshot() -> String {
        let mut out = String::new();
        for (name, list) in tool_lists() {
            out.push_str(&format!("== {name}\n{list}\n"));
        }
        for perm in crate::app_mcp::PERMISSION_MODES {
            for automation in [false, true] {
                for read_only in [false, true] {
                    out.push_str(&format!(
                        "== app guidance {perm} automation={automation} read_only={read_only}\n{}\n",
                        crate::app_mcp::prompt_guidance(perm, automation, read_only)
                    ));
                }
            }
        }
        for read_only in [false, true] {
            out.push_str(&format!(
                "== app chat guidance read_only={read_only}\n{}\n",
                crate::app_mcp::chat_prompt_guidance(read_only)
            ));
        }
        out.push_str(&format!(
            "== tasks guidance\n{}\n{}\n",
            crate::tasks_mcp::PROMPT_GUIDANCE,
            crate::tasks_mcp::PROMPT_GUIDANCE_READ_ONLY
        ));
        out.push_str(&format!(
            "== envelope\n{}\n",
            json!(schema_properties()
                .iter()
                .map(|(k, v)| json!([k, v]))
                .collect::<Vec<_>>())
        ));
        out
    }

    /// Each owned server's tools/list JSON, by server and access level.
    fn tool_lists() -> Vec<(String, String)> {
        let list = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
        let mut out = Vec::new();
        for (access, a) in [
            ("read", crate::app_mcp::AppAccess::Read),
            ("write", crate::app_mcp::AppAccess::Write),
        ] {
            for depth in [0, crate::app_mcp::MAX_START_DEPTH] {
                let ctx = crate::app_mcp::Ctx {
                    access: a,
                    depth,
                    broker: None,
                };
                let answer = crate::app_mcp::handle(&list, &ctx).unwrap();
                out.push((
                    format!("hollow_app {access} depth {depth}"),
                    answer.to_string(),
                ));
            }
        }
        for (access, a) in [
            ("read", crate::tasks_mcp::TasksAccess::ReadOnly),
            ("write", crate::tasks_mcp::TasksAccess::ReadWrite),
        ] {
            let ctx = crate::tasks_mcp::Ctx {
                vault_root: None,
                access: a,
                client: None,
            };
            let answer = crate::tasks_mcp::handle(&list, &ctx).unwrap();
            out.push((format!("hollow_tasks {access}"), answer.to_string()));
        }
        for (label, scope, scratch) in [
            ("any project", None, None),
            (
                "any project, scratch",
                None,
                Some(PathBuf::from("/scratch")),
            ),
            ("disconnected", Some("gone".to_string()), None),
        ] {
            let target = crate::vault_mcp::Target {
                root: PathBuf::from("/nonexistent-hollow-vault-tc1"),
                scope,
                workspace: None,
                skills: None,
                app: false,
                scratch,
            };
            let answer = crate::vault_mcp::handle(&list, &target).unwrap();
            out.push((format!("hollow_vault {label}"), answer.to_string()));
        }
        // CC2: one list whatever the checkout.
        let target = crate::code_context::Target::default();
        let answer = crate::code_context_mcp::handle(&list, &target).unwrap();
        out.push(("hollow_code_context".to_string(), answer.to_string()));
        out
    }

    /// OB1: each owned server's tools/list bytes stay under a cap, set a
    /// little above the size measured on 2026-11-17 with OB1's two envelope
    /// arguments in (printed by `cargo test --lib tool_list_bytes --
    /// --nocapture`). A tool list is sent with every turn (Codex lists every
    /// schema; Claude loads the always-load set), so growing one is a
    /// decision: raise its cap here in the same change, with the reason.
    #[test]
    fn tool_list_bytes_stay_under_their_caps() {
        // Measured 2026-11-17 with OB1's two arguments in (they add about
        // 200 bytes per enveloped tool: two in hollow_app and hollow_tasks,
        // one in hollow_vault), in bytes: app read 9163 (depth 0 and 2),
        // write 25022 at depth 0 and 18409 at depth 2; tasks read 2797,
        // write 12052; vault 4389, 4503 with scratch, 4364 disconnected.
        // Each cap is about 3% over. OR1 (2026-11-17) added the optional
        // `handoff` argument to start_session, send_to_session and
        // sessions.handover (a short schema): app write at depth 0 measured
        // 26730 after it, so its cap rose to 27550; depth 2 (18505) fits.
        // CM1c (2026-11-17) added the run_script tool (about 790 bytes with
        // its schema): read 9952, write 27423 at depth 0 and 19198 at depth
        // 2, so those caps rose to 10250, 28250 and 19800.
        let caps: &[(&str, usize)] = &[
            ("hollow_app read depth 0", 10_250),
            ("hollow_app read depth 2", 10_250),
            ("hollow_app write depth 0", 28_250),
            ("hollow_app write depth 2", 19_800),
            ("hollow_tasks read", 2_900),
            ("hollow_tasks write", 12_450),
            ("hollow_vault any project", 4_550),
            ("hollow_vault any project, scratch", 4_650),
            ("hollow_vault disconnected", 4_500),
            // CC2 (2026-11-18): six tools with strict schemas and the two
            // envelope arguments on five measured 4674; cap about 3% over.
            // CC4 (2026-11-18): the `slice` tool (six arguments and the
            // two envelope ones) measured 5781; cap about 3% over.
            // CC5 (2026-11-18): `changes` and `affected_tests` (two and
            // four arguments, plus the envelope's) measured 7340; cap about
            // 3% over.
            ("hollow_code_context", 7_560),
        ];
        let lists = tool_lists();
        let mut report = Vec::new();
        for (name, list) in &lists {
            report.push(format!("{name}: {}", list.len()));
        }
        eprintln!("tool list bytes:\n{}", report.join("\n"));
        assert_eq!(lists.len(), caps.len(), "every list has a cap");
        for (name, list) in &lists {
            let cap = caps
                .iter()
                .find(|(n, _)| n == name)
                .unwrap_or_else(|| panic!("no cap for {name}"))
                .1;
            assert!(
                list.len() <= cap,
                "{name}: {} bytes, over its cap of {cap}",
                list.len()
            );
        }
    }

    /// Set in the child process [`contract_is_byte_stable_across_processes`]
    /// starts: print the snapshot instead of testing.
    const PRINT_ENV: &str = "HOLLOW_TC1_PRINT";
    /// Set with [`PRINT_ENV`] to plant a `HashMap` iteration in the
    /// snapshot, which must make two processes differ.
    const PLANT_ENV: &str = "HOLLOW_TC1_PLANT";
    const BEGIN: &str = "<<<TC1-SNAPSHOT\n";
    const END: &str = "\nTC1-SNAPSHOT>>>";

    /// The child half of TC1: prints [`contract_snapshot`] between markers
    /// when [`PRINT_ENV`] is set; does nothing in a normal run.
    #[test]
    fn tc1_print_contract_snapshot() {
        if std::env::var_os(PRINT_ENV).is_none() {
            return;
        }
        let mut snapshot = contract_snapshot();
        if std::env::var_os(PLANT_ENV).is_some() {
            let planted: std::collections::HashMap<String, usize> =
                (0..64).map(|i| (format!("tool_{i}"), i)).collect();
            for (k, v) in &planted {
                snapshot.push_str(&format!("{k}={v}\n"));
            }
        }
        println!("{BEGIN}{snapshot}{END}");
    }

    /// Run [`tc1_print_contract_snapshot`] in a fresh copy of this test
    /// binary and return what it printed between the markers.
    fn snapshot_from_fresh_process(plant: bool) -> String {
        let exe = std::env::current_exe().expect("test binary path");
        // The test's path without the crate name.
        let module = module_path!()
            .split_once("::")
            .map_or(module_path!(), |(_, rest)| rest);
        let name = format!("{module}::tc1_print_contract_snapshot");
        let mut cmd = std::process::Command::new(exe);
        cmd.args(["--exact", &name, "--nocapture", "--test-threads=1"])
            .env(PRINT_ENV, "1");
        if plant {
            cmd.env(PLANT_ENV, "1");
        } else {
            cmd.env_remove(PLANT_ENV);
        }
        let output = cmd.output().expect("spawn the test binary");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(output.status.success(), "child failed: {stdout}");
        let start = stdout
            .find(BEGIN)
            .unwrap_or_else(|| panic!("no snapshot in: {stdout}"));
        let rest = &stdout[start + BEGIN.len()..];
        let end = rest.find(END).expect("snapshot end marker");
        rest[..end].to_string()
    }

    /// TC1: the owned servers' tool lists and the static guidance are the
    /// same bytes in two fresh processes (prompt caches stay warm across
    /// runs), and a planted unordered map breaks that, so the check bites.
    #[test]
    fn contract_is_byte_stable_across_processes() {
        if std::env::var_os(PRINT_ENV).is_some() {
            return;
        }
        let a = snapshot_from_fresh_process(false);
        let b = snapshot_from_fresh_process(false);
        assert!(
            a.len() > 10_000,
            "the snapshot has every list: {} bytes",
            a.len()
        );
        if let Some(diff) = first_difference(&a, &b) {
            panic!("the MCP contract differs between two processes: {diff}");
        }
        assert_eq!(a, contract_snapshot(), "and matches this process");
        let planted_a = snapshot_from_fresh_process(true);
        let planted_b = snapshot_from_fresh_process(true);
        assert!(
            first_difference(&planted_a, &planted_b).is_some(),
            "a HashMap's order differs between processes, so the planted snapshot must too"
        );
    }

    // ── BI5: refusal codes ─────────────────────────────────────────────────

    /// Representative refusals of every path in the three servers (their
    /// own texts, the app broker's and the task broker's), with the code and
    /// next each must resolve to.
    #[test]
    fn every_refusal_resolves_to_a_code_of_the_closed_set() {
        use Server::{App, Code, Tasks, Vault};
        let unknown = crate::app_mcp::unknown_action_for_tests("sessions.start_session");
        let cases: &[(Server, &str, &str, &str)] = &[
            (App, unknown.as_str(), "UNKNOWN_ACTION", "list_actions"),
            (App, "unknown action `zzz`; list_actions shows every action id.", "UNKNOWN_ACTION", "list_actions"),
            (App, "unknown tool: frobnicate", "UNKNOWN_TOOL", "list_actions"),
            (App, "sessions.start: unknown argument `prompts` (expected: workspace, prompt).", "INVALID_ARGUMENT", "describe_action"),
            (App, "sessions.start: `workspace` is required (arguments: workspace, prompt).", "INVALID_ARGUMENT", "describe_action"),
            (App, "sessions.wait: `timeoutSeconds` must be a whole number from 0 to 300.", "INVALID_ARGUMENT", "describe_action"),
            (App, "list_actions: `tier` must be read, write, confirm or external (got `x`).", "INVALID_ARGUMENT", "describe_action"),
            (App, "list_actions: `cursor` must be a nextCursor from an earlier page.", "CURSOR_INVALID", "retry"),
            (App, "list_sessions: this cursor belongs to another query (another tool, or other arguments than the call that returned it).", "CURSOR_INVALID", "retry"),
            (App, "no actions in area `zz`; areas: sessions, vault.", "NOT_FOUND", "list_actions"),
            (App, "sessions.start changes Hollow, and this run can only read it (plan mode, a build workspace's turn, or an automation limited to reading). Nothing ran; describe the change in your reply instead.", "NOT_PERMITTED", "reply"),
            (App, crate::app_broker::RUN_ENDED, "NOT_PERMITTED", "reply"),
            (App, "The user declined sessions.start; nothing changed.", "NOT_PERMITTED", "reply"),
            (App, "No answer on the card for sessions.start within two minutes; nothing changed.", "NOT_PERMITTED", "reply"),
            (App, "The card for sessions.start was cancelled (the turn stopped); nothing changed.", "NOT_PERMITTED", "reply"),
            (App, "sessions.start needs the person's Allow on a card, but Hollow's window is closed.", "NOT_PERMITTED", "reply"),
            (App, "x needs the user's approval, which an automation can't ask for.", "NOT_PERMITTED", "reply"),
            (App, "This session was itself started by an agent 2 levels deep; sessions started this deep can't start or message others. Nothing started.", "NOT_PERMITTED", "sessions.handover"),
            (App, "Hollow app tools are unavailable in this run: Hollow's app broker isn't connected. Nothing ran.", "UNAVAILABLE", "retry"),
            (App, "Hollow did not answer within 60 seconds.", "UNAVAILABLE", "retry"),
            (App, "Hollow closed the connection before answering.", "UNAVAILABLE", "retry"),
            (App, "Hollow's app broker sent an unreadable answer (eof).", "INTERNAL", "retry"),
            (App, "`abc` is not a session key (list_sessions shows them). Nothing changed.", "NOT_FOUND", "list_sessions"),
            (App, "REQUEST_ID_REUSED: requestId `abcdefgh` was already used for a different sessions.start call.", "REQUEST_ID_REUSED", "sessions.request_status"),
            (App, "REQUEST_PENDING: the sessions.start call with requestId `abcdefgh` started.", "REQUEST_PENDING", "sessions.request_status"),
            (App, "TASK_INVALID: priority 'urgent' is not one of low, medium, high", "TASK_INVALID", "describe_action"),
            (App, "Something no rule names.", "REFUSED", "reply"),
            (Tasks, "TASK_CONFLICT: tasks/a.md changed on disk after it was opened.", "TASK_CONFLICT", "get_task"),
            (Tasks, "TASK_INVALID: priority 'p0' is not one of low, medium, high", "TASK_INVALID", "retry"),
            (Tasks, "TASK_LINKED: its tracker issue holds status.", "TASK_LINKED", "get_task"),
            (Tasks, "TASK_CYCLE: A > B > A", "TASK_CYCLE", "get_task"),
            (Tasks, "TASK_BLOCKER: no task named x.", "TASK_BLOCKER", "list_tasks"),
            (Tasks, "TASK_KEY: an idempotencyKey is required; reuse the same key when you retry.", "TASK_KEY", "retry"),
            (Tasks, "IDEMPOTENCY_MISMATCH: this idempotency key already created \"a\".", "IDEMPOTENCY_MISMATCH", "retry"),
            (Tasks, "TASK_CLAIMED: claimed by chat x.", "TASK_CLAIMED", "next_claimable"),
            (Tasks, "This run can read tasks but not change them (plan mode). Nothing was written.", "NOT_PERMITTED", "reply"),
            (Tasks, "Task writes are unavailable in this run: Hollow's task broker isn't connected.", "UNAVAILABLE", "retry"),
            (Tasks, "Hollow's vault is unavailable to this server (HOLLOW_VAULT_ROOT is missing).", "UNAVAILABLE", "retry"),
            (Tasks, "get_task: ref is required (a task's ref from list_tasks).", "INVALID_ARGUMENT", "retry"),
            (Tasks, "list_tasks: limit must be a positive whole number.", "INVALID_ARGUMENT", "retry"),
            (Tasks, "tasks/x.md is not a readable task (it has no title).", "NOT_FOUND", "list_tasks"),
            (Tasks, "projects/x.md is not a readable project (it has no title).", "NOT_FOUND", "list_projects"),
            (Tasks, "unknown tool: nope", "UNKNOWN_TOOL", "tools/list"),
            (Tasks, "list_tasks: unknown argument `max_bytes`; did you mean `maxBytes` (the text budget)?", "INVALID_ARGUMENT", "retry"),
            (Tasks, "Hollow's task broker sent an unreadable answer (x).", "INTERNAL", "retry"),
            (Vault, "Refused: read_source reads only this conversation's workspace.", "NOT_PERMITTED", "reply"),
            (Vault, "read_skill: no skill named `x` in the vault's .agents/skills (there are none).", "NOT_FOUND", "retry"),
            (Vault, "read_skill: `source` must be vault or shared (got `x`).", "INVALID_ARGUMENT", "retry"),
            (Vault, "read_skill: no reference named `x` in skill `y` (it has none).", "NOT_FOUND", "retry"),
            (Vault, "`query` must be at least 2 characters", "INVALID_ARGUMENT", "retry"),
            (Vault, "from_line 90 is past the end: a.md has 3 lines.", "INVALID_ARGUMENT", "retry"),
            (Vault, "a.pdf is not text: it needs a reader or a text export.", "INVALID_ARGUMENT", "reply"),
            (Vault, "search_vault: `cursor` `7` isn't a cursor this tool gave.", "CURSOR_INVALID", "retry"),
            (Vault, "Hollow's vault is unavailable to this server (HOLLOW_VAULT_ROOT is missing). Nothing was searched.", "UNAVAILABLE", "retry"),
            (Vault, "unknown tool: nope", "UNKNOWN_TOOL", "tools/list"),
            (Code, "callers: unknown argument \"qualifer\"; did you mean \"qualifier\"?", "INVALID_ARGUMENT", "retry"),
            (Code, "callees: no definition named `x` here (symbols finds names by their words).", "NOT_FOUND", "symbols"),
            (Code, "map: no such indexed file or folder `x`.", "NOT_FOUND", "symbols"),
            (Code, "impact: more than 200 functions reach `x` by depth 2, over the impact cap; ask callers on the busiest first.", "INVALID_ARGUMENT", "callers"),
            (Code, "callers: the code index is unavailable: git ls-files failed.", "UNAVAILABLE", "retry"),
            (Code, "callers: this cursor belongs to another query.", "CURSOR_INVALID", "retry"),
            (Code, "changes: no such commit `zz` here (base takes a git rev: a branch, tag or commit; default the merge-base with origin/HEAD).", "NOT_FOUND", "symbols"),
            (Code, "changes: 2400 changed files, over the changes cap of 2000; pass paths, or a nearer base.", "INVALID_ARGUMENT", "retry"),
            (Code, "affected_tests: `symbols` must be given alone: pass symbols, or base and paths, not both.", "INVALID_ARGUMENT", "retry"),
            (Code, "unknown tool: nope", "UNKNOWN_TOOL", "tools/list"),
        ];
        for (server, text, code, next) in cases {
            let got = classify(*server, text);
            assert_eq!((got.code, got.next), (*code, *next), "{server:?} {text}");
            assert!(refusal_code(got.code).is_some(), "{text}");
        }
        let got = classify(App, &unknown);
        assert!(
            got.did_you_mean.contains(&"sessions.start".to_string()),
            "{unknown}: {got:?}"
        );
        assert!(got.did_you_mean.len() <= 3);
        assert_eq!(
            classify(Tasks, "list_tasks: unknown argument `max_bytes`; did you mean `maxBytes` (the text budget)?").did_you_mean,
            vec!["maxBytes".to_string()]
        );
    }

    /// The rule table names only codes of the set, every code is documented
    /// in agent-tools.md, and a next is never empty.
    #[test]
    fn the_refusal_table_is_closed_and_documented() {
        for (phrase, code, _) in REFUSAL_RULES {
            assert!(refusal_code(code).is_some(), "{phrase}: {code}");
            assert_eq!(phrase.to_lowercase(), *phrase, "rules match lower case");
        }
        let doc = include_str!("../../docs/reference/agent-tools.md");
        for c in REFUSAL_CODES {
            assert!(
                c.next.iter().all(|n| !n.is_empty()) && !c.meaning.is_empty(),
                "{}",
                c.code
            );
            assert!(
                doc.contains(&format!("`{}`", c.code)),
                "{} is not in agent-tools.md",
                c.code
            );
        }
        assert_eq!(
            REFUSAL_CODES.last().unwrap().code,
            "REFUSED",
            "the fallback is last"
        );
    }

    /// The refusal block: the sentence first, unchanged; the code block
    /// last, byte for byte the same each time (sorted keys).
    #[test]
    fn a_refusal_keeps_its_sentence_first_and_adds_a_stable_code_block() {
        let text = "unknown action `sessions.strat`; did you mean `sessions.start`? list_actions shows every action id.";
        let content = refusal_content(Server::App, text);
        let blocks = content.as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0], json!({ "type": "text", "text": text }));
        assert_eq!(
            blocks[1]["text"],
            json!(
                r#"{"error":{"code":"UNKNOWN_ACTION","didYouMean":["sessions.start"],"next":"list_actions"}}"#
            )
        );
        assert_eq!(
            classify(Server::Vault, "x is not text").block()["text"],
            json!(r#"{"error":{"code":"INVALID_ARGUMENT","next":"reply"}}"#)
        );
    }

    #[test]
    fn first_difference_names_where_two_texts_part() {
        assert_eq!(first_difference("same", "same"), None);
        let diff = first_difference("{\"a\":1,\"b\":2}", "{\"b\":2,\"a\":1}").unwrap();
        assert!(diff.contains("byte 2"), "{diff}");
        let diff = first_difference("abc", "abcd").unwrap();
        assert!(
            diff.contains("byte 3") && diff.contains("lengths 3 and 4"),
            "{diff}"
        );
        // A map whose order is its hasher's: two maps of the same keys built
        // with different seeds render differently, and the helper catches it.
        use std::collections::HashMap;
        use std::hash::RandomState;
        let render = |m: &HashMap<String, usize, RandomState>| {
            m.iter()
                .map(|(k, v)| format!("{k}={v};"))
                .collect::<String>()
        };
        let mut one = HashMap::with_hasher(RandomState::new());
        let mut two = HashMap::with_hasher(RandomState::new());
        for i in 0..64 {
            one.insert(format!("k{i}"), i);
            two.insert(format!("k{i}"), i);
        }
        assert!(first_difference(&render(&one), &render(&two)).is_some());
    }
}
