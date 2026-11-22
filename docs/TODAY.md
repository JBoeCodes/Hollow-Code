# Today

The Today screen was cut overnight 11-08 S3. Work is Home, Map and Running. The rest of this file is how that screen worked. Code that remains: `src/store/todayBrief.ts`, `src/todayBrief.ts`, `src-tauri/src/today_brief.rs`.

## What the person sees

Past timed Calendar rows offer **Recap**: the meeting recap uses the shared prep dialog, offers
that day's recordings when none is linked, then saves a timecoded page and sends proposals to Review.
It runs only on click. See [Meeting prep packs and recaps](PREP_PACKS.md).

- **Opening it:** the **Today** tab of **Work** (since consolidation D, 2026-10-13: a view of that page, no
  longer an overlay), `g d`, or the palette's **Open Today**; all open the same view, on **Day**.
- **Day / Week:** a switch at the top of the tab (S3, 2026-11-03; group "Today range"). Day is this brief; Week is
  the [weekly review](WEEKLY_REVIEW.md), which was a tab of its own until then. The Today tab opens on Day and keeps
  the range while it stays chosen; `g e` and the palette's **Weekly review** open it on Week. The tab carries the
  weekly review's Friday-to-Monday count. A button that leaves for another page (Draft for the COO, Open as
  a note) resets Work to Home, with Today on Day.
- **The hero:** the date in small caps, "Good morning, Alex" (the first name from `me/Identity.md`; a placeholder
  like "(your name)" is ignored), and the day's paragraph. While it is being written, three shimmering lines; on
  error, the reason and **Try again**.
- **Recommended:** up to three cards (numbered, title, why, and the request itself in quotes). **Draft for the Main
  agent** opens today's COO conversation on its page with that request in its message box for the person to
  read, edit and send, and Today closes (`store/todayBrief.ts` `askMainAgent`; the weekly review's button goes the
  same way). Agent-written text is never sent on its own; the brief is written from calendar titles anyone can set.
- **Sections** (cards in a responsive grid):
  - **Calendar:** today's events, all-day first. A past meeting is muted, one within the hour is marked "soon", and
    an upcoming one has **Prepare**, which opens its prep pack (A4, [PREP_PACKS.md](PREP_PACKS.md)). With no
    calendar connected it says so.
  - **Due:** overdue tasks (oldest first) and tasks due today, excluding local snoozes.
    A row opens the task without closing Today. Done and Snooze offer Undo;
    A status the task's tracker issue holds opens the issue instead (finish it there).
    A task whose issue is missing offers Open only. Fourteen rows are shown, with a counted
    Tasks link for the rest. Loading, failed and partial reads stay explicit with Retry.
  - **Commitments due:** open ones due today or earlier; a row opens the commitment note. Past six, the button says
    "N more in Commitments" (P1-12).
  - **Waiting on you:** the first five titles, each opening its item the way its Review row does (`openWaitingItem`,
    P1-12), and **Open all N**.
  - **Last night:** pull requests, commits and failures from `overnight_summary`, shown after a night with runs, or
    when it couldn't be read (with the reason and Retry).
- **Footer:** when it was written, **Write it again**, **Open as a note**. When the facts' fingerprint
  (`briefFingerprint`, kept in localStorage `hollow.todayBrief.facts`) no longer matches the one the brief was written
  from, it says "Facts changed since h:mm" instead (P1-12). The automatic write is held while the window is away
  (Hollow not the app in front, `autoWriteStep`); when it is back, every source is read again (`refreshAll`) and then
  the brief is written, so a Today left open overnight covers the night instead of the 00:01 read.
- **Cards:** each recommendation is `components/Recommendation.tsx`, the same card as the Weekly review's priorities.
- **Open or finished:** one rule, `isOpenTask` in `taskStatus.ts`, for Today, the Agenda, Tasks (scopes, the tick, bulk
  edits), the palette's tasks and Home's Due card; snoozes stay a display filter. Cancelled, Won't do and Closed are
  not open without counting as finished (`isCancelledStatus`).
- **Motion:** the hero, cards and sections rise in staggered (70 ms apart). With Reduce motion there is no motion;
  the shimmer pauses with the ambient switch.
