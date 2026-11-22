# Several worktrees in one build workspace (F6 part B)

Status: **built** (2026-11-18, build run 11-17 row B2, ENG-130). Designed 2026-10-20
(concurrency programme PR 7), when part A shipped: every chat turn but a build
worktree's own leases a `PORT` / `PW_PORT` of its own (see
[WORKSPACES.md](WORKSPACES.md#build-workspaces), "Ports for every turn"). The target test
`chat::tests::two_worktrees_of_one_build_workspace_run_at_once_each_alone`
(`src-tauri/src/chat_tests.rs`) runs now. **As built** (below) says where the code
differs from this design; the rest of the note is the design as written, corrected where
main had moved.

## As built (2026-11-18)

- **API names.** `workspace_for_worktree` still returns the id (its 15 callers keep
  working; lanes match too); `worktree_lane(vault, cwd)` returns `(id, lane)` and
  `worktree_build` adds that worktree's own `BuildInfo` (lane branch, folder, port). The
  lane's port rides on `BuildInfo.port` (`#[serde(skip)]`, not in the IPC type), so
  `build_slots::port_for` and `run_env` take no new argument.
- **Admission.** `automated_history::ScopeMode` is `Shared`, `Exclusive` or
  `Worktree(lock)`. `ScopeState` keeps `exclusive` beside the new `locks` map: a workspace
  archive's lease is still exclusive, and an exclusive run in `workspaces/<id>` still
  keeps worktree turns out. `try_begin_scoped_mode` is the chat path;
  `try_begin_scoped` (exclusive) stays, `try_begin_scoped_shared` is tests only.
  `blocked_by` and `scope_blocker_kind` take a `&ScopeMode`. `live_scope`'s flag means
  "holds its folder alone" (false for a worktree turn, so it gets the shared-workspace
  note); `scoped_turn_of`'s means "admitted alone on something" (folder or lock).
- **Gated on lanes** (Risks, second bullet; review of #1599): with no live lane a
  worktree turn holds the whole folder as before (`build_workspaces::turn_mode` is
  `Exclusive`), since a folder chat there is told to work in that worktree and gets its
  target. Once a lane exists every worktree turn takes `Worktree(lock)`, and the
  folder's conversations get no worktree: no build note naming one (a short note says
  not to change code in any worktree from there) and no `CARGO_TARGET_DIR`. Commit, push,
  PR, update and removal wait in the same mode. A lane added while the primary's
  exclusive turn runs waits for that turn.
- **A run keeps the mode it was admitted in** (delta review), so two more modes record
  who was given the primary worktree: `PrimaryFolder` (a folder chat admitted with no
  lane: shared, but it waits for any worktree turn in its folder) and
  `PrimaryWorktree(lock)` (the primary's turn once lanes exist: it also waits for any
  `PrimaryFolder` run there). A folder chat that adds the first lane therefore keeps the
  primary's next turn out until it ends (lanes' turns still start), and a folder chat
  that would be given the primary after the last lane goes waits for a primary turn
  admitted while it existed. `ScopeState.given_primary` holds the flag.
- **Each turn's note names its own worktree** (`workspaces::build_note`, from
  `worktree_build(cwd)`), with a line telling a lane's agent to use git there.
  `build_commit`, `build.publish`, `build_push`, `build_open_pr`,
  `build.update_from_base`, `start_app_build`, `build.run_setup_hook` and
  `build.approve_setup_hook` act on the primary and refuse a caller whose turn holds a
  lane's lock (`build_changes::refuse_from_another_worktree`).
- **Refusals.** As the table below, plus: the primary's Remove worktree waits until its
  lanes are removed, a trash refuses while one is live, and Update from base reserves in
  the primary's turn mode. Remove worktree (a lane's, and the primary's in its turn
  mode) holds a reservation for the whole removal and says a conversation is working there when one is. The store is
  written only under its write lock (`modify`), so a removal or a Runs on change never
  erases a lane added meanwhile; a lane added during the primary's removal stays bound. A "merged" settlement policy is refused in a lane's
  conversation (its pull request is read for the primary's branch only).
- **Lanes.** `bind_lane` names lanes by slug (not `main`), at most 8 live, branch default
  `<primary branch>-<lane>`, base default the primary's (OC5's stale-base refusal
  applies), path beside the primary's under the worktrees root, `node_modules` seeded as
  for create. OC15's setup hook is not run for a lane (its result is per workspace). A
  removed lane keeps its entry with `removedAt`; its name can be used again.
- **Ports.** A new primary (`create`) and every lane get a stored `port`: the hash when no
  other live worktree of the vault has it, else the next free one.
