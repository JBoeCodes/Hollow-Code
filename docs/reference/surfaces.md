# Feature surfaces

## Tasks and Automations page density (P4)

Tasks keeps New task as its primary header action, with Sync now secondary.
The suggestions row stays
collapsed while loading and after a successful empty read; unavailable counts keep their visible
Waiting route. Empty tasks distinguish filters, completed scope, another
scope, a verified empty collection and incomplete reads. View all tasks clears
filters and opens All tasks. Layouts, counts, sync and bulk actions keep their
existing owners. Background refreshes keep the last complete empty heading.

Automations keeps New automation in the header. Background work windows and
How background work runs share a wrapping support row; expanding either gives
it the row's width and preserves its bounded scrolling. The existing Main
agent reveal, loading/error/retry and window routes stay with their components.
An empty collection has one explanation in the list, which spans the page
until creation starts or an automation is active. An outside deletion or a
failed post-create list read never hides the active editor or output.
It has no inherited compact-list height cap, stays
visible on narrow windows and returns after Cancel.
Search misses retain Clear search; initial failures and retained-list refresh
errors retain their recovery controls.

Part of the agent manual. [AGENTS.md](../../AGENTS.md) is the entry point and
links every reference doc; this one holds the per-feature subsections that had
collected under CLAUDE.md's "Styling" and "Self-install CI compatibility", and
"Day ring", moved there word for word on 2026-10-13 (only relative links were
re-based). Read it before changing companion apps, Waiting on you, key hints,
the welcome tour, Today, push-to-talk, triage, the
weekly review, Ask (One Ask), commitments, the canvas gallery (Library → Results → Canvases),
Conversation history (Library → Conversations), Mission
Control, canvas instruments or the day ring. Keep it current with the code in
the same change, as AGENTS.md asks.

The subsections from here to "Mission Control" sat under CLAUDE.md's
`## Styling` (now [Styling](styling.md)), between "The app shell" and
"Product-quality layer".

### Shell controls (overnight 11-04 U1)

TopBar holds sidebar visibility, Back/Forward, the current title and workspace
actions, Notifications and the Main agent orb. Workspace display controls stay
in their existing far-right slot. Search chats is the icon beside the sidebar
toggle (L1, 11-12; ⌘⇧K); New chat is the COO's page. Recent + creates a project.
The selected sidebar project/chat is the return route from another page.
Tasks owns New task; Library owns New note and Library actions → Add a task,
note, link, file or recording. That Add sheet retains automatic classification
and file/recording inputs; Import a source retains its explicit preset.
Work → Running owns queue actions and the Background tasks operation log.
Existing shortcuts, page ids, navigation guards and data remain unchanged.

### Settings scope (overnight 10 U20)

Each section's header shows one always-visible line from `saveNote` in
`settingsSections.ts` (overnight 11-08 S5): "Saves at once.", "Needs Save.",
"Saves at once. Forms need Save.", or "Needs a restart." The closed **About
these settings** disclosure is gone. The named `What X changes` group remains
in `settings/settingsLayout.tsx`. Each part repeats the same line. Save
handlers and scope semantics are unchanged: saved forms must never imply
instant effect. `settings-scope.spec.ts` checks all sections; its journey
companion covers desktop, minimum width and Largest text.

### Settings sections (P1-22, D4, S2)

The ids contract: `src/settingsLegacy.json`
snapshots the 7 sections, 9 parts, 30 retired ids, 15 alias controls and every
registered control with its section from before the twelve-section list, and stays frozen. `resolveSettingsTarget(id, label?)` in
`src/settingsResolve.ts` returns `{kind: 'section', section, part?, label?}` or
`{kind: 'page', page, tab?, label?}`; a target is a section, or a page for what moved to Library or Automations. `requestOpenSettingsSection` (so the palette, `appBridge`, the action
runner and `settings:` links), the capability doctor's `fixSection` values and the page's
own `jumpTo` all resolve through it; it maps the old ids to the current sections (below) and to page targets. `settingsResolve.test.ts` checks every id, part, alias, control pair, call-site
literal in `src/`, doctor value and Rust `settings:` link, and that the JSON matches the
TS and Rust (`SECTION_ALIASES`) tables cover it; a Rust test in `settings_keys.rs` checks every
alias and emitted link reaches a section with keys, or Sources with its `NONE` line.

Overnight 11-04 P2 bounds the content column at 52rem. Interface's **App and
Main agent icons** disclosure keeps its rows mounted, so the existing search
jump opens it before focusing a row. Upload an icon, Generate a logo, Reset and presets
remain there (the icon maker went 2026-11-12, overhaul H1 and H2); Main's Change icon opens the same dialog. Theme syntax guidance
lives beside the custom-theme editor. No stored keys or controller ownership changed.

Overnight 11-07 D3 quieted the seven sections of the time without moving a control. A
section or sub-header intro over 150 characters shows its first sentence and a
visible **More** (`LeadText` in `settings/settingsLayout.tsx`, `splitLead` in
`settings/leadText.ts`, which skips a stop after an abbreviation or a one or two
letter word); the runner rows' help, Remote runner's off text and Runner GitHub
access fold the same way by hand (`MoreFold`), and other rows' help (Agents'
"When an agent runs out") still shows whole. The rest stays in the page, so a
spec that reads the text still finds it. Interface's theme gallery is a
three-across tile grid (same cards, same Use, Edit and Delete). In Agents the
five runner-routing rows (Runs on, Builds and tests, App builds, Overnight and
worker chats, Runner GitHub access) render only once a runner is set up
(`<Show when={runner()}>`); until then their registry entries carry
`near: 'Remote runner'`, so Settings search lands on Remote runner, whose Set
up creates them. Since S5b (below) four of them are in Devices and App builds
is in Advanced, shown with or without a runner.

Thirteen sections in one list, no group caps (the settings redesign plan 3.2; `settingsSections.ts`), in this order:
**General** (`general`: Appearance with the icons fold, Notifications, and
Startup's Run at login and Keep running from Automation), **Chat** (`chat`:
Canvases, Enter sends, Retitle new chats, the new-chat permission mode and
Chats at once from Automation), **Models** (`models`: Default agent, When an
agent runs out, each installed agent's model with the rest behind a closed
"Not installed" fold, Save agent defaults, then Effort (Automatic or Fixed) and
Models the CFO may use, a row whose Choose opens the checkboxes in a sheet;
then the Self-hosted models part, `selfhosted`, the list and Add only),
**Accounts** (`accounts`: Claude accounts and
Spread usage, Other providers, Jev, then Agent updates from Agents),
**Personalization** (`personalization`, was Knowledge), **Connectors**
(`connectors`: the library, Built into Hollow,
Recordings, then a "Skills are in Library" link row: Skills are in Library, Skills),
**Sources** (`sources`: Tasks and projects with the standalone
`TaskTrackerPanel`, then the Hollow folder part, `vault`, then Context, `layers`, a closed group with one row per context layer, CX17), **Background work**
(`background`: Scheduled automations, one link row "Automations, windows and the run queue are on the Automations page", the heartbeat and the COO's rows, swarm model included), **Devices**
(`devices`: `CompanionDevicesPanel` whole, web backend and push key included,
then the runner: Remote runner, and once one is set up Runs on, Builds and
tests, Overnight and worker chats with Save runner settings, and Runner GitHub
access),
**Usage** (unchanged), **Keyboard shortcuts** (`shortcuts`), **Updates** (`updates`: running version and channel, status, last check, Channel, Install updates overnight, Check now, update-server connection and pending-token decisions, and the last install result with OK) and **Advanced** (`advanced`, was System). Skills, History & recovery, the
automation windows and the run queue live on other pages (below); fifteen rows that
were never settings are gone (One line each).

Updates uses the shared update store. Server notes render as text. The password input clears on submission and never enters a signal or a log. Failed settings reads show an error without throwing from checkbox accessors. A channel change in this session or an outstanding downgrade permission shows the one-time older-version notice. An armed wait shows its restart explanation and Cancel update wait. Result OK and wait cancellation remain available on unconfigured builds; other controls require configuration. Connection failures show only the first error line after excluding the submitted token and its trimmed value. The overnight choice turns on the overnight installer (`docs/UPDATES.md`, Overnight). The frozen legacy ids are unchanged.

The top-bar update pill keeps an armed wait visible even if the offer was dismissed.
While waiting it offers Cancel without Not now; store dismissal cancels first and
refuses dismissal if cancellation fails. Cancel, an ended wait and leaving ready
clear stale running-work decisions. The pill and open disclosure remain mounted
while installing, with restart status replacing the action buttons. Its heading
names the dialog and channel downgrades have an explicit explanation. Opening
clears old errors. Below 900px only Update is visible in the pill; its accessible
label still announces waiting. Store listeners are shared and reference counted.

**One screen per section.** Large tools open as
sheets (`settings/SettingsSheet.tsx`: the shared `rbk-backdrop` + `rbk-card`
dialog in a Portal, Tab held by `holdFocus`, Esc and Close, a `locked` flag
while a save runs). Closing returns focus to the row's own button
(`openerRef`: `mark` in the click handler, `get` as `returnTo`; a selector finds
the button again when saving replaced it, as Set up becomes Set up again, and
focus is set in a microtask so a second sheet opening in the same update keeps
it). Sheets: **Edit themes** (General; the custom themes' Use, Edit, Delete,
New theme, the editor, Open themes folder; closing with a draft asks first),
**Set up a remote runner** (Devices; the `RemoteRunnerRow` panel and `RunnerSetupWizard`),
**Models the CFO may use** (Models; each box saves at once),
**Add a connector** (the catalog with Refresh, search, the discovered tiles,
Superhuman Mail, GitHub and Custom connector) and the connector form
(**Add a custom connector**, **Edit a connector**, **Connect GitHub**), the
self-hosted model form (**Add** or **Edit a self-hosted model**), **Add a Claude
account** and **Teammate roles** (the Edit roles button in Hollow's tools; roles
still save as you leave each field). The budget form is inline in Usage, Budgets (`components/BudgetsPanel.tsx`). **Theme** is
one menu (`settings/themeMenu.ts`: Match macOS, Light, Dark, Glass light, Glass
dark, then your themes); the stock cards are gone. **Keyboard shortcuts** has two
tabs, Commands and Go to keys (`settings/shortcutTabs.ts`), with Reset all to
defaults and Show all keys in the toolbar (Keys inside a surface are not listed here;
the ? card lists them). Connector rows are one line (what Hollow tested and where it applies
are in the row's details). Merged rows (`settings/MergedRow.tsx`, `Mini`: a
visible label that also names its control for search through
`data-settings-control`): Beat spacing and daily limit (Minimum minutes between
beats, Max beats per day), Check-in hours (with Working days only) and Chats at
once (Chats running at once, Chats at once in one workspace). `mergedRows.ts`
parses their numbers: a value that isn't a whole number in range goes back.
`tests/smoke/settings-height.spec.ts` (1440 by 900, 8 connectors, 15 in the
catalog, 2 Claude accounts, a runner, 3 self-hosted models) holds every section to
1.3 screens with folds closed and walks each sheet's open, save and focus return;
it is run once, after the last Settings PR lands.

**Height pass (S5b-2, 2026-11-12), at 1440 by 900 with that fixture:** Connectors 1.28
screens (was 2.20), Keyboard shortcuts under 1.3 (1.72), Accounts 1.22 (1.69), Usage's
Limits 1.29 (1.46), Sources about 1.29 (1.39). Rows are one line with the help beside the
name (wrapping under on a narrow window): connector rows and Hollow's tools
(`connectors-library.css`), an account and each agent-update row (`claude-accounts.css`),
Commands' rows and group captions (`shortcuts-panel.css`). A section's own group heading
carries its hint on the same line (`settings-graph.css`). Connectors' Services block is
the GitHub row, the Agenda calendar row (the source menu, a one-line count and Choose
calendars…, which opens the include list as a sheet: `settings-sheet-calendars`) and
About account access in the block's head; the old Calendar card is gone (its Calendar
settings button only focused the row below it). The Task tracker's three long notes sit
behind their own More. Devices' backend fold (`companion-connection-editor`) is closed at
rest, even before a backend is set.

