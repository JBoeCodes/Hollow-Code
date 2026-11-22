# Extraction (master plan F3)

`src-tauri/src/extract.rs`. What was said in a recording or a chat becomes
**proposals on the review queue**. Nothing becomes a record, task or
commitment until the person accepts it.

## Sources

| Source | Shape | Anchor |
|---|---|---|
| Recording | `{kind: "recording", path}`: a kept transcript `.md` in the vault, lines `[MM:SS - MM:SS] Speaker: text` (what `recordings::keep_transcript` writes) | `<path>@<MM:SS>` |
| Chat | `{kind: "chat", sessionKey, fromTurn?, toTurn?}`: one conversation's turns, 1-based by user message | `<key>#turn<n>` |
| Meeting recap | Click-only Plaud segments (`loaded_from_segments`), bounded before loading; ledger id `plaud:<id>` | `plaud:<id>@<MM:SS>` |

Meeting recaps call `queue_checked` after checking their single writer's proposals. It claims the
source, rechecks its hash, then shares extraction's dedup, Jev, submit, identity and ledger path.
Normal `extract.run` retains its original prompt and pre-model claim/ledger skip. The recap file is
the Review source path; evidence quotes remain on Review proposals, never an added transcript copy.
Recordings' local **Create tasks** parser does not write this ledger; existing tasks participate in
a later recap's local dedup, as do proposals from a separately kept transcript (`recording:<path>`).
The reverse order is not covered: Create tasks doesn't check pending recap proposals, and sufficiently
different model paraphrases may not match. Review before accepting potentially overlapping work.

Email is not a source here: it can only be read through the shared Superhuman
connector inside an agent turn (the email side of #2, Phase 3).

## Pipeline

1. **Read** the source into numbered segments. A source over 120,000
   characters is refused (extract a range).
2. **Skip** when `.hollow/extracted.json` holds the source id with the same
   content hash: no model call. A damaged ledger is an error, never a reset.
3. **Ask** the configured agent tool-free (`ConfiguredModel` →
   `claude_runner::run_oneshot`; `claude-haiku-5-5` when the agent is Claude,
   else the provider's default). Cost is metered as `oneshot` scope `extract`.
4. **Check** each proposal: kind is `commitment`, `task`, `decision` or
   `person_fact`; text one line ≤ 300; it cites a segment (`S3`) and its quote
   must appear in that segment (else dropped: "its quote isn't in the
   segment it cites"). Owner, counterparty and due stay `null` unless stated;
   a due date must be `YYYY-MM-DD`.
5. **Dedup**: within the batch; locally against tracked tasks and commitments,
   pending or reviewed task/commitment/record items (a rejected one stays
   rejected); then Jev when it's on. Exact wording, or near-identical wording
   (≥ 80% of meaningful words, numbers apart), is dropped, except a near match
   that changes a number, amount or date, which is queued with
   `possibleDuplicate` ("Possibly the same as …"; `jev_decide::changes_a_value`,
   the #405 rule).
6. **Route**:
   - `task` → a `meeting_task` review item with `anchor`; accepted through
     `task_broker::accept_extracted_task` (the agent-task create, idempotent).
     Old Outlook meeting tasks (no anchor) still refuse.
   - `commitment` → a `commitment` proposal (`vault_commitments::submit_extracted`):
     direction from the person's point of view (`me/Identity.md` name, or
     "me"/"I"). When the source doesn't say who owes whom, the proposal's
     direction is `unknown` and accepting it refuses until the person picks
     one on the card (#2: never guessed). A promise between two other people,
     or with no other party named, is dropped with the reason.
   - `decision`, `person_fact` → a `record` review item, accepted through F4
     (`records::accept_proposal`, [RECORDS.md](RECORDS.md)).
7. **Record** the source in the ledger with counts.

## Review fixes (2026-10-09)

- "The person" is the full name in `me/Identity.md` only: a pronoun in a
  recording is relative to whoever spoke, and a first name can be a
  colleague's. Anything else is `unknown`.
- Dedup compares a promise with its other party ("Send the slides · Ana" and
  "· Raj" are two), keeps two-letter names (Jo, Al), and checks only open
  tasks, open commitments and pending items (an exact repeat of a rejected
  item still lands on its sticky card). Fingerprints include owner and
  counterparty.
- A quote under three words drops the proposal.
- Jev compares with what existed before the source, and gets titles only,
  never what was said.
- A queue write that fails leaves the source unrecorded (the next run reads
  it again); refusals ("between two other people") don't. One run per source
  at a time. The ledger keeps the first 20 drop reasons, shown by
  `extract.status`.
- An extracted task keeps `anchor` through a failed claim, and its owner in
  the note.

## The commitment tracker (#2)

- Accepted promises are ordinary `commitments/*.md`; the context keeps
  `<source> (<at>): <anchor>`. Overdue nudges already fire (`nudges.rs`).
- Tasks page → **Commitments** tab (`CommitmentLedger.tsx`): I owe / Owed to me /
  All; Open, Snoozed and Closed; cited sources and the full lifecycle actions.
  This is the sole ledger home; the duplicate task-list panel is removed.
- `list_commitments` / `commitments.list_commitments` take optional
  `direction` and `person` filters.
- After a kept transcript is extracted, one notification: "N promises found
  in <title>" (then the other kinds), opening the transcript.
- Reminders to others: none in this phase (drafts come with the email side,
  Phase 3). Lead time: overdue only, as before.

## Triggers

- Keeping a recording's transcript (`keep_recording_transcript`) extracts it
  in the background (recordings first, the plan's recommendation). A failure
  never fails the keep.
- On request: `extract.run` (write; `Lease::Outside`), `extract.status` (read).
- Not yet: a nightly pass over the day's chats (#2's question; recordings
  first).

Unattended Main agent turns can't call `extract.run` (not in the M4 safe
list).
