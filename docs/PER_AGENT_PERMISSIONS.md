# Per-agent permission modes

Since 2026-11-12 a chat's permission menu shows the chat's own agent's native
modes, with that agent's real names. Hollow keeps one canonical set underneath
(`plan`, `default`, `acceptEdits`, `auto`, `bypassPermissions`,
`app_mcp::PERMISSION_MODES`, weakest first) and passes each straight to the
agent's CLI; the names are presentation only. The argv per mode is the table in
[agent-tools → Permission modes across agents](reference/agent-tools.md#permission-modes-across-agents-2026-10-02).

Hollow's own cards (delete, send, secrets, install, the always-ask list) sit on
top of every mode and are unchanged: `app_actions::asks_in_mode`,
`mode_contract::cards`.

## What each menu says

One table, `src/agentPermModes.ts` (pure, `src/agentPermModes.test.ts`). Every
canonical mode resolves to exactly one option per agent, or is unavailable.

| Agent | Menu (canonical value) |
|---|---|
| Claude Code | Default (`default`), Accept edits (`acceptEdits`), Plan (`plan`), Auto (`auto`), Bypass permissions (`bypassPermissions`) |
| Codex | Read only (`plan`), Ask for approval (`default`; a stored `acceptEdits` shows as it, same sandbox), Approve for me (`auto`), Full access (`bypassPermissions`) |
| Grok | Default, Accept edits, Plan, Auto, Bypass permissions; its `dontAsk` has no canonical mode and isn't offered; its Auto runs with the sandbox off, so it counts as danger |
| Antigravity | Accept edits (`default` shows as it), Plan, Auto (listed, unavailable: its sandbox holds only shell commands), Skip permissions (`bypassPermissions`) |

The composer's access chip, the model menu's Permissions section, `/permissions`
and the danger confirm all read the table. `/permissions` also takes the old
words (bypass, edits) and canonical values.

## Changing a chat's agent

`switchChatAgent` (model menu, `/agent`, Retry with another agent) moves the
chat's mode to the nearest one on the new agent and shows one line, for example
"Permissions: Accept edits on Claude is Ask for approval on Codex." It never
moves a chat into a danger mode it wasn't already in. Restoring a chat leaves its
mode alone.

## Defaults

Settings → Chat has one mode for new chats per agent (`newClaudePermissionMode`,
`newCodexPermissionMode`, `newGrokPermissionMode`,
`newAntigravityPermissionMode`). Until one is saved: Auto for Claude and Codex,
Accept edits for Grok and Antigravity (Auto isn't available there), mapped to a
mode the agent can run. The single "mode for new chats" from before is retired:
it was re-seeded from the last chat's mode, so it was never a deliberate choice.
Existing chats keep their recorded mode.

## Codex Auto for builds

Codex's `auto` is `--approve-for-me`: `approval_policy="on-request"`,
`approvals_reviewer="auto_review"`, `workspace-write` sandbox. It now also passes
(exec, resumed exec and the app-server, one code path, `agent.rs`
`codex_auto_sandbox_overrides`):

```
-c sandbox_workspace_write.writable_roots=["<extra_dirs>"]
-c sandbox_workspace_write.network_access=true
```

In a checked linked worktree (the git folder is granted) it passes a **permission
profile instead** (2026-11-12, `agent.rs` `codex_auto_profile_overrides`), with no
`-s`, no `sandbox_mode` on a resumed exec and no thread `sandbox` on the app-server
(a thread's `sandbox` overrides a profile; `codex_app_server_start` leaves it empty
and the profile rides process config, probed with `thread/start`, codex-cli 0.167.2).
The worktree is checked once per run (2026-11-13): the thread reads the decision
from the spawned command's own argv (`codex_command_uses_profile`), never from a
second check, and names its mode's sandbox unless that argv carries the profile
(`codex_app_thread_sandbox`), so a layout change between the two can't leave a
thread with neither:

```
-c default_permissions="hollow_auto"
-c permissions.hollow_auto.extends=":workspace"
-c permissions.hollow_auto.filesystem={"<extra_dirs>"="write","<common>/worktrees/<name>"="write","<common>/objects"="write","<common>/refs"="write","<common>/logs"="write","<common>/worktrees/<name>/commondir"="read","<common>/worktrees/<name>/gitdir"="read","<common>/worktrees/<name>/config.worktree"="read","<worktree>/*/.git"="deny","<worktree>/*/**/.git"="deny"}
-c permissions.hollow_auto.network.enabled=true
```

The two `deny` globs mean **no nested `.git` can be created** under the worktree
(no `git init`, no `.git` file), so a planted submodule entry has no decoy repository
to run (probed live: refused, commits still pass). The top-level `.git` the worktree
needs is not matched. Cost: a nested repository already in the worktree (a vendored
clone) is unreadable to the Auto turn.

`:workspace` keeps the cwd, `/tmp` and `$TMPDIR` writable and each root's `.git`
read-only, as `workspace-write` did. The profile carves `commondir` and `gitdir`
out of the writable git folder, which a root can't express. Any granted path with
a glob character (`*?[]{}\`) keeps the `workspace-write` form (Codex reads such a
key as a glob and refuses the profile: "only supports `deny`"). A remote runner
turn never gets the profile. Plan and bypass never do; default and acceptEdits get
the cut-down one below (N22).

**Edit modes (N22, 2026-11-15, `agent.rs` `codex_edit_profile_overrides`).**
Codex `default` and `acceptEdits` (local only) run the same `:workspace` profile
with **only** the nested-`.git` deny and the network off, in place of `-s
workspace-write` (no `-s`, no `sandbox_mode`, no thread `sandbox`, like Auto's):

```
-c default_permissions="hollow_auto"
-c permissions.hollow_auto.extends=":workspace"
-c permissions.hollow_auto.filesystem={"<cwd>/*/.git"="deny","<cwd>/*/**/.git"="deny"}
-c permissions.hollow_auto.network.enabled=false
```

No roots, no git folder, no network: what these modes could write is unchanged,
except that a turn can no longer create a decoy nested repository in place (the
planted-submodule route Auto closed on 2026-11-12). **Known gap (review of N22,
2026-11-15):** the deny matches the created or renamed path only, and `/tmp` and
`$TMPDIR` stay writable, so `git init /tmp/d && mv /tmp/d <worktree>/sub` passes in
Auto and in the edit modes alike (verified with `codex sandbox` 0.169). Not a
regression for the edit modes (they had no deny); the index guard and
`harden_common_submodule_ignore` still apply. The live fixture pins it
(`MOVED_IN_GIT_OK`, asserted present so a fix flips it). Fix idea, a later row: make the
temp folders read-only except the run's scratch folder.

**Open: a resume after bypass may not restate the sandbox (untested).** A resumed
exec re-states a sandbox only through `sandbox_mode` (none under a profile), and an
app-server thread under a profile names none in `thread/resume`. If Codex keeps the
stored `danger-full-access` of a thread that ran in bypass, a resumed Auto, default or
acceptEdits turn would still be unsandboxed, now for the edit modes too. No free
check exists (a stored thread needs a model call to create), so it is untested; the
probe is one bypass turn, a switch to acceptEdits, and a `touch` outside the cwd. The deny list is one function shared by both
(`codex_nested_git_denies`). A cwd with a glob character keeps `-s workspace-write`.
Auto outside a checked linked worktree (no git folder granted) still runs
`workspace-write` and has no nested deny.

**What the git grants allow, per mode** (Codex; the branch-switch and refs route
from the 2026-11-12 report, N22: keep as is, documented):

| | plan | default, acceptEdits | auto, checked linked worktree | bypass |
|---|---|---|---|---|
| Commit, `git add` | no (read-only) | no (the worktree's git folder is outside the sandbox) | yes (worktree git folder and `index`) | yes |
| `git switch`/`checkout` to another branch | no | no (cannot write `HEAD` or the index) | yes: `HEAD` and the index are in the worktree git folder, so the branch checks out; the working tree files change within the cwd | yes |
| Move or delete a ref (any branch, main included), write any object | no | no | yes: `refs`, `objects` and `logs` of the shared git folder are granted whole (narrower breaks fetch and new branches); a packed ref cannot be deleted or packed | yes |
| Shared `config`, `hooks`, `commondir`, `gitdir`, `config.worktree` | no | no | no (read-only or outside the grant) | yes |
| Nested `.git` created or renamed in place (git init, `.git` file, `--separate-git-dir`) | no | **no (N22)**; a repo built in `/tmp` and moved in is not caught (gap below) | no, same gap | yes |
| Network (push, `gh`, registries, runner) | no | no | yes | yes |

**Branch switch.** An Auto turn may switch branches in its worktree. The index
guard's allowed set of submodule entries is fixed at the worktree's first arm, so
after a switch to a branch that has a submodule the first `HEAD` lacked, Hollow
unstages that gitlink while it runs (files untouched; `git restore --staged`, or
restart the worktree's guard). Refreshing the set from the current `HEAD` was
rejected: the turn can write `HEAD`, so a refreshed set is forgeable. **Refs.**
Any branch is movable by an Auto turn by design; an Auto turn that must not touch
`main` should not run in a worktree of the repository that holds it, and the
person's cards and review of what lands (the PR) stay the control, not the sandbox.

- `extra_dirs` is the chat's own folders (a build workspace's vault folder). A
  build chat already starts with its worktree as the working folder, for every
  agent.
- A linked worktree's git data lives outside the working folder and Codex blocks
  writes to it, so a commit needs the worktree's own git folder
  (`<common>/worktrees/<name>`) as a root plus the shared object and ref folders.
  Only `objects`, `refs` and `logs` of the shared git folder are granted, never
  the folder itself, so its `config` and `hooks` stay read-only (a hook or
  `core.fsmonitor` written there would run unsandboxed at the next git call).
- **The git folders are validated before anything is granted**
  (`checked_linked_worktree`), because `<cwd>/.git` and the files it names can be
  written by an earlier Auto turn. All paths are resolved through symlinks
  first, and any failed check grants no git folder (and logs nothing):
  `<cwd>/.git` is a regular file, not a symlink; its `gitdir:` is
  `<common>/worktrees/<name>` outside the working folder; that folder's
  `commondir` READS exactly `../..` or the canonical `<common>`, and its `gitdir`
  file reads exactly the canonical `<cwd>/.git` (one trailing newline allowed;
  both read without following a symlink). The text is checked, not only what it
  resolves to: a path through a symlink the agent can repoint resolves right at
  the check and to a decoy later. `<common>` has `HEAD`, `objects` and `refs`, sits outside
  the working folder and has no `extensions.worktreeConfig` (which would make a
  file in the writable worktree folder a git config); the folder holding
  `<common>` is not `/`, the home folder or above it, and holds none of the
  chat's `extra_dirs` (a repository around the vault is not a build's); and
  `objects`, `refs` and `logs` don't lead out of `<common>`. A main checkout or a
  submodule gets no git folder. The checks read only Hollow-chosen paths plus
  those files: `agent.rs` has no vault, so Hollow's own build-workspace binding
  isn't consulted yet (a follow-up could pass it through `RunExtras`).
- Measured under the narrowed roots (`codex sandbox`, codex-cli 0.167.2, a
  linked worktree with a `git daemon` remote on loopback): commit, new branch,
  fetch (FETCH_HEAD lands in the worktree's own git folder), push of a branch
  all work. `git push -u` pushes and exits 0 but can't record the upstream
  (`could not lock config file`). Deleting a packed branch and `git pack-refs`
  are refused (`packed-refs.lock` sits in the shared folder). Writes to the
  shared `hooks/`, `config` or any other file at the shared folder's top are
  refused.
- **The `objects` and `refs` grant lets an Auto turn move any branch of the
  repository, main included** (and write any object). Granting only
  `refs/heads/<branch>` would break fetch (`refs/remotes`) and new branches, so
  it isn't narrower.
- **`commondir` guard** (`agent.rs` `arm_codex_worktree_guard`, delta and final
  reviews 2026-11-12). The worktree's own git folder is writable, and it holds
  `commondir` and `gitdir`, which git reads on every call. An Auto turn could
  point `commondir` at a decoy folder whose `config` sets `core.fsmonitor`, and
  the next unsandboxed git in that worktree (Hollow's or the person's) would run
  it (probed: it does). So when a local Codex Auto command is built:
  - If either file's text isn't git's own (left while no guard ran: Hollow quit
    or crashed, and Codex can outlive it), Hollow writes git's own text back
    (`../..` and `<cwd>/.git`), derived from `<cwd>/.git` alone (`dot_layout`:
    `worktrees/<name>`, outside the working folder, a real common folder and the
    other checks), never from `commondir` or the back-pointer, so both files
    broken at once are repaired too. The one exception: a back-pointer naming
    another live worktree whose `.git` claims this git folder is left alone, and
    nothing is granted.
  - Hollow then snapshots that canonical text only. A guard thread puts both
    files back every 200 ms while Hollow runs, whatever way the turn ends. A
    rewrite, a symlink or a folder in their place all count.
  - Hollow runs unsandboxed, so the guard never follows a link: it reads with
    `O_NOFOLLOW` and writes a fresh `create_new` temp file beside the target,
    then renames it over the entry (a rename replaces a planted symlink itself,
    never its target; a folder in the way is removed with `remove_dir_all`,
    which doesn't follow links).
  - Hollow's own git in that worktree (`build_workspaces::git_command` sets
    `GIT_DIR`, `GIT_COMMON_DIR` and `GIT_WORK_TREE`, `repo_diff` sets
    `GIT_COMMON_DIR`) puts the files back first and is pinned to the recorded
    folders, so it never reads `commondir`. With no snapshot (no Auto turn
    since Hollow started), `build_workspaces::git_command` still pins a linked
    worktree from `<cwd>/.git` alone (`worktree_git_pins`), and gives a linked
    worktree whose structure doesn't hold (no real common folder,
    `worktreeConfig` on) no git at all (`GIT_DIR` set to a folder that can't
    exist). A plain checkout or a submodule runs git as before. `repo_diff`
    without a snapshot isn't pinned this way; it already forces
    `core.fsmonitor=false` and `core.hooksPath=/dev/null` on the command line.
  - A guard pass does its file work outside the lock (a slow volume must not
    stall every Hollow git call); a `forget` (before `git worktree remove`)
    sets the snapshot's flag, checked right before each write, so it can't be
    undone mid-pass.
- **Index and submodule route** (review 2026-11-12). The working folder is
  writable, so an Auto turn can add a submodule entry to the worktree's index
  (`git update-index --add --cacheinfo 160000,<sha>,sub`) and write `sub/.git`
  naming a decoy whose `config` sets `core.fsmonitor` (Codex protects only the
  top-level `.git`). Plain `git status` then starts a child git in `sub/` and
  runs it (probed: it does). Every Hollow git through `build_workspaces::git_command`
  (Changes, Commit, Push, Update, worktree hygiene, self install, the Hollow
  repo) runs with `-c core.fsmonitor=false -c diff.ignoreSubmodules=all
  -c submodule.recurse=false -c status.submoduleSummary=false`, which git
  forwards to any child git; hooks stay on. No caller needs submodule
  recursion: `git add -A` still stages a submodule's new commit, and the commit
  check asks `diff --cached --quiet --ignore-submodules=none` (a cached diff
  reads no submodule), so a submodule-only commit still goes through. The
  Changes list no longer shows a submodule whose checkout moved but isn't
  staged. Probed for the Commit path too (planted gitlink, decoy with a
  `core.fsmonitor` and a `filter.x.clean` payload, `sub/.gitattributes`
  `* filter=x`): with the flags, `add -A`, `commit`, `merge`, `rebase` and
  `checkout` ran no payload; plain `status` and `add -A` ran the fsmonitor one
  (the clean filter never ran, even plain). A commit whose only change is a real
  submodule bump still commits under the flags. So no `ls-files` pre-check was
  added. `repo_diff` already runs with `core.fsmonitor=false` and
  `--ignore-submodules=all`. `remote_folder`'s git runs in the vault folder
  copies, not build worktrees, and is unchanged. Since 2026-11-13 (Q-R7) two
  more of Hollow's own unsandboxed git callers carry the same flags: every
  local git of the remote runner preparation and apply
  (`remote_session::git_env`: the snapshot's `status` and `add -A`, the
  apply's `read-tree`, `reset` and the rest, with their own `GIT_*` overrides
  kept) and the gardener's idle-worktree scan (`gardener::scan_git`, now
  `build_workspaces::git_command`, so also `GIT_*` stripped and a linked
  worktree pinned), and `install_ready`'s git (`git_bytes`, the same helper;
  its clean check reads the checkout named in the agent-writable
  `src-tauri/build-provenance.json`). Their `status` calls pass
  `--ignore-submodules=dirty` (a command-line option, so a planted
  `.gitmodules` `ignore = none` can't override it), so
  a submodule's new commit still counts as a change while git never enters a
  submodule; a submodule's inner uncommitted edits no longer count (the
  runner snapshot doesn't carry them, and the gardener doesn't call that
  worktree busy for them). Residual: the snapshot copies the index as it is,
  so a planted entry the guard hasn't removed yet (up to one pass) travels to
  the runner as a tree entry; it runs nothing here.
- `worktreeConfig` is read as the `[extensions]` key (section-aware,
  case-insensitive, bare or true), not the word anywhere in the config. The
  check reads `<common>/config` as text and doesn't follow `[include]` or
  `[includeIf]`, which is git's own rule for `extensions.*` (the repository
  format is read without includes; probed, git 2.54: an included
  `worktreeConfig = true` leaves `config.worktree` unread). Belt and braces
  since 2026-11-13 (Q-R6): the profile makes `<gitdir>/config.worktree`
  read-only whether or not it exists (create, rename-over and symlink refused,
  commits still pass, probed with `codex sandbox`, codex-cli 0.167.2).
  - Before Hollow's own `git worktree remove` (and the create rollback) and
    `git worktree prune` (`build_workspaces`), the snapshot is dropped
    (`forget_codex_worktree_guard`: that worktree, or the repository's worktrees
    whose folder is gone), so the guard can't recreate files git is deleting.
    Hollow runs no `worktree move` or `repair`.
  - Since the profile above, `commondir` and `gitdir` are **read-only to the turn**
    (a write is refused, probed live), so the guard above is the second layer:
    it covers the `workspace-write` fallback (a glob character in a path) and a
    `commondir` left by an older Hollow. Residuals of the fallback only: the
    person's own git can read a tampered `commondir` for at most
    one guard pass (200 ms). The guard keeps restoring for as long as Hollow
    runs, not only while a turn is live, so a `git worktree move` or `repair`
    the person runs by hand in a guarded worktree is reverted until Hollow
    restarts. The snapshots live in memory, so a tamper in the last pass before
    Hollow quits stays until the next Auto spawn there repairs it. Hollow callers
    that run git in the worktree without these helpers (`remote_folder`'s
    `git_run`) aren't pinned and read the files as they are (put back within a
    pass).
  - **The index can't be read-only** (2026-11-12, probed with `codex sandbox -P`:
    with `"<gitdir>/index" = "read"` both `git add` and `git commit` fail with
    "unable to write new index file", because git replaces the file). So the
    planted-submodule route is closed two ways instead:
    1. `diff.ignoreSubmodules=dirty` in the common config
       (`harden_common_submodule_ignore`, at every Auto spawn, through
       `git config --file`; kept when already `all` or `dirty`, replaced
       otherwise). A turn can't write the common `config`. It stops git entering a
       submodule for `status`, `diff`, `log -p`, `checkout`, `switch`, `stash`
       and `commit` (probed, 11 commands; plain `status` ran the decoy's
       `core.fsmonitor` without it). Cost: in that repository a dirty submodule's
       inner changes no longer show in `git status` (a new commit still does).
    2. **Per-submodule `ignore` in a `.gitmodules` the turn writes beats
       `diff.ignoreSubmodules`** (review probe), and **`git add -A` (and
       `commit -a`) don't read the setting at all** and still enter
       the submodule (probed: the control runs the payload). So the guard also
       scans the index whenever its length or mtime changes
       (`scrub_planted_gitlinks`): any `160000` entry that `HEAD`'s tree didn't
       have when the worktree was armed is removed with Hollow's git
       (`update-index --force-remove`, `SUBMODULE_SAFE`, pinned folders), and
       arming removes one left while no guard ran. An embedded repository that
       `git add -A` stages by accident is removed the same way, and so is a
       deliberate `git submodule add` in an Auto turn (run that outside Auto).
    The first line of defence is the nested-`.git` deny above; the config and the
    scrub cover a decoy made before it (an earlier turn, or one moved in; the
    edit modes got the same deny on 2026-11-15) and are not airtight: the scrub's allowed set is fixed at
    the first arm and kept (the turn can write `HEAD`), and its change check is
    size, inode and ctime of the index, with a symlinked index removed. Since
    2026-11-13 (Q-R5) it also covers every `sharedindex.*` in the worktree git
    folder (name, inode and ctime, and a symlink target's): a split index
    (`git update-index --split-index`, no config needed) keeps its entries in
    a shared file the turn can write, and git reads it without checking its
    checksum, only the copied trailer, so a patched shared file added a
    `160000` entry with `index` untouched (probed, git 2.54). The scrub works
    on a split index as before (removal through a copy of `index`).
    **A decoy can still be moved in** (the deny checks the destination path only:
    `git init /tmp/x; mv /tmp/x <wt>/moved` works, probed by the round 2 review), so
    the scrub is the defence for that route: it removes the planted entry on a copy
    of the index renamed over it, which a held `index.lock` can't stop. The
    allowed set is fixed at the first arm, so a person who switches that worktree
    to a branch with a submodule the first `HEAD` lacked sees Hollow unstage it
    while Hollow runs (the folder is untouched; `git restore --staged` or a
    restart of the worktree's guard fixes it).
  - **Routes still open, by name:** (a) a moved-in decoy plus a planted `160000` entry (and a `.gitmodules` `ignore = none`, which beats the config) can sit in the
    index for up to one guard pass (200 ms) or until the next Auto spawn if Hollow
    quit or crashed first; during that time the person's `git status`, `add -A` or `commit -a` would run it. (b) A submodule the worktree
    had when first armed is never touched, so a decoy made in a non-Auto turn at a
    real submodule's path runs as in any repository with submodules; a thread
    started under an earlier plain `workspace-write` or resumed with a stored
    sandbox may not use the profile. (b2, closed 2026-11-13) The app-server
    used to decide profile or not twice; the thread now follows the process
    argv, so a layout change between them can't leave it with neither. (c) The `refs`/`objects`
    grant above. Everything else a planted git-folder file could run (`hooks`,
    `config`, `info`, `commondir`, `gitdir`) is read-only or outside the grant.
- Network on lets git, `gh`, package registries and the Mac mini runner work, so
  `hollow-slot` in an Auto Codex turn reaches the runner instead of falling back
  to this Mac. In Auto it also applies to remote runner turns, by design: the
  runner's `runner` account holds Alex's GitHub and gcloud logins.
- Codex 0.167.2 writes these paths into its macOS sandbox profile unescaped, so
  Hollow leaves out any folder whose path contains a double quote or control
  character; it stays readable.
- A remote runner turn gets only the runner's copy of the folders, never this
  Mac's paths, and no git folders.
- Only Auto changes its grants: plan stays read-only with no network, default and
  acceptEdits write the same places as before (they now also refuse a nested
  `.git`, N22), bypass is `danger-full-access`.

Proof: `agent_tests` `codex_auto_sandbox_lets_a_build_write_commit_and_connect_live`
runs the real `codex sandbox` with the flags `build_codex` emits (in a linked
worktree a write, a vault-folder write, a commit, a new branch and a loopback
connect pass; a write outside the roots and writes to the shared `hooks/` and
`config` are refused; under accept edits all but the worktree write are
refused). It skips itself when `codex` or seatbelt is missing.
The live test now runs the profile (`-P hollow_auto`, its `-c` values verbatim from
`build_codex`) and checks the commit, branch and network pass while a `commondir`
or `gitdir` write is refused. `no_profile_without_a_checked_worktree_git_folder`
covers the fallbacks and
`arming_the_guard_sets_ignore_submodules_and_plain_git_no_longer_runs_a_planted_fsmonitor`
runs real git against a planted gitlink and decoy (controls included).
`a_forged_or_odd_dot_git_grants_codex_auto_nothing` covers the forged layouts,
`a_tampered_worktree_commondir_is_put_back_and_hollows_git_ignores_it` the
`commondir` guard (it fails with the pin removed),
`a_commondir_through_a_symlink_is_refused_then_repaired_never_snapshotted`,
`the_guard_never_writes_through_a_planted_symlink` and
`forgetting_a_worktree_drops_its_guard` the final review's three cases,
`both_git_files_broken_hollows_git_runs_no_payload_and_the_spawn_repairs_them`
and `a_guard_pass_never_restores_a_forgotten_worktree` the last two,
`a_planted_submodule_in_the_index_runs_nothing_from_hollows_git` and
`the_sibling_exception_reads_a_relative_back_pointer_and_worktree_config_is_a_key`
the final round, and the
live fixture shows the turn can rewrite `commondir` and the guard puts it back.

## Grok sandbox network probe (R6, 2026-11-15)

Not run live, and why. Grok 1.0.79 has no command that runs a shell command inside
its sandbox: `--sandbox <profile>` only wraps a session, and a command reaches it
through a model turn (logged in as grok.com here, so it would work, and it is a
paid call, which this row did not make). What was checked for free: `~/.grok/docs/user-guide/18-sandbox.md`
says child-network blocking in `read-only` and `strict` is **Linux only (seccomp)
and a no-op on macOS**, and `workspace` allows network. So on this Mac the
`--sandbox read-only` Grok runs (plan, default) are expected to reach the network
from a command, unlike Codex's. Hollow's `grok_sandbox` leans on filesystem write
limits only; the network is not a limit it relies on. To settle it with one cheap
turn later: `nc -l 127.0.0.1 PORT` in one shell and
`grok --sandbox read-only -p "run: nc -z 127.0.0.1 PORT; echo rc=$?" --always-approve --max-turns 2`
in another, then `workspace` the same way; the doc's claim is confirmed if both
connect.

## A chat keeps its own mode

Turns Hollow runs for a chat without the person typing run in the chat's own
last mode (`session_store::last_permission`, `app_mcp::own_mode`; accept edits
only when none is recorded): COO wakes and check-ins, child-completion turns,
`wake_me` wakes, event triggers, automation-in-chat turns, `send_to_session`,
held agent sends, fresh follow-ups, routed sends and handover successors. A
caller may only lower it (`app_mcp::lowered`).

Accepted consequence (owner's decision, 2026-11-12): a send from a chat in a
weaker mode, or an unattended check-in or event trigger, can run a bypass chat
in bypass. Hollow's cards (always-ask actions, auto's asked classes, external
confirms) and `unattended::SAFE` still apply to that turn; the caller's mode
does not cap it.

| Path | Rule |
|---|---|
| COO wake, check-in, event trigger (`main_agent::wake_mode`) | the thread's own mode |
| `wake_me` wake (`woken_mode`) | the conversation's current mode, never above the turn that asked |
| Automation in a chat (`turn_mode`, `named_mode`) | the chat's own mode, lowered when the automation's file names a weaker mode (acceptEdits included); a file that names none never lowers it. Saving an automation always writes `permission_mode` (acceptEdits when it was left empty), so an automation saved from Hollow caps its turn at accept edits; only a hand-written file without the key follows the chat's mode. This safe-side cap is kept on purpose |
| `send_to_session` (`send_mode_lifted`) | the target's own mode, or the sender's own when that is stronger (2026-11-20: a chat sender, not unattended, not plan; on another agent held to what the sender's mode could do, `agent::inherited_mode`; receipt `inheritedMode`); optional `permission` may only lower it (a stronger one is refused); a caller in plan sends in plan; a mode the running conversation's agent can't run is never queued, judged by the agent of the chain's current conversation (`mode_agent`, `send_mode_for_agent`) |
| Held agent send (`refreshed_agent_input`) | the conversation's mode when it goes, lowered by the sender's stored `HeldInput.modeCap`, and kept at the sender's `HeldInput.modeLift` while that is stronger than the conversation's own |
| Fresh follow-up, routed send (`routed_settings`) | the chain's current conversation's own mode, lowered by the send's mode |
| Handover successor (`handover_mode`, `handover_mode_for_agent`) | the chat's own mode, never above the turn handing over; a successor on another agent (named, or the app default) inherits it from the chat's agent through `agent::inherited_mode` (below), so Grok or Antigravity get acceptEdits instead of auto, a Claude `default` or `acceptEdits` chat's Codex successor runs plan, and the result's `modeNote` says so |

Unattended turns still obey the standing orders, which sit in Hollow's cards and
`unattended::SAFE`, not in the mode. They still start chats at most in accept
edits (`main_agent::UNATTENDED_START_CAP`, `app_sessions::unattended_start_mode`).

## An inherited mode never gains capability across agents

A chat Hollow starts with a mode it inherited rather than one named for its
agent (`sessions.start` without `permission`, `sessions.handover` to another
agent, the phone's new chat, a teammate through `team_mcp::delegate_grant`)
never does more than the parent's mode could do on the parent's agent (Q-R3,
2026-11-13; `agent::inherited_mode(from, kind, mode)`). Mode names are not
compared across agents: Claude's headless `default` reads only, Codex's
`default` runs the workspace-write sandbox. Each mode has a capability on two
axes compared each on its own (`agent::mode_capability`, `ModeCapability::within`):
how far it reaches (the teammate cap's model, `team_mcp` tests, `Effective`) and
whether it runs commands that can change things (a command in a read-only
sandbox can't, so it doesn't count).

| Mode | Claude | Codex | Grok | Antigravity |
|---|---|---|---|---|
| plan | read only | read only | read only (read-only sandbox) | read only |
| default | read only | folder + commands | read only (read-only sandbox) | read only (headless denies every write and command) |
| acceptEdits | folder, no commands | folder + commands | folder + commands (workspace sandbox) | read only (as default) |
| auto | checked + commands | checked + commands | unchecked + commands (sandbox off) | unavailable |
| bypassPermissions | unchecked + commands | unchecked + commands | unchecked + commands | unchecked + commands |

Reach, weakest first: read only; folder (edits, or commands in a sandbox held to
it); checked (beyond the folder, each action judged by a classifier or
reviewer); unchecked (no sandbox, no check).

The child runs, among its agent's modes it can run whose capability is within
the parent's on both axes and that sit no higher in Hollow's own layer than the
parent's mode (cards and connectors: plan, then default and acceptEdits, then
auto, then bypass), the one that reaches furthest, then runs commands, then the
parent's mode itself, then the strongest (`agent::capped_mode`). Every agent's
plan is read only with no commands, so there is always one.

| Parent | Claude | Codex | Grok | Antigravity |
|---|---|---|---|---|
| Claude plan | plan | plan | plan | plan |
| Claude default | default | plan | default | default |
| Claude acceptEdits | acceptEdits | plan | default | acceptEdits |
| Claude auto | auto | auto | acceptEdits | acceptEdits |
| Codex plan | plan | plan | plan | plan |
| Codex default | acceptEdits | default | acceptEdits | default |
| Codex auto | auto | auto | acceptEdits | acceptEdits |
| Grok acceptEdits | acceptEdits | acceptEdits | acceptEdits | acceptEdits |
| Grok auto | auto | auto | auto | acceptEdits |
| Antigravity acceptEdits | default | plan | default | acceptEdits |
| any bypass | bypass | bypass | bypass | bypass |

Codex can't edit without running commands, so a Claude `acceptEdits` parent's
Codex child reads only (the accepted conservative result, review round 2). The
result's `modeNote` (or the teammate's note) says when the mode changed.

A parent whose agent is unknown reads the most restrictive way: what every agent
that runs a mode of that name can do (`agent::parent_capability`), so `default`
and `acceptEdits` read only with no commands (Antigravity's) and `auto` reads as
Claude's and Codex's. The window reports its new-chat mode to the phone with the
agent it is for (its preferred agent: `phone_access_new_chat_mode`'s `agent`,
kept in the record as `newChatAgent`, and held with the mode as one value,
`routes::NewChatMode`, so a request never pairs one report's mode with another's
agent), so the phone's new chat inherits from that agent; an older report without
one reads the most restrictive way, and with no report at all the chat starts in
its own agent's acceptEdits. A workflow caller reads on the agent its run was
spawned on, a budget fallback's included (`app_broker::set_workflow_run_agent`,
read by `caller_agent`), never its file's; an unrecorded run is unknown. A
teammate reads on `HOLLOW_PRIMARY_AGENT`.

Named modes are checked on both axes too. A named `permission` on a named agent
asks the start card when it can do more than the caller's own mode, even at an
equal rank (`app_sessions::escalation_for`: a Claude `default` chat naming Codex
in `default` asks; a named `auto` for Grok counts as bypass). A named
`permission` on an agent the CFO placed (no card could name it) is kept exactly
when it fits the caller's capability (joined with the named mode's on Claude
when it is stronger by rank: its card was asked then, in Claude's canonical
names) and lowered only when it doesn't, never raised
(`agent::named_mode_within`); the `modeNote` says to name the agent so Hollow
can ask (`start_mode_for_agent`). A send never queues Antigravity's auto.
The phone's send that moves an existing chat onto another agent with no mode
named, and a "When an agent runs out" continuation on another agent
(`agent_out::Continuation::onto`), inherit the same way (2026-11-13).
A send someone lowered (say to plan) becomes the chat's last mode, so later
wakes there run lowered until the person sends again.
