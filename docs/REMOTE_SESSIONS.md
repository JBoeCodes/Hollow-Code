# Remote sessions: agent turns on the Mac mini runner

2026-11-01, Phase 2 of the Mac mini runner plan (vault
`workspaces/boelab-778c8a32c0c4c034/outputs/mac-mini-plan.md`). Code:
`src-tauri/src/remote_session.rs` (tests in `remote_session_tests.rs`), wired
in `chat.rs` (`execute_turn`, `finish_observed_run`) and `agent.rs`
(`build_with_codex_servers`). Phase 1, `hollow-slot --remote`, is in
[WORKSPACES.md](WORKSPACES.md) (Remote runs); since smart routing
(2026-11-03) a plain `hollow-slot <cmd>` chooses between this Mac and the
runner itself (WORKSPACES.md, Smart routing), and `--remote` only forces it.

## What moves and what stays

A build workspace's Claude or Codex turn can run its **agent process** on the
remote runner (the SSH host in `~/.config/hollow/slot-remote`, the one
`hollow-slot --remote` uses; on Alex's Mac `runner-host`, the Mac mini's
`runner` account). Everything else stays on this Mac: the conversation and
its transcript, the run queue, History and receipts, the usage ledger,
question cards, Stop, and **every tool Hollow serves** (`hollow_app`,
`hollow_tasks`, `hollow_vault`, the library's stdio connectors). The vault
never leaves this Mac.

Where it is chosen (one setting, visible in three places; a vault
workspace's own choice is Phase 3's, below):
- Settings → Devices → **Runs on** (This Mac, the default; Remote runner, named Mac mini until remote runners 2):
  `agent_settings.runsOn`, also `change_setting agents.runsOn` (confirm).
- A build workspace's **Changes** tab → Runs on (Default, This Mac, Remote runner):
  the workspace's own choice, stored in `build-workspaces.json` beside its
  worktree (app data, not the brief: an agent that writes the vault must not
  move where runs execute). IPC `build_set_runs_on`; the COO's action is `build.set_runs_on` (confirm, L13).
- A new conversation in a build workspace shows the same control in its empty
  state, so the choice is made before the first message.

The control (`components/RunsOnControl.tsx`, its hint `components/runsOnHint.ts`)
says on Default that it follows Settings → Devices → Runs on, and, when
Overnight and worker chats (below) is the runner, that night and worker
chats follow that setting instead (2026-11-03; M7). On Remote runner it adds that
a self-hosted model's turns stay on this Mac, as Settings' Runs on and
Overnight and worker chats help do (self-hosted models Q7 review).

Build workspaces moved in Phase 2 (their work is in git); a vault
workspace's own chats move too since Phase 3 (below, "Workspace folders").
The COO and chats in Chats always run here, and so do Antigravity and
Grok. So does a chat turn on a self-hosted model (self-hosted models Q7,
2026-11-03), whatever Runs on says: Hollow serves the model through this
Mac's local forwarder, which the runner can't reach, and signs in to its
endpoint here. `remote_session::turn_place` decides it before anything is
sent, and, when Runs on is the runner, the turn's first line says why
(`remoteRun` phase `stayed` with `selfHosted`, whose `leftOnRunner` adds
what of an earlier turn is still there: `pending` when a turn stopped by a
quit wasn't brought back because the runner didn't answer (or wasn't asked,
because it didn't answer minutes ago), which a later message brings back
once the runner answers; `runner` for work only a turn there brings back,
by the morning report's `left_on_runner` rule). The one call to the runner such a turn
makes is Overnight R1's: a turn still running there from before Hollow quit
is stopped and brought back first, as for any message (below, "Anything
else"), and its line follows the first one. Details in
[agents.md, Self-hosted models](reference/agents.md#self-hosted-models).

## One remote turn

1. **Prepare** (`remote_session::prepare`, before the command is built). The
   worktree's exact state (HEAD, or a snapshot commit of HEAD plus every
   uncommitted and untracked change, made in a private object folder; the
   chat's `.hollow-attachments/` is added by name with `add -f`, since its
   own `.gitignore` of `/*` keeps it out of `status` and `add -A`; the tree
   kept for comparing, `snapshot_tree`, leaves it out) is
   pushed into the runner's bare repository `~/hollow-remote/repos/<repo>.git`,
   as `hollow-slot --remote` does (the runner has no GitHub credentials). The
   runner checks it out in `~/hollow-remote/sessions/<worktree>-<hash>` (a git
   worktree of that repository), on the same branch, with a dirty tree's
   changes left uncommitted; gives it a scratch folder, a port no other session
   folder holds (2700-2999), a build folder (a clone of the same worktree's
   `hollow-slot` one when there is one) and `npm ci` when `package-lock.json`
   changed. The runner's tools are sent each turn
   (`~/hollow-remote/bin/hollow-remote-run`, and `hollow-slot` so the agent's
   gates run in the runner's own slots, and `this-mac.md`, the runner's own
   machine note from `this_mac::runner_note`). The supervisor sets
   `HOLLOW_THIS_MAC` to that note, `HOLLOW_BUILD_SLOT_DIR` to the runner's
   slot folder and `HOLLOW_SLOT_REMOTE=off`; this Mac's values of all three
   stay out of the header (`LOCAL_ONLY_ENV`).
2. **Build.** The agent command is built exactly as for a local turn (account
   routing, run tag, the brokers' bound run ids, the run's private tool
   config), for the runner's paths, then `into_leader` turns it into the run's
   **local leader**: `/usr/bin/perl -e LEADER_PL <config> <relay socket>
   <runner socket base> <run> -- ssh … <host>`; the leader adds each
   connection's `-R <base>.<n>.sock:<relay socket>` and the runner command
   (`hollow-remote-run --run`, then `--attach` on a reconnect) itself.
3. **Run.** The leader listens on the relay socket, starts ssh, and writes the
   agent's command line and environment as one JSON line to ssh's stdin, then
   its own stdin (a steerable turn's messages) in frames after it. On the
   runner, `hollow-remote-run --run` reads that line and starts the run's
   **supervisor**, detached from the connection (below, "Surviving a dropped
   connection"), which starts the agent in its own process group; `--run`
   then streams the agent's output back over ssh into the usual stream parser
   and passes the input on.
4. **Tools.** Each tool the agent starts on the runner is
   `hollow-remote-run --bridge <name>`. It connects to the run's own **tool
   socket** there, which the run's supervisor serves: it admits a connection only when the
   connecting process (`LOCAL_PEERPID`) descends from the agent, then forwards
   it through the relay with the run's token. The leader starts **that server
   here**, from the run's private config file, as its own child.
5. **Back** (`remote_session::collect`, in `finish_observed_run`, whichever
   way the turn ended). When the agent exits, the runner records the branch
   head it left and a snapshot of what it didn't commit as result refs (the
   chat's untracked `.hollow-attachments/`, ignored by its own `.gitignore`,
   is never in them: this Mac compares and applies without it, so an
   attachment saved here during the turn is no change, the switch leaves
   those attachments where they are, a result kept on a branch can't commit
   one, and nothing the agent changes or writes in that folder there comes back, which
   its note says; attachments committed before C5 are ordinary tracked
   files; C5 third review), then removes its `<folder>.running` marker; collecting waits for that marker
   (30 s at most, so a Stop's TERM, wait and write finish first). This
   Mac fetches them (`git fetch` over ssh from the bare repository) and, if
   the local worktree is exactly as it was sent, applies them: the files
   first (a two-tree switch from what is here to the result), then the branch,
   with every uncommitted change unstaged. If the worktree changed here while
   the turn ran, nothing here is touched and the result goes on a branch
   `<branch>-mini-<n>`; so does a result that isn't a fast-forward of the
   branch the turn started on (the agent switched or rewrote branches there).
   Once brought back, the runner drops exactly the result refs that were
   fetched (compare and swap). A run still finishing after the 30 s wait is
   reported as such, not as "no changes"; that result, or one never brought
   back (a failed fetch), is kept by the next preparation and fetched here as
   `<branch>-mini-<n>`, which that turn's start line names. Every kept
   result is listed at each preparation and dropped there only once it is
   here (compare and swap on the listed commit), so a fetch that fails is
   retried next turn; a kept commit a local branch already contains is
   dropped there without a new branch or a mention. If the files were
   applied but the branch couldn't be moved (its ref locked here), the
   agent's commits go on a `<branch>-mini-<n>` before the result refs are
   dropped. The runner drops a turn's result refs only once its work is safe
   here (applied, unchanged or on a branch); when no branch can be made here
   (the message carries git's own last error, such as a locked ref),
   it keeps them and the next preparation brings them, and a preparation
   that can't keep an earlier result stops before the turn could write over
   it.
   Attempts of one turn (an account switch) never overlap there: the new
   attempt's supervisor waits for the last one's marker (stopping it when its
   leader is gone) and doesn't start if it is still there after 30 s, and
   each has its own scratch folder. The
   receipt's file capture runs after this, so it shows the runner's changes
   like a local turn's.

The agent is told it runs on the runner and that Hollow's build tools
(`build.build_commit`, `build.publish`, `build.status`) act on this Mac's
copy until the turn ends, so it commits with git there. It is also told
that paths under this Mac's worktree are at the runner's copy (an attached
file's path included), and that an attached file there is a copy to read:
nothing in `.hollow-attachments` comes back, so it writes what it makes
outside that folder.

**Who commits there (2026-11-02).** The runner account (`runner`) has no
git identity, so before this an agent's commit there was git's guess from the
account: `Hollow runner <runner@build-mac.localdomain>` (found on the first
real Claude turn after the 0e3a0efa install). Now the preparation resolves the
identity this Mac's git uses for the checkout (`checkout_identity`:
`git -c user.useConfigOnly=true var GIT_AUTHOR_IDENT` and
`GIT_COMMITTER_IDENT` in the worktree, a build workspace's Phase 3 turn
included, or in the workspace's folder for a workspace chat's turn), so
includes and `includeIf` (by `gitdir`, by a remote's URL)
pick a work identity where one is set, a repository's own config wins, and
Hollow's own `GIT_AUTHOR_*` would win as they do for a commit here. The
leader puts it in the run header's environment as `GIT_AUTHOR_NAME/EMAIL` and
`GIT_COMMITTER_NAME/EMAIL` (the same channel as the account token: ssh's
stdin, never a command line, never the runner's config); a variable the
agent command sets itself stays and the others are this Mac's, as git here
would combine them. The supervisor sets the header's environment for
the agent, so Claude's and Codex's commits and anything they start carry it.
Nothing is set globally for `runner`: each turn carries its own checkout's
identity, and a work checkout keeps its own. `user.useConfigOnly` keeps this
Mac's own guess (`<account>@<host>.localdomain`) from being sent: with nothing
configured here, nothing goes, git's default there stays, and a worktree
turn's start line says so once ("This Mac has no complete git name and email
for this worktree, …", the start event's `noIdentity`); a failure that isn't
a missing or empty name or email (a broken include) also goes to the app
log as `[remote] no git identity from <folder>: <git's last line>`, an email
in it left out. A repository git won't trust (another owner) is read as no
repository, so its own `user.*` is skipped and the global identity goes (a
commit in it here would be refused). The runner's own
transport commits (`hollow-session`, `hollow-folder` result refs) keep their
fixed names; they never reach a branch. The agents' `Co-Authored-By`
trailers are untouched. `hollow-slot --remote` does the same for its command
(WORKSPACES.md, Remote runs). Shown on the real mini (11-02, the probe agent
through the real launch path, `real_runner_commit_carries_this_macs_identity`):
a commit made there as `runner` came back as `example-user
<dev@example.com>` for author and committer; withheld, the same probe
showed `Hollow runner <runner@build-mac.localdomain>`.

The chat shows a quiet line when the turn starts ("Running on the runner
(runner-host) …") and when its work comes back (applied, unchanged, kept on a
branch, or why not): the `remoteRun` event (`remoteRunNotice.ts`), also written to the
conversation's event log (it isn't shown again when the conversation is
reopened).

## Security model

- **The brokers' per-run proof is unchanged.** A broker serves a request only
  when the connecting process (its pid from `LOCAL_PEERPID`) descends from a
  registered run leader and the request's run id is that run's
  (`run_auth::check_peer`), then checks the server's proof. Here every tool
  server is a child of the leader, which Hollow registers as the run leader
  exactly like a local agent CLI (the guard adopts it with the run tag and
  bound run ids). The proofs live only in the run's 0600 config file on this
  Mac; the runner's config names only the bridge.
- **The forwarded socket only reaches the relay**, and the relay serves a
  connection only when its first line carries the run's 32-hex relay token
  (compared in constant time) and names a server in the run's config;
  anything else is closed before any server starts. The relay socket is 0600
  in a 0700 folder (`/tmp/hollow-relay-<uid>`); on the runner it is in
  `~/hollow-remote/run` (0700), named per connection (`<run>.<n>.sock`).
- **On the runner the token stays inside the run's perl processes**
  (`--run`'s supervisor and the attachers, which prove it to the supervisor
  on its control socket; Phase 4). It is never in the
  agent's environment: on macOS `ps -E` shows a process's environment to
  other processes of the same user when the program isn't Apple's own (node,
  `claude`, `codex`), so any other `runner` process (another run, a
  `hollow-slot --remote` gate's branch code) could have read it there. They
  are `/usr/bin/perl`, whose environment `ps -E` doesn't show; the agent gets
  only the path of its tool socket, which admits the agent's own descendants
  and adds the token itself, the same peer check Hollow's brokers make here.
- **Account tokens** never touch a command line or a file. The Claude account
  Hollow's routing picked rides as `CLAUDE_CODE_OAUTH_TOKEN` in the leader's
  environment (deleted at start), then in the header over the SSH channel, then
  in the agent's environment on the runner, as it does locally. A turn routed
  to the Claude Code login (no token: none of the added accounts is usable) is
  refused with a clear message: the runner has no login and must never use
  one. A command that names the login (`HOLLOW_CLAUDE_ACCOUNT=login`) is refused
  even with a token in its environment, and the leader carries the account id
  (not secret) so the usage ledger records the added account: before 2026-11-14
  the leader dropped it and every remote run was ledgered `login` (ENG-33). Codex takes only an API key from its environment (`CODEX_API_KEY`,
  from this Mac's Codex login when it holds one); a ChatGPT sign-in reaches
  Codex only as `auth.json` in its home (its `CODEX_ACCESS_TOKEN` is a Codex
  agent identity, not a ChatGPT token: checked with Codex 0.164 and 0.166),
  so a Codex turn signed in with ChatGPT is refused before anything is sent,
  with a message to run Codex on this Mac.
- **Stop** ends everything the run started there. Stop kills the leader's
  process group here (leader, ssh, relay children, tool servers), and the
  turn's finish (`collect`, which runs however a turn ends) sends the run's
  supervisor TERM; it sends TERM, and KILL after 10 s, to the agent's process
  group and to every process and group it saw the agent start (sampled once
  a second: Claude's Bash tool and Codex run commands in groups of their
  own), and when the agent ends it does the same for what it left behind,
  including any process still carrying the run's tag (`HOLLOW_REMOTE_RUN`,
  found with `ps -E` for programs that show their environment). A command
  double-forked into a session of its own between two samples, running an
  Apple program, can escape, as it can locally. A **dropped connection** ends
  nothing there (Phase 4, below). Verified against the real mini: the agent
  was gone 2.2 s after the stopped turn's finish, no `hollow-remote-run`
  process left (`ps`).
- **The runner has the same authority as a local turn, no more.** It runs
  branch code as `runner` (a standard user, no sudo, no stored GitHub or
  Google Cloud credentials, no agent sign-in; a workspace can hand its turns
  GitHub and Google Cloud one request at a time, below), and the agent there has the same Hollow
  tools a local agent has, with the same cards. What it can't do is reach
  this Mac's files: work comes back only as git objects, applied as a
  fast-forward onto an unchanged worktree or kept on a branch. Two limits to
  know: anything persisted in `runner`'s home (its `~/.zshenv` runs before
  `--run`) sees the next runs' headers, account token included, as code that
  edits this Mac's shell startup would locally; and the person's own ssh
  config applies to the connection (agent and X11 forwarding are switched
  off on every call).
- **Not given to remote runs:** HTTP connectors with headers, for Claude
  (their headers would be on the runner's argv in its inline config) and
  Codex (they would ride in its environment there); stdio connectors work
  (they run here through the relay, with the agent's PATH). The vault
  stays here: only the turn's own workspace folder goes, as a copy (Phase 3).

## Workspace folders (Phase 3)

2026-11-02. Code: `src-tauri/src/remote_folder.rs` (tests in
`remote_folder_tests.rs`; round trips through the stand-in runner in
`remote_session_tests.rs`), wired through `remote_session::prepare_folder`,
`prepare_with` and `collect_folder`.

**What goes.** A turn whose working folder is a vault workspace's own folder
(a workspace chat; never the COO's, Chats', a subfolder's or
the vault root's)
runs in a copy of that folder on the runner, which is then its whole
session: scratch folder, port and build folder as for a worktree. A build
workspace's turn takes its vault folder (brief, plan, outputs) beside the
worktree as the agent's one extra folder there, so the agent edits the brief
and plan with its own tools as it does here. Each conversation gets its own
copy (`~/hollow-remote/folders/<folder>-<hash of folder and conversation>`),
so two chats in one workspace never share one, and a chat's native session
resumes there.

**Where it is chosen.** A vault workspace's own Runs on (Default, This Mac,
Mac mini) is in a new conversation's empty state there, stored in app data
(`workspace-runs-on.json`, by vault and workspace id; not the brief, for the
same reason as a build workspace's); IPC `workspace_runs_on` (plumbing) and
`workspace_set_runs_on` (the COO's action: `workspaces.set_runs_on`, confirm, L13). A vault workspace moves **only on its
own choice**: Settings → Devices → Runs on stays the default for
build workspaces, because a vault folder may hold sources (exports, notes
about people) that should stay on this Mac unless the person sends them.

**How it travels.** The vault isn't a git repository, so the folder goes as
git objects from Hollow's own private repository (app data,
`RemoteFolders/<vault>.git`; the folder is its work tree only through
`GIT_WORK_TREE` and an index of its own per folder, so nothing is written
into the vault; git runs with `core.autocrlf=false`, no attributes file and
`LC_ALL=C`, so bytes travel as they are). Each turn: a snapshot commit of
the folder, leaving out junk (`.DS_Store`, `node_modules/`, `.trash/`),
Hollow's own `sessions/` records (never taken back from the runner),
secret-looking names (below), any repository inside the folder, and files
over 50 MB (the agent there doesn't see them, and their absence there
removes nothing here; one that grew past the cap, became a repository or
was sent before secret-looking names stayed leaves the index too),
pushed to the vault's bare repository on the runner and checked out fresh
there with no `.git` in it. When the agent ends, `--run` commits the copy on
top of that snapshot to the folder's result ref (the same junk left out).

**`.gitignore` files don't decide** (overnight R3, 2026-11-03). The vault
isn't a git repository, so a `.gitignore` inside a workspace folder is the
person's own, for some other tool; until R3 `git add -A` honoured it and
what it named never reached the runner. Now what it names goes like any
other file, in bounds. Git has no switch for that (`add -f` drops
`core.excludesFile` too, so it would take the junk), so
`remote_folder::snapshot`:
1. walks the folder (`left_here`) for what stays: secret-looking names (a
   secret-looking folder whole), files over 50 MB, repositories inside it,
   and folders that ignore themselves, which are junk like `node_modules/`
   (`self_ignoring`: a `.gitignore` of just `*`, as `python3 -m venv` from
   3.13, `uv venv`, pytest, mypy and ruff write, or a `CACHEDIR.TAG`, or a
   venv's `pyvenv.cfg`; a chat's `.hollow-attachments/` writes `/*` instead,
   so it is no cache and travels); each goes into the exclude list as a literal
   anchored path (a name with a line break is skipped there: it would be two
   patterns, `!*` among them) and leaves the index first (`rm --cached -f`
   by a NUL pathspec file: the private repository has no HEAD, so without
   `-f` git refused a file changed since it was added, which is how a file
   that grew past the cap used to stay in the index);
2. runs `add -A` as before;
3. lists what is still untracked with Hollow's exclude list and no
   `.gitignore` (`ls-files -z --others --exclude-from=<list>`): exactly
   what only the folder's `.gitignore` files hid; less what may not travel
   (inside a cache too, checked per path, since the walk's names can differ
   from git's in Unicode form), and in bounds: at most 2,000 such files in
   the snapshot, those an earlier snapshot sent counted first (`ls-files
   --cached --ignored --exclude-per-directory=.gitignore`), and new ones only
   while those and the new ones come to 200 MB or less, in path order so the
   same ones go each turn; the rest stays and is named. The file count is
   the hard bound: a file sent before is sent again as it is now, at any
   size under 50 MB. That list goes in with
   `update-index --add --remove -z --stdin` (linear; a long `add -f`
   pathspec list was quadratic, about 14 s for 30,000 files, and failed the
   whole add when one listed file went meanwhile); it stops at a file it
   can't read, so those are left out first;
4. takes out of the index anything that may not travel after all
   (`drop_what_stays`): a `.gitignore` negation (`!node_modules/`,
   `!/sessions/`, `!render.mov`) outranks `core.excludesFile`, so `add -A`
   can take junk, `sessions/`, a secret or a file over the cap. So what
   stays is Hollow's call alone, whatever a `.gitignore` says.
A file sent that way is then in the folder's index, so later `add -A` runs
change or remove it like any other. Secret-looking (`secret_looking`) is
Hollow's rule `directory_connections::secret_name` (what `read_source`
refuses and repo receipts withhold: `.env` and `.env.*`, `*.pem`, `*.key`,
`id_rsa*`, `id_ed25519*`, names with `secret`, `password`, `credential` or
`service-account`, `token*`, `.netrc`, `.ssh/` and the rest) plus key files
and token names it doesn't list (`id_ecdsa*`, `id_dsa*`, `*.p8`, `*.jks`,
`*.ppk`, `*.kdbx`, `serviceAccount*`, `*-token`, `*_token`, `*-token.*`),
compared as APFS compares names, and applied whether or not a `.gitignore`
names the file; `secret_name` itself is unchanged, since other features use
it. What stayed is reported by kind (`FolderSync.left`) and the turn's first
line names each kind's first 5 and counts the rest (`leftHere` in the
`remoteRun` start event: secret-looking items, files over 50 MB,
repositories of their own, files past the budget; "Left on this Mac, so the
agent there doesn't see them: ..."). Junk and caches aren't named.

The runner's finish and `salvage_folder` make their result the same way
(`folder_commit` in `REMOTE_RUN_PL`, with the same numbers, which a test
pins): `folder_stays` walks the copy for caches and files over 50 MB, both
go into its exclude list and out of the index after the adds; what the
copy's `.gitignore` names is added by `update-index --stdin` within 2,000
new files and 200 MB a turn (`add_gitignored`); what the snapshot held of
what it leaves out (a sent file grown past 50 MB there, a sent folder that
became a cache there) goes back in as it was sent (`ls-tree` of the base
into `update-index --index-info`; round 2 review: left out, it read as a
removal and this Mac removed its copy); and what it left out (over
50 MB, past the budget) is named in the result's message (`left: <kind>
<count>`, `left-path: <kind> <path>` for the first 5), which this Mac reads
(`remote_folder::left_there`) into the back line. The runner doesn't sort
secret-looking names out (that would be a second copy of the list in Perl);
this Mac refuses the dangerous ones (below). When git fails there, the
finish writes no result and leaves the folder's marker, so the next
preparation keeps the copy (`salvage_folder`) instead of taking the turn as
unchanged and resetting it. Secrets sent before R3 stay in the runner's bare
repository (unreachable objects go only when git's own `gc --auto` runs
there, and then only after two weeks) and in copies set aside there;
nothing purges them.

**How it comes back** (`collect_folder`, after the worktree's): the result
is fetched into the private repository and the difference from the snapshot
is applied file by file against what is here now:
- unchanged here: the runner's version is written (or the file goes);
- changed here only: it stays;
- changed the same way on both sides: nothing to do;
- changed on both sides: merged line by line when the file is Markdown or
  plain text (`.md`, `.markdown`, `.txt`) that was there before, all three
  versions are UTF-8 under 1 MB and the edits don't overlap
  (`shared_docs::merge3`, the merge Hollow's own writes use), written only
  if the file is still what was read; otherwise (JSON, code, binaries, a
  file added on both sides) this Mac's version stays and the runner's is
  written beside it as a conflicted copy (`vault::keep_conflict_copy`, the
  same `<name> (conflicted copy <stamp> <id>)` Waiting on you offers; its
  folder is made again if this Mac removed it);
- changed here, removed there: kept here; removed here, changed there: the
  runner's version comes back as a conflicted copy;
- never written: a symlink or submodule entry, a path with `..`, `.git` or
  an absolute path, or anything under a folder that is a symlink here;
- never taken: what the snapshot leaves out (`sessions/`, junk, anything
  inside a repository or a cache that stays here; names compared without
  case, the long s and the Kelvin sign folded as APFS folds them, since
  this Mac's volume doesn't tell `Sessions/` from `sessions/`), so a
  tampered runner can't add it ("not brought" in the back line);
- never written (overnight R3 review): names dangerous to create in a
  vault, anywhere in the path (`remote_folder::never_taken_back`: `.env*`,
  `.npmrc`, `.netrc`, `.pypirc`, `.ssh/`, `.aws/`, `.kube/`, `.docker/`,
  `.gnupg/`, `id_rsa*`, `id_ed25519*`, `id_ecdsa*`, `id_dsa*`, `*.pem`,
  `*.key`, `*.p12`, `*.pfx`, `*.p8`), an agent's own included ("never
  brought back"); the broader secret-looking rule decides only what goes,
  so a deliverable like `token-samples.json` comes back the usual way (it
  can't overwrite a file this Mac didn't send; a clash with one that stayed
  here becomes a conflicted copy);
- left there: a file over 50 MB in the result (its size read with
  `cat-file -s`, never the file) and what the runner's message names;
- each kind of what didn't come back is named in the back line as its first
  5 and a count (`notBrought`), so a result with thousands stays one line;
- a file that can't be written here is named in the notice and the rest
  still come back.
The result is applied against the snapshot that was sent (a result whose
parent isn't that snapshot is refused), never a base the runner names. A
kept result (below) is applied only when its parent is a snapshot this Mac
sent for that folder for an earlier turn: each snapshot is remembered as
`refs/hollow-folder/<hash>/sent/<sha>` in the private repository (the newest
16 per folder, by commit time; `remote_folder::was_sent`), and stays after
its result came back, so a result kept on it again (a drop that failed
there, a later attempt of the same turn kept after it died) still applies,
three ways: what already came back counts as changed the same way, unless
that file changed here since, which leaves a conflicted copy beside it.
The snapshot just sent for the turn being prepared is never a kept result's
base. A kept result on any other base is never applied three ways, since a
made-up base could turn "unchanged here" into a deletion or an overwrite:
what it changed from its base comes here only as conflicted copies beside
the files (`remote_folder::apply_copies`; a file it removed stays, one
already the same here is left alone; nothing here is written over or
removed), the start line names the copies ("made on a copy this Mac can't
confirm it sent", the first 10), and the runner drops it like any other;
copies aren't idempotent, so each one brought is marked in the private
repository (`refs/hollow-folder/<hash>/brought/<sha>`) while the runner
still lists it, and a drop that failed there is only sent again
(overnight P3L-4: it was
left there with only an app-log line, listed again at every preparation,
with no way to drop it). On the first snapshot after this was added
the last one sent before it (`refs/hollow-folder/<hash>/snap`) is counted as
sent, so a result kept on it then still comes back.
Every file written counts as the run's own in History
(`automated_history::note_own_writes`), so a turn that shared its workspace
with other chats is credited with exactly what came back. The chat says how
(`remoteRun`, scope `folder`). The runner drops the result ref only once
applied; one that couldn't be (a failed fetch, a Stop still finishing) is
kept by the next preparation (`<hash>-kept-*`, as for worktrees) and applied
here three ways against its own snapshot, after which the folder is sent
again so the new turn's copy has those changes; the start line says so.

**Limits.** Paths in the prompt name this Mac's folder; the agent is told
where its copy is and to use relative paths. Hollow's own tools
(`hollow_vault`, `hollow_tasks`, `hollow_app`) still act on this Mac's vault
directly during the turn, so a file changed both ways in one turn is
treated as changed on both sides. A copy whose run died before writing
its result (the runner restarting mid-turn: its `.running` marker left with
no supervisor) is kept at the next preparation before the copy is reset
(`salvage_folder` in `--prepare-folder`: its files as one commit on top of
the snapshot it was prepared from, recorded there as `<folder>.sent`, with
the finish's junk left out), and comes back here like any kept result. A
worktree session gets the same (`salvage_worktree` in `--prepare`: the
branch head it left plus its uncommitted changes, kept as
`<hash>-kept-<time>-<pid>-died` and brought back on a `-mini-` branch).
Nothing is kept when the run left the copy exactly as it was sent, or when
the run the marker names wrote its result after all (the finish stamps
`<folder>.done` with its run id right after the result ref, so a run killed
between that and removing its marker isn't kept a second time). Another
attempt of the same turn in the same copy (a retry, an account switch) that
died after one finished there is still kept, beside the earlier attempt's
uncollected result. The marker goes only once that is settled: a kept ref
that can't be written fails the preparation (exit 70) and leaves the copy
for the next one, for an hour from the first failed keep (stamped
`<folder>.keep-failed`; overnight P3L-5: it counted from when the run was
last heard, which can be hours earlier, so the first failure gave up at
once). After that the copy is moved aside to
`~/hollow-remote/aside/<folder>-<time>-<pid>` (not reset; only a rename
that fails, said with its error, falls back to the old reset) and the
folder starts fresh, so one that can never be kept doesn't stop the folder
for good; the turn's start line names where it went ("kept it aside at …
(for up to 14 days)"). A worktree copy's `.git` link is renamed
`.git-was-worktree` and its HEAD written to `.hollow-aside` (the fresh
checkout reuses the session's admin folder, so git in the aside copy would
move the live session; a repository an agent made in a folder copy, a
`.git` folder, is left as it is). The run's own commits are unreachable
once the fresh checkout resets the branch and stay in the bare repository
only until its gc prunes them (two weeks by default): `.hollow-aside`
names the HEAD to recover them before then. A preparation that fails after
moving one still names it on its last line, and a build turn whose folder
then fails (or a folder sent a second time) names the copy too. A worktree copy whose git directory isn't this repository's
is remade, not kept. Limits: a marker left before this was added has no
`.sent`, so that copy is still reset; the agent of a run whose supervisor
alone died (not the whole runner) may still be writing while the copy is
kept; a supervisor pid reused after the runner restarts by another run's
process is taken for alive (as before); and two preparations of one copy at
once would both keep it (chat turns in one worktree run one at a time). A copy
unused for 14 days goes, and sooner when the runner's disk runs low (see
Disk below); its objects stay in the private repository there. Verified against the real Mac mini
(`real_runner_workspace_folder_round_trip`): prepared in 0.8 s, back in
0.8 s, an edit made here meanwhile kept, a two-sided edit kept twice; again
on 2026-11-03 with a `.gitignore` naming `build/` and `.env` (overnight R3:
`build/out.txt` there, `.env` not, a new `build/new.txt` back; 0.8 s and
0.6 s).

## Surviving a dropped connection (Phase 4)

2026-11-02. A run on the runner outlives its connection: this Mac sleeping,
Wi-Fi changing or ssh dying only pauses the stream, and Hollow reconnects and
picks up where it was. Code: `REMOTE_RUN_PL` (`--run`, `--attach`, the
supervisor, `--bridge`) and `LEADER_PL` in `remote_session.rs`.

- **The supervisor.** `--run` reads the header, makes `run/<run>/` (0700) and
  starts the supervisor as a grandchild in a session of its own (stdio
  detached), which waits for an earlier attempt in the folder, writes the
  folder's marker (`<folder>.running`: its pid, then the run id) and starts
  the agent: stdout appended to `run/<run>/out`, stderr (and its own lines)
  to `err`, stdin a pipe it holds. When the agent ends it ends what the agent
  left, writes the result refs, removes the marker and writes `exit` (the
  code, by rename) last.
- **Attaching.** `--run` then acts as the first attacher, and a reconnect
  runs `--attach <run> <stdout bytes> <err offset> <relay socket>
  <supervisor pid> 1`. The leader sends each attacher `<attach secret>
  <nonce>` over the SSH channel (after the header on the first connection):
  the secret, the leader's own and never sent to a relay socket, proves the
  attacher on the supervisor's control socket (`run/<run>/ctl.sock`), and
  only after the attacher has checked that socket's peer is the
  supervisor (`LOCAL_PEERPID` against the pid the supervisor itself gave the
  first attacher), so a socket someone else put at that path never gets it.
  The attacher names this connection's relay socket and gets `ok <input
  bytes> <input ended>`; a newer attacher replaces the last (the supervisor
  TERMs it and the tool connections made through its relay, whose bridges
  reconnect). It then streams `out` and `err` from the bytes the leader had,
  and once `exit` is there and both are drained, says `done`. For a leader
  that asks (the header's `err_lines`, `--attach`'s last `1`; overnight R2,
  2026-11-03) each whole line of `err` goes as a mark of its own, `<nonce>
  e <offset> <line> <nonce>`: where the line starts in `err`, and the
  connection's nonce again at its end, so a line cut short can't pass for a
  whole one (`f` for a last line with no newline); its other marks end with
  the nonce too, and `in` says the format. A leader from before doesn't ask
  and gets the lines bare, with only the `err` marks, as before. The marks
  (`<nonce> in <input bytes> <input ended> <supervisor pid>`, `<nonce> err
  <offset>` after each run of lines, `<nonce> done <code>`, `<nonce> gone`)
  all start with the nonce, so a line in the run's own output shaped like
  one is passed on as text.
- **The leader** frames its stdin (`D <length>` and the bytes, `E` when input
  ends, `H` every 15 s) and keeps it whole; after `@in` it sends exactly what
  the run hasn't had. It counts the stdout bytes it passed on and the `err`
  bytes of each `e` line it unwraps and passes on, and keeps the runner's
  marks (`in`, `err`, `done`, `gone`, and the `e` wrapping) out of the
  stream. When ssh ends
  without `done` after the run started, it says once on stderr that the
  connection dropped, drains ssh's pipes (until 2 s pass with nothing new;
  after `done`, everything), and reconnects with
  backoff (1 s doubling to 30 s) through a new forwarded socket name each
  time (`<run>.<n>.sock`: sshd won't reuse a name an old connection may still
  hold), for 12 hours at most; a run the runner no longer has ends the turn
  with a message. So stdout reaches the parser exactly once, and so does the
  run's stderr: a reconnect asks from the end of the last `e` line the
  leader passed on (before R2 it asked from the last `err` mark, so lines
  between that mark and a drop showed twice). A line cut by the drop is
  dropped and comes whole after the reconnect. Any line with the nonce in
  it that isn't a whole mark (cut short with ssh's own words joined on or
  landed inside it, which ssh can write mid-stream, such as a failed
  forward), an `e` line that doesn't start where the last ended, or an
  `err` mark past the count, means a line went missing: the leader passes
  nothing more from that connection, ends it, says "a line of the run's
  output from <host> came through damaged; asking for it again", and the
  reconnect asks again from the count (ssh gets a second to end by itself
  first, so a line cut by a drop is reported as the drop it is). Five such damaged lines on live
  connections in one run (ssh keeps writing into the stream) and the run's
  stderr comes bare from then on, at-least-once as before, rather than
  reconnecting for ever. Any whole mark says the format, so a damaged `in`
  doesn't hide it; on a reconnect a damaged `in` asks again too. Limits:
  ssh's own words that land on such a line aren't shown; ones that land in a
  mark's first bytes leave that start (`hollow-rem…` with them) as text; and
  a damaged `in` on a run's first connection is only left out: that
  connection takes no input, and if it later drops the turn ends there
  instead of reconnecting (it is the connection's first write, so this is
  theoretical).
- **Tools across a drop.** Each `--bridge` keeps the agent's `initialize` and
  `initialized` and the ids of requests waiting. When its connection ends it
  answers each waiting request with a JSON-RPC error ("unreachable right now;
  try again"; the call may already have run here), refuses new ones at once
  while it reconnects (every 2 s, every 15 s after a minute, for 12 hours as
  the leader does), and replays `initialize` (id `hollow-replay-<n>`, its
  answer swallowed) and `initialized` to the new server, so the agent keeps
  its MCP servers.
- **Ending.** Stop and every other finish reach the run only through
  `collect`, which leaves the turn's stop file (`run/<turn>.stop`; a run not
  yet started then never starts, and one running stops within a second) and
  TERMs the supervisor of the turn's own run (its id starts with the turn's
  id, so another conversation's run in the same folder is never stopped);
  once the folder is free it removes the turn's finished runs' folders there
  (their output). A run whose leader is gone (Hollow quit mid-turn) goes on
  until its conversation's next turn picks it up or brings it back (After
  Hollow quits, below), or until another preparation of its folder, which
  stops it when no attacher is connected or none was heard for 45 s, and
  keeps its result (brought here as `<branch>-mini-<n>`); a new attempt of a
  turn (an account switch) does the same to the last. With no word from Hollow for 12 hours the
  supervisor stops the run itself. The host preparation removes `run/<run>/`
  folders and stop files over a day old whose supervisor is gone. The 12
  hours hold while this Mac sleeps; awake but offline, Hollow's own stall
  watchdog ends a turn with no output sooner, as for a local turn.
- **Verified** against the real Mac mini (`real_runner_reattach_after_a_drop`):
  the leader's ssh killed mid-turn, the agent still running there, the leader
  reconnected, every line arrived once, a tool call made after the drop was
  served here through the new forward, the commit came back, and no
  `hollow-remote-run` process was left.
- **Limits.** The runner's account is one trust domain: a process running as
  it (another run, a gate's branch code, the agent's own commands) can
  append to a run's `out` file and so add events to that run's stream, as it
  could already read later runs' headers through `~/.zshenv` (above). It
  can also, as since Phase 2, put its own socket at a run's forwarded relay
  path (`run/<run>.<n>.sock`) and so receive the next tool connection's relay
  token, then use the real forward to start the run's tool servers here,
  with that run's authority. It can't forge the runner's marks (the nonce)
  or get the attach secret (the peer check). The supervisor is per run: a runner that restarts mid-turn
  loses the run itself (the agent stops there and its result is not
  written); what it had left in its folder is kept at the next preparation
  (Workspace folders, Limits) and comes back then. A resident supervisor (a
  launchd agent on the runner) would let the run itself go on; not built.
  A Hollow that quits mid-turn reattaches at the conversation's next turn
  (below).

## After Hollow quits (overnight R1)

2026-11-03. A quit (or a crash, or an install's restart) kills the turn's
leader with Hollow's runs; the run there goes on. Before this, the table of
remote turns was in memory only, so the next preparation of the folder
stopped the run and its work came back as a `<branch>-mini-<n>`, with the
native session on the runner remembering work the new turn's checkout
didn't have.

- **What is kept, and where.** Each remote turn leaves three small files in
  this Mac's app data, `RemoteTurns/` (0700; each file 0600): its target
  (`<hash of the session key>.json`, written by `begin_turn`), what its
  leader knows (`<turn>.state`: run id, relay token, attach secret,
  supervisor pid and err offset; written by the leader, `HOLLOW_REMOTE_STATE`,
  as it starts, when `in` arrives and after each run of stderr lines), the
  agent, and the stdout bytes Hollow has handled (`<turn>.out`, a
  fixed-width number written in place after each line by
  `remote_session::Progress` in the attempt's drive). Only a line that is a
  whole JSON event counts at once (the agents' streams are one per line);
  any other counts once a line follows it, so a line cut off by the quit is
  never counted and comes whole after the reattach. A record that replaces
  an earlier one (its runner didn't answer) takes that one's files with it,
  and a record is read only when its host, ids and paths are ones Hollow
  writes. They go once the turn's work came
  back (`forget_turn`, after `collect` in `finish_observed_run`); records
  older than 14 days are pruned. The relay token and attach secret are at
  rest on this Mac only: they add no authority (any process of this account
  can already ssh to the runner and read the run's tool config here), and
  nothing new is kept on the runner.
- **The next turn** of that conversation finds a record with no turn of this
  process in the table (`stale_turn`) and decides (`stale_choice`, pure):
  - **Continue** (the composer's Continue and Waiting on you's Resume send
    the same text, `restart_resume::CONTINUE_PROMPT`), a Claude turn, the
    same runner, the folder the turn ran from, an earlier turn that was a
    Claude one too, and a leader that heard from the run: if the run is
    still there and still its folder's (`run_there`, one ssh call: its
    output in `run/`, and the folder's marker names it, or, ended, the
    folder's `.done` does and its result ref is still there), this turn
    **reattaches**. Its leader (`leader` with `target.reattach`)
    reuses the run id and relay token (the supervisor adds that token to
    every tool connection, so tool calls made from then on are served here
    by this turn's servers and proofs), passes the attach secret and starts
    with `--attach <run> <stdout bytes Hollow handled> <err offset> ...` on a
    random forwarded socket number (`HOLLOW_REMOTE_ATTACH`, no header) and a
    local relay socket of its own; a failed first connection reconnects as
    any later one does. The output from that byte on reaches this turn's transcript
    once; the new turn's own first input line (the Continue) is dropped, later
    steers go to the run after the input it already had, and the input's end
    (Hollow closes it at the `result`, as for any steerable turn) lets the
    agent exit. The start line says "Picked up the turn that was still
    running on the runner …" (`reattached`). Its work comes back at this
    turn's finish like any turn's: applied when the worktree is as it was
    sent. A later attempt of the turn (an account switch) starts a run of
    its own.
  - **Anything else** (another message, Codex, Runs on changed, another
    folder): if the run is still there, that earlier turn is **brought back
    first** (`settle_stale` runs its `collect`, which stops it there as Stop
    does, and its folder's), with "The turn that was running on the runner
    when Hollow quit was stopped there before this one." in front of the
    usual line (`earlier`); this turn then prepares from that work. It runs
    before this turn's repository snapshot, so this turn's receipt doesn't
    show that work, and its folder files aren't credited to this run. A
    turn on a self-hosted model (which runs here, Q7) brings it back too;
    when that turn's Runs on is the runner, its lines follow the turn's
    first line, which says why it runs here and what is still on the runner
    afterwards (with Runs on This Mac there is no such line). A self-hosted
    turn whose route is refused (its model switched off or gone) ends before
    this, so the earlier run keeps going there until a later turn settles it.
  - A run that is **gone** (another conversation's preparation of the same
    folder stopped it and kept its result, a run since, or a day passed and
    the host preparation removed its folder) or a leader that never heard
    from it: the record goes and what it left comes back as before (on a
    branch, from whichever preparation kept it). A runner that doesn't
    answer keeps the record for a later turn; so does one found unreachable
    in the last 5 minutes when this turn isn't going there and isn't a
    Continue (it doesn't wait on it).
- **Never automatic.** Nothing reattaches at launch: the person's Continue
  (or another message) decides, as `restart_resume` requires for anything an
  interrupted chat does next.
- **Limits.** Exact for lines Hollow handled: a crash between handling a line
  and counting it shows that line twice; a line with invalid UTF-8 would
  count differently. Tool calls after the reattach are served by this
  turn's servers, so a connector the earlier turn had and this one doesn't
  is refused. What the earlier turn had only in memory (text it was still
  streaming) is in neither turn. The run's own `result` reports its whole
  usage, which this turn's ledger row takes (the interrupted row has only
  what it saw). A turn an older Hollow started (the install that
  brings this in) left no record, so its work comes back as before. Codex
  never reattaches.

## When the runner can't take it

`prepare` fails the turn before anything runs, with the reason, and nothing
falls back to this Mac on its own (the person chose the Mac mini; the
exceptions are unattended work and worker batches, below, and a chat on a
self-hosted model, above, which run here):
- unreachable (ssh exit 255): "The runner (runner-host) didn't answer:
  <ssh's error>. Nothing ran there. <what to check>. To run this turn here,
  set Runs on to This Mac." What to check follows ssh's error
  (`remote_status::likely_causes`, overnight N2): no address (Tailscale not
  connected on this Mac, or on another tailnet), refused (Remote Login off,
  or still starting), key refused, host key changed, the connection closed
  while logging in (its SSH server refuses for a while after failed logins),
  and otherwise off, asleep, restarting or the network, with "after a
  restart it waits at the FileVault unlock screen until someone signs in on
  it". `hollow-slot --remote` matches the same errors when it falls back
  (`r_why`, shorter words);
- its `hollow-run-preflight` refusing (disk under 20 GB, memory under 30%):
  "runner-host isn't taking work now (<its line>)";
- no runner set up: the turn says where to name one.
- a Codex turn whose sign-in is ChatGPT (no API key): refused with why, before
  anything is sent.

## Unattended work and worker batches (overnight N6)

Work nobody is watching can go to the runner, so this Mac can sleep
through the night, and it never fails because the runner can't take it.
Settings → Devices → **Overnight and worker chats**
(`agent_settings.unattendedRunsOn`, `change_setting agents.unattendedRunsOn`,
confirm): `runner` ("Runner when it answers"); absent and `same` ("Follow
Runs on", the default since M7, 2026-11-15) give those turns the person's
Runs on, strict as above. Before M7 absent was the runner; see the freeze
below.

**Naming a runner turns nothing on (M7, PER-21, 2026-11-15).** Absent
Overnight and worker chats follows Runs on, and absent Builds and tests is
This Mac (`HOLLOW_SLOT_ROUTE=this-mac`; Auto is a saved `auto`). Runs on and
App builds were already off when absent. A one-time freeze at launch
(`agent_settings::freeze_runner_defaults_at_launch`, before the run queue
starts) keeps a setup from before M7 as it was: when `runnerDefaultsFrozen`
is unset and a runner is set up, each of the two that is absent is written as
its old default (`runner`, `auto`) with the marker, in one atomic write, and
saved values (an old `mac-mini` included) are left as read. With no runner
it writes the marker only, so a runner named later starts with nothing on.
Every later save keeps the marker (`agent_settings::carry_over`). Set up's
last step, **What runs there?** (`RemoteRunnerRow.tsx`, after Use this
runner or the wizard's last step), lists Overnight and worker chats, Build
and test commands (Auto), Chats in build workspaces (Runs on) and App
builds, nothing ticked; Save writes only the ticked ones through the agent
settings save (`runnerSetup.ts` `whatRunsTherePatch`), Not now writes
nothing. The COO sets the same four with `change_setting`.

Which turns (`chat::execute_turn`, `remote_session::turn_place` and
`unattended_runner`):
- an unattended turn (`ChatSendRequest::unattended`: a night-queue item, a
  chat an unattended run starts; an unattended runbook never runs in a build
  workspace) or a COO worker
  batch's chat (`ChatSendRequest::worker`, set by `app_sessions::queue_turn`
  for `project_workers`);
- when it starts its conversation's native session
  (`remote_session::starts_native_session`: no id asked to resume, and the
  stint `session_store::start_turn` opened holds no native id yet), so a turn
  that ends up here never leaves a session there to resume, or the reverse;
- in a **build workspace with no Runs on of its own** (a night code item's
  new workspace always qualifies). A workspace's own This Mac or Mac mini
  wins; a vault workspace (C-3), the COO and chats in Chats never move by
  it; only Claude and Codex go, and never a self-hosted model's turn (Q7,
  above: it runs here with its own first line, not this section's); nothing
  moves while no runner is set up.
  The rule itself is `unattended_runner_for` (pure, tested).

Such a turn is **preferred** there, not chosen: this is the one exception to
B-9 above, and it holds even when Settings → Runs on is Mac mini (choose
Follow Runs on to make these turns strict too). It runs here, and its first
line says why (`remoteRun` phase `stayed`, "Running on this Mac: the runner
(runner-host) couldn't take this turn. <why> Overnight and worker chats go
there when it can (Settings → Models)."), when, before anything is
sent (`remote_session::preferred_blocker`):
- a remote turn of this Mac's still has the same worktree there (`TURNS`).
  Chat turns in one build worktree already run one at a time (its lease), so
  worker chats in one workspace go there one after another;
- the runner's last status, at most 5 minutes old, was unreachable or
  refusing (`remote_status::recently_unavailable_state`). Besides a status check,
  any ssh call to the runner that fails that way records it
  (`remote_status::note_failure`, from `ssh_call`: a preparation, a collect),
  so after one turn finds the runner down, turns in the next 5 minutes run
  here at once instead of each waiting out ssh's 10 s connect timeout (the
  first after that tries it again), and Settings shows it with what the last
  check found about the machine and its gates. "5 minutes" holds by both
  clocks (lane S9, 2026-11-03): macOS's `Instant` stops while the Mac
  sleeps, so the status keeps the wall-clock time beside it, a failure is
  recent only while both ages are inside the window, and "checked N minutes
  ago" is the wall clock's. A wall clock set back before the failure makes
  it not recent (the next turn asks the runner itself). Any ssh call that
  gets through clears an unreachable status, a check's or a noted one
  (`note_answered`); a refusing one clears only when a host preparation
  passes the preflight (`note_prepared`, from `host_prep`), never with an
  unrelated call. An ssh killed by a signal (no exit code) says nothing
  either way. Both clear only a status found by the time the call began,
  and a check (up to 30 s) keeps its answer as the last status only when
  nothing newer came while it ran: no status kept or cleared, no call that
  got through (for an unreachable answer), no passing preparation (for a
  refusing one) (`remote_status::Slot`, lane S10, 2026-11-03); Check now
  still shows its own answer. Check now never takes a noted failure for its own answer;
- Codex has no API key; or Claude would use the Claude Code login (a claude.ai
  connector on the turn, or the routing's next account,
  `claude_accounts::next_run_has_token`, with long-work routing for an
  unattended turn). That predictor means "an added account the routing would
  pick", not one with room: routing falls back to a full added account, and
  such a turn goes there and waits for its quota as it would here;

or when `prepare` fails for any reason (unreachable, preflight, a folder
still finishing an earlier run), with its message less the "set Runs on to
This Mac" advice. The fingerprint is restamped with this Mac's folder then,
so the native session resumes here. Anything after preparation (an account
signed out between the check and the start) fails as it would for a chosen
turn.

Later turns in that conversation follow Runs on: the person's own messages,
and a COO's `send_to_session` follow-up to a worker chat (it isn't a
worker batch). When that is another machine than the first turn's, the
fingerprint differs and the follow-up starts a fresh native session,
re-grounded with the conversation (no cache), as switching agents does. A
night item's work whose collect failed stays on the runner as a kept result
until the next turn of that worktree prepared there brings it back (on a
`-mini-` branch; a turn that runs here doesn't). The morning report names
it (overnight 11-02): each night item's conversation's `remoteRun` start
and back notices (`session_store::remote_run_trail`, newest 40, so a
self-hosted chat's own `stayed` notes, one a turn, never push the last
start out; self-hosted models Q7 follow-up) are read by
`night_queue::left_on_runner`, and after the last turn that started there
a failed return whose work is still on the runner, or no return at all
(Hollow quit mid-turn), adds "Its work is still on the runner (<host>):
the next turn there from this conversation brings it back (set the
workspace's Runs on to Remote runner first if it runs here)." to the
item's note (Automations → Last night, whose line now prefers the morning
item's note, and the written report). When the conversation's newest turn
asked for a self-hosted model (`session_store::last_requested_model`, else
the item's own model), the parenthesis adds "; a turn on a self-hosted
model runs on this Mac, so use a model that is not self-hosted for it"
(self-hosted models Q7 review; Claude or Codex since Q9). A worktree's failed return records where
its work is (`on`: `here` when it came back but couldn't be applied, so it
is on a branch here, from `collect_placed`; `runner` otherwise), and only
`runner` counts. A return that applied, changed nothing or went on a
branch here says nothing, and a running item isn't checked. Nor, in the
live report (Last night and `night_queue_morning_report`, lane S10), is an
item whose conversation has a chat turn claimed or running now (a
follow-up there may still bring it back); the report written each morning
names it regardless. Child chats the item's turn started aren't read: their
work left on the runner isn't named. Notices the
store's event retention dropped (by age, or a session outside the 200 most
recently active) aren't read; runbook night items have no conversation,
so their step turns aren't checked.

"Nothing leaves the Mac overnight" (the night queue's rule) still holds: the
agent process runs on the person's own runner, while its tools, connectors
and limits stay here.

## Each routed turn records where and why (RT1, overnight 11-17)

A turn that goes to the runner, or was meant for it and ran here, keeps
the decision in its record: the `remoteRun` `start` and `stayed` notices
carry `route` (`remote_session::Route`): `target` (`runner` or `here`),
`host`, `cause`, `reason` (plain words) and `fallback` (true when it was
meant for the runner and ran here). Causes there: `chosen` (Runs on),
`preferred` (the unattended default), `reattached` (Continue after a
quit). Causes here, all fallbacks: `unreachable` ("the runner did not
answer", now or at a check in the last 5 minutes), `refusing` (its
preflight), `busy` (a turn of this worktree still on it), `signIn` (the
agent's sign-in stays here) and `prepare` (anything else). What
`preferred_blocker` keeps here carries its cause (`Blocked`; a recent
status's comes from its state, `remote_status::recently_unavailable_state`,
so a stalled check is `unreachable`); a failed preparation's is read from
its error (`prepare_cause`: didn't answer, didn't finish the check, not
taking work, a checkout send that lost its connection).
`each_fallback_sentence_maps_to_its_cause` builds each sentence with its
real producer (`unreachable_message`, `refusing_message`,
`checkout_send_error`, `stalled_message`, the status's "When checked"
sentence, the blocker's consts), so a reworded producer fails it. The chat
line says why once (`remoteRunNotice.ts`): "Running on this Mac because
the runner (runner-host) did not answer: ssh: ...", the reason's own opening
clause dropped; a blocker's sentence follows "instead of on the runner";
a start line ends "It went there because Runs on is set to the runner."
Records from before RT1 have no
`route` and keep their old line. A chosen runner that can't take a turn
still fails it (nothing ran, no route); a self-hosted turn's `stayed` line
already says why and gets no route. `hollow-slot` keeps its own decisions
in `route-log` (where, why, then a `here` line with the fallback's reason).

## GitHub and Google Cloud on the runner

Remote runners, 2026-11-03 (code `runner_access.rs`, `commands/runner_api.rs`;
the Perl in `remote_session.rs`; design and decisions in the vault workspace's
`outputs/remote-runners-2026-11-03/`). A runner turn can push, open and merge
pull requests and reach Google Cloud, when its workspace turns them on.

- **Where it is chosen.** Beside Runs on (Changes, a new conversation's empty
  state), once a runner is set up: **On the runner: GitHub ☐ Google Cloud ☐**,
  both off. Google Cloud then has an access level: *My account, no production
  changes* (the default), *My account (full)*, or *A service account*
  (impersonated; its own IAM is the limit). Stored in app data
  (`runner-access.json`), never the brief, so a vault write can't grant a turn credentials. Since L13
  (2026-11-10) the COO changes it only through confirm actions with a card
  (`build.set_runner_access` and the three `build.runner_github_*` setters).
- **Where the GitHub credential comes from** (Settings → Devices → **Runner
  GitHub access**): a **runner token** (the default), a fine-grained token the
  person makes for the runner (only the repositories it should push to;
  Repository permissions Contents and Pull requests, Read and write; nothing
  else), saved in this Mac's Keychain (`runner.github.token` in Hollow's
  secret bundle); or **this Mac's gh login** (`gh auth token`, broader: every
  repository that login reaches). Never stored on the runner.
- **How it gets there: one request at a time.** The secrets go only into the
  run's leader's environment (`HOLLOW_CRED_GITHUB`, or the argv that mints a
  Google Cloud token here, `HOLLOW_CRED_GCLOUD_MINT`; deleted at start like
  the relay token; `/usr/bin/perl`, whose environment `ps -E` doesn't show),
  read when the leader is built (each attempt, a reattach too), never into
  `RemoteTarget` or its saved record. The header carries only flags
  (`HOLLOW_RUNNER_GITHUB=on`, `HOLLOW_RUNNER_CLOUD=<access>`, `GH_REPO`,
  `HOLLOW_RUNNER_ORIGIN`, `GIT_TERMINAL_PROMPT=0`; none named like KEY,
  SECRET or TOKEN, which Codex drops from its commands; and this Mac's own
  values of these, `GH_TOKEN`, `GITHUB_TOKEN`, the Google variables and
  `GIT_CONFIG_GLOBAL` never ride along). With GitHub on, the run's supervisor
  writes the run's own git configuration in its scratch folder and points
  `GIT_CONFIG_GLOBAL` at it (`run_git_config`): it includes the runner
  account's own global files, then, for `https://github.com` only, an empty
  helper (dropping any other, such as Apple's Keychain one) and
  `hollow-remote-run --credential`; `url.https://github.com/.insteadOf` for
  `git@github.com:` and `ssh://git@github.com/`; and `origin` from this Mac's
  checkout's GitHub remote under an `includeIf "gitdir:"` of the run's own
  repository, so no other repository the agent touches gets it (the
  runner's repository is Hollow's transport and has no remote). A folder turn
  gets the helper but no `origin`. Ordinary lookups (a commit's identity
  included) still see the runner account's global values, but
  `git config --global --get` reads only the run's file, and a
  `git config --global` write lands there and goes with the run.
  On the runner, git's helper and the `gh` and `gcloud` wrappers Hollow puts
  first on the agent's PATH (`~/hollow-remote/bin`, sent each turn) connect to
  the run's tool socket, which admits only the agent's descendants, and ask
  for `hollow-credential-github` or `-gcloud`; the supervisor splices that to
  the relay with the run's token, and the leader answers `ok <value>` or
  `no <why>`, starting no server. `gh` gets `GH_TOKEN` for that one process;
  `gcloud` gets a 0600 token file in the run's scratch
  (`CLOUDSDK_AUTH_ACCESS_TOKEN_FILE`, created exclusively, never through a
  link) for that one call, deleted when it ends. While that one call runs,
  the token is in gh's environment or that file, which another process of the
  runner account could read (`ps -E`, same user), as an account token in the
  agent's environment already is; outside those calls it isn't on the runner.
  Each Google Cloud request runs `gcloud auth print-access-token` here (with
  `--impersonate-service-account=` for a service account), which returns
  gcloud's cached hour-long token, so long turns stay signed in. With this Mac
  asleep (a dropped connection) the request fails with that reason and the
  run goes on; nothing is cached there.
- **What the wrappers refuse.** `gh auth token|login|logout|refresh|setup-git|switch`
  and `--show-token`, and `gcloud auth print-access-token|print-identity-token|login|activate-service-account|application-default`
  and `config config-helper`: they would print or store the credential.
  Unless the access is *My account (full)*: `gcloud builds approve|reject`,
  `run services update|update-traffic|replace|delete`, `run deploy` without
  `--no-traffic`, and `deploy releases promote` / `rollouts approve|reject`;
  the level is the one the leader returns with each token, so the run's own
  variable can't lift it. These are guard rails (an agent could call the
  real binary with a token), so the hard limit is the account: use a service
  account for one. A wrapper finds the real program on PATH, never itself
  (its own folder comes from its path, not HOME), and stops after four
  nested calls.
- **What the turn is told and shows.** The agent's runner note says what is
  on (and not to print or store the credential); the start line names it
  ("This turn reaches GitHub (owner/name) with your runner token; Google Cloud
  as you, no production changes"), or why not (no runner token saved, gh not
  signed in here, gcloud not installed here).
- **Tested** (`remote_session_tests.rs`, the stand-in runner):
  `a_runner_turn_gets_github_and_cloud_one_call_at_a_time_and_keeps_none`
  (git's helper, gh and gcloud get it, past a credential helper in the
  runner account's own global config, whose other settings still apply; the
  refusals, also with the agent setting `HOLLOW_RUNNER_CLOUD=full`; `origin`
  only in the run's checkout; no token in any file on the runner, the
  per-call file gone, nothing on argv, in the header or in what the leader
  printed; a process outside the agent gets nothing from the tool socket),
  `a_runner_turn_without_access_reaches_nothing`, and
  `into_leader_puts_the_credential_only_in_the_leaders_environment` (a token
  in this Mac's own environment never rides along either: `GH_TOKEN`,
  `GITHUB_TOKEN` and the Google variables are local-only).
- **Verified against the real Mac mini** (2026-11-03,
  `real_runner_cloud_read_through_a_forwarded_token`, `--ignored`, with
  `HOLLOW_REMOTE_IT_HOST` and `HOLLOW_REMOTE_IT_GCP_PROJECT`): `gcloud run services list --project
  example-project` listed three services there through a token this Mac's
  gcloud minted, handed over one request at a time; `gcloud builds approve`
  was refused; nothing was left in the run's scratch, and no file in its
  session, scratch or run folders held the token. The mini has the Google
  Cloud CLI (as `builder`, readable by `runner`) and no gcloud or gh login
  for `runner`. A real `git push` / `gh pr create` from there waits for a
  runner token.

## Runner status

Settings → Devices → Remote runner shows the runner's state (overnight N1,
2026-11-02, under Runs on until remote runners 2 moved it;
`src/settings/RunnerStatusBlock.tsx`,
`src-tauri/src/remote_status.rs`): Answering, Not taking work (its
preflight said no), Not answering (with what to check, as above) or Check
failed; when it was checked and how long that took; the machine's name and
its `hollow-run-preflight` line as facts (free disk, memory free, Rust,
Node, browser); remote sessions running there now; and its newest gate run
(passed or failed and at which step, how long, for which worktree and
commit, or running now). One ssh call runs `RUNNER_STATUS_PL` there, which
changes nothing (the preflight gets 15 s, then its process group is
killed and the status says it stalled); the answer stays in memory, Settings
checks again when it is over five minutes old, and **Check now** asks at
once (30 s at most; one asked while another runs gets that answer). The runner
runs branch code, so its answer is read leniently: a field of the wrong
type is dropped alone, every text is cut to 300 characters.
Agents read the same with `build.runner_status` (`fresh` to check again).
Verified against the real Mac mini (`real_runner_status`, `--ignored`):
ready in 0.24 s with its preflight line and last gate.

## One owner per runner

Remote runners 2 (2026-11-03): a runner belongs to one person, made
explicit in four ways (`src-tauri/src/runner_owner.rs`, tests in
`runner_owner_tests.rs`).

- **No runner until you name one.** Hollow ships no host or account name;
  `scripts/check-no-host-names.test.mjs` scans what ships (src, src-tauri/src
  outside `#[cfg(test)]` modules, the app's config files) for the names of
  the machines it is built on. Settings → Devices → **Remote
  runner** shows **Off** with **Set up** until `~/.config/hollow/slot-remote`
  names a host. Set up takes an SSH host (a `Host` in `~/.ssh/config` for the
  runner's own account), **Check** asks it how it is (the read-only status
  check, `runner_check_host`) and **Use this runner** writes the file
  (`runner_set`); **Turn off** writes `off` and keeps the old host as a
  comment. `HOLLOW_SLOT_REMOTE` in Hollow's environment still wins, and then
  Settings refuses to write. Runs on's choice reads **Remote runner**, and
  the turn notices say "the runner (<host>)". Since M6 (2026-11-15) the
  stored value is `runner`; `mac-mini`, the value before it, is still read
  and accepted everywhere as the same choice (Settings fields, the
  per-workspace and build workspace Runs on files, actions, the phone API,
  `HOLLOW_SLOT_ROUTE`). Reading never rewrites a file, and a save writes
  `runner` only into a field the person changes: an agent settings save
  keeps an old `mac-mini` it leaves alone, and the phone's failed-send undo
  puts one back as it was. Two places keep emitting `mac-mini`: the `HOLLOW_SLOT_ROUTE`
  Hollow sets for a turn, so an older installed `hollow-slot` (this Mac's
  until the next install, the runner's) still routes, and the phone API's
  `runsOn`, which the shipped iPhone app reads. Someone else's Hollow, with
  no file, shows Off and never reaches your machine.
- **The runner records its owner.** **Record me as owner** (under the
  runner's state) writes
  `~/hollow-remote/owner.json` there, 0600, replaced whole:
  `{"v":1,"installId","key","at","by"}`. `installId` is this Hollow's own
  id (32 hex characters, app data `install-id`, made on first use; it names
  this Hollow and grants nothing); `key` is the SHA256 fingerprint of the key
  this Mac offers that host and its runner account lets in (of every
  `IdentityFile` in `ssh -G <host>`, by its `.pub`, the first one that
  account's authorized_keys holds; `ssh-keygen -l -E sha256`). Replacing
  another Hollow's record asks first. It is the person's click or the
  COO's `build.runner_record_owner` (external, with a card, L13), and it changes nothing else there (no sshd,
  firewall or account change).
- **The status check judges it.** `RUNNER_STATUS_PL` also reads that file
  and fingerprints every key in `~/.ssh/authorized_keys` and
  `authorized_keys2` there (regular files only, `/usr/bin/ssh-keygen -l`
  first; each key once, 50 listed, with counts of keys, key lines and
  fingerprints read). `runner_owner::judge` makes the status's `owner`:
  `you` (this install id, or this Mac's own key, which covers a candidate
  build with its own app data), `other` (another Hollow) or `none`, when
  it was recorded,
  how many keys open the runner account and how many are neither the
  owner's nor this Mac's, and warnings, worst first: another key can log in
  (whoever holds it can run as that account and read what runs there, your
  turns' files and credentials while they run); another Hollow recorded
  itself; the record names a key this Mac no longer offers; lines that
  aren't readable keys; no key readable at all (sshd may keep them
  elsewhere), said rather than counted as zero. Settings shows them under the state with **Record
  me as owner** when the record is missing, another Hollow's, or names an
  old key; agents read the same in `build.runner_status`.
- **`hollow-slot` warns too.** The host preparation's capability line ends
  `keys=<n> others=<n> owner=yes|none` (smart routing's `caps` line, counted
  against the owner key in `owner.json`), and every remote call says
  `hollow-slot: warning: <host> lets <n> SSH key(s) in besides its owner's`
  when another key can log in, or `... lets <n> SSH keys in and has no owner
  recorded` when several can and none is recorded.

Don't share the runner's tailnet node or its runner account's key: anyone
who can reach it and log in runs code as that account, beside your turns.
One person's runner is checked against that person's Hollow only; Sam's
Hollow, with no runner named, shows Off and its own Set up.
**Builds and tests (smart routing 3, 2026-11-03).** Under Runs on, Settings →
Devices → Builds and tests (This Mac, the default since M7 / Auto / Remote runner)
is where `hollow-slot` sends each turn's build and test commands
(docs/WORKSPACES.md, Smart routing); every turn gets it as
`HOLLOW_SLOT_ROUTE` (`this-mac` for the default, so an inherited value never stands in),
never sent to the runner. Its block lists `hollow-slot`'s latest six
decisions from `route-log` and the runner's last tool report from
`runner-caps`, which `runner_status` and `build.runner_status` carry as
`routing`, read fresh from this Mac's slot folder and cut to one line of 300
characters like the rest of the status.

## Set up a runner from scratch

Remote runners 3 (2026-11-03): Settings → Devices → **Remote
runner** → **Set up** (or **Set up again** once a runner is set) holds, below
the quick form that names an existing runner, a runner set up from scratch
(`src/settings/RunnerSetupWizard.tsx`, `src-tauri/src/runner_setup.rs`, tests
in `runner_setup_tests.rs`). The UI calls it Set up, never a wizard (Settings
search must find no "Setup wizard", `settingsSections.test.ts`). Two paths,
each a list of steps; every step shows its exact commands (Copy each), and
what it shows is what runs: a shell line run with `/bin/sh -c` on this Mac
(the ssh calls to the admin login or the runner included, with
`-a -x -o BatchMode=yes -o ConnectTimeout=10 -o ControlPath=none`, a
multi-line script fed to `/bin/sh -s` through a quoted here-document), a
gcloud argv shown shell-quoted (a test types each into a shell and compares
the argv), or one of Hollow's own actions, named as such.

- **Check** runs every step's check commands in order, each bounded (30 s)
  and ended with its process group, and reports per step: Done, Not yet
  (what is missing), Couldn't tell (why) or, after an Apply, Failed. Checks
  are read-only by construction: they live in their own list per step, and
  `check_commands_never_write` holds every one to a deny-list of writes
  (`sudo` other than `sudo -n true`, any `>` but to /dev/null, `tee`,
  `mkdir`, `rm`, `mv`, `cp`, `chmod`, `chown`, `install`, `-create`,
  `-addUser`, `sysadminctl`, `pmset -a`, `launchctl`, `brew`, `kickstart`,
  `ssh-keygen -t`, `git init`, gcloud's `create`, `enable`, `add`, `start`
  and more). A step that depends on an earlier one is skipped, saying why,
  when that one failed: no admin step without the admin login, no runner
  step without the runner login, no gcloud step without a signed-in gcloud,
  and on Google Cloud no ssh to a VM that isn't running (its ProxyCommand
  would start it). The key step's `ssh <alias> true` runs only through a
  Host block (a bare name would get this Mac's other keys offered to it).
  Side effects the deny-list can't see are designed out: the toolchain
  check reads gcloud's version from its SDK's `VERSION` file instead of
  running gcloud (which rewrites its GCE-probe cache every 10 minutes; seen
  live on the first Check), sets `RUSTUP_AUTO_INSTALL=0` and
  `GH_NO_UPDATE_NOTIFIER=1`.
- The UI sends Check, Apply and Create exactly the filled fields the shown
  commands were built from, and keeps those buttons off while a changed
  field's plan is on its way. **Notes** under the fields stop nothing: one
  says when the admin login reaches another machine than the runner's
  address (a new Mac set up while the fields hold the current runner's).
- **Apply** (per step) is the only thing that changes anything. It runs the
  step's apply commands in order, stops at the first that fails (naming it
  and its last line), then runs the step's check. Steps that change
  accounts, keys, sleep or sshd show a confirm dialog naming what changes
  first. Admin commands use `sudo -n`; when sudo there wants a password the
  step fails saying so and shows **by hand** commands for Terminal: copy the
  same script to the admin's home, then `ssh -t <admin> 'sudo /bin/sh
  ~/hollow-setup-<step>.sh; rm -f ...'`, which asks for the password there.

**A Mac you own, always on.** Fields: the admin login (`user@host` or an
alias, used for setup only), its address (from the admin login when empty),
the runner alias (default `hollow-runner`), the runner account (default
`hollowworker`) and the key file (default `~/.ssh/id_ed25519_<alias>`).
With a runner already set up, the alias comes from `slot-remote`, the
account and address from `ssh -G <alias>`, and the key file from the
alias's own Host block in `~/.ssh/config` (its first `IdentityFile` under
`~/.ssh/` with a `.pub` here; `ssh -G` lists a `Host *` block's keys
first), else `ssh -G`'s first such key (a block with no key of its own, or
one in an `Include`d file).

| Step | Check (read-only) | Apply |
|---|---|---|
| 1 Admin login | `ssh <admin> 'id -un; sw_vers -productVersion; uname -m; sudo -n true ...'` | none; by hand `ssh-copy-id <admin>` (asks the password once) |
| 2 Hidden standard user | `dscl . -read /Users/<acct>`, `dseditgroup -o checkmember` for `admin` and `com.apple.access_ssh` (when that group exists), its home | creates a user: `sysadminctl -addUser` (full name Hollow runner) with a throwaway random password made there; then, on every Apply and only for an account whose full name is Hollow runner, a fresh random one fed to `dscl .` on its stdin (`passwd`, printf is built in), so the password it keeps was in no argv, `ps` line or sudo log, even after an Apply that stopped right after sysadminctl (a failure prints a fixed line, never dscl's output); never kept or shown; an account someone else made keeps its password; `createhomedir -c -u`, `dscl . -create ... IsHidden 1`, joins `com.apple.access_ssh` when it exists; refuses an existing admin account |
| 3 A key of its own | the key file and its fingerprint, `ssh -G <alias>` (HostName, User, IdentityFile), `ssh <alias> true` | `ssh-keygen -t ed25519` when missing; appends `Host <alias>` (HostName, User, IdentityFile, IdentitiesOnly yes, ForwardAgent no) to `~/.ssh/config` after copying it to `config.bak-hollow-<date>`; refuses when a `Host <alias>` already says otherwise, skips when it says the same; installs the public key in the account's `authorized_keys` (700/600, never twice) as that account itself (`sudo -n -u <acct>`), never as root, so a link planted in that home can't reach a root-owned file; only a well-formed `ssh-ed25519 <base64>` line leaves this Mac (an `awk` match, else nothing is sent and the script there refuses) |
| 4 Toolchains | as the runner: its PATH for ssh commands, git, node, cargo, rustc (and rustup's 1.96.0), uv, python3, gh, gcloud, `pg_ctl` of Homebrew's `postgresql@*`, Playwright Chromium | admin: `brew install node uv gh postgresql@17`, `brew install --cask gcloud-cli`, the SDK opened `go+rX`; runner: a `~/.zshenv` line putting `~/.cargo/bin`, `~/.local/bin` and Homebrew on its PATH (a non-interactive ssh command reads only that file), rustup 1.96.0 `--profile minimal` with rustfmt and clippy, `npx -y playwright@1.61.1 install chromium` |
| 5 Preflight | finds `hollow-run-preflight` (PATH, then `~/.local/bin`) and runs it | writes Hollow's default (`PREFLIGHT_SH`: macOS and Linux; `no: ...` and exit 1 under 20 GB free disk or 30% free memory, else `ok host= free= mem= rust= node=`) to `~/.local/bin`, 755; a different one there is kept as `.bak-hollow-<date>` |
| 6 No sleep, SSH keepalive | `pmset -g`, `ClientAlive*` lines in `/etc/ssh/sshd_config` and `sshd_config.d/*` (plain reads) | changes sleep settings and sshd: `pmset -a sleep 0 disksleep 0 autorestart 1`, writes `/etc/ssh/sshd_config.d/100-hollow-keepalive.conf` (`ClientAliveInterval 30`, `ClientAliveCountMax 4`), `sshd -t` must pass, else the file is removed and the step fails; nothing restarts sshd (launchd starts `sshd` per connection on macOS, `inetdCompatibility` in `ssh.plist`, so new connections read it and no session is cut) |
| 7 Use it | `slot-remote` names the alias; the runner status check (`remote_status::check`) is ready and its owner is this Hollow | in order: the status check of the alias (it runs the preflight), `record_owner`, Hollow's own `hollow-slot` put in place (`build_slots::ensure_tool`, shown), a test call `hollow-slot --on <alias> sh -c 'uname -sm; node --version; cargo --version'` in a throwaway git repository in `$TMPDIR` (a fallback to this Mac fails the step), and only then `runner_owner::set_runner`, so a runner that fails any of these never replaces the one in use; shows, never runs, `hollow-slot --remote npm run gate` |

**A Google Cloud machine.** Built and tested against stubs only: a stand-in
gcloud records each argv and answers canned output; no real project, VM,
service account, IAM binding, firewall rule or budget was ever created, and
Settings says so beside the estimate. Fields: project id, zone (region from
it), machine type (`e2-standard-4`, `e2-standard-8`, `n2-standard-8`,
`t2a-standard-8` on Arm, `e2-standard-16`), boot disk GB (200), stop after N
idle minutes (30), expected hours a month (160), billing account and a
monthly budget in USD, runner alias (`hollow-gcp`), instance and service
account names (`hollow-runner`), key file.

- **Google Cloud sign-in** checks `gcloud config list account` and
  `gcloud projects describe`; by hand, `gcloud auth login`.
- **The machine**: checks `gcloud services list --enabled`,
  `compute instances describe`, `iam service-accounts describe`,
  `compute firewall-rules describe`, `billing budgets list`,
  `compute routers nats describe`. **Estimate first**: the machine at the
  expected hours, the pd-balanced disk all month, and the Cloud NAT (its
  gateway, 0.0014 USD per VM-hour, and its address, 3.65 USD a month; the
  0.045 USD per GB it carries is extra and said so), from `MACHINE_TYPES`,
  `DISK_MICRO_PER_GB_MONTH` and `NAT_*` (approximate us-central1 list
  prices, labelled with the date they were taken, 2026-11-03; other regions
  differ and the panel says so); the total is the sum of its lines as
  shown, each rounded to the cent. **Create** fetches the estimate again,
  shows it in a confirm dialog naming the project, the machine and the
  monthly estimate, and hands that estimate (in cents) to
  `runner_cloud_create`, which recomputes it and runs nothing unless they
  match. Then, in order, every one shown: the startup script written to a
  0600 file in this Mac's temporary folder (`umask 077 && cat > ... <<'STARTUP'`,
  the whole script in the shown command); `services enable compute iap
  billingbudgets`; `billing budgets list`, then a budget with 50% and 90%
  alerts (before anything billed; skipped when the list already has its
  name, since a budget never says "already exists"); a service account with
  no roles; a firewall rule admitting tcp:22 only from IAP's
  `35.235.240.0/20` to the VM's tag; a Cloud Router and NAT
  (`<instance>-router`, `<instance>-nat`, all subnet ranges, auto-allocated
  address) so the VM, which has no public address, reaches apt, Node,
  rustup, npm and crates (the NAT's `describe` comes first and its create is
  skipped when it finds it: a second create fails as a duplicate, not
  "already exists"); a Debian 12 VM (`debian-12-arm64` for Arm) with
  `--no-address`, shielded VM, `enable-oslogin=TRUE`, that service account
  with no scopes, and the startup script; last, `rm -f` of that file, run
  also when an earlier command failed. The startup script (`STARTUP_SH`,
  `bash -n` clean, `set -u` and `pipefail`, so a failed `curl | sh` fails
  its install) first installs a systemd timer that powers the VM off after
  N minutes with no SSH connection, no `hollow-remote-run` or `hollow-slot`
  process and no install under way (a `/run/hollow-installing` marker,
  counted for 90 minutes at most), so a VM whose install fails or hangs
  still stops and one mid-install doesn't; then `/etc/environment` for
  every login and the default preflight in `/usr/local/bin`; then the
  installs (`dpkg --configure -a` first, git, perl, util-linux, procps,
  build-essential, Node LTS, uv, Rust 1.96.0 shared read-only in
  `/opt/rustup`, installed again through rustup when rustup is already
  there, so a toolchain cut short is repaired, Playwright Chromium with its libraries in `/opt/ms-playwright`), retried
  at the next boot when one fails. A command
  that finds its resource already there counts as done; any other failure
  stops the sequence and names it. A project whose default network already
  has a NAT for that region makes the NAT command fail; Create stops there
  and says so.
- **This Mac's way in**: the key, `gcloud compute os-login ssh-keys add`,
  the ProxyCommand helper `~/.config/hollow/gcp-runner-proxy` (`PROXY_SH`:
  starts the VM when it isn't RUNNING, waits for it, then `exec gcloud
  compute start-iap-tunnel <instance> 22 --listen-on-stdin`), and a
  `Host <alias>` block whose User is the OS Login name from
  `gcloud compute os-login describe-profile` (`StrictHostKeyChecking
  accept-new` for the new VM's key). A stopped VM's first call times out
  in ssh's 10 s banner wait while it starts and falls back; the next one
  gets through.
- **Use it**: as step 7 above.

The commands are the person's clicks: `runner_setup_plan`,
`runner_setup_check`, `runner_setup_apply`, `runner_cloud_estimate` and
`runner_cloud_create` are Tauri commands, and since L13 (2026-11-10) also the
COO's `build.*` actions of the same names (reads, or external with a card).
`real_mac_setup_check` (`--ignored`) runs Check against a real Mac:
`HOLLOW_SETUP_IT_ADMIN`, `HOLLOW_SETUP_IT_HOST` (the runner alias) and
`HOLLOW_SETUP_IT_ACCOUNT`. It runs on the person's Mac (it logs in as the
admin, which the runner account can't). First live run, 2026-11-03, against
the Mac mini that was set up by hand on 11-01, in 1.8 s: admin, account, key
(`id_ed25519_mini_run`, from its Host block), toolchains and preflight Done;
sleep Not yet (no `ClientAliveInterval`; pmset already right); Use it Not yet
only for the owner record (Settings → Record me as owner, or this step's
Apply).

## Disk on the runner

Overnight N5, 2026-11-02. Everything lives under `runner`'s
`~/hollow-remote`, and two things prune it, each only its own folders. A
disk under the preflight's 20 GB refusal recovers by itself: a preparation
prunes sessions before its preflight, and a refusal (there or in
`hollow-slot`) runs `hollow-slot`'s prune once and asks again.
- **Remote sessions** (`HOST_PREP_PL`, at every preparation): a session's
  `sessions/<id>` or `folders/<id>` with `scratch/<id>`,
  `targets/session-<id>` and its small files in `sessions/` goes once it
  hasn't been used for `HOLLOW_REMOTE_DAYS` (14) days. Then, while the
  volume has less than `HOLLOW_REMOTE_MIN_FREE_GB` (40) free, the least
  recently used go one at a time, each deleted before free space is read
  again, and only those unused for 12 hours: a session takes no lock between
  its preparation and its run, and a run can last that long. Never the ids
  the turn being prepared uses (stamped `sessions/<id>.prepared` first, so
  a preparation running beside it sees them as in use), never one whose run
  is live (a `.running` marker whose pid is a `hollow-remote-run`; a build
  workspace's vault folder beside its worktree gets the run's marker too).
  Both are asked again right before each removal, which renames the folders
  aside first, since a preparation beside this one may have started on it.
  Last use is the newest of its folders, scratch, stamp and markers. The
  turn's start notice says when the floor removed some ("The runner had
  31 GB free, under its 40 GB floor, so it removed 2 folders other turns
  left there"). Results are refs in the bare repository, so a pruned
  session's next turn checks out again from them. Copies moved to `aside/`
  (overnight P3L-5) go after the same 14 days, or, oldest first, once 12
  hours old while the volume is under the floor.
- **`hollow-slot --remote`** (`remote_prune`, at every call): worktree
  folders past the newest 8 or unused for 14 days, then, under the same
  40 GB floor, the least recently used ones unused for an hour (what the
  first rules set aside is deleted first, so it counts), never the call's
  own or one whose slot lock another call holds, each removal said on
  stderr.
  It leaves `targets/session-*` alone (a worktree whose id would start
  `session-` gets `w-` in front): until N5 it moved every build folder
  without a `work/<id>.used` to the trash, which included remote sessions'
  build folders, even mid-turn.

Free space comes from `df` on the volume (`HOLLOW_REMOTE_DF` stands in for
it in tests), not `du` of the folder: build folders are APFS clones of one
another, so `du` counts shared blocks once per clone (it reported 41 GB of
build folders on the mini). The floor is twice the preflight's 20 GB
refusal, so pruning starts well before the runner stops taking work. When
the disk is low for reasons outside `~/hollow-remote`, the idle limits keep
pruning from clearing everything a chain is using; the preflight then
refuses and says why.

**Cloned build folders name themselves.** A new worktree's build folder is
an APFS clone of another's, and a session's of its worktree's. Cargo build
scripts write absolute paths into their outputs (tauri's permission lists
name files in its own `OUT_DIR`), so every clone kept reading the first
folder's files, and once the count rule pruned that folder (2026-11-02
03:01, the ninth worktree) every Rust build there failed with "failed to
read plugin permissions". Every run now rewrites text files under
`<profile>/build/` that name another build folder to name its own
(`r_repath` in `hollow-slot`, `repath` in `REMOTE_RUN_PL`; a grep, about
0.1 s), keeping their times so cargo sees nothing new. Not covered: files
with a NUL (binaries), `<triple>/<profile>/build` (no `--target` builds run
there), symlinks, and paths compiled into artifacts.

## Linux runner

2026-11-03, item 4 of the remote runners build. The runner side
(`hollow-remote-run`, the host preparation, the status probe and
`hollow-slot`'s host half) also runs on a Linux host, and a macOS runner
runs the same commands as before. Most Linux code sits behind perl's `$^O eq
'linux'` or `hollow-slot`'s `uname -s` (`hs_os`); a few fallbacks run on any
system once the macOS source gave nothing, which on a Mac never happens:
the `/proc/meminfo` reading, Stop's `/proc/<pid>/cmdline` check and the
`uname -n` / `POSIX::uname` machine name. What changes there:
- **Peer check:** the supervisor asks the kernel with `SO_PEERCRED`
  (`SOL_SOCKET`, `struct ucred`: pid, uid, gid) where macOS has
  `LOCAL_PEERPID` (`peer_pid`). On the accepting end it names the process
  that connected; on the connecting end, the one that made the listening
  socket, so an attacher still checks it reached the run's own supervisor.
- **Non-dumpable runs:** the `--run` and `--attach` processes call
  `prctl(PR_SET_DUMPABLE, 0)` (`no_dump`) in a `BEGIN` block, the script's
  first statement, before perl compiles the rest or loads a module, and the
  supervisor forks from `--run` and inherits it. The syscall number comes
  from perl's own build (`archname`: 157 for x86_64, 167 for aarch64,
  skipped for any other), and a call that doesn't take says so on stderr.
  The leader starts the run with `exec`, so no shell sits between ssh and it.
  From the moment it runs, another process of the account can't read their
  memory (`/proc/<pid>/mem`, ptrace), environment or descriptors, and their
  `/proc` entries turn root's; one that opened a descriptor before (the
  header and secret are already in the stdin pipe when perl starts) keeps
  it. What Hollow's own readers
  lose: nothing they need. The run-tag search reads other processes'
  `environ`, never the supervisor's own; the agent is exec'd, which makes it
  dumpable again; command lines and `stat` stay readable to the account, so
  the running count, marker checks and the parent table work; the status
  probe's "this account's processes" list no longer shows a supervisor, and
  it only looks for a gate's `node` there.
- **Process facts from `/proc`:** command lines, the parent and group table
  the supervisor samples, and the run's tag in other processes' environment
  (`/proc/<pid>/environ` of this account's processes, where macOS has
  `ps -E`), so procps isn't needed. `hollow-slot`'s parent walk reads
  `/proc/<pid>/stat`, and Stop's check of a supervisor falls back to
  `/proc/<pid>/cmdline` when `ps` is missing.
- **Locks:** util-linux `flock -o -n -E 75` for a try and `flock -o` for a
  wait, where macOS has `lockf -s -t 0` and `lockf -s` (`-o` keeps the lock
  from the command, `-E 75` is lockf's exit code for a taken lock). All of
  them, and the perl fallback, take `flock(2)` locks, so they see each other.
- **Memory floor:** `MemAvailable` as a percentage of `MemTotal` in
  `/proc/meminfo`, when neither `kern.memorystatus_level` nor
  `memory_pressure` answers.
- **Build folders:** a new folder starts as a reflink clone (`cp -a
  --reflink=always`, btrfs or XFS) where macOS uses `cp -cR`. Where the file
  system can't clone (ext4: a one-byte probe file's reflink fails) it starts
  empty and the call says so: a whole copy of a cargo target takes minutes
  and holds the other folder's lock. `CARGO_BUILD_JOBS` defaults to half of
  `nproc` when `sysctl hw.ncpu` has no answer.
- **Names and tools:** the machine's name falls back to `uname -n` without
  a domain when there is no `hostname`; the tool report finds Playwright's
  browsers in `~/.cache/ms-playwright` and Postgres in
  `/usr/lib/postgresql/*/bin`, and checks `xcode-select` for `/usr/bin/python3`
  only on macOS.

**Isolation is weaker than on macOS: one account per person.** On macOS a
run's relay token and attach secret live only in perl's memory, which other
processes of the account can't read. On Linux, `no_dump` closes that for the
supervisor and its attachers from the moment it runs (a descriptor opened
before keeps working), but not for the agent: it is an ordinary
process of the account, so another process of the same account can open its
stdin pipe through `/proc/<agent>/fd/0` and write to it, or read its
environment, and nothing inside the run can prevent that. The Linux peer
check proves which process connected; it doesn't make same-account processes
untrusted. So a Linux runner must be one person's OS account, used for
nothing else (a runner is already one person's since item 2), and two
people's runs must never share an account there.

**What stays on the Mac.** The preparation's capability line says the
system (`caps ... os=linux slots=0/2 cargo=free`, from `uname -s`), and the
status probe says it too (`os` in `build.runner_status`). `route_pl` marks
work that needs macOS: `codesign`, `xcodebuild`, `xcrun`, `swift`, `swiftc`,
an MLX tool (Apple silicon only) as the command or as uvx's `--from`
(`mlx_whisper`, `uvx --from mlx-whisper`, `mlx_lm.generate`; not a word that
merely starts `mlx_`, such as `tools/mlx_bench.py`), `tauri build`, a webkit browser for Playwright or vitest
(`--project=webkit`, `--project "webkit"` inside `sh -c`, `--browser
webkit`, `--browser.name=webkit`), and any Rust build, `npm run gate` or
`gate:rust` included, in a checkout whose `src-tauri/tauri.conf.json`
exists (Hollow's links AppKit and WebKit and is built and signed here). With
a Linux runner such work stays here and the routing line says why ("on this
Mac (the runner runs Linux and gate needs macOS ...)"); `--remote` and
`--on` run it there anyway, saying so. A report with no `os=` comes from a
runner tool older than this, so it is a Mac's. Everything else can go: Node
and vitest, Python, Portal, Playwright's Chromium, `gate:frontend`.

**Requirements on the host:** a merged `/usr` (Debian 12 or later, Ubuntu
22.04 or later: the scripts call `/usr/bin/perl`, `/usr/bin/true`,
`/usr/bin/grep`, `/usr/bin/find` and `/bin/sh` by path); perl with
`JSON::PP` and `shasum` (Debian's full `perl` package, not `perl-base`);
util-linux (`flock`); git; and Node, Rust, uv or Postgres as the work needs.
`hostname` and procps are optional. Its login shell's PATH must reach them
for a non-login `ssh host <cmd>`. `/bin/sh` there is dash: `hollow-slot`
was checked under it. A btrfs or XFS volume for `~/hollow-remote` keeps
build folders cheap to start.

**Checked:** in a Debian 13 container (colima, `debian:stable-slim` with
`perl util-linux git`, no procps and no `hostname`, as an unprivileged
account; BUILD_LOG 2026-11-03): the status probe printed `os=linux`, the
peer check named the other end on both sides, two holders of one slot lock
took turns through `flock`, the memory floor read `/proc/meminfo`, a nested
`hollow-slot` ran in its caller's slot (the `/proc` parent walk), the
preparation's line said `os=linux`, and a whole `--on` run against the same
box (a stand-in ssh) brought its exit code back, cloned a build folder and
was stopped cleanly. That run predates the review's changes: `no_dump` and
the reflink-or-empty seeding are checked by unit tests here (the syscall
numbers, the call order, the empty folder) but not yet on Linux, because
this Mac had under 12 GB of disk free for colima. No real Linux runner has
run a whole agent turn yet.


## Sessions and switching machines

A native session lives on the machine that ran it (`~/.claude/projects` or
`$CODEX_HOME/sessions` there). The runner's session folder path is stable per
worktree, so turns there resume their native session. The machine is part of
the folder the resume fingerprint hashes (`chat_resume::config_fingerprint`),
so switching Runs on mid-conversation starts a fresh native session,
re-grounded with the conversation, exactly as switching agents does.

## Testing

- `remote_session_tests.rs` plays the runner with a stand-in `ssh` (the
  command runs in its own session in a fake home; `-R` is a symlink to the
  relay socket) and the runner's own probe agent (`hollow-remote-run
  --probe`), with a real `run_auth` peer check behind a test broker.
- Phase 4 locally: `a_dropped_connection_reattaches_and_the_output_arrives_once`
  (the leader's ssh killed mid-run; stdout and stderr lines once each),
  `stderr_arrives_once_when_the_connection_drops_between_marks` (a scripted
  connection that drops after a line the last `err` mark doesn't cover and
  inside another: the reconnect asks from the exact byte) and
  `a_line_with_ssh_words_joined_on_is_asked_for_again` (ssh's words on a cut
  line of a live connection: it is ended and asked again),
  `input_sent_across_a_drop_reaches_the_agent_once`,
  `a_run_whose_leader_is_gone_is_stopped_at_the_next_preparation`,
  `stopping_the_turn_ends_the_agent_on_the_host` (Stop through `collect`) and
  `a_bridge_reconnects_and_replays_initialize` (a stand-in tool socket, no ssh).
- After a quit (overnight R1): `a_restarted_hollow_reattaches_to_its_run_from_the_bytes_it_handled`
  (the leader killed with its group after Hollow handled one line; a new
  leader from the record attaches: the rest once, a tool call served through
  the run's relay token, the work applied), `another_message_after_a_restart_brings_the_earlier_turns_work_back`,
  `a_stale_turn_whose_run_is_gone_is_forgotten`,
  `only_a_continue_of_a_claude_turn_in_place_reattaches` and
  `progress_counts_the_bytes_of_each_line_handled`; against the real runner,
  `real_runner_reattach_after_a_restart`.
- The git identity: `the_identity_is_the_checkouts_own_with_includes` (global,
  `includeIf` by `gitdir` and by remote URL, a repository's own config, none,
  a name without an email) and `the_agents_commits_there_carry_this_macs_identity`
  (stand-in end to end, with and without one); the header test checks it is in
  the header's environment and on no argv.
- `real_runner_round_trip_and_stop`, `real_runner_workspace_folder_round_trip`,
  `real_runner_reattach_after_a_drop` and
  `real_runner_commit_carries_this_macs_identity` (`#[ignore]`) run the same flows
  against a real runner: `HOLLOW_REMOTE_IT_HOST=runner-host cargo test --lib
  remote_session::tests::real_runner -- --ignored --test-threads=1` from this Mac.
