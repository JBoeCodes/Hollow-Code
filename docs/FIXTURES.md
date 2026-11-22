# Browser fixtures and the state matrix

The browser mock (`?mock=1`, `src/mockApi.ts`) runs the real UI against synthetic data, isolated from any vault.
This page is the fixture contract the 2026-10-13 review asked for (roadmap B, verification V07): which whole-app
states exist, how to open each, where it shows, and which test proves it. Plan completion L5, 2026-10-14.

**Chromium results are never native evidence.** These fixtures prove the UI handles a state; WKWebView, real
window scrolling, VoiceOver, glass and provider behaviour need the native plan in
[verification-2026-10-14/NATIVE_PLAN.md](verification-2026-10-14/NATIVE_PLAN.md).

## How to open one

Fixtures stack in the URL, and a reload keeps them because the URL does:

| Part | What it does | Module |
|---|---|---|
| `?mock=1` | Empty library: the browser mock with nothing seeded | `src/mockApi.ts` |
| `&demo=1` | Populated: seven believable workspaces with briefs, plans, Results, sources, chats, 8 tasks, 40 notes | `src/mockDemo.ts` |
| `&demo=large` | Large: the populated set plus 190 more workspaces, a 1,000-turn thread, a 60-turn canvas thread, 5,000 tasks, 10,000 notes | `src/mockDemo.ts` |
| `&main=1` / `&missions=1` | Adds the Main agent's day / Mission Control's running and queued turns (with `demo`) | `src/mockDemo.ts` |
| `&scenario=<names>` | One or more named whole-app states, comma-separated, applied after the demo seed | `src/mockScenarios.ts` |
| `&setup=1` | The first-run folder setup instead of the app | `src/main.tsx` |

`npm run dev`, then open e.g. `http://127.0.0.1:1420/?mock=1&demo=1&scenario=interrupted`. An unknown scenario
name is reported in the console (`mock scenario "x" doesn't exist`) and ignored; `window.__mockScenario.names`
lists what was applied.

## The state matrix

| State | Open with | Where it shows | Proven by |
|---|---|---|---|
| Empty | `?mock=1` | Home's first-run, empty Library, empty Review | `fixture-parity` (Settings + Main agent), `empty-art`, `app-smoke` |
| Populated | `?mock=1&demo=1` | Everywhere | `fixture-parity`, J1–J10 journeys |
| Large | `?mock=1&demo=large` | Long thread, 5,000 tasks, 10,000 notes, ~200 workspaces | `fixture-parity`, `next-level-scale`, `long-chat`, `npm run perf` |
| Duplicate names | `?mock=1&demo=large&scenario=library-scale` (also spec-seeded in `shell-scale`) | Work's Find, Search, sidebar, Every workspace tell them apart by origin | `fixture-states` "library-scale", `shell-scale` (L4), J1 at the small sizes |
| Real library scale | `?mock=1&demo=large&missions=1&scenario=library-scale`: exactly 213 workspaces (the native library's count in the review), long titles, ~160 conversations, 30 open questions, running and queued work | Work "213 total", Review | `fixture-states` "library-scale" |
| Disconnected | `&scenario=disconnected` | Review names the source it couldn't read; Settings → Devices → Companion devices shows the unreachable backend in its own words; Connectors offers the Microsoft sign-in; the Agenda plans without the calendar | `fixture-states` "disconnected" |
| Interrupted | `&scenario=interrupted` | Review's "Resume 1 interrupted chat" (opens a confirm, sends nothing on its own); the chat's unsent follow-up back in the composer, with its notice; Continue on the interrupted turn; an interrupted automation in the run queue | `fixture-states` "interrupted" |
| Permission required | `&scenario=permission-required` | Review lists the tool approval; Automations says the held automation is "paused until its settings change is approved" | `fixture-states` "permission-required" |
| Loading | `&scenario=loading` (workspaces, Home cards, Review, tasks and Results answer after 4 s) | `aria-busy` placeholders, never an empty-state claim, then the content | `fixture-states` "loading" |
| Stale | `&scenario=stale` | The workspace's "The brief may be stale…" note; Review's question "can no longer be answered"; Home's cards kept from the last good read (spec-driven: `home-digest-recovery`) | `fixture-states` "stale", `home-digest-recovery` |
| Partial success | `&scenario=partial-success` | Review "Couldn't read questions and Linear sync" while what was read still shows; the day ring's calendar coverage says partial | `fixture-states` "partial-success", `day-ring` |
| Conflict | `&scenario=conflict` | Review lists a task changed in Linear and here, and the vault's conflict copies | `fixture-states` "conflict", `conflict-copies` |
| Stale after a failed refresh | `?mock=1&demo=1`, then `window.__mockFailCommandOnce = 'list_workflows'` (or `'list_skills'`) before an action that re-reads the list (Run now; New skill → Create skill) | “Couldn’t refresh: showing what was here before.” with Retry above the Automations list and Library → Skills; Retry clears it (X21) | `stale-refresh` |
| Subagents | `&scenario=subagents` | The report chat's four finished subagents: the side panel card (top bar Side panel, or ⌘\) says "4 done" with their glyphs; its Subagents tab lists Active · 0 and Done · 4; "Hollow context" opens its transcript with the model at the header's right (overnight 11-12 C4, C5) | `fixture-parity`; captures |
| Unavailable | `&scenario=unavailable` | Tasks says it couldn't read the tasks folder, with Retry; Library → Results shows where the damaged review state was moved | `fixture-states` "unavailable", `confirm-load-recovery` |

Every scenario also mounts every Settings section and opens the Main agent without a page error
(`fixture-parity`), and the shared `fixtures/test.ts` fails any test on an uncaught page error.

## What each scenario sets

Scenarios only set the mock's existing knobs, so a spec can still set one knob by hand. Keys (all localStorage
unless noted):

