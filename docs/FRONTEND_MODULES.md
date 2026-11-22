# Frontend module map

`components/ImageZoom.tsx` lazily opens already-loaded data images in a native
modal dialog with Fit, actual size, zoom, Close and the existing Preview route.
`styles/image-zoom.css` owns the static dialog geometry. No remote image loads
or global shortcuts are added.
`replyRichText.ts` parses reply-only bounded native MathML and GitHub alerts, and validates exact local conversation links. `replyRichText.test.ts` covers math, code, alerts and route refusal; `styles/reply-rich-text.css` owns their presentation.
`components/ThinkingRow.tsx` renders provider-reported reasoning in a closed native disclosure. `styles/thinking-row.css` keeps its text selectable and bounded; capture, identity and durable storage live in the chat adapter/runtime modules.

`composerField.ts` defines source-offset fields and registers rich message
editors for dictation. `editor/MarkdownEditor.tsx` has a lazy composer mode;
ChatBody still owns draft text, send keys, mentions, paste and prompt history.
`editor/composerDocument.ts` synchronizes controlled drafts without user-input
callbacks or undo entries and resets the editor state on chat identity changes.

`transcriptWarning.ts` groups exact host warning repeats within one user turn; standalone host errors disclose without rewriting records. `transcriptWarning.test.ts` covers grouping and disclosure boundaries.

`composerMentions.ts` owns caret-local @/$ ranges, safe Markdown file references
and their displayed chips. ChatBody reads project files and the Library skill
catalog only while a trigger is active; ComposerCommands exposes click routes.

Conversation organization: `conversationOrganization.ts` owns precedence and
local-time parsing; `store/conversationOrganization.ts` holds shared keyed state;
`conversationOrganizationLoad.ts` reads and accepts revision-checked writes;
`conversationOrganizationActions.ts` owns lazy menu mutations and Undo.
`components/RailOrganizedChats.tsx` renders counted groups and owns the clock.
`mockConversationOrganization.ts` models persistence and conflict refusal.
The corresponding grouping and parsing cases are in `conversationOrganization.test.ts`.

`canvas/kit/material.ts` owns one static Bayer tile per frame, shared CSS/SVG
material and SVG screen-scale correction. No chart allocates a tile per element. `canvas/kit/ditherFill.ts` paints bar, area,
line and donut series as opacity-only ordered dither cells on a 2px lattice (no pattern,
no tile): `ditherFill` buckets cells by alpha into a few `Path2D` fills clipped
to the exact series path, `ditherRing` does the same for a donut sector, and
`lineTop` gives an area's value line per column. `hc-chart` keeps the dithered data
layer in one offscreen canvas (`_layer`, keyed by size, dpr, colours, Quiet, texture
and the data), so a hover repaints axes, one `drawImage` and the active mark only.

