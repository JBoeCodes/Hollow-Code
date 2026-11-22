# Records (master plan F4)

`src-tauri/src/records.rs`. Decisions and facts about people, accepted by the
person from extraction proposals (F3), and the generated part of each
person's note.

## Files

| Record | Where | Frontmatter |
|---|---|---|
| Decision | `decisions/<date>-<slug>.md` | `type: decision`, `title`, `decided_at`, `decided_by` (a name or `unknown`), `scope` (`global` or a workspace id), `status` (`settled`, `superseded`, `revisit`, `retracted`), `supersedes`, `superseded_by`, `evidence` (anchor), `source`, `accepted_at`, and, all optional (DR1, below), `rejected`, `constraint`, `derives_from` |
| Person fact | `records/facts/<date>-<slug>.md` | `type: person-fact`, `title`, `person` (`people/<Name>.md`), `stated_at`, `evidence`, `source`, `accepted_at` |

Every record links its anchor (`<transcript>@MM:SS` or `<session>#turnN`); the
body ends `Evidence: “quote” — anchor`.

## Rejected options, constraints, links and retraction (DR1, 2026-11-17)

Four optional frontmatter keys on a decision. A record without them parses and
recalls exactly as before.

| Key | Form | Meaning |
|---|---|---|
| `rejected` | one line, `option -> reason; option -> reason` | Options turned down and why. The reason is optional (`option` alone is fine). A `;` ends an entry and `->` splits it, so neither can appear inside a part. At most 8 entries, an option clipped to 200 characters and a reason to 300. One line because `frontmatter::parse_map` reads only `key: value` lines and skips list items. |
| `constraint` | `true` | A hard constraint. Recall never cuts it (below). |
| `derives_from` | comma-separated vault paths (`vault:` prefix allowed) | The findings or records this decision answers. A link only: nothing follows it yet. |
| `issue`, `why`, `verified` | one line each | The build-log comment's source (DR1b): the tracker issue key (`ENG-12`) it goes on, why it was decided and how it was checked. An agent may attach them to a proposal (`input.issue`, `input.why`, `input.verified`); written when the person accepts. Without `issue` nothing is ever made. |
| `status: retracted` | | Withdrawn. The file stays; nothing deletes it. Setting it writes `retracted_at: YYYY-MM-DD`; settling again removes that key. |