- **disconnected:** `mockCalendarOff`, `mockBuiltins` (built-in connectors listed), no `mockM365`, a claude.ai
  `mockConnectors` row, `mockCompanionBackend` (status override), `mockNeedsYouUnavailable` (`Linear sync`),
  `mockIngestFail`.
- **interrupted:** `mockInterruptedChats` (what `list_interrupted_chats` returns; `resume_interrupted_chats` drops
  the keys it queues from it and appends them to `mockResumedChats`, and refuses an `agentStarted` one; P1-40's
  `restart-resume.spec.ts` replaces it with three chats and an agent-started one), an `interrupted` row in
  `mockNeedsYouExtra`, two `interrupted` rows in `mockRunQueue` (an automation, and a chat with an unsent
  follow-up), and the demo chat's last turn rewritten to `status: 'interrupted'`, with the
  conversation's `permissionMode` recorded so it resumes its native session and offers Continue.
- **permission-required:** `mockRefusedWorkflows` (`morning-brief`), an `approval` row in `mockNeedsYouExtra`,
  a claude.ai `mockConnectors` row.
- **stale:** `briefStale` on the report workspace's `mockWorkspaceReentry`, a `stale` question in
  `mockNeedsYouExtra`.
- **partial-success:** `mockCalendarPartial`, `mockNeedsYouUnavailable` (`questions`, `Linear sync`).
- **conflict:** a task `conflict` and a `copies` row in `mockNeedsYouExtra`.
- **loading:** `window.__mockScenario.delayMs` for `list_workspaces`, `workspace_digests`, `list_needs_you`,
  `tasks`, `list_all_results`.
- **library-scale:** `mockWorkspaces` padded to exactly 213 with the same-named and long-titled set, a
  `mockWorkspaceSessions` row for three in four workspaces, 30 questions in `mockNeedsYouExtra`.
- **subagents:** `mockSubagents` (four done rows) for the demo's two Q3 chats, and `mockAgentConversations` for "Hollow
  context" (one exchange, its receipt's `cfo` naming the model).
- **unavailable:** `mockResultReviewsDamaged`, `mockArtifactSessionsError`, and
  `window.__mockScenario.unavailable` rejecting `tasks`.

Other one-off knobs live where they are used (`mockDigestFail`, `mockChatSendError`, `mockCancelledWorkflowRun`,
`mockOutsideEffects`, `window.__mockFailCommandOnce`, `window.__mockHoldOnce` (hold one command's answer until the spec releases it, for a late response without a fixed delay), …); grep `localStorage.getItem('mock` in `src/mock*.ts`.

## Adding a state

1. Add a scenario to `SCENARIOS` in `src/mockScenarios.ts`: a one-sentence `summary`, its `states`, and an
   `apply` that sets knobs (add a knob to `mockApi.ts` only when none exists).
2. Add a `fixture-states.spec.ts` test that reaches the state by clicks and asserts what the person sees, and add
   the name to `fixture-parity.spec.ts`.
3. Add its row above.

## Found by these fixtures (2026-10-14)

- **Tasks never rendered when the task list couldn't be read.** An errored Solid resource throws on read; with no
  ErrorBoundary the page stayed on "Loading page" with an uncaught error. `preserveDuringRefetch`
  (`src/stableResource.ts`) now never throws: it returns the last good value (or nothing) and the page shows the
  error with Retry. Applies to every resource wrapped by it. The kept value looked current after a failed refresh until X21 added `refreshFailed`
  and `components/RefreshNotice.tsx` (where each list shows it: docs/FRONTEND_MODULES.md → `stableResource.ts`).
- **Continue vanished after a restart restored a queued follow-up.** The "a message you queued … is back in the
  composer" notice was the last turn, so `canContinueChat` stopped at it; it is now marked as a notice about an
  earlier turn (`src/store/chatRuntime.ts`).
