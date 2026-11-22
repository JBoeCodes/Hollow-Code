# As-of history (master plan F5)

`src-tauri/src/asof.rs`. What a workspace's brief, plan, tasks and decisions
said on a given day.

- **Nightly snapshot** (window job `as-of-snapshot` on `nightly`, run once on
  wake): hashes each workspace's `WORKSPACE.md` and `PLAN.md`, `tasks/*.md` and
  `decisions/*.md` (≤ 2 MiB each), stores new contents in History's object
  store (`.hollow-history/objects`) and writes
  `.hollow-history/snapshots/<YYYY-MM-DD>.json` (`{day, takenAt, files:
  {path: hash}}`). A day identical to the latest snapshot writes nothing.
- **Which day**: a snapshot taken before 06:00 (the nightly window) stands for
  the end of the previous day, and is named for it; the night's metrics are
  that day's, and count files changed only when the pass took a new snapshot.
- **Retention**: snapshots older than 90 days are pruned, the newest always
  kept. History's object GC (`enforce_retention`) counts snapshot references,
  so it never deletes their content, and skips collecting entirely when a
  snapshot manifest can't be read; History's 512 MB ceiling still counts
  runs only (snapshot growth is bounded by the 90 days and content dedup).
- **`asof::read(path, day)`** (`history.read_as_of`): as of the end of that
  local day, the newer of the newest snapshot then and History's newest
  agent-run version finished by then. `source`: `snapshot`, `history`,
  `absent` (that day's snapshot didn't hold a file of a tracked kind), or
  `not-captured` (a file snapshots don't track, or nothing
  from then: snapshots start at install; History keeps agent runs only).
  Uncaptured is not known, never shown as empty.
- **Metrics** (`workspaces.metrics`): `.hollow-history/metrics/<day>.json`, per
  workspace: chat turns and known cost from the usage ledger's workspace
  column, runs with unknown cost, files changed since the previous snapshot,
  open `- [ ]` plan rows.

## The vault time machine (#17)

- **Compare with a past day…** (P1-50) went with the
  workspace panel's Brief and Plan (COO rebuild L4, 2026-11-10); earlier
  versions of a note are in History & recovery, and `history.read_as_of`
  still answers.
- **In chat**: a request that asks what was known on a past day ("what did I
  know on 9/15 about Atlas", "as of 2026-09-01"; a date with only "on" isn't
  enough) gets an
  `<as-of-context date workspace>` block beside the trusted recall
  (`asof::question_block`): the brief and plan as of then, each with its
  source note and capture date (5,000 characters each, 12,000 in all), and
  the decisions settled by then; the agent is told to cite capture dates and
  say what isn't known. The workspace is the chat's own, else the one
  workspace whose title the request names as whole words. A yearless date still ahead this
  year isn't read as a past day (it may be a due date).
- **Limits**: snapshots start when this build is installed; no backfill from
  git (the plan's question, answered conservatively: from install onwards).
