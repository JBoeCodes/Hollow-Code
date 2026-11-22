# Commitments ledger (overnight 2026-10-10/11, A5)

The Commitments screen was cut overnight 11-08 S3. Tasks is a list or a board. The ledger stays for agents. The rest of this file is how the screen worked.

What I owe and what I'm owed, in one view with keyboard triage.

## What it looks like

Tasks → **Commitments** since consolidation lane B (2026-10-13): a view of the Tasks page, chosen from the visible
**Tasks · Commitments** switch at its top, with the rail and top bar around it (it was a full view over the app).
`g k`, the palette's Open Commitments and Today's link open the same view.
The duplicate panel above the task list was removed in overnight 10 U19.
It is a page region, not a dialog: no focus trap, and leaving is the switch (or any page), not esc.


- **Header:** counts for I owe, owed to me, overdue (in the risk colour when there are any), snoozed; the Tasks page keeps the sole title.
- **Toolbar:** whose (I owe / Owed to me / All, keys `1` `2` `3`), status (Open / Snoozed / Closed), a filter field (`/`)
  matching every word against the title, the other party and their people note, and **New** (`n`).
- **List:** pending commitment proposals first, under "Found, not yours yet". Then open rows grouped Overdue, This
  week (the next 7 days), Later and No date. Snoozed rows are sorted by wake day, closed rows newest first. Each row
  shows a direction arrow, the title, "to Pat" or "from Raj", and the due day (or "until …" when snoozed).
- **Detail** (sticky beside the list; below it under 860 px): the state (Open, Overdue, Snoozed until …, Done,
  Dropped), the title, "You owe Pat Smith" or "Raj owes you", and the due line. Then the citation: the quoted words it
  rests on, and the source note's name with its anchor (a recording timestamp or a conversation turn) as a button
  (`o`). Below that: Done / Snooze / Drop (or Reopen), the snooze choices, and Open person (`p`) / Open note (↵).
- **Keys:** Settings → Keyboard shortcuts → Commitments.

Motion: the header, toolbar and body rise in on a short stagger; none with Reduce motion. Tokens only, so light,
dark, glass and custom themes carry it.

## Promises and tasks

A commitment is a promise between you and a person; a task is something to do. They stay separate (consolidation G,
2026-10-13). On an open commitment you owe, **Make a task** adds it to Tasks once (idempotency key
`commitment:<fingerprint>`, origin `commitment:<path>`); the promise stays open here until you mark it done. Once the
task exists the promise says "Also a task: …" with a link to it, and Make a task goes away. The task's detail and editor
say "From: A promise you made to Jordan, in Commitments" (the counterparty once Commitments has loaded), that finishing
the task doesn't close the promise, and **Open the promise**, which opens this view with that exact row selected in
its Status view (`openCommitmentLedger(query, focusPath)`; plan completion item 4). Tasks made from a workspace plan, a
recording or Incoming say where they came from the same way.

The view keeps its place while you are elsewhere (plan completion items 1 and 4): Whose, Status, the filter, the
selected row and a half-written **New** commitment are module state in `CommitmentLedger.tsx`, so opening a source or
person note and coming back by the view switch finds them; an explicit open (`g k`, a person link, Open the promise)
applies its own filter. The New form's draft survives esc, **Close** and leaving Tasks (the button reads **New (draft
kept)**); **Add** or **Discard** clears it.

## Keys

The keys are bare letters, read only while the view is shown, no dialog, palette or `g` leader holds the keyboard, and nothing is being typed in a field (the New form
keeps every key but esc). A chord with ⌘, ⌃ or ⌥ passes through untouched, so no macOS or app shortcut is
shadowed. Enter and Space on a focused button press that button. The letters and delete are off with Settings → Keyboard
shortcuts off (esc, ↵ and the arrows still work), and a held key repeats only movement, never an action. A shortcuts
file that had already given `k` to another target loads with the ledger's key empty (`shortcuts::yield_new_defaults`),
so an older file stays valid instead of being dropped.

| Key | Does |
|---|---|
| `j` / `k` (↓ / ↑) | Move |
| `d` | Done (on a closed one: reopen) |
| `delete` (⌫) | Drop (cancelled; nothing on a closed one). Was `x` until P1-15 (2026-10-25): `x` selects a row on the Tasks tab, so it isn't destructive here |
| `s` then `1` `2` `3` | Snooze until tomorrow, next Monday, a week from today |
| `u` | Undo the last done, drop, snooze, wake or reopen (status and snooze put back; not accept, reject or new). Also the **Undo** button beside the change's note (since 2026-10-13) |
| `o` | Open the source note |
| `p` | Open the person's people note; with none, filter the ledger by them |
| ↵ | Open the commitment's note |
| `a` / `r` | Accept / reject a found one (`1` I owe, `2` they owe me when the source didn't say) |
| `n` | New commitment (what, which way, who, due) |
| `1` `2` `3` | I owe / Owed to me / All |
| `/` | Filter |
| esc | Close an open snooze or who-owes-whom choice; leave a field (the view itself stays) |

## Data

There are no new commands. Changes go through `update_commitment`, which gains `snoozedUntil` (`YYYY-MM-DD`, `""`
wakes it) and, for undo, `fulfilledAt` (overnight 10-11/12 A9: puts a reopened or dropped commitment's original done
date back; webview only, not an agent action argument). Two new frontmatter keys go on `commitments/*.md`:

- `snoozed_until`: kept only while the commitment is open; closing it clears the key. A malformed hand-written value is
  dropped on read, so the note stays visible instead of disappearing.
- `evidence`: an accepted proposal's `evidenceExcerpt`, collapsed to one line (at most 1,000 characters). It is not part
  of the fingerprint.

A snoozed commitment (`is_snoozed`: open with `snoozed_until` after today) is left out of the overdue nudge
(`nudges.rs`), Today's "Commitments due", the Tasks panel and the ledger's open list until that day. People pages'
generated block still lists it, because it is still open.

Accepting and rejecting a found commitment goes through `resolve_commitment_proposal`, as in Waiting on you. Only the
person's own keypress does it.

## Empty and error states

- Nothing open: "You don't owe anything open." / "Nobody owes you anything open." / "No open commitments."
- Nothing snoozed or closed yet, and no match for the filter, each say so.
- No row selected: "Select a commitment to see where it came from."
- A source-less commitment: "Added by hand." (or the channel it was made in). For `o`: "No source note for this one."
- A failed write shows inline above the list; the row stays as it was.

## Not in this slice

Deriving commitments from tasks (the extraction pipeline proposes them from recordings and chats). Editing fields in
place (open the note). A snooze picked by date. A Main agent standing order for overdue commitments: the overdue nudge
already covers it.