- **Keys:** it is a page view, not a modal: not in `MODAL_SEL`, global shortcuts work, and another Work tab or
  the rail leaves it (until consolidation D it was a modal-grade overlay closed by Esc).
- **Empty states:** "No meetings today", "Nothing due today", "No commitments due", "Nothing waits on you". On Home,
  a clear day reads "A clear day: nothing scheduled or due." An empty state shows only when its source was read.
- **Unknown is never zero (P0-04):** `TodayFacts.coverage` holds each source's read (`ok`, `partial`, `unavailable`,
  `not-connected`, or `loading` before the first answer, with a reason) for the calendar, commitments, last night
  and the waiting list. While the calendar is still loading the card says "Reading the calendar…", never a failure.
  Home's counts line says "calendar unknown" (and never "A clear day") for a source it couldn't read. A brief written
  while a source was unknown stays cached for the day; **Write it again** rewrites it after a successful Retry.
  Helpers: `todayCoverage`, `calendarCoverageOf`, `coverageHeader`, `coverageNote`. A card whose source failed says "Couldn't read <source>: <reason>." with
  **Retry** (the calendar adds **Open Connectors** for an expired Microsoft 365 sign-in); commitments keep the last
  good list (`commitmentsList` keeps it, `commitmentsError` names the failure). The digest prints
  "<Section>: unknown (<reason>)" for such a source, and "(at least N; partial read: …)" for a partial one, never a
  count. While facts aren't ready, the hero names what it waits for ("Waiting for the calendar, tasks.") with
  **Retry**, and **Try again** and **Write it again** are disabled.

## How it works

- **Facts** (`todayFacts()`, one shared store for Today and Home, owned by its own root): `agenda_events` for the
  local day, `tasksData` (enabled here), `list_commitments`, `overnight_summary` and the waiting list. All are
  deterministic, with no model. The resources are keyed by the day string, so they load once a day and again each
  time Today opens, never on Home's minute tick. A timed meeting counts when it overlaps today, midnight to the next
  local midnight (`dayBounds`: a calendar day, 25 hours on the autumn DST change, never "midnight plus 24 hours",
  P0-03); an all-day one by its dates (`allDayCovers`: Graph sends UTC midnights, and a multi-day one shows on each
  of its days). The header's date comes from the facts' day (`dayLongDate(todayFactsDay())`), which the open view's
  minute tick moves at midnight. The Agenda's "today" and Overdue follow a minute day clock (`createDayClock`,
  `src/dayClock.ts`, removed 11-10); a person viewing today follows it to the new day (`followToday`), except while a focus-block
  form or an event's details are open. Its Week view titles the range in the locale's range form ("Oct 22 – 28",
  `weekRangeLabel`). The paragraph
  waits until all of them, the tasks and the waiting list included, have arrived (`ready`).
  Task reads are non-suspending, refresh on opening, and failed/partial tasks
  prevent automatic generation. The factual digest retains totals before snoozes;
  only the action list and Home count subtract snoozes. Dated Due buckets
  reclassify when the local day changes.
- **The brief** (`today_brief.rs`): `today_brief_cached(date)` reads `.today/<date>.json` in the COO's folder
  (a dot-folder, so it isn't listed as a Result), creating nothing. `today_brief(date, facts, regenerate)` returns the cached one unless asked to
  regenerate; otherwise the default agent (any provider, saved model, tool-free, 120 s, metered `today_brief`) writes
  `{paragraph, recommendations}` from the facts digest, which is fenced as data (a `>>>` in the facts can't close the
  fence). The prompt forbids inventing facts and suggesting sends. The reply is parsed leniently and cut to size. The
  readable copy `outputs/today/<date>.md` (a Result) is written first, then the cache; the COO workspace is
  created only then. One generation runs at a time.
- **When it is written:** when Today opens and today's brief isn't cached yet. A failure shows with **Try again**;
  reopening Today (or a new day) clears it. Home never generates; it shows the
  cached paragraph or the day in counts.

## Limits

- The paragraph reflects the facts when it was written; **Write it again** refreshes it.
- Calendar facts are only as complete as `agenda_events` (Microsoft 365 connector).
