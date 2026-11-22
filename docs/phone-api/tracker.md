# Task tracker extensions

The existing [task protocol](tasks.md) remains supported. These fields are
optional, so the installed companion and older local-task fixtures keep
working. [Portable fixtures](tracker-fixtures.json) show the optional row
fields and the retained legacy status request. They contain synthetic data.
Tracker project/create/archive actions are separate from this task protocol.

`GET /v1/tasks` rows may include:

| Field | Meaning |
|---|---|
| `statusType` | `backlog`, `unstarted`, `started`, `completed` or `canceled`. Done/open grouping uses this type when present. |
| `trackerKey` | Provider issue key. |
| `trackerUrl` | Provider issue URL. |
| `trackerLinked` | True for a tracker mirror; absent means false. |
| `notionLinked` | Compatibility alias, true for either a tracker mirror or a retained legacy Notion link. |
| `sourceLabel` | Where the task lives, the words the desktop shows (H4b, `sources::label`): `kind` (`tracked`, `stored`, `readOnly`, `unreachable`), `text` ("Tracked in Linear", "Stored in Hollow", "Linear unreachable since 9:40 AM"), and `url`, `openLabel`, `since`, `wasTracked`, `wasTrackedUrl` when they apply. Absent when Hollow can't read its settings. |

`priority` retains P1 to P4 on the wire. Tracker priorities 1 urgent, 2 high,
3 medium and 4 low map to those exact levels. An unprioritized issue has no
priority. The legacy local-task priority parser stays unchanged.

`GET /v1/tasks/statuses` requires the same paired-device authentication as
the task list. It returns `{statuses, tracker}` from the validated cached
team metadata. Without readable metadata it returns the old three local
choices and `tracker: false`; no network request or credential read occurs.

Task status/update requests keep `expectedHash`. Tracker writes require it
and return TASK_CONFLICT for stale snapshots. Exact scoped status ids or
names win. Old status words map by kind only when that kind has one unique
choice; an ambiguous mapping returns TASK_INVALID rather than guessing.
The normal tracker renderer and outbox handle delivery while offline.
Provider comments, description blocks and relations are not local task
checklists. New companion and agent edits preserve those blocks.

Confidential workspace documents and records remain outside this task
protocol. This extension does not change their phone access rules.