- **UI not built.** The App chunk is at its bundle budget, so lanes are reached through
  agent actions: `workspaces.add_build_worktree` (confirm; runs without a card in auto
  like `create_build_workspace`), `workspaces.build_worktrees` (read) and
  `remove_build_worktree`'s new `lane`. A conversation runs in a lane when its folder is
  the lane's worktree. The UI section below is still to do, in a lazy chunk.
- **Hygiene.** A live lane's worktree is never offered by Remove finished worktrees
  ("a build workspace's extra worktree"); `port_owners` and media folders list lanes.

## Why

The overnight lanes of 10-19/20 ran two or three agents in one Hollow workspace, each in
a `git worktree add` folder Hollow didn't know about. Every lane chat's working folder
was the workspace's vault folder, so Hollow saw several shared-folder turns: no
per-lane build folder or port, no exclusive lock on each checkout, receipts that missed
the code changes, and History that credited one lane's files to another. A build
workspace already gives one worktree all of that. It should give it to several.

## Rules

- One turn per worktree, never more (shared git index, files, `target/` and port).
- Two worktrees of one workspace run at once, each with its own `CARGO_TARGET_DIR`,
  `PORT` and `PW_PORT`.
- The workspace's own vault folder stays a shared scope, and worktree turns no longer
  keep its conversations out (today they do: see "Scope keys").
- Existing single-binding `build-workspaces.json` files keep working unchanged.

## Schema (`build-workspaces.json`, app data, outside the vault)

Today: `vaults[<vault stem>][<workspace id>] = Binding { repo, worktree, branch, base,
createdAt, removedAt? }`, one per workspace (`build_workspaces.rs`, `Store` /
`Binding`).

Add one optional field to `Binding`; no schema bump:

```json
"<workspace id>": {
  "repo": "…", "worktree": "…/app/main-lane", "branch": "hollow/x", "base": "origin/main",
  "createdAt": "…",
  "port": 1712,
  "lanes": {
    "lane-b": { "worktree": "…/app/lane-b", "branch": "hollow/x-b", "base": "origin/main",
                "createdAt": "…", "removedAt": null, "port": 1544 }
  }
}
```

- The top-level binding stays the **primary** worktree: the workspace's `cwd`, and the
  one everything that reads a single binding keeps using (commit/publish in
  `build_changes`, install in `self_install`/`hollow_repo`, gardener, night queue,
  retrieval scope, `Workspace.build` in the IPC type).
- `lanes` (name → lane, `#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]`)
  are extra worktrees **of the same repository**. A lane name is a slug unique in the
  workspace; the primary's name is `main`.
- `port` (optional, on the primary and each lane) is assigned once at bind time from
  1500-1899, skipping every other binding's port in this vault. Missing means today's
  `build_slots::port_for` hash, so a v1 file's ports don't move. This also removes the
  hash's chance of two build workspaces landing on one port (400 slots, birthday odds).
- Back-compat: a v1 file has no `lanes` and no `port` and parses as today. An older
  Hollow reading a new file ignores both fields (`Binding` doesn't deny unknown
  fields): its lane conversations then run as full-vault turns in their folder, which
  is safe, only slower. **Writing from an older build drops `lanes` and `port`** (its
  `Binding` has neither): after a rollback, any change it saves to the store (a Runs on,
  a new build workspace, a removal) leaves each lane's worktree on disk, unbound and
  with no lock, and the primary on its hashed port. Remove lane worktrees (or note them)
  before rolling back; `git worktree list` in the repository still shows them.

API: `workspace_for_worktree(vault, cwd)` returns `(id, lane)` (`lane` is `main` for the
primary) and matches lanes too; `binding_for` is unchanged (primary); a new
`lanes_for(vault, id)` lists all, primary first. `bind_lane` / `remove_lane` mirror
`create` / `remove_for` (git worktree add -b from the primary's repo; Remove keeps the
branch and asks about unsaved work the same way).

## Scope keys

The scope string can't carry the lane. It is a **vault-relative path**, and code relies
on that: `begin_scoped` requires `vault.join(scope)` to be a real folder
(`automated_history.rs`, `symlink_metadata(vault.join(scope))`), the scoped baseline and
diff use it as their root (`vault.join(scope)`, `strip_prefix(scope)`), restore and
History overlap match paths with `starts_with(scope)`, `is_notion_scope` parses it, and
every History record stores it. `workspaces/<id>#<lane>` would break all of these.

So a run gets two things instead of one:

- **History scope** (unchanged): `workspaces/<id>`, the vault folder whose files its
  diff covers. A worktree turn edits the workspace's brief, plan and outputs there,
  exactly as a folder chat does.