`store/settingsPageTarget.ts` opens the page a Settings target moved to (Settings PR 4: a Library tab, or Automations' windows and run queue fold);
`store/profileBuilder.ts` is the profile builder's one shared view for Personalization and Library → Learnings.

`projectCreation.ts` validates project names; `projectDestination.ts`
filters eligible projects and ranks recorded turn activity. The shared
`components/ProjectDestination.tsx` dialog confirms Home or existing-chat
destinations; `store/overlays.ts` holds its explicit request owner.

`components/MainAgentActions.tsx` is the single action-detail disclosure on
Automations. It consumes the existing Main store, retains timestamps and full
summaries, and adds no reads on mount. `store/automationsViews.ts` owns its
guarded opening; the Main page header's More links to it.

`canvas/kit/textureElement.ts` shares presentation-only policy attributes for
bars, gauges, heatmaps, progress and flows. Updates preserve rendered controls
and table state; flow teardown also releases its presentation subscription.

`texture.ts` shares live policy/ancestor
attribute subscriptions across connected graphics and releases them at zero.

`canvas/kit/texture.ts` validates named static variants and resolves the nearest
request only when the sandbox runtime's host permission and frame media policy
allow it. `canvas/fence.ts` carries root texture into display/export bodies;
legacy source and persistent state are unchanged; rendering now defaults to dither.

`pages/workflows/runDurations.ts` projects only loaded run durations into a
bounded, gap-preserving sparkline. Workflows keeps its exact reading list;
the helper adds no reads, timers or scheduler state.

`components/PalettePreview.tsx` lazily reads a highlighted note/workspace brief
or shows loaded task text. `palettePreview.ts` bounds excerpts, rejects retired
message paths and owns latest-request cancellation. It never dispatches actions.

`components/ExperimentalVisuals.tsx` lazily owns the static-texture and ambient
policies and their event cleanup; there is no pointer layer or frame loop.
`store/studio.ts` persists independent texture levels and the existing ambient
consent key; Settings General's local lazy preview uses the same policy.
`thresholdField.ts` caches four bounded deterministic folded/orbital glyph fields.
`components/ThresholdArt.tsx` renders rows in one decorative SVG and supplies
a plain doorway fallback; DoorwayLandscape lazy-loads it for Home/empty chat.

`pages/Work.tsx` remains the eager workspace host; its Home/destination view
`components/YourWork.tsx` loads lazily outside Suspense.

`components/ModelMenuHealth.tsx` is the lazy per-agent catalog warning, Refresh
and update status under the model listbox. `store/agentHealth.ts` shares guarded
Agent updates navigation, unavailable notices and installed-status invalidation
of future-turn steering support. Running-turn snapshots remain independent.

ExecutionReceipt lazy-loads `components/RunOutcome.tsx` for its common head
and `components/ReceiptCheckActions.tsx` for exact command copying/draft-only
requests. `components/ReceiptCites.tsx` keeps per-cite recorded file navigation;
`components/PreviewSourceLines.tsx` draws and scrolls one source range after
ChatPreview verifies loaded path and nonce. See docs/reference/runs.md.

`store/noticeTimers.ts` owns windowAway and shared countdowns; runtime re-exports
its focus state. `store/notificationToasts.ts` owns the five toast slots and
their interaction holds. `store/appMessages.ts` retains text-only session history;
`components/AppMessages.tsx` renders it inside the bell's NotificationHistory
(`components/RecentActivity.tsx`) with Copy. That history owns saved filters,
repeat folding, full Details and scoped list keys; stable ids retain disclosures.
`components/NotificationBell.tsx` owns the nonmodal shell and focus return;
lazy `components/NotificationCenter.tsx` reads the same notification resource,
offers listed-id actions and App messages, and links to the full Needs you view.
SnackbarStack renders sticky install receipts alongside transient notices.

`components/RailMinis.tsx` owns the collapsed sidebar's pinned-project
shortcuts (each opens its lead, L8) and live badges. Rail loads it only while collapsed, with local loading/error recovery.

`components/RailQuickChats.tsx` (2026-11-14) is the sidebar's Quick chats section under the
Projects list: the Quick chats workspace's chats, newest first (8, the open one past them), each
opening its chat with the conversation menu on right-click, and a + that starts a quick chat.
Rail loads it lazily while the sidebar is shown.

`components/QuickChatMoveOffer.tsx` (lazy, from ChatBody) offers "Move to <project>" under a quick
chat's latest reply when `quickChatMove.ts` finds the exchange clearly one project's work.

M5 chat moves ([peers](reference/peers.md), "Moving a chat"): `components/ChatMoveNotice.tsx` (lazy, from ChatBody),
`store/chatMoves.ts` and `store/chatOffline.ts` (lazy; Continue here offline), the pure words in
`store/chatMoveWords.ts` and `store/chatOfflineWords.ts`, and the signals ChatBody reads in `store/moveFlags.ts` and
`store/offlineFlags.ts`.

New workspace creation lives in lazy `components/NewWorkspaceForm.tsx`.
`pages/Work.tsx` mounts it after the first form request and keeps that owner
alive until leaving Work, so hiding the form does not cancel its first queued
goal. Creation receipts survive remounts; cancellation is request-identity scoped.
The form is a portaled, focus-contained dialog with inert background and bounded
scrolling fields. Optional templates/location are disclosed. Successful creation
opens `store/workspaces.ts`'s provider-neutral `openProjectAgent`; the rail and
workspace menu expose that same persistent conversation. Main/task/fresh drafts
stay separate. A recovered receipt never reopens the dialog without a request.

Canvas source navigation is shared by Results and gallery through
`store/canvasSource.ts`: resolve the displayed revision's recorded session,
then open it through the guarded workspace/session route.

`components/StudioHome.tsx` shows Ask and five recent projects, expanding into
origin-aware search and recovery. HomeReturnRow and GlanceStrip, never
mounted by chat-first Home, were deleted on 11-10.
See WORKSPACES.md.

`components/RecentlyDeleted.tsx` (P1-41, lazy) is All projects' Recently deleted:
`trashed_workspaces` re-read into a signal whenever the workspace list changes,
each row with Restore (`store/workspaces.ts` `restoreWorkspace`, also Delete's
Undo). Lifecycle failures go through `lifecycleFailed` (banner plus toast).

`settings/CalendarSource.tsx` owns the Agenda source picker, Mac calendar
selection and permission recovery, inside Settings → Connectors.

`components/RecoverableView.tsx` owns named page/section recovery and copy/retry (Copy details copies `errorDetails`: message, cause, stack);
`components/ResourceReadError.tsx` keeps a failed read's Retry beside its data.

`components/RunsOnControl.tsx` is a workspace's Runs on control (Default,
This Mac, Remote runner); the line under it comes from the pure, unit-tested
`components/runsOnHint.ts`. Once a runner is set up it also shows
`components/RunnerAccessControl.tsx` (remote runners, 2026-11-03: On the
runner, GitHub and Google Cloud with its access level, `runner_access_set`;
its line from `components/runnerAccessHint.ts`), and Settings → Devices shows
`settings/RunnerGithubRow.tsx` (the runner turns' GitHub credential: a runner
token saved in the Keychain, or this Mac's gh login). Settings → Devices → Remote
runner (in `AgentDefaults.tsx`'s `RunnerSettings` since S5b) is `settings/RemoteRunnerRow.tsx` (remote runners 2: Off until set up;
Set up checks an SSH host read-only and saves it, `runner_check_host` and
`runner_set`; Turn off; naming a runner ends on What runs there?, its
`WhatRunsThere`, four unticked choices saved through `runnerSetup.ts`'s
`whatRunsTherePatch`, M7), holding `settings/RunnerStatusBlock.tsx` (its state
and owner, Record me as owner, `runner_record_owner`; the owner line from
`runnerStatus.ts`'s `ownerSentence`). Settings → Devices → COO on this Mac, just under the Macs panel, is
`settings/CooDispatcherRow.tsx` (M3a PR2: `agents.cooDispatcher`, Dispatcher when a runner COO is paired
or Always the full COO, saved at once, shown once a paired Mac runs a runner COO or while saved off; its Now line from `main_agent_status`'s `dispatcher`).
Settings → Sources → Hollow folder → Move Hollow home is `settings/MoveHomePanel.tsx`
(2026-11-12, lazy: Check the move, Also move the folders beside the vault, Quit and move… with a
second confirm in the panel, the progress view polled while a move is staged or running, and Roll back…;
words from `moveHome.ts`, requests from `moveHomeApi.ts`).
Settings → Sources → Context is `settings/ContextLayersPanel.tsx` (CX17, a closed lazy group, mounted when its fold first opens or a jump wants it: Home,
Personal, Organization, Project folders, Context check (CX10) and Vault sync rows; words from `contextLayers.ts`, requests from
`contextLayersApi.ts`). The Personal row's Set it up for me opens `settings/ContextSetupCard.tsx` (CX18, a card only: the repo
name, Private, the `gh repo create` line with Copy, the address to connect; Hollow creates nothing and writes nothing).
Settings → Sources → Where your work lives (a closed group since browser fix 2, 11-19, with Run setup again as its last row) is `settings/SourcesPanel.tsx` (H4b, lazy: the three role rows, switches with their dry run and Roll back, knowledge references, workspace overrides; words from `sourcesSettings.ts`, requests from `sourcesApi.ts`); `components/SourceLabelChip.tsx` draws a `SourceLabel` on the Tasks page and in Settings.
Settings → Connections → Task tracker is `settings/TaskTrackerPanel.tsx`

Settings → Sources → Task tracker is `settings/TaskTrackerPanel.tsx`
(overhaul A14: provider, the one allowed team, the key's state with Save key,
Use the key saved in Keychain and Remove key, last and next sync, requests left,
edits waiting, the last error, the interval and Sync now; words from
`trackerSettings.ts`). The capability doctor's controller reads `tracker_status`
so the doctor has the tracker in every section.
Tasks is `pages/Tasks.tsx` (overhaul B1: the tracker's issues grouped by
status plus a Local tasks group, the pane, New issue, the page keys) with
`pages/tasks/IssueList.tsx` (the grouped rows, drawn 60 per group at a time, `rowWindow.ts`), `pages/tasks/IssueDetail.tsx`
(one issue: every field, edits, conflicts, sub-issue and relation rows,
activity, Agent activity, Archive; beside the list or as the full page),
`pages/tasks/issueActivity.ts` (the activity feed: the tracker's change history
from `tracker_issue_history` merged with comments, or the synthetic feed when
it can't be read) and `pages/tasks/issueIcons.tsx` (status by kind, priority
bars, the avatar);
a local task still opens `pages/tasks/TaskDetail.tsx`. `pages/tasks/ViewBar.tsx`
(B2) holds the view tabs, the filter chips with Filter and Clear filters, and
Display's Group and Order; the page keeps the choice per window.
`canvas/canvasModel.ts` is a canvas as a plain model (`canvasModel`, `mermaid`,
`codeLanguage`; moved out of the removed `canvas/toNotion.ts`, 2026-11-10), which
Read as text (`canvas/canvasText.ts`) and the canvas flow checks read.
`store/lastCanvas.ts` holds the canvas last used (the palette's Export canvas…
target; split from the removed `store/canvasNotion.ts`).
`store/canvasAttach.ts` holds which saved canvas "Attach to issue…" (A19)
is for; the shell mounts the lazy `components/AttachCanvasDialog.tsx` (find
an issue in the mirror, confirm on a card, `attach_canvas_to_issue`) from it.
`components/ResultCanvas.tsx` puts the item in a saved canvas's ⋯ menu. The
project detail's Write status now (A18) calls `write_friday_status`.

`pages/tasks/ProjectDetail.tsx` (B3) is one project: the tracker's fields, the
portfolio fields with Edit, Track in Linear and Open the project chat, shared
by the Tasks pane (an issue's Open project) and Library's Records (a
PROJECT.md opens as it). `pages/tasks/ProjectList.tsx` (B4) is the Projects
tab: one row per team project with status, lead, target, progress and issues
(`orderProjects`: active first); the page shows a project's issues inside its
detail with the same `IssueList`.

Remote runner's Set up panel also holds `settings/RunnerSetupWizard.tsx`
(remote runners 3, 2026-11-03; Set up again once a runner is set): a runner
from scratch on a Mac you own or a Google Cloud machine. Fields, then every
step with its state, its exact check, apply and by-hand commands (Copy each),
Check (`runner_setup_check`, read-only), a step's Apply (`runner_setup_apply`,
after `showConfirm` when the step asks first) and for Google Cloud the
monthly estimate and Create (`runner_cloud_estimate`, then
`runner_cloud_create` with that estimate). Its words and wire types are
`runnerSetup.ts`.

Subagent group membership, recorded cost totals and honest outcome labels are
pure helpers in `subagentsModel.ts`; `SubagentsPanel.tsx` adds work cards and
native group disclosures. The backend activity contract is in
[COO: Subagent work view](COO.md#subagent-work-view).

`components/PreviewPane.tsx` (run 15 P1) is the app's one right-hand pane, a
lazy chunk the shell (`App.tsx`) mounts beside every page only while
`store/preview.ts` has a target: back/forward, close (× / Esc / ⌘\), its
remembered width (`sidePaneWidth`), and the body by kind: `ChatPreview.tsx`
for a file (rendering by type through `previewKinds.ts`), `BrowserPane.tsx`
for a web page, `PreviewVisual.tsx` for a canvas's Expand, and the lazy
`SubagentsPanel.tsx` for a chat's Subagents tab (`{ kind: 'subagents',
parent, open? }`, one tab per chat; COO rebuild L3), and the lazy
`PanelNewTab.tsx` for a New tab (`{ kind: 'newtab', id, chat? }`, the +
after the last tab: Tools Files, Browser, Subagents, Changes; a pick takes
the tab's place through `replacePreviewDocument`; overnight 11-12 C5).
`ChatBody` mounts lazy `SidePanelCard.tsx` while the pane is closed:
Outputs, live Subagents, Sources and Recall. No independent dismissal state.
`togglePreview` opens Tools when there is no remembered target. Chat tools
live in `ConversationTitle`'s menu, portalled into the top bar for workspaces.
Styles: `styles/preview-pane.css`, `styles/side-panel.css`. It replaced `SidePane.tsx`, `SideVisual.tsx`,
`ConversationLogs.tsx` and `PanelShelf.tsx` (all removed). Its state and API
live in `store/preview.ts`: `openPreview(target)` (a file path or image data
URL, an http(s) address, a conversation visual, a chat's subagents or a build
workspace's Changes), `closePreview`,
`togglePreview` (⌘\), back/forward history (30 items), the web mode's
load requests (`takePreviewWebLoad`), `composerPage` and
`dropConversationPreviews`. It imports nothing from the rest of the store, so
any caller can use it without a cycle. Unit-tested.
The mixed strip (chat-first port) keeps 12 app-wide documents beside eight web tabs;
`PdfPreview.tsx` lazily displays native page pixels/text in Preview and Results,
with bounded session/path page, zoom and scroll state. Refresh retains the page.
`OfficePreview.tsx` (B1b, ENG-104) lazily shows a `.docx`, `.xlsx` or `.pptx`
file's text from `office_preview` (paragraphs, sheet tables, slides) under a
text-only header with Open in app, and is ChatPreview's binary fallback too.
It calls `invoke` from `api.ts` directly, so the App chunk gains no method.
An Excel file renders through `SheetGrid.tsx` (UN1, its own lazy chunk, no
spreadsheet library): sheet tabs, a formula bar for the selected cell (formulas
come from `office_preview`'s per-row `formulas`), arrow-key selection, numbers
right-aligned; pure helpers in `sheetCells.ts`. Read-only.
COO rebuild L4 (2026-11-10) removed the workspace panel and its sections:
a project's notes open as files (the top bar's Project files; `ChatPreview.tsx`'s
Files picker pins WORKSPACE.md, PLAN.md, SOURCES.md and AGENTS.md, then a build
workspace's changed files, then the conversation's), PLAN.md's boxes tick in the
Markdown view (`NoteRenderer`'s `tasks`), a Result opens with lazy
`components/ResultReviewBar.tsx`, a build workspace's Changes opens as a lazy
`components/WorkspacePreview.tsx` tab, and decisions are
in Library, Records (lazy `components/RecordDecisions.tsx`).
`components/ResultThumb.tsx` shares bounded raster/poster previews and host-only
canvas SVG sketches across Library Results and Home; its styles live in `styles/result-thumb.css`.

One line per top-level module in `src/` (the frontend's own files; components, pages, stores and settings panels live
in their folders). Each line comes from the module's own header comment, which stays the authority; a module with no
header shows what it exports. `scripts/check-module-map.mjs` (in `npm run test:scripts`) fails when a module has no line
here or a line names a module that is gone, so add or remove the line in the same change (overnight E5). The backend's
map is in [the backend module map](reference/backend-module-map.md#backend-module-map).

| Module | What it is |
|---|---|
| `historySwipe.ts` | Two-finger trackpad swipe for Back and Forward (t3code parity 4.6): a pure detector (one step per gesture, horizontal must beat vertical, a sideways-scrolling box keeps its swipe) and `createHistoryWheel(deps)`, which App.tsx mounts as a passive wheel listener; Settings, Keyboard shortcuts has its switch (`store/prefs.ts` `swipeNav`). |

| `fontChoice.ts` | Font choice (t3code parity 4.7): Hollow's default, a system stack or a typed family for the interface and code fonts; pure, `store/fonts.ts` applies `--ui` and `--mono`. A typed name is cleaned to letters, digits, spaces, dots and hyphens. |

| `planProposal.ts` | Latest completed live proposal eligibility and explicit accept/refine follow-up requests. No send or permission mutation. |
| `replyRichText.ts` | Bounded native MathML, GitHub alerts and strict local conversation routes for lazy replies. |

| `composerField.ts` | Source-offset composer field contract and rich-editor dictation registration. |
| `composerActions.ts` | Accepted-send background navigation with origin guards. |

| `transcriptWarning.ts` | Exact-repeat host warning grouping and standalone host error disclosures within one user turn. |
| `composerMentions.ts` | Caret-local file and skill triggers, Markdown file references and chip ranges. |
| `projectCreation.ts` | Bounded one-line project-name validation and duplicate checks. |
| `projectDestination.ts` | Eligible project choices and activity-based defaults, excluding the source project. |
| `sheetCells.ts` | Pure helpers for the Preview pane's live sheet: numeric-cell test and formula-bar text (UN1). |
| `vscodeThemeImport.ts` | Import a VS Code colour theme (t3code parity 4.7): JSONC in, Hollow's Kitty-style theme text out (accent, background, foreground, selection, the 16 terminal colours, success, warning, error); Edit themes → Import VS Code theme… opens it in the editor for review, nothing saved until Save. |
| `windowLayout.ts` | Live scaled root-container tiers for navigation and the narrow-window Preview pane policy; no store imports. |
| `taskDrafts.ts` | Canonical session draft envelopes, independent local/mapped baselines and three-way field rebase. |
| `eventTriggersApi.ts` | Event triggers' IPC (COO rebuild F2; `commands/event_triggers_api.rs`): status and set, the event token helpers and `turnOnFor`, which turns an event automation's trigger on after its save. Imported by the lazy Automations page and, on first use, by the project menu's Wake the lead for new issues. |
| `trackerApi.ts` | The task tracker's typed IPC wrappers (overhaul A2; `commands/tracker_api.rs`, [Tracker sync](TRACKER_SYNC.md)): status, view, issue, team metadata, the writes, the key and settings. Imported lazily by Settings' Task tracker and the Tasks page; the key goes in and never comes back. |
| `sourcesApi.ts` | Settings, Sources' typed IPC wrappers (H4a; `commands/sources_api.rs`, [Sources of truth](TRACKER_SYNC.md#sources-of-truth-h4a)): status, Save, owner switch plan, apply, rollback and status. Imported by the lazy `settings/SourcesPanel.tsx` and the Tasks page. |
| `sourcesSettings.ts` | Settings, Sources' words (H4b, T5): role titles, the tasks row's two choices (Vault only, Vault plus Linear), `Inherits:`/`Override:` lines, edits waiting, why a switch is blocked, a dry run as plain lines, label tones, the knowledge list edits. Pure, unit-tested. |
| `linearLink.ts` | Opens a queued `hollow://` link from Linear's Custom link (`linear_link.rs`, [Tracker sync](TRACKER_SYNC.md#linear-link)): the issue's project-lead chat, else the COO, with the draft in the composer; never sends. Loaded lazily by `appBridge.ts` on the `linear-link-pending` nudge. |
| `trackerSettings.ts` | Settings, Sources, Task tracker's words and choices (overhaul A14): the one allowed team per provider (`ALLOWED_TEAMS`, Engineering only), the headline, the row's dot, sync times both ways, requests left, Sync now's readiness and the 1 to 60 minute interval. Also feeds Service health's tracker row and the capability doctor's tracker check. |
| `projectPortfolio.ts` | A project's record (overhaul B3): the portfolio fields in display order and kind, the editor's draft (`draftOf`), what changed (`changedValues`), number and address checks (`draftProblems`), vault-path list items (`isVaultPath`, `itemLabel`) and issue counts with progress, completed over all (`projectCounts`). |
| `projectFiles.ts` | A project's files in the Preview pane (COO rebuild L4): the pinned notes (`PINNED_NOTES`, `pinnedNotes`), vault path joins (`inVault`, `vaultRelative`), which file is a Result (`resultFileOf`) or a workspace's PLAN.md (`planFileOf`), and the Files picker's groups (`filesPickerOptions`). Pure. |
| `resultReviewBar.ts` | A Result's review bar in the Files tab (COO rebuild L4; it was ResultsView.tsx): which controls a review state offers (`reviewBarModel`) and the toast (`reviewedMessage`). Accepting stays the person's. |
| `retainedPage.ts` | `retainedOffscreen`: whether an element of the retained Work page is out of sight (hidden or inert), so a hidden chat takes no focus or keys. It was part of workPanel.ts. |
| `trackerViews.ts` | The Tasks page's views (overhaul B2): the view presets (the tabs in `TAB_VIEWS`, plus Conflicts: `conflictCount`, `showConflicts`, T2), the seven filters (`applyView`, `dueMatches`), grouping by status, assignee, project or priority (`groupRows`), the three orders (`comparator`), the filter options and chip words, and the per-window save (`readSavedView`/`writeSavedView`: sessionStorage, then the last choice in localStorage). Replaced `store/tasksLayout.ts` and the saved-filter store. |
| `trackerTasks.ts` | The Tasks page's words and edits (overhaul B1): groups by status in the tracker's kind order, row order (priority, then latest update), search, initials, due words, the one-field compare-and-swap edit (`fieldEdit`), why a field is locked (`fieldLock`), key moves (`step`) and the Agent activity rows lane 4 adds (`agentRunsOf`). |
| `trackerTeams.ts` | The Tasks page's two-team helpers (2026-11-12): whether two teams exist, the team filter and chips, the create picker's choices (AI, HOL only). Pure; `pages/tasks/TeamChoice.tsx` renders them. |
| `appIconPresets.ts` | App preset names, default selection and legacy Custom labels. |
| `customIcons.ts` | Upload an icon (H1): PNG, JPEG or SVG checks by name, size and bytes, image decoding, the fitted 512 px render (at prepare's size) and the dialog's stale-result tickets. |
| `logoGenerate.ts` | Generate a logo (H2): which installed agent draws (Codex with room, else Claude, else any), the request and its cancel id. |
| `mockCustomIcons.ts` | Browser-only custom icon fixtures and Generate a logo's stub agent; native validation stays in Rust. |
| `accountSwitchNotice.ts` | R1's chat notice when a turn moves to another Claude account ("Switched to Tech (Admin hit its limit)"); its own module so `claudeAccounts.ts` stays in the lazy Settings chunk (P1-42). Also the notice and the message of a continuation on another agent (`agentContinuedNotice`, `isAgentContinuedPrompt`, which Retry and Edit skip, C3). |
| `routeReceipt.ts` | A dispatcher COO's `sessions.route_to_runner` call in a transcript (M3a follow-up, 2026-11-15): `isRouteCall` and `routeReceipt` (the Mac and chat its answer names, never that chat's words; the broker's note after it is skipped), `routeChipLabel`. `components/ToolCall.tsx` puts the lazy `components/RouteChip.tsx` in a route row's head (none of this module rides ToolCall's chunk), which opens that chat on that Mac (`store/peerChat.openRemoteChat`). Pure, unit-tested. |
| `remoteRunNotice.ts` | The Mac mini runner's chat lines (2026-11-01): a turn running on the runner, and how its work came back (`remoteRun` events; a workspace folder's, scope `folder`, 2026-11-02, and what of it stayed on this Mac, `leftHere`, or didn't come back, `notBrought`, overnight R3; docs/REMOTE_SESSIONS.md). `chatRuntime.ts` loads it on first use, outside the shell chunk (overnight 11-07, lane D3). |
| `runnerStatus.ts` | The Mac mini runner's state in words (overnight N1, 2026-11-02): the state label, the preflight line's facts, the last gate's sentence and when a check is old enough to repeat, for `settings/RunnerStatusBlock.tsx` under Settings → Runs on (`runner_status`). Smart routing 3: `capsSentence` (the runner's tools from its last report) and `routeWhere` (a decision's side) for Builds and tests. |
| `runnerSetup.ts` | Set up a runner's words and wire types (remote runners 3, 2026-11-03): step state labels and tones, who runs a command, the Google Cloud estimate's lines and the Apply and Create confirm text, for `settings/RunnerSetupWizard.tsx` (`runner_setup_*`, `runner_cloud_*`). Pure, unit-tested. |
| `accountSwitchRows.ts` | A reply's account moves (receipt `switches`: R1 limit, #420 ceiling, agent move, #421 login stay) as chat timeline rows, less those its live notices still show (overnight D4). |
| `App.tsx` | The app shell: navigation, pages, global dialogs and the window-level listeners. |
| `backgroundWork.ts` | What runs without being asked (plan completion L2): scheduled automations, the silent heartbeat, COO check-ins and the Settings automation control. The Tonight row was cut overnight 11-08 S4. Rendered by `components/BackgroundWorkExplainer.tsx`. |
| `backgroundClock.ts` | Pure explicit-zone civil-day window bands, midnight tails, unknown coverage and SVG arcs. BackgroundClock renders within the lazy Automations page. |
| `MdPreview.tsx` | The Markdown preview window. |
| `Onboarding.tsx` | The setup window shown when the config is missing or unreadable (setup mode). |
| `sidePanelModel.ts` | The side panel card's and New tab's words (overnight 11-12 C4, C5; the header's Open in rows `openInRows`, machine name `cardMachineName`, parity 5.1): one glyph per subagent up to four (`cardGlyphs`), "4 done" / "2 running, 5 done" / "None yet" (`cardSubagentWords`), the newest three outputs and sources (`cardOutputs`, `cardSources`), View all's target (SOURCES.md, else WORKSPACE.md), a file row's name, folder hint for same-name files (`cardFileRows`), type icon and full-path label (`cardFileRow`, C8) and a New tab's Tools with why one can't open (`panelTools`). Pure, unit-tested; `components/SidePanelCard.tsx` and `components/PanelNewTab.tsx` draw them. |

| `cardAutomationsModel.ts` | The card's Servers and Automations sections (parity 5.5): which automations run in this chat (`chatAutomations`, by the `inChat` key), the row and count words (`automationWords`, `automationsCount`, `serversCount`) and the Add an automation draft. `CardAutomations.tsx` and `ServicesPanel.tsx` (now also taking a folder and a compact mode) render them. |

| `githubModel.ts` | The Pull requests page's pure words and filters (parity 7.3): list filtering, state and checks wording, Linear key links, the quoted draft for Ask about this. `components/GithubPages.tsx` is the page, lazy from `YourWork.tsx`. |

| `terminalScreen.ts` | The terminal drawer's screen (parity 7.2): `TermScreen`, a dependency-free VT emulator (printing and wrap, cursor movement, erase, insert and delete, scroll regions, the alternate screen, SGR in 16, 256 and true colour, line drawing, the window title, answers to cursor-position and device-attributes requests). It ignores OSC 52, OSC 8 and mouse reporting and escapes everything it renders (`rowToHtml`). Scrollback is chunked HTML blocks, capped at 2000 rows. |
| `terminalKeys.ts` | Key presses to shell bytes (`keyToBytes`: arrows in application mode, Ctrl letters, Option as Meta, function keys; null for any Cmd chord and for Shift+Tab, which lets focus out) and `pasteBytes` (line endings to Enter, control and escape characters dropped, bracketed paste). |
| `terminalChatLink.ts` | Loaded with the Terminal tab and then alive all session: re-asks the backend's mode gate when the chat, its agent or its mode changes, and hands an unsent chat's terminals to the key its first send creates. |
| Terminal store (`store/terminals`) | Each chat's terminals and their screens (parity 7.2): open, type, resize, close, the refusal kept for the tab, output drawn one frame later (newest 2 MiB wins), and the Terminal tab's close ending the chat's shells (`onTerminalTabClosed`). The tab can be left without ending anything. `components/TerminalPanel.tsx` renders it. |

| `prPanelModel.ts` | The PR tab's words (parity 5.4): state chip (`prStateWords`), checks summary and worst state (`checksSummary`), review and merge wording, the quoted draft for Ask about this (`commentDraft`) and the https-only link guard (`safeLink`). `PrPanel.tsx` renders them. |

| `diffPanelModel.ts` | The Diff tab's pure parts (parity 5.3): the file tree with single-child folders joined (`buildTree`), Find (`filterFiles`, `splitMatches`, `countLineMatches`), the Compare menu's scopes (`scopeChoices`, `scopeKey`) and Viewed per folder and scope, which clears when the file's signature changes and is capped (`setViewed`, `isViewed`). `DiffPanel.tsx` renders them. |

| `cardGitModel.ts` | The card's git rows' words (parity 5.2): the branch chip (`branchWords`, `branchLabel`), the file count (`changesCount`), the primary button and why it is off (`primaryAction`), and the generated commit message split into the prompt's subject and the file list kept while the subject is unchanged (`splitMessage`, `finalMessage`). `SidePanelCardGit.tsx` renders them. |
| `sidePanelModel.ts` | The side panel card's and New tab's words (overnight 11-12 C4, C5; the header's Open in rows `openInRows`, machine name `cardMachineName` and `PROJECT_SCRIPT_PROMPT`, parity 5.1): one glyph per subagent up to four (`cardGlyphs`), "4 done" / "2 running, 5 done" / "None yet" (`cardSubagentWords`), the newest three outputs and sources (`cardOutputs`, `cardSources`), View all's target (SOURCES.md, else WORKSPACE.md), a file row's name, folder hint for same-name files (`cardFileRows`), type icon and full-path label (`cardFileRow`, C8) and a New tab's Tools with why one can't open (`panelTools`). Pure, unit-tested; `components/SidePanelCard.tsx` and `components/PanelNewTab.tsx` draw them. |

| `subagentsModel.ts` | The Subagents summary, tab and session header's words (COO rebuild L3): counts ("2 running, 5 done"), up to three distinct agent glyphs, durations in the chat's unspaced form ("4m 12s", "16h ago", from `turnFooter.ts` `durationText`), tokens ("182K"), live elapsed from the read's `at`, queued places, a row's right-aligned "when" (`rowWhen`) and quiet second line (`rowMeta`, which leads with what a queued row waits for, `waitingWords`, 2026-11-12), the CFO's choice under the session header from the latest receipt's `cfo` (`cfoHeaderLine`), model names from each agent's catalog, a transcript as exchanges, and which tool calls start or message a subagent; `chipShown`/`chipWords` keep the chip for earlier chats alone ("4 earlier chats", L8). Pure, unit-tested. `store/subagents.ts` shares one polled `session_subagents` read per chat; `components/SubagentsInChat.tsx` (transcript line), `components/SidePanelCard.tsx` (persistent summary) and the lazy `components/SubagentsPanel.tsx` (the tab; its transcript replies take the chat's Useful/Not useful and Read aloud, `Reply.runId`) draw it. |
| `monthDay.ts` | ST2 follow-up (2026-10-14): "Oct 14" in the person's locale from one `Intl.DateTimeFormat`, for labels drawn per row (Tasks' due dates, the sidebar's and Home's "when", imported conversations, Home's workspace dates); same output as `toLocaleDateString` with those options, an invalid date included. |
| `runOutcome.ts` | Shared run summary facts accepted from wire or normalized receipts; operational words (`OPS_STATE`), changes, checks, outside effects and unknown calls. Automation hosts separately supply cost and retry status. Lazy `components/RunOutcome.tsx` heads ExecutionReceipt in Chat, Activity and Automations; docs/reference/runs.md and docs/AUTOMATION_RECOVERY.md. Pure, unit-tested. |
| `SmokeHarness.tsx` | The smoke tests' harness root: mounts single components for Playwright. |
| `previewKinds.ts` | Run 15 P1: how the Preview pane shows a text file: CSV/TSV as a table, JSON pretty-printed, code highlighted (plain above 200,000 characters). Pure, unit-tested. |
| `reviewNotes.ts` | Row OC9: line-anchored review notes on a file in the Preview pane. A note keeps its line's words and neighbours; `reanchor` follows it across edits (or marks it lost); `composeBatch` makes the one message for the chat draft (resolved never, 12,000 characters); bounded, tolerant storage under `hollow.reviewNotes.v1`. Pure, unit-tested; `components/PreviewReviewNotes.tsx` draws it. |
| `previewDetails.ts` | What a Preview pane tab says about its item (2026-11-06, one header row): a file's kind in words, its size and time, the tab tooltip (details line, then the full path) and the details line ending the file's ••• menu. Pure, unit-tested. |
| `previewCursor.ts` | Parity 5.6: the Preview pane's inspection cursor. Pure placement of the first visible, uncovered match inside the viewport and the `browser_show` `cursor` presentation argument (panel duration, primary, surface and ink colours); no cursor argument clears the marker. The mini player itself is `components/PreviewPlayerControls.tsx` ([Preview player](PREVIEW_PLAYER.md)). |
| `railDrag.ts` | Parity 3.2: pure drop-destination plans for a dragged sidebar chat (Pinned, Active, Settled). Each plan names the exact existing row-menu command and its enabled state; a pin protects against settlement and there is no combined unpin-and-settle. `components/RailDropZones.tsx` is the lazy drop-zone strip. |
| `railChatDrag.ts` | Parity 3.2: the drag source for sidebar chat rows: native ghost, cleanup that only clears its own drag. |
| `railListMotion.ts` | Parity 3.2: measures keyed sidebar rows across shelves and animates reorder and entry fade under the panel-animation duration; zero, reduced motion and navigation cancel owned animations. No cloned exit rows. |
| `railProjectChildren.ts` | Parity 3.2: keeps the full descendant graph while row visibility follows organization, so a moved parent does not hide active descendants and orphaned subagent roots stay reachable. |
| `railRowDetails.ts` | Parity 3.4: window-only hover and focus state for the sidebar row details card (`components/RailRowDetails.tsx`, lazy); Details in the row menu pins it open. |
| `railRowFiles.ts` | Parity 3.4: synchronous capture of the exact row key, conversation version, page and DataTransfer files at drop time, before any lazy import; remote and lead-less rows refuse explicitly. |
| `railFileDrop.ts` | Parity 3.4: waits for that exact destination's composer (history hydration included) and checks identity before each file; existing composer limits, never auto-sends, no pending-file queue. |
| `aboutYou.ts` | Pure helpers for the profile builder (Settings → Personalization's `settings/AboutYouPanel.tsx` and Library → Learnings' `components/ProfileSuggestions.tsx`, over `store/profileBuilder.ts`; backend `profile_builder.rs`). No store or Tauri imports, so the node vitest suite can load it. |
| `activity.ts` | Activity (next-level P19). Pure, unit-tested. |
| `agentLabel.ts` | Shared recorded-provider display names for Activity and Main presence. No imports; keeps Activity lazy when Main reads a label. |
| `canvasGallery.ts` | Saved-canvas metadata mapping, comparison pairs and bounded mapping. `components/LibraryResults.tsx` owns the single mixed list; lazy `GalleryCanvas` in `components/CanvasGallery.tsx` owns selected revision/source/Compare actions, reusing ResultCanvas, HtmlArtifact and DiffView. Store aliases preserve palette, g x, workspace-filter and source-return routes. |
| `cooMenu.ts` | The COO's items in the top bar's "Actions for COO" menu (`workspaceMenu.ts` spreads them in), moved from the removed page header band's More (2026-11-11): actions it took, Change hard stop, check-in settings, How check-ins run, Open failed conversation, Create or Edit standing orders. |
| `emptyScene.ts` | Evidence gating for decorative empty-view artwork; failed/partial/loading reads stay distinct. |
| `quickChatMove.ts` | Quick chats (2026-11-14): `quickChatMoveTarget`, the one project a quick chat's latest prompt and reply are clearly about (its folder named, or its title said twice; two projects or a common title: none), for `components/QuickChatMoveOffer.tsx`. Pure, unit-tested. |
| `railView.ts` | The Quick chats list's view (pure): sort, filter and group choices, saved views (12, newest kept), damaged-storage reads; persistence in `store/railView.ts`, ticking in `store/rowSelection.ts`, bulk writes in `conversationBulk.ts`. |
| `conversationBulk.ts` | Bulk Settle, Snooze and Move on the ticked chats: each chat written with its own revision and activity, a stale one left and counted, one 5 s Undo that wakes the changed ones. |
| `railChildren.ts` | Subagent chats nested under the chat that started them in the sidebar (pure): the split of top rows and children, a group's most urgent state, counts and the fold's words. |
| `rowStatus.ts` | The status marks on a sidebar chat row (pure): Approval or Input while waiting, Plan ready, an unread dot, the linked pull request; facts come from `store/rowStatus.ts`. |
| `runElapsed.ts` | A running row's elapsed time as `45s`, `12m` or `1h 05m` (pure); the clock is `store/secondClock.ts`. |
| `minuteClock.ts` | Shared timestamp for relative ages in Rail and section views; one 30-second timer while consumers are mounted, released after the last consumer. Section parts re-export it without pulling panel code into the rail. |
| `dayRing.ts` | Pure local-day SVG geometry, source coverage labels and exact timed-item projection for Today and Agenda. |
| `agentToolPath.ts` | Settings → Advanced → "Folders agents can run tools from" (overnight 2026-10-16 A12): rows from `agent_tool_path` (each PATH folder with its source and whether it exists, or "didn't answer" when its check passed the 2 s deadline), the joined PATH Copy puts on the clipboard, and the note when the shell PATH failed or is pending. Pure, unit-tested; the panel is `settings/AgentToolPath.tsx`. |
| `agenda.ts` | Agenda layout (next-level P10). Pure, unit-tested. |
| `agendaTypes.ts` | Rust `agenda::events_with` (next-level P10). `complete: false` (reason in `partial`) when the read stopped before Graph's last page (merged plan slice 5): the events are what was read, not the whole range. |
| `agentSettingsPayload.ts` | Paperclip PR 14a (cross-m34 M1): the ONE builder of the agent-settings save payload, so no field (models, efforts, the no-output watchdogs, the run deadlines) is silently reset by a save that forgot… |
| `annotate.ts` | Annotate in the Preview pane's web mode (BrowserAnnotate.tsx): the marks a user draws on a snapshot of the page, and the text that goes to the chat with the marked-up image. Pure, so it's unit-tested. |
| `sharedPage.ts` | Annotate → Include page text (repo ideas 2026-10-27 #3): the `browser_page_text` shape, the composer chip's label and summary, and the `<shared-page>` block the agent receives (the page as `<`-escaped JSON, marked as the person's share and untrusted). Pure, surface-independent, unit-tested. |
| `tabGrants.ts` | Agents read a Preview tab the person shares (repo ideas 2026-10-27 #5, read only): the `TabGrant` shape (Rust `tab_grants::Grant`), the grant on a tab, and the open chat's name for the In use by strip. Pure, unit-tested. |
| `appBuildBrings.ts` | What the Build app panel (`components/AppBuildPanel.tsx`) shows for "what this install brings" (RF-787-F1/F2): a note with nothing listed (a failed comparison, or "adds no commits") replaces the counts and the disclosure, so a failure never reads as "0 pull requests, 0 direct commits"; any other note shows outside the disclosure. Pure, unit-tested. |
| `api.ts` | Every Tauri command the webview calls, as typed functions (`api.*`); the browser mock answers them in mock mode. |
| `appBridge.ts` | The webview half of Hollow app tools (`hollow_app`, `src-tauri/src/app_broker.rs`). |
| `appBridgeCore.ts` | The pure half of the webview bridge (`appBridge.ts`), loadable in the node vitest suite: event shapes, refresh plans, the UI-owned setting keys (the Ui rows of `src-tauri/src/settings_keys.rs`; a… `navigateTarget` (with `NAVIGATE_ALIASES`), `workViewForRecord` (today + a Work view) and `visiblePath` (the answer, as the person reads it) since P1-05. |
| `actionDefs.ts` | The shared action definitions (plan completion L4, 2026-10-13): one typed row per action reachable from more than one place, with its effect (`navigate`/`view`/`draft`/`write`/`run`/`external`, plus `confirm`), its condition and the words shown when it can't run, its route-ledger row, its palette / `g` / ⌘ / menu-bar ids and any surface's own words. Pure data; `docs/ACTIONS.md` is rendered from it. |
| `actionRun.ts` | The one dispatcher for `actionDefs.ts`: `ACTION_RUN` (what each action does), `CONDITION_HOLDS` (whether its condition holds now) and `runAction(id)`, which refuses an unavailable action with its words in the footer. The palette's fixed rows, `goTo.ts`, `appMenu.ts` (native menu and ⌘ keys) and sidebar creation all call it. Rare actions load their module on first use. |
| `appMenu.ts` | Native menu actions (audit H14). `app_menu.rs` emits `app-menu` with an item id; each id is a shared action's `menu` (or ⌘ command's `key`) and runs through `runAction`, refused behind a modal (with "Close the dialog first") except copy-build and dictate. |
| `askScopes.ts` | Ask's scopes (consolidation D, 2026-10-13): the COO (Across all my work) or a new chat in Chats, each with the context it uses and what its button does, so a question never goes somewhere the person didn't pick; and the command bar's Ask row (`looksLikeSentence`, `askRowIndex`, moved from the removed `askHollow.ts`). Pure, unit-tested; Work's Ask field (`StudioHome.tsx`) offers them and `store/ask.ts` runs them (`askHere`, also the palette row's; `openHomeAsk` for `g v` and Ask anything…). The top bar's Ask ⌄ panel (with an In this workspace scope and a kept draft, `askDraft.ts`) was removed 2026-10-21; One Ask (2026-11-03) removed the my-notes and Hollow scopes with their overlays (Ask your vault, `vaultAnswer.ts`; Ask Hollow, `askHollow.ts`): every question is a chat. |
| `artifactSession.ts` | A known id is a resume candidate, not proof the provider still retains it. |
| `asOfTypes.ts` | Master plan F5/#17: `history_read_as_of` (`asof.rs`). |
| `automationWindows.ts` | Automations → Automation windows (overnight Spare S10; on Automations since Settings PR 4, `pages/workflows/AutomationWindowsPanel.tsx`): the editor's pure model. Draft from `automation_windows`, the backend's rules (`schedule_windows.rs` `WindowDef::problem` / `validated`) per control, the `set_automation_windows` payload with labels composed again for changed windows, and a backend refusal placed on the control it names. The panel is `settings/AutomationWindowsPanel.tsx` (lazy). |
| `resultCover.ts` | Pure listed-metadata cover labels; bounded file extension and truthful size, no file reads. |
| `browserReview.ts` | Browser and annotation review (next-level P15, 2026-10-01): the pure half. |
| `browserUrl.ts` | Address-field rules for the Preview pane's web mode (BrowserPane.tsx). Pure, so they're unit-tested; the Rust side (browser_pane.rs) re-checks every address before a tab loads it. |
| `budgets.ts` | Usage-budget labels and checks (Paperclip PR 8a, gate D2). Pure: no store or Tauri import, so the node vitest suite loads it. |
| `builtInCaps.ts` | The caps Hollow keeps for you (P1-07): the COO's daily hard stop and the overnight cap as rows with spend, limit and state, and the typed-cap parser. Pure; `components/BuiltInCaps.tsx` lists and edits them above Settings → Usage → Budgets' own budgets. |
| `canvasBridge.ts` | Canvas bridge v1 (Canvas C1, 2026-10-04): the typed, versioned message contract between Hollow and a sandboxed artifact frame. |
| `canvasFocus.ts` | Canvas focus (10-11): a finished turn with a canvas keeps the top of its last canvas in view until the next message or the person's own scroll; the "Back to canvas" chip. Pure; `components/chat/chatScrollController.ts` supplies geometry and events. |
| `canvasFrequency.ts` | Canvas C3 wave 2 C4: how often agents answer with a canvas. |
| `cfo.ts` | COO rebuild L7: the CFO's frontend helpers ([CFO](CFO.md)). `cfoFooterLine` (the turn footer's "Sonnet 5.5 · Medium · review", "(you)" when the person chose the effort; `modelName` names any provider's model id), the allowlist editor's model (`allowRows`, `setAny`, `setModel`: a model outside the agent's catalog changes nothing, so there is no free text; empty means every installed agent) and `accountRoomLine` for Settings → Usage. `settings/CfoSettings.tsx` renders the rows. Pure, unit-tested. |
| `canvasTurn.ts` | Where a conversation shows a canvas (Canvas C4 A3): "waiting on you" reopens a conversation at its ask-canvas, and the chat looks for the turn here. |
| `capabilityDoctor.ts` | Exports `CapabilityState`, `CapabilityId`, `DeclineableCapabilityId`, `VerifiableCapabilityId`, `CapabilityCheck`… |
| `capabilityState.ts` | One vocabulary for "can this be used?" (next-level P00/P16, 2026-10-01). |
| `captureClassify.ts` | Unified capture (next-level P12): what an "Add" input most likely is, and where it would go; `ingestSourceFor` builds Import a source's `ingest_run` source (S5). Pure, unit-tested. |
| `chatApproval.ts` | Approval badges on chat tool rows (Paperclip PR 21). Pure: type imports only, so the node vitest suite can load it. |
| `chatApprovalOrigin.ts` | Where a subagent's approval card shows (the chat that started the subagent too) and its "From subagent" line (SA1); `approvalPlacement` decides card, "waiting in the COO chat" note or hidden for a card raised to the COO (2026-11-20). Kept apart from `chatApproval.ts` so the shell chunk does not carry it. |
| `chatChainWords.ts` | One Main conversation (2026-11-07): the COO listed once (`oneMainConversation`) and a chain's forward-note and Earlier words. Read only by lazy chunks (ChatChainBar, Library → Conversations). Pure. |
| `chatContinue.ts` | Paperclip PR 14 (14b): the manual Continue after an interrupted or stopped turn, Retry's target under a failed last turn (`retryIndex`, P1-27) and under a finished last reply (`regenerateIndex`, the reply footer's Retry, C3), and the notice a fresh native session is announced with. Pure and dependency-free (vitest loads it in node). |
| `chatContinuedFrom.ts` | The earlier conversations a new one continues from (agent effectiveness F4, K1 review). |
| `chatCwd.ts` | Unsafe chat working folders (Paperclip PR 19). |
| `chatLimits.ts` | Settings → Chat → Chats at once (2026-10-20): `parseChatLimit` (a whole number in range, else the error shown beside the input) and `serialSaver` (saves one at a time on the last acknowledged state, and re-reads after a refused save in the same line, so overlapping edits or a late re-read never undo a lowered limit; a throwing callback never stops the line) and `chatLimitDrafts` (per field edit revisions: every acknowledged answer sets a field unless it has a newer typed or pending edit, so an untouched field follows a change made elsewhere and a refusal restores a field only when the refused edit is still its latest; the inputs stay mounted). `CHAT_LIMIT_MAX` is the ceilings, 25, 10 and 10 (subagents per chat, 2026-11-12; 25 and 10 since 2026-11-07), in step with the backend's `MAX_AT_ONCE`, `MAX_PER_WORKSPACE` and `MAX_SUBAGENTS_PER_CHAT`. Pure; the panel is `settings/ChatLimitsRows.tsx`, the backend `chat_limits.rs`. |
| `chatFileLinks.ts` | Chat file widgets (tool chips, reply-footer chips, file links in a reply) open in the Preview pane. These helpers turn what a widget carries into the absolute path the pane reads, and a reply's file link into that path plus the lines it cites (`fileLinkFromHref`). `videoRefsFromText` reads links and tags as hrefs (percent-decoded, cut at `?` or `#`) and inline code and bare paths as literal paths, so a video named `100%.mov` or `take#2.mov` resolves as written. |
| `chatArtifactKind.ts` | How the side pane shows a conversation file (`chatArtifactKind`: html, markdown, svg, image, video, code, binary), by extension. Its own module so tests import it without the store; `store/chatRuntime.ts` re-exports it. |
| `mediaKinds.ts` | Inline video (2026-10-17): `VIDEO_EXTS` (mp4, m4v, mov; equal to media.rs, a vitest checks), `isVideoPath`, `mediaSrc` (the `hollow-media` URL in Tauri, `/__media__/<token>` on the mock), the refusal codes and their copy, `classifyFailure` (a load failure judged by granting again), the app-wide budget of 4 mounted `<video>` elements (`takeMediaSlot`: an explicit open evicts the least recently visible paused inline player, never a playing one) and one-player-at-a-time. `components/MediaPreview.tsx` is the one player (`pane`, `result`, `inline`). Pure, unit-tested. |
| `replyMedia.ts` | Inline video (2026-10-17, PR 2): the videos under a finished reply. `receiptVideos` (the receipt's created or changed mp4/m4v/mov rows, vault rows joined to the vault, repo rows to their root, deletes and `clean` rows out: how a render made through Bash is found), `replyVideos` (receipt rows, then `chatFileLinks.ts` `videoRefsFromText`, each once), `splitReplyVideos` (three players, the rest "+N more"). `components/ReplyMedia.tsx` renders the strip in `ChatBody` on settled replies, outside the Markdown and the run fold; `store/chatRuntime.ts` commits the receipt's videos to the Files picker without marking the run as having written a file (`commitArtifact(…, { touch: false })`). Pure, unit-tested. |
| `chatHydrate.ts` | Overnight 10-11 B3: `turnsFromConversation`, the mapping from `read_agent_conversation` to `ChatTurn`s used by the workspace open (Conversation history, `components/ConversationHistory.tsx`, keeps its own, with notices), and `savedUserAt`, a saved user turn's send time for both (C3); `savedTurnAt` adds a saved reply's end (`endedAt`, overnight 11-12 C8) for the reply row's time; `savedUserText` (B1a, ENG-104) gives a saved user turn its file chips (the store's `files`, else a trailing `<attached-files>` block in its text), for both mappings. |
| `chatQueue.ts` | Queued chat turns (Paperclip PR 7b, D3): the pure decisions behind the composer's queued-turn state, and the sends getting ready that Stop marks before they queue (C10, `stopPreparingSend`). |
| `chatSteer.ts` | Mid-turn steering (2026-10-21): the composer's three choices while a turn runs (Steer now, Queue for after, Stop and send), what Enter does per agent (`defaultBusyChoice`), their words, steer rows (`steerTurn`, `steerRowsOf` for restored turns, a refusal's `reason`), what rules Steer now out (`steerBlockedWhy`, `steerTooLong` mirroring the backend's limits, P1-30) and every refusal in words (`steerRefusalNotice`; the first three fall back to Queue for after). Styles in `styles/chat-steer.css`; `busyMenuItems` is the chat's Send options rows, `components/HeldChips.tsx` its held-send chips, and `components/SteerActions.tsx` a refused steer's Send as next message and Put back in the box. |
| `chatRouting.ts` | Which conversation a chat event belongs to (build programme PR B4). Pure, so the node vitest suite can load it; the store in `store/chatRuntime.ts` applies it. |
| `chatSegments.ts` | Chat message segmentation — pure, dependency-free so it can be unit-tested. |
| `chatReadingAnchor.ts` | Visible-content reading anchor for late frame growth; residual native anchoring correction, input reset and observer cleanup. Since night ST4 (2026-10-14) a canvas that starts above the reading line is never the anchor when text follows it (scrolling up brought one in from the top, and holding its top let its growth push the text being read down); a canvas that fills the view is held by its bottom edge. |
| `chatSettle.ts` | Opening settle (10-12): after a conversation opens or ChatBody mounts again, keep the view at its end (or its restored place) while canvases grow from placeholders, until quiet or the person interacts. Pure; `components/chat/chatScrollController.ts` supplies geometry and size changes. |
| `chatFrameFocus.ts` | H10 (2026-10-15): `isTranscriptFrameFocus`, whether a window blur means focus moved into a frame inside the transcript (a click in a canvas or HTML frame, whose own events never reach the transcript). Pure; `components/chat/chatScrollController.ts` handles it as transcript focus (ends the settle and a reading hold). |
| `chatStop.ts` | Copy for the neutral notice a stopped chat turn leaves in the transcript, and the pause helpers (P1a PR 2): `pausedNotice` ("Paused: <note>. Resume picks it up from here."), `RESUME_PROMPT` (the backend's resume words, pinned to `chat_pause.rs` by its test) and `pausedRowsToProbe` (each chat's newest run-queue row when it ended cancelled, asked about once after a reload). `PausedChat` also carries `reason` (`user`, `all`, `usage_limit`) and `resumeAt` since P1b to P1d. Pure. |
| `chatResumeAt.ts` | Resume at a chosen time (P1c, 2026-11-15): `resumePresets` (in 1 hour; tonight 9 PM while it is still ahead; tomorrow 8 AM), `resumeAtWhen` / `resumeAtLabel` ("Resumes at 21:00", "tomorrow at 08:00", "Fri at 08:05", "Dec 2 at 09:00") and the custom time's `datetime-local` value both ways (`parseLocalDateTime`, `toLocalDateTime`). Unix seconds, the machine's zone. Pure; drawn by `components/chat/ResumeAtControl.tsx`. |
| `chatTranscript.ts` | Overnight 10-11 B3: the transcript reducer (`createTranscript`): text, deltas, tool calls/results/images, receipts, the turn boundary. Indexed store writes avoid repeated whole-history traversal on tool events; `scripts/agent-eval/transcript-bench.mjs` measures the browser-store reducer. Used by the on-screen conversation (`store/chatRuntime.ts`). |
| `chatTurnTypes.ts` | Overnight 10-11 B3: `ChatTurn`, `ChatToolUse`, `ChatToolStatus` (moved from store/runtime.ts, which re-exports them). |
| `chatUsage.ts` | Pure helpers for the composer's usage popover (components/ChatUsageMenu.tsx): context-window size, plan-window rows and labels, and (repo features R6) `handoverOffered` / `HANDOVER_TOKENS` (mirrors `context_gauge::ADVISORY_TOKENS`) for its Continue in a new conversation, plus `messageContextSplit` for recorded prompt shares, diagnostic `toolUseRows`, `largestRows` and `cacheRows` from `conversation_context_use`. No store or Tauri import, so the node vitest suite loads it. |
| `chatVisuals.ts` | Things the agent shows (```html / ```svg previews, canvases), of the open conversation. They render inline everywhere (Canvas C3, wave 2 C1); a canvas's Expand opens it in the Preview pane. |
| `canvas/canvasFlows.ts`, `canvas/flowAudit.ts`, `canvas/designAudit.ts`, `canvas/canvasProblems.ts`, `components/CanvasLayoutAudit.tsx` | Inert shared flow extraction, host-only geometry findings, the design check of a canvas's hand-written parts (repo ideas 2026-10-27 #1: a pure source pass plus the frame's measured `design` report, advisory only) and lazy current-reply controls: one findings row per canvas. Ask to fix appends a same-id draft with an optional guarded PNG; never sends. Large flows require Check layout. `canvasProblems.ts` keeps each run's counts for its receipt's Canvases row. Since repo ideas 10-27 #4, `canvas/pixelContrast.ts` (pure) and `canvas/measurePicture.ts` settle unresolved findings on two offscreen captures (Measure on the picture, Rust `canvas_measure`). |
| `customThemeVars.ts` | Pure custom-theme derivation: app core-text AA floor across five surfaces; raw derivation for named canvas adjustment receipts. Loaded on demand by the app theme store. |
| `canvasLook.ts` | Host-only canvas contrast adjustment and measured warnings. `store/canvasLook.ts` owns the stale-safe named palette cache; `store/canvasLookTarget.ts` isolates eager menu state. Lazy `components/CanvasLookDialog.tsx` previews, saves and removes the look. Live frame messages preserve document state. |
| `checklist.ts` | Checklist dialogs (`showChecklist`): the row shape and how the dialog's answer (the ticked ids, as JSON) is read back. Pure, so it is testable without the store. |
| `claudeAccounts.ts` | Pure helpers for Settings → Claude accounts, including scoped failover explanations. Type-only imports, so the node vitest suite can load it. |
| `commitments.ts` | Shared invalidation token for the Today, People, and Project commitment views. Durable state remains in commitments/*.md; this only coordinates refetches after a modal/status action. |
| `conflictCopies.ts` | Conflict copies (overnight A10): the pure parts of the person's resolve. |
| `conversationActions.ts` | The conversation menu's changing actions (P1-39): rename, archive and restore (both with Undo; archive refused while a turn is live), and New workspace from this chat (trashes the workspace it made when the move fails). Loaded on first use, outside the App chunk. |
| `conversationOrganization.ts` | Pin, snooze and settlement wire types, visible precedence and explicit local-time validation. |
| `conversationSettlement.ts` | Lazy opt-in automatic settlement choices, coalesced minute checks and revision-checked Undo. |
| `conversationOrganizationActions.ts` | Lazy menu writes, revision-checked Undo (5 s, also Edit → Undo Chat Action) and exact-key Wake. |
| `snoozePicker.ts` | The Snooze dialog: quick times as buttons and a date and time field in the shared prompt; resolves an ISO instant or null. |
| `conversationOrganizationLoad.ts` | Lazy IPC reads and writes, latest-request protection and unchanged row identity. |
| `mockConversationOrganization.ts` | Browser fixture persistence, revision checks and archive-backed settlement. |
| `conversationAbout.ts` | Who owns a conversation, where it is kept and how it continues (plan L3 X10, 2026-10-14; review V05): workspace, quick, COO and vault conversations, imported ones (`adopted`), agent changes (`stints`) and whether the agent resumes its own session (`resumable`). Pure; the context panel's "This conversation" group and Conversation history's "About this conversation" show it. |
| `connectedFiles.ts` | Pure helpers for the connected-project file list and reader (next-level P07). |
| `connectorCapability.ts` | Connector capability and repair (next-level P16). |
| `connectorsLibrary.ts` | Types and pure helpers for the Connectors library (Rust `connectors.rs`). Dependency-free so the node vitest suite can load it. |
| `contextLayers.ts` | Settings, Sources, Context (CX17): the five layer rows' pills (home, personal, org, access check, vault), the project folders line, the remote URL check (never a URL with credentials) and the machine name check, Create's guided steps, CX18's Set it up for me card (`setupCard`: repo name check, the `gh repo create` line, the address and the Connect field's prefill), and CX10's context check pills and buttons, leak mode line and the Monthly context audit automation template (`CONTEXT_AUDIT_TEMPLATE`, listed in Automations' templates). Pure, unit-tested; rendered by `settings/ContextLayersPanel.tsx`. |
| `contextLayersApi.ts` | Settings, Context's IPC: `context_layers_status`, `context_org_access_check`, `context_personal_local_only`, `set_context_root`, `set_context_machine_slug` (Rust `context_settings.rs`) and the sync engine's `context_sync_now`, `set_context_sync_enabled`, `configure_personal_sync`, and CX10's `context_check_status`, `context_install_check`, `context_remove_check` (Rust `leak_checks.rs`). Its own module so the App chunk carries none of it. |
| `contextReview.ts` | The found-context review card's words (CX20): each item's title and origin, why an excerpt is withheld, scan flags (never the text), a duplicate, why Accept is off (a secret, a blocking rule, an unread file, no personal or org clone, a file already at the target, not a project file), what each layer does (Personal copies, Project writes nothing, Org opens a pull request after a confirm), the Org confirm and what an Accept did; PX6: an auto-memory entry's "Why this layer" line, Home (this Mac's home folder) and Stale (proposes Skip, Accept off). The card is `components/ContextReviewCard.tsx` (lazy, at the top of Library, Learnings; exported for the first-run sheet with a scan id: Scan this Mac, then one Accept, Change layer and Skip per item, no select-all). |
| `contextReviewApi.ts` | The review card's IPC: `start_context_scan`, `context_scan_status` (CX19) and `context_review_list`, `context_review_accept` (one id), `context_review_skip`, `context_review_unskip` (Rust `context_review.rs`). Its own module so the App chunk carries none of it. |
| `contextSetupBrief.ts` | Lazy-loaded portable context-layer setup brief copied from Settings -> Sources -> Context ("Set up without Hollow"). |
| `firstRun.ts` | The first-run setup sheet's step logic and words (H6, with CX21 and F2): the five steps (Connect your context, Find what's already here, Where your work lives, Your agents, Start) with one line each, where it opens and resumes (the first open step), where Continue or Skip goes next, Back, the dots' names, the agents readout (CLI rows plus sign-in rows), the Start step's name check and summary. Pure, unit-tested. The sheet is `components/FirstRunSheet.tsx` (lazy; Settings' context, Sources and task tracker panels inline, `components/ImportConversations.tsx` and CX20's review card after a scan). |
| `firstRunApi.ts` | The sheet's IPC (Rust `first_run.rs`): `first_run_status`, `first_run_set_step` (done or skipped), `first_run_reopen` (Run setup again). Its own module, out of the App chunk. |
| `firstRunMount.tsx` | Mounts the sheet in its own root outside App: `checkFirstRun` (main.tsx, after the shell renders: one status read, the lazy sheet only when a step is open) and `openFirstRun` (Settings, Sources, Where your work lives, Run setup again). |
| `importConversations.ts` | Import conversations' words and selection (H6 with H5a): which source cards show (a folder source only when its folder exists, an export source when listed), the default selection (everything not imported yet), Select all, an inspect pass's progress, the list's counts and a commit's result line. Pure, unit-tested. The component is `components/ImportConversations.tsx` (lazy: the setup sheet now, Library later). |
| `composerChip.ts` | The message placeholder names the project or COO. |
| `computerWords.ts` | Cua CU8: `actionWords`, the words for one row of the side panel's Computer section ("click: act tier, ran"); pure, so its test loads alone. The chat's desktop store is `store/computerChat.ts`; the chip and card are `components/ComputerChip.tsx` and `ComputerCard.tsx`; the Settings sheet is `settings/ComputerControlSheet.tsx`. |
| `composerBar.ts` | The composer's bottom row (overnight 11-12, C3): `composerAddItems` (the + menu's rows: Attach files…, Add to Sources… in a workspace chat, Claude Code's own lists, the local commands, All commands…; a command goes only in an empty box, `KEEP_DRAFT`), `accessMenuItems` (the access chip's menu: every permission mode, the current one marked, also when this agent can't run it, then its note; nothing changes while a turn runs) and `accessChipName` ("Access: Full access (unrestricted)"). Pure; `components/ComposerCommands.tsx` and `components/ComposerAccessChip.tsx` render them, in the ChatBody chunk. |
| `popoverRoom.ts` | How tall an upward popover from a composer control may open (overnight 2026-11-04, C2): `roomAbove` (pure) up to the nearest clipping ancestor (`clipTopsAbove`) and never over the 44px title bar; the model menu sets `--model-pop-room` from it. |
| `composerDrop.ts` | The chat composer's drop target (P0-07; C5, 2026-11-04): every drop with files is taken (no webview default), every file attaches, a folder is refused in words (`FOLDER_REFUSED`, with Add to Sources… through `store/sourceAttach.ts` in a workspace chat). `splitDrop` asks the entry API (`webkitGetAsEntry`) which item is a folder and falls back to `isFolderLike` (no type, no bytes); the drag label (`dragLabel`) says "Drop files to attach". |
| `windowFileDrop.ts` | A file dropped anywhere on the window attaches to the chat (2026-11-04): `dropTargetLabel` says where it goes (this chat, a project's chat behind another page, or a new quick chat on Home, since QC2) and `routeDroppedFiles` brings that chat on screen (`focusWorkspaceConversation`, `startQuickChatForFiles`) and hands each file to the composer's own attach path (`attachChatImage`). `components/WindowFileDrop.tsx` listens on the window after every inner target, so the message box and Skills' import keep their drops, and shows a veil that never takes the pointer. |
| `composerFiles.ts` | Files attached to a chat message (C5, 2026-11-04): `isImageFile` (PNG, JPEG, GIF and WebP take the image path; other types are files), `attachRefusal` (a folder, an empty file, over 20 MB, before reading) and `messageCapRefusal` (10 attachments, 50 MB a message), `fileSizeLabel` for the chip, `displayFileName` (no controls, separators or bidi controls), `attachedFilesBlock` (the `<attached-files>` block: each file's original name, then its saved path; `promptFileName` keeps a name to one safe line), re-exporting `turnFiles.ts`. Pure; ChatBody reads and saves through `chat_save_attachment` with the name. |
| `turnFiles.ts` | The part of attaching files the launch chunk needs (C5 bundle fix, 2026-11-04): `ATTACH_MAX_COUNT` (10 a message) and `attachmentFiles` / `turnFilesField` for the sent message's chips, and `savedTurnFiles` (B1a, ENG-104), which reads them back from a saved turn's trailing `<attached-files>` block (only paths in a `.hollow-attachments/` folder) and drops the block from the words. `store/chatRuntime.ts` and `BrowserAnnotate.tsx` import it, so `composerFiles.ts` stays in the ChatBody chunk (importing that from the store put every composer helper into the App chunk, past its 340 KB budget). Pure. |
| `conversationTools.ts` | Long-conversation navigation (next-level P04): find in the thread, an outline of the user's prompts, bounded rendering, an honest "continue in a new conversation" handoff, Copy conversation's Markdown (`conversationMarkdown`, P1-27) and the title of its note (`conversationNoteTitle`, overnight 11-07), and `lastUserIndex` (one pass for the footers' "latest", overnight C5). |
| `chatOutlineRail.ts` | The chat outline rail (overnight 2026-11-12, E9): `railDashes` (which prompts get a dash: all up to `RAIL_MAX_DASHES`, 40, else 40 evenly spaced keeping the first and last), `dashFor` (the dash standing for the current prompt) `currentPrompt` (the prompt above a reading line a third down the transcript, or the one before the render window) and `railTabStop` (the strip's one tab stop: the dash keyboard focus moved to, else the current one). `components/ChatOutlineRail.tsx` (lazy, its own `styles/chat-outline-rail.css`) renders the strip and the Outline card. Pure, unit-tested. |
| `findHighlight.ts` | Find in this conversation, marked in the text (overnight 2026-11-04, C4): `matchOffsets` and `sameHits` (pure), `touchesFindText` (which DOM changes repaint), and the CSS Custom Highlight painters over `FIND_TEXT` containers only, the current turn first (`chat-find-current`) then the rest (`chat-find`); the DOM is never changed. |
| `userMessageFold.ts` | Long messages of yours fold behind Show more (overnight 2026-11-04, a C3 follow-up): `foldsUserText` (more than `FOLD_LINES`, 12 lines, or `FOLD_CHARS`, 1,500 characters of visible words, trailing blank lines not counted; decided from the text, so the fold is there from the first paint), `findShowsWhole` (while Find searches, at its own two-character floor, every long message shows whole) and `findKeepsOpen` (the long message Find was on stays open as it closes). It counts line breaks and characters, not wrapped lines. `components/ChatBody.tsx` renders the fold and Show more / Show less, the open messages in one set of turn keys (`openTexts`), both buttons working in place through `components/chat/chatScrollController.ts` `toggleInPlace`; the nine-line height (`9lh`, `FOLDED_LINES`), the mask fade and the print rule are in `styles/chat-workspace.css`. Pure, unit-tested. |
| `decisionRows.ts` | Master plan #20's decision rows: a workspace's own (settled first, then revisit) and Library, Records' list of every scope (`libraryDecisionRows`, revisit first; COO rebuild L4), with `decisionLabel` and `decisionScope`. Superseded ones leave the lists. |
| `delivery.ts` | A build workspace's delivery record (next-level P20). Pure, unit-tested. |
| `diagnosticsBundle.ts` | "Save diagnostics…" (overnight C6): the words around the redacted zip that `save_diagnostics` writes. Pure, so the unit tests import it directly. |
| `diffParse.ts` | Pure unified-diff parser for the Changes pane (Build PR B2). Input is `git diff` text (`build_diff`); output is files → hunks → lines with old and new line numbers. |
| `diskGuard.ts` | Settings → Advanced → Disk space (overnight D1): the words around `disk_guard`'s report, and the confirm dialogs and outcomes of its three cleanups, Delete old install builds…, Remove finished worktrees… (overnight H20) and Remove old indexes… (repo features R7b). Pure, so the unit tests import it directly. |
| `docOutline.ts` | A Markdown document's outline (next-level P04/P08): its headings, skipping fenced code and frontmatter. Pure, unit-tested. |
| `domLifecycle.ts` | Exports `listen`, `listen`, `listen`, `listen` |
| `dragdrop.ts` | Drag and drop for tasks and projects: `applyDrop` (optimistic status, priority and due writes with Undo), `isCompletionStatus`, and the conversation-drag re-exports (`sessionDrag.ts`, `sessionDragHandlers.ts`). Only lazily loaded code imports it (pages, the palette); the rail imports `sessionDragHandlers.ts` directly. A static import from the launch path would bring it back into the App chunk. |
| `escStop.ts` | Esc stops a running turn (frontier F6), but only when nothing else wants that Escape (an open menu, dialog or popover, the composer's Connectors included since P1-27), and only while the composer is empty (FIX-ESC). |
| `executionReceipt.ts` | Exports `ExecutionReceiptStatus`, `ReceiptOrdering`, `ReceiptCheckKind`, `ReceiptCheckOutcome`… |
| `extractTypes.ts` | Master plan F3: the extraction pipeline (`extract.rs`). |
| `extractionModel.ts` | The extraction model setting's wording (overnight A11). The setting is per agent: a model id means nothing to another provider. |
| `fileDrafts.ts` | Unsaved file drafts that survive an app restart (next-level P08). |
| `fileSave.ts` | One compare-and-swap save of a workspace file's draft (next-level P08). |
| `flightRecorder.ts` | Flight recorder (master plan #15): a turn's steps (`run_steps`, the F6 step spine) as a timeline. Pure, so the ordering and nesting are tested without the app. |
| `focusHold.ts` | Keyboard focus for the hand-made dialogs (overnight B8): Tab stays inside the dialog, and closing it puts focus back where it was. |
| `focusReturn.ts` | Focus back to the opener when an overlay is dismissed (H12, nested overlays): the command palette's Escape and backdrop. Since A05 (2026-10-16) also after a palette row that doesn't navigate: `returnsFocusAfterRun` compares the location stamp (`store/locationStamp.ts`: page, workspace, Work and Review views) before and after the row, and `giveFocusBackAfterRun` decides 180 ms and a frame after the row returns (shared action rows return at once), only when focus fell to the body. Only in the palette's lazy chunk, as is `store/locationStamp.ts`. |
| `gardener.ts` | The gardener's review card (master plan #19, `src-tauri/src/gardener.rs`): pure helpers for its lines and the person's ticks. |
| `highlight.ts` | Zero-dependency syntax highlighter tuned for the languages that actually show up in Hollow chat (TS/JS, Rust, Python, JSON, shell, CSS, HTML, diff). |
| `historyRun.ts` | Human presentation of an automated-write run (audit H12): lead with the workspace, time, outcome and changed files; keep raw ids, hashes and byte sizes for the details. |
| `hollowCards.ts` | The chat's notice for an agent that can't take Hollow's per-run MCP config (merged plan slice 2 item 4). |
| `hollowDatabases.ts` | Settings → Advanced → Hollow's databases (repo features R7b): the words around `hollow_databases`'s report (kind, owner, size, WAL, version, the integrity check's result and row counts, and the line pointing to Disk space → Remove old indexes…). Pure, unit-tested. |
| `formatTime.ts` | The one time formatter (N36, DL1): `formatRelative` (`16h ago`, `in 23h`), `formatClock` (`9:40 AM`, `Fri 11-13 9:40 AM`, a date after 7 days), `formatDuration` (`2m 55s`). Pure, unit-tested. |
| `homeReturn.ts` | `relativeWhen`: how a time reads ("12m ago") for Results, Needs you and file views. The return-row helpers went with HomeReturnRow (11-10). Pure, unit-tested. |
| `icons.tsx` | The icon set: `Icon` and the named SVG paths every component uses. |
| `iconMeaning.ts` | Typed glyph meanings shared by action, tool and vault metadata; sparkles is reserved for at most three meanings. |
| `inventoryMap.ts` | The review's control and action inventory reconciled with today's source (plan completion L4, N1): every scanned file's class (capability with its ledger rows, primitive, registry, harness), files removed since the review, baseline actions the scan can't match, and the command-only ways in (slash commands, in-surface keys, tray) with their visible routes. `scripts/inventory-scan.mjs` repeats the review's scan; `scripts/inventory-reconcile.mjs` writes `docs/INVENTORY_RECONCILIATION.md`; `inventoryMap.test.ts` fails on anything unclassified or a stale doc. Pure data. |
| `inlineAiModel.ts` | Overnight 10-11 B5: inline AI's state without a view: the suggestion `StateField` (mapped through edits), `requestFor`, `acceptSpec`, the slash menu's items and matching. The view is `editor/inlineAi.ts`. Node-tested. |
| `keyedList.ts` | Stable row identity for lists fed by the backend; `keepUnchanged`/`keepUnchangedRecord` keep store signals' unchanged items (P4). |
| `keyboardLeader.ts` | The go-to leader (five asks A4): `g` then a key, its targets and default keys, rebinding, the keydown step machine and the typing check. `NAVIGATE_PAGES` (P0-06) is the agent `ui.navigate` page list built from the targets; the Rust list in `app_actions.rs` must equal it plus `appBridgeCore.ts`'s `NAVIGATE_ALIASES` (navigation.test.ts). |
| `goTo.ts` | The go-to targets as shared actions (five asks A4; L4): `GO_ACTIONS` / `runGoTarget` run each target's action through `runAction`. |
| `macReserved.ts` | macOS's own shortcuts no Hollow command may take (five asks A4), and the two Hollow uses for their macOS meaning. |
| `menuState.ts` | Greys the menu bar's Stop Running Turn, Back and Forward when they can't act (P1-49), through `set_menu_enabled`; loaded after launch by App. |
| `keymapSections.ts` | The keyboard shortcuts card's content (overnight F4), built from registries rather than written in the component. |
| `launchIpc.ts` | The IPC calls this window makes while it launches, against a budget (overnight Y7): every `api.ts` call is tallied for the first 15 s; a call repeated with the same arguments more than twice is over budget (Settings → Advanced → Launch calls, and one console warning). Tiny and store-free, because `api.ts` is on the launch path. |
| `lastConversation.ts` | Launch restore (frontier F4). Hollow opens onto the conversation that was on screen when it last closed, the way Codex and Conductor reopen the last thread. |
| `latestRequest.ts` | Request identity, the one way an async view drops a stale response (plan L3 X21a, 2026-10-15): `latestRequest()` gives `start()` (a ticket that supersedes every earlier one), `isLatest(ticket)`, `peek()` (the current ticket without superseding it, for a save that belongs to the open file) and `cancel()` (onCleanup, input cleared); `runLatest(order, request, deliver)` delivers only the latest outcome; `unchangedSince(read)` says whether the identities `read` lists are still the same after an await (`store/chatRuntime.ts` `sameChatAsNow()` keeps a permission confirm's answer on the chat it was asked on). Moved here from `searchState.ts` (`requestOrder`, H09). Pure, unit-tested; `scripts/check-latest-request.mjs` keeps the migrated views, stores and factories (X21b) free of hand-rolled `= ++counter` guards. |
| `learningBulk.ts` | Bulk review of learning proposals in Waiting on you: which learnings can be ticked, the Select all state (capped at one 500-item batch), how many selected may repeat a saved learning, the optional Reject reason, the per-item footer messages for a batch and its Undo, and the "Undo last batch (…)" button's words (`undoLastLabel`) and what it keeps after an Undo (`batchAfterUndo`: nothing, or the learnings a partial Undo left; `nothingLeftToUndo`: the only error that lets it go), and the confirm before an Accept that names selected learnings not on screen (`offScreenAccept`, P0-10). Pure, unit-tested in learningBulk.test.ts. |
| `libraryListModel.ts` | Library → All (overnight 11-12 L3): one list from the classified vault index, Results' files and canvases and Plaud recordings; the Meetings, People and Outputs rules over paths and kinds, which notes it leaves out, dedupe of a note at a result's path, newest-first sort, Recent's 30-day window, search, by-meaning rows, per-source states (`readState`, `recordingsState`, `stillReading`: a source with no value yet is loading on its own; Meetings and People read result files too) and `limitShowing` (the page count that paints the row Show older focuses) and empty states; a result's meeting or person kind comes from the raw vault items (`buildLibraryList`'s `items`). `components/LibraryList.tsx` (lazy) renders it. |
| `libraryResultsModel.ts` | Mixed Library Results metadata join, workspace/canvas deduplication, nullable review evidence, type/workspace/search filters and independent reset scopes; no body reads. |
| `libraryCanvasListing.ts` | Six-reader metadata scheduler, refresh coalescing, timeout coverage without abandoning native reader slots, stale-delivery and disposal guards. |
| `canvasCompareMode.ts` | Selected canvas Compare toggle; new request numbers reopen it without remounting the detail or overriding a local close on refresh. |
| `libraryGroups.ts` | One Library row: what the note is called and a line about it. |
| `collate.ts` | Shared collators (`naturalCompare`, `keyCompare`): `localeCompare` with options builds a collator per call, so the Library and Tasks sorts use these (profiling night 11-20). |
| `transcriptWindow.ts` | The transcript's window of drawn turns at both ends (parity 0.3): `TURN_CAP` 200, `LOAD_STEP` 80, `AUTO_STEP` 20, `trimEnd`, `trimStart`, `laterEnd`, `windowFor`; pure, tested without a DOM. |
| `chatVisitMark.ts` | "New since you were here": turns seen per conversation in localStorage (`hollow.chatVisits.v1`, 200 kept), `watermarkAt`, the divider's words. |
| `libraryNoteManage.ts` | Rename, move and trash for Library notes (P1-18): which files offer them (`canManageNote`: Markdown outside workspaces and hidden folders) and where a rename or a move lands. Pure, unit-tested. |
| `libraryViews.ts` | Library saved views and note templates (next-level P06/P08). Pure, unit-tested; the views persist in localStorage `hollow.library.views`, each with `byMeaning` (P1-19; older views read as false). Also `meaningInCollection` / `meaningCountText`: By meaning's hits kept to the chosen collection, and its count line. |
| `lineDiff.ts` | One line of a diff: kept, removed (only then) or added (only now). |
| `linkGuard.ts` | Main-window link guard (2026-10-02, review A P2). A web or mail link clicked anywhere in the main window (a note, rendered markdown, a setting) would otherwise navigate Hollow's own window away. |
| `listKeys.ts` | Single-key list navigation (five asks A4): j / k / x / e / r / . in lists marked `data-key-list`, which list a key applies to, and the step rule. |
| `localKeys.ts` | In-surface keyboard commands and their policy (next-level P18; the policy is written in docs/reference/keyboard-shortcuts.md, "Local-command keyboard policy", and shown in Settings → Keyboard shortcuts). |
| `main.tsx` | The webview entry point: mounts the app (or the onboarding, capture or smoke-harness roots). Pure `store/textScale.ts` shares normalized size choices with the store and Settings. |
| `mainAgentSpace.ts` | The COO's pure parts (five asks A3; the module keeps its Main agent name): the standing orders file name, the spend gauge, check-in words, the acts count and presence. Its page header band was removed 2026-11-11; `settings/MainAgentSettings.tsx` shows its day (check-ins, spend, suggestions) with these. |
| `mainAgent.ts` | The COO (main agent M1, 2026-10-07): the always-on agent behind the top bar's orb (`MainAgentOrb.tsx`; consolidation D made it an Ask button 2026-10-13, restored 2026-10-14; the Ask ⌄ beside it was removed 2026-10-21). Its conversations live in the built-in `main` workspace (Rust `workspaces::MAIN_AGENT_ID`). |
| `markdownOptions.ts` | marked's options per surface (P1-18): a whole file (`NOTE_MARKDOWN`: the Library reader, the preview window, Skills) joins a soft line break; messages keep it. |
| `meetingsCopy.ts` | How meetings reach Hollow and its agents, in one wording (P1-20): Recordings in Hollow (Hollow's own Plaud sign-in), Plaud tools for agents (the connector), and where calendar comes from. Settings → Connectors' Recordings subhead and Plaud row, Library's Meetings and Kept transcripts collections and Recordings' sign-in button use it; `chat.rs`'s `MEETINGS_BRIEF` repeats the agent half (checked by `meetingsCopy.test.ts`). |
| `memoryProvenance.ts` | Where a memory came from, in words (next-level P18). |
| `messageActions.ts` | The quiet actions under your own chat messages (overnight 2026-11-04 C3): Edit's target (`editIndex`: your last message, while nothing runs or waits), which messages Retry and Edit can send again (`repeatable`: not one Hollow wrote, `hostWrittenTurn`, such as a wake, the Continue prompt, a card's or canvas's answers or the continuation on another agent, and not one with images, files or a shared page, `carriesAttachments`) and when it was sent (`sentTime`, the reply footer's `7:12 PM` / `Nov 3, 7:12 PM` with the full date in the tooltip). `components/ChatBody.tsx` renders them; a saved turn's send time is `chatHydrate.ts` `savedUserAt`, so the launch chunk never loads this module. Pure, unit-tested. |
| `missionControl.ts` | Overnight 10-11 B2: Mission Control's pure parts: the live feed reducer over `chat-event` (`applyFeedEvent`), the cards from the run queue plus the feed (`missionCards`), the orbit layout, and elapsed/money formatting. Node-tested. |
| `missionTracks.ts` | Pure Mission Control lane grouping, active tool phase and recorded-cost coverage; consumed only by the lazy view. |
| `missionFeed.ts` | Overnight 10-11 B2: Mission Control's live feed reducer (`applyFeedEvent`), import-free because `store/missionControl.ts` loads it at launch; a tool's input is kept as a compact copy (string fields, 300 characters). |
| `mockAutomationWindows.ts` | Browser-mock `automation_windows` / `set_automation_windows` (overnight S10): localStorage `mockAutomationWindows`, refusing what Rust refuses with its wording; `mockAutomationWindowsError` fails the next save for the error path. |
| `mockApi.ts` | The browser mock: answers every `invoke` in mock mode (smoke tests, `npm run dev` without Tauri). |
| `mockBuild.ts` | Browser-mock fixtures for build workspaces beyond B1 (`?mock=1` only): the Changes pane (Build PR B2). |
| `mockCanvases.ts` | The canvas store in the browser mock (Canvas C3; the real one is `src-tauri/src/workspace_canvases.rs`). |
| `mockConflictCopies.ts` | Browser-mode fixtures for conflict copies (overnight A10): two pairs in memory per page load; a keep removes the pair, as the real resolve does, and Not a copy sets it aside until its Undo (morning M7). |
| `mockSources.ts` | Browser-mock Settings, Sources (H4b): the status, Save of knowledge references and the owner switch's dry run, apply, rollback and status in memory; starts tracked in Linear with a Notion reference, `mockSources=vault` starts vault only. |
| `mockContextLayers.ts` | Browser-mock Settings, Context (CX17): one in-memory layers fixture under `/Users/mock` answering every Context command; `mockContextLayers=connected` starts with a personal remote and an org clone, `mockContextOrgAccess` (`denied`, `offline`) sets the access check's answer; the context check rows (CX10) list the fixture's git projects, and install or remove flips their state. Touches nothing on disk. |
| `mockContextReview.ts` | Browser-mock context scan and review card (CX19, CX20): Scan this Mac answers scanning once then ready with seven files (a secret-like one, a duplicate, a project file, and two Claude Code auto-memory entries sorted Home and Stale); `mockContextLayers=connected` sets up both clones, `mockContextReview=org` adds one queued org fact. Accept, Skip and Unskip change the in-memory list only. |
| `mockFirstRun.ts` | Browser-mock first-run setup and session import (H6, H5a): `first_run_*` progress in localStorage `mockFirstRunSetup` (`?firstRun=1` or `mockFirstRun=open` starts a fresh setup, else setup is complete), and the import commands (two folders and a ChatGPT export card, `mockSessionImport=none` for no folders; Look reads once then lists four Claude Code conversations). |
| `mockDiskCleanup.ts` | Browser-mock Disk space (overnight D1, H20): `disk_guard`, `install_targets` / `prune_install_targets`, `worktree_hygiene` / `clean_worktrees`. What was cleaned stays in localStorage (`mockDiskPruned`, `mockDiskCleaned`) so the next measure shows it gone; each delete is logged in `mockDiskCleanups` with its hash, and a hash that isn't the preview's refuses with the backend's words. `mockDiskInUseUnknown=1` plays an `lsof` failure. Also `hollow_databases` and Remove old indexes… (`stale_indexes` / `remove_stale_indexes`, `mockDiskIndexesRemoved`; repo features R7b). Touches nothing on disk. |
| `mockDamagedStores.ts` | Browser-mock damaged state stores (B·F08). |
| `mockDemo.ts` | Realistic browser-mock fixtures for design review and performance checks (next-level P01/P21). |
| `mockScenarios.ts` | Named whole-app browser-mock states (`&scenario=disconnected,interrupted,…`): the fixture contract in `docs/FIXTURES.md` (plan completion L5). |
| `mockDirectories.ts` | Browser-mock connected-project inventory (next-level P07). |
| `mockMode.ts` | Whether this page runs on the browser mock (`mockApi.ts`) instead of Tauri. |
| `mockPhoneAccess.ts` | Browser-mock `phone_access_*` (the phone chat API's Settings section, 2026-11-04): off, nothing paired, Tailscale running and unpublished; the switch, Publish on Tailscale / Stop publishing, Pair an iPhone (code, address, QR) and Revoke change it, kept in localStorage `mockPhoneAccess`, which a spec may seed (`fail` makes the next click fail with that sentence; `failAfter` fails it after its change, as a Stop publishing that tried every step and had one fail). Stop publishing leaves another app's 8443 and a foreground or service forward, as Rust does. Refuses what Rust refuses, with its words. v1.1: the push key's Import and Remove (`phone_push_*`, no key until imported) and `mockPhonePushTest`. |
| `mockPeers.ts` | Browser-mock `peer_*` (M2, the peer link; `docs/reference/peers.md`): no other Mac until localStorage `mockPeers` seeds one (JSON, or `demo`: a Mac mini online with a running chat, one with an open card and one with a canvas; the route ledger's `paired-mac` seed). The state is live on `window.__mockPeers`, so a spec stands in for the other Mac (change a state or a chat, then `__mockEmit` an event); `pairFail`, `busyKeys` and `onCheck` play a failed pairing, a busy send and Check now's answer; `peer_cancel_connect` turns a connecting Mac offline with Cancel's sentence; sends, stops and answers change the chat and emit a `detail` event as the hub would, and reject with a `PeerFailure`. `mockPhoneAccess.ts` adds Let another Mac connect's code and the Macs paired to this one. |
| `mockEventTriggers.ts` | Browser-mock `event_triggers_status` / `set_event_trigger` (F2): localStorage `mockEventTriggers`; tracked projects are the mock's projects without a build binding (`mockEventTriggerTracked` overrides), connected ones its build workspaces. Refuses what Rust refuses, with its words. |
| `mockMoveHome.ts` | Browser-mock Move Hollow home (2026-11-12): a plan under `/Users/mock` (vault and Projects moves with Personal, a second vault to Archive, two folders beside the vault that join with `includeVaultSiblings` and bring a clash with a winner, 3 stores, 4 Sessions columns, 2 id changes, 1,200 iCloud-only files, backup unknown); `mockMoveHomeChatRunning=1` adds a running chat so the plan isn't ready. Start sets `mockMoveHomeState` to `staged`, and each status read moves it on to running, then done; `mockMoveHomeRefuse=1` refuses a rollback. Starts and rollbacks are logged in `mockMoveHomeCalls`. Touches nothing on disk. |
| `mockTracker.ts` | Browser-mock task tracker (overhaul A2): plays the finished tracker (engine on, set up, a small synced team of issues, projects, states, labels, members, a cycle and a milestone) so Settings' Task tracker and the Tasks page build before the sync engine lands. localStorage `mockTracker` may seed `provider` (null = off), `key`, `engine: false` (this build's real answers), `teamId`, `teamKey`, `pollSeconds`, `error`. Refuses what Rust refuses, with its words. |
| `mockSyncClashes.ts` | Browser-mock context sync clash cards (CX6c): one clash on the personal root, only while localStorage `mockSyncClash` is set (the route ledger's `sync-clash` seed); a choice removes it, Edit marks it editing. |
| `mockPhoneLid.ts` | Browser-mock `phone_lid_*` (Keep Hollow running with the lid closed, 2026-11-05): off and on power, following `mockPhoneAccess`'s switch and paired phones; a click stands for the person's password. Kept in localStorage `mockPhoneLid`, which a spec may seed (`source`, `batteryPercent`, `otherDisabled`, `orphan`, `otherCopy`, `heldOff` for sleep still off on Hollow's marker after Hollow stopped asking, `stopped` for a helper both of whose daemons last passed 10 minutes ago, `outdated`, `unreadable` for a `SleepDisabled` Hollow can't read, `fail`). A click on the helper it already has asks nothing, as in Rust; refuses what Rust refuses, with its words. |
| `mockRecordings.ts` | Browser-mock Plaud recordings (next-level P11): one with transcript and summary, one with a summary only, one with a transcript only, and one with neither. |
| `mockReviews.ts` | Browser-mock (`?mock=1`) versions of the page-review commands (Rust `workspace_reviews.rs`, next-level P15), with the same rules: the review JSON lives at… |
| `mockWorkspaces.ts` | Exports `workspaceFixture` |
| `modalGuard.ts` | Modal-grade surfaces: anything that owns the keyboard while it's open. `modalOpen` reads live class collections (P4). |
| `modelCatalog.ts` | One model-catalog request per agent per session, shared by every model picker (the backend caches each list too). `loadCatalog(agent, true)` asks the agent again (Refresh). `newerModel` flags a pinned Claude ID whose family has a newer model; `checkedLabel` words the list's time. A failed load is not cached. `chatOnlyModel` / `withoutChatOnly` (self-hosted models Q5): Claude's list ends with the switched-on self-hosted models as `self-hosted:<id>`, which only a chat runs, so `ModelPicker` and the Extraction model's suggestions leave them out; Settings' Self-hosted models part calls `forgetCatalog('claude')` after a change. |
| `updatesText.ts` | Pure update result notices and relative last-check wording; covered by `updatesText.test.ts`. |
| [settings/Updates.tsx](../src/settings/Updates.tsx) | Settings Updates: guarded settings reads, channel downgrade notice, saved overnight preference, checks, wait cancellation, connection and pending-token decisions, and acknowledged install results. Token input clears on submission; connection errors exclude the submitted token. |
| [settings/UpdatesGoBack.tsx](../src/settings/UpdatesGoBack.tsx) | Settings Updates, Go back card (U6), lazy: plan, confirmed Go back or restore, chats-finish wait, skip Undo; `updatesGoBack.test.ts`. |
| [settings/UpdatesPublish.tsx](../src/settings/UpdatesPublish.tsx) | Publish to Main card (U8), lazy, publisher only: status on open, Refresh and pick; confirmed Publish and Withdraw; `updatesPublish.test.ts`. |
| [settings/UpdatesPeople.tsx](../src/settings/UpdatesPeople.tsx) | People card (U9), lazy: publisher switch, People list, install link shown once with Copy; `updatesPeople.test.ts`. |
| [store/updates.ts](../src/store/updates.ts) | Shared update state and subscription lifetime; restart, wait, cancel, dismissal, checks and result acknowledgment; Show in Update button for a Local build (row U10). |
| [store/updates.test.ts](../src/store/updates.test.ts) | Update store lifecycle, waiting and cancellation regressions. |
| [components/UpdateButton.tsx](../src/components/UpdateButton.tsx) | Top-bar update pill and nonmodal release disclosure, delegated install decisions, outside dismissal and Escape focus return. |
| [components/UpdateButton.test.ts](../src/components/UpdateButton.test.ts) | Update disclosure safety and accessibility source contracts; CSS reads use the filesystem. |
| [updatesText.test.ts](../src/updatesText.test.ts) | Pure update notice and last-check wording tests. |
| `agentUpdates.ts` | Settings → Agent updates wording: an agent CLI's version line, its main button (Update / Cancel) and in-progress line. Since repo ideas 10-27 #9 also `knownBadLines` (the warnings from Hollow's list of versions known to break it). |
| `modelPrices.ts` | Settings → Advanced → Prices (overnight 9 A13, moved 11-08 S5): the add-or-change form's pure model (`priceFromForm`, mirroring `usage_prices::validate`; a cache write priced at input) and `perMillion` formatting. The panel is `settings/PricesPanel.tsx` (it imports `usage-panel.css`); backend `usage_prices.rs` (`price_table` rows carry `saved`). |
| `modelMenu.ts` | Pure helpers for the composer's model menu (ChatModelMenu): one searchable list of every installed agent's models, grouped by agent. Type-only imports, so the node vitest suite can load it. |
| `moveHome.ts` | Move Hollow home (2026-11-12; Rust `move_home.rs`): the words around the dry run's plan and the move's journal (each report block: what moves, links left at the old places, records updated, name clashes, iCloud-only files, what must stop first, git worktrees, agents' records, login items and scripts, backup, notes), why Quit and move is disabled, when the panel polls and when Roll back shows. Pure, unit-tested; rendered by `settings/MoveHomePanel.tsx`. |
| `moveHomeApi.ts` | Move Hollow home's IPC (`commands/move_home.rs`): `move_home_plan`, `move_home_start`, `move_home_status`, `move_home_rollback`. Its own module so the App chunk carries none of it; imported only by the lazy panel. |
| `movedNotice.ts` | "This conversation moved" (next-level P13, 2026-10-01). |
| `nativeEvents.ts` | The one adapter for native (Tauri) event subscriptions (review F05, 2026-10-13): `listen` is Tauri's in the app and an in-page bus on the browser mock (`window.__mockEmit(name, payload)` delivers one in specs); `onNative` returns a synchronous disposer that works before the subscription resolves and never leaves a rejection unhandled. `scripts/check-native-events.mjs` refuses a direct `@tauri-apps/api/event` import elsewhere in `src/`. |
| `navigation.ts` | Exports `NavSection`, `PageNavMeta`, `NAV_PAGES`, `RAIL_WORKSPACE`, `RAIL_KNOWLEDGE`… |
| `needsYou.ts` | Exact-id helpers for the "needs you" query (merged plan slice 4, B·F15). Pure, unit-tested in needsYou.test.ts. |
| `operatorSwitch.ts` | The "Share About you with agents" switch at launch (wave 2 follow-up). |
| `overnightCard.ts` | Home's overnight card (overnight C8): the words, and whether it shows. Pure, so the unit tests import it directly. |
| `paletteDefault.ts` | The command palette's list before anything is typed: rows that act without asking never lead or join Recent, and run/write rows carry their effect line (P0-02). |
| `paletteSearch.ts` | Command-palette search scopes and result rows (next-level P07); the Actions chip is the `>` scope. |
| `paletteNav.ts` | The palette's pure navigation rules: the `>` prefix, Backspace back target, and the ⌘1 to ⌘9 row digits that work only while it is open (t3code parity 4.1). |
| `palettePreview.ts` | Read-only palette preview target, path guard, bounded plain excerpt and latest-request cancellation. |
| `paneScale.ts` | Resizable panes (the Preview pane, Conversation history's list in Library → Conversations) report their width to the resize handle. |
| `permModes.ts` | Which permission modes an agent can run (merged plan slice 2). |
| `agentPermModes.ts` | Each agent's own permission modes (per-agent menus, 2026-11-12): native names, chip names, one honest line each, danger, the nearest mode on another agent, `/permissions` words and the per-agent new-chat defaults (Auto for Claude and Codex, Accept edits for Grok and Antigravity). Pure. |
| `phoneAccess.ts` | Settings → Devices → Companion devices → iPhone app (the phone chat API, 2026-11-04; `docs/PHONE_CHAT_API.md`, backend `phone_access.rs`): the state chip, the server and Tailscale lines, `forwardLines` (a raw TCP forward Stop publishing can't reach, in a terminal's `tailscale serve` or a Tailscale service, with how to stop it and, for a service with a plain `svc:` name, the command; for any other name, the steps in words), the Publish on Tailscale / Stop publishing choice and why it can't be pressed, why Pair an iPhone can't, the QR code as an image address (an SVG without script or outside references only), and `deviceRows`, the one device list companion devices and paired phones share, with each kind's Revoke wording; v1.1's push status words (`pushStateLabel`, `pushLines`) and the Key ID in Apple's key file name (`keyIdFromPath`). Pure, unit-tested; the section is in `settings/CompanionDevicesPanel.tsx`. |
| `phoneApprovals.ts` | Settings → Devices → Companion devices → a phone's Approvals row (B4, 2026-11-19; `docs/phone-api/approvals.md`): `approvalsLine` (off, or on with the key fingerprint both screens show and when it was set up) and `APPROVAL_WAIT_CHOICES` (2, 5, 10 and 15 minutes, the range Hollow accepts). Pure, unit-tested; the row is `settings/PhoneApprovalsRow.tsx`. |
| `phoneLid.ts` | Settings → Devices → Companion devices → iPhone app → Keep Hollow running with the lid closed (2026-11-05, backend `phone_access/lid.rs`): the headline that says whether the lid can close now (on power, on battery, On battery too, waiting for a phone, sleep someone else disabled, a helper gone with its marker, sleep still off after Hollow stopped asking: back within 10 seconds, or the helper isn't running and Off turns it back on), never "closing the lid sleeps this Mac" while sleep reads off (or can't be read with Hollow's marker set, which counts as off), the unplug promise only while the restore check passes, the switch's rule (on needs the iPhone app, off always works), Turn sleep back on, Update the helper for one from another version, the battery warning and each click's notice (from the reading after the click). Pure, unit-tested; the control is `settings/PhoneLidClosed.tsx`. |
| `plainErrors.ts` | Raw OS, SQLite, network, JSON and Tauri errors in plain language: a sentence for the person, the raw text kept as the snackbar's Details (overnight C7); Hollow's own `CODE: ` prefixes dropped from the text (night ST10); `retryNotice` adds a Retry button (overnight G2). |
| `errorDetails.ts` | What a recovery screen's Copy details copies: the message, a `cause` that says more (stringified safely) and the stack. WKWebView's `Error.stack` holds frames only, so the stack alone lost what failed (10-23). Unit-tested. |
| `proseFileRefs.ts` | File paths written in a reply's prose rather than backticks (2026-11-04): `prosePathRefs` finds them (relative with a folder, absolute, `~/`, or a bare name with a document-like extension, so `Node.js` stays a word; not inside a URL, address or web host) and `markProsePaths` wraps each in a `span.file-ref.prose-file-ref` the reply's click opens like a backticked path. Its own chunk, loaded by ReplyMarkdown's decoration. |
| `replyTable.ts` | A table in a reply on the clipboard (overnight 11-07, D1): `tableToTsv` (tab-separated; a cell's tabs and line breaks become single spaces so a cell never splits), `tableRows`, `tableLabel` ("Copy table 2 of 3" only when a reply has several) and `copyTable` (the text and the HTML table together through `ClipboardItem`, plain text only where a webview refuses that). Reply-only: the caller is the lazy `ReplyMarkdown`, which wraps each table in a scrolling box with a Copy table button. |
| `selectionQuote.ts` | A passage selected in the transcript as a Markdown quote (overnight 11-07, D1, gap 9): `quoteMarkdown` prefixes each line with "> ", keeps one bare ">" for blank lines, ends with a blank line so the caret lands under it, turns CRLF and no-break spaces into plain ones and cuts at `QUOTE_LIMIT` (4,000 characters) with an ellipsis. The Quote button is `components/QuoteSelection.tsx`. |
| `fileDrag.ts` | A drag from Finder as the window and the message box read it (2026-11-04), with no imports so the window's drop listener stays tiny in the launch chunk: `carriesFiles` (by `dataTransfer.types` "Files", which WebKit gives during the drag) and `readDrop` (each item's file and whether it is a folder, read while the event lives). |
| `fileChip.ts` | File chips (2026-11-10): `fileKind` (a path's icon and tone by type: md, csv/xlsx, docx, pptx, pdf, html, json, txt, images, video, code, archives, folders, a plain file otherwise; also the tool ledger's chips through `toolDescribe.ts`), `chipName` (the name and cited lines a chip shows), `chipFolders` (the fewest folders that tell same-name files in one reply apart) and `chipLabel` (the accessible name, with the path). `components/ReplyMarkdown.tsx` `chipify` builds the chips. Pure, unit-tested. |
| `fileLocate.ts` | Where a file a chat names may be (2026-11-04: one base made vault-relative, repo-relative, worktree and Mac mini paths read as moved files): `linkBases` (the chat's folder, worktree, workspace folder, the repos its turns changed, the vault), `locateCandidates` (each reading, a runner path `…/hollow-remote/{folders,sessions}/<id>/…` mapped onto each folder, then the conversation's own files by path end) and `missingFileText` (scratch, Mac mini, moved or deleted). `store/fileOpen.ts` `openNamedFile` reads a link against each folder, asks `chat_locate_file` which candidate exists and opens it in the Preview pane: reply links and prose paths, tool rows, reply chips and run details. When none does, a relative path is looked for under every project's folder and repository, then the vault (`chat_find_file`, 2026-11-10): one match opens, several offer a picker. |
| `filePreviewFailure.ts` | Why the Preview pane couldn't show a file (`chat_preview_file`'s refusals: missing, a folder, an image over 10 MB), as a sentence and the way on (Retry, Show in Finder, Open in default app). Unit-tested. |
| `planModel.ts` | PLAN.md as sections of checklist rows (Canvas C5a, plan §6). Pure, no DOM. `planRowsInOrder` (the Files tab's ticking boxes) and `planItemLine` (a task's From: plan row) since COO rebuild L4. |
| `questionCardList.ts` | The question-card helpers the chat store needs on every event (overnight 12 A0): `applyInteractionUpdate`, `isOpenCard`, `answerSendFailureAction`, `ANSWERS_IN_COMPOSER_NOTICE`. Split from `questionCards.ts` (which re-exports them) so `store/chatRuntime.ts` keeps the card form's validation out of the App chunk. Pure. |
| `questionCards.ts` | Structured question cards (Paperclip PR 13): the pure half of the composer's card handling. |
| `questionDock.ts` | Docked question cards (2026-10-14): rows, starting cursor, plain-key mapping, step checks and the waiting chip for `components/QuestionDock.tsx`; `tooSoon`/`GRACE_MS`, the half second after the dock shows (or an ask-canvas's frame changes height, `HtmlArtifact`) in which a key or click can't answer. |
| `motion.ts` | Shared app/system reduced-motion reader and live per-window media query; store-free, used by exits, scrolling, theme and view transitions. Boot and isolated canvas kit remain separate. |
| `motionKit.ts` | Effective panel-duration and visibility observation, plus interruptible measured height/width lifecycles for `components/MotionKit.tsx`; clears owned geometry on completion, policy changes and disposal. |
| `railSearch.ts` | The sidebar's Search chats rows (overnight 11-04 S2; opened from the top bar's search icon or ⌘⇧K since L1, 11-12, `requestSearchChats` in `store/runtime.ts`): up to five projects by title, the chats `search_sessions` found, then Search everything (the palette). |
| `railWorkspaces.ts` | Which workspaces the rail lists, in order; `railGroups` splits them into Pinned and the rest, each listed once. One row per project, which opens its lead (L8, 11-10): nothing folds. |
| `projectIconColors.ts` | Project monograms and colors (4.4, A8): `monogram` (initials of two words), the nine `PROJECT_COLORS` hues, `defaultProjectColor` (a stable hash of the id), `projectColor` and `nextProjectColor`. The store half, `store/projectColors.ts`, keeps the person's picks per Mac in localStorage (`hollow.projectColors`); `ProjectIcon.tsx` shows the monogram when a project has no artwork or GitHub owner; the project menu has Change color and Reset color. |

| `projectScripts.ts` | Project scripts' pure half (4.5, A10): the `ProjectScript` shape, `SCRIPT_CHORD_KEYS` and the chord guard `scriptChordProblem` (⇧⌘ and one letter or digit; never a `macReserved.ts` chord, a Hollow command's `activeChord`, or another script's) with `scriptForChord`. `store/projectScripts.ts` holds each project's list (read when it opens), `runProjectScript` (starts the service, opens the Preview tab `{ kind: 'script' }`), `scriptForKeystroke` (App.tsx runs it when no Hollow command holds the key) and the editor request; `components/ProjectScriptsControl.tsx` is the card's Scripts section and editor, `components/ScriptOutput.tsx` the output tab (polls `list_services` and `service_log`). |

| `workspacePins.ts` | Pinned workspaces' pure half (2026-10-21): `pinOrder` (rank, then id; never archived or built-in), `canPinWorkspace`, `dropPosition` (a drag's 1-based place) and `pinAnnouncement`. The store half, `store/workspacePins.ts`, reads `pinnedWorkspaces()` from the workspace list and writes through `update_workspace` (Pin, Unpin, move), and adopts the old window-local `hollow.studio.pins` once. See [Pinned workspaces](WORKSPACES.md#pinned-workspaces). |
| `recordTypes.ts` | Master plan F4: records (`records.rs`). |
| `recordingTasks.ts` | Task proposals from a recording (next-level P11). Pure, unit-tested. |
| `recordingSearch.ts` | Literal transcript match parts for safe text-node highlights, preserving original spelling and lowercase-expansion offsets. Pure; no reads or writes. |
| `recordingTypes.ts` | Rust `recordings` (next-level P11). Recordings stay in Plaud; these are on-demand reads, explicit links and an opt-in kept copy. |
| `remoteImageLoader.ts` | The runtime half of click-to-load remote images (merged plan slice 1, reviews A·S3 / B·F13; the pure rewrite is `remoteImages.ts`). |
| `remoteImages.ts` | Remote images in rendered markdown load only when clicked (merged plan slice 1, reviews A·S3 / B·F13). |
| `restartResume.ts` | Resume after restart (overnight A1, P1-40): the "Resume N interrupted chats" checklist (one ticked row per chat with what its resume runs with; agent-started chats apart, to open) and the footer that names every skip. Pure and dependency-free (vitest loads it in node). |
| `receiptCites.ts` | Repo features R5: the words of a Run record's Cites row (`citesLine`: "9 checked · 7 seen · 1 not opened · 1 past end of file", zero parts left out; `citeItemLine`: "src/a.rs:12 · past end of file"). Imported only by the lazy `components/ReceiptCites.tsx`, so it stays out of the App chunk. Pure, unit-tested in `executionReceipt.test.ts`. |
| `resultKind.ts` | Paperclip PR 14a: the result kinds a chat turn, a workflow run or a receipt can carry, and their labels. |
| `resultExport.ts` | Export a Result as Markdown (overnight F6): which Results can go (text kinds), the file name from the title, the saved note. Pure; the dialog is in `store/resultExport.ts`. |
| `resultsCards.ts` | Pure Results card model: review/origin/visit metadata, thumbnail eligibility, shared THUMBNAILS=24 and libraryCards retaining workspace/path identity. `resetLibraryResultFilters` keeps search and review/workspace recovery scopes independent. Shared ResultThumb draws saved-canvas sketches, video posters and allowlisted raster images up to 2 MiB. Library's PreviewWhenNear defers reads to the owning scroll viewport; Canvases reserves two reads for its detail. |
| `reviewKeys.ts` | Review's one keymap (P1-13: x dismisses, a accepts, Shift-R rejects, e edits, z snoozes; `reviewKey(surface, event)`), `snoozeUntil` (09:00 the next morning) and `wakeLabel`. |
| `reviewInbox.ts` | Shared invalidation token for every review consumer (the waiting-on-you surface's cards, the review queue snapshot, Settings → Personalization). |
| `reviewQueue.ts` | ── Local suggestions: one snapshot of the vault-canonical .review-queue.json v2. |
| `routeLedger.ts` | The capability route ledger (consolidation D, 2026-10-13; L4): every user-facing capability with its home, visible click path, context entry, prerequisite and unavailable-state words, and an executable `walk` (targets by role and accessible name, with small-window `orElse` forms) ending at a `lands` landmark. `tests/smoke/route-ledger.journey.spec.ts` clicks every walk at desktop, 720 × 480 and Largest text; `route-ledger-accelerators.spec.ts` runs each row's accelerators (read from `actionDefs.ts`) to the same landmark. `routeLedger.test.ts` fails on a shortcut- or palette-only accelerator, a walk-less or keyed row, a conditional row without its unavailable words, a missing test or a stale `docs/ROUTE_LEDGER.md` / `docs/ACTIONS.md` (`node scripts/route-ledger.mjs` rewrites them). Pure data. |
| `runActivity.ts` | Paperclip PR 15 (15b): the silent-run indicator's pure helpers. No store or Tauri import (`import type` erases), so the node vitest suite can load it. |
| `runFold.ts` | Finished-run folding (frontier F5): `foldSummary` "Worked for 1m 10s" (overnight 11-12 C1), `foldSteps` the count for the tooltip and screen readers, `foldTail` the reply's last assistant turn (whose receipt feeds the chips above the fold); `foldKeepsText` keeps a card's preface and the reply's opening words (written before its first tool call, PER-13) on screen. |
| `replyActions.ts` | The row under a reply (overnight 11-12 C2): `replyRowButtons` (Copy, Useful, Not useful, Read aloud, Retry, More), `canVote`, `replyTime` ("Sunday 11:27 PM"), `replyFinishedAt` (the receipt's finish, else the reply turn's own `at`, C8), and the vote's input and words; votes save through `record_turn_feedback` (`store/replyFeedback.ts`), which the friction loop reads. |
| `runQueue.ts` | Pure helpers for the durable automation queue (Paperclip PR 7). No store or Tauri import, so the node vitest suite can load it. The dock's live-row helpers (`isLive`, `queuedCount`, `runningAutomations`, `dockVisible`) live in `runQueueLive.ts` and are re-exported here. |
| `runQueueLive.ts` | ST1 (2026-10-14): the run queue's live-row helpers, split from `runQueue.ts` so the top bar (App chunk) imports only these and not the hold/permission text (`workflowPermissions.ts`). |
| `scopedListen.ts` | Overnight 10-11/12 P6: a Tauri `listen` whose disposer also unlistens a registration that resolves after the view closed. |
| `searchState.ts` | Asynchronous search states (audit H09). Which request may answer is `latestRequest.ts`. |
| `sectionMarkdown.ts` | Line-level Markdown reading shared by the workspace section models (Canvas C5a: `planModel.ts`; the Brief and Overview models went with those views). Pure: no DOM, no Tauri, so vitest runs it in node. |
| `sectionSource.ts` | Where a workspace section comes from, and when it must be read again (Canvas C5a P2, docs/history/CANVAS_C5.md). Pure, no DOM. `SECTION_MARKDOWN` (P1-09): the section toolbar's third button per section, Markdown and the file it opens (Results: Full review). |
| `selfHostedModels.ts` | Pure helpers for Settings → Models → Self-hosted models (self-hosted models Q4; `settings/SelfHostedModelsPanel.tsx`): the add/edit form's draft and the entry it describes, the token command's arguments on one quoted line, and the words for a row and its Test. Q10: the Background jobs block's choices per job kind (`jobOptions`: As now, each switched-on model, the saved one when it can't run) and the line for jobs waiting for a model (`waitingLine`, `waitingDetail`). Type-only imports, so the node vitest suite can load it. |
| `selfHostedWake.ts` | Self-hosted models Q6: waking a chat's self-hosted model and the start chip's state. `wakeChip` (idle with Start now, Checking for the first 3 s of a wake, starting with the time so far as text, ready for 10 minutes after the health check answered, or the error for 10 minutes with Start now after the 2-minute wait; a refusal leaves the wait alone), `typingWakes` (no `/` command, no emptied box), `createWakes` (per-entry state shared by every chat on that entry; `nudge` wakes at most once per 2 minutes, `startNow` at once, a wake in flight is joined, answers anchored to their wake's start; `watch` reads the backend's last wake while a chip shows it, polling a start this window didn't send, with backoff after a failed read and a 17-minute cap; `forget` starts an entry over when Settings changes it, through `api.ts` `onSelfHostedChange`) and `createStatusView` (the chip's clock and mount reads, never a wake; unmounting ends its watch), with the calls and clock injected so the node vitest suite runs them. `store/selfHostedWake.ts` is the app's instance (`wakeFromComposer`, called only by the composer's input event and a send; the saved entries; `selfHostedChatWindow` for the usage ring) and `components/SelfHostedStatus.tsx` the lazy chip above the composer. `selfHostedModels.ts` `selfHostedEntryId` reads a chat's `self-hosted:<entry id>` model. |
| `selectModel.ts` | Pure list logic for the themed Select (components/Select.tsx), kept free of Solid/DOM so it is unit-tested in the node vitest environment. |
| `serviceHealth.ts` | Service health by capability (audit H04); since Settings PR 6 only Plaud's agent-access line reads it (the table is cut). |
| `sessionDrag.ts` | The conversation drag contract (2026-09-30): what a dragged conversation row carries and how a drop onto a workspace header is dispatched. |
| `sessionDragHandlers.ts` | The JSX handlers for a draggable conversation row and a workspace header that accepts one (`sessionDragSource`, `sessionDropTarget`, `SESSION_DROP_CLASS`). Split from `dragdrop.ts` (which re-exports them) by overnight 12 A0 so the rail doesn't pull the task drop (`applyDrop`) into the App chunk. |
| `sessionIndicator.ts` | One indicator family for a conversation's live state (asks 10-11 A1/A2): the states and their order (waiting on you > failed > interrupted > running > queued > paused; interrupted = Hollow's quit cut the turn short, P1-40; paused = the person paused it, P1a, from `store/chatPause.ts`), label words, a workspace roll-up and `projectRollup` (a project row's badge: lead, subagents and chats counted and named, 11-11), `sessionStateIndex` from the live evidence, and the Main agent's own states. Pure; drawn by `components/SessionIndicator.tsx`, fed by `store/sessionStates.ts`. |
| `sessionMove.ts` | Pure helpers for Chats and moving conversations (2026-09-30): the workspace choices the palette offers, which workspace a conversation is in, and the new workspace "New workspace from this… |
| `settingsJump.ts` | Settings' jump to a control (palette, links): `retryFind` looks again until a lazy panel's control appears, the person moves on, or 3 s pass (morning P3: a cold first jump into Usage). Pure scheduling, tested with fake timers. `openFoldsAround` opens every closed disclosure above a row (Advanced, 2026-11-12). Page-target search entries live in `settings/pageTargets.ts`. |
| `settingsResolve.ts` | The one resolver for Settings targets (PR 1, 11-12): `resolveSettingsTarget(id, label?)` returns a section (with part and label) or a page (with tab); reads the frozen ids in `settingsLegacy.json`. Pure, small enough for the main bundle. |
| `settingsIndex.ts` | The Settings registry's eager half (build run 11-17 App size pass): each section's id, label, icon, scope, parts and visible control labels, the old ids and moved controls that still resolve, and the resolvers `settingsResolve.ts` needs. In the App chunk; no search words. |
| `settingsSections.ts` | Settings section registry: the index plus every section's, part's and control's search words (`SETTINGS_KEYWORDS`) and search. Loaded only by the command palette and Settings, both lazy, so the words stay out of the App chunk. Same exports as before the split. |
| `settingsTransfer.ts` | Settings export/import (overnight F7): the file name, the export note, the import preview (old → new, what is left alone) and the result line. Pure; the dialogs are in `store/settingsTransfer.ts`. |
| `shipActions.ts` | The next step a finished reply leaves the user, offered as a suggestion in the empty composer. Merge, install and deploy offers (and agent lines that say so) always need a second Enter; install offers only in a Hollow build workspace (P0-01). |
| `shortcuts.ts` | The one keymap. Command labels and the palette's key chips come from `actionDefs.ts`. |
| `skillPortability.ts` | NV3: words for the Skills page's per-agent portability table (`skill_portability_table`): effect labels, version lines ("not verified on this version"), findings per channel, the one-line summary. |
| `skillCatalog.ts` | One Skills catalog (audit H13). `list_skills` returns every installation, including the same name from several sources (skills.rs never hides a duplicate). NV1: `entryLint`, `lintBadge` and `lintFindingsInOrder` for the lint badge and the Skill check list. |
| `skillImport.ts` | Skill import (Skills page): turning a picked path or a drop into the `preview_skill_import` source. |
| `sourceCatalog.ts` | One vocabulary for shared material and workspace attachment. No connector reads here. |
| `sourceCoverage.ts` | Connected-source coverage summary (audit H03, next-level P07). The scan reports structured coverage (directory_connections.rs `Coverage`) alongside its warning strings. |
| `sourceAuthority.ts` | What a file is and how current it is, in plain words, summary first (plan L3 X3, 2026-10-14; review U library): a snapshot, a live project file, a record tracked in the task tracker, a Result or a Library note (`sourceStateLine`), plus the path helpers the Sources cards, Library reader, palette and file toolbar share. Pure. |
| `sourcesModel.ts` | SOURCES.md and the workspace's source listing, read for the Sources cards (Canvas C5b, plan §6). Pure, no DOM. |
| `stableResource.ts` | Keep the last resolved value visible while a Solid resource refetches, and never throw once errored. Since X21 (2026-10-14) a wrapped read carries `refreshFailed`: its last refresh failed while an earlier value is still on screen (cleared by the next successful read), which `components/RefreshNotice.tsx` shows. Where each wrapped list says so: `workflowsList` above the Automations list, `skillsList` above Library → Skills, `projectsData` under the task editor's Project field. Not marked, because no surface shows them: `vaultItemsData`, `activityLedger` and `vaultGraphData` have no reader and nothing enables them (their pages were consolidated away; run 15 P4 deleted the people, goals, goal-alignment and git-activity feeds that were in the same state); `vaultTree` is read only by the command palette's Notes group, which lists nothing because nothing enables the tree either. `savedOrRead(read)` (H6 follow-up) is the fetcher for a resource a view also saves: save with `refetch(saved)` rather than `mutate(saved)`, so the save is the newest load and a re-read still in flight can't put the older value back (Learning suggestions, Extraction model, Retitle new chats). Unit-tested. |
| `store.ts` | The store barrel: re-exports `store/runtime` and the lazy resources, the state every component reads. |
| `store/fonts.ts`, `store/viewPrefs.ts` | Per-machine view preferences (t3code parity 4.7): the interface and code fonts, and the diff view's wrap and line numbers (root data attributes read by `styles/build-changes.css`) with the web-link target (Preview pane or the default browser); loaded by App.tsx so they apply at launch. |
| `streakAction.ts` | The workflow failure-streak card's action URL (`workflow-failures:<slug>`); split from `workflowHealth.ts` so the launch path carries only this (overnight P3). |
| `streamCaret.ts` | Whether a chat turn shows the streaming caret (frontier F5). |
| `studio.ts` | Exports `WORKSPACE_STARTERS`, `WorkspaceStarter`, `WorkspaceSort`, `latestWorkspaceSession`, `orderWorkspaces`… |
| `syncPass.ts` | A finished background tracker sync pass (moved here from the removed `notionSyncPass.ts`, 2026-11-10): `passFromEvent` reads the `tracker-sync-updated` payload (`{ok}`) defensively; `store/runtime.ts` keeps the latest as `trackerSyncPass` for Tasks, an issue, a project and Settings' Task tracker panel. Pure, unit-tested. |
| `syncClashes.ts` | Context sync clash cards (CX6c): one card's words (title, root, "N clashes", the line-count summary, each side's caption), which choice is blocked and why (Keep theirs on a file changed since the clash; both keeps on a folder clash), and the confirm for a choice that moves a file. Pure, unit-tested; rendered by `components/SyncClashesDialog.tsx` (lazy; `ClashCard` is its card) and the Personal and Vault rows' "N clashes" link in `settings/ContextLayersPanel.tsx`; `store/syncClashes.ts` opens it. |
| `syncClashesApi.ts` | The clash cards' IPC: `sync_clashes`, `resolve_sync_clash` (Rust `sync_clash.rs`). Its own module so the App chunk carries none of it. |
| `tablePreview.ts` | Bounded table previews (next-level P08). Parses CSV/TSV completely, so the row count and numeric column totals cover every row (each cell visited once), then renders only a window of them: at most 100 columns and 200,000 cells (`tableWindow`, `drawnRow`; run 15 FIX-FINAL-P). |
| `taskBulk.ts` | A task's origin workspace and the Work on this prompt (once next-level P09's bulk changes; the bulk edits went with the old Tasks list, overhaul B1). Pure, unit-tested. |
| `taskEdit.ts` | Pure TaskModal edit model: the prefill snapshot taken when the sheet opens and the changed-field diff sent on save. |
| `taskLocks.ts` | Which task fields a linked tracker issue owns (`lockedTaskFields`, `dueTimePart`); split from `taskEdit.ts` to keep the editor out of the App chunk (overnight P3). Today's Due actions use the same status ownership. |
| `taskOrigin.ts` | Where a task came from, in words, and where that leads (consolidation G, 2026-10-13): its stored `origin:` (a workspace plan, a workspace, a recording, Incoming, a commitment, an agent run) as "An item in “Q3 review”’s plan" with Open the plan / Recordings / Incoming / Commitments. `components/TaskOrigin.tsx` shows it in the task detail and editor. Since plan completion item 4 the link opens the exact item where the origin names one (the plan row, the recording, the promise, the run), says how task and source relate, and `planItemState` tells where a plan task's item stands. Pure, unit-tested. |
| `taskQuery.ts` | Pure task filtering and ranking — extracted from store.ts and the pages so it can be unit tested (and reused by every surface that lists tasks) without booting the store's module-init fetches. |
| `taskStatus.ts` | Task status vocabulary, pure: `isCompletionStatus` (dragdrop.ts re-exports it), for helpers that must not pull in the store. |
| `taskStructure.ts` | Pure task-structure helpers (Paperclip PR 11). Dependency-free so the node vitest suite can load them. |
| `textMerge.ts` | Line diffs and a three-way merge for file conflicts (next-level P08). |
| `toastPlacement.ts` | Connected full-view notice regions in DOM stacking order; global toast stacks use the uppermost region. |
| `toastPolicy.ts` | Quieter toasts (Paperclip PR 22). Pure, so the node vitest suite can load it. |
| `dictation.ts` | Push-to-talk (overnight 10-11 B7), pure: the anchor that writes dictated words into a field (`beginAnchor`, `applyTranscript`, stops once the person edits around them), hold-and-release (`endsHold`), the mic's tooltip (`statusTitle`). |
| `triage.ts` | The triage inbox (overnight 10-11 B6), pure: rows from Waiting on you, recordings and `triage_arrivals` (`buildTriage`), filters and day groups, what e/a/t/x do to each kind (`triageActions`), the task a row makes (`taskFrom`), the keys (`triageKey`). |
| `weeklyReview.ts` | The weekly review: ISO week stepping, labels, prompt timing and target week, money/bar scaling, facts digest and quiet-week test. `store/weeklyReview.ts` holds its view state (the screen was cut overnight 11-08 S3). |
| `commitmentAwake.ts` | Whether a commitment is snoozed or awake (overnight 10-11 A5): the two checks Today, Home and the Tasks panel need at launch, apart from the ledger's helpers so those stay in the ledger's lazy chunk. |
| `commitmentLedger.ts` | The commitments ledger (overnight 10-11 A5): snoozed/awake/overdue, counts, rows per scope/status/filter, groups, snooze choices, undo records and the view's bare-letter keys (`store/commitmentLedger.ts` routes its open/close aliases to Tasks → Commitments, `store/tasksViews.ts`). |
| `todayBrief.ts` | Today (overnight 10-11 A3): the greeting, today's meetings, due/overdue buckets, commitments due, the facts digest the brief is written from and Home's counts line (src-tauri/src/today_brief.rs writes the paragraph). |
| `toolDescribe.ts` | One-line descriptions of agent tool calls: the icon, verb and target a chat row shows ("Read …/components/ToolLedger.tsx"), and the turn-level summary a ledger header shows ("Ran 2 commands, read… |
| `toolDiff.ts` | Line diffs of agent edits: the red/green body an Edit row expands to, and the `+a −d` counts its header shows. Pure, so the node vitest suite can load it. |
| `toolFolding.ts` | Tool-call folding and the Raw view's window math (Paperclip PR 21). Pure: no Solid, no store, so the node vitest suite can load it. |
| `turnFooter.ts` | The footer under a finished agent reply (Conductor-style): how long the run took and when it finished, then the files it changed as `name +a −d` chips; `chipTarget` sends a chip for a workspace `outputs/` Result to Results, anything else to Files. Save as note's title, file name and folder (`replyNoteTitle`, `replyNotePath`: a workspace's `outputs/answers/`, else `library/`, `-2`… for a taken name) and Markdown (`replyNoteBody`); `store/replyNote.ts` writes it (One Ask, 2026-11-03). |
| `types.ts` | Hand-written IPC types shared with the Rust side (generated ones are in `generated/ipc.ts`). |
| `ultracode.ts` | Merged plan Track U: the per-chat Ultracode switch's wording, and since P1-10 where each conversation keeps it (`chatUltracodeStorageKey`, `parseStoredUltracode`). |
| `usageInstruments.ts` | Ledger-bucket series, gap-preserving stacked geometry, coverage labels and the native V1 gauge model. |
| `usageFormat.ts` | Usage labels (Paperclip PR 6, gate D2). Pure: no store or Tauri import, so the node vitest suite loads it and PR 8's budgets reuse it. |
| `utils.ts` | Small shared helpers: dates (`localDateKey`, `dueLabel`), titles, times, `taskErrorText`, opening external links. |
| `valueLedger.ts` | AI value ledger v1 (master plan #6): the shapes `value_summary` and `get_baselines` return, and how Settings → Usage → Value words them. |
| `vaultKinds.ts` | Frontend metadata for the semantic vault index. |
| `viewTransitions.ts` | Overnight 10-11 B1: page jumps inside the View Transitions API (`withViewTransition`, `morphFrom`), with the motion gates, no nesting and a 300 ms start guard so navigation never stalls. Node-tested with a fake document. |
| `visualExplain.ts` | "Explain visually" in a reply's ⋯ menu: the request it drafts into the composer. Whiteboards were removed on 2026-10-07; it asks for a canvas whose structure is an `hc-flow` diagram. |
| `waitingOnYou.ts` | The one "waiting on you" surface (merged plan wave 3, decision 7). Pure, unit-tested in waitingOnYou.test.ts. |
| `windowLabels.ts` | The saved automation-window labels (overnight S10) as a signal, written when Settings, the Automations page (on open) or the COO's queue line reads `automation_windows`, so `describeSchedule` names edited and custom windows as saved. Tiny, so the main-bundle store can write it. |
| `workState.ts` | Shared state vocabulary (next-level P01/P04/P05/P20). One word per state, the same everywhere: the rail, a conversation, Results, Work and a build's delivery record. |
| `workflowEnds.ts` | Automation end rules (G2): the list's "ends Fri 6 PM" text, the end fields' datetime-local conversions, `endsForSave` (only what changed) and the problem line. Pure. |
| `workflowForm.ts` | The automation editor's form state (merged plan slice 4, review A·D3). |
| `workflowHealth.ts` | Pure helpers for failure-streak cards (Paperclip PR 9, 9a-2): Automations → Failing automations. No store or Tauri import, so the node vitest suite can load it. |
| `workflowPermissions.ts` | Pure helpers for automation permission reviews (Paperclip PR 5). No store or Tauri import, so the node vitest suite can load it. |
| `workflowPreview.ts` | The configuration preview (next-level P17): what a run of this automation would do, computed locally from data Hollow already has. |
| `workflowSchedule.ts` | Pure helpers for workflow schedules — the structured picker's model over the stored 7-field cron format (sec min hr day-of-month month day-of-week year; hours are LOCAL wall-clock, evaluated… |
| `workspaceContent.ts` | Exports `WORKSPACE_REVIEW_KINDS`, `WORKSPACE_REVIEW_SOURCES`, `WorkspaceReviewItem`, `SuggestionContext`, `SUGGESTION_CONTEXTS`… |
| `workspaceOrigin.ts` | Where a workspace comes from, in words (local folder, legacy Notion folder, build worktree), and which titles are shared so need it (consolidation E, 2026-10-13). The rail, Home's cards and the palette use it, so a same-named workspace reads the same everywhere. Pure, unit-tested. |
| `workspaceMenu.ts` | Menus for workspaces and their conversations. One builder so the rail tree and the Work cards offer the same actions; the right-click and the visible "⋯" (`components/MoreButton.tsx`, consolidation D) open the same items. One conversation menu (`conversationMenuItems`, P1-39) for rail rows (`sessionMenuItems`), Conversation history rows and the open conversation's title (`historyMenuItems`); a disabled item says why (`conversationBusy`). |
| `archiveOlderEntries.ts` | Archive older entries… (V5a): the `SharedDocArchive` shape of `workspaces.archive_shared_docs` and the confirm dialog's text for WORKSPACE.md and PLAN.md (sizes before and after, first lines that move); the hash from the dry run binds each apply. |
| `workspaceMerge.ts` | Merge into… (master plan #18): which workspaces can receive a merge, and the confirm dialog's text listing every file and conversation that moves. |
| `workspaceRecap.ts` | What remains of the removed Overview's model (COO rebuild L4): `CONTEXT_DOCS`, `CONTEXT_BUDGET` and `contextHealthFromSizes` (the browser mock's context health), re-exporting `isResultBookkeeping`. |
| `workspaceShell.ts` | The tiny `isResultBookkeeping` filter (a saved canvas's record, dot folders, archive and old curated-view files are not Results), re-exported by workspaceRecap. |
| `workspaceTypes.ts` | Generated from the Rust structs (src/generated/ipc.ts, merged plan slice 6); re-exported so existing imports keep working. |
| `experimentalVisuals.ts` | Shared static-texture / ambient-motion eligibility and honest state mapping. |
| `thresholdField.ts` | Original deterministic folded/orbital glyph fields, four bounded cached variants with no runtime loop. |
| `threadNotifications.ts` | Desktop finish announcements (0.4): the pure rules for the banner and dock badge. `chatEvents` (running or queued to idle is a finish, to failed a failure, to waiting needs-you; the first snapshot announces nothing), `shouldAnnounce` (switch, subagent, the chat on screen in a focused window, 20 s per-chat gap), `bannerFor` (the chat's title for one event, counts for a burst) and `badgeCount` (waiting or failed chats plus finished unseen, capped at 99). Run by the lazy `components/ThreadNotificationCoordinator.tsx`; switch in `store/threadAnnounce.ts`. |

- `components/chat/`: ChatBody's controllers (plan L3 X17, 2026-10-14; review V06/F11), each `createX(props, deps)` called once in ChatBody: `chatScrollController.ts` (render window, jumps, scroll memory, follow, canvas focus, settle, reading anchor, in-place toggles), `slashCommands.ts` (`LOCAL_COMMANDS`, discovery, local and `claude mcp|plugin` commands), `chatSendController.ts` (preflight, send and queued dispatch, Continue, question cards, Stop, the activity line). ChatBody keeps folds, find, feedback and the JSX. `ResumeAtControl.tsx` (P1c) is a component, lazy in ChatBody and Mission Control: a paused chat's "Resume at…" disclosure (presets, a custom time, Clear time) and its set time.
- `pages/tasks/TaskDetail.tsx`: one task's detail pane, split out of `pages/Tasks.tsx` by ST11 (2026-10-14, no behaviour change): a local task's details and its edit form (the shared `TaskEditor`), the blockers editor, and Work on this (`openWithTask` and its one gate `workOnBlocked`, which `IssueDetail` also uses). The Notion-mirror form, `components/mappedTaskDraft.ts` and Move to Notion trash went with Notion sync (2026-11-10). Same lazy Tasks chunk.
- `store/navHistory.ts` (P1-42) owns the Back/Forward stack runtime.ts steps through: `visit`/`step` (pure; only identical entries coalesce), the entries' `workspaceId`/`sessionKey` on Work's page, `learnConversation` (a new conversation's first send), `launchLanded`, `quietly`, and the `bindWorkspaceNav` hook store/workspaces.ts binds to name and reopen a workspace entry.
- `store/navigationGuard.ts` owns mounted draft and destination-request identity before navigation mutations. `pages/workflows/draftGuard.ts` keeps the automation confirmation and exact form-revision checks in the lazy page.
- `pages/workflows/TriggerEditor.tsx` edits local trigger tokens; `pages/workflows/EventWhen.tsx` is an automation's When, a Schedule or an Event (F2), saved as one event token; `pages/workflows/newRequest.ts` carries one guarded Settings-to-new-automation window request without adding to the eager app store.
- `pages/workflows/RunsIn.tsx` is an automation's Runs in choice (overhaul F1): a new chat each run, or this chat; `pages/workflows/runsInChoice.ts` holds its options, hint and save rule and the conversation menu's Schedule in this chat… request (`conversationActions.ts` sets it, the lazy automations page reads it once).
- `pages/workflows/RunsOn.tsx` is an automation's Runs on choice (M4 follow-up): no owner, this Mac or a paired Mac; `pages/workflows/runsOnChoice.ts` holds the owner rule as the list and editor read it (a mirror of Rust `runs_on::decide` and `Gate::new`), the options, hints, the row note and the save rule; `pages/workflows/ownerMacs.ts` reads this Mac and the paired Macs lazily when Automations opens (`peer_machines` through `store/peerSessions.ts`, plus `phone_access_status`'s `machines`).
- `store/chatPause.ts` (P1a PR 2): paused and pausing chats by session key (`pausedChats`, `pausingChats`), fed by the `paused` chat event (`store/chatRuntime.ts` `notePauseEvent`, every route), by `agent_session_live(key)` when a conversation opens (`refreshChatPaused`) and by `probePausedRows` over the run queue (from `store/sessionStates.ts`). Imports only the API, so the chat store and the indicator store both read it. The composer's Pause (beside Stop; Pausing… while asked) and Resume (once paused), Mission Control's Resume and `/pause`, `/resume` call `chatRuntime.ts` `pauseChatRun` and `resumePausedChat`. The `paused` event's `reason` and `resumeAt` (P1b to P1d) are kept with the note (`markChatPaused`, `setPausedResumeAt`).
- `store/chatPauseActions.ts` (P1b, P1c): Pause all (`pauseAllChats`, every asked chat shows Pausing), Resume all (`resumeAllChats`), `setChatResumeAt` and `refreshPausedChats` (the backend's `chat_paused_list` is the truth: note, reason, resume time). Apart from `store/chatPause.ts` so only Mission Control and `ResumeAtControl.tsx` load it, not the App chunk.
- `store/runtime.ts` and its domain controllers (plan L3 X19, 2026-10-14; review V06/F11/AR): runtime.ts keeps navigation, the sidebars and resizable widths, the lazy shared resources, startup warm-up, vault note requests, live vault updates, notifications and the settings switches, page context, and re-exports (`export *`) the modules moved out of it verbatim, so every `store/runtime` import still works: `store/chatRuntime.ts` (the on-screen conversation's engine: agent settings and permission modes, chat state and persistence, queued turns, question cards, transcript reducers, the conversation's files for the Preview pane, the `chat-event` handler and listener, conversations running in the background), `store/chatApprovals.ts` (inline Allow/Deny cards and the `approval-*` listeners), `store/overlays.ts` (context menu, confirm modal, command palette, shortcuts card), `store/snackbars.ts` (undo snackbars, `flashFooter`, `flashRetry`), `store/activityDock.ts` (`startTask`/`trackTask`…) and `store/plaud.ts` (Plaud connector health). chatRuntime.ts and chatApprovals.ts import a few runtime.ts bindings back, read only inside functions and callbacks (never at module top level), so the import cycle cannot hit a TDZ at launch; the header of chatRuntime.ts says why. The other four import nothing from runtime.ts.
- `components/SourceRefresh.tsx`: lazy; Check for changes on a snapshot, the changed lines, Replace snapshot / Keep this snapshot, and Check again when the original or snapshot moved on after the check (plan L3 X5).
- `components/ExpandedCanvas.tsx`: lazy Visuals canvas scroll viewport, heading anchor, keyboard return and cleanup; reuses `canvasFocus.ts`.
- `components/ReplyMarkdown.tsx`: lazy reply-only local thumbnails, OS-home file links, blocked-scheme feedback and explicit bulk remote-image loading, and (11-07) each table in a scrolling box with a Copy table button (`replyTable.ts`). Keeps reads and DOM decoration scoped to the current conversation; `MarkdownView` remains the shared sanitizer.
- `components/CodeBlock.tsx`: a fenced block's language label, copy, fold past 24 lines, and (11-07) the **wrap** toggle, one setting for every block in `localStorage` `hollow.codeWrap` (kept here, not in `store/prefs.ts`, to spare the launch chunk).
- `components/PlanWindows.tsx`: provider-neutral plan rows with percent used, used-fill and reset text, shared by ChatUsageMenu and the lazy Settings Usage surface.
- `settings/ProviderQuotaCard.tsx`: passive Codex plan snapshot and Refresh control; failed reads stay distinct from missing limits.
- `settings/leadText.ts`: `splitLead` cuts a Settings help text over 150 characters after its first sentence (overnight 11-07, D3); `LeadText` and `MoreFold` in `settings/settingsLayout.tsx` show that lead with a visible More for the rest, which stays in the page. Pure, unit-tested.
- `canvas/CaptureHost.tsx`: storeless offscreen export page; shared `canvas/artifactStyles.ts`, validated baked palette, bridge theme/state and settled capture handshake. `components/ExportCanvas.tsx` loads on demand from `store/canvasExport.ts`; `canvas/canvasExport.ts` holds the pure request/response and naming contract. Lazy `canvas/exportLook.ts` reads fresh app/custom palette snapshots for file exports and correction pictures. See [Canvas export](CANVAS.md#save-a-canvas-as-image-or-pdf).
- `components/CanvasItemsDialog.tsx` loads from `store/canvasItems.ts`; `canvas/canvasItems.ts` extracts bounded plain rows and stable task keys. `canvas/itemsApi.ts` keeps its calls lazy through the shared tallied `api.ts` invoke wrapper. The host previews and confirms copies from the saved current version; see [Canvas item copies](CANVAS.md#copy-canvas-items).

- `components/RefreshNotice.tsx`: "Couldn’t refresh — showing what was here before." with Retry, shown only while a `preserveDuringRefetch` read's `refreshFailed` is true (X21). Same look as Home's "Not refreshed" row; its role is `status`, named "Couldn’t refresh <what>".
- `components/ConfirmFallback.tsx`: cancel-only lazy-dialog loading surface and reactive failure cancellation, with keyboard/focus cleanup.


Priority-paused reports use the existing automation and queue surfaces: shared
`runOutcome.ts` maps Paused without claiming success, `runQueue.ts` recognizes
restricted continuations as stoppable automation work, and Automation options
always offers Stop pending work through the existing cancellation owner. `runReview.ts`
selects the latest paused attempt and binds outcome, cost, check badges and
failed-check guidance to that exact run rather than an older retained receipt.

