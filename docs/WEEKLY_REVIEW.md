# Weekly review (overnight 2026-10-10/11, A7)

The Weekly review screen was cut overnight 11-08 S3. There is no Week range. The rest of this file is how that screen worked. Code that remains: `src-tauri/src/weekly_review.rs`, `src/weeklyReview.ts`, `src/store/weeklyReview.ts`, `src/store/weeklyReviewRead.ts`. The Friday hook is in `digest.rs`.

## What the person sees

- **Ways in:**
  - **Work → Today → Week**: the Week side of the Today tab's **Day / Week** switch (group "Today range").
    It was a **Weekly review** tab of its own from consolidation D (2026-10-13) until S3 (2026-11-03). The Today tab
    opens on Day; the ways below open it on Week. The current tab and range are memory only.
  - `g e`, the palette's **Weekly review**, or an agent's `navigate` to `weekly-review` (or `today` + `weekly`). Each
    opens this week (`openWeeklyReview()`; `today` + `weekly` through `openWorkTarget`), never the week browsed to last.
    The week is set only once the jump is accepted, in the same batch as the view: a leave guard that refuses it keeps
    the week on screen (2026-11-03).
  - The **Today** tab carries a one-review prompt from Friday 15:00 to Monday noon. Choosing Week then opens that week:
    on Monday morning, last week. Opening acknowledges that week; quiet weeks have no badge, while loading/failed reads
    show an ellipsis/dash, and the tab's description says to choose Week.
  - The Friday 15:00 notification "Your week in review is ready", which opens the snapshot.
- **The view** (Today's Week range; a view of Work since consolidation D, it was a modal-grade overlay, `.wr-backdrop`):
  - **Hero:**
    - "This week" or "Week in review", with ‹ › (and ← →) to step weeks, never past this one.
    - The week label, e.g. "Week 41 · 5–11 Oct".
    - The take: two sentences by the person's default agent.
    - Stat tiles: Results accepted, tasks done, PRs merged, decisions, spend.
  - **Next week, suggested:** up to 3 priorities, each with why, the ask itself in quotes, and **Draft for the Main
    agent**, which fills the Main agent's message box (the person sends it). Each is `components/Recommendation.tsx`,
    Today's card (P1-12).
  - **Cards** (a responsive grid, staggered rise; none with Reduce motion):
    - **Shipped:** accepted Results, installs, tasks done, merged PRs. A row opens its note or conversation.
    - **Slipped:** tasks due this week and past, still not done, or done with the task file last changed after the
      due day ("due Tue 6 Oct, done; last changed Thu 8 Oct": the only date a task has, and a later comment moves it
      too, so it says "last changed"; a recurring task rolls forward when done, so it leaves the list); then commitments
      due by the week's end and still open now ("overdue now" on this week), snoozed ones excepted.
    - **Decisions** recorded this week, not superseded.
    - **Time & spend:** a bar per day from the usage ledger, the top 6 workspaces, runs without a known cost, and the
      Friday update's other numbers (conversations and runs, hours saved).
    - **Commitments:** kept this week and made this week.
    - **Next week:** the calendar by day (`agenda_events`, grouped by `nextWeekByDay`: a multi-day all-day event
      under each day it covers, a timed one under each day it overlaps: its start time on its first day, then
      "continues", and "until <end>" on the day it ends) and tasks due.
    - **In progress**, when there is any.
  - **Footer:** **Save to Results**, **Write the take again**, and the saved note's link.
- **States:**
  - "Gathering the week…" while loading.
  - "A quiet week: nothing recorded." (no take is written).
  - "Spend not available" when the ledger can't be read.
  - "No calendar connected".
  - "Couldn't read next week's calendar: <reason>." with **Retry** (and **Open Connectors** for an expired
    Microsoft 365 sign-in), never "Nothing on the calendar yet"; the take's facts then say
    "Next week on the calendar: unknown (<reason>)" (P0-04).
  - The take's error with **Try again**.

## How it works

- **Facts** (`assemble`, pure): `digest::gather` for the week (the Friday update's own gather), then:
  - PRs agents merged (`overnight_summary::pull_requests_between`, the same tool-call reading as Last night).
  - Decisions with `decided_at` in the week.
  - Commitments: kept (`fulfilled_at` in the week), made (`created` in the week) and overdue (open, not snoozed, due
    before today).
  - Tasks due in the week before today and not done.
  - Ledger rows summed per local day and per workspace.
  - No model call is involved. Sections are capped at 40 lines.
- **The take:**
  - One tool-free one-shot by the default agent (any provider, metered `weekly_review`). The facts digest comes from
    the webview (`factsDigest`, including next week's calendar), fenced as data.
  - The reply has Today's shape (`{paragraph, recommendations[]}`, parsed by `today_brief::parse`). It is cached in
    the Main agent's `.review/<week>.json`, outside Results.
  - It is written on its own only for the week the view opened on, when that week isn't quiet. Other weeks show a
    button. A take that comes back after the person stepped to another week isn't shown there.
- **Snapshot:** Markdown in the Main agent's `outputs/review/<Friday>.md`: the take, the numbers, every section with
  links to notes, and the priorities. **Save to Results** writes it, replacing an earlier one. The Friday job writes it
  once per week, stamped `review:<week>`: after the weekly update's draft, under the same scheduler lease, replacing a
  mid-week Save. It then notifies. A failure is logged and never blocks the prose turn.
- **Reads:** the Today tab's weekly prompt makes one read and works out Monday's
  previous week on the client. The full view consumes that read once if it is
  still under a minute old; otherwise opening or retrying reads afresh. A view
  error shows beside the last good week; the tab shows unavailable instead of zero.
- **Commands:** `weekly_review`, `weekly_review_take` and `weekly_review_save`, all user-only. Agents read the week
  through the Friday update and the records.

## Limits

- PRs come from agents' `gh` tool calls; one merged by hand on github.com isn't counted (as on Last night's card).
- "Tasks done" comes from a done task's file modified time, as the Friday update does (labelled not verified there).
- The snapshot is Hollow's own note: edit a copy if you want to keep changes past the next save.
