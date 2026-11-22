# Phone API v1.2: tasks

2026-11-07 (overnight lane I3). Four routes for the iPhone app's Tasks tab: list the tasks, make one, change its
status (complete or reopen it) and change a few of its fields (reschedule it). The same file lives in both
repositories (`docs/phone-api/tasks.md`), with the same fixtures in `docs/phone-api-fixtures/tasks/`. Change it only
in both at once. v1 and v1.1 are unchanged.

Every route needs the bearer token and passes the same Host and Tailscale checks as v1 (Who may call, and Deviations
2 in [../PHONE_CHAT_API.md](../PHONE_CHAT_API.md)). A Hollow without these routes answers 404 `not_found`; the app
then shows "Update Hollow on your Mac to use this".

**Writes are the person's own edits, made through Hollow's task writer.** The three write routes call the same
function a paired companion device's op uses (`task_broker::dispatch_companion`), so a task made or changed from the
iPhone is validated like any other (dates, priorities, one-line titles, lengths), goes through the tracker's rules
for a task linked to a tracker issue ([tracker extensions](tracker.md)) and takes the vault's short companion write lease (it never lands inside an automated
run's History capture). A task made here has the origin `companion:<the iPhone's device id>`. There is **no delete**
and nothing that touches a project, a blocker or a body checklist. A task is a file `tasks/<name>.md`; `ref` is its path as
`vault:tasks/<name>.md`, and only a real file under `tasks/` is ever named (no link, no `..`, no hidden part).

## The task row

Every route that sends tasks sends this shape (fixture `tasks/list-open.json`):

```json
{
  "ref": "vault:tasks/send-the-report.md",
  "title": "Send the report",
  "status": "Not started",
  "done": false,
  "priority": "P2",
  "due": "2026-11-09",
  "dueTime": "14:30",
  "bucket": "thisweek",
  "daysUntilDue": 2,
  "project": "Atlas 2.0",
  "labels": ["work"],
  "recurrence": "every friday",
  "description": "The quarterly numbers for Jordan.",
  "descriptionTruncated": false,
  "subtasks": [{"text": "Draft", "done": true}, {"text": "Send", "done": false}],
  "subtasksTotal": 2,
  "blocked": false,
  "contentHash": "5f0d…"
}
```

- `ref`: the task's identity for the other routes.
- `status`: the status text as stored (Hollow's own are `Not started`, `In progress`, `Done`; a tracker issue or a
  task imported from Notion can carry other names). `done` is true for a finished or dropped status (`Done`, `Delivered`,
  `Complete`, `Completed`, `Cancelled`, `Canceled`, `Won't do` or `Wont do`, in any casing): the Mac's own rules for
  finished and for cancelled, together.