- **Who sets what.** `constraint: true` and `retracted` are a person's: a
  person edits the record, or (retraction) uses the `records_set_decision_status`
  command. No agent action sets `constraint`; the agent action
  `records.records_set_decision_status` still offers only `settled` and
  `revisit` (always on a card). An agent may attach `rejected` to a proposal
  (`records.records_propose_decision`'s `input.rejected`, same one-line form); it
  is written to the record only when the person accepts. A person can settle a
  retracted decision again; a retracted one cannot be superseded (or be the
  replacement) until then. Retracting a decision that replaced another
  (`supersedes` set) is refused ("Retract is for a decision that replaced
  nothing"), so no older record is left superseded by a withdrawn one.
  **Trust:** `constraint` and `retracted` are person-set by convention, at the
  same trust as every other frontmatter field: nothing in Hollow stops a
  vault write (an agent's file edit) from adding them. What Hollow guarantees is
  that no Hollow action sets them for an agent and that an agent's proposal
  never carries `constraint`.
- **Review card.** A record card shows the proposal's `rejected` text
  ("Rejected: option (reason); ...", plain text, read only) before the person
  accepts. Accepting reads it again with the same parse and 8-entry cap, so the
  card item can't write a different line than a proposal could.
- **Recall.** The statement carries the state in plain words. A superseded
  record starts "Superseded by <path>: do not follow without confirmation." A
  retracted one starts "Retracted: this decision was withdrawn; do not follow it
  without confirmation." and says "Decided" instead of "Settled". Rejected
  options end it: "Rejected: option (reason); option (reason). Do not repeat these
  without confirmation." A constraint starts "Constraint: ". The rejected text
  does not make a request match (a decision is matched on its own title only).
- **Retracted is history, not hidden and not current.** Like a superseded
  record it is left out of a current-state request and recalled, labelled
  "historical: retracted", for a past-tense request that names it. (`revisit`
  stays out of recall entirely: it is not settled, and a withdrawn decision is
  worth showing as withdrawn so nobody re-adopts it blind.) The Library and a
  workspace's decision list leave it out like a superseded one.
- **The budget never cuts a constraint.** Of the recall's two caps (5 items,
  3,000 characters), a settled constraint that the request matches counts
  against them but is never dropped by them: the other entries share the slots
  and characters that remain. With more than five matching constraints, all are
  kept, up to 12 (the 5-item cap is then exceeded on purpose); a matching
  constraint past 12 is cut, best-ranked 12 kept, and the run's recall decision
  log records each cut (`status: constraint`, `action: cut`). **A constraint is
  recalled only when the request shares words with it** (decisions are not
  global, so it is not injected into every turn): a constraint whose words a
  request never uses is not delivered. A superseded or retracted constraint is no longer binding and has
  no exemption.
- **As of.** A retracted decision reads as settled for days before its
  `retracted_at` (as for `superseded_at`); one with no `retracted_at` (edited by
  hand) reads as retracted on every day. The recall's prompt block says retracted
  entries "were withdrawn with no replacement", not "replaced by the entry they
  name". The weekly review leaves retracted decisions out, like superseded ones.
- **Build-log comment (DR1b, 2026-11-17).** A decision that names a tracker
  issue (`issue:`) with an agent run linked to it makes one T3g build log comment
  when it is recorded (accepted) or its status changes (settled, revisit,
  retracted, superseded; a repeat of the same status makes none): `What:`,
  `Why:` (the record's `why`, else `see <evidence>`), `Rejected:` (left out when
  there is none) and `Verified:` (`no check recorded` when blank). Same gate as
  every T3g comment ([TRACKER_SYNC](TRACKER_SYNC.md#build-log-comments-t3g-2026-11-15)):
  `trackAgentWork` off makes nothing, `dryRun` (default) one `tracker_write`
  audit row and no request, `on` queues it in the outbox. Code:
  `tracker/decision_log.rs`. Not done: a list showing rejected options.

## Accepting (the person only)

**Quotes are checked again at accept (EV1, 2026-11-16).** Extraction already
drops a proposal whose quote isn't in the segment it cites; accept now checks
it again, because a transcript can be re-synced or a turn change between the
extraction and the click (`quote_check.rs`). An agent's
`records.propose_decision` carries one free-text string as both quote and
anchor, so it is not covered. For an anchor
Hollow can read, `<session>#turn<N>` (with or without `chat:`) or
`<note>.md@<MM:SS>`, the quote must appear in the text there (spaces and case
folded), else the accept is refused before anything is claimed or written: "The
quote isn't at <anchor>. The nearest line there is “…”. Nothing was saved. Reject
this proposal and have it proposed again from the source, or fix the source text
it quotes." The card can't edit a quote or an anchor. The item stays pending. Not
checked, and accepted as before: any other anchor (a file path, `plaud:<id>@…`,
an agent's free text), a conversation or transcript that can't be read now, and
a quote that is its own anchor. Not covered yet: memory files and Linear
build-log comments, which carry quotes no anchor format ties to a source.

A `record` review item is accepted through `resolve_review_item` →
`records::accept_proposal` (claim, create, accept; a failed create returns the
item to pending). Edits: `text`, `owner` (decided by), `scope`, `person`.

- A decision's scope defaults to the recording's workspace
  (`recording:workspaces/<id>/…`), else `global`.
- A fact needs its person: an edited `person` path, or exactly one
  `people/` note whose title or file name is the counterparty's name. Anything
  else refuses ("Choose who this is about"): never a first-name guess.

## Decisions

- `records.records_decisions` (read): filter by scope (a workspace also sees
  global ones), status and words.
- `records.records_supersede_decision` (confirm, **always asks**, every mode):
  `old` → `superseded` + `superseded_by`; `new` gets `supersedes`. Refuses a
  self-supersede, an already superseded `old`, a `new` that isn't settled or
  already supersedes another.
- `records_set_decision_status`: settle again or mark `revisit` (or, the person's command only, `retracted`: "A decision can be set to settled, revisit or retracted"); also the agent action `records.records_set_decision_status`, which offers only `settled` and `revisit` and always asks the person on a card (COO rebuild L4).
  A superseded decision stays superseded.

## People notes

`people/<Name>.md` gets one block between
`<!-- hollow:generated:start · … -->` and `<!-- hollow:generated:end -->`:
"From your records" → Owes you, You owe (open commitments), Their decisions,
Facts you accepted. Everything outside the block stays byte-for-byte
(`splice_generated`; appended after a blank line the first time). It is
refreshed when a record naming the person is accepted or superseded.
`records.records_people` and `records.records_person` (read) return the same
data.

## Agents see relevant decisions

`records::decision_contexts` feeds `vault_learning::recall_with`: settled
decisions are `<trusted-recall>` entries ("Settled <date>[ in <scope>]: <text>
(decided by …, evidence …; see <path>)"), non-global, so only a request that
shares words with one (or names its path) recalls it; an unrelated decision
costs no bytes. Superseded ones are history (a past-tense request only, behind
the replacement), as are retracted ones; `revisit` ones are left out. The
recall's own caps apply (5 items, 3,000 characters), except that a constraint is
never cut by them (DR1, above).

## Review fixes (2026-10-09)

- A hand-broken fence (a start marker with no end after it, or two blocks) is
  refused, never spliced: the person fixes the note and Hollow writes again.
  An end marker the person quoted above the block stays theirs.
- Dates are local days; an imported brief bullet keeps its own date (if not
  ahead).
- Recall matches a decision on its own words only (not the "Settled …,
  decided by …" framing), and a workspace's decisions only in its own chats
  (`recall_with_scope`; global ones everywhere).

## The decision ledger (#20)

- **Proposing.** `records.records_propose_decision` (write): `{text, evidence,
  decidedBy?, scope?, conflictsWith?}` queues a `record` item for the person.
  Evidence is required. A settled decision with the same wording refuses
  ("Already settled …: cite it instead"). A conflicting settled decision
  (`conflictsWith`, or `conflict_with`: close wording that isn't the same
  words, such as a changed value or an added "not") is named on the card as
  "Possibly contradicts settled decision …"; only the same words (case and
  punctuation aside) refuse;
  the settled one is never edited. Extraction (F3) flags the same way through
  its near-duplicate rule.
- **Importer** (on request): `records.records_import_brief_decisions
  {workspace}` turns each top-level bullet of the brief's first
  `## Decisions…` section (nested lines joined, bold removed) into a decision
  proposal, read as written (no model; its date becomes `decided_at` on
  accept): anchor
  `workspaces/<id>/WORKSPACE.md#decisions`, the first `YYYY-MM-DD` noted,
  scope the workspace. Repeats are skipped; the brief is never changed.
- **As of.** `records_decisions {asOf: YYYY-MM-DD}` reads decisions as they
  stood that day: not yet decided ones are left out; one superseded later
  (`superseded_at`, written by supersede) reads as settled.
- **`me/Decisions.md`.** When the file exists, its generated block lists the
  global settled decisions (newest 40) with links; the person's text above
  stays byte-for-byte. Refreshed on accept, supersede and status changes. The
  file is never created by Hollow.
- **Library, Records.** Library → Notes → Library folder → Records lists
  every decision that isn't superseded (revisit first, with its workspace or
  Everywhere) with Revisit / Settle again (`records_set_decision_status`;
  `components/RecordDecisions.tsx`). It was on a workspace's Overview until
  COO rebuild L4. Superseding is only on the always-ask card.

## People dossiers (#3)

`src-tauri/src/people_dossier.rs`.

- **Who a name means.** A name matches a person only when exactly one
  `people/` note has it as its title, file name or one of its `aliases:`
  (frontmatter; written as a YAML list). Never a first-name guess, and only a
  bare first name ("Ana") has candidates: "Ana Garcia" is never asked as Ana
  Ruiz. The card can always add a new page instead.
- **Questions, not guesses.** When extraction queues a promise or a fact, each
  named owner or counterparty (not the person) that matches no one becomes a
  `person_alias` review item, asked once per name (sticky): "Is 'Ana' Ana
  Ruiz?" (one note shares the first name), "Who is 'Raj'? 2 people could be"
  (pick under Edit), or "Add a page for 'Dana Wu'?" (none; pages grow by
  suggestion). Accepting links the alias (frontmatter only; the body stays
  byte-for-byte; a name another note already means is refused) or creates
  `people/<Name>.md`. Only through `resolve_review_item` (the person).
- **The generated block** adds **Last contact** (the newest kept recording
  transcript they speak in, by its `retrieved_at`, or the newest commitment or
  fact with them) and **Open threads** (open tasks and active workspace briefs
  naming them, whole words, at most 5 tasks).
- `records.records_refresh_people` (write) regenerates every block; nothing
  else in the notes changes.
- No HR data, no inference about performance or personal life: only what the
  person's own recordings, chats and records say.
