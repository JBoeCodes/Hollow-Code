# The Hollow folder (Projects and Vault)

Merged in [PR #107](https://github.com/example-org/hollow-code/pull/107) and installed
as 0.5.3 from the clean Pretoria merge `1d18c13` on 2026-09-24. Native Settings
and the Projects picker were verified. The existing vault remains selected;
the user can choose the Projects directory in Settings → Sources → Hollow folder (the
section once called Files & storage; Library → Connected projects' empty state
links to it).

## Setup and ownership

First-run onboarding and Settings → Sources → Hollow folder use the same storeless
`DirectorySetup` component. The user chooses ONE **Hollow folder** (default
`~/Hollow`; created if missing, its parent must exist). Inside it Hollow keeps
two folders: **`Projects/`** for repositories and project folders, and
**`Vault/`** for workspaces, plans, notes and outputs (Obsidian-compatible).
`directory_connections::connect_home` plans both slots before creating anything:
an empty slot becomes a new folder; an existing folder chosen under **Bring
existing folders** is linked in as a symlink (`Projects → /old/projects`),
never moved or copied. A different folder already at `home/Projects` or
`home/Vault` is refused, as is a Hollow folder inside a folder being brought in
or overlapping Projects/Vault. Config stores the canonical real paths
(`projectsPath`, `vaultPath`) plus `hollowHome`, so every other subsystem sees
real folders and never the links. `connect_hollow_home(homePath,
existingProjects, existingVault)` replaced the two-folder `connect_directories`
command (2026-09-26); the internal `connect(projects, vault)` remains for tests
and clears `hollowHome`. An install from before the change keeps working; its
Settings pre-fills **Bring existing folders** with the two current folders, so
one click links them into `~/Hollow` without a restart (the vault path is
unchanged).

**Prepare and connect** creates only missing `workspaces/` and `library/` folders
and new Hollow workspaces. Original project files, repositories, vault notes,
Obsidian configuration and existing workspace documents are never overwritten,
moved or imported. A conflicting file/symlink at either scaffold path fails
with an actionable error. Invalid existing config or connection metadata is
preserved and fails explicitly. Retry after fixing an error is safe; successful
additive scaffolding can remain after a later config-write failure.

The active config file is merge-written through the existing private atomic
publisher. Only `vaultPath` and `projectsPath` change. Unknown keys, legacy
Notion keys (read only by the importer), Plaud cadence and Calendar identity remain
intact. Authentication stores and connector sync behavior are unchanged; new
account connections still use Settings and normal authentication. An explicit
restart activates a changed vault because AppState's vault and its existing
watchers/schedulers are immutable for a process lifetime. Changing only Projects
within the active vault is live and needs no restart. Pending vault selection
survives reopening Settings; Change folders allows correcting it before restart. Setup itself performs
no remote authentication, sync or account mutation.

### Move Hollow home (2026-11-12)

Linking keeps the old folders where they are. **Move Hollow home** makes the
Hollow folder real: the vault moves to `home/Vault`, the Projects folder to
`home/Projects` (a child named `personal` becomes `Projects/Personal`), and,
when asked, the folders beside the vault join Projects. A second vault (a folder
with `workspaces/` and `AGENTS.md`) and the losing copy of a name found in two
roots (older content) go to `home/Archive`, never into Projects. Every old path
keeps a compat symlink to where it went. Code: `move_home*.rs`; module map in
`docs/reference/backend-module-map.md`.

- **Dry run** (`system.move_hollow_home_plan`, Settings → Sources → Hollow
  folder → Move Hollow home → Check the move, or `Hollow move-home --dry-run` from a built
  checkout with no install) changes nothing and reads no file content under a
  moved root before checking its `SF_DATALESS` flag, so iCloud-evicted files
  are counted (with their size), never downloaded by the check. It reports every move with size and file
  count and its method (rename or copy), the stores re-keyed to the new vault stem, the Sessions DB columns
  rewritten, the `local-*` ids that change, collisions, dataless files with what
  they take to download, which folders are in iCloud and how long their copies
  take, running chats and processes that block,
  `git worktree` repairs, agent stores (Claude, Codex, Gemini, Claude desktop,
  Obsidian), LaunchAgents and scripts that name a moved path, and whether a
  Time Machine backup covers the Hollow folder.
- **Run** (`system.move_hollow_home`, a card bound to the plan's hash) is
  two-process: the app writes `home/Backups/move-home-<time>/plan.json`, starts
  the same binary detached as a helper (`move-home --run`), and quits 1.5 s
  later the way a restart does, which ends the calling turn. The helper waits
  for Hollow to exit and for the Sessions DB to have no other connection, runs
  exactly that plan, verifies, and opens Hollow again. Moves stay on one volume
  (a cross-volume plan is refused). A folder outside iCloud is `rename(2)`d
  after a one-file rename probe; a folder in iCloud is copied (next bullet).
- **Folders in iCloud (Desktop & Documents, a File Provider domain) move by
  copy, not rename** (2026-11-12, PER-24). The first real run renamed the
  vault out of `~/Documents` for 8 min 51 s and failed with ESTALE (os error
  70): a rename out of the domain makes the File Provider daemon process the
  whole subtree. Measured in scratch folders: 2,000 and 10,000 files 4 s,
  30,000 files 6 min 25 s by rename, 4 to 18 s by clone, 1.4 s to remove the
  original. So the plan marks a non-nested move whose source is in a domain and
  whose target is not as `method: copy`, and the run does: `move:N` clones the
  folder beside the target (`clonefile(2)` of the whole tree, else `cp -cRp`; no
  extra disk, hard links become separate files, folder mtimes are reset),
  compares both trees entry by entry (names, kinds, sizes, modes, file mtimes,
  link targets), renames the clone into place and marks it in the journal; every
  other step runs; `retire:N` (after `verify`, only when verify passed) removes
  the original and leaves the compat link. Rollback before `retire` removes the
  copy and touches nothing in iCloud; after it, the original is rebuilt from the
  copy (cloned back, compared) once later steps are undone. The probe clones,
  compares and removes a small tree through each iCloud source instead of
  renaming one file.
- **iCloud-only (evicted) files in a copy folder must be downloaded first.**
  Cloning or renaming one out of iCloud blocks while that one file downloads
  (3 MB took 41 to 53 s), so a copy would take days. `Hollow move-home
  --download [--with-vault-siblings]` (`move_home_icloud.rs`) reads every
  evicted file of the copy folders with 128 threads (about 12 small files a
  second, 14 to 26 MB/s per large file), can run while Hollow is open, resumes
  on a rerun, and prints progress. Until none are left the plan has a blocking
  `dataless` blocker with the count, size, estimated time and the command, and
  the probe refuses too (a file evicted again by macOS between check and run).
  Check the move shows the size and time up front.
- **Re-key**: every entry in the support and app-data folders named with the
  old vault stem is renamed to the new one; the Sessions DB is cloned, its WAL
  checkpointed, path and workspace columns rewritten, checked and swapped in.
  Path rewrites are one pass, longest prefix first, at path-segment boundaries
  only. A moved project's workspace folder is renamed to its new
  `local-<sha256(new path)[..24]>` id and every id reference in the stores is
  rewritten; history text keeps old ids and paths.
- **Append-only tables keep their ids; `workspace_aliases` maps them**
  (2026-11-13, after run 2 failed with `audit_log is append-only`). The re-key
  never writes a table a `BEFORE UPDATE ... RAISE(ABORT)` trigger guards
  (`audit_log` today, any later one too; `move_home_plan::append_only_tables`).
  Instead the same transaction that rewrites the copy appends one row per moved
  id to `workspace_aliases(old_id, new_id, move_id, at)` in the Sessions DB
  (append-only itself; a rollback restores the whole file, so nothing is ever
  deleted). The step then checks the audit hash chain reads the same as before.
  Lookup (`workspace_aliases.rs`): `Aliases::load` reads the table once per
  read, oldest row first; a row's new id stops pointing anywhere from then on,
  so a project moved back to its first path gets its first id back and the map
  never holds a cycle. `current(id)` follows the chain to today's id (A to B to
  C gives C), `ids_for(id)` is every id of the same workspace for an `IN (...)`
  filter, `same_vault_path` and `current_vault_path` match and map
  `workspaces/<id>/...` paths across a move. Readers that resolve through it:
  `audit::query` (the `tracker.audit` workspace filter; `read_action` adds
  `currentWorkspaceId` beside a moved row's recorded id),
  `cfo::budgets::project_tokens`, `usage_ledger::spend_buckets` (Settings →
  Usage by project), `run_ledger::rows_between` and its `workspace` filter
  (`usage.ledger_query`), and History & recovery (`list_runs`,
  `list_note_history`, `get_run`, `read_version`, `restore_change`,
  `restore_run`, `restore_to_versions`): a run recorded before the move shows
  and restores its files at their current path, in memory only (manifests
  keep what was recorded), and never while a folder with the old id exists.
  Known limit: if a new project later lands at a moved project's old path
  (its compat link removed), its rows read under the moved project's id. Mutable id
  columns (`usage_ledger`, `work_links`, `runbook_*`, `night_queue`) and paths
  (`sessions.cwd`, so a workspace's chats) are still rewritten. Symlinks can't
  replace the re-key: `directory_connections.rs` canonicalizes the Projects
  root and ids hash the absolute path. The plan notes how many append-only rows
  keep an old id; verify has a `workspace aliases` check.
- **Special files** (2026-11-13, after run 1 stopped on
  `card-recon/.git/fsmonitor--daemon.ipc`: `cp` can't copy a socket). The plan
  walk and the probe list every socket, pipe and device file in a copy
  folder (`move_home_icloud::special_files`). Known throwaway runtime files,
  by name (`THROWAWAY_SPECIAL_FILES`: git's `fsmonitor--daemon.ipc`), are
  dropped from the copy after the clone and skipped by every tree comparison;
  the source keeps them until the retire removes the original. Any other one
  is a blocking `specialFile` blocker in the plan, a probe refusal before any
  change, and a stop in the copy step, each naming the path.
- **Rollback** (`system.rollback_move_home`, or Roll back in the panel): every
  file the run rewrites is cloned into the backup folder first and each step is
  journaled, so a crash at any step can be undone. It is refused once the new
  vault or Sessions DB changed after the move's verify record.

### Context roots (CX3, 2026-11-13)

The context layers (vault workspace `hollow`, `outputs/context-layers-2026-11-13/plan.md`)
keep personal and organization context in two git clones outside the vault.
`context_roots.rs` says where they are and which machine this is. Its
callers are the home and org layers (`context_layers.rs`, CX4, CX5, CX5b),
the sync engine (CX6a), the memory guard and the Settings actions; while both roots are
unset, Hollow behaves as it did before them.

| What | Where | Shape |
|---|---|---|
| Clone roots | `context-roots.json` in the app data folder | `{"personal": "<absolute path>" or null, "org": "<absolute path>" or null}`; absent or null is unset. Settings will offer `~/Hollow/Context/personal` and `~/Hollow/Context/org` (`default_root`), never read as saved roots. |
| This machine | `~/.config/hollow/machine.json` (`$XDG_CONFIG_HOME/hollow/` when set), local, never synced | `{"version": 1, "slug": "<slug>"}`; unknown fields kept on write; a file that isn't valid JSON is refused, never overwritten. The slug names `home/<slug>.md` in the personal clone. |

- **Slug:** 1 to 40 lowercase ASCII letters, digits and single dashes, starting
  and ending with a letter or digit. The person sets it; `suggest_slug` offers
  one from the Mac's name (an owner's "Name's " prefix dropped). No machine
  name is a literal in code.
- **Path rules:** a root is absolute, has no `.`, `..` or empty part, and is
  never `/`, the home folder or a folder above it. It is saved resolved (the
  folders above it canonicalized once; the root itself may not be a symlink),
  so the saved path holds no symlink. The two roots may not be the same or
  nested. Both checks ignore case (APFS is case-insensitive by default and
  `canonicalize` keeps the typed case). A `context-roots.json` that doesn't
  parse reads as unset but is refused, not overwritten, by a save; `root`
  checks the save rules again, since the file can be edited by hand. At use time `check_root` refuses a root with any symlinked or
  missing part, `resolve_in_root` refuses an entry with a `.`, `..` or empty
  part or any symlink in it (a cloned repo can carry symlinks), and
  `read_in_root` opens every folder from `/` down with `O_NOFOLLOW`, `lstat`s
  the file through its folder and opens it only when it is a plain file (no
  device or FIFO is opened), then checks it is the same file and within its
  size cap. `resolve_in_root`'s answer holds only when given: read through
  `read_in_root`, and a writer needs the same walk.
- **Status read** (`status`, for the Settings rows CX17 adds): path (saved,
  else the default), configured, exists, git clone (a non-symlink `.git` and
  `git rev-parse --show-toplevel` equal to the root), last commit time (HEAD's
  committer date, ISO 8601) and a problem line. It runs only the two git reads.
- **Auto memory** (CX4b): with a personal root, local Claude turns pass
  `autoMemoryDirectory` = `<personal clone>/memory/<slug>`
  (`context_layers::auto_memory_dir`).
- **Later:** Claude Code's auto memory can point at a folder (CX0b,
  `autoMemoryDirectory`); its `memory/<project>` entry will go through
  `resolve_in_root` with its own name check (the CLI's project names start
  with `-`).

### Context layers, read side (CX4, CX5, 2026-11-14)

`context_layers.rs`. While the personal root and the machine slug are set, a
fresh session's `<operator-context>` block ends with `## home/<slug>.md`: one
lead line naming the machine and its topic folder, then the home file
(frontmatter dropped, cut to whole lines within 4,000 bytes). While the org
root is set, it ends with `## org/INDEX.md`: a lead line naming the read-only
clone (read files with Read, Grep or Glob, not Bash, as needed; never write in it), then `INDEX.md`, or
a generated list of top-level folders and Markdown files while the repo has
none, within 2,000 bytes. Every read goes through `read_in_root` (no symlink
followed); a file that doesn't read is skipped, never a failed turn.

- **Paths.** The sections ride the profile: sent once per native session, a
  changed line reaches a resumed session as the `<operator-update>` delta,
  About you off sends neither. The turn budget counts them as settled bytes
  (never cut), like Decisions and Guardrails.
- **Pull.** `spawn_pull_timer` (from `lib.rs` setup) runs `git pull --ff-only
  --quiet` in the org clone 30 s after launch and every 30 minutes, with no
  prompt of any kind (ssh in batch mode), the clone's hooks and fsmonitor off,
  in its own process group killed at a 60 s timeout, on its own thread. It
  never clones or pushes, and never touches the personal clone.
- **Delta.** The baseline holds what the digest sent (cut to the cap); the
  sources hold each file whole, so a line pushed past the cap is never
  reported removed, and the cut mark is never a changed line.
- **Retrieval (CX5b).** While the org root is set, the clone's `.md`,
  `.markdown` and `.txt` files are a retrieval source of kind `org`
  (`org_passages`, called by `retrieval_broker`): hidden entries (`.git`,
  `.github`), the top-level `tools/`, symlinks and files over 256 KB are
  skipped, at most 8 folders deep; a request reads at most 400 files and
  4 MB, files whose path names a term first, with a warning when some were
  left out. A passage cites the file's absolute path in the clone and ranks
  like a connected project file, under the same pool and evidence budget;
  `kinds: ["org"]` asks for org files only. Retrieval and the sections read
  the working tree as it is and never wait on the pull (it takes no lock a
  reader honours; `a_hanging_pull_never_holds_retrieval_or_the_prompt`).
  Names retrieval keeps out of connected projects stay out here too (a
  likely secret such as `credentials.md` or `token.md`, a `restricted`
  folder; `project_text_index::private`), and the warning also counts files
  over 256 KB and says when the 20,000-entry walk limit stopped the listing.
  Org passages ignore a workspace's project scope on purpose: org context is
  for everyone. `read_source` can't read a cited org path
  (`source_evidence::read_project` admits connected projects only), so an
  agent reads an org file with its Read tool; an automation whose allowed
  tools have Bash but no Read can't read the clone at all (Claude's Bash
  rules refuse it).
- **Write block.** What refuses a write in the clone, per agent and mode
  (`write_rules_for`, `agent::codex_auto_profile_overrides`, pinned by
  `the_org_clone_write_block_per_agent_and_mode`):

  | Agent and mode | What holds |
  |---|---|
  | Claude, every mode | `--disallowed-tools` gets `Edit(//<root>/**)` (Edit, Write, NotebookEdit) and `Bash(*<root>*)`, plus `Bash(*~/<rel>*)` and `Bash(*$HOME/<rel>*)` for a root under the home folder: any shell command whose text names the clone by one of those spellings is refused, reads included (Read, Grep and Glob still read it). Claude checks each part of a compound command. |
  | Codex auto in a checked linked worktree | Its permission profile names the clone `read`, which beats a `write` folder holding it (probed with `codex sandbox`, codex-cli 0.169.0). |
  | Codex auto, any | The clone, or a folder in it, is never a writable root. |
  | Codex plan | The read-only sandbox. |
  | Codex default, Accept edits, auto outside a checked worktree | The `workspace-write` sandbox writes only the run's folder, its roots, `/tmp` and `$TMPDIR`; it can't carve the clone out of one of those. |
  | Codex full access, Grok, Antigravity | Nothing: full access has no sandbox, and the other two get no org rule. |

  **Limit** (`ORG_BLOCK_LIMITS`, pinned by
  `the_org_block_names_what_it_cannot_refuse`): the Bash rules match command
  text, so a command that reaches the clone by a relative path, another
  spelling, a variable or a script gets past them, and the table's last two
  rows hold nothing when the clone sits inside a writable folder. A root whose
  path holds anything but letters, digits and `/._-~` gets no Claude rule
  (the CLI splits its list at spaces and commas). The Bash rules match a
  prefix, so they also over-refuse: a custom root refuses commands naming a
  sibling that starts with its path (root `~/org` also refuses
  `~/organizer`); the default `~/Hollow/Context/org` has no such sibling.
  Boundary forms (`<root>/*`) were not used, since a quoted spelling would
  slip past them. The Bash rules are tested as generated text, not against a
  live Claude CLI. The real fix is an OS
  sandbox for every shell (Claude's `sandbox.filesystem.denyWrite`), not
  turned on because it would confine every Bash write.
- **Not yet.** The runner's own home file for a remote turn (CX12) and the
  read-only extra folder.

### Context sync engine (CX6a, 2026-11-14)

`sync_engine.rs` (tests in `sync_engine_tests.rs`). One engine with a list of
roots; only the personal root is built. Its git directory is in app data and
its work tree is the vault, so the five `me/` files keep their path.

| What | Where | Shape |
|---|---|---|
| Settings | `context-sync.json` in the app data folder | `{"version": 1, "roots": [{"id": "personal", "kind": "personal", "remote": "<url>", "branch": "main", "enabled": true}]}`. A URL with a password in it is refused; no secret is saved. A file that doesn't parse reads as off and is refused, not overwritten, by a save. Kinds `vault` (CX6b) and `org` (V6) parse but don't run yet. |
| Git directory | `ContextSync/<id>.git` in app data | A bare repository driven with `--git-dir` and `--work-tree`; `info/exclude` lists the synced folders. Never inside the vault. |
| Lock, state | `ContextSync/<id>.lock`, `ContextSync/<id>.state.json` | An exclusive `flock` per cycle (the kernel drops it when a process dies); the last cycle's state, error, blocking file and clash copies. |

- **Allow-list.** Personal: `me/`, `memory/`, `learnings/`, `home/`. Staging is
  `git add -A -f` with those folders as literal pathspecs (`.DS_Store` out), so
  the vault's own `.gitignore` can't hide them and nothing else is staged.
  Files the remote holds outside them (its `README.md`, `INDEX.md`,
  `templates/`) stay in the commits and are never written into the vault.
- **Cycle.** A tracked synced folder missing from the vault (evicted,
  unmounted, a move half done) blocks the cycle, and so does a commit removing
  more than 20 files or more than half of those tracked (two or more): neither
  is ever pushed as a deletion. The same guard holds for removals arriving
  from the remote. Sync now with removals allowed (`context_sync_now`
  `allowDeletions`, the person's) lets one cycle past it. Stage; leave out and name a symlink, a nested
  repository, a `.git`, `.` or `..` part (any case) or a file over 64 MB, and
  commit the rest; scan every staged blob with the shared rules
  (`scan_rules::scan_bytes`, layer `home` for `home/`, else `personal`, allow
  lines ignored); commit with plumbing (`write-tree`, `commit-tree`,
  `update-ref`; no hook runs; trailers `Hollow-Actor` and `Hollow-Machine`);
  `fetch`; merge the remote branch with `merge-tree --write-tree` (a merge,
  never a rebase); write the result into the vault ourselves; push, never
  forced. A refusal or scan hit puts the index back to `HEAD` and commits
  nothing: the root is blocked until the file is fixed.
- **Writing the vault.** Only paths inside the allow-list, only plain files.
  Every incoming blob is read and scanned first: a version holding a secret
  is withheld (never written here, left out of every `add`, named with "remove
  it on the remote or on the Mac that pushed it") while everything else
  merges and pushes; a clean version later replaces it. Each file is checked to still hold what `HEAD` holds before the
  pass and again right before its own write (else the next cycle retries);
  a file that became a folder or a folder that became a file works (folders
  left empty, `.DS_Store` aside, are removed);
  the settings are re-read before each write, so the off switch stops a
  running cycle; writes go before removals and files before `HEAD` moves, so
  a crash re-commits them instead of reverting them. A remote symlink, `.git`
  path or oversized file is left out, named, and kept out of every later
  `add` so it is never staged as removed. Fetches run with
  `fetch.fsckObjects` and `transfer.fsckObjects`.
- **Clash.** The local version keeps the name; the remote's is written beside
  it as `Name (from <machine>).md` (`Name (from <machine> 2).md` when taken),
  the machine read from the remote commit's `Hollow-Machine` trailer. No
  conflict marker is ever written (a blob the merge made, not one either Mac
  saved, is checked for markers). A
  file where the other Mac has a folder is kept beside it the same way. Each
  clash is saved as a record and shown as a clash card (CX6c, below); no error
  notification is raised.
- **Offline.** A failed fetch or push leaves the commits in the git directory;
  the status reads "N changes not synced" (commits plus working changes not yet
  committed) and the next cycle pushes them. A push the remote rejects is
  retried, never forced; three in a row raise an item, and a remote whose
  history no longer holds the tip seen last time raises "the remote's history
  was rewritten".
- **Remote URL.** `https://` with any user part is refused (a token); the
  status and errors show URLs with any user part as `***`.
- **Timer.** `spawn_timer`, its own thread: every 15 s it fingerprints the
  synced folders (sizes and times, no symlink followed) and runs a cycle once
  they have been still for 60 s, and every 10 minutes regardless. Each git
  runs in its own process group with a timeout (60 s local, 120 s network),
  no prompt, no hook (`add` gets 300 s). While the cycle lock is held, git's
  own lock files are removed when no process names the git directory, else
  when older than 10 minutes. An empty first tree is not committed.
- **Off switch.** `enabled: false` returns before anything is read or written:
  the vault, the git directory and the remote stay as they are.
- **Commands.** `context_sync_status` (read, also `settings.context_sync_status`
  for agents); `configure_personal_sync`, `set_context_sync_enabled` and
  `context_sync_now` are the person's (user-only). The Settings rows are
  [Settings, Context](#settings-context-cx17-2026-11-15).
- **Needs** git 2.38 or later (`merge-tree --write-tree`).

### Org changes digest (CX8b, 2026-11-15)

`context_org_digest.rs`, the `org-digest` window job on `nightly` (02:00 to
05:00, once on wake). It reads `context-org-prs.jsonl` (CX8's record) and lists
each owner PR (`identity: owner`, `digest: true`) that merged and was never
listed: on its opened line's time when it merged at once, else its first
`merged` line's, so a PR that waited shows the day it merged. Teammates' PRs
and owner PRs still waiting on checks are not listed. One line per PR (title
or branch, url, files, merge time, chat; never the reason or contents) goes
into the Main workspace's `outputs/org-changes/<YYYY-MM-DD>.md`, beside the
weekly update and gardener reports, and one in-app notification ("Org changes
merged as you", no native pop) opens it. The listed urls are kept in
`context-org-digest.json` beside the record, so a second run lists nothing and
the same day's note never gets a line twice; a broken watermark or an unreadable
note stops the run instead. Titles are markdown-escaped. Nothing new: no note, no item.

### Settings, Context (CX17, 2026-11-15)

Settings → Sources → **Context** (part id `layers`; `context` was already an
old alias of Advanced) is a closed group holding one row per layer, drawn by
`settings/ContextLayersPanel.tsx` from one read, `context_layers_status`
(`context_settings.rs`; agents read it as `settings.context_layers_status`).

| Row | Shows | Buttons |
|---|---|---|
| Home | status pill, this Mac's slug and `home/<slug>.md`, the personal clone folder | Save name (Use the suggested one), Use folder |
| Personal | the sync engine's pill and line, last sync, the repo (user part `***`), the last error | Sync now, Stop syncing or Turn sync on, Local only (a confirm first), Connect an existing repo (URL field), Create a private repo (guided steps, Open GitHub) |
| Organization | pill (Read only, Not connected, Not a git clone), last pull (`.git/FETCH_HEAD`'s time), the origin | Use folder, Check access; a Request access line; no Create |
| Project folders | how many project folders carry a `CLAUDE.md` or `AGENTS.md` (plain files, first 60), and which | none (read only) |
| Context check (CX10) | the leak-check mode, each repo's hook state (Checked before push, Not checked, Has its own hook) | Install context check, Remove ([Leak checks](#leak-checks-cx10-2026-11-15)) |
| Vault sync | Not set up, or "Remote saved; the first push is held" once a `vault` root is in `context-sync.json` (CX6b) | Connect, disabled: the first push is held |

- **Create is guided steps, decided 11-15.** The person makes an empty private
  repo on GitHub themselves (Open GitHub opens `github.com/new`), then pastes
  its URL into Connect. No account name is in code. Hollow making the repo
  through `gh` is CX18, behind an External card showing the name and
  visibility, so a Settings click never creates anything on GitHub.
- **Connect** checks the URL in the window first (`remoteProblem` in
  `contextLayers.ts`: no user part in an `https` URL, no password in `ssh`, no
  `http`, no `ext::`) and the backend checks it again (`valid_remote`).
- **Local only** (`context_personal_local_only`, refused while a sync runs)
  saves the personal root with no remote and sync off; the vault's files, the git directory in app data and
  the remote repository stay. Connect again to resume.
- **Check access** (`context_org_access_check`) runs `git ls-remote --exit-code
  origin HEAD` in the org clone (hooks and fsmonitor off, ssh in batch mode, no
  terminal prompt, 30 s at most) and answers ok (exit 0, or 2 for an empty
  repo), denied, offline, missing or error, in words. `.git` must be a real
  folder (no symlink, no gitfile). `redact_urls` hides any `user:secret@` and
  `token=`-style query values in the origin and git's errors. A changed org
  folder clears the last result.
- **Removals:** a sync the mass-removal guard blocked offers Sync and allow
  removals… (a confirm, then one `context_sync_now` with `allowDeletions`).
- **Tiers.** The status is a read action; the access check, Local only, the
  clone folders (`set_context_root`) and the machine name
  (`set_context_machine_slug`) are user-only, each with its COO exception.
- **Mock:** `mockContextLayers.ts` (knobs `mockContextLayers=connected`,
  `mockContextOrgAccess`).

### Connect this Mac's context (F3, 2026-11-15)

`mac_context.rs`, the two-Macs form of "Back up and sync your vault". When
another Mac appears (a runner named in `slot-remote`, or a Mac paired in
Machines), Settings, Sources, Context opens with a card, **Connect this Mac's
context**: one ticked row per layer (Personal, Organization, This Mac's name
and home file, Vault), repo address fields, and **Connect**, **Not now**,
**Not for this Mac** (`settings/MacContextCard.tsx`). With no open offer and a
layer still missing, the same rows sit in a closed fold there.

| What | Where | Shape |
|---|---|---|
| Offers | `mac-context-offers.json` in the app data folder (0600, temp file and rename) | `{"version": 1, "pairs": {"<id>+<id>": {"offeredAt", "answer": "connected" / "not-now" / "never", "answeredAt"}}}`; ids sorted, so A-B is B-A; a runner is `runner:<alias>`; unknown fields kept; a file that doesn't parse is refused, never overwritten |

- **When.** `offer_due`: a new pair; never after Connected or Not for this
  Mac; after Not now (or an offer left unanswered), once 14 days have passed
  since the later of the offer and the answer. Checked a minute after launch,
  after a pairing and after a runner is named; a due pair with a layer missing
  here gets one in-app notification (Open goes to the card) and `offeredAt`.
- **Connect** (`mac_context_connect`, user-only). The request holds layers and
  addresses, never a path: each clone goes to the layer's saved root, else
  `~/Hollow/Context/<layer>`, checked by the context-roots path rules. A
  folder that is already a git clone is kept (saved as the root if it
  wasn't); one holding other files is refused. git clones into a hidden
  staging folder beside the root (no prompt, ssh in batch mode, hooks off),
  renamed into place only when done, then the root is saved; a failure leaves
  nothing. An address with credentials is refused and messages are redacted;
  no access says to sign in to git here. Home names this Mac when unnamed
  (`save_machine_slug`, the same save as Settings' Name) and writes a short
  `home/<slug>.md` only when none is there. Vault reports **held**: it has no
  remote until V6. Nothing is committed or pushed; each layer reports alone,
  and the pair is recorded Connected only when every step worked and no
  personal, org or home row is still missing (an unticked one included). One
  Connect runs at a time (a second is refused); the other layer's root is
  checked for overlap before any clone; the check stamps a due pair offered
  in the same write that finds it due, so two triggers notify once.

### Hashed identifier denylist (BG3, 2026-11-16)

Separate from the leak checks below: `scripts/identifier-guard.mjs` keeps named identifiers (clients,
employees) out of *this repo's* tracked files without listing them. Words are read
across the whole file (a name a hard wrap split still matches; two paragraphs do
not join), folded with Unicode NFKC and lowercase, and a file that could not be
scanned (unreadable or over 8 MB) fails the run (exit 2); a binary file is counted and not read, a UTF-16 text file is decoded
rather than passing.
`scripts/identifier-denylist.sha256` holds only SHA-256 values of lowercased
one- and two-word phrases; the plaintext names stay in a file of your own
outside the repo. The guard hashes every one- and two-word shingle of every
tracked text file and prints `path:line:column` and the first eight hex
digits of a hit, never the text. A missing, empty or malformed list fails
closed (exit 2). Add names with `node scripts/identifier-guard.mjs --hash <
scripts/identifier-names.txt >> scripts/identifier-denylist.sha256` (one or two words per line, a
single word at least 4 characters; that file name is git-ignored). `scripts/identifier-guard.test.mjs`
(`npm run test:scripts`, so the gate) scans the repo and plants the committed
canary phrase to prove the guard is live. A short or common name will
false-hit and a short name's hash can be brute forced, so this stops casual
reading only. Not wired yet: the pre-push hook's Python `ctx-scan` for other
repos, and the in-app write rules; the list lives in `scripts/` because
`vendor/ctx-scan/` is a hash-checked copy of upstream files.

### Leak checks (CX10, 2026-11-15)

`leak_checks.rs`. Flag-only for a week; CX10b turns on blocking. Every check
uses the shared rules (`scan_rules`, `vendor/ctx-scan/scan-rules.json`) and
names file, line and rule id, never the matched text or a value.

- **The switch.** Setting `context.leakChecks` (`flag` | `block`, Confirm
  tier, section Sources), saved in `context-leak-checks.json` in the app data
  folder. Unset means `leak_checks::DEFAULT_MODE`, which is `flag` now; CX10b
  flips that one constant.
- **Write time.** Context writers call `leak_checks::check_write` (or
  `check_write_in_root`), which runs `memory_guard::check_layer_as` in the
  current mode. In `flag` a Project write whose text names a computer or a
  person is written, and one line goes to `context-leak-flags.jsonl` (app
  data, 0600): time, source `write`, layer, relative path, rule ids, severity,
  the layer the text belongs to (`home` or `personal`), actor and chat. In
  `block` it is refused with CX7's message, unchanged. `check_layer` itself
  stays the strict form. **A secret is refused in both modes**: the safety
  scan (stored keys, injection, hidden text) runs first and a key file name
  (`secret.key_file`) is refused whatever the mode. The org layer stays read
  only (a structural refusal, not a scan finding); `context.propose_org_change`
  keeps refusing block-level org findings, since the org repo's own check
  would fail that pull request anyway.
- **Runs.** `run_guard` already puts the project-layer scan on an automated
  run's `CLAUDE.md`/`AGENTS.md` flag (CX7); `after_finish` now also records
  those hits in the ledger (source `run`, actor = the run's source, chat = its
  id). Nothing is undone.
- **Review.** `context.leak_flags` (read; `days`, default 7) returns the mode,
  counts by rule, layer and source and the newest flags.
- **Install context check.** Settings → Sources → Context → Context check
  lists the org clone (layer `org`), the personal clone (`personal`), the
  vault when it is a git repo (`vault`) and each connected project repo
  (`project`), with Install context check or Remove. `context_install_check`
  (action `context.install_check`, write, dry run by default; apply only from
  an attended chat, never an automation) writes a local `pre-push` hook where
  `git rev-parse --git-path hooks/pre-push` says (worktree-aware). It is never
  committed: a hooks folder outside the repo's git folder (`core.hooksPath`
  in the work tree or a shared folder) is refused, a pre-push hook that isn't
  Hollow's is refused and left alone, and Hollow's own earlier hook is kept as
  `pre-push.hollow-backup-<time>` before it is replaced. The hook carries the
  marker `# hollow-context-check v1`, runs `/usr/bin/python3 -I <app
  data>/ctx-scan/ctx-scan --layer <layer> --fail-on block` over what the repo
  would commit, and fails the push only on a blocking finding. It scans the
  whole work tree as `ctx-scan` lists it (tracked files and untracked ones
  that aren't ignored, not only the pushed commits) and ignores the
  `context.leakChecks` switch: the layer's own severities apply, so an org
  clone's hook blocks machine facts even in the flag week. It fails open: a
  missing scanner or a scanner error (exit 2) lets the push through and says
  so. Its git calls and the scanner run with the repo's fsmonitor off and no
  prompt; Hollow's own git calls also set `core.hooksPath=/dev/null`, except
  the one asking where the hooks folder is.
  `context_remove_check` (`context.remove_check`) deletes only a hook with
  the marker.
- **Why a copy in app data.** The scanner and its rules are built into the
  binary and written to `<app data>/ctx-scan/` on every install and audit.
  App data outlives app updates and moves; the app bundle's resources change
  path or vanish with them, which would leave every hook pointing at nothing.
  A copy is only as new as the last install or audit.
- **Monthly audit.** `context.audit` (read) runs the same copy over every
  candidate repo: the personal clone twice (`home/` at layer `home`, the rest
  at `personal`, where Acme facts are allowed and not flagged; secrets,
  SSN shapes and data files are), the org clone at `org`, project repos at
  `project` (machine facts warn, secrets block). Bounded: two minutes per
  scan and five for the whole audit (repos past that are named in `skipped`),
  200 listed findings per repo for up to three scans and 25 beyond that, 500
  in all (`leftOut` counts the rest), plus the last 31 days of flags. A mode
  file that exists but doesn't read counts as `block`. Automations → + New
  automation → **Monthly context audit** is a built-in template
  (`CONTEXT_AUDIT_TEMPLATE` in `contextLayers.ts`, 9 AM on the 1st): it runs
  `context.audit` and `context.leak_flags`, writes `outputs/context-audit-<year>-<month>.md`
  and a notification. Nothing turns it on; the person saves it.

### Folder abstracts (CX23, 2026-11-18)

`folder_abstracts.rs`. A one-line abstract and a short overview per folder of
each active project workspace (`workspaces/<id>`, root `project:<id>`) and of
each top-level org section (root `org:<folder>`): the cheap middle between the
short core (CX4) and retrieval (CX5b).

- **Off by default.** Setting `context.folderAbstracts` (Bool, Confirm tier,
  section Sources), saved in `folder-abstracts.json` in app data. It gates
  building and showing alike. It stays off until a measured recall comparison
  says abstracts help; that comparison is not built yet.
- **Bottom up, keyed.** Only Markdown and text files count (no hidden or
  secret-looking names, the rule `read_source` applies (`forbidden_component`),
  no symlinks, `node_modules`, `target`, `dist`, `build`, `vendor` or `log`; 3
  folders deep and 80 files per root at most). A file's key hashes the prompt
  version and its text; a folder's key hashes the prompt version and its
  children's names and abstracts. A node whose key matches makes no model call,
  so an unchanged tree makes none, and a changed file refreshes each folder
  above it once while the abstract it hands up changes. Bumping
  `PROMPT_VERSION` rewrites every entry.
- **The index** is derived: `FolderAbstracts/<vault stem>.json` in Hollow's
  support folder (beside `EmbedIndex/`), never in a layer's own files. Delete
  it and the next refresh rebuilds it.
- **The model** is the app's agent with its extraction model, tool-free, one
  one-shot per changed node, metered as `folder_abstract`. File text (up to
  6,000 characters each, through `claude_runner::redact`) goes to that provider
  while the setting is on. It doesn't route to a self-hosted model yet (no
  `self_hosted_jobs` kind of its own). The index is saved after each root that
  made calls.
- **Who runs it.** `context.refresh_folder_abstracts` (Write, `maxCalls`
  default 100; stops there keeping its work, call again to go on); nothing runs
  on a timer. `context.folder_abstracts` reads the overviews. With the setting
  on, `workspaces.list_workspaces` rows carry `abstract`, so the COO or a lead
  choosing between projects reads it before opening one; off, that answer is
  unchanged.

### Vault root (CX6b, 2026-11-15)

`sync_vault.rs` (tests in `sync_vault_tests.rs`). A second root of the same
engine: settings entry `{"id": "vault", "kind": "vault", "enabled": true}`,
git directory `ContextSync/vault.git`, its own lock and state, work tree =
the vault. **Off unless the person turns it on** (`configure_vault_sync`,
user-only; `enabled: true` saves it and runs one cycle, `false` turns it off
and touches nothing; `allowDeletions` lets that one cycle past the removal
and missing-folder guards). No timer runs it yet.

- **Local only until V6.** A vault root with a `remote` is refused (in the
  config and by the command), so a cycle never fetches, merges or pushes; it
  commits in app data and its state is `localOnly` ("Local only: committed on
  this Mac, no remote yet"; pending commits read 0).
- **What stays out.** The personal root's `me/`, `memory/`, `learnings/`,
  `home/` (never tracked twice), `tasks/`, every top-level dot file or folder
  (`.hollow-history/`, `.trash/`, `.archive/`, notification, health and
  tracker state), `node_modules/`, `.git/`, `.hollow-attachments/`,
  `screenshots/`, `captures/`, `recordings/`, `video/` and other build or
  cache folders at any depth, `*.jsonl`, binary extensions, CX1's
  `file.data_type` extensions and `secret.key_file` names, any file over
  1 MB and any file with a NUL byte (a new file's first 8000 bytes are
  sniffed, every staged blob is checked whole). The personal folder names
  match in any case (`Me/` too). The vault's own `.gitignore` files are
  honoured.
- **Generated ignore.** CX0's manifest is a one-off report, not runtime
  input, so its vault decisions and the V0 list are built in as a versioned
  `MANIFEST`. Its ignore goes to the root's `info/exclude` (app data), is
  rewritten at the next cycle whenever the manifest or CX1's rules change,
  and never into the vault. `path_allowed` (same manifest) decides; new
  files are filtered before `git add`, so a big or binary file is never
  hashed.
- **Already tracked.** A file that grows past 1 MB (checked with `lstat`
  before `git add`, so it is never hashed) or turns binary keeps its last
  committed version and is named (Waiting on you); one a newer manifest
  leaves out stays at its last version (untracking it would delete the other
  Mac's copy once V6 adds a remote).
- **Guards.** The scan of every staged blob (layer `vault`): a hit leaves
  that file out at its last version and names file and rule, and the rest
  commits (the personal root still commits nothing on a hit), so one false
  positive never holds back the vault. Symlinks and nested repositories are
  left out and named. A tracked top-level folder the manifest still allows
  that is missing from the vault, or a mass removal, blocks the cycle until
  the person allows removals for one cycle. The lock and the off switch as
  for the personal root.

### Binary copy between the Macs (V4, 2026-11-15)

*Reworked 2026-11-18 to Alex's N15 answer (11-13): binaries stay on the
Mac that made them and come over ssh only when asked. The heading keeps its
anchor for older links.*

`sync_files.rs` (tests in `sync_files_tests.rs`). The write-once files the
vault root's git keeps out stay where they were made. Nothing copies on a
timer and nothing is sent from this Mac.

- **Fetch on demand.** `vault.peer_files` (read; optional `prefix`, `limit`,
  default 200) lists the other Mac's fetchable files that this Mac lacks,
  with size and mtime. `vault.fetch_peer_file` (write, `path`) brings one
  here at the same vault path. It never overwrites: a path already here
  returns `alreadyHere` without asking the other Mac, and the copy links in
  without clobbering, so a file that appears meanwhile wins. Size and
  SHA-256 are checked, the other Mac's mtime is kept, and an interrupted
  fetch resumes from its `.<name>.hollow-part` next try. One fetch at a time.
  Both only read on the other Mac. Both are `experimental` until they run
  against a real second Mac.
- **What can come over.** A file the vault root leaves out because it is
  binary or big: anything under a media folder (`screenshots/`, `captures/`,
  `recordings/`, `video/`), a binary or CX1 data extension (png, pdf, mov,
  csv, xlsx...), or any other file over 1 MB. **Never:** `.md`, `.txt`,
  `.json`, `.jsonl`, `.yaml`/`.yml`, `.toml`, `.env`, `.sql`, lock, swap and
  database journal files, the personal folders (`me/`, `memory/`,
  `learnings/`, `home/`), `tasks/`, any dot file or folder at any depth,
  `node_modules/`, `target/`, `venv/`, `__pycache__/`, `test-results/`,
  `playwright-report/`, CX1 `secret.key_file` names, symlinks (a symlinked
  file or folder on the other Mac reads as not there).
- **The peer.** The runner in `~/.config/hollow/slot-remote`, reached with
  `ssh -T -o BatchMode=yes -o ConnectTimeout=10` (no password or keyboard
  auth, no agent or port forwarding, the host's own key from `~/.ssh/config`).
  Its vault is `remoteVault` (home-relative or absolute; default: this
  vault's path below the home folder, `Hollow/Vault`), set by the person with
  `configure_file_copy` (user-only). Every step runs one Perl program there
  (`perl -e`; the fetch uses modes list, stat, hash and send) with each
  argument single-quoted; it refuses absolute paths, empty or dot components
  and symlinks at every level (`lstat`, `O_NOFOLLOW`). Each step is killed
  with its process group after 20 minutes; the part stays for the next try.
- **Status.** `settings.file_copy_status`: `mode` `fetchOnDemand`, the peer
  and its vault path, `state` (`noPeer` or `ready`), and the retired timed
  copy's last run as history.
- **The retired timed copy.** V4 first merged (#1454) as a two-way copy
  every 15 minutes, off by default, never run on a real second Mac. Since
  the rework no timer starts, `configure_file_copy` refuses `enabled: true`
  and saves the switch off, and its planner and cycle stay in the code,
  unused and tested, until Alex confirms or drops it. Nothing was
  migrated or deleted: `file-copy.json` and `.hollow-files.json` in app data
  stay as they were (the status reads the record).

### Clash cards (CX6c, 2026-11-15)

One card per clash, the same on every root (`sync_clash.rs`, a child module of
the engine).

- **Record.** A merge that keeps both versions saves a record in the root's
  `ContextSync/<id>.state.json` (`clashes`): the file, the copy, both machine
  slugs, when, and a SHA-256 of each version as written. It survives restarts.
  A record whose copy is gone (removed by hand, or by the other Mac) is
  dropped. A copy set aside because a folder took its name is not
  replaceable: only Keep both and Edit apply.
- **Waiting on you.** One `clash` item per record (category `context-sync`,
  `sourceId` `<root>:<path>`, state `pending` or `editing`), grouped as
  Context sync clashes under Decisions. It replaced the interim error
  notification. The conflict copies item leaves these copies out, and a copy
  the person kept (Keep both, Done) stays out while it exists
  (`resolved_copies` in the state), so no file is offered twice. The query reads the state files only, never a vault file.
- **The card** (`components/SyncClashesDialog.tsx`, `ClashCard`): root, both
  machines and the time, a line-count summary of what differs and the first
  8 lines (200 characters each, files up to 512 KB, read without following a
  symlink) of each version. Opened from the Waiting on you item and from the
  Personal and Vault sync rows of Settings, Sources, Context (**N clashes**).
- **Choices** (`resolve_sync_clash`, user-only, naming the record by root,
  file and copy): **Keep mine** moves the copy to the vault's `.trash/`;
  **Keep theirs** renames the copy over the file (the old text to `.trash/`)
  only while the file still hashes as recorded; both refuse, naming it, when
  the copy changed since the clash; **Keep both** closes the record and
  leaves both files; **Edit** marks it editing and opens the file, **Done**
  closes it. The root id, the choice and the path are checked first (a
  record's own path only, inside the root's tracked folders, no symlink on
  the way, no `..`). Keep mine and Keep theirs hold the workspace edit lease,
  so they refuse while a run holds the vault. A resolution is an ordinary
  local change: the next cycle stages, scans and commits it. An open clash
  never stops a cycle, and other files keep syncing.
- **Agents** read `settings.sync_clashes`: paths, machines and times, no file
  contents. The choices are the person's (`COO_EXCEPTIONS`).

## What belongs in the vault

The vault holds what a person or an agent reads as notes, plus the few state
files whose meaning is tied to those notes. Anything derived, re-fetchable or
per-machine lives outside it (the app data, cache and session folders), so the
vault stays small, syncable and safe to open in Obsidian. Settled 2026-09-26; see
the build log.

| Path | Written by | Why it is in the vault |
|---|---|---|
| `workspaces/<id>/` | Hollow + you | `WORKSPACE.md`, `PLAN.md`, `SOURCES.md`, `AGENTS.md`, `NOTION.md` (legacy, still read), `outputs/`, `sessions/`. The unit of work. |
| `tasks/` | Hollow, tracker sync | One note per task. A tracked project's mirror is its workspace's `PROJECT.md`; there is no top-level `projects/` (2026-09-26). |
| `me/` | you | The five About-you notes; Settings → Personalization creates any that are missing. |
| `learnings/` | Hollow (on approval) | Approved memory only. |
| `library/` | you, ingest | Reference you keep: `work/` (the approved work wiki), `atlas/` (Atlas knowledge base), `notes/`; ingest writes to `library/imports/`. |
| `workflows/`, `drafts/heartbeat/`, `briefings/` | you, automations | Created on first use only. |
| `README.md`, `AGENTS.md`, `CLAUDE.md`, `docs/BUILD_LOG.md` | you | The vault's own orientation and history. |
| `goals/`, `commitments/`, `people/` | backend only | Still read by retrieval and alignment; no page creates them. |
| `.hollow-history/`, `.evidence-receipts.json` | runs | Restorable before/after history of vault files and each run's receipt. |
| `.review-queue.json`, `.notifications.jsonl`, `.heartbeat-log.jsonl`, `.hollow-health.jsonl` | Hollow | Suggestions awaiting review, notification center, audit trails. |
| `.notion-sync.json`, `.notion-sync-backups/`, `.notion-imports.json`, `.notion-imports.lock` | Notion sync (removed 2026-11-10) | History: the removed sync's baselines and page mapping. Nothing writes them; the importer copies `.notion-sync.json` read-only and its apply moves them to `.archive/`. |
| `.workflow-triggers.json`, `.nudges-state.json`, `.task-filters.json` | Hollow | Per-vault automation watermarks and saved filters. |

Not in the vault at all: calendar (the source connected in Settings → Connectors;
Outlook was removed on 2026-09-26), recordings (Plaud: Recordings in Hollow reads
them on demand with Hollow's own sign-in, and agents use the Plaud connector; a
kept transcript copy goes into a workspace's sources) and projects (Linear; each
is a workspace). Outside the
vault: the project index (`Hollow/Directories/`), sessions, usage, run queue and
feedback (`Hollow/Sessions/`), search and meaning indexes, heartbeat counters,
workflow permission stamps, lease locks, run permits, budgets, agent settings
and connectors. Hollow removes its own retired vault files at startup
(`vault_hygiene::RETIRED_STATE_FILES`). Hollow never creates empty top-level
folders ahead of use; the old scaffold wizard that did is gone.

## Discovery, storage and refresh

`src-tauri/src/directory_connections.rs` owns discovery and retrieval. The project
index (version 1, camelCase; `directory_connections::manifest_path`, at
`~/Library/Application Support/Hollow/Directories/{vault stem}.json` since
2026-09-26, outside the vault; a legacy `.hollow-directories.json` in the vault is
adopted and removed on first use) stores canonical roots,
`refreshedAt` (the last successful refresh), project IDs/paths/titles, coverage
and warnings. Since next-level P07 (2026-10-01) the supported-file inventory
lives beside it in `{vault stem}.inventory.json` (`inventory_path`; version 1,
compact `[relative, project index, size, modified ms, state]` rows plus folder
mtimes, written atomically, 0600, cached in memory by length and mtime); a
manifest that still holds an inline `files` list is adopted as the last-known
inventory until the next refresh writes the store. Both contain metadata, not
copied project contents or connector secrets.
The manifest is the connection registry used by the app and owned MCP process;
`projectsPath` also records the selection in config. Hand-editing config alone
does not build a connection: use Prepare and connect. Existing installations
without the manifest continue to work and can connect their Projects directory
later. `gitReposPath` remains an unrelated legacy heatmap setting.

Every discovered repository gets a workspace. Supported project documents outside
repositories group under their top-level project folder; loose root files group
under the selected root. Nested repository boundaries are retained. IDs are
`local-` plus 24 hex characters of the canonical project's SHA-256 path hash.
Repeated setup, refresh and reconnecting the same path reuse those IDs. Renaming
or relocating a project creates a new identity; prior workspaces/conversations
remain available and are never deleted. A removed/ignored project leaves its
Hollow workspace intact but contributes no live evidence.

The scan respects `.gitignore`, `.git/info/exclude`, global Git ignores, `.ignore`
and `.hollowignore`, including nested rules. Hard exclusions still win over
negation: hidden paths, symlinks, Git internals, linked Git worktrees, common
credential filenames/extensions, dependency folders and build outputs. Linked
worktrees are omitted rather than duplicated; choose the main repository instead.
These filename exclusions are not a content-based secret detector: put additional
private material in `.hollowignore`. Hollow never changes Git configuration. Projects is read-only to this feature, so readable source
folders do not need write permission.

The inventory is complete (P07; before it, 2,500 files per project and 20,000
in total). Every supported file is listed up to a per-project share of 50,000
(`MAX_PROJECT_FILES`, so one huge folder early in path order can't hide every
project after it; before 2026-09-26 a 56-project folder never reached
`personal/hollow`) and a 200,000-file safety bound (`MAX_FILES`). Past either,
files are still walked and counted (`omitted`, per project and in total, with a
warning naming the project), never dropped silently, and stay findable by
name (below). The walk itself stops only at 1,000,000 visited entries
(`scanLimited`; counts become lower bounds). Each listed file carries a state:
`text` (UTF-8 text/code through 1 MB), `tooLarge` (a text format over 1 MB:
listed as "not read", never indexed as empty), `extracted` (PDF, Word,
PowerPoint and Excel: Hollow reads the text itself, repo features R14a) or
`needsReader` (images, `requires_reader` coverage: they need a runtime with
appropriate file tools or a text export). `extracted` exists in memory and on
the wire only: the inventory store keeps writing state `2` for documents and
images alike, and loading maps a stored `2` to `extracted` by extension
(`doc_text::kind`) and size (a document over the 64 MB reader cap is
`needsReader`), so the per-file state ints are unchanged and an older build
reads the same rows. The saved connection's coverage summary does gain an
`extracted` count (per project and in total), which older builds ignore.
Extraction (`src-tauri/src/doc_text.rs`): a PDF through PDFKit,
`## Page N` before each page's text layer (macOS only); a Word file one
paragraph per line; a deck `## Slide N` before each slide, slide10 after
slide9; a workbook through `xlsx_cells`. Caps: files up to 64 MB, the first 500
pages or slides, 4 MB of text, 50 MB of uncompressed Office XML, every cut
stated on the first line (`[truncated: …]`). A PDF with no text on any page is
`image-only PDF: no text layer (needs OCR)` (there is no OCR); a locked one
`password-protected PDF`. Extracted text is cached in memory per process by
(path, size, mtime), 64 MB and 512 files in all, oldest out first, failures
included; no extracted text is written to disk (the content index, below,
keeps a word index of it, outside the vault, while it is on). Unknown formats are not catalogued. Hollow reads only
`.git/config`'s `origin` URL, for workspace icons (below).

The native background worker reconciles at startup and every 60 seconds; Settings
and Library expose Refresh projects. A refresh is incremental: every folder is
still listed (so ignore rules apply exactly), but a file in a folder whose mtime
is unchanged reuses its recorded size and time, with a full re-stat at least
every 10 minutes (`FULL_STAT_SECS`; an in-place edit that pushes a file past
1 MB can show its old state until then, while reads always recheck the size),
and the inventory store is rewritten only when its files, folder times or
full-stat stamp changed; `coverage.changedAt` moves only when the listed files
did. Library polls the status each minute and re-pages its list only when
`changedAt` moves, and Work refreshes workspace rows each minute. Reads and
request retrieval no longer re-walk the whole folder: they take candidates from
the stored inventory and confirm them with a fresh walk of just those files'
folders (`confirm`), under every scan rule, so removals and changed ignore
rules apply immediately; a file added since the last refresh is readable by
path at once and becomes a retrieval candidate at the next refresh (within a
minute). Unavailable roots preserve the last inventory and report it as
last-known; reads fail instead of silently serving stale content. Scan errors
and limit warnings are visible. File previews are current when opened; click
again after an external edit to reload.

## Library, workspaces and agent context

Library → **Connected projects** browses originals 100 at a time from
`connected_files` (debounced, latest request only), filters by project, name/path
words and file state (**Read by Hollow (PDF, Office)** among them), reads
supported text and a document's extracted text (tagged "read by Hollow · PDF",
Word, PowerPoint or Excel) without edit controls, reveals the
original in Finder for a native reader, and opens the associated workspace. A
name search also lists files **found directly** outside the listed inventory
(a typed path, or a name inside a project past its share, found by a fresh walk
of just that project). The coverage line above it shows listed / not-listed /
not-read counts, the last successful refresh, and the repair path (Refresh
projects, leave a folder out, Show the unread files). The command palette
(⌘K) looks up connected files by name too (scope chips All · Notes · Projects ·
Workspaces), and opens one in Library scrolled to its best-matching line.
A linked workspace's Sources pane has **Browse connected project**. Explicit
snapshot attachment remains separate and retains its existing provenance contract.
Briefs, plans, source snapshots, outputs and bounded session summaries remain
under `vault/workspaces/<id>/`; full conversation history stays in Hollow SQLite.

Before each chat or automation request, Hollow retrieves query-selected evidence
on a blocking worker (for chat, inside the vault lease since Paperclip PR 7b,
after the turn has been admitted from the run queue). Chat retrieves on the user's own words: the composer's
conversation, operator, page and intent blocks are stripped first, by
`chat::accept_turn` (stored as the queued turn's `asked`) and again by
`directory_connections::context` (the intent block
alone used to fill the whole term budget, so chat never retrieved on the
request). A literal close tag inside the page note or operator profile does not
end its block early. Workflows retrieve on their prompt body. Everything below
is the host retrieval broker (`src-tauri/src/retrieval_broker.rs`, agent
effectiveness F3, 2026-10-06), which automatic retrieval, `search_vault` and the
vault search app actions share. External evidence is limited to one project
when the run's folder belongs to it (`resolve_scope`): a connected project's
workspace, a build workspace's worktree (whatever it is called: the project its
repository is in), a folder inside a project, or a linked git worktree of a
repository in one. Other workspaces can retrieve across the selected Projects
root. Until 2026-10-06 only a `local-…` folder name scoped, so a build worktree
never was. Existing supported local vault notes
(including legacy Notion records, Calendar and Plaud material) are scanned by the same
term matcher as keyword search: newest first, only the FTS index's candidates
when there is one (OB12: the best 200 by BM25, name matches and the 20
newest; the app's index, which the `hollow_vault` subprocess reads
read-only), else (or with a kinds filter) every eligible note, skipping
(and reporting) notes over 1 MB; with only vault passages a note's BM25
band ranks before its line coverage, and once project passages compete
the order is coverage first, the band only a tiebreak. The 32 MB vault
budget is charged only for notes that match, after the rest are skipped, so
newer unrelated notes can't starve an older exact match (until 2026-10-06
every eligible note was charged before the skip); a 256 MB read limit bounds
a scan with no index. With
semantic search on (the app only), notes whose best chunk is close in meaning
to the request (cosine ≥ 0.6, the bundled on-device model) join the lexical
candidates by reciprocal-rank fusion. Profiles,
workflow prompts and conversation/session documents are excluded from automatic
retrieval; workspace outputs remain reusable. No per-turn connector network
refresh is added, so local connector notes may be stale remotely.

The deterministic retrieval uses up to 16 meaningful prompt terms (longer than
two bytes, stopwords removed), de-duplicated in prompt order, and all of them
reach both the project and the vault half. Matching is OR-style coverage, not
all-terms: prompts are whole sentences. Project and vault lines share one score
(`vault_search::LineScorer::best_lines`, one scorer per request: each term is
searched once per file body instead of once per line, with exactly the answer
of the line-by-line reference `best_lines_by_line`; run 15 S1): coverage is the
number of distinct terms in the
line plus terms in the file's name. For projects the name is the path under the
Projects root; for vault notes it is the file name and title, never the folder
(`tasks/`, `calendar/` are schema words). Numeric terms such as `2026` count
only inside lines, so date-slugged file names don't flood the evidence. Only
lines that match on their own are cited, so blank lines and `---` fences never
become passages. A frontmatter line matches on its value only, so a prompt
word that is also a key (`status`, `project`, `due`, `subject`) doesn't turn
every note's `status:` line into evidence. A name-only file contributes one
passage at its first body line. Equal scores interleave vault and project
passages instead of sorting by absolute path; inside a tie, vault passages run
newest first by mtime and project passages by path, so a recurring meeting cites
its latest instances rather than its oldest date slugs. The whole candidate set is
ranked before anything is cut: up to three lines per file, a file's third and
later passages ranked behind every other file's first two, the first 24 handed
to the context prioritizer, and then one evidence budget of eight passages,
900 characters each, with a notice when ranked passages didn't fit. External text scanning is
bounded per request, independently of the inventory's size: at most 4,000 files
and 32 MB are read (`RETRIEVAL_FILES`, `RETRIEVAL_BYTES`), files whose name
matches a term first, then the most recently changed, and a warning states how
many text files were read out of how many when the budget is hit. After the
text files, `extracted` documents whose name matches a term are read through
the cache, at most 40 files and 16 MB on disk per request
(`RETRIEVAL_DOC_FILES`, `RETRIEVAL_DOC_BYTES`; best name match, then newest),
and cited as ordinary `excerpt` passages with line numbers counted in the
extracted text (what `read_source` and Library show); documents past the cap
keep a line-0 passage and a warning says how many were read. Without the
content index (below), a document is never read for content alone. A listed file
it can't read contributes a line-0 metadata passage when its name matches:
`requires_reader` for an image or a document whose read failed (the excerpt
says why, e.g. image-only PDF), `too_large` for text over 1 MB. Results include the
absolute original path, one-based line, excerpt, source kind and coverage. Vault
passages cite positive line numbers; line zero means only an unread file's
metadata, never an asserted line citation. A filename match can disclose a
binary source without claiming its body was read. Evidence is JSON-encoded with
delimiter escaping and labelled untrusted data, not instructions. This bounded
evidence reaches all adapters, including tool-free runtimes. It does not widen a
runtime's file permissions or grant it write access.

**The content index** (repo features R14b, `src-tauri/src/project_text_index.rs`)
lets that project half choose files by what they say, once the user turns it
on (it is off by default, below). It is a SQLite file per
vault and Projects folder, `{app support}/SearchIndex/<vault stem>.projects-<16
hex of the canonical Projects path>.db` (the vault stem is `vault_file_stem`'s
`<name>-<16 hex>`, which never holds a `.`, so the name says whose it is and
never looks like a vault index), created mode 0600 (its `-wal`/`-shm` too), WAL
with R7a's 16 MB `journal_size_limit`, `secure_delete` on: `files(id INTEGER
PRIMARY KEY, path UNIQUE, size, mtime, kind, units, chars, error)` and a
contentless FTS5 `words(path, name, body)` table (`unicode61 remove_diacritics
2`, rowid = `files.id`). Contentless only halves the file (no second copy of the
text); the positional index still holds every word, so a pass that drops rows
merges segments under `secure_delete` (freed pages zeroed): a full FTS5
`optimize`, after which a dropped file's words aren't readable in the file, when a
dropped row's file is now in a restricted folder or secret-named (matched by name,
size and mtime when it moved) or more than 5% of rows went; otherwise a bounded
`merge` step of 500 pages, so a file that comes and goes doesn't rewrite a large
index every minute (its words may then sit in unmerged segments a while). Office's
`~$` owner/lock files are left out of the inventory itself (`excluded`), so they
are neither listed, read nor indexed. It is a derived cache; deleting it means a
rebuild. After each successful refresh the worker runs `reconcile` on a blocking
thread: rows for files removed, left out or no longer readable go; changed or new
`text` and `extracted` files are read newest first, at most 1,500 files or 48 MB
on disk per pass (the first file whatever its size), after a `confirm` walk over
the batch (a file that fails it is skipped this pass, not dropped; the next
refresh drops it if it is gone); unchanged files (same size and mtime) are never
read again. It never holds text a read would refuse, and more: a file with a
`restricted` folder anywhere in its path (any case) or a component `secret_name`
matches is skipped and counted (`skipped`), even though reads serve the first.
**Crash-loop guard, two strikes:** PDFKit runs in-process, so a document's row is
committed as `parsing` before it is parsed and replaced after. A row still
`parsing` at the same size and mtime on a later pass (a crash, or a quit
mid-parse) is tried once more as `parsing (retry)`; one still `parsing (retry)`
after that is marked `crashed while parsing` and never parsed again until the
file changes. Retrieval reads a file in none of these states (a name match gets a
line-0 passage: "Hollow stopped twice while reading this file", or that an
interrupted read is being retried). Retrieval opens the file read-only
per request (the app and the `hollow_vault` subprocess alike) and uses it only
when every term is one word and the index holds at least 95% of the in-scope
`text`/`extracted` files it is for, as listed now. Then the candidates are the
BM25 top 200 (`bm25(words, 0, 2.0, 1.0)`, terms ORed as prefix queries like
OB12's `words_query`), name matches, the 20 newest, and any file the index
doesn't hold as it is now; text candidates are read name matches first, then by
BM25 rank, then newest, within the same 4,000-file and 32 MB budget. A PDF or
Office file is parsed without a name match only when the index ranked it or
doesn't hold it as it is now (one it holds, unranked, has no term in it, so
being among the newest is no reason), within the same 40-file and 16 MB cap.
Otherwise everything runs as above. The budget warning names the
path: "…candidate connected text files (… name matches, then ranked by contents
in the local index, then the newest)" against "…connected text files (… name
matches and the most recently changed first)". Settings → Sources → Hollow folder →
**Search inside project files (local index)** turns it on or off
(`connected_index_status`, `set_connected_index_enabled`); it is off until the
user turns it on (since 2026-10-19). On writes `<vault stem>.content-index-on`
beside the manifest and removes the legacy `<vault stem>.content-index-off` a
default-on build wrote, which still wins over the on marker while it exists.
Off removes the on marker and, once a running pass stops, deletes every
`<vault stem>.projects-*.db` of this vault with its `-wal` and
`-shm`, whether or not its Projects folder is reachable (unmounted, moved,
disconnected), and any `-wal`/`-shm` of this vault left without its `.db`; a
pass while it is off deletes them too (an index a default-on build left).
Connecting another Projects folder deletes this vault's index of the old one,
unless an indexing pass is running (`connect` never waits on one while it holds
the connection's lock); then the next pass deletes it, as any pass that finds the
Projects folder disconnected or changed does. An unreachable folder's index is
kept for when it is back. Moving or renaming the **vault** folder changes its stem,
so Off and those deletions no longer find the old vault's index; Remove old
indexes… then offers it as another vault's once it has gone unused an hour. Its
progress line reads "12,340 of 15,002 files · 41 image-only PDFs" and is polled
only while the Hollow folder section is open and the index is still filling in.
Library's coverage line adds "Contents indexed: N of M". Settings → Advanced →
Hollow's databases lists it as its own kind, `projectIndex`, whose by the vault
stem in its name; another vault's, unused for an hour, is offered by Disk space →
Remove old indexes… like that vault's search index, this vault's never.

Its companion, `read_source` (agent effectiveness F3 part 2, 2026-10-06), reads
the rest of a connected project file retrieval cited: numbered lines, at most
600 lines or 24,000 bytes per call, by the file's absolute path, only inside the
run's `HOLLOW_VAULT_SCOPE` project (every connected project when unscoped) and
only after the same live membership check `read` uses, so ignored, excluded and
secret-looking files stay unreadable. A PDF or Office file reads as its
extracted text, numbered as the passages that cite it (R14a); so does one in a
workspace's `sources/` (see SOURCE_CONTEXT.md).

The owned `search_vault` MCP tool also returns connected evidence for targeted
follow-up queries: the same broker's ranked passages, cut to its `limit`
instead of the eight-passage budget. It always searches the configured vault root
(`HOLLOW_VAULT_ROOT`), wherever the chat runs: the vault, a workspace folder or
a build worktree outside the vault. Its connected evidence is scoped by a
separate field (`HOLLOW_VAULT_SCOPE`, from `retrieval_broker::resolve_scope`,
the rule automatic retrieval uses), so a project's workspace or worktree chat
sees only its project there too, and the same note carries the same paths in both. Until
2026-10-06 the server was rooted at the chat's cwd, so a workspace chat
searched only its own folder (agent effectiveness F1). Its keyword half follows `vault_search`: every word must
appear in any order across the file name/title and body, `"double quotes"` keep
an exact phrase, 1-3 letter words match at word starts, filler words are
ignored, at most 8 terms are used, and ranking puts file name/title before
headings before body. When nothing matches literally, conservative typo
recovery matches file names, not frontmatter titles. Its `kinds` apply before
ranking and both caps; `kinds: ["connected_project"]` restricts connected evidence to
project files. Its connected evidence carries connection and project-budget
warnings but not the vault half's size and budget notices: the keyword half
already reads every note with no size cap or budget, and a notice that is true
of the vault rather than the query would otherwise replace every "No vault notes
matched" reply. The MCP subprocess opens the app's FTS5 database read-only
(it never writes or reconciles it; a note changed since it was indexed is
always read) and scans linearly only when there is none. It has no embedder,
so its passages are lexical and its answer says so. Its existing toggle and provider
permission model are unchanged. Automatic per-request retrieval is independent
of whether the runtime has MCP tools. No paid reranker or external service is
required; the semantic blend is optional and on-device.

## Repository diff receipts (Paperclip PR 16)

A chat turn or workflow run records what changed in its target git
repositories next to the vault diff automated history already takes
(`src-tauri/src/repo_diff.rs`). The vault itself may be a git repository; its
own repository is never diffed.

**Targets.** Both the vault and the run's cwd are canonicalized.

- (a) Unless the cwd IS the vault root (no git runs at all then: its toplevel
  could only be the vault, an ancestor or nothing), the cwd's git toplevel,
  unless it is the vault or an ancestor of the vault. A repository nested
  inside the vault is a target: automated history skips dot-paths and files
  over 2 MiB, so its `.gitignore`, `.github/**` or large-asset changes would
  otherwise be invisible. Its rows that vault history did record are dropped
  from the receipt (matched through the repository's vault-relative prefix),
  so nothing is counted twice.
- (b) For a `vault/workspaces/local-<24 hex>` cwd (canonical, or as given, so a
  workspace folder symlinked into its project dedupes), the linked project from
  the project index, but only when its canonical path equals its own
  git toplevel. A plain folder under a repo-backed Projects root therefore never
  diffs the enclosing repository. Projects and Vault never overlap (`connect`
  refuses it), so a (b) target is never nested; nested repositories arise only
  from (a).

Targets are deduplicated by canonical root, at most two. The heartbeat and
ingest are not wired (they always run in the vault root). A teammate
`ask_agent` delegation with its own `cwd` is not a target either, so a receipt
that shows no repository changes says nothing about delegated repository work.

**Timing and diff model.** The pre-run snapshot runs after admission (the lease
is held) and before the spawn, never under `HISTORY_LOCK`: HEAD (none when
unborn), `status --porcelain=v1 -z --untracked-files=all
--ignore-submodules=all --no-renames`, and a signature per dirty or untracked
path (sha256 within a 64 MiB budget, else length and mtime; a secret-named path
only ever gets length and mtime). The baseline is armed only after a
successful spawn, so a run stopped or failed before it does no post-run git
work. The post-run capture runs after the run's processes ended, its usage row
finished and orphaned delegate rows closed, and BEFORE the lease is released,
so the next admitted run can never land in this receipt; it ignores the run's
stop signal. The changed set is: post entries whose status or signature
changed, pre entries now clean (`status: clean`, `dirtyBefore`), and paths
committed between the two HEADs (`diff --raw`, gitlinks marked). A file dirty
before the run and untouched is left out; one edited further carries
`dirtyBefore`, and its patch includes the earlier edits.

**Caps and batching.** 200 rows per repository (`totalFiles` keeps the real
count), 512 KiB per file read, 256 KiB per patch, 1 MiB of patches in total,
10 s per git call, 15 s for the pre-run and 30 s for the post-run capture, and
at most two git processes at once. Tracked patches come from ONE `git diff
--numstat -z` and ONE capped `git diff` per repository (split on `diff --git`
headers with C-unquoting), so a capture costs about five to seven git spawns
per repository. Truncated status or raw output makes the repository
`unavailable` (a `repo` failure row on the receipt); a patch stream cut by the
cap keeps its partial last section (`patchTruncated`) and sets `truncated`;
running out of time after status keeps every row without patches and sets
`truncated`, never `unavailable`.

**Sidecar, receipt, attribution.** The run's
`.hollow-history/runs/{runId}.repo.json` holds `{schemaVersion: 1, runId,
capturedAt, repos}` with patches, written only while the lease names the run.
It is display-only (restore never reads it), pruned with its run, swept when
orphaned, counted toward history's 512 MiB ceiling and removed by Clear
history. The receipt's `repoChanges` carries the same rows without patches, at
most 50 per repository. A repository row is `confirmed` only when a successful
structured write named it exactly: under one of the repository's alias roots
(its canonical root, the cwd as given minus its in-repo prefix, the linked
project path as stored) or relative to the cwd inside the repository. A run
with observed, unlisted or unrecorded repository changes is `unknownOrder`;
`noChanges` needs no vault file of its own (a `shared` file is another chat's,
see [WORKSPACES.md](WORKSPACES.md)) and every repository captured with nothing
changed. The vault matcher no longer lets an absolute repository path confirm a
same-named vault file. A structured write is Edit, Write, MultiEdit,
NotebookEdit, apply_patch (its `Move to:` destination too) or (since
2026-10-20) Codex's `file_change` item. Since 2026-10-20 a vault file is
confirmed only by the file such a write really reaches (`evidence::own_writes`):
the path, relative to the run's folder, is resolved by the file system
(symlinks, `..` after them, the stored letter case) before it is compared; a
deleted file's name matches History's in another case only on a volume known
to ignore case (`automated_history::case_insensitive_at`, `pathconf`), and the old
suffix match is gone (a relative `workspaces/w1/report.md` written from
inside `workspaces/w1` confirmed the folder's own `report.md`).

**Secrets and external content.** A path any component of which is a secret
name (`directory_connections::secret_name`: keys, credentials, tokens, `.env`
and `.env.*`, `.envrc`, `.netrc`, `.npmrc`, `.pypirc`, `.htpasswd`,
`.dockercfg`, `.pgpass`, `kubeconfig`, and anything under `.kube`, `.docker`,
`.aws`, `.ssh` or `.gnupg`) is listed as `withheld`: its content is never read,
hashed into a patch or stored. Other dotfiles (`.gitignore`, `.github/**`,
`.editorconfig`) are read like ordinary source. Every stored patch passes
through `claude_runner::redact`, which is a best-effort pattern list, not a
security boundary: a secret in a file with an ordinary name and no recognised
shape (an AWS or Stripe key, say) is stored as written. Ordinary source is still copied into the
vault's `.hollow-history`, so a git-backed vault should ignore that folder.

**Hardening.** Git runs read-only and never executes anything the repository
planted: see the [quirks archive](history/CLAUDE-archive-2026-10-quirks-and-reference.md#quirks-to-know-about) → "Hollow's repository capture never runs a
program the repo planted". Nothing is ever staged; untracked files are read
directly (regular files inside the root only, never following a symlink,
never blocking on a FIFO). Restore stays vault-only.

## IPC and verification

- `directories_status()` → Connection (last inventory plus availability warnings
  and `pendingRestart` computed against the active process vault).
- `connect_directories(projectsPath, vaultPath)` → Connection; persists paths,
  additive workspace creation; `pendingRestart` controls the restart handoff.
- `refresh_directories()` → Connection; safe to repeat.
- `connected_files(query?, projectId?, fileState?, offset?, limit?)` →
  `FilePage {total, offset, files, indexed, omitted, direct, refreshedAt}`
  (P07): one page (default 100, at most 500) of the listed inventory; every
  word of `query` must appear in the path, name matches first; `fileState` is
  `text`, `extracted`, `tooLarge` or `needsReader`. The first page of a query adds `direct`
  lookups outside the listed inventory. App action `vault.connected_files`.
  The Connection itself no longer carries `files`.
- `read_connected_file(path)` → current UTF-8 text after rechecking membership
  (a fresh walk of the file's own folders, so a file past an inventory limit is
  still readable), canonical path and size; for a PDF or Office file, its
  extracted text after the same checks (or why it can't be read). Extraction
  runs on the blocking pool. It never accepts arbitrary external paths.
- `read_history_repo_changes(runId)` → `RepoSidecar | null` (Paperclip PR 16):
  the run's repository rows with their bounded, redacted patches, read from
  `.hollow-history/runs/{runId}.repo.json` (symlinks and files over 4 MiB are
  refused). Display-only.

Rust temporary-filesystem tests exercise empty/populated setup, invalid paths,
config/scaffold preservation, repeated setup, ignore rules/worktrees, symlinks,
changes/deletions, missing roots, binary/large coverage and live agent context.
P07 tests cover a relevant file after item 2,500 of one project being listed,
found by name and cited with the real limits; an over-share project's file found
directly and read by path; the inventory bound counted and warned; pagination
and name-first ranking; the incremental refresh (reused metadata, no rewrite
when nothing changed, a changed folder re-read, a stale full-stat stamp); a file
ignored after the last refresh never served; the retrieval budget and its
warning; and a pre-P07 manifest's inline inventory adopted and then dropped.
Retrieval tests cover every prompt term reaching the vault, the stripped chat
payload versus the raw one (and `context()` on the raw payload), a close tag
inside the page note, vault lines outranking weaker project lines,
interleaving of equal scores, newest-first vault ties, positive vault line
citations, no blank, fence or key-only frontmatter passages (including a status
question), the profile/workflow/session exclusions, kinds applied before the
cap, folder- and number-blind name coverage, oversized-note skipping, and BM25
candidates scored exactly as the linear scan scores them (including stem-only
notes), bounding the notes read and outranking line coverage. The MCP tests
drive real `tools/call` dispatches against connected fixture files, including
the multi-term grammar, kinds before the cap, and a note over 1 MB leaving the
empty-result reply intact.
Repository diff receipts are tested against real `git` in temp directories
with isolated config (`repo_diff_tests.rs`): a git-backed vault gets no
repository diff for a vault-cwd run and spawns no git; the vault's repository
and its ancestors are never targets; a nested repository reports only what
vault history missed; a linked-project run records a modify, an untracked
create, a delete and a commit and stores the sidecar; dirty-before files,
every cap and a mid-patch budget; the index is never rewritten (including by
`git diff <commit>`'s auto-refresh when two reverted files are stat-dirty,
which `diff.autoRefreshIndex=false` stops); hostile
repository config (fsmonitor, filters including an `x=y` driver name,
textconv, a promisor `ext::` remote, a submodule filter, a post-run
`core.worktree`) never runs; FIFOs, symlinks and secret-named files are never
read, including `.envrc` and `.kube/config`. `automated_history` tests cover sidecar retention, the storage ceiling,
clear and the lease check; `evidence` tests the verification rules.
Browser checks use fixture IPC and establish desktop/narrow layout and interaction,
not native picker behavior, live account freshness or a live provider run.

## Workspace icons

`project_icons.rs` (`workspace_icons` IPC, fetched by `store/workspaces.ts`
only when the set of workspace ids changes) gives each workspace that maps to a
connected project an identity mark, rendered by `components/ProjectIcon.tsx` in
the rail, the overview cards and the workspace header: the project's own app
artwork (favicon / apple-touch-icon / Tauri `src-tauri/icons/128x128.png` /
app icons, a fixed list of paths inside the project, ≤ 512 KiB, magic bytes
checked, never through a symlink leaving it, returned as a `data:` URL) with
the GitHub owner's avatar as a small badge; else the owner's avatar alone, so
organization (`example-org`) and personal (`example-user`) repos read
differently; else a folder glyph. The owner comes from `.git/config`'s
`[remote "origin"] url` parsed directly (no git process); only github.com
remotes count. Avatars come from `https://github.com/<owner>.png` through
the backend's `fetch_remote_image` (the app CSP allows no remote images), and
a failed image falls back a step. `local-` workspaces map
by id; any other workspace whose title matches exactly one connected project's
name (letters and digits, case-insensitive) uses that project's icon. Results
are cached per project by modification time.