- **Lock** (new, only for a worktree turn): `worktree:<canonical worktree path>`,
  exclusive. `chat::run_scope_mode` returns `(scope, Mode)`, with `Mode::Shared` or
  `Mode::Worktree(lock)` in place of today's `bool`.

In the registry (`ScopeState`), `exclusive: BTreeSet<run id>` becomes
`locks: BTreeMap<run id, lock key>`. Admission (`begin_scoped`, `blocker_in`):

| Request | Blocked by |
|---|---|
| Worktree turn, lock `k` | any live run holding lock `k` |
| Folder chat, scope `s` | nothing in `s` (a run with no lock key that is exclusive, which none is after this) |

A worktree turn then counts as a **shared** run of its folder for History: what it and
a folder chat (or a second lane) both change in `workspaces/<id>` is marked "shared" by
the existing overlap marking. This is the deliberate change from today, where the build
worktree's exclusive run on `workspaces/<id>` keeps every folder chat out. It means two
lanes and a folder chat can all edit PLAN.md/WORKSPACE.md at once, so this PR lands
**after PR 2 (F2+F4 shared docs)**, which gives those files compare-and-swap writes.

`scope_holder(vault, "workspaces/<id>")` keeps meaning "any run in this workspace's
folder", worktree turns included, because they still register that scope.

## Refusals that use scope today

| Site | Today | After |
|---|---|---|
| `build_changes.rs` `refuse_while_busy` (commit, publish, git ops) | any holder of `workspaces/<id>` | the holder of that worktree's lock only (a folder chat doesn't own the git index) |
| `build_workspaces.rs` remove worktree | any holder of `workspaces/<id>` or the vault lease | the lane's lock holder or the vault lease |
| `workspaces.rs` `trash_checks` | any holder of `workspaces/<id>` | unchanged (lane turns hold that scope too) |
| `conflict_copies.rs` resolve | any holder of `workspaces/<id>` | unchanged (vault files) |
| `workspace_merge` | `scope_holders` | unchanged |
| `descendants.rs` `BEHIND_THIS_WORKTREE` (`blocked_by`) | same scope and exclusive | same lock key |
| History restore/clear (PR 3, F5) | live scope overlapping the paths | unchanged: History scope is unchanged |

## Environment (`build_slots`)

`run_env` takes the lane: `CARGO_TARGET_DIR` = `hollow-build-targets/<repo>-<hash of
lane worktree>` (`worktree_id_of` already hashes the worktree path, so a lane gets its
own with no change), `PORT`/`PW_PORT` = the lane's stored `port`. `forget_target` runs
per lane on remove. Part A already keeps a vault-folder conversation off every
worktree's port (turn ports are 1900-2299). Each lane's first build is cold: budget the
disk (a Hollow target is several GB).

## UI

- Workspace menu: **Add worktree…** (name, optional branch, base defaulting to the
  primary's base); per lane Reveal, Copy path, Copy branch, Remove worktree….
- Header: the branch chip becomes a lane picker when there is more than one; cards read
  `Build · 2 worktrees`.
- New conversation in a build workspace: which worktree (default the primary), or the
  workspace folder. The conversation's `cwd` is that folder, so nothing else changes
  per session.
- History & recovery labels a lane turn `<workspace> · <lane>`.

## Other code that must learn about lanes

- `worktree_hygiene`: it protects bound worktrees via `all_bindings`; it must list lane
  worktrees as bound, or **it would offer to remove a lane's worktree** once merged.
- `services` (per-worktree dev servers), `workspace_activity`, `run_attribution`
  (repo receipts): key by the run's own cwd, which is already the lane's folder.
- `gardener` `worktree:` line, `night_queue`, `main_agent`: primary only at first.

## Admission

Lanes are ordinary scoped turns for the caps: they count against the app-wide budget
and the per-workspace folder limit is not theirs (they run in their own folder). The
findings recommend at most 2 lanes running at once (it matches the two `hollow-slot`
slots; a third only queues on them and the browser lock). That is a cap for PR 4's
settings, not a rule here.

## Risks

- Changing `exclusive: bool` to a lock key touches every admission path (`chat_queue`,
  `run_pump` `chat_scope_mode`, `descendants`, `automated_history` `begin_scoped`,
  `blocker_in`, `blocked_by`, `scope_blocker_kind`); keep the old functions as thin
  wrappers for one release to limit the diff.
- A worktree turn becoming shared in its folder changes History for build workspaces
  that never use lanes: their folder chats can now run beside the worktree turn. If
  that is unwanted, gate it on the workspace having at least one lane.
- The registry is per process (one Hollow per vault); a second Hollow on the same vault
  sees none of it, as today.
