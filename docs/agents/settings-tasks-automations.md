# Settings, Tasks and Automations

Part of the agent manual; [AGENTS.md](../../AGENTS.md) is the entry point.

## Settings

- Settings has seven sections and search. Each section shows one save line (Saves at once, Needs Save, or Needs a restart). Old section ids, aliases, Save handlers and dirty guards stay. See [Settings sections](../reference/surfaces.md#settings-sections-p1-22-d4-s2).
- A form with a local draft registers it (`registerDirty`) so a section switch asks first; every indexed control must be reachable by its jump ([Settings shell](../reference/surfaces.md#settings-shell-switching-unsaved-drafts-search-p1-21)). A new control gets its label in `settingsIndex.ts` and its words in `SETTINGS_KEYWORDS`.
- Settings → Updates owns update preferences, connection decisions, wait cancellation and result acknowledgment. Shared update listeners release only after every subscriber stops.
- Cap meters pair spend with its own period's limit; incomplete and zero-cap cases stay worded, without a numeric meter.

## Tasks

- Tasks and their sync: [Tasks](../TASKS_WORKSPACE.md). Linked tasks and projects mirror Linear ([Tracker sync](../TRACKER_SYNC.md)); Notion sync was removed on 2026-11-10.
- Rows keep URL identity through status/hash replacements; read current payloads through the URL map. Motion never holds a row or delays a write.
- Task sheets and mapped editors share per-task session drafts; stale saves use per-field review and keep CAS.
- The Commitments tab is the only ledger home.

## Automations

- An empty list never hides an active output or editor; drafts stay visible after an outside deletion.
- Drafts guard navigation before destination state changes, with a full form revision check ([Automation drafts](../reference/automation.md#automation-drafts)).
- The editor's trigger edits go through the existing stamped save; omitted trigger inputs stay backward compatible and unknown tokens are kept until explicitly removed ([window and trigger editor](../reference/automation.md#window-and-trigger-editor)).
- Use the host's `nightPlan` interval; never reconstruct its time-zone boundaries in the browser. The clock plots civil hours in the window settings' explicit time zone ([Background work clock](../reference/automation.md#background-work-clock)).
- Active runs derive from the queue as well as events; Stop run is in both views ([run state](../reference/automation.md#automation-run-state-and-last-night)).
- Tonight and Runbooks are removed; [docs/RUNBOOKS.md](../RUNBOOKS.md) says what replaced them.

## Connectors, calendar and companion

- Settings → Connectors → Services is the setup home for Calendar and GitHub. GitHub fills an unsaved read-only preset with relevant-only sharing, automations and automatic approval off. See [connectors](../reference/connectors.md).
- Calendar keeps macOS 11 compatibility and requests permission only from the person's source choice or Allow calendar access ([Calendar adapter constraints](../reference/architecture.md#calendar-adapter-constraints)). The test binary never touches native permission or calendar data. `agenda.calendar_source_status` is passive; source changes and opening privacy settings are user-only commands.
- Companion settings: [Companion settings](../COMPANION_SETTINGS.md).