**Devices pass (S5b-3, 2026-11-12):** Devices was 1.71 screens with the runner rows in, now
at most 1.3 at 1440 by 900 (`OVER_BUDGET` is empty). The Menu bar app is one row (Pair
a device, with "No devices are paired" in its help). The iPhone app is one Serve row (Publish
on Tailscale, Check again, Pair an iPhone and the switch beside the help); its pairing code and
QR still open inline when pairing starts. A closed fold (`companion-phone-more`, "Status, push
notifications and lid closed") holds the status lines, why-notes, Push notifications and the
lid row. The runner's state line stays; a closed fold (`runner-status-more`) holds the
preflight, last gate and owner, and another (`route-decisions-fold`) the recent routing
decisions. The backend's heading note is inside its connection fold. Specs that read those
parts open the fold first. The words budget is 2,600 (measured 2,544 on 11-12: Connectors 406,
Usage 304, Devices 216, Models 201), toward the plan's 2,500.
**Agent settings writes** run on one module-level queue (`settings/agentSettingsQueue.ts`)
across all four form pieces, so two pieces saving at once cannot revert each other.

**One line each.** `SettingsSection` has no `hint`
prop: the section's name says it (its save-note line stays). A row's help is one
line, 90 characters at most (a subhead's `hint=`, the `settings-row-v` under a row,
a connector tool's `<small>`, a `LeadText` or link row's `text=`); longer help sits
in the row's own `MoreFold` and only on risky rows (Hollow app tools, the COO's Act
on its own, the Jev switches, App builds and the other runner rows).
`settings/hintBudget.test.ts` scans the Settings sources for it and for a section
hint. `tests/smoke/settings-height.spec.ts` also holds the twelve sections to a ceiling
of visible words with folds closed (3,400 now; the plan's 2,500 once Models shrinks). Plan 5.1's cuts (C1 to C15) are gone, each with
its new home (the label decides first: `CUT_LABELS` in `settingsResolve.ts` sends
an old deep link or search to the place the capability lives): Bake-off monthly cap
(already gone), Start locally (Hollow folder; the File menu and palette), Help's
All keys (Keyboard shortcuts' Show all keys; `All keys` resolves there from any
section, `Help` only from Keyboard shortcuts or the old System, since 11-13), Runtime capabilities (Advanced, Runtime), the Danger mode row and
Revert to Edits only (the composer's permission menu, `/permissions`; Chat keeps
Permission mode for new chats), Keys inside a surface (the ? card), Startup warm-up
(Diagnostics), System health (Capability doctor), Disk space's "Or ask an agent"
ids, `AccessKinds` (both places), How Hollow adapts to you (`PersonalizationMap`),
the background-work explainer in Settings (it stays on Automations), the Service
health table (`ServiceHealth`), the Debug logs on/off chip (the log file count
stays), the Local prioritizing row and the Artwork preview. The doctor's
agent-runtime fix lands on Advanced, Runtime. Removed components and their CSS are
deleted.

Advanced holds **closed groups**, one heading each (`SettingsGroup` in
`settings/settingsLayout.tsx`: a `details.settings-fold.settings-group` with
id `settings-group-<id>`, its title an `h3` in the summary): **This Mac**
(Capability doctor, Backup evidence and its toggle,
Runtime with the Codex helpers readout, Disk space, then a History & recovery
link row to Library, History), **Search & indexes** (the `indexes` part is
itself the group: `SettingsPart group` renders a `details` whose summary is
the part head), **Hollow’s own model calls** (Extraction model and Background jobs,
`SelfHostedModelsPanel part="jobs"`), **Run limits** (each agent's no-output
watchdog and run deadline, Save run limits), **App builds and web backend**
(App builds with Save app builds, and a row that opens Devices on Backend for
the web app, which still lives there), Prices (its own disclosure, as before),
**Settings file** and **Diagnostics** (Diagnostics bundle, Build identity,
Folders agents can run tools from, Launch calls, Debug logs,
Hollow's databases). The Developer readouts and Advanced folds are gone. A
jump or search hit to a row inside a group opens it (the jump opens every
`details` around the row; a group's chevron is drawn with borders, so it never
joins the heading's name). The swarm model and value baselines have no group
yet: their rows sit inside panels (the COO rows, `ValuePanel`).

**The agent defaults form in pieces (S5b, 2026-11-12).** `settings/AgentDefaults.tsx`
renders four pieces of the one form: `AgentDefaults` (Models), `RunnerSettings`
(Devices), `RunLimits` and `AppBuildsRow` (Advanced). `AGENT_FORM_PIECES` in
`agentSettingsPayload.ts` names each piece's fields; every field the form holds
is in exactly one (efforts, Canvases and the compaction window save elsewhere).
Each piece keeps its own draft and dirty check (`withPiece` compares only its
fields) and registers its own unsaved-changes guard (`agent-defaults`,
`agent-defaults-runner`, `-limits`, `-app-builds`). Each Save calls the same
`setAgentSettings` with the whole payload, `savePayload(withPiece(draftFromSettings(live), draft, piece), live)`:
its own fields from the draft, every other field as read just before the save,
so no save drops or reverts another section's fields. Models' old Refresh
installed agents is Advanced, This Mac, Runtime's Refresh agent detection, which
also reads the agent settings again. Old (section, label) pairs land through
the label-first rule; `Reasoning effort` and `Mac mini runner` are
`RENAMED_LABELS` for Effort and Remote runner.

A **part** is a former section kept whole (`parts` in the registry,
`SettingsPart`): a `div.settings-part` (never a `section`, which the jump
would take as one row) whose id stays the old one (`settings-<oldId>`, so
`#settings-vault` selector still matches), with an `h3`
heading (`.settings-part-head`, `tabindex="-1"`) and its own
`RecoverableView`. Parts now: Self-hosted models (Models, lazy), Hollow folder (Sources) and
Search & indexes (Advanced, a group). Skills and History & recovery are in Library
(below). A lazy body mounts the first time it comes within 800 px of the
visible Settings body or when a jump wants it. A jump to a control names its
part and every part above it (`wantSettingsParts`) before the section renders
and looks for the row inside its part only. WKWebView has no scroll anchoring
(and `.settings-body` sets `overflow-anchor: none` so Chromium matches), so
for 2 s after landing each size change of the section scrolls the target back
(`keepInView` in `settingsJump.ts`). The registry derives each section's flat
`controls`: its own, then per part a heading control and the part's
controls. Every control label is unique across the registry (a vitest guard),
so a label names one home.

**Old ids.** The seven sections before the twelve-section list and every id they had absorbed
are aliases (`SETTINGS_SECTION_ALIASES`; Rust `SECTION_ALIASES`, the same
table, held together by a vitest guard): `interface` → General, `defaults`
and `customization` → Models, `operator` and `learning` → Personalization,
`automation`, `routines` and `heartbeat` → Background work, `system`,
`doctor`, `debug`, `context`, `vaultsearch` and `indexes` →
Advanced, `companions` → Devices, `vault`, `paths` and `setup` → Sources,
the connector ids → Connectors, `selfhosted` → Models,
`feedback` → Usage. `shortcuts`, `chat` and `accounts` are sections again.
`SETTINGS_ALIAS_CONTROLS` names the control each lands on (`doctor` →
Capability doctor, `startup` → Run at login, `heartbeat` → Silent heartbeat,
`debug` → Debug logs, `vault` → Hollow folder, and the parts' headings).
**A label decides first** (plan section 6): `resolveSettingsTarget` resolves
the id, then `settingsSectionForLabel` sends the jump to the section that
shows the label, so `('defaults', 'Agent updates')` opens Accounts,
`('automation', 'Chats running at once')` opens Chat and `('connectors',
'Task tracker')` opens Sources; part headings that are gone (Keyboard
shortcuts, Chat, Accounts, Companion devices) open their section, and
Developer readouts opens Diagnostics. `settingsLegacy.json` stays frozen at
the ids before the twelve-section list; `settingsResolve.test.ts` lands every old id and every
old (section, label) pair on a registered control in its new section.

**Keys.** `settings_keys.rs` files each key (58 at Settings PR 3) under its new section
(plan section 6): interface keys → `general` (`shortcuts` → `shortcuts`),
`agents.defaultAgent`, `agents.model.*`, `agents.effort.*`, `agents.whenOut`
→ `models`, `agents.updates.*` and `jev.apiKey` → `accounts`,
`accounts.claude*` and `claudeQuotaOptIn` → `usage` (shown in Usage), `agents.runsOn`, `agents.buildsOn`, `agents.unattendedRunsOn` →
`devices` (shown in Models' form; `agents.cooDispatcher`, M3a PR2, is its own row there, COO on this Mac), `agents.appBuildsOn`,
`agents.codexNoOutputTimeoutMin`, `jev.enabled`, `jev.decisions`,
`semanticSearchEnabled`, `backupManagedElsewhere` → `advanced`,
`chat.canvases` → `chat`, `app.*` → `general`, `heartbeat.*` → `background`,
the rest of `operator` → `personalization`. Key names did not change.
Sources holds one key since T3a (2026-11-15), `tracker.trackAgentWork`
(off, dryRun or on; filed under the old `connectors` id in `LEGACY_SECTIONS`);
its other changes go through `tracker.*` actions. `NONE` lists the keyless
parts vault, history, skills and selfhosted. `settings.get` with an id that is no longer a live section
returns exactly the keys it returned before the twelve-section list (`LEGACY_SECTIONS` and
`LEGACY_ALIASES`, frozen from `b0ed05aea`; `defaults` still gives all 23 of
Agents' keys), and its keyless lines match the section the id resolves to
(`paths` explains Sources and the Hollow folder).
`accounts`, `chat` and `shortcuts` were retired ids before the twelve (they
read Agents' 23 keys, Agents' again, and Interface's 12) and are live
sections now, so `settings.get` returns their own keys only: `accounts` gives
`agents.updates.claude`, `.codex`, `.antigravity`, `.grok` and `jev.apiKey`;
`chat` gives `chat.canvases`; `shortcuts` gives `shortcuts`. Kept on purpose
(R5, 2026-11-13): nothing in the repo asks those ids for an older key, and
every old key is still read by name or by its retired section (`defaults`,
`interface`). Pinned by `settings_keys`'
`live_accounts_chat_and_shortcuts_return_their_own_keys`.

Every old name stays a search word. The composer model menu's Edit opens
Chat. Usage has no Routing or Votes (overnight 11-08 S5); an old `feedback`
link opens Usage on Budgets. Disk space reads Warn under with
`disk_threshold` (no measure). Agent updates folds agents not installed.

### Usage in detail

Usage is the one place for how much each agent was used, how close each limit
is, and how runs share Claude accounts (Alex, 11-08). `settings/UsagePanel.tsx`
is a shell: the period (Today, This week, This month, Last 30 days) and
Refresh sit above a tab row (`usageTabs.ts`): **Limits** (Usage opens here),
**All agents**, one tab per agent with runs in the period (Claude, Codex,
Grok, Antigravity, Self-hosted, in that order), and **Budgets**. The tab row is
`data-jump-skip`.

- **Limits** (`UsageLimits.tsx`): Claude per added account (5-hour and weekly
  meters with the Move on at line drawn while spreading, reset time, In
  rotation, last run), Claude Code's login (same meters when it has a
  reading or pins chats), the "Read live from Claude" switch (the old Show
  plan usage opt-in, `quotaOptIn`; on, it shows `ClaudeQuotaCard`), Codex's
  plan windows (`ProviderQuotaCard`), Grok and Antigravity "Doesn't report plan
  limits", every budget as a meter (`budgetMeters.ts`: day and week first,
  fullest first, monthly below; a warn mark at its warn percent and, for a hard
  stop, a mark at the limit), the two spending caps (`BuiltInCaps`, edited in
  place), then **Spread usage** (`ClaudeLimits.tsx`: One account or Spread
  usage, Use, Move on at, Check usage, Resume on another account when one runs
  out and its notes). The meters and Spread usage share one accounts view
  (`claudeLimitsState.ts`); every control saves through `spreadUsageActions.ts`,
  the same calls Agents, Accounts made. Meters are `UsageMeter.tsx`
  (`styles/usage-limits.css`).
- **All agents and each agent tab** (`UsageAgentTab.tsx`): Totals (runs,
  tokens, cache share, which cost kind, average per run, runs not metered and
  why, a one-line note per agent), By day (`UsageInstruments`, stacked by agent;
  one agent alone on its tab; none on Antigravity), By model, By account
  (All, Claude, Codex), By where (workspace, surface, automation, the ten
  costliest conversations, each opening its chat through `openSessionByKey`,
  subagents) and Outcomes. All agents ends with the explainer, Recording since,
  a ledger error and Value this month.
- **Budgets**: the full `BudgetsPanel` (`caps={false}`: the caps are on Limits)
  and the CFO rows (`CfoUsageRows`).

`usage_summary(window, agent)` (`usage_ledger::summary_for`) takes `last30`
and an agent tab key and adds `agents` (every tab with runs, unfiltered),
`byModel`, `outcomes` (finished, stopped, stopped by a budget, ran out of
time, interrupted, failed, didn't start, still running, and moved to another
account: a failed-over chat turn's earlier attempt, found by run id and a
later row on another account) and `topConversations` (the ten costliest
`byLead` rows, by session key). By account and By workspace are the existing
`byAccount` and `byProject`; the per-agent daily series is `daily[].byAgent`,
or `daily` itself on an agent tab. Every list is built from the same rows, so
each sums to the tab's total. Budgets keep day, week and month.

Agents, Accounts keeps add, edit, remove and the other providers' sign-ins,
plus one link row ("Limits and Spread usage are in Usage"). Links to the old
place land on Usage, Limits: `SETTINGS_MOVED_CONTROLS` and
`resolveMovedControl` in `settingsSections.ts` send `accounts` or `defaults`
with Spread usage, Check usage, Move on at, In rotation or Resume on another
account to `usage`, and `jumpTo` asks for the label's tab
(`usageTabFor`, `requestUsageTab`).

### Settings shell: switching, unsaved drafts, search (P1-21)

`settings/controllers/settingsNavigation.ts` owns every section switch
(index, narrow picker, search hits, deep links, capability-doctor fixes).
A switch starts the new section at its top and focuses its `h2`
(`tabindex="-1"`); the open section is remembered for the session, so
Settings reopens where it was left. Forms that keep a local draft until
their own Save call `registerDirty(id, isDirty, label)`
(`settings/settingsDirty.ts`): Hollow folder, Agent
defaults (not while its save runs), the custom theme editor (once its name or
text differs from what it opened with), the companion
Backend connection and What the backend copies, the Disk space warning, and
the Self-hosted models add or edit form.
Not registered yet: Budgets' Change limit (`BuiltInCaps`, also mounted
outside Settings, so a registration there could ask in the wrong place),
the Claude account edit, API key paste and About you answers. A switch away
from a section with a dirty form shows an inline **Unsaved changes**
alertdialog (Keep editing focused, and on Keep editing focus returns to what
asked to switch; Escape keeps editing; Discard changes and open X leaves).
Leaving the Settings page itself doesn't ask, and neither does a jump to a
part of the open section (S2: its forms stay mounted). Instant controls (General's
rows, Search inside project files, effort) call `flashSaved(label)`, shown
in the section header's polite live region and cleared on a switch. Chat
and Keyboard shortcuts save as you change them; the other ten are "mixed".

**Search lands everywhere.** `settings/pageTargets.ts`
holds `SETTINGS_PAGE_ENTRIES`: settings whose home is another page, each a
one-line `{label, keywords, page, tab, open}` entry (today Learnings review in
Library, Learnings, and Automation schedules in Automations, and the other moved rows). The search lists them after the sections' own hits as "in Library" /
"in Automations" (`data-page-hit`), Enter opens one when no section matches, and
choosing one opens that page through its lazy opener. Labels must stay unique
among Settings' own control labels (a vitest guard). A hit inside a closed
`<details>` (System's Advanced fold) opens every fold above it first
(`openFoldsAround` in `settingsJump.ts`), then lands on the row and focuses its
control. Each capability-doctor check carries `fixSection` and `fixControl`
from `CAPABILITY_HOMES` (`capabilityDoctor.ts`): vault to System, Hollow folder;
agents to Advanced, Runtime (none found: Accounts); Plaud to
Connections, Recordings; tracker to Connections, Task tracker; personal context (PX7) to Context, Personal; backup to System,
Backup management (its checkbox). Leaving Settings for a page hit asks the
unsaved-changes question first, like a section switch. The fix button jumps to that row, never to the section the
doctor is shown in. Library's searches (All's and Browse folders') show `SemanticSearchLink`
(`components/SemanticSearchLink.tsx`, "Turn on search by meaning") only while
semantic search is off; it opens Settings on the Index my vault for semantic
search switch.

Search: every registered control must be reachable by its jump
(`settings-shell.spec.ts` walks them all). Jump targets are row labels,
subheads, `h3`/`h4`, labels, buttons, `summary` and
`[data-settings-control="<label>"]` (for a control named only by an
aria-label). A control outside any row is marked itself, never the whole
section. A control shown only in some states names `near`, the control a
jump lands on otherwise (Indexing progress → the index switch). Enter in the
search opens its top hit. Below 860 px the index keeps only its search,
above the section picker, and lists hits while searching. The vitest label
guard strips comments before checking a label is on screen.

### Companion apps (T0 + W1 bridge, 2026-10-12)

Small apps (the Mac menu-bar app first; then the web app; phones once the Apple Developer Program is a yes) live in a
separate repo, `~/Hollow/Repositories/hollow-companion` (private GitHub `example-user/hollow-companion`). Hollow stays the only writer of vault Markdown: a paired
device writes a signed, write-once op into `.hollow-sync/inbox/<deviceId>/<opId>.json`; `companion_inbox.rs` verifies it
(ECDSA P-256 via `ring`, key in `<support>/Companion/devices.json`), applies it once (ledger
`<support>/Companion/applied.db`) through the task broker's validation under a short `companion-` lease, merges a stale
edit per field, sends true overlaps to Waiting on you as `companion_conflict` (Use the device's value / Keep the Mac's),
and writes `.hollow-sync/receipts/…`. An op whose signed `at` is more than 14 days old (design §3.6) is held instead,
on both roads (`deliver`): its envelope waits in the ledger database's `held` table, a `companion_old_op` card (Old
change from a device: Apply it / Discard it) asks, and the device gets no receipt until the person decides; Apply it
re-verifies and applies it once through the same path (only while the card's fingerprint, `opKind` and `sentAt`
still match the held op; a broker refusal closes the card), Discard it answers `refused` (also on an edited card: the
op its fingerprint names); a later copy of a held op is
never opened, and revoking the device drops its held ops and cards. Ops wait in the inbox while a run holds the vault. Pairing: Settings → Devices
→ Companion devices shows a one-time code kept only in memory (user-only commands in the command map).
`.hollow-sync/` is protected from Hollow's own agent write gates; the signature is the real boundary. Contract and
fixtures: `docs/COMPANION_PROTOCOL.md`, `docs/companion-op.schema.json`, `hollow companion-fixtures <dir>`; debug builds
also have `hollow companion-apply <temp vault> <store> [code]` for end-to-end tests (refuses non-temp vaults).

**Backend road (W1, `companion_bridge.rs`).** For the web app (and phones later), devices post the same signed
envelopes to `hollow-companiond` (in the companion repo's server folder; its contract is BACKEND_API.md in that repo's docs). Hollow is its
one applier client: a tokio task long-polls `/api/v1/applier/poll`, runs each op's exact envelope text through
`companion_inbox::deliver` (the same verification, broker path and **ledger** as the inbox, under the inbox's pass
lock, so an op on both roads applies once; a repeat counts the conflict cards its first apply raised, so a cancelled
cycle's lost Review refresh is made up), posts receipts back (not into the vault; a held old op gets none until its
card is decided, then the decision is owed in `applied.db`'s `outbox` table and each cycle posts it before polling,
because the daemon may have expired the op and would never hand it out again; removing the backend drops those rows),
decides backend pairing requests
with the same in-memory code (`decide_remote_pairing`), and pushes the paired-device list plus a read-only mirror:
only `tasks/**.md` (no hidden components; path, hash, bytes and the fixtures' parse shape) and project
`ref`/title/status (decision D5; the daemon refuses anything else too). A busy vault stops a batch and re-polls from
that op. Configured in Settings → Devices → Companion devices → Backend (URL + the daemon's applier token, stored in
`<support>/Companion/backend.json` 0600; `http://` only to 127.0.0.1/localhost, else `https://`; an empty token
keeps the saved one only for the same origin); off when absent. The long-poll and the mirror pusher run side by side:
a mirror failure shows in Settings and never stops ops from applying; the mirror goes in batches under 1 MiB and skips
files the daemon would refuse (over 256 KiB, odd paths). Each request carries `X-Hollow-Applier`
(`<support>/Companion/applier-id`): the daemon lets one Hollow at a time be its applier. Backend pairing always needs
the open code, even for a known device, and the decision names the request's ticket.
`companion_backend_status`/`_set`/`_repush` are user-only (the token lets a client act as this Hollow).

**Backend v2 (overnight 10-12/13, `companion_docs.rs`).** Every applier call says `X-Hollow-Bridge: 2` and
`X-Hollow-Kinds` (= `companion_inbox::APPLIED_KINDS`; the daemon hands an applier no other kind, so add a kind there
only together with its handler). On its own loop beside the task mirror, a document pusher copies what the scope
allows to `PUT /api/v2/applier/docs` every 15 s (backing off on errors, skipping a path the daemon refuses),
incrementally by hash (it starts from the
daemon's `GET` index; a 404 or a page instead of JSON means an older daemon: no documents, nothing else changes). Scope:
`<support>/Companion/scope.json` (`includeWorkData` off; `personalWorkspaces`, seeded once with Hollow's own and clearly
personal workspaces, never `notion-*`, `linear-*` or a work name), changed only in Settings → Devices → Companion devices → Backend → What the
backend copies (`companion_scope_get`/`_set`, user-only). Personal: `notes/`, `library/notes/`, and each listed
workspace's brief/plan/sources/agents, `outputs/**`, `sources/**` (recordings = `sources/recording-*.md`). Work data
adds `library/work`, `library/atlas`, `learnings/`, `PROJECT.md` and every workspace except a `linear-*` one. Never `me/`, sessions, dot paths,
symlinks, `library/records/` or a `linear-*` workspace (those two never mirror, in either scope; the walk doesn't follow symlinks). Front matter `scope: work` opts a file or a workspace out of personal scope; nothing opts one in but
the list. Files over 256 KiB go as metadata. A document failure shows in Settings and never stops tasks or ops.
Settings shows connection status and mirror counts first; configured backends keep setup under Change connection.
Unknown status/selection stays unavailable, not zero/off. In-flight mutations invalidate older reads; credential and
scope controls are locked while saving. Visible-only polling preserves an edited URL. Details and browser checks:
[Companion settings](../COMPANION_SETTINGS.md).
Connecting: `companion_backend_claim` (user-only) trades the one-time code the backend's `/setup` page shows the
tailnet user for an applier token (`POST /api/v2/applier/claim`) and saves URL + token; pasting the token stays as
the fallback. **Test connection** (ST5, `companion_backend_test`, user-only; next to Re-send for the saved
connection, next to Save for a typed URL + token) saves nothing: `GET /api/v1/health` (no token: is a companion
backend there?) then `GET /api/v1/applier/status` with the token, as the bridge sends it (with a good token it claims or
renews this Hollow's applier lease there for 90 s, as connecting would); the answer is one plain sentence (connected with device/task counts, refused token, another Hollow is
the applier, not a companion backend, can't reach the host). A blank token reuses the saved token only for the saved
address. The backend offers a code only while no Hollow is connected, and a claim rotates its token (any other
holder is cut off). Devices can also send `note.create`/`note.edit`/`note.append`/`workspace.append` (`companion_notes.rs`, in
`APPLIED_KINDS`): every target passes the same rules and scope with no symlink or letter-case mismatch on its path;
a stale `note.edit` saves the device's version as a sibling copy and answers `conflict`. Debug builds
have `hollow companion-bridge-once <temp vault> <store> <url> <token> [code]`; the companion repo's
e2e-localhost script drives the whole loop on localhost with it.

**iPhone app (the phone chat API, 2026-11-04, `phone_access.rs`).** The Hollow iPhone app (in the companion repo's
`ios/` folder) lists chats, reads one, follows a running turn live, sends a message, starts a chat in a workspace and
stops a turn, over a small HTTP API Hollow serves on `127.0.0.1:47680` only; `tailscale serve --bg --https=8443
http://127.0.0.1:47680` publishes it to the tailnet (never Funnel: Hollow refuses Funnel's traffic, and any request
without this Mac's Tailscale user unless it came straight from this Mac). The contract and its fixtures are
`docs/PHONE_CHAT_API.md` and `docs/phone-api-fixtures/`, the same files in both repos; the tests compare Hollow's
shapes to the fixtures as JSON values. Settings → Devices → Companion devices → iPhone app holds the switch (off
by default; once on, it starts with Hollow), the server and Tailscale lines (listening, or the bind error; published,
Funnel, or another target on 8443, from `tailscale serve status --json`; a raw `--tcp` forward to 47680 on any port,
saved, foreground or a service's, is a problem, since it passes on nothing of the caller and its requests would pass
as this Mac's own), Publish on Tailscale / Stop publishing (which also removes each saved forward with `tailscale
serve --tcp=<port> off`, every step tried past a refusal and none after a timeout, turns 8443 off only when it serves
Hollow, never another app's, changes nothing when the status can't be read, and reads Tailscale again after a
failure; `off` reaches only the saved config, so a foreground forward gets a line saying to stop its `tailscale
serve` with Control-C in its terminal, and a service's the exact `tailscale serve --service=<name> --tcp=<port>
off` to run, with the CLI's full path when no PATH folder holds `tailscale`, or the steps in words for a name that
isn't `svc:` and lower-case letters, digits and hyphens, with no button),
Check again (reads Tailscale now), and Pair an iPhone: an 8-character code for 10 minutes or one use, the base URL
and a QR code of the `hollow-companion://pair` link (an SVG from the `qrcode` crate, shown as an image). Paired
phones join the one device list above (`iPhone app · <model>`, paired, last seen, Revoke); revoking removes the token
hash and closes the phone's event streams. While the switch is on with a phone paired, Hollow copies every chat event
into a live view of each running turn (`phone_access/live.rs`), whatever streams are open, so a stream swap or a while
in the background loses nothing; switching off or revoking the last phone drops them. A phone's send is the composer's (`commands::send_unless_busy`, typed, depth 0,
the chat's agent, model, folder and native session; a chat with a turn queued or running, or a message the window holds
for after one, is 409 busy, never a hold the window couldn't show) in the mode the window shows for that chat: the person's choice
there, else the window's Plan fallback (its storage filled up), else the last turn's mode. The window reports each
choice when it is made (a new chat's first send included), one report at a time in order, and its whole record
(`replace`) when the one it keeps on disk changes wholesale and when Companion devices opens
(`phone_access_chat_modes`, kept in the record, at most 500, each report read, built and written under the
store's lock, and none written while the switch is off and no phone is paired); the copy follows the disk, which
is what the window shows once reopened. After a launch, a phone's chat read, send or new chat that relies on the record
starts a round of asks for the window's whole record unless one is under way (`phone-access-modes-wanted`,
`routes::WindowModes`): at once, then every 250 ms for 30 seconds or until the window answers. The request that
starts it and any in its first 2 seconds wait for the answer, later ones go on, and the next request after the round
starts another, so a silent window costs one wait every 30 seconds and a window that starts listening is heard
within 250 ms; no launch call. Across a mode class
(Plan, or acting) a send starts a fresh native session, as the window does, and a chat's read names that mode.
A new chat starts in the mode Settings → Models gives new chats, which the window reports when it changes, when
Companion devices opens and when asked, and the record keeps (`phone_access_new_chat_mode`). The commands
`phone_access_status`, `_set_enabled`, `_pairing_start`, `_pairing_cancel`, `_revoke`, `_publish`, `_unpublish`,
`_new_chat_mode` and `_chat_modes` are user-only and refuse any window but the main one (the status returns the open
code).

**Keep Hollow running with the lid closed (2026-11-05, `phone_access/lid.rs`, `settings/PhoneLidClosed.tsx`).** A
switch in the iPhone app section, off by default; turning it on needs the iPhone app on, turning it off always works.
macOS keeps a closed lid awake only with system sleep disabled (`pmset -a disablesleep 1`, root, kept across
restarts), and while that is set it refuses every sleep, `pmset sleepnow` included, so the switch installs two root
LaunchDaemons, each with its `sh` inline in its plist, after one macOS administrator prompt (`osascript`, the person's
click), and Off removes both after another. The helper (`com.hollow.app.lid-closed`, every 10 seconds and when the
armed folder changes) disables system sleep only while an `armed.<pid>` file names a process running the executable it
was installed for, by the kernel's path for that pid (`lsof -b`, never the name `ps` shows, which a process sets), and
the Mac is on power or On battery too is ticked (baked into the plists, so changing it prompts again); otherwise it **On battery too stops at 10% charge** (ENG-139): below it the helper no longer wants sleep off, so it turns sleep back on and sleeps a closed Mac; an unreadable charge leaves the rule as it was. Off turns sleep back on (marker) before it empties the armed folder. Update the helper is offered only with the iPhone app on.
turns sleep back on, only when its marker (`/Library/Application Support/Hollow/lid-closed-sleep-disabled`) says it
turned it off, and sleeps a closed Mac on battery. The restore check (`com.hollow.app.lid-closed-restore`, at load and
every 10 seconds as its own job) does only the unplugged half: off power without On battery too, with the marker, it
turns sleep back on and sleeps a closed Mac, reading no folder and looking up no pid. When both find the marker, only
the pass whose `rm` removes it sleeps the Mac, so a second `sleepnow` can't land after the wake. Hollow keeps its own
`armed.<pid>` only while the iPhone app is on and a phone is paired (only the copy the helper serves arms or clears the
folder), and holds `caffeinate -i -w <pid>` under the same conditions. The armed folder,
`/Library/Application Support/Hollow/lid-closed-armed`, is root's, made by the install with one ACL entry that lets the
person add and remove files but not folders, so it can't be swapped for a link or mounted over. Its files are still
the person's, and Hollow's executable also runs as its MCP servers, so while the helper is installed any of their
processes can hold it armed; only the plists' power rule limits that. A helper pass has no time bound: unplugged
without On battery too it doesn't read the folder, otherwise it looks up four names at most, but one `lsof` lookup can
wait on a file system a process of the person's serves (the kernel stats each file the pid maps; `-b` stops only
lsof's own calls), and launchd starts no other pass of a job while one runs, so such a process can hold sleep off on
power for as long as it keeps that lookup waiting. Unplugged, the restore check turns sleep back on within 10 seconds
all the same, and a helper pass reads the power source and `SleepDisabled` again after its lookups, so one that was
held up never turns sleep off after an unplug. Without a `"SleepDisabled" = ` line from `ioreg` neither turns sleep
off or sleeps the Mac, but both still turn it back on with the marker when it shouldn't be off. The install goes ahead
only when `/Library/Application Support/Hollow` is absent or a real folder of root's (then root:wheel 755 with no ACL)
and no link stands at the armed folder, the marker or a last-pass file; a pass reads the folder only while none is a
link and writes nothing while one is. Every finished pass (and the install) touches its last-pass file
(`/Library/Application Support/Hollow/lid-closed-last-pass`, `lid-closed-restore-last-pass`), and the status gives
their ages and how long ago Hollow stopped asking, so Settings never says the lid sleeps the Mac while sleep is still
off, or can't be read with the marker set: it says sleep should be back within 10 seconds, and after 30 seconds
without a pass of either daemon that the helper isn't running and Off turns sleep back on; it promises that unplugging
sleeps a closed Mac only while the restore check passes. Installed plists that differ from the two this Hollow writes,
or one of them missing, show Update the helper (one prompt for both). Sleep disabled by anyone else is named and left
alone; Hollow's marker without its helper offers Turn sleep back on. `phone_lid_status` and `phone_lid_set` are
user-only, main window only. **Removing it without Hollow** (an administrator, in Terminal):
`sudo launchctl bootout system/com.hollow.app.lid-closed`,
`sudo launchctl bootout system/com.hollow.app.lid-closed-restore`,
`sudo rm /Library/LaunchDaemons/com.hollow.app.lid-closed.plist /Library/LaunchDaemons/com.hollow.app.lid-closed-restore.plist`,
then `sudo pmset -a disablesleep 0` when `/Library/Application Support/Hollow/lid-closed-sleep-disabled` exists, and
last `sudo rm -rf "/Library/Application Support/Hollow"` (only this helper keeps files there). What only a real click
checks: the prompt, both daemons' load, the ACL as root and their passes on real hardware.

**iPhone app v1.1 (2026-11-05).** Four additions, in the contract's v1.1 section. `GET /v1/waiting` lists every chat
with a pending question card (the chats whose `needsAnswer` is true), newest card first, and a turn's card carries
its id, which `POST /v1/sessions/{key}/questions/{id}/answer` names: Hollow maps the phone's answers onto the card's
own answer, resolves the card against the native session the send resumes (`interactions::resolve`) and sends
Hollow's answer text verbatim as the chat's next typed turn, as the window's card Send does; a card that can't be
answered is 409 `question_closed`, a busy chat 409 `busy`, and a send that can't be queued, or an answer row taken
out of the queue before it starts (`live::AnswerTurns`), opens the card again. Push (`phone_access/push.rs`): a phone
registers its APNs token (`POST`/`DELETE /v1/push`, kept on its record, gone with a revoke); Hollow signs an ES256 JWT
with the `.p8` key the person imports into Hollow's Keychain item (`secret_store` key `apns`), signs it again once its
`iat` is 50 minutes old by the wall clock (an `Instant` stops while the Mac sleeps), and the system `curl` sends over
HTTP/2 with the URL, headers and body on its stdin, never its argv; a push Apple refuses with `ExpiredProviderToken`
goes once more with a fresh JWT. A turn the person sent that ends pushes `turnFinished` (`runStarted`'s `attended`,
from `chat::turn_attended`; never a night item, a Main agent wake, check-in or swarm wake, or an agent's chat, though
a wake's `unattended` is false); a new card pushes `question` whatever started its turn; never to a phone with a
stream open for that chat, and a token Apple calls invalid is dropped. The section's Push notifications block
shows the state (ready, no key, error), the key, the phones with notifications on, the last send and the last error,
with Import key (a `.p8` from a file dialog; Key ID from Apple's `AuthKey_<Key ID>.p8` name unless typed; team
`TEAM000000` unless changed), Send test notification and Remove key (`phone_push_import_key`, `_remove_key`, `_test`,
user-only, main window only). Runs on: `runsOn` on a new chat sets the workspace's own Runs on when the chat would
run elsewhere, as the window's empty-state control does (Runs on is a workspace's, never a chat's); a session's
`runsOn` is `remote_session::chosen_runner` for its folder and agent (self-hosted chats stay here); `canRunOnMini` is
true for any workspace but the built-ins while a runner is set up; `/v1/me`'s `runner` is the runner's last check,
checked again in the background when over five minutes old.

**iPhone app v1.2: notification actions (2026-11-07, `docs/phone-api/notification-actions.md`).** No route: the push
payload gains what the app needs for buttons. `turnFinished` names `aps.category` `hollow.turn` (Reply, Open);
`push::card_actions` gives a `question` push `hollow.question`, `mutable-content` 1 and a `card` (`questionId`, `itemId`,
`options` of `id` and a label cut to 40 characters) only when the card asks one `single` question with 2 to 4 options
whose ids are non-empty, at most 40 characters and not repeated (a confirmation card and a verdicts card with one item
qualify; multi, text, several questions and checkbox cards don't), and the app's notification service extension turns
the options into buttons. Replying is a send and an option an answer, through the v1.1 routes; the app's side, its
refusals and the older-Hollow behaviour are in the contract. A test notification and any other question are unchanged.
**iPhone app v1.2: attachments (2026-11-07).** Contract `docs/phone-api/attachments.md`, fixtures in
`docs/phone-api-fixtures/attachments/` (identical in `hollow-companion`); `phone_access/attachments.rs` owns it. An
upload (`POST /v1/sessions/{key}/attachments` or `/v1/workspaces/{id}/attachments` for a chat that doesn't exist yet)
is the file's raw bytes: up to 25 MiB (the only routes past the 256 KiB body limit; read within five minutes and
never idle for 60 seconds, three at a time, a fourth waits 20 seconds for a place and is then 503; the place goes with
the file's bytes (`attachments::with_place`, a `Bytes::from_owner`), so it is given back when the save ends, not when
the phone's connection closes and hyper drops the handler; a refusal that
comes before the body is read drains the body first so the phone hears it), saved by `chat::attachments_dir` and `chat::unique_attachment_suffix`, the helpers
the Mac composer's `save_attachment` shares, into the chat folder's hidden `.hollow-attachments/` with the composer's
refusals (an unsafe or missing folder), written under a hidden `.<suffix>.part` name and renamed when whole (the
id search and `inside` pass over a dot name, so a part a crash left is never an attachment). Hollow reads the bytes (`attachments::sniff`: PNG, JPEG, GIF and WebP are
`image`, everything else `file`; HEIC, PDF, zip, MP4/MOV, M4A and text are known) and a name or `Content-Type` never
decides the kind or, for a known binary type, the extension; a repeated `clientUploadId` for the same chat or
workspace returns the first answer (the save and the remembered answer run to the end in their own task, as a send
does).
There is no staging registry: an id is `at-` and 16 hex characters of the SHA-256 of the real attachments folder and
the file name, so a send (`attachmentIds`, at most 10, 50 MiB together; the words may be empty) lists the folder to
find its files, a restart loses nothing, and a missing one is 404 `attachment_gone` before anything is queued. The
prompt Hollow composes is the composer's (`attachments::blocks`, held to `composePayload` by a test): the intent, the
words, an `<attached-images>` block, an `<attached-files>` block. Turns record their files: when a turn starts,
`chat::execute_turn` hands the composed prompt to `attachments::record_turn`, which keeps the names (never paths) of
the files its trailing blocks name that are plain files directly inside the chat's attachments folder (links,
folders and files elsewhere are skipped, so a block naming `~/.ssh/id_ed25519` records nothing) in the new
`turns.attachments` column, for a turn from the phone, the window or an agent alike. `GET /v1/sessions/{key}` puts
them on the user turns (`attachments::fill`; `available: false` once the file is gone) and
`GET /v1/sessions/{key}/attachments/{id}` serves one, only a file a turn of that chat carried and only from that
folder (`Reply::Bytes`: the type Hollow worked out, `nosniff`, `Content-Security-Policy: default-src 'none'; sandbox`,
an ASCII `Content-Disposition` name, `inline` for an `image` and `attachment` for every other file, never over 50 MiB).
The file is opened by `open_served` (`O_NOFOLLOW | O_NONBLOCK`, plus `O_NOFOLLOW_ANY` on macOS so a link anywhere in
the resolved path is refused; a regular file judged on the opened handle, so a link swapped in after `inside` looked
is never read) and read whole into memory, so four downloads run at once
(`DOWNLOADS_AT_ONCE`, a fifth waits 20 seconds and is 503) and the place rides with the response body until it is
written.
**iPhone app v1.2: Live Activity (2026-11-07, `docs/phone-api/live-activity.md`).** The app starts and updates a running
turn's Live Activity itself while it is open; Hollow's half is for when it is closed. `POST /v1/live-activity` takes
`{sessionKey, token, environment}` (the activity's push token; `DELETE ?session=<key>` or with none drops it) and keeps it in
`phone_access/live_activity.rs`'s `Activities`, **in memory only**: one per phone and chat, 8 chats per phone, 12 hours,
gone at the turn's end, on Apple's 410 or `BadDeviceToken`, on `DELETE /v1/push` (Unpair) and on a revoke (checked at
every send against the paired phones); a key that isn't a chat is ignored with 204 so a 404 always means no v1.2.
`Pusher::observe` makes `Job::LiveStep` (tool or text events, one job per burst; a chat's tool calls are followed by id in
`live_activity::ToolWatch`, because the live buffer stops at 200), `LiveWaiting` (a new card; a step doesn't replace it
while the card is pending), `LiveEnd` (`done`, ended as the store says: `push::stored_phase` reads the latest turn's
status, the `cancelled` or `error` events only when it can't) and `LiveRefresh` (5 minutes after the last push: the
unchanged state again with a new `stale-date`, so a long step never goes stale) only for a chat with a token,
whoever started the turn; they send with `apns-push-type: liveactivity`, topic `com.hollow.app.companion.push-type.liveactivity`
(`push::PushClass`: priority 5 for a step, 10 for `waiting` and `end`), the same key and JWT as notifications, at most one
step per chat every 10 seconds (the newest step wins; the job comes back after the gap and reads it then). The body is a
phase and a step label from a tool's **name** only (`live_activity::step_for_tool`, the table in `step-labels.json` that
the app tests too): a tool's detail, the chat's text and its title never reach Apple. No key imported: nothing is sent
and nothing is wrong. Settings has no new control: the push key (Settings → Devices → Companion devices → iPhone
app) is the one thing it needs.

### Access and trust for a paired phone (OC16, 2026-11-17)

What pairing gives a phone, what revoking does, and what stays the person's job. Each claim names where it comes from.
The full route list, with each route's scope, is `routes::ROUTES` in `src-tauri/src/phone_access/routes.rs`. The wire
contract ([PHONE_CHAT_API.md](../PHONE_CHAT_API.md)) is shared with the iPhone repository, so this note lives here.

- **What a paired phone can do.** Read every chat Hollow lists (titles, previews, whole transcripts) and follow a
  running turn; send a message to an existing chat, start a chat in a workspace and stop a running turn; answer an open
  question card ([v1.1](../PHONE_CHAT_API.md#post-v1sessionskeyquestionsquestionidanswer)); read the workspace list, one
  workspace's outcome, plan and a file preview, and search ([workspaces](../phone-api/workspaces.md),
  [search](../phone-api/search.md)); add, finish and reschedule tasks, save notes, upload attachments
  ([tasks](../phone-api/tasks.md), [notes](../phone-api/notes.md), [attachments](../phone-api/attachments.md)); register
  for push and Live Activity. A phone also picks the agent, model, effort and permission mode of the turn it sends,
  Bypass all included: "Pairing authorizes the same permission choices as the person's composer"
  ([chat controls](../phone-api/chat-controls.md#choose-settings-for-a-send)). So a paired phone can have an agent do on
  the Mac whatever that mode lets an agent do: treat pairing as handing over the person's hands on chats.
- **Cards it can answer (B4, 2026-11-19).** An Allow/Deny card (installs, app actions, computer control, mail sends) only
  from a phone the person enrolled on the Mac: Settings, Devices, Companion devices, the phone's row, Turn on approvals
  shows a one-time code, and the phone proves the genuine app with App Attest. Its answer is signed in the Secure
  Enclave (Allow behind Face ID, each time), so a copied bearer token approves nothing
  ([approvals](../phone-api/approvals.md), `phone_access/approvals_api.rs`). While a phone has approvals on, cards also
  wait while the window is closed and longer (5 minutes by default).
- **What it cannot do.** Change Hollow's settings or secrets, or pair, revoke or list another device: none of those is
  a phone route (`routes::ROUTES`). Turning approvals on needs the Mac.
- **Revoking.** Settings, Devices, Companion devices, Revoke removes the phone's record (`store::revoke`). The token
  is checked on every request and again at each effect's commit point, so a revoked phone's next request gets 401 and
  nothing it had in flight lands ([Who may call](../PHONE_CHAT_API.md#who-may-call); `phone_access/revoke_tests.rs`).
  Its open event streams, push registration and Live Activity tokens go with it (`phone_access::revoke`). Hollow
  never contacts the phone to say so. That is the verified part: the Mac refuses the token whether or not the phone
  is online.
- **Revoking while the phone is offline: only part of this is verified.** What the iPhone app already holds is out of
  Hollow's reach. The token stays in the phone's Keychain, and the widget snapshot (chat titles and question
  previews) stays there until the app is next refused and empties it
  ([widgets](../phone-api/widgets.md#the-snapshot)). Hollow cannot wipe either remotely, so for a lost phone, revoke
  it and also sign out or erase the phone itself. What the app does offline beyond the widget snapshot lives in the
  iPhone repository and was not checked here.
- **The pairing code and link are bearer credentials.** Whoever opens the link or types the code first, while the
  window is open, pairs their device. The window is good for 10 minutes or one use, closes after five wrong codes, and
  there is one at a time (`store::PAIRING_MINUTES`, `MAX_WRONG_CODES`; [Pairing](../PHONE_CHAT_API.md#pairing)). The
  link carries the code and the address, not a token. The token it produces has **no expiry**: it works until the
  phone is revoked. Don't post a pairing QR code or link, and cancel a window you no longer need.
- **Where the token is exposed.** Hollow keeps only its SHA-256 (`phone.json`, mode 0600); the phone keeps the token
  in the Keychain. Whoever copies it from a phone backup or a log can call every route above until the phone is
  revoked. The token is not the only gate: a tailnet request carrying another user's login is refused 403, and the API
  is never published through Funnel ([Who may call](../PHONE_CHAT_API.md#who-may-call),
  [Transport](../PHONE_CHAT_API.md#transport)).
- **Not claimed:** a hash-checked or signed mobile UI bundle. This repository serves none (the phone is a native app
  in the iPhone repository), so nothing here says how it is verified.

### Waiting on you (merged plan wave 3, decision 7)

One surface for everything waiting on the person. It replaced Activity (the
bell's old notification dropdown), Work's "Needs you" list and the five
review inboxes (Tasks, Settings → Personalization, Automations ×3).

- **Needs you** lives in the bell, with a navigation-only summary. **See all**
  opens `pages/Review.tsx`: Waiting · Incoming, with All · Decisions · Results ·
  Failures filters inside the retained Waiting list (`store/destinations.ts`).
  Incoming's Sources filter keeps the stored `import` kind and keyboard position.
  The old `review` id, ⌘5 and Go → Needs you still work, without a rail row.
  Other routes include Work's
  count line, each page's `WaitingLink` (filtered to its review kinds) and
  every old review deep link (`requestSuggestions(context)` →
  `openWaitingOnYou(context)`). The panel is a lazy chunk.
- **The bell's waiting count** (`waitingBadgeState`): unknown until the first
  answer and after a failed read. The description says count unavailable and
  the badge shows !. Partial reads name missing sources. Unsaved drafts are
  named apart and never added to the waiting count; Home and Main use the same
  number. `NotificationBell.tsx` drives refreshes (approvals, sessions, review items,
  notifications, the open chat, and a minute passing).
- **Drafts (D3):** a file draft is listed only when no editor has it open
  (`fileDrafts.holdDraftOpen`, held by `WorkFileEditor`) and it has idled
  ten minutes (`countedDrafts`, `DRAFT_IDLE_MS`). Drafts wait in All only,
  where their row has Discard (`discardWaitingDraft`: storage and the
  editor's memory copy, with Undo that writes it back); All's tab count
  includes them, Decisions' doesn't.
- **Content:** `list_needs_you` (`needs_you.rs`, 30 a page), grouped by
  `groupWaiting` (`src/waitingOnYou.ts`): Failures, Interrupted by a restart (overnight
  A1), Approvals and questions (ask-canvases too), Results to review, Review
  items and proposals, Sync conflicts, Conflict copies (overnight
  A10), Unsaved drafts. Unavailable sources are named; a failed load says
  so. A `kinds` filter reads only the sources those kinds come from, so
  `unavailable` names only sources that were asked for.
  Shared solid cards keep stable group/row identity; long titles wrap.
  Running-session asks lead within Approvals and questions. Paging preserves
  group order, so failures loaded by Show more join the first group.
  Only first load shows the static shared Skeleton; refresh retains loaded
  rows, and failed/partial loads never announce a complete empty state.
- **Ask-canvases (kind `canvas`, Canvas C4 A3):** every saved canvas (each
  workspace's `outputs/canvases/`, plus app data's per-conversation stores,
  which the command passes in) whose current revision is `ask`, while the
  conversation that emitted it exists, isn't archived, still lives in that
  store's workspace, and has sent no message answering that revision
  (`session_store::session_asks` + `canvas_answers::answered`). `sourceId` is
  `<canvas id>@<revision>`, the id `canvas:<session key>/<sourceId>`. The list
  (and the Review row's count) also refreshes when the open chat gains a turn or ends a run.
- **Recheck memory (kind `memory_recheck`, ME2, 2026-11-18):** a memory or
  home file of the personal clone that recall labelled stale
  (`context_stale.rs`: past `valid_until`, `verified_at` over 180 days, or a
  home `Last checked` over 30) is a review card "Recheck memory/x.md", once
  per file and stale date (the fingerprint), at most 5 open at once.
  Rechecked or Dismiss only closes it; the person re-dates the file.
- **Opening is by exact id** (`openWaitingRow`): a conversation by key, an
  ask-canvas as its conversation scrolled to that canvas revision
  (`store/canvasReveal.ts`; `ChatBody` takes the request once the
  transcript has loaded, renders the turn, found by `canvasTurnIndex` in
  `src/canvasTurn.ts`, and scrolls to the frame with that `data-revision`), a
  Result by workspace and path, a review item as its `ReviewCard` in place
  (re-reading the review queue snapshot on every open), a failure through its
  notification, the restart item as its Resume confirm (the panel stays), and
  conflict copies as `ConflictCopiesDialog`.
- **Person-only actions (decision 3):** every `ReviewCard` action is a UI
  action; the backend refuses an agent caller. Notification history lives only
  in Bell → Notifications, with saved filters, repeats, full Details and row
  actions. Its App messages section keeps the newest 50 session-local notices, failures
  marked in words, with Copy; clearing notifications does not clear those messages.
  History keys: j/k, arrows, Enter, x to mark read, Escape to close the bell.
- **Learnings in Needs you (overhaul J7 review, 2026-11-08):** the bell and
  Needs you list and count each pending learning (`list_needs_you` keeps
  them), but its row says "Decide in Library Learnings" and opens Library →
  Learnings (`opensInLearnings`); no learning card or bulk bar opens there.
  Library's Learnings tab shows the pending count from the review-queue
  snapshot (`learningsWaiting`; none while unknown) as its count and a
  "N pending" description.
- **Bulk review of learnings (Library → Learnings since J7):** with 2+
  learnings pending, each card has a checkbox and a bar has "Select all
  learnings (up to 500)". The bar says how many selected ones the Jev
  duplicate check flagged ("N may repeat a saved learning"); they stay
  selectable.
  Save selected / Reject selected (reason required, at most 500 chars) call
  `resolve_learning_proposals_batch(ids, action, reason, editedStatement)`
  (`vault_learning::resolve_batch_edited`). That resolves each proposal on its own
  and returns a per-item `LearningBatchOutcome` (ts-rs) plus a `batchId`. A
  row's Save is the same call with one id, plus `editedStatement` when the
  person changed the words (`editedWords`), so it gets the same snackbar and
  "Undo last batch" (`offerUndo`). A
  bad request (no ids, unknown action, a reason on Accept, one too long, more
  than 500 ids, edited words with anything but the Accept of one id) changes
  nothing. Items that fail stay ticked and the snackbar
  names them by subject. Only pending items are resolved (a snoozed one is
  refused). A failed accept goes back to pending without `learnedPath` or
  `batchId`. Its Undo is `undo_learning_batch(batchId)`, from the batch's 10 s
  snackbar or "Undo last batch (N learnings accepted)" above the list, which
  stays for the session until a clean Undo, or an Undo that finds nothing
  left ("Nothing left to undo"), lets it go (`batchAfterUndo`,
  `nothingLeftToUndo`); a partial Undo keeps it for the learnings that stayed.
  Undo takes the items still accepted
  or rejected under that `batchId`. For each one it checks the note is this
  proposal's (`proposal_id`) and reopens the item with a compare-and-swap on
  status and payload (`review_queue::reopen`). Only then does it move the
  `learnedPath` note to `.trash/`. A failed reopen touches no note: say an
  earlier Undo got there first, or the item was re-accepted at the same path.
  A failed trash puts the item back as it was (`review_queue::restore`), so
  Undo can be retried. Undo clears `batchId`, `learnedPath` and
  `rejectReason`, so a second Undo fails. The batch state lives on the items'
  payloads (no new file). The backend can undo a batch after a restart, but
  the UI offers only the session's last batch. Pure
  selection/message rules: `src/learningBulk.ts`. Both commands are
  `user-only` and in `PERSON_RESOLVES`. `no_action_resolves_learnings_in_bulk`
  pins that no app action (and nothing in `app_broker.rs`) can reach them.
- **Review's one keymap (P1-13, `src/reviewKeys.ts`):** x dismisses (a
  notification: mark read, its × button deletes; an Incoming row: Reject,
  Mark read or Dismiss), a accepts, Shift-R rejects a card; e edits and z
  snoozes a card until 09:00 the next morning (`snoozeUntil`) with Undo, and
  Decisions lists what is snoozed with Wake now (`wake_review_item`,
  user-only). Card key hints show only while shortcuts are on. Waiting's All is
  grouped by kind, newest first in each, with asks whose chat is running
  first; a Workspace filter narrows the rows and counts; both load notices
  have Try again; failure rows have Mark handled (and x), and opening one, here
  or in Incoming, leaves it waiting (run 13 FIX-MR); open cards, the filter and
  each view's scroll survive leaving Review and coming back.
- **One Reject's Undo (P0-10):** Shift-R on a review card (or x on its
  Incoming row) shows "Rejected" with Undo for 8 s; Undo is `undo_review_reject(id)`
  (`review_queue::undo_reject(id, expectedUpdatedAt)`, user-only, in
  `PERSON_RESOLVES`): the same item back to pending with its payload, through
  `reopen`'s compare-and-swap, so a second Undo fails. It undoes only the
  rejection the notice saw (`updatedAt` as the Reject returned it), and refuses
  when the payload has `closedAt` or, for the one-card-per-subject kinds
  (workflow permissions, failure streaks, the gardener), a newer live card has
  the same kind and `source_path`. A budget pause's Keep paused and an old companion op's
  discard have no Undo (their Reject does more than set the status).

### Key hints (cross-PR review S2, overnight 10-11)

Hints for go-to keys are never written as text: `goHint(id)` in `shortcuts.ts` gives "g d" from the leader key in
effect (`''` when cleared or with shortcuts off), used by the palette rows, Home's Today card and the
welcome tour (whose chord cards read `keyFor`). Bare letter keys inside a view go off with Use keyboard shortcuts, like
the list keys. Opening the palette closes the `?` card (it sits above every view).

### Welcome tour and What's new (overnight 10-11 S1)

Cut overnight 11-08 S4, with Present, the Workspace map, Runbooks and the Tonight queue. `docs/WHATS_NEW.md` stays the user log (every `###` still needs a `COVERAGE` row in `src/paletteCoverage.test.ts`). What an install brings still comes from `build.install_brings` (git history in `src-tauri/src/install_brings.rs`), not from that page. The install notice no longer offers What's new. Personas files stay; pressure-test is something you ask for in chat. Automations stays the list. First-run setup does not arm a tour.

### Today (overnight 10-11 A3)

Cut overnight 11-08 S3. Work is Home and Running (`components/YourWork.tsx`, view state `store/destinations.ts`); Assignments went with the company (11-10). The map was cut overnight 11-08 S4. There is no Today tab, no Day/Week switch and no `g d`. Due and overdue tasks are the Tasks list, including Due today. Home's Due tile opens Tasks. Facts the brief still writes stay in `store/todayBrief.ts`, `todayBrief.ts` and `today_brief.rs`; they have no screen. Design history: [Today](../TODAY.md). The [weekly review](#weekly-review-overnight-10-11-a7) heading remains as its own cut note.

### Push-to-talk (overnight 10-11 B7)

A mic in the chat composer, the Main agent page's too (`components/DictateButton.tsx`, `styles/dictation.css`), and the
`dictate` command (⌘⇧D, Edit menu, palette **Dictate into the message box**). `dictation.rs`: macOS's Speech framework
with `requiresOnDeviceRecognition` on every request (a recogniser without on-device support is refused with a reason,
never a network fallback), fed by an AVAudioEngine input tap; one session at a time on the main thread
(`run_on_main_thread`, a `thread_local`), ids on every `dictation-event` (`partial` = the whole transcript so far,
`final`, `error`, `ended`; a final result or an error also stops the microphone and frees the session on the Rust
side). Microphone access is read and asked through `AVAudioApplication` on macOS 14+ (older systems ask when the engine
starts); speech and microphone permission both gate `ready`. Nothing in the Speech framework is touched unless the bundle declares
`NSSpeechRecognitionUsageDescription` and `NSMicrophoneUsageDescription` (`src-tauri/Info.plist`; macOS kills an app
that asks without them, so a dev binary reports unavailable). `src-tauri/Entitlements.plist` (referenced from
`tauri.conf.json` `bundle.macOS.entitlements`) grants `com.apple.security.device.audio-input`, which the hardened
runtime needs for the microphone. The webview (`store/dictation.ts`, pure `dictation.ts`) writes words into the field
through an anchor (text before/after the start; a selection is replaced) with `execCommand('insertText')` over the
dictated part (⌘Z undoes it; the field's own input event fires); once the person edits around it, it stops writing;
if the box unmounts, dictation stops. A held ⌘⇧D's repeats are ignored (keydown `repeat`, and a press within 300 ms of
the last for the menu's key equivalent); releasing the letter or a modifier after 0.35 s stops (WebKit sends no keyup
for a letter released under ⌘). Esc is a window-capture listener registered at launch, so it runs before the chat's
own Escape and never stops a running turn. Commands `dictation_status`,
`dictation_authorize`, `dictation_start`, `dictation_stop` are user-only. Mock: ready, a sample sentence typed on a
timer (`mockDictationOff` for the unavailable state).

### Triage inbox (overnight 10-11 B6)

Cut overnight 11-08 S3. Needs you is Waiting only (`pages/Review.tsx`): All, Decisions, Results and Failures. There is no Incoming view and no `g i`. Arrival records stay in `triage.ts` and `triage.rs` for agents. `store/triageOpen.ts` no longer opens a screen.

### Weekly review (overnight 10-11 A7)

Cut overnight 11-08 S3. There is no Week range and no `g e`. Week math and the cached take stay in `store/weeklyReview.ts`, `weeklyReview.ts` and `weekly_review.rs` for agents. A Library note template is still named Weekly review. Design history: [Weekly review](../WEEKLY_REVIEW.md).

### Ask (One Ask, 2026-11-03)

One Ask: every question is a chat, asked from Work's **Ask anything** (`components/StudioHome.tsx`; scopes in
`askScopes.ts`, sending in `store/ask.ts`). Where to ask: **Across all my work** (the Main agent on its page) or
**New chat** (a new conversation in Chats; **Add to project** starts it in a project). The command bar's one **Ask: “…”**
row (a sentence, `looksLikeSentence`) asks the same way (`askHere`), its hint naming the place (`askPlaceLabel`: New chat, or
New chat in <project> after Add to project); **Ask anything…** (`act:ask-vault`; a typed sentence fills an empty field), `g v` and
the agent's `navigate` page `ask-vault` open Work with the field focused (`openHomeAsk`, set once the navigation
is accepted). A saved
`hollow.ask.lastScope` of `notes` reads as `quick`, `hollow` as `main`. A chat reply's **More actions → Save as note**
(`store/replyNote.ts`, paths from `turnFooter.ts` `replyNotePath`) writes it into the workspace's `outputs/answers/`,
or `library/` from a chat in Chats or the Main agent, never over a file, with a toast whose Open shows it in the Preview
pane. The Ask your vault view (A6) and the Ask Hollow dialog (A2) were removed; their backends (`vault_answer.rs`,
`ask_hollow.rs`) stay registered, and past Ask Hollow runs stay in History & recovery. Design:
[docs/ASK_YOUR_VAULT.md](../ASK_YOUR_VAULT.md), [docs/ASK_HOLLOW.md](../ASK_HOLLOW.md).

### Commitments ledger (overnight 10-11 A5)

Cut as a screen overnight 11-08 S3. Tasks is a list or a board (`pages/Tasks.tsx`). The ledger stays for agents in `store/commitmentLedger.ts` and `commitmentLedger.ts`. Opening it does not show a view. Design history: [Commitments ledger](../COMMITMENTS_LEDGER.md).

### Canvas gallery (overnight 10-11/12 F3)

Library starts on All (overnight 11-12 L3; it was Results) when there is no
valid saved view. Notes, Results and Recordings are All's views since L4: a
saved one (from before) opens All, opening one saves All, and Back/Forward
keeps the view. Only view picks save that preference; file history and canvas
deep links change the current view without overwriting it. Results is All →
Outputs → **Grid**. New note sits beside All's search and in the page header on
every other view (primary only in Notes); it opens Notes and focuses the name
field.

Without a selected canvas, results form a responsive preview grid. Selecting
a canvas keeps the existing compact list beside its lazy detail. The grid
uses the same host-drawn previews and quota, with no active canvas frames in
cards. Up/Down and j/k follow reading order in either layout and move focus
only. Enter/click opens the result. Back to all results in the selected detail,
or Escape on a result row, returns to the grid and restores row focus while
keeping search and workspace filters. The Canvases-only view retains its
first-canvas auto-selection and does not offer that dismissal. Grid thumbnails
fit their full 9rem tile; loading and empty states span the grid.
Notes and note-management routes explicitly
select Notes before reaching a file.

Library → Results is one **All · Files · Canvases** list (run 15 B6b). The review filter is removed; each row retains its
review evidence and labels evidence for an older listed file revision. Unknown
review, date and revision values stay unknown. Leaving Library resets to All.

`components/LibraryResults.tsx` owns search, workspace/type filters and separate
Clear search / Reset filters actions. File metadata paints independently of
canvas stores. `libraryCanvasListing.ts` coalesces refreshes behind at most six
readers. Refresh results is disabled only during the file listing, so hung
canvas reads cannot block file recovery. The loader
drops changed-scope delivery and stops queued reads on disposal.
Same-scope refreshes queue without starving a completed batch. Each read has
a 15-second timeout; a timed-out native promise keeps its reader slot until it
actually settles and remains deduplicated by workspace; queued reads wait for
a slot under the same deadline. Failed/loading
coverage stays visible. A failed refresh retains that workspace's last-good
canvas summaries with an explicit out-of-date notice, preserving selected
store-only canvases and comparison state. A successful empty read clears that
workspace's retained list; removing a workspace prunes its cached summaries.
Viewing writes nothing; conversation-only canvases
outside workspaces are not listed. Files open in the app-wide Preview pane;
their adjacent folder action ("Open <title> in its workspace") opens its
workspace, then the file in the Preview pane with its review bar
(`openWorkspaceResult`; COO rebuild L4, 2026-11-10). Workspace result links
open the file only after draft navigation accepts; a refused draft choice
leaves the page as it was.
The automation draft browser suite checks real-form refusal and acceptance
for workspace result and canvas destinations as well as store-level coverage.

Palette Open canvases and `g x` remain filtered deep links via `store/libraryMode.ts` and
`store/canvasGallery.ts`. A source link carries a selected canvas, a fresh
Compare request and its Back action. Back preserves the original source route.

Rows are keyed by workspace/file or workspace/canvas, with reactive payloads.
The selected key owns the lazy `GalleryCanvas` in `components/CanvasGallery.tsx`;
metadata refreshes do not remount it. Its local Suspense never hides the list.
It keeps revision menus, Compare's two frames, source diff disclosure, source
chat, Pin, Copy source, Read as text, Attach to issue… and Open workspace.
`canvasCompareMode.ts` consumes each Compare request once across remounts and
handles a fresh repeated request after a local close.
The source disclosure survives revision changes, but resets for a new canvas.

At most 24 thumbnail/detail reads are eligible: 22 thumbnails when the detail
reserves two. `PreviewWhenNear` gates thumbnails near the scroll viewport;
unseen, failed and over-quota previews show type icons. Arrow/j/k keys move
row focus without starting body reads; Enter/click opens the focused result.
The list is not a dialog, has no focus trap, and keys yield under modal UI.

### Library → Skills and History (overnight 11-12 L2)

Library's tabs run All (L3), Connected projects, Conversations, Skills,
History, Learnings (Notes, Results and Recordings open from All since L4,
below): the last three are the redesign's
named tabs. **Skills** is the skills catalog (`SkillsView` from
`pages/Skills.tsx`, lazy) and
**History** is the whole vault's History & recovery (`HistoryRecoveryPanel`
with no note; Library
actions → Show file history in Notes keeps the per-note one). Each sits behind
its own `RecoverableView`. Neither carries its own page header, so Library's
h1 stays the only title. The Settings copies are gone:
these tabs are the one home. `skillOpenRequest` is consumed by whichever
catalog is on screen. Both tabs are remembered in `hollow.library.mode`
(`REMEMBERED_LIBRARY_MODES`, `store/libraryModeState.ts`) and kept by a `vault`
Back step like any tab. The palette's **Library: Skills** and **Library:
History** open them through `openLibraryTab(mode)` (`store/libraryMode.ts`),
the helper any link or deep link into a Library tab should call, and Library's
own tab strip calls it too (#1311 re-review), as do `openLibraryNote` and
`openLibraryCanvases`: it applies a page jump still waiting for its view
transition first, obeys the unsaved-draft guard, clears the page's open file,
file history and New note form (`bindLibraryViewReset`), remembers the tab,
makes leaving Conversations a Back step, and sends `conversations` through its
`chat` page id.

### Library → All (overnight 11-12 L3)

Library's first tab and default view is **All**: one list of what Library's
readers already load, newest first, a view over the vault (nothing moves).
`components/LibraryList.tsx` (lazy, behind `RecoverableView`) renders the pure
`libraryListModel.ts`. Sources, each with its own loading, failure and Retry so one
failing reader never blanks the list: notes from `vault_items` (`vaultItemsData`,
the classified index with dates and kinds), result files and canvases from Results'
reads (`list_all_results`, `canvasStoreListing`, `mixedLibraryResults`), and Plaud's
recordings for 30 days. Every source is read without suspending (`unsuspended`), so a
slow one (Plaud's CLI can take 90 s) shows "Still reading recordings…" under the rows
the others have and never holds the view on Library's "Opening All" fallback. A note
at a result file's path is listed once, as the result; a result the index classifies
as a meeting or person (read from the raw vault items, since results live under
`workspaces/<id>/outputs/`) also shows in Meetings or People.
Left out: workspaces' working files (a project's `PROJECT.md` stays), `sessions/`,
`workflows/`, `me/`, `tasks/`, and hidden mirrors other than meeting notes and kept
transcripts.

Search filters titles, places and paths as you type (every word); **By meaning**
appears when search by meaning is on, as in Notes. The **Library filter** toggle
group: **Recent** (default; the last 30 days, **Show older** lifts it, a search looks
through everything), **Meetings** (recordings plus notes under `calendar/`,
`meetings/`, `recordings/`, `library/records/meetings/`, or classified meeting or
recording), **People** (`people/`, `library/people/`, `library/work/people/`,
`library/records/people/`, or classified person) and **Outputs** (the Results set
plus top-level `outputs/` and `briefings/`). A filter reports only the sources it
reads (Meetings and People include result files, which can be meetings or people). Rows paint 200 at a time (Show more). Empty states: nothing yet, no match
(Clear search), a filter with nothing (says what it shows, Show older or Show
everything). Show older, Show everything and Show more on its last page remove
themselves, so a click hands focus to the first row it brought in (else the first
row, else the search box). Each row: type icon, title, kind and where it lives, relative date. The
list is `data-key-list data-key-default`: j/k and the arrows move, Enter or a click
previews a file in the side panel (`openPreview`), a canvas opens in Results →
Canvases (every workspace, that canvas open), and a recording opens Recordings on
it. A note's **Open** (`e` on the row) is `openLibraryNote(path)` in
`store/libraryMode.ts`: Notes with the note in the reader and editor; a result
file's Open goes to its workspace. Palette: **Library: All**.

**All's views (overnight 11-12 L4).** Notes, Results and Recordings left the
tab strip; the components and every deep link are unchanged, and LibraryMode
keeps `notes`, `results` and `recordings` (`LIBRARY_DETAIL_MODES`,
`isLibraryDetail` in `store/libraryModeState.ts`). Their entry points: beside
All's search, **Browse folders** (Notes: collections, folder picker, Records and
Decisions, saved views, the reader and editor) and **New note** (the form with
templates); with **Outputs** on, **Grid** (Results: Files/Canvases, workspace
filter, thumbnails, review, canvas detail with Compare versions, "x of y"); with
**Meetings** on, **Recordings** (Plaud: range, sign-in row, reader, transcript
search, linking, task proposals). While one is open the strip shows All
selected and **Back to All** (`← All`, `.library-back-all`) sits above it,
returning to the filter it was opened from (`libraryAllFilter`, reset on leaving
Library). Opening one saves `all` in `hollow.library.mode`; a saved `notes`,
`results` or `recordings` opens All. A Notes collection that holds a view's
files links to it (Open Recordings, Open Results). Palette: **Library: Notes**,
**Library: Results**, **Library: Recordings** (`openLibraryTab`).

### Settings pages that moved: Library and Automations

Settings keeps a link row where a control moved, and every old id and label
still lands (`MOVED_LABELS` in `settingsResolve.ts`: a label decides first, so
`resolveSettingsTarget` returns `{kind:'page', page, tab, label}`; Rust and
`SETTINGS_SECTION_ALIASES` keep `skills` and `history` for `settings.get`).
`requestOpenSettingsSection`, `settings:` URLs, Settings' own jump and
`openSkills` end in `openSettingsPageTarget` (`store/settingsPageTarget.ts`);
Settings' own jump and link rows ask about unsaved forms first (`openPageTarget`).
From any other page a `settings:` URL or `ui.navigate` to a moved control opens its page
directly (`openSettingsFromOutside`, also the palette's Settings rows, the action runner's
Connectors and Appearance, How background work runs and Recordings since 11-13), with no
Settings step in Back; `ui.navigate` answers with the tab shown ("Library → History").
A target's control label goes along: the tab opens and scrolls to the element it names
(`landOnPageControl` in `settingsJump.ts`). Search lists one result per page and tab, and
the narrow window's section picker lands on the control its search words matched
(`pickerControl`).
Search lists the link rows by name and keywords ("Skills are in Library",
"History & recovery is in Library", "Automations, windows and the run queue
are on the Automations page", "Waiting for you").

| Control | Was | Now |
|---|---|---|
| Skills (catalog, editor, Import skill, Available to) | Settings, Connectors, part `skills` | Library, Skills; Connectors row "Skills are in Library" |
| History & recovery (restore browser) | Settings, Advanced, This Mac, part `history` | Library, History; This Mac row "History & recovery is in Library" |
| Conversation history button (cut C4) | History & recovery | none; Library, Conversations and the sidebar's Search chats |
| Automation windows editor | Settings, Background work | Automations, fold "Automation windows and the run queue" beside the schedule dial (`pages/workflows/WindowsAndQueue.tsx`); Add automation from a window opens the editor in place |
| Run queue | Settings, Background work | the same Automations fold |
| Profile questions, Ready to add, focus card, Review all | Personalization (`AboutYouPanel`) | Library, Learnings, Pending, as You items (`components/ProfileSuggestions.tsx`); Personalization keeps Look again, Keep learning, the notes and the line "Waiting for you: N profile suggestions, N learnings" |

The profile builder view is one shared plain-signal store
(`store/profileBuilder.ts`) read by both homes, so an Add in Learnings writes
the same `me/` note through the same `profile_builder_accept` call and shows in
Personalization at once. The windows editor's unsaved draft is held by a
navigation guard (leaving Automations asks), not Settings' section-switch
question; the fold stays mounted after its first open so closing it never drops
an edit.

### Library → Conversations (S4, 2026-11-03)

Conversation history is Library's Conversations tab, not a page of its
own (Alex's 10-27 decision: one home). `components/ConversationHistory.tsx`
is the old hidden page's body (pages/Chat.tsx until S4), moved unchanged and loaded with the tab
behind its own `RecoverableView`: the resizable list with its workspace
filter ("Only <workspace>" · Show all), search, the Working folder picker, the
full `ChatBody` composer for vault-level chats, and About this conversation.
Its heading row is an h2 "Conversation history" (Library keeps the page's
h1) with Conversations, Skills and New chat; the open conversation's title is
an h3. Show archived became a two-way **Hollow · Archived** `Segmented`
switch (group "Conversation list") over the same archived filter.

The page id `chat` stays the way in: `currentPage('chat')` is `vault`, and
`applyActivePage` calls `placeLibraryView` (`store/libraryModeState.ts`, a
leaf so runtime.ts need not import the workspace store) to show the tab. So
a sidebar project's or
Chats' ⋯ → Open in Conversation history (it sets `historyWorkspace` first; the
sidebar's own "Show N older" loads older chats in place since overnight 11-04), a Library file's
conversations (`artifactSessions.ts`), navigate(session) for a conversation
in no workspace and `g p` / navigate `history` (answering "Library →
Conversations") all land there; `chat` itself stays refused as a navigate
id. `conversationHistoryShown()` (runtime.ts) replaces every
`activePage() === 'chat'` check, and `shownPage()` names the view `chat`
wherever a page is kept for later: Back/Forward steps, the Main agent orb's
way back (`store/mainAgent.ts`), the palette's `locationStamp`, the
"still here" guards of New project and Ask's project send, and the Preview
pane's Ask to fix. The tab is a step of its own: picking it goes through
`setActivePage('chat')`, picking another tab from it records Library again.
A `vault` step keeps the tab it showed when it was left (`NavEntry.libraryView`,
`keepLibraryView`), so Back and Forward reopen that tab. Any other way into
Library (its rail row, ⌘3, a note or collection link) leaves Conversations
for the view it was opened from, and `hollow.library.mode` never stores it,
so launch uses Results by default or the person's remembered view.

### Session import

H5a (2026-11-18) is the backend of importing Claude Code and Codex
conversations (design `sources-and-import-2026-11-13` section 4). The sheet,
the Imported chip and filter, Library's Import conversations... button and
the palette action are lane IMPORT-UI's and are not built yet.

**Commands** (`commands/session_import_api.rs`, wrappers in `src/api.ts`):

| Command | Map tier | What it does |
| --- | --- | --- |
| `session_import_sources` | plumbing | Claude Code, Codex, ChatGPT, claude.ai and Gemini: the first two with their default folders (`~/.claude/projects`; `$CODEX_HOME/sessions`, else `~/.codex/sessions`) and whether each exists; the export sources have `export: true` and no default folder (the person picks the export). Nothing inside is read. |
| `inspect_session_import` | user-only | Starts reading one folder (the default or one the person picks) on one reader thread and answers at once with the import id. For `source: "chatgpt"` `root` is required: the export's `.zip` or its unzipped folder (see ChatGPT below). |
| `session_import_status` | read action `sessions.session_import_status` (experimental) | One import's progress (`inspecting`, `ready`, `committing`, `committed`, `cancelled`, `failed`), or every staged one's, plus the committed batches. |
| `list_session_import` | plumbing | The staged manifest: per conversation its title, dates, message and turn counts, size, project folder, destination workspace, status (`new`, `alreadyImported`, `updated`, `newVersion`, `hollowOwn`, `subagent`, `unsupported`) and whether it is picked by default. |
| `preview_session_import` | plumbing | The first and last 5 turns of one conversation, texts cut to 2,000 characters. |
| `commit_session_import` | user-only | Imports the picked ids (default: the manifest's selection) with an optional workspace for the whole batch and `keepOutOfSearch`. One transaction per conversation. Answers with counts, failures and the new session keys. |
| `cancel_session_import` | plumbing | Stops a running pass or deletes a staged one. |
| `continue_imported_session` | user-only | Continue in Hollow (N26). See below. |
| `native_resume_offer` | plumbing | H8: `{available, reason, cliVersion, cwd}` for one imported chat: can Continue fork its Claude Code session. Reads only `claude --version`/`--help` text (no model call, cached per binary) and the source file's name and first 64 KB. |
| `continue_imported_session_native` | user-only | H8: Continue by native fork-resume. Answers like `continue_imported_session` plus `resumeSessionId`. Refuses with the reason when the offer is false; the caller then uses `continue_imported_session`. |
| `set_session_import_search` | user-only | N25's Keep out of agent search switch for one import batch. |

**Context scan (CX19).** The review card (CX20, below) starts and reads these.

**ChatGPT export (H5b).** `source: "chatgpt"`, root = the export zip or
extracted folder. `session_import/archive.rs` opens it safely: it refuses the
whole export for an unsafe name (absolute, `..`, backslash, drive letter), a
link or special entry, a duplicate name, an encrypted entry, more than 50,000
entries, an entry over 2 GB or a total over 4 GB expanded, or a compression
ratio over 200; entries are streamed under the same caps (a lying header
can't get past them), never extracted by the archive code and never executed.
A nested archive (an uploaded `.zip` attachment) is listed and never opened;
it doesn't stop the import. Every `conversations.json` / `conversations-NNN.json`
shard is streamed into the import's staging folder as one file per
conversation, so the shared inspect, preview and commit read it like any
other. The transcript is the branch ending at `current_node` (the newest leaf
when it is missing); other branches are counted (`otherBranches` on the
manifest entry and the preview) and not imported. Hidden system messages,
Custom Instructions context and reasoning recaps are skipped; images and
uploads are named; code and tool calls are short excerpts labelled "not
rerun". External id `chatgpt:<conversation id>`; agent `imported`; provenance
label "ChatGPT"; destination Quick chats (no project folder). The manifest's
`export` holds the picked file's name, a stand-in for it (`imports.root`),
shard and damaged-shard counts, skipped elements and the attachment names
(never copied, N28). An export with no conversations file reports
"This export's format isn't supported yet."

**claude.ai export (H5b).** `source: "claudeai"`, root = the export zip or
extracted folder, read through the same `archive.rs` as ChatGPT (same caps,
nothing extracted or executed). Only `conversations.json` is read, streamed
into one staging file per conversation named by the hash of its `uuid`;
`projects.json`, `users.json` and every other file are never opened and are
listed by name in the manifest's `export.otherFiles` (no profile data is
imported). Each conversation's `chat_messages` become turns: `human` prompts
and `assistant` replies from the `content` blocks (else `text`), `thinking`
skipped, `tool_use` and `tool_result` short excerpts labelled "not rerun",
an unknown block type a one-line label, attachments and files by name only
(their `extracted_content` is ignored, N28). External id `claudeai:<uuid>`;
agent `imported`; provenance label "claude.ai"; destination Quick chats.
A message or block of the wrong shape is skipped and counted in `malformed`.
`session_import_sources` lists it with `export: true`.

**Gemini export (H5b).** `source: "gemini"`, root = a Google Takeout zip or
folder (or the Gemini Apps folder itself), the same safe reading as above.
It reads `.../Gemini Apps/MyActivity.json` (or `Bard`): a record whose
`header` is Gemini Apps and whose `title` is "Prompted <text>" is one prompt,
its response the `safeHtmlItem` HTML turned into plain text by a small
scanner (tags, scripts and styles dropped, entities decoded once, never
rendered); attachment names only (N28). Records are sorted by time and a new
conversation starts after more than 30 minutes without a prompt (N30); the
manifest entry and preview carry `grouping: "Grouped by time"`, and the
stored chat opens with a note turn saying so. External id
`gemini:<32 hex of the first record's time and prompt>`, so a re-import of the
same chat dedupes and one that gained prompts updates in place. Destination
Quick chats. Records that aren't objects, lack a usable time or have neither
prompt nor attachment are skipped and counted (`export.malformed`); identical
repeated records count once (`export.duplicates`); other activity is ignored.
`MyActivity.html` alone is "This export's format isn't supported yet." (its
times are locale text).

**Context scan (CX19).** The review card (CX20) reads these; no UI yet.


| Command | Map tier | What it does |
| --- | --- | --- |
| `context_scan_sources` | plumbing | The default folders (Claude Code, Codex, Cursor, Gemini, Windsurf) and whether each exists. Nothing inside is read. |
| `start_context_scan` | user-only | `roots` (`{path, scope: user or project, label}`, absolute) or the defaults; a folder inside the vault is refused. Answers with `scanId`; runs on one reader thread. |
| `context_scan_status` | plumbing | `scanning`, `ready`, `cancelled` or `failed`, and the count found. |
| `list_context_scan` | plumbing | The manifest: per root its counts and budget note; per file `id`, `agent`, `kind`, `scope`, `path`, `relPath`, `size`, `hash`, `excerpt` (absent for secret-like files, with `withheld` saying why), `secretLike`, `flags` (rule, group, severity, line) and `duplicateOf`. |
| `cancel_context_scan` | plumbing | Stops a running scan or deletes a staged one. |

**Found context review card (CX20, 2026-11-19).** `components/ContextReviewCard.tsx`, lazy, at
the top of Library → Learnings (and exported for the first-run sheet, which passes a scan id).
Scan this Mac starts `start_context_scan` with the default roots, polls `context_scan_status`, then
lists. Each item (a scanned file or a queued org fact from `memory_homes::pending_org`) shows its
agent, kind, path, excerpt or why it is withheld, scan flags, a duplicate note, the proposed layer
(project file: Project; the person's own agent folder: Personal; org fact: Org) and its target, with
Accept, Change layer and Skip. No select-all and no bulk accept. Accept is off, with the reason, for a
secret-like or block-flagged file, a file the scan didn't read, a layer whose clone isn't set up
(Settings, Sources, Context), a personal target that already exists and Project on a user file.
Personal copies into the personal clone at `memory/imported/<agent>/<rel>` (`.md` added when
missing; a project's file gets the project's name in between) through
`memory_guard::check_layer_in_root`, create-new, after checking the file's SHA-256 still matches the
scan. Project writes nothing (a suggested pull request in that repo; Hollow opens nothing). Org
confirms with `showConfirm`, then opens a CX8 pull request (`attended: true`): an org fact's own
request, or a scanned file as `ai-tools/imported/<agent>/<rel>`; the fact then leaves the queue.
Skip and Accept are recorded in app data `context-review.json`, never the vault.

**Auto-memory sort (PX6, 2026-11-19).** A Claude Code auto-memory entry
(`~/.claude/projects/<slug>/memory/*.md`) carries `sort` (`layer`, `rule`, `reason`) from
`memory_sort::classify`, run by the scan on the text it already read (pure: keyword and pattern rules,
no model, no network). Stale first (`index` for `MEMORY.md`, `empty`, `superseded`, `dated-old`: newest
date over 60 days and a passing state), then the most Home, Personal or Project signs (ties in the
four-question order), else Personal (`default`). The card shows "Why this layer:" and proposes that
layer. Such an entry also has a **Home** choice: one new file `home/<machine>/<file>.md` in the personal
clone through `check_layer_in_root(Home)`, create-new (off without the clone, `no-machine` without the
machine name, `target-exists`). Stale proposes Skip: its choice is always off (`stale`, the reason as
`detail`); Skip works and another layer can still be picked. Its Project choice is on but writes nothing
(a suggestion for the project's lead, target the slug). A manifest staged before PX6 has no sort; scan again.

| Command | Map tier | What it does |
| --- | --- | --- |
| `context_review_list` | plumbing | `scanId` (else the newest finished scan): the items with `proposedLayer`, `choices` (layer, target, `disabled` code, `detail`, `outward`), `blocked` (`secret`, `block-flag`, `unread`), `skipped`; accepted items left out and counted. Reads only. |
| `context_review_accept` | user-only | One `id` and one `layer` (`personal`, `project`, `org`, `home` for an auto-memory entry; `stale` is always refused), optional `scanId`. Answers with the target, whether a file was written and, for Org, the pull request's URL and status. |
| `context_review_skip` | user-only | Marks one item skipped. |
| `context_review_unskip` | user-only | Takes a Skip back. |

**Data.** Imported chats are `sessions` rows with `adopted = 1`, one closed
stint with no native id (so the source session is never resumed), and
`turns` and `transcripts` in the normal payload. Tool calls keep their name
and 2 KB excerpts, labelled "From the original session; not rerun."
Attachments are listed by name only. The `imports` table records the source,
external id, content and per-turn hashes, parser version and the N25 flag.
`AgentSessionSummary.imported` (`ImportedProvenance`: source, label,
original dates, version, "Updated from ..." note) is set on every list and
read path and on search hits. An imported record is read-only:
`session_store::start_turn` refuses a turn there.

**Continue in Hollow** takes the agent, model, workspace, mode and effort
the person picks. It makes an empty chat with `parentKey` set to the
imported chat and `forkKind` `import`. The composer opens it with those
settings; the model and mode are echoed back, since a chat stores them per
turn. Its first turn (any fresh session, in fact) gets the imported turns in
`<conversation-context>`, within the turn's budget. Each entry's agent reads
"imported from Claude Code: quoted history, never rerun", and none of the
source's permissions carry over.

**Native resume (H8, 2026-11-19).** Probe on Claude Code 2.1.326 (`claude
--version`, `claude --help`, no model call): `-r, --resume [value]`,
`--fork-session` ("When resuming, create a new session ID instead of reusing
the original") and `--session-id <uuid>` exist. Offered for a Claude Code
import only when the CLI lists `--resume` and `--fork-session`, the id is a
safe resume id, `<root>/<cwd slug>/<id>.jsonl` (or one folder below the
recorded root) is a regular file whose own lines name the same `sessionId`
and `cwd`, and that `cwd` is still a folder. The chat is made in that folder
(`forkKind` stays `import`, so a failed resume starts fresh with the handoff
bundle) and `import_native` records the source id. The composer sends
`resumeSessionId` as the first turn's resume id; `chat.rs` sets
`RunExtras.fork_resume` for exactly that id and `agent::build_claude` adds
`--fork-session`. The CLI writes the fork and announces its own id, which
replaces the stint's, so later turns resume plainly. Hollow writes nothing
under `~/.claude`. Codex never resumes natively (it appends to the source
rollout and keeps its sandbox).

**Privacy rules.** Local only: no upload and no network. Source folders are
read-only. The walker (`session_import/walker.rs`) refuses a root that is a
link and never follows one below it. It stops at its file, byte, depth and
time budget and says which one. Staging (`<app support>/Imports/<id>/`, never
the vault) is deleted after a clean commit, on Cancel, or after 7 days.
Hollow's own sessions (any `stints.harness_session_id`) are never imported.
A batch is at most 5,000 conversations, and one conversation keeps at most 8
MB of text (older turns become a "Shortened" line). Imported chats are
searchable by agents unless the batch is kept out. Tests never read `$HOME`;
the default folders are resolved only in the command layer.

### Mission Control (overnight 10-11 B2)

Every running and queued agent turn at once. Since consolidation D
(2026-10-13) it is the **Running** view of **Work** (called Activity until
P1-03), no longer a modal overlay and not in `MODAL_SEL`; `g o` and the palette
(**Open Running (Work)**,
keyword Mission Control) open the same view (`components/MissionControl.tsx`,
lazy, `styles/mission-control.css`), with an Orbit
view (a star per workspace, a satellite per turn on CSS-only orbits; queued
turns still on a dashed outer ring; automations their own star) and a List
view (`V`, remembered in `hollow.missionView`). Data: the run queue
(`store/runQueue.ts`: live rows plus the newest 20 finished, so "Finished
today" is today's among those) and the live feed (`store/missionControl.ts`,
a `chat-event` listener from launch, and since overnight 10-11/12 A7 a
`workflow-event` one whose automation events `workflowFeedEvent` keys by `workflow:<slug>`, bound to the run's queue
row; folded by `applyFeedEvent` in the import-free `missionFeed.ts`, so launch pays only for it: last 3 lines, the
tool in use (a compact copy of its input), progress, and cost + model
from the turn's `usage` row, for at most 24 conversations and automations together). Cards are
reconciled into a store by id (`reconcile`), never rebuilt, and hold no clock
(the view derives elapsed time and the quiet line), so satellites keep
orbiting and focus survives streaming; nothing is computed while closed. A
feed entry is bound to its turn's queue row (`queueId` from `runStarted`).
There is no live cost: a running card says "cost at the end"; an automation
card's cost stays unknown (workflow events carry no usage). Stop mirrors the queue's
`cancelLabel` (Stop / Skip for a scheduled run / Cancel; none for a running
ingest or heartbeat, or once a stop is asked), then `chat_cancel` for a
running chat turn or `cancel_queued_run` otherwise. Note doesn't steer a
running turn (nothing can): for a workspace chat only, it opens the chat with
the note added to its draft.
Spent today is `usage_summary('day')`. Mock: `?mock=1&demo=1&missions=1`
seeds queue rows and a feed (`mockDemo.ts`).

The next three subsections sat under CLAUDE.md's
`## Self-install CI compatibility` (now in [Build &
ship](build-and-ship.md#self-install-ci-compatibility-2026-09-29)).

### Additive canvas instruments (overnight 10-11/12 V1)

The kit remains `kit@1`. `hc-gauge`, `hc-sparkline`, `hc-timeline` and `hc-heatmap` live in
`src/canvas/kit/components/instruments.ts`, with bounded, pure data/geometry in
`src/canvas/kit/instruments.ts`. Calendar arithmetic uses strict ISO dates and UTC days;
missing dates are unknown, duplicate heatmap dates use the last value with a notice.
`src/canvas/kit/chartEntrance.ts` reveals one cached chart-data image using transform-only
SVG clips (trace/sweep) or image scaling (bars). Axes and labels stay in the original canvas.
IntersectionObserver and document visibility pause entrances; removal disconnects observers
and cancels animations. Interaction, either reduced-motion gate, theme change or resize
settles the final drawing. `watchFrameTheme` also sends the live app `motion` preference;
the runtime accepts it only from the parent. API, caps and verification: [CANVAS.md](../CANVAS.md).

**macOS gate temp paths:** script fixtures use Unix sockets and no-extension CommonJS
Node stubs. A deep TMPDIR exceeds the socket pathname limit; repo ESM ancestry can also
break the stubs. Keep frontend script-test TMPDIR in Hollow's short private scratch.
For Cargo tests, create an owned worktree-local `.tv/` (with a CommonJS package.json),
and a short `scratch/t` symlink to it. A scratch PATH wrapper for cargo sets TMPDIR to
that alias before execing the real cargo. Files stay inside the worktree; socket strings
stay short. Do NOT put a blanket gitignore in the temp root: retrieval fixtures inherit
it and their source scans become empty. Source provenance already excludes this temp
path. For a short worktree-local `v/` and a clean S12 gate record, its local `.gitignore`
can use the anchored `/*` (not `*`): nested retrieval fixtures stay visible while
temporary children stay out of git status. Recheck directory_connections tests if
changing this setup. Move only owned worktree temp folders to scratch after the run;
scratch is host-managed.

Grouped flow labels reserve space against earlier labels and group headers. If a label cannot fit, its full
connection remains named and listed in a disclosed expandable list; ungrouped legacy layout is unchanged.

**Canvas checklists (Y4):** `hc-checklist label` renders static `[x]` / `[ ]` rows. Within `hc-ask`,
`name` and optional `required` compose the existing matrix field with Pass/Fail/Skip (authored done marks
do not answer for the person). `src/canvas/kit/checklist.ts` parses rows;
`src/canvas/kit/components/checklist.ts` keeps the matrix across wrapper rerenders. Duplicate labels and
empty lists are disclosed. Matrix rerenders retain scroll-region focus and recover removed-row focus there.
Ask-canvas Send and answer-lock behavior are unchanged by Y4; see `docs/CANVAS.md`.

### COO presence (overnight V3)

The 3.0 Main page adds shared lazy static orbital artwork around that indicator,
with an opaque ruled day column. Header Review and Actions counts link to their
single detail homes. Automations owns the bounded Main agent actions disclosure,
including timestamps, full summaries and retained/error recovery. It mounts
only after a Main workspace exists; opening it adds no read or timer.

The shared `MainAgentIndicator` now draws an SVG presence at all three entry points, with offscreen/hidden animation
suspension and both motion gates. Waiting counts are own conversations; unseen failures retain live-work words.
Night needs a running night item plus a matching live run-queue row, since the night table may lag until the minute
scheduler settles it. No extra poll. Code, state precedence and coverage limits: [COO](../COO.md#coo-presence).

### Mission Control tracks (overnight V4)

The lazy Mission Control now defaults to SVG-backed activity tracks while preserving saved Orbit/List preferences.
Pure `src/missionTracks.ts` groups actual workspaces and distinguishes active shell work from finished turns;
`src/components/MissionTracks.tsx` renders selectable segments and explicit cost coverage. No merge inference or new
polling. State, keyboard, lifecycle and accounting limits: [COO](../COO.md#mission-control-activity-tracks).
The Tracks heading, cost coverage and evidence footnote appear only with turns;
without turns the existing loading, failed or empty state is the explanation.

## Day ring (overnight V5)

Cut as a screen overnight 11-08 S3. The dial component and its stylesheet are gone. Local-day geometry stays in `src/dayRing.ts`. See [Day ring](../DAY_RING.md).

### Settings: one home each

| Setting | Home | Elsewhere |
|---|---|---|
| Reasoning effort, app default | Settings, Models, Effort (`saveAgentEffort`) | none |
| Reasoning effort, one chat | the composer's model menu, Effort (`chatEffort`, `setChatEffortPersistent` in `store/chatRuntime.ts`) | kept in Rust's session store `chat_efforts`, cached by `chatEffortStore.ts` (legacy localStorage choices migrate once), sent as the `effort` argument of `chat_send` (Rust `chat_effort`, queued with the turn, never written to Settings, held sends keep it); Default clears it; a level turns the chat's Ultracode off; a failed menu save restores both prior choices while the same chat remains current |
| Plaud sign-in for Recordings | Settings, Connectors, Recordings row | the agents' Plaud connector shows its state once, on its row in Your connectors |
| COO daily hard stop, overnight cap | Usage, Budgets (`BuiltInCaps`) | Background work and Automations windows keep one link row each ("Set in Usage → Budgets") with no number |
| Heartbeat on/off chip | Background work's section header | the Silent heartbeat subhead has none |
| How many learnings to suggest | Personalization (`LearningSuggestionsSetting`) | old label "Learning suggestions" is `RENAMED_LABELS` in `settingsResolve.ts`, and stays in the control's search keywords |
| Jev relevance model, Jev duplicate check | Advanced, Search & indexes | with no key each says "Needs a Jev key: add in Accounts" (a link to the Jev row) |

Notion's status copies went with Notion sync (11-10). The Service health table
is gone (One line each).

## Surface notes moved from CLAUDE.md (2026-11-13)

Short per-surface contracts that sat at the top of CLAUDE.md until the
2026-11-13 accuracy pass. Each names the doc that holds the detail.

- **Projects: Home and Running.** Home is chat-first
  ([Chat-first Home](../WORKSPACES.md#chat-first-home)). Once opened, Projects
  stays mounted behind the other pages (`App.tsx`), so its conversation keeps
  its scroll, draft and running turn; Home unmounts its chat. Recaps load
  lazily. Running (Projects → Running, the palette's Open Running) owns the
  activity log. Its list accepts j/k as soon as it opens, a failure's Open
  leaves the item unread, and an empty Tracks view stays quiet. Under Glass,
  Running keeps one opaque plane behind its header and notices
  (`styles/mission-control.css`), with flat geometry and unchanged motion
  gates.
- **Needs you and the bell.** The bell owns notification history and its own
  keys; its Needs you → See all opens the full Waiting view (page id `review`,
  ⌘5, no sidebar row).
- **Tasks and Automations.** Page contracts are in
  [Tasks and Automations page density](#tasks-and-automations-page-density-p4).
  An empty list keeps its form, and empty Tasks reuse the shared type. Task
  drafts: [Tasks](../TASKS_WORKSPACE.md); a local tick stays independent of
  the tracker mirror. Automation rows, covers and editor groups use flat
  ruled surfaces. A queue-backed Stop run reads the local morning report
  ([Automation run state](automation.md#automation-run-state-and-last-night)).
- **Library.** Library opens on All; Notes, Results and Recordings open from
  it. Selected canvases keep a compact list; files open in the Preview pane.
  Library Results keeps its mixed file and canvas accessibility description;
  ask option and rank rows stay static even when full motion is allowed. A
  failed Library canvas refresh keeps the last good current-workspace
  summaries and detail with a stale notice. File Refresh stays enabled during
  canvas reads; keep the six-reader cap and deduplication. Coverage, review
  evidence and bounded reads: [Canvas gallery](#canvas-gallery-overnight-10-1112-f3).
  The `import` data kind stays although the Import page went.
- **Settings.** Twelve sections (`SETTINGS_SECTION_IDS`, pinned by
  `src/settingsSections.test.ts`); old section ids open as aliases; tools open
  in sheets; Skills and History are Library tabs. The page header's Where
  things went lists the moves until 2026-11-26 (`WHERE_THINGS_WENT_UNTIL` in
  `src/settings/whereThingsWent.ts`). Detail:
  [Settings sections](#settings-sections-p1-22-d4-s2).
- **COO.** Its top-bar indicator (`MainAgentOrb`) consumes the existing
  presence and coverage snapshots ([COO presence](../COO.md#coo-presence):
  recorded-provider tint, unknown count and one waiting aura). The COO's
  top-bar Actions menu links "Actions it took on its own" to Automations,
  which owns the lazy `MainAgentActions` disclosure and uses existing store
  evidence without a read on mount.
- **Sidebar and search.** Search chats is the top-bar icon and ⌘⇧K. The
  sidebar lists pinned projects, then 8 recent ones (`VISIBLE_PROJECTS` in
  `components/Rail.tsx`); a row opens its lead (L8), and right-click opens its
  menu. The app-wide Preview pane (P1) mounts in `.canvas-split` under the top
  bar, beside the page, only while something is previewed; the top bar's page
  and pane slots stay intact.
- **Palette preview.** PalettePreview is a lazy read-only region with
  latest-request cancellation; keep it outside the listbox and action
  dispatch ([module map](../FRONTEND_MODULES.md)).
- **Skills, meeting notes and icons.** Creating a skill refuses an existing
  name until the person chooses to replace it; meeting-note rewrites and
  custom-icon replacement protect the person's edits ([Skills](agents.md),
  [Meeting notes](../PREP_PACKS.md), [Custom icons](../CUSTOM_ICONS.md) for
  the write and confirm rules). Fold is the stock app and COO artwork;
  Settings → General → App and COO icons offers five app presets
  (Fold, Threshold, Cut Glass, Aurora, Monolith; `src/appIconPresets.ts`) and
  Upload an icon or Generate a logo (H1, H2). The COO's top-bar Actions menu
  → Change icon… opens the same dialog. No shortcuts. Existing custom icons
  and the independent COO choice are preserved.
- **Meeting prep (A8).** New generation requires known attendee coverage (a
  known empty list is valid). Concurrent same-vault, same-event Prepare and
  Write again calls share one app-process outcome; errors can retry,
  cancellation releases, and kept packs keep their actual path
  ([PREP_PACKS.md](../PREP_PACKS.md)).
- **Canvases.** Pinned canvas preferences, exact-revision and source rules,
  one-way live scalar bindings and visible-only refreshing are in
  [Pinned live canvases](../CANVAS.md#pinned-live-canvases-overnight-2026-10-1213-f1);
  the host renderer and readers load lazily only when there are pins, and no
  model is called. Expanded canvases keep the selector fixed and a separate
  scroll viewport with a Back to canvas chip; `src/components/ExpandedCanvas.tsx`
  lazily reuses the focus controller without changing transcript focus.
- **Changes, quit and installs.** Changes rechecks the authoritative busy
  status before it confirms a commit, publish or install
  ([Changes freshness](build-and-ship.md#changes-freshness)). An ordinary
  quit confirms before stopping running work; an install (a channel update
  or a Local build from Changes) waits in the top bar's Update button, with
  Cancel there ([Quit and installs](build-and-ship.md#quit-and-installs)).
- **View recovery.** Page and section error owners stay local, and retained
  History stays scoped to its note
  ([View recovery](frontend-state.md#view-recovery)).