- `priority`: `P1` to `P4`, or null for none.
- `due`: `YYYY-MM-DD`, or null; `dueTime`: `HH:MM`, or null (a due that carries a time, as a Notion import stores it, is split
  into the two; the task's own `due_time` wins when both name one, and a time with no valid date reads as none).
  `daysUntilDue`: whole days from the Mac's `today` (negative when late), null without a date. Every write takes the
  same spelling: a due as ten characters `YYYY-MM-DD` and a time as `HH:MM` with two-digit hours (`2026-1-5` and `9:30`
  are 400).
- `bucket`: where the Mac's Tasks page groups it, by the Mac's local date and the date alone (a time of day never
  moves a task between buckets): `done` for a finished or dropped task (a Cancelled task is `done` here), else
  `overdue` (before today), `today`, `thisweek` (one to seven days ahead), `later`, or `undated`.
- `project`: the project's name, or null. `labels`: at most 20. `recurrence`: a phrase such as `every weekday`, or
  null; **finishing a repeating task does not finish it**, Hollow moves its due date to the next occurrence and the
  status goes back to `Not started` (the status route says so in `rolledForwardTo`).
- `description`: the task's text, cut to 1,000 characters (`descriptionTruncated` says so), or null when empty.
- `subtasks`: the body's checklist items, at most 50, each cut to 200 characters; `subtasksTotal` counts them all.
  The phone shows them and never changes them.
- `blocked`: true while a task it waits on (a blocker) has not been finished; a blocker that was cancelled or no
  longer exists still counts, as it does on the Mac.
- `contentHash`: sha256 of the task file as read. **Send it back as `expectedHash`** on every write: Hollow refuses
  the write when the file has changed since (see 409 `task_changed`).

## `GET /v1/tasks[?state=open|done|all][&limit=<n>]`

- `state` (default `open`): `open` is every task that is not finished or dropped; `done` is the finished and dropped
  ones, newest change first; `all` is both (open first). Anything else is 400 `bad_request`.
- `limit`: 1 to 500; the default is 500 for `open` and `all`, 30 for `done` (`done` is never more than 100: a larger
  `limit` is 400). 400 `bad_request` otherwise.

200 (fixtures `tasks/list-open.json`, `tasks/list-done.json`):

```json
{"today": "2026-11-07", "tasks": [Row], "total": 62, "hasMore": false}
```

- `today`: the Mac's local date, which the buckets use. A phone in another time zone should say so rather than
  regroup the tasks.
- `tasks`: open tasks come ordered by `bucket` (overdue, today, thisweek, later, undated), then `due`, `dueTime`
  (none last), priority (`P1` first, none last) and title; done tasks newest change first.
- `total`: how many tasks matched; `hasMore` is true when `total` is more than the rows sent. There is no offset:
  the open list stops at 500 rows and the done list at 100, so a vault with more shows the first of them and says so.
- Only a real file under a real `tasks/` folder is listed, and only one a write could name (no hidden part, no
  backslash, a ref of at most 400 characters). A `tasks/` that is a link lists nothing and every write is 400.

## `POST /v1/tasks`

Body (fixture `tasks/create-request.json`): `idempotencyKey` (required, 8 to 64 characters of letters, digits,
`-`, `_` and `.`: the app makes one when the person starts a task and **reuses it for every retry**), `title`
(required, one line, at most 240 characters; runs of spaces fold to one), and optionally `due`, `dueTime`,
`priority`, `description` (at most 20,000 characters), `labels` (at most 20, duplicates ignoring case dropped),
`recurrence` (a phrase such as `every weekday`, at most 60 characters). The one-line fields (title, labels,
recurrence, status, ref) refuse control characters and line or paragraph separators. Unknown fields are a 400.

200 (fixture `tasks/created.json`): `{"task": Row, "disposition": "applied"}`. A repeated `idempotencyKey` from the
same iPhone makes no second task: the answer is the same, with `"disposition": "duplicate"` and the task as it is
now (even when it has been renamed or finished since, and even when the repeat carries another title: the key is
the task's identity). Two iPhones never share keys. One iPhone may make at most 200 new tasks in 24 hours (counted
while Hollow runs); past that a new key is a 400 and a repeated key still answers `duplicate`.

## `POST /v1/tasks/status`

Body (fixture `tasks/status-request.json`): `ref`, `status` (one line, at most 60 characters, any text: the app sends
`Done` to complete and `Not started` to reopen; a tracker-linked task takes its team's statuses, see
[tracker extensions](tracker.md))
and `expectedHash`. A call whose answer was lost and is sent again with the same hash is a 409 `task_changed`, because
the first call changed the file: the app lists again and sees the new state.

200 (fixture `tasks/status-changed.json`): `{"task": Row, "disposition": "applied", "rolledForwardTo": null}`.
`rolledForwardTo` is the new due date when the task repeats and finishing it moved it instead (then `task.status` is
`Not started`). A task linked to a tracker issue takes the status through the tracker's rules
([tracker extensions](tracker.md)); when the link cannot take it the answer is 400 with the reason. A task imported
from Notion (a legacy `notion_id`) is a plain vault task since Notion two-way sync was removed on 2026-11-10.

## `POST /v1/tasks/update`

Body (fixture `tasks/update-request.json`): `ref`, `expectedHash` and `fields`, an object of at least one of
`title`, `due`, `dueTime`, `priority`. An empty string or null clears `due`, `dueTime` and `priority`; a title cannot
be cleared. A date and its time of day go together: clearing the time of a task whose `due` carries one (a Notion import's
`2026-11-09T14:30`) also trims `due` to its date, and clearing the date also clears a stored time. A retry after a
lost answer is a 409 `task_changed`, as for the status route. Any other field is a 400 (the status has its route; the description, projects, labels, blockers and the
checklist are the Mac's).

200 (fixture `tasks/updated.json`): `{"task": Row, "disposition": "applied"}`.

## Errors

The error body is v1's (`{"error": {"code", "message"}}`, one plain sentence the app may show as it is). Nothing is
written on any of them.

| Status | `code` | When |
|---|---|---|
| 400 | `bad_request` | Missing or malformed body, an unknown field, a bad date, priority, title or key, a field a tracker issue holds that cannot be changed from here, a malformed `ref`, a `tasks/` folder that is a link, or the day's 200 new tasks used. |
| 404 | `not_found` | No task at `ref` (moved or deleted since the list was read). The app lists again. |
| 409 | `task_changed` | `expectedHash` is not the file's hash any more: the task changed on the Mac, from the tracker or from another surface. The app lists again and lets the person redo it. |
| 409 | `busy` | Hollow is busy for a moment (an automated run holds the vault, or tracker sync is mid-pass). Safe to retry with the same body. |

## Notes for the client

- Make the `idempotencyKey` when the person opens the new-task sheet, keep it with the unsent task (an offline or
  failed send retries with it), and drop it after a 200. A 409 `busy` and a network error are retries; a 400 is not.
- Show the task as soon as the 200 arrives, from `task` in the answer, and refresh the list in the background.
- Completing a task: send `Done` with the row's `contentHash`; keep the row's old `status` and the answer's new
  `contentHash` for an Undo (a status call that sends the old status back). A repeating task shows its new due date.
- Every response carries `Cache-Control: no-store`, as in v1. Nothing here is a stream.
