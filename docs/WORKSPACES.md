Tracker project workspaces carry `tracker_project` in WORKSPACE.md and
PROJECT.md, plus the provider in `tracker` and the local mirror reference in
`tracker_project_path`. Their kind is `tracker`, including when a linked legacy
folder has been renamed. Rename, local archive/delete, merge-from and department
changes refuse a managed project; change its provider fields through the tracker.
Gardener and archive suggestions exclude it. The strict legacy Notion UUID
recognition stays separate, so a similarly titled ordinary workspace stays ordinary.

New projects use `workspaces/<provider>-<slug>-<id8>/`. Previously stored or
discovered paths keep their ids. Generated TRACKER.md joins the context envelope;
retained NOTION.md is still read. Authored TRACKER.md is preserved with an error.
Existing migration briefs without an explicit tracker binding are preserved byte
for byte; PROJECT.md supplies their managed identity until the importer binds
the brief. Connected-folder twins and retrieval use the same managed identity.

The built-in Chats workspace (`quick-chats`, called Quick chats until
2026-11-04) is no longer in the sidebar (L8, 11-10): a new prompt with no
project goes to the COO, and its chats are under the COO's Subagents chip →
Earlier chats. No paths or chat IDs change. A project is one sidebar row that
opens its lead ([Project leads](COO.md#project-leads-l8)). New projects require a unique one-line name; instructions
are optional (ordinary Create uses the name as the initial outcome).
Create and start remains explicit and requires instructions.

Add to project replaces Move to in chat menus and is also visible on Chats
and Home. Its shared host dialog refreshes eligible projects and ranks
real conversation activity, then validates the selected project before Add.
Opening and choosing are read-only; Cancel does nothing. Existing-chat moves
keep Undo and Move to the COO (the quick-chats folder, 11-10). Home's choice persists with its draft and
sends only into the successfully opened fresh chat, without consuming foreign
composer text or overwriting a pending send. Main remains a separate page.

> **Studio update (2026-09-27; merged as PR #193 `c0c9fc5` and installed as 0.5.3
> the same day):** see [Product Studio](history/PRODUCT_STUDIO.md) (now history) for that
> release's design and feature plan. Later releases changed parts of what follows; the
> side panel was replaced on 2026-10-27 by the Preview pane and the Workspace view
> (run 15 P1), and that view's sections went on 2026-11-10 (COO rebuild L4), described in [What Hollow is](reference/product.md) and in
> "Conversation-first interface" below; what this note says about the panel
> (its Side panel view selector, Panel actions, Expand/Reduce) is history.
> At the time, the workspace panel used one **Side panel view** selector in place of the
> Context/Files/Visuals tab row plus Brief/Plan/Sources/Output tab row. All views,
> including build Changes and optional Notion, remained available. Panel actions
> contained Expand/Reduce and Refresh; File actions contains Ask about this file,
> Reload file (keep draft), and Show file in Finder. A single-file output shows its filename once; multiple
> files get a selector in that same toolbar. Save appears only while dirty.
> New workspace is in the rail and Work returns to the overview. The
> conversation no longer repeats a context bar. Existing resizing, narrow-mode
> behavior, exact-session resume, draft preservation and file guards are unchanged.
> Home is now [chat-first](#chat-first-home), with recent projects and an expanded
> searchable list. Saved workspace outputs remain in the workspace's Results (removed 11-10).
> Older layout descriptions below describe the previous tab chrome.
> The [core workflow finish](CORE_WORKFLOW.md) adds the compact, evidence-backed
> Welcome back recap (removed 11-10), live result arrivals, explicit loading/retry states and
> ⌘S / Ctrl+S in the guarded file editor.

# Workspaces and Library

Installed baseline as of 2026-09-24: version 0.5.3.
The [Projects + Vault feature](DIRECTORIES.md), conversation-first UI and
automatic intent are installed from the merged Pretoria source. See
[build identity](reference/build-and-ship.md#build--ship-how-a-change-reaches-the-installed-app).
This builds on the implemented first slice of
[the approved product reset](history/HOLLOW_PRODUCT_RESET.md).

Hollow can discover existing repositories and project documents from one Projects
directory, creating stable linked workspaces while keeping originals in place.
The separate Vault stores Hollow context and outputs. [Directory setup](DIRECTORIES.md)
covers onboarding, Settings, Library browsing, refresh and query-selected evidence.

Hollow starts with an outcome. A workspace keeps its current context, source
material, agent conversation, and deliverables together. Library is a simple
shared file list and reader. Automations retains the existing explicit workflow
runner. Settings configures files, agents, and optional connectors.
[Tasks](TASKS_WORKSPACE.md) uses the same shell with a focused list and context
pane, direct workspace links, and guarded tracker field editing.

## New workspace form

`components/NewWorkspaceForm.tsx` loads only on a New project request;
Work keeps it mounted after closing until navigation leaves the page, so the
form's owned pending send can reach the conversation before cleanup.

Sidebar → Projects + and Work → New project explain the first reason
Create is unavailable: an invalid/duplicate name, an untouched starter, a missing repository or a message still
being accepted. Command-Enter submits ordinary Create; Escape cancels when
idle. These form-local keys have the same visible Create and Cancel buttons.
Build mode labels its optional starting revision Base branch, and its kind
button is the single build choice inside the form.

Create and start saves the goal in the brief and queues that exact goal as a
preset for the newly opened conversation, scoped by cwd and fresh session key.
It never consumes a foreign draft or attachments. A delayed creation cannot
take navigation back from another page or conversation. A failed open keeps
the saved workspace and form, offering Open project / Open and start without
creating a duplicate. The receipt survives page remounts until explicit Cancel
or a successful open; an unconsumed first goal is cancelled when its conversation
leaves and its receipt is restored for explicit retry. Inputs are frozen while
creating and after creation.
If creation rejects after navigation, its submitted title, goal, kind,
repository, branch, base and error remain available through New project.
Retry is explicit; Cancel dismisses that failed draft.
The retained fields are the last submitted snapshot, not later unsent edits;
an existing failed draft or created receipt takes precedence over a starter.

Repository discovery rejects missing Projects configuration and unreadable
roots with reasons; Retry repositories keeps the form values. A readable
empty directory is still a successful empty list. The IPC success payload
remains `RepoCandidate[]`. Failed workspace-list reads use failure copy and
Retry rather than continuing to claim Loading.

## Conversation-first interface

The conversation is the primary workspace. `Work.tsx` mounts the full shared
`ChatBody` in the centre, always. The workspace panel and its sections
(Overview, Brief, Plan, Results, Sources, History) are gone (COO rebuild L4,
2026-11-10); a project's notes are files. The top bar's **Project files**
opens WORKSPACE.md in the Preview pane (pressed again, it hides the pane). The
pane's **Files** picker pins WORKSPACE.md, PLAN.md, SOURCES.md and AGENTS.md
("Project notes"), then a build workspace's "Changed files" (its last row
opens the Changes tab), then "This conversation" (`src/projectFiles.ts`).
PLAN.md's checkboxes tick in the Markdown view. A build workspace's branch
chip ("Changes on <branch>") opens a **Changes** tab in the pane.

Files the agent wrote and things it showed are not in a panel any more:
previews and canvases render in the transcript, each reply's footer chips list
its files, and a click on a chip, a link, a citation or a canvas's Expand
opens the app-wide **Preview pane** beside the page (`PreviewPane.tsx`,
`store/preview.ts`). Two chips go elsewhere: one for a file this workspace's
Results lists (in its `outputs/`) opens it in the Preview pane with its
**review bar** (`ResultReviewBar.tsx`: state and checks, origin, Mark ready,
Accept, Compare with accepted, Ask for a revision, More), with **Review
result** on its right-click menu and **Review <file>** in the reply's More
actions; one for a deleted file opens
its change in Library history (a repository file: the run's details).
Nothing opens the pane when a turn ends. ×
/ Escape inside it / ⌘\ close it. Drag its left edge to resize it (arrow keys
on the focused edge move it 16px, Shift 64px; double-click returns to the
default width). The width is saved (`sidePaneWidth`, localStorage
`sidePaneWidth`); while the pane and the page are side by side, the page
keeps at least 360px. (Whiteboards were removed on 2026-10-07.)

In a compact or medium window (up to 68.75em, 1100px at Default text size)
the Preview pane takes the page's place, with × as the way back; the page
stays mounted. Cmd+Shift+C focuses the workspace composer and
brings Chat back. From other pages it
returns to the active workspace; without one it opens workspace creation.
Historical conversations remain available through Library → History.

Workspace starter cards prepare next-step, first-draft and review requests. They
append to an existing draft and never send automatically. The full composer
infers the approach from your request without Ask/Do/Review/Automate buttons.
The agent/model/permission popover remains available.
The Context button (a folder icon) names only what is news: No project (a chat in Chats), a
project a conversation just moved to, or its source count; its tooltip retains
the exact cwd, and the empty box's placeholder names the project.
Existing bounded context, permission and session-resume contracts are unchanged.

Home lists recent projects; All projects adds search and archived projects.
Rows retain non-active status, merge notes, the latest chat title and running
chat summaries. Duplicate names show distinguishing origin metadata; every
row's tooltip retains its full name and origin. Empty searches offer recovery.
The rail keeps the established navigation and shortcuts with a compact visual
hierarchy and accessible full workspace names even when labels truncate.

Library initially uses a full-width file grid, grouped by folder (`src/libraryGroups.ts`).
Each row shows the note's own title (frontmatter `title` or first `# heading`,
an ID prefix like `Q0c68d734b62a —` dropped) and its first sentence, from
`vault_notes_list`'s `title`/`summary`; the path is the row's tooltip.
Evidence, raw, readable, archive and attachment folders start folded; a folder
with one note joins its parent unless that note is the folder's own hub
(`atlas/Atlas.md`). Groups preview 12 rows with Show all. Selecting a file opens the list
and reader; All files returns to browsing without clearing the search/filter. The open row
carries `aria-current`. A writable note (`canManageNote` in `src/libraryNoteManage.ts`: Markdown
outside `workspaces/` and hidden folders) has a ⋯ on its row and, in the reader's File actions,
Rename…, Move to folder… and Move to trash (P1-18): `vault_move_note` then a note-index refresh
and the new path reopened; Rename warns first about notes linking by name. Trash confirms and
offers Undo, which moves the note back from the `.trash/` path `vault_delete_note` returns.
None runs while the note has an unsaved draft, open or kept. Linked from names notes by title.
Library's views are the shared `Tabs` (navigation "Library view": All, the default since
overnight 11-12 L3 and one searchable list over every reader
([details](reference/surfaces.md#library--all-overnight-11-12-l3)), then Connected projects,
Conversations, Skills, History, Learnings); a tab starts clean (no open file, history or New note form).
Notes, Results and Recordings are All's views since overnight 11-12 L4: Browse folders, Outputs →
Grid and Meetings → Recordings open them, the strip keeps All selected, and Back to All returns.
New note (beside All's search, or the page header elsewhere) opens
its form in Notes with the name field focused. Library actions → Show file history opens the
history panel in Notes from any tab and reads Hide file history while shown. By meaning keeps its
hits inside the chosen collection or folder ("12 in People by meaning"), and a saved view keeps it
(`byMeaning` in `hollow.library.views`; older views read as by name). The collections that hold a
view's files are named apart from it (Result files, Kept transcripts) and link to that view. Library
→ Results reads its list again, debounced, when the watcher reports a change under any
workspace's `outputs/` (`vaultChangeUnderAnyOutputs`), and has Refresh (P1-19). In the browse
state the folder picker sits under the search field. A compact description
leads one wrapping utility row for count and Pin/Save; saved chips stay
together beneath it. Browse cards show listed title/summary with a paper-like
file mark, without additional reads. The reader's sidebar stays compact.
Recordings retain the existing mic identity (no waveform data is provided),
date/duration and two-line names. Transcript segments share a fixed time gutter
and put the existing speaker above the exact words; absent speakers add no label.
Tasks similarly reserves its detail column for a selected task or creation form.
Automations uses a list/detail switch below 900px, with All automations returning
to the list. Its rows have keyboard-accessible open buttons, and search misses
have a distinct Clear search action. A **Permission changes** review panel
(`ReviewInbox` context `automations`) sits above the list/detail split, so it is
visible at every width; it appears once any permission review exists. It lists
automations whose stamped settings or prompt changed outside Hollow's editor,
with a structured before/after diff, Approve (always confirmed; a danger
confirmation when access widens) and Reject — no editing or snooze. A refused
Run now (or a refusal notification) opens it. Beside it, a **Failing
automations** panel (context `automationHealth`, Paperclip PR 9) appears once
an automation has failed three times in a row: one card per automation
("Failed N times in a row · <last error>"), **Pause schedule** (sets the
schedule to manual; shown only while the live automation has one; triggers
keep running), **Dismiss** and snooze, no editing. Both stay until the
automation succeeds again; the streak notification opens this panel. A **Budget pauses** panel
(context `budgets`, Paperclip PR 8a) sits beside it for automations a
hard-stop usage budget paused. Settings has a desktop section-index search (including palette keywords);
the narrow section selector remains available. Agent-default fields use explicit
columns (stacked on narrow windows), preventing full-width inputs from collapsing
their explanatory labels.

The shared shell, inspector and secondary-page refinements live in
`src/styles/workbench.css`, scoped to `.hollow-shell` where they supersede retained
legacy styling. Theme color tokens remain authoritative; no new palette or
backend/data migration is introduced.

## Legacy Notion workspaces

Notion two-way sync was removed on 2026-11-10 ([history](history/NOTION_SYNC.md));
projects are tracked in Linear ([Tracker sync](TRACKER_SYNC.md)). A `notion-<id>`
workspace it made is now a plain local workspace: rename, status, archive,
delete, merge and department all work, unless the importer bound it to a
tracker project (`tracker_project`), which makes it a tracker workspace. It
still counts as a connected project's twin (`directory_connections`,
`retrieval_broker`), `list_workspaces` kind `notion` still finds it, and a
retained `NOTION.md` is still read as context. Hollow no longer syncs with
Notion; its official connector (`notion-official`) is an ordinary connector once
signed in.

## Build workspaces

Build PR B1 ([build plan](history/HOLLOW_BUILD_PLAN.md)). New workspace → **Build in a
repository** picks a git repository (the connected Projects folder's repos, or
Choose folder…), an optional branch (default `hollow/<name>`, numbered when
taken) and an optional base (default: origin's default branch, fetched first;
a failed fetch is a warning, not an error). Hollow runs `git worktree add -b`
into `~/Hollow/Worktrees/<repo>/<name>` (`<hollowHome>/Worktrees`), then
creates the workspace and binds the two in `build-workspaces.json` in the app
data folder, outside the vault. A failure after the worktree exists removes it
and the branch again.

**Clone-seeded `node_modules` (OC7, 2026-11-17).** A new agent-lane worktree starts with
no `node_modules`. Right after `git worktree add`, `create` (and only `create`: Build
app's release build keeps its clean `npm ci`) looks through `git worktree list` for a
sibling or the main checkout, under the worktrees root or the repo root, whose
`node_modules/.hollow-lock-sha256` marker equals this worktree's `package-lock.json`
sha256, whose own lock file still hashes to the same value (a sibling that ran `npm
install` since is skipped), and in which no top-level file or `.bin` entry is newer than
the marker. It clones that folder with `/bin/cp -c -R` (APFS copy-on-write; never a
symlink or hard link, so one worktree's install can't change another's) into a fixed
staging folder in the worktree's git directory (never in `git status`), then renames it
into place only when the clone carries the marker and the sibling's lock and marker
still match. **`cp -c` is not fail-closed**: on a volume without clone support BSD `cp`
quietly copies every byte (minutes for `node_modules`). So the clone is tried only when
the worktree and the source are on APFS (`statfs`) and one device (`st_dev`), and the
child gets 90 s before it is killed and its staging removed. A changed or missing hash,
a torn, stale or slow clone, another volume or any other error is a quiet skip: nothing
is left behind and the lane runs `npm ci` itself. The marker (the lock's sha256) is
written by Build app after a successful `npm ci`; a `node_modules` an agent installed by
hand has none, so it is never a seed. Since a clone keeps file times and the carried
marker, a clone may itself seed a later worktree (untested for `.bin` links). **Residual risk:** a sibling's
installed tree is trusted only as far as its marker plus the shallow time check. An
agent in that lane could have altered files inside a package folder or left a symlink
inside the tree, and the clone copies both as they are (symlinks are not followed or
rewritten). Code: `seed_new_worktree` and `seed_node_modules_with` in
`build_workspaces.rs`. The runner's own `npm ci` rule is separate (`.hollow-remote-lock`,
`build_slots.rs`).

While bound, the workspace's `cwd` is the worktree: every conversation in it
runs there, the agent loads the repo's own `CLAUDE.md`/`AGENTS.md`, and the
receipt's repository changes come from it. The `<hollow-workspace>` envelope
gains one sentence naming the worktree, branch, base and the absolute vault
folder (the brief, plan and outputs stay there). Claude and Antigravity get
that folder as an extra `--add-dir`; under Codex's sandbox it is read-only.
The header shows the branch chip, and cards read `Build · <branch>`. Right-click
→ Reveal worktree in Finder, Copy worktree path, Copy branch name, **Remove
worktree…**. Removing refuses while a run holds the vault, and asks a second
time when there are uncommitted changes or commits neither the base nor a
remote has (`WORKTREE_UNSAVED:`). It keeps the branch. The workspace then runs
in its vault folder, and its worktree conversations stay listed. Delete is
refused while the worktree exists.

**Scoped runs (B3; every workspace since 2026-09-28).** A turn in a build
workspace, or in any workspace whose conversation runs in its own vault
folder, does not take the full-vault lease. It is a scoped history run: its baseline and diff cover
only the workspace's vault folder, its repository changes come from the
worktree as before, and it runs while an automation holds the vault (and an
automation starts while it runs). Overlapping full-vault runs leave that folder
out of their changes. Its task tools are read-only, History & recovery labels
it by its workspace, and Remove worktree and deleting the workspace wait for
it. Hollow app tools are read-only there too. Since 2026-10-20 (F5) a restore
waits for it only when it would change a file in its folder, or a file Hollow
changed on its behalf elsewhere (a task write), and names the folder; restores
everywhere else go ahead, while an automation or vault-root conversation still
holds them all off. Clearing History keeps a running conversation's record and
the stored versions it needs and clears the rest.

**Several conversations in one workspace (2026-10-07).** A workspace's own
folder, Chats included, runs several of its conversations at once
(up to Settings' Chats at once in one workspace, 5 by default and 10 at most; 2026-10-20);
a build workspace's worktree still runs one turn
at a time. Each overlapping run's History covers the whole folder, so the run
that finishes second marks the files both changed ("shared" in History &
recovery, with a banner naming the other run). Restoring a shared file asks
first, since History can't tell whose edit it was; the run's other files
restore one at a time, and its whole-run restore is off. While another
conversation runs beside it, a turn is told to re-read the brief and plan
right before editing them; a turn already running when the other began learns
of it at its next turn. A peer whose History record is missing or broken may
have changed anything, so every file of the run in the folder counts as
shared; History re-checks the peers' records when it lists runs and before a
restore, so a mark that failed to save is never the only guard.

**Who gets the credit (chat concurrency F1/F3, 2026-10-20).** A run that
finishes while a peer is still live marks the files that peer's diff will
also hold (the peer's baseline against the folder now), so its receipt says
so at once. A chat turn hands History the files its own successful structured
writes named (Edit, Write, MultiEdit, NotebookEdit, apply_patch, Codex's
`file_change`; absolute, relative to its folder, or through `~/Hollow/Vault`)
before its finish (a write is the file the path really reaches, resolved by
the file system; a Claude subagent's calls stream into the turn and count as
its own, since Hollow can't tell them apart from the main thread's). If it
shared the folder at any point, every other change in
its diff is marked `unattributed` ("another chat" in History & recovery), its
receipt lists the file as `shared` ("changed by another chat") with the runs
that also changed it, and the file is not credited to it: Results' origin,
the task-state record's artifacts, line totals, child-task and Activity file
counts and artifact sessions skip it, and a read-only reviewer's receipt says
it changed nothing (`noChanges`). What Hollow's app and task tools wrote in
the folder for the run counts as its own: only the files those writes
touched, on whichever thread the operation used (`vault::logging_writes`,
carried into workers by `vault::spawn_blocking_logged`), never a peer's write
that lands in the
folder meanwhile. A file changed only by shell
commands (`sed -i`, a redirect) in a shared folder is unattributed too: a
conservative miss. Restore still goes by the shared marks. A turn's session
record (`sessions/<key>.md`) is written after its run ends, while peers may
still be live, as a host write (`automated_history::host_write`, which notes
the bytes the write committed, not what is on disk afterwards): a peer's
History leaves it out unless something else changed the file too or the
peer's own writes named it. What [Shared docs](#shared-docs-while-chats-run-in-parallel)
writes at a run's finish (a merge-back of Hollow's own change, a conflict
copy) is a host write made for that run (`automated_history::host_write_for`):
it stays in the finishing run's History, as that section says, but is
`unattributed` there (not the agent's work, unless its own writes named the
file), and the other live runs leave it out. A history restore needs nothing
of this: it refuses any path in a live scoped run's folder or broker changes,
so its writes never reach a live run's diff.

**Build settings (B7).** Each build workspace's turns run with their own Rust
build folder (`CARGO_TARGET_DIR` under `~/Library/Caches/hollow-build-targets`, deleted with its worktree),
their own dev-server and browser-test port (`PORT`, `PW_PORT`), this Mac's
toolchain caps, and `hollow-slot <command>`, which holds one of the machine's
two build slots while the command runs and waits when both are taken. The
agent is told to run heavy builds and tests through it, taking, running and
freeing the slot in one call (a slot is a `flock` on
`/tmp/hollow-build-slots-<uid>/slot-N.lock`, freed when the command exits, so
a wait only ever costs the call's own time limit).

**Where the locks live.** `/tmp/hollow-build-slots-<uid>` (`build_slots::slot_dir`,
and the script's own default: they must agree), made 0700 by the tool, which
refuses it (exit 1) when it is a symlink, not a folder, or another user's;
`build.slots_status` then reports it `untrusted`, its locks unread.
`HOLLOW_BUILD_SLOT_DIR` overrides it, and Hollow passes it to every turn that
gets the tool. Until the OF07 review it was
`~/Library/Application Support/Hollow/BuildSlots`, which sandboxed agents
can't write: Codex's `workspace-write` writes only the working folder and the
temp folders, so `hollow-slot` failed in every sandboxed Codex turn. A temp
folder works for every agent that can run commands (Grok's `read-only` and
`workspace` profiles write `/tmp` too); not `$TMPDIR`, which differs between
processes. A Hollow still running an older build uses the old folder until it
restarts, so for that while the two sets of slots don't see each other.

**Slots for every chat (OF07, 2026-10-17).** Since OF07 the caps
(`CARGO_BUILD_JOBS=8`, `VITEST_MAX_WORKERS/THREADS/FORKS=4` and vitest's
minimums of 1; `build_slots::CAPS`), `HOLLOW_BUILD_SLOT_DIR` and
`hollow-slot` first on PATH go to every chat turn that can take a slot
(`build_slots::can_take_slot`: every mode but plan, except that Antigravity's
sandboxed accept-edits runs no commands, so only its bypass gets them;
`chat::build_workspace_env` → `build_slots::chat_env`); only a build workspace
also gets its own `CARGO_TARGET_DIR`, in every mode as before, and its
worktree's turn its stable `PORT` and `PW_PORT` (1500-1899). Grok's `default` runs under its `read-only` profile, which writes
`/tmp`, so it takes slots too. Chat turns only: workflow steps, heartbeat
passes and `ask_agent` delegates aren't given them yet (a delegate has
whatever its parent's agent passes on to the `hollow_team` server). The
capability index says so in one line, under the same rule, and only when
`hollow-slot` was actually written and put on the turn's PATH.

`HOLLOW_THIS_MAC` names `this-mac.md` beside `hollow-slot`. `this_mac.rs`
regenerates it when the tool is ensured, writing only changed bytes. It lists
detected cores and memory, the slot and target folders, the runner placement
uses (`remote_session::runner_setting`, so a value placement rejects is
reported as no runner) and the caps from `build_slots::CAPS` and `SLOTS`. This is one plain
machine note for every provider, at most 3,000 bytes. Generation is best effort
and never removes the slot tool on failure; a failed generation removes the old
note, and a turn gets `HOLLOW_THIS_MAC` only while the note exists. Provider
context files stay untouched, and no provider is told to read the note until
Alex moves his own machine notes.
`hollow-slot --dev-server <command>` holds a third lock, `dev-server.lock`: one dev server at a time on
this Mac. The command gets `PORT` and `PW_PORT`: the turn's own `PORT` (every
chat turn that can run it has one, below), else `HOLLOW_DEV_SERVER_PORT`, else 1499. While another
holds it, it tries three times a second apart, then exits 75 naming the
holder. Exit 75 is also what a taken lock returns, so a command that itself
exits 75 is retried (a build in the other slot and after each poll, forever
if it always does). Whoever takes a lock writes `<lock>.holder` (pid, since,
port, command line or `HOLLOW_SLOT_LABEL`; written aside and moved into
place), and the read-only app action
`build.slots_status` lists each slot and the dev-server lock: held or free
(it tries each lock for an instant), and the holder while held. Dev servers
started through `hollow_services` don't take the lock.

**One Rust build at a time (2026-10-20).** The two slots stay, but at most
one of them compiles Rust: two Hollow compiles together peak around 13 GB (one
`hollow_lib` rustc reached 6.36 GB). A command `hollow-slot` takes for a Rust
build holds `cargo.lock` in the same folder as well as a slot. It is one when
an argument is `cargo`, `rustc`, `cargo-*` or `tauri` (or a path ending in
one), or a word of the string after a shell option cluster holding `c` (`-c`,
`-ce`, `-lc`, `-xec`, as in `sh -c '…'`; other wrappers such as `perl -e` or a
script file aren't looked into) is, or it runs one of the repo's npm scripts
that run cargo (npm's options before or after `run`, such as `--silent` or
`--prefix "/tmp/my project"`, are skipped, an option's value being one whole
argument): `gate`, `gate:rust`, `perf`, `tauri`, `tauri:dev`, `tauri:bundle`,
`tauri:build`, `tauri:rebuild` (not `gate:frontend` or `tauri:install`).
`HOLLOW_SLOT_CARGO=1` makes any command one, for a wrapper whose words don't
show it; `=0` opts one out. A Rust build takes both locks in one try per
slot (the slot, then the cargo lock without waiting) and, if either is taken,
holds neither and tries again after the poll (`HOLLOW_SLOT_POLL_SECS`, default
5); while the cargo lock is plainly taken it doesn't touch the slots at all.
On the remote runner a Rust build tries two lanes (R0, below), so the same
holds for "every lane taken".
Nobody ever waits while holding a lock, so no wait can close a cycle. It says
once "waiting for the other Rust build to finish (one at a time on this Mac)"
or that both slots are in use. Lock files are close-on-exec (perl's set and
checked; macOS `lockf(1)` keeps its own from the command), so a lock lives
exactly as long as its holder: a `kill -9` frees it at once, and the build
that holder started then runs on unlocked. A slot's command gets
`HOLLOW_SLOT_HOLDER_PID` and a Rust build's also `HOLLOW_SLOT_CARGO_PID`
(its own pid). A `hollow-slot` called from inside such a command (the pid is
a live ancestor, found by walking `ps -o ppid=` up to 64 levels) takes no new
slot: a non-Rust command runs directly in its caller's slot, a Rust build
waits for `cargo.lock` alone (holding nothing; the slot is its caller's), and
a Rust build inside a Rust build is refused at once with exit 2, "already
inside a Rust build that holds the cargo lock; run cargo directly instead of
through hollow-slot". The hints are only hints: a wrong or forged one can move
a call out of a slot or refuse it, never let a Rust build skip `cargo.lock`. A
same-user process forging them on purpose is outside the tool's reach; it can
opt out with `HOLLOW_SLOT_CARGO=0` anyway. `build.slots_status` reports the
lock as `cargo` with its holder, the command's own pid, and the runner's second
lane as `cargoLane1` (free on this Mac, which doesn't set `HOLLOW_SLOT_RUST_LANES`;
that is cooperative, an exported `HOLLOW_SLOT_RUST_LANES=2` here would open it). Two known
limits.
`--dev-server` is unchanged: it takes only the dev-server lock and never looks
at the command, so a Tauri dev server (`hollow-slot --dev-server npm run
tauri:dev`) compiles Rust without the cargo lock; this Mac's rule already keeps
a dev server from running beside a build. And exit 75 still means "a lock is
taken": a Rust build that itself exits 75 can't be told apart from that, so it
is run again forever, like any build that exits 75.

**Remote runs (2026-11-01, the Mac mini runner).** `hollow-slot --remote
[--fetch <path>]... <command>` runs a test or gate on this Mac's remote
runner instead (since smart routing, below, a plain call decides
for itself and `--remote` forces it): the SSH host in `HOLLOW_SLOT_REMOTE`, else the first line of
`~/.config/hollow/slot-remote` that isn't blank or a `#` comment (`off`,
`local` or `none` turn it off; Settings → Devices → Remote runner
writes it, Set up and Turn off, remote runners 2); `--on <host>` names one
for a single call. A
host is just an SSH alias with a key (`ssh -o BatchMode=yes`, no agent
forwarding needed); on Alex's it is `runner-host`, the Mac mini's
`runner` account, which has no GitHub credentials and no agent sign-in.
The call sends the worktree's exact state: HEAD when `git status` is clean,
else a commit of HEAD plus every uncommitted and untracked file (ignored files
such as `.env` stay here; the chat's attachments in `.hollow-attachments/`,
ignored by their own `.gitignore` of `/*`, go by name), built with a copy of the index in a private object
folder, so this repository gets no new objects or refs. `git push` puts it in
a bare repository per local repository in the host's `~/hollow-remote/repos/`
(the first push sends the history, about 80 MB for Hollow, in about 12 s on
the LAN; HEAD also goes to a `<hash>-base` ref there, so later pushes send
only what changed and a whole call costs under a second; without that base
the host's only ref was a snapshot commit this side never has, and every push
sent the whole history again). There each local worktree has its own
checkout (`work/<worktree name>-<hash>`, a git worktree of that repository,
so `build.rs`'s provenance script sees a real checkout; a leading dot in the
name becomes `_`, since 2026-11-03, because `ls` and the prune's `*.used` globs
skip dot names, so such a folder was never pruned; folders named with a dot
before that are pruned too), its own
`CARGO_TARGET_DIR` (`targets/<same>`; a new one starts as an APFS clone of the
newest free one of the same repository's there, then the checkout's files are
touched so cargo can't take a cloned test binary for fresh; since 2026-11-03
`work/<same>.used` holds the repository's id, only a folder whose id matches
seeds it, and a checkout that tracks no `Cargo.toml` gets no clone: before,
the newest folder of any repository seeded every new one, Rust or not), `TMPDIR=/tmp/hs-<6 hex>` (short enough
for the prompt-budget tests and `gate-run.mjs`'s 16-character limit), and a
`PORT`/`PW_PORT` in 2300-2699 that the folder keeps while it exists
(`work/<same>.port`, dropped with it): the port its name hashes to, else the
next one no other folder there holds, picked under `work/ports.lock` (until
2026-11-01's close-out it was the hash alone, so two worktrees shared one
about once in 400). A folder that is broken or belongs to another repository (a crash
left `index.lock`, a renamed checkout) is made again. Ignored files stay between runs, so
`node_modules` is reused; `npm ci` (then `playwright install chromium`) runs
when `package-lock.json` changed. The command runs in the same subfolder,
through the host's own copy of this tool (the caller sends it each time;
`~/hollow-remote/bin/hollow-slot`), so it takes one of the host account's two
slots and a Rust lane, `cargo.lock` or `cargo-1.lock` (per user: `/tmp/hollow-build-slots-<uid>`; **two gates compile at once there since R0, 2026-11-14**, below), with the host's own caps from its login environment
(`CARGO_PROFILE_{DEV,TEST}_DEBUG=0` when unset, so targets stay flat). None of
this Mac's environment goes along: write `env NAME=value <command>`; only
`HOLLOW_SLOT_CARGO` is passed, and (2026-11-02) the git identity this Mac uses
for the checkout (`r_identity`: `git -c user.useConfigOnly=true var` in it,
includes and `includeIf` honoured), which the command gets as
`GIT_AUTHOR_NAME/EMAIL` and `GIT_COMMITTER_NAME/EMAIL` so a commit it makes
there is this Mac's, not a guess from the runner account. It travels as the
first line of the preparation's stdin (never a command line), waits in
`work/<id>.<token>.ident` (0600) for that call alone, and is removed as the run
starts (a call that never ran leaves it for the next day's prune). With none
configured here nothing goes and git's default there stays; a job (`--send`)
gets none. Output streams back as it comes and the exit
code is the command's; the call says where it ran and how long it took.
**Slow setup is named (overnight N3, 2026-11-02):** each step before the
command is timed in whole seconds on both sides (here: reading the checkout,
ssh and preflight, push; there: the run's own ssh login, prune, waiting for
its folder, checkout, build folder, npm ci when it ran, temp folder and port;
this side's steps travel in `HOLLOW_SLOT_SETUP` in front of the command,
which a host tool without this ignores, and none of the timing variables
reach the command; the ssh login is the one step measured across the two
machines' clocks, from `HOLLOW_SLOT_SETUP_SENT`, and is left out unless they
agree within 0 to 600 s; a host tool from before this lets the variables
through, harmlessly; an "ssh login" of the same few seconds on every call
means the host's clock runs ahead, not a slow login), and when they add up to `HOLLOW_SLOT_SETUP_SAY` (5) seconds or more
the host says one line before the command, such as `setup took 21 s before
the command on build-mac: push 18 s, checkout 2 s`, naming only steps of a
second or more. Wrapping up after the command (ending leftovers, packing
results) says the same way, and this side says when copying the results back
took that long. Waits that already say themselves (a host slot, its cargo
lock, its memory floor) come after the line. Measured on the Mac mini with
the full gate (193 s wall, 191.7 s of gate): setup 1 s, wrap-up under 1 s,
copying back 1 s; a brand-new worktree's first call 3.7 s, a 6.6 GB build
folder's clone 0.8 s. The one Phase 1 call that spent 20 s before its command
wasn't reproduced; the line names the step if it happens again. Two
calls from one worktree take turns with its host folder (a lock per
worktree); calls from different worktrees share only the host's slots.
Whatever the command leaves running in its process group is ended when it
exits (TERM, then KILL after 5 s), as `gate-run.mjs` does, since it would
hold the session's output. Before letting the folder go, the host packs the
`--fetch` paths, new gate records and new gate run records; afterwards `--fetch <path>` (the
command's folder's, like its arguments; never a `.git` part) replaces that
path here with the host's copy (never through a symlinked folder here).
Since 2026-11-03 only a path this run wrote comes back: something in it
changed (by ctime, so `tar -x` or `cp -p` output with old modification times
counts) after the command started. An ignored path stays on the host between
runs (`git clean -d` keeps it), so a run that failed before writing
`test-results` used to bring the last run's copy back and say "copied ...
back"; now the host says the path "is older than this run (the command
didn't write it)" and nothing is replaced here. A folder the run wrote
anything in comes back whole. So an incremental tool that finds nothing to
do (a no-op `make`, `tsc -b`) has its output refused even when this Mac's
copy is missing: make the command write it, by deleting the output first
(`hollow-slot --remote sh -c 'rm -rf dist && make'`, which deletes it on the
host) or touching it last (`sh -c 'make && touch dist'`; a changed folder
counts). A fetched path that is a symlink on the host,
or a folder holding a link that is absolute or has a `..` part (which could
point out of it; relative links without `..` can't), is refused with a line
saying so. A path inside another fetched path is dropped from the list
(copied back after its file, the folder replaced it and the file was lost),
and so is a second copy of one; since 2026-11-03 `--fetch` and `--send`
paths are first written one way (repeated slashes squeezed, a leading `./`
and a trailing `/` dropped), since the list was compared as typed and
`a//b/c` beside `a/b` stayed in it. The old copy here is moved aside and the
new one moved in with INT, TERM and HUP ignored across the swap (a stop takes
effect right after it, and still deletes the old copy): before, a stop
between the `rm -rf` and the `mv` lost both copies. The old copy is deleted
after the swap, not inside it, so signals are ignored only for the two
renames (Stop still signals the whole group, so a big old copy can be left
half deleted beside the path, and the next run sends it as a change); a name already taken beside the path
(an earlier run's old copy) is skipped for the next one, never emptied. The old copy waits
beside the path as `.<name>.hollow-slot-old.<pid>.<n>` (2026-11-03; it was
in the call's temp folder, another volume when `TMPDIR` is, where moving it
aside was a whole copy), so both moves are renames; when the move in fails,
whatever part of the new copy it left is deleted and the old copy put back,
and if even that fails the call says where the old copy is. And when the tree was clean a gate record the command wrote
for this exact commit comes back to this checkout's `hollow-gate-records/`,
so `build.install_ready` and merge tooling see a gate run on the host. A
gate's run record (`hollow-gate-runs/`, failed runs too, only those naming
this worktree's folder as their `checkout`, since the host's worktrees share
one repository) comes back whatever the tree, so `scripts/gate-stats.mjs`
here counts gates run there. The results are collected after every run but
one whose connection dropped (ssh exit 255; ssh can't tell its own failure
from a command that exits 255, so such a command's results stay on the
host), and a pack whose top folder is a
symlink is refused whole (the host runs branch code). The
command gets `HOLLOW_REMOTE_HEAD` (this checkout's HEAD, not the snapshot
commit only the host has) and `HOLLOW_REMOTE_DIRTY`, which `gate-run.mjs`
records as the run's `commit` and `dirty` (with `host` and `remote: true`;
every run now names its `host`, and gate-stats prints runs by machine).
**Falling back:** when the host can't be reached (ssh exit 255), its
`hollow-run-preflight` (on its PATH or in `~/.local/bin`; without one the call
says so) says no, the folder isn't in a git
checkout, `git status` fails here (2026-11-03: it read as a clean tree, so
HEAD went alone and the uncommitted changes were left out), or the push
fails, the call says why and runs the command here, in
this Mac's slots; nothing ran there. When ssh itself failed it also says what
to check for that error (`r_why`, overnight N2, 2026-11-02: the FileVault
unlock screen after a restart, Tailscale when the name doesn't resolve,
Remote Login when refused, the key, the host key, a login lockout), as a
turn's refusal does. A Codex turn in plan, default or accept edits runs
in Codex's `workspace-write` sandbox, which has no network
(`CODEX_SANDBOX_NETWORK_DISABLED=1`), so its calls run here: the call says
so at once instead of waiting out ssh's 10 s timeout. Since 2026-11-12 a Codex
**auto** turn turns the sandbox's network on
(`sandbox_workspace_write.network_access`, with the chat's worktree folders as
`writable_roots`, [PER_AGENT_PERMISSIONS](PER_AGENT_PERMISSIONS.md)), so it
reaches the runner, git, `gh` and package registries; bypass has no sandbox.
**Memory floor (2026-11-01):** a run on this Mac, a fallback included, starts
no build or dev server while less than `HOLLOW_SLOT_MIN_FREE_PCT` (30) percent
of memory is free (`kern.memorystatus_level` by `/usr/sbin/sysctl`'s full
path, else `memory_pressure -Q`'s "free percentage", which Codex's sandbox
allows where it refuses that sysctl). The wait runs inside the lock's own
shell, once the call holds its slot or the dev-server lock: time queued
behind both slots never counts, and a waiter holding a slot starts nothing
that uses memory. It says so once per wait, and after more than
`HOLLOW_SLOT_MEMORY_WAIT` (300) s under the floor refuses with exit 69 (75
means a lock is taken), freeing the slot. That is the toolchain guard's floor,
which before only Claude's hook applied (and never to `hollow-slot` itself),
so Codex and every other agent get it too. A build call inside a slot's own
command doesn't wait (its caller did; a `--dev-server` call always does); on
the runner the same
floor applies to the runner's own memory. `0` turns it off: like
`HOLLOW_SLOT_CARGO`, it is cooperative, not a boundary.
**Stopping:** ssh's stdin is a pipe whose
only writer is the calling process, and on the host a watchdog kills the
command's process group when that stdin ends: a Stop, a TERM (the call says
so and exits 143) or a `kill -9` of the call ends the remote command within
a moment once it runs, and a call stopped while it waits for its folder or
while the folder is made ready (checkout, build folder clone, `npm ci`) is
dropped before its command starts (the host checks stdin once it has the
folder and again right before the command, each with its own message; the
watchdog waits for the command's process group to exist, so a stop while
the command is still starting gets the TERM at once, not the KILL 10 s
later; tests cover both). A
signal sent to the call alone while it sets up (the connection check, the
push, the collect) takes effect when that step ends: the shell runs a trap
only between commands, so at most the 10 s connect timeout or a first push. **Disk:** each run first drops worktree folders past the newest
`HOLLOW_REMOTE_KEEP` (8) or unused for `HOLLOW_REMOTE_DAYS` (14), with their
build and temp folders, and build folders whose worktree is gone (never a
remote session's `targets/session-*`), each only while no run holds it
(moved aside, deleted in the background); then, while the host's volume has
less than `HOLLOW_REMOTE_MIN_FREE_GB` (40) free, the least recently used
worktree folders unused for an hour, never the call's own, each said on
stderr; a preflight that refuses gets one prune and is asked again. A
cloned build folder's build-script text outputs are pointed at itself at
every run, so pruning the folder it came from no longer breaks its builds
(docs/REMOTE_SESSIONS.md "Disk on the runner"). When a runner
is set up, a plain call decides where it runs (Smart routing, below) and its
first line says where and why, which is how an agent in any chat learns about
the runner; the capability index line is unchanged (until 2026-11-03 the "got
a build slot" line named `--remote` instead). Builds whose output you
need here (`tauri:rebuild`, a `dist/` for an install), dev servers and checks
of the installed app stay on this Mac. Tests: `build_slots_remote_tests.rs`
drives all of it against a stand-in `ssh` (`HOLLOW_SLOT_SSH`); every call
there gets 120 s (`CALL_LIMIT`), after which the test fails listing what was
still running and which pipes each process held, and stops it. Plan and
host setup: vault `workspaces/boelab-778c8a32c0c4c034/outputs/mac-mini-plan.md`.

**Smart routing (2026-11-03).** With a runner set up, a plain `hollow-slot
<command>` (or `--auto`) decides where it runs; `--here` keeps it on this Mac,
`--remote`/`--on` send it as before, and `HOLLOW_SLOT_ROUTE` (`auto`,
`runner` or `mac-mini`, `this-mac`) changes the default. Hollow sets it for each turn
from Settings → Devices → Builds and tests (This Mac, the default since
M7, sets `this-mac`; Auto sets `auto`; the runner is set as `mac-mini`,
which older installed copies read, M6), and the line then says "Builds and tests is set to This Mac". Remote runner still keeps a command
that needs this Mac, that changed the checkout, or that needs a tool the
runner lacks here, and skips a runner that just didn't answer. Without a
runner nothing changes and nothing new is said. Not routed: dev servers, a
call from inside a slot's own command (it runs in its caller's slot), and the
runner's own runs (`HOLLOW_SLOT_REMOTE=off` there). The line names the runner
"the runner" (`HOLLOW_SLOT_REMOTE_NAME` renames it; "the Mac mini" until remote runners 2). The rules, in order:

1. **Requirements** (`route_pl`, a perl reader of the command's words,
   including the string after `sh -c`, `${VAR}` read as `$VAR`): database
   variables (`DATABASE_URL`, `DIRECT_URL`, `PG*`, `POSTGRES_*URL*`) in this
   Mac's environment or the command, gcloud credentials or tools, `gh`,
   `git`, `ssh`/`scp`/`rsync`, `docker` and deploy tools (`vercel`,
   `firebase`, `fly`, `wrangler`, `aws`), `security`/`codesign`/`xcodebuild`,
   `osascript`/`open`, package installs (`npm ci`/`install`, bare `yarn`,
   `pip install`, `uv sync`/`pip`/`add`, `poetry`, `bundle`, `brew`),
   publishes, `prisma generate`/`migrate`/`db`, `cargo fmt` (without
   `--check`)/`fix`/`update`, `cargo run`, benchmarks (`cargo bench`, `npm run
   perf`/`bench`), writers (`--fix`, `--write`, `-u`, `prettier -w`, npm
   scripts named like `fix`, `format`, `update`, `generate`, `snapshot`,
   variables named like `*_REGENERATE_*`, `UPDATE_*`, `*_SNAPSHOTS`, not
   `NO_*`), app builds (`tauri build`, `tauri:build`/`bundle`/`rebuild`), dev
   servers (`next dev`/`start`, `vite` or `vite preview`, `npm start`, `pnpm
   dev`, `npm run dev:*`, Storybook and the like), media work without
   `--send` (whisper, `ffmpeg`, `hyperframes render`), Python tests not run
   through `uv run`/`uvx` (they use this Mac's environment), a file written
   with a `>` redirect (not one `--fetch` brings back; quoted text and `[[
   ]]`/`(( ))` tests aren't redirects), an absolute path into the checkout (this Mac's path;
   the runner's copy is elsewhere, so write it relative), and any path under
   home (`~`, `$HOME`), a temp folder (`$TMPDIR`), `/Volumes` or
   `/Applications` outside the checkout. These stay here.
2. **Kind:** heavy (to the runner): `npm run gate*`, `test*`, `typecheck`,
   `lint*`, `build*`, `npm test`, `cargo build/test/check/clippy/nextest/doc`,
   a whole `vitest`/`jest` run (`.`, a folder or a glob counts as whole),
   `playwright test`, `node --test` with several files or a glob, `tsc`, a
   whole-project `eslint`/`next lint`, `next`/`vite`/`turbo build`, `pytest`,
   `swift build/test`, and with `--send` whisper and renders. Quick (stays):
   `vitest`/`jest` or a lint with named files, one `node --test` file,
   `cargo fmt --check`. A heavy kind anywhere in the command wins over a
   quick one.
3. **History:** `route-history` in the slot folder (time, repository id,
   key, where, seconds, exit code, changed-the-checkout), a line per routed
   run; the key is the words that gave the kind (`cargo test`, `run gate`,
   `vitest +files`), else the command's first three words. The usual time is
   the median of the last 9 runs that passed (one that didn't pass never
   counts). A heavy kind under `HOLLOW_SLOT_QUICK_SECS` (5) stays here; a
   quick one at `HOLLOW_SLOT_LONG_SECS` (10) or more goes there; one of no
   known kind goes only after two passing runs at that time or more (its
   words can't show what it needs here). One whose run here or there changed
   the checkout stays here while its history is kept, whatever its kind: a
   run here of a kind that isn't heavy is checked by hashing the checkout's
   files before and after (a tree in a private object folder, so a rewrite
   of an already changed file counts), a run there by the host's list. One
   of no known kind that failed there and passes here stays here, and for a
   day (`HOLLOW_SLOT_START_FAIL_SECS`) so does any command that couldn't
   start there (exit 126 or 127; `--remote` tries it there again). A host's
   own failure before the command (checkout, `npm ci`, temp folder, port)
   exits 247 with no results pack, and the call runs the command here,
   since nothing ran there; a command's own 247 leaves its pack and reads as
   its exit code. Known limits: the "before" hash of a run here is taken
   before it waits for a slot, so an edit made while it waits counts as the
   command's; a `TMPDIR` inside the checkout that git doesn't ignore makes
   every run look like a writer; a `npm ci` that keeps failing there is
   tried again on each call; the host's own memory floor (exit 69 after 5
   minutes) reads as the command failing there.
4. **Pressure:** a command that stays here goes to the runner when this Mac
   is under `HOLLOW_SLOT_MIN_FREE_PCT`, both of its slots are taken, or it
   is on battery (`pmset -g batt`), if its kind is known or it has two
   passing runs and never changed the checkout. A runner that didn't answer
   (ssh exit 255) is noted in `runner-down` for `HOLLOW_SLOT_DOWN_SECS`
   (300), and automatic runs stay here meanwhile instead of each waiting out
   ssh's 10 s timeout; `--remote` still tries, and any answer clears it.

**Two Rust lanes on the runner (R0, 2026-11-14).** This Mac runs one Rust
build at a time; the Mac mini (10 cores, 32 GB) runs two. Its runs
(`remote_host_run`) have `HOLLOW_SLOT_RUST_LANES=2` (the runner's own login
can set `1`), so a Rust build there takes a slot and then lane 0
(`cargo.lock`) or, when that is taken, lane 1 (`cargo-1.lock`), each in the
same one-try, no-wait way, so no wait holds a lock and no cycle can form.
Each lane's cargo gets `CARGO_BUILD_JOBS=5` whenever two lanes are enabled
(`HOLLOW_SLOT_LANE_JOBS=1`; `0`
in the runner's login leaves the variable alone). The build folder is the
checkout's own (`~/hollow-remote/targets/<worktree id>`; one checkout's runs
take its folder lock one at a time), so two lanes never share a target. A
memory watch makes lane 1 unavailable while `kern.memorystatus_level` (the
toolchain guard's number; `HOLLOW_SLOT_MIN_FREE_PCT`, default 30, `0` turns
it off; a reading that fails counts as low) is under the floor: the second
gate waits for lane 0, says "one at a time while this runner's memory is
under its floor", and the lane left keeps its 5 jobs (low memory lowers the
work, never raises it, so 8 jobs plus lane 1's 5 never meet on 10 cores; 8
jobs only where lanes aren't enabled). It is read at every try (each
poll), so lane 1 opens again as memory returns. The preparation's report
carries `lanes=<1 or 2>` (`hollow-slot --remote-lanes`) and says `cargo=busy`
only when every usable lane is taken, so an automatic Rust build still goes to
the runner while one lane is free. Tests: `two_rust_lanes_run_two_builds_at_once_and_low_memory_falls_back_to_one`,
`two_gates_run_at_once_on_a_runner_with_a_build_folder_each` and
`a_low_memory_runner_falls_back_to_one_rust_lane`.

**What the runner has (smart routing 2).** Each preparation on the runner
reports its tools and load in one line (`caps rust=1.96.0 node=26.3.1
uv=0.13.5 python3=3.14.6 postgres= playwright=chromium-1243 gcloud=yes
slots=0/2 cargo=free lanes=2`; versions re-read every 10 minutes and kept in its
`~/hollow-remote/caps-tools`, Postgres looked for on `PATH` and in Homebrew's
`postgresql@*` kegs, `HOLLOW_REMOTE_PG_DIRS` overriding), which this side
keeps in the slot folder's `runner-caps`. `route_pl` names what a command
needs there (`rust`, `node`, `playwright`, `uv`, `python3`, `postgres`). A
routed command that needs a tool the report lacks runs here: at once by the
kept report while it is under `HOLLOW_SLOT_CAPS_SECS` (600) old, else by the
report its own preparation just got (before anything is pushed). `--remote`
and `--on` only warn. An automatic run also comes back here when the runner
is busy (both of its slots taken, or every Rust lane it may use now for a Rust build)
while this Mac has a free slot, memory over its floor and mains power. gcloud
is a requirement, not a tool: the runner has a binary but no sign-in.
Since 2026-11-03 the line also names the runner's system (`os=darwin` or
`os=linux`), and `route_pl`'s `macos` need (Apple's tools, MLX, a webkit
browser for Playwright or vitest, a Rust build or gate of a Tauri checkout)
keeps such work here when the runner is Linux ("the runner runs Linux and
gate needs macOS"); see [REMOTE_SESSIONS.md](REMOTE_SESSIONS.md) (Linux runner).

Each decision is a line of `route-log` in the slot folder (time, where, why,
label, repository, what chose it; Settings → Builds and tests shows the
latest six with the runner's last tool report, and agents read the same in
`build.runner_status`'s `routing`), kept to its last 200 to 400 lines, and the
history to its last 1000 to 2000. After a run there, the host lists what the
command changed in its copy of the checkout (`git --no-optional-locks status
-z`, less the `--fetch` paths) and what it wrote that git ignores (less
`node_modules` and fetched paths), and the call says so: changed paths always
("the command changed 2 path(s) in the checkout on runner-host, which don't come
back here (...); to change them here, run it with hollow-slot --here"),
ignored ones for a build, Playwright or a command of no known kind ("it also
wrote dist on runner-host, which stay there (ignored files)"). `--help` prints
the usage and exits 0; an unknown option exits 2 (it used to take a slot and
fail with `exec: --: invalid option`). Tests: `build_slots_remote_tests.rs`
(the routing tests at its end).

**A throwaway database (smart routing 6, 2026-11-03).** `hollow-slot
--postgres <command>` runs the command inside a small wrapper (`pg_wrap`),
wherever routing sends it. It starts a Postgres of its own (`pg_ctl` on
`PATH`, else the highest Homebrew `postgresql@<n>`; `HOLLOW_SLOT_PG_DIRS`
overrides the folders) in a new `/tmp/hs-pg.*` folder (0700; short, for the
socket path's 103-byte limit; `HOLLOW_SLOT_PG_TMP` moves it in this tool's
tests, so one test's sweep of dead-owner folders never meets another's):
`postgres` itself as a direct child (not `pg_ctl start`, which detaches it
into its own session, so a stopped or
killed wrapper left it running), on a free port on 127.0.0.1 only, a random
password over TCP, `fsync` off, with empty databases `hollow_test` (test
setups such as Portal's refuse a name without "test") and
`hollow_test_shadow`. The command gets `DATABASE_URL`, `DIRECT_URL`,
`POSTGRES_URL`, `POSTGRES_PRISMA_URL`, `POSTGRES_URL_NON_POOLING` and `PG*`
pointed at it, `SHADOW_DATABASE_URL` at the shadow, and every other `PG*`
setting cleared before the server starts (`PGSERVICE`, `PGSSLMODE` or
`PGHOSTADDR` would steer the readiness check and `createdb` away from it),
so no real database is reached through those names; and vitest is held to one worker
(`VITEST_MAX_*=1`: the runner's 4-worker cap overrides a config's
`fileParallelism: false`, and parallel files deadlock one database). A
command's own `env NAME=value` still wins inside it. The server is stopped
(INT, then KILL after 10 s) and the folder deleted when the command ends or
is stopped; it shares the call's process group, so a group stop reaches it
even when the wrapper is killed, and the next wrapper sweeps a folder whose
owner is gone (a pid it may not signal, as from a sandbox, counts as alive;
a folder with no owner after a day), stopping a server only when its
arguments name that folder. The server must answer within 60 s; one that
dies moves on to another port, one that never answers fails the call. A
`SIGKILL`ed server can leave a SysV shared-memory segment behind (rare;
`ipcs -m`, `ipcrm`). The command's exit code is kept. No Postgres there: exit 69
and the command doesn't run; one that won't start, or no 127.0.0.1 port
(a sandbox without network): exit 70.

For routing, `--postgres` means the command brings its own database: a
database variable from this Mac's environment no longer keeps it here (one
the command sets itself to a literal value still does: that isn't the
throwaway one), `prisma migrate deploy`/`reset`/`status`/`resolve` and `db
push`/`seed`/`execute` are fine (`migrate dev`/`diff` and `db pull` write
into the checkout and stay here either way), and it needs Postgres where it
goes (the runner's capability report). Its history key gets ` +postgres`. A
step that readies the rest (`npm ci`, a `--frozen-lockfile`/`--immutable`
install, `prisma generate`) doesn't keep a command here when a heavy step
follows it after `&&` or `;`; alone, or before nothing heavy, it does, and an
install that can change the lock file always does. The Mac mini has
Homebrew `postgresql@17` (17.11, installed as `builder` 2026-11-03, no
service); this Mac has `postgresql@16`. Proven 2026-11-03 with Portal main
`095306e8` from a clean clone: `hollow-slot --remote --postgres sh -c 'npx
prisma migrate deploy && npx vitest run --config vitest.integration.config.ts
--no-file-parallelism'` applied 109 migrations and passed 1469 tests (1
skipped) on the mini in 446 s.

**Jobs on the runner (overnight N4, 2026-11-02).** `hollow-slot --remote
--send <path>... [--fetch <path>]... <command>` (or `--on <host>`; with a
runner set up, `--send` alone also asks for it unless Builds and tests is
This Mac, but with none it needs `--remote`, which then runs it here) runs work
that needs no checkout, such as a transcription or a video render, from any
folder: the `--send` paths (files or folders under the current folder) go by
`tar` over ssh (`./`-prefixed; symlinks named on the command line followed)
into a new 0700 folder of the job's own on the host
(`~/hollow-remote/jobs/job-<pid>-<time>`, `--job-receive`; the host's `tar`
refuses absolute paths, `..` and writing through a symlink), and the command
runs there with the same relative paths. A `--send` path that isn't here (a
dangling symlink included) or lies behind a symlinked folder, a `--fetch` path
that is or lies behind a symlink here, and a `--fetch` folder that exists here
but wasn't sent whole (it would come back holding only what was sent, and
replace this one) stop the call with exit 2 rather than run a long job here.
A folder both sent and fetched comes back as the job left it, so changes made
to it here during a long job are replaced. One job
at a time on the host (`jobs/jobs.lock`; a second says it waits), in one of
its slots, with a short `TMPDIR` (`hj-<6 hex>`), the checkout runs' caps,
`PRODUCER_MAX_WORKERS` at most 2 (a `--workers` flag in the command still
wins) and, unless the command sets them, HyperFrames' pins (no update check,
self-install, skill refresh or telemetry). The `--fetch` paths come back over
this folder's like a checkout run's (no gate records), then the job's folder
and temp folder are deleted. Stop during the upload, or before the run starts
there, ends the call at once (the upload's two ends run in the background)
and asks the host to drop what arrived (`--job-drop`, best effort): it leaves
a `jobs/<job>.drop` tombstone, which a receive still unpacking (it holds
`jobs/<job>.run` meanwhile; `tar` accepts a stream cut between files) and a
later `--job-run` both obey, and deletes the folder unless a receive or a
started run holds that claim. A run
stopped on the host packs nothing (nobody is left to collect it; checkout runs
too, since this change), and a folder a lost connection left behind goes with
a prune after a day while no job runs. A fetched tree holding a `.git` is
never copied back (it would be a repository here whose config git obeys;
checkout runs too). Falling back to this Mac (where the files already are)
and the setup-time line work as for a checkout run; no git push is involved.
Examples (the tools come from the host's own login: `uv`, `ffmpeg`, `node` on
the Mac mini; first runs download into `runner`'s caches): `hollow-slot
--remote --send talk.m4a --fetch talk.json uvx --from mlx-whisper mlx_whisper
talk.m4a --model mlx-community/whisper-large-v3-turbo --output-format json
--output-dir . --word-timestamps True`, and `hollow-slot --remote --send video
--fetch video/renders/out.mp4 npx --yes hyperframes@0.8.169 render video
--output video/renders/out.mp4 --workers 2`. On the Mac mini (11-02) a 3 s
clip came back transcribed in 43 s including the first downloads (2.3 GB of
`uv` cache, 1.5 GB of model; 5 s once warm), and a 30 s 1080p HyperFrames
project rendered in 23.0 s on its GPU, 30 s end to end.

**Remote sessions (2026-11-01, the Mac mini runner, Phase 2).** A build
workspace's Claude and Codex turns can run on the same runner: Runs on in
Settings → Models (the default), in the workspace's Changes tab and
in a new conversation's empty state (Default, This Mac, Mac mini; stored in
`build-workspaces.json` beside the worktree). Only the agent process moves;
its tools run here through a relay, and its work comes back to the worktree
as commits and uncommitted changes when the turn ends. Commits made there
carry the git identity this Mac uses for the worktree (2026-11-02; the
runner account has none, so they were `Hollow runner <runner@…>` before),
sent in the run's header, never set on the runner; with none here, the start
line says so. Mechanics, the
security model and what the person sees when the runner doesn't answer:
[REMOTE_SESSIONS.md](REMOTE_SESSIONS.md).

**Ports for every turn (F6 part A, 2026-10-20).** Every chat turn that gets the
caps, and a build workspace's vault-folder conversation in every mode, also
leases a `PORT` / `PW_PORT` of its own for the length of the turn
(`build_slots::lease_turn_port`, held by `chat::execute_turn`), from 1900-2299:
clear of the build workspaces' 1500-1899, 1499 and Vite's 1420. It starts from a
port hashed from the conversation's session key (so a conversation usually gets
the same one each turn), skips any port another live turn holds or something on
this Mac already listens on (IPv4 or IPv6, loopback or every address), and is
freed when the turn ends. If all 400 are taken the turn gets no Hollow-assigned
port, never a worktree's, rather than failing: a `PORT` / `PW_PORT` inherited
from Hollow's own environment stays, otherwise the defaults apply. Before this,
several conversations in one workspace folder (allowed since 2026-10-07) all
fell back to 1499 and had no `PW_PORT`, so two Playwright runs in the two slots
fought over one port; and a conversation in a build workspace's vault folder got
its worktree's port. A build worktree's own turn keeps its stable port. These
turns get no `CARGO_TARGET_DIR` of their own: a workspace folder holds no code,
and one per conversation would be a cold multi-gigabyte build each time; a build
workspace's vault folder keeps sharing its worktree's warm one (cargo's lock
serializes two builds in it).

**Several worktrees (F6 part B, 2026-11-18).** A build workspace can bind extra
worktrees of its repository, each a named lane with its own branch, `CARGO_TARGET_DIR`
and stored port (`workspaces.add_build_worktree`; no UI yet). Once it has one, a turn in
a worktree holds that worktree's lock alone, so two worktrees run at once, one turn each,
beside the folder's own conversations (which are then given no worktree or target);
History treats them all as shared runs of `workspaces/<id>`. With no lane a worktree turn
holds the folder alone, as before. Each turn's note names its own worktree. Commit,
publish, update, install and the gardener stay on the primary and refuse a lane's turn. Design and what was built:
[BUILD_WORKSPACES_MULTI.md](BUILD_WORKSPACES_MULTI.md).

**Changes** (Build PR B2). While the worktree exists, the top bar's branch
chip and the palette's Open Changes open a Changes tab in the Preview pane
(`WorkspacePreview.tsx` renders `BuildChanges.tsx` over `build_changes.rs`;
a workspace panel section until COO rebuild L4). Its header shows the
branch, the base, ahead/behind, whether it is pushed, and the PR (from
`gh pr view <branch>`, when `gh` is installed and signed in). The file list is
the worktree against the merge-base of base and HEAD: committed work, uncommitted
tracked changes, and untracked files (counted as additions; binaries show
"binary"). A dot marks files not yet committed. Clicking a file loads its diff
(`build_diff`, redacted, 256 KB per file, 1 MB in all), parsed by
`src/diffParse.ts`. **Ask about this** on a hunk appends the file, line range
and the hunk in a ```` ```diff ```` fence to the composer draft and focuses it;
nothing is sent. **Commit** (`git add -A` then `git commit -m`), **Push**
(`git push -u origin <branch>`) and **Open PR…** (title = workspace title, body
= its Outcome; `gh pr create --base <base without origin/> --head <branch>`,
pushing first when the branch has no upstream) each confirm first, run through
`trackTask`, and are refused (in the backend too) while any Hollow agent run
holds the vault. The pane refreshes itself when a chat turn in this worktree
finishes. Reads use `--no-optional-locks`, so a refresh never takes
`index.lock` from an agent using git there. Open PR… shows only with no PR or
a closed one (an open or merged PR is the branch's). When the branch is behind
its base, **Update from <base>…** (P1-46, confirmed) runs `git fetch origin
<branch>` for an `origin/` base, then `git rebase <base>`, or `git merge
--no-edit <base>` once the branch is pushed (its commits aren't rewritten, so
Push stays a fast-forward); a failed or timed-out rebase or merge is aborted,
so the branch is left as it was. It is refused while an agent works in
the worktree or with uncommitted changes, and is the person's only (agents
rebase with git in their own turn). IPC: `build_status(id)` (with
`installedContainsHead`: whether the last successful install's commit contains
HEAD), `build_diff(id, path?)`, `build_commit(id, message)`, `build_push(id)`,
`build_open_pr(id, title, body)`, `build_update_from_base(id)`.

**Delivery record** (next-level P20; P1-46). The top of Changes lists local →
committed → PR → merged → built → install scheduled → installed
(`src/delivery.ts`), each with its evidence. The app build is one signal in
`BuildChanges`, fed by the Hollow app section's reads and `self-install-event`,
so the record and that section never disagree. Committed is done once the PR is
merged even with nothing ahead; Installed is done when the running app's receipt
is this head or, for a branch with commits of its own or a merged PR, contains it
(a squash-merged branch's head is never contained, so it stays pending). `self_install_receipt` errors when
`last-result.json` exists but can't be read, which the record shows as unknown
with Retry (never "No install recorded"). A running build shows "Building · N
min (stops at 45 min)", and Build app's confirm says how far behind the branch
is.

**Install from Hollow** (Build PR B5). When the worktree is Hollow's own
repository (`src-tauri/tauri.conf.json` with identifier `com.hollow.app`), the
Changes tab ends with a Hollow app section. **Build app** (confirmed) starts one
background job app-wide (`self_install.rs`): `xattr -cr src-tauri/models` when
present, `npm ci` when `node_modules` is missing, then `npm run tauri:bundle`,
with `CARGO_TARGET_DIR=~/Library/Caches/hollow-install-target-<workspace>`
(deleted when the workspace's worktree is removed or the workspace trashed, unless it built the installed
app or the rollback copy or a build still holds it; night ST7),
`CARGO_BUILD_JOBS=8`, a 45-minute deadline and a Stop button. It refuses while
another app build runs or memory is short (free < 30 % or macOS pressure at
warning). Progress arrives as `self-install-event` (and a 5 s poll), with the
newest log lines (redacted). Since row U10 there is no install card here:
the finished build waits in the top bar's **Update button** as "Local build
of <branch>" (commit, version and what it brings) and installs from there with
Restart to update or Update when chats finish, through the in-app installer
`hollow install-update`; the bundle is re-hashed against the receipt recorded
when the build finished, when it is offered and before the installer starts
(slice 6, review A·S4/B·F12). The section says whether the build is in the
button and offers **Show in Update button** after Not now. After the restart
the Update button's notice says "Updated to the local build of <branch>" (or
why not), and `last-result.json` records it already acknowledged
([UPDATES.md](UPDATES.md), "Local builds (U10)"). IPC: `app_build_status(id)`,
`start_app_build(id)`, `cancel_app_build(id)`, `update_offer_local_build(id)`,
`self_install_result()`, `ack_self_install_result()`.

**Services that outlive a turn** (Build PR B6). Hollow ends every process a
turn started when the turn finishes, dev servers included. A chat in a build
worktree (Claude or Codex, not plan mode) therefore gets `hollow_services`
(`services_mcp.rs`, which only forwards to the GUI's broker in `services.rs`):
`start_service {name, command, cwd?, port?, kind? (dev-server|other)}`,
`list_services`, `stop_service {name}`, `service_logs {name, lines?}`. The GUI
starts the command itself (`/bin/sh`, its own process group, stdout and stderr
to `~/Library/Logs/Hollow/services/…/<name>.log`, no `HOLLOW_*` variables,
the system `TMPDIR`, a PATH with the usual tool folders), so it belongs to the
worktree, not the turn. At most 4 run at once and one `dev-server` (this Mac
allows one dev server at a time). The Changes tab lists them (Open :port, Log,
Stop, Remove for an exited one); `services-changed` refreshes it. They stop
when the worktree is removed and when Hollow quits; a service a crashed Hollow
left behind is stopped at the next launch only when its pid is alive, leads
its group and has the recorded start time. The tools are not preapproved, so
they run only in `bypassPermissions` (headless Claude denies them in
`acceptEdits`; Codex fails them). IPC: `list_services(cwd?)`,
`stop_service(cwd, name)`, `service_log(cwd, name, lines?)`.

## Pinned workspaces

Like pinned chats in the ChatGPT Mac app (2026-10-21). Pinned workspaces lead
the sidebar's **Projects**, in pin order, each with its pin, and are listed
only there (a Pinned group of their own until overnight 11-04 S1). The
collapsed sidebar uses the same order. Home's recent projects use last
activity instead.

- **Where it is stored:** `pinned: <rank>` in the brief's frontmatter (lower
  first, ties by id; `src-tauri/src/workspace_pins.rs`), so it survives restarts
  and goes with the vault. Every write is `shared_docs::update_doc`
  compare-and-swap, keeping the mtime unless a chat runs there; a move rewrites
  only the briefs whose rank changes. `Workspace.pinned` carries the rank;
  `src/workspacePins.ts` orders it and `src/store/workspacePins.ts` writes it.
- **Click paths:** the workspace's right-click menu (a sidebar project row's
  only menu since 2026-11-07, when its ⋯ went) and every **⋯** that opens
  it (the project's ⋯ on Work and in the top bar, and Home's project rows) offer
  **Pin**, or **Unpin** with **Move up** / **Move down** (the keyboard way to
  reorder). A pinned row's pin is a quiet mark (until 2026-11-06 it turned
  into **Unpin** on hover, then Unpin sat in the row's ⋯ until 2026-11-07);
  dragging a pinned
  header onto another places it above or below. Each change is announced in the
  snackbar's live region. No keyboard shortcut.
- **Rules:** built-in workspaces and archived ones can't be pinned; archiving
  (`update_meta`, the gardener, a merge) removes
  the pin in the same write, and
  a stale `pinned` left in an archived brief is ignored. Legacy `notion-*`
  workspaces and connected folders can be pinned. Hide from sidebar still hides a pinned workspace from the rail.
- **Agents:** `hollow_app` `update_workspace` takes `pinned` (true pins at the
  end, false unpins) and `pinPosition` (pin or move to that place, 1 = top);
  `list_workspaces` returns `pinned`.
- **Before:** pins were a window-local `hollow.studio.pins` list that only Home
  wrote. On launch, once the workspace list has loaded, the sidebar adopts
  such a list into the briefs, in order (gone and archived ids skipped), and
  deletes it once every pin was written.

## Plain-file contract

The configured local vault path remains the storage root; no database migration,
folder rename, or bulk conversion of existing notes is required.

```text
<vault>/
  library/                         reusable reference notes
  workspaces/<slug>-<random-id>/
    WORKSPACE.md                   outcome, current facts, next step, decisions
    PLAN.md                        local unfinished work and checks
    NOTION.md                      legacy Notion project context (history; still read)
    SOURCES.md                     original paths, capture times, SHA-256 hashes
    AGENTS.md                      workspace operating instructions
    sources/                       explicit, read-only-in-UI source snapshots
    outputs/                       agent-produced deliverables
    sessions/<hollow-key>.md        bounded, generated session records
  <existing folders>/              retained and accessible through Library
```

`WORKSPACE.md` frontmatter: `hollow_workspace: 1`, `title` (quoted YAML string),
`status` (initially `active`), `created` (RFC3339). Optional `canvas_theme`
names a custom theme for that workspace's live canvases (workspace menu →
Canvas look…); missing themes fall back to the app. Its local setter preserves
body and other frontmatter, including legacy Notion keys, and keeps the mtime unless
a chat runs there (see [Shared docs](#shared-docs-while-chats-run-in-parallel)). Also optional: a hand-edited
scalar `project_url` (`vault:projects/…md`) naming the workspace's project, and
a legacy `notion_project_path` (written by the removed Notion sync for `notion-*`
workspaces; still read first).
Either (or a unique exact title match) lets the context carry the project's
`goalAlignment` block; see [SOURCE_CONTEXT.md](SOURCE_CONTEXT.md#context-and-provenance-contract).
Folder IDs are path-safe and unique. A workspace is recognized by its brief marker, not by an inferred
project type. Missing/malformed briefs cannot be opened as workspaces; repair
such files through Library. Brief mtime determines overview ordering.

### Shared docs while chats run in parallel

Every conversation in a workspace's own folder shares `WORKSPACE.md`, `PLAN.md`
and `SOURCES.md` (`shared_docs.rs`, concurrency PR 2, 2026-10-20). Agents edit
them with their own tools, which take no Hollow lock: Claude Code's Write
replaces the whole file and refuses only when the mtime moved since the agent
read it, so a Write from an earlier copy silently undoes whatever landed since.

- **Hollow's own writes** (rename, status, canvas look, department, the
  gardener's archive) go through `shared_docs::update_doc`: read, transform,
  `write_note_expecting` on the revision, up to 4 tries, then a plain refusal
  ("… kept changing …; nothing was changed"). They keep the mtime (overview
  order) only while no scoped run is live in the workspace; while one is, the
  mtime moves so the agent's stale-read check sees the write. Compaction
  (refuses while a chat runs, revision CAS) and the Plan tick (CAS) were already safe and are unchanged. Attach/refresh append to
  `SOURCES.md` stays content-checked.
- **The `<hollow-workspace-shared>` note** goes with every turn in a
  workspace's own folder (a shared scope; since F6 part B a build worktree's
  turn too once the workspace has lanes, with its log's absolute path) while `shared_docs::workspace_allows_parallel` holds, a peer live or
  not. It holds while Settings → Chat → **Chats at once in one
  workspace** is above 1 (concurrency PR 4; app-wide, so every workspace
  alike); at 1 the pump starts one turn per folder and no note is sent.
  Since V5b (2026-11-15) the note also names the chat's own log, `log/<session key>.md` in the
  workspace folder (`shared_docs::chat_log_path`; one writer per file, so no clash), and the shared
  docs take only changes the whole workspace needs. `log/` is a plain folder: not a deliverable (only
  `outputs/` is), not protected, not in the context envelope, not capped; the safety net below stays
  the fallback for chats that still write the shared docs. The file list shows it as `context`.
- **The safety net** (`shared_docs::run_finishing`, called from the public
  `automated_history::finish_run` while the run is still live). Hollow keeps,
  in memory, each shared doc's text at every scoped run's start, each finished
  run's before/after while another run there is live, and each of its own
  writes' before/after, within budgets (256 KB a side, 64 versions a doc, 4 MB
  a workspace, 16 MB in all; past one, that doc stops being journaled, with a
  log line, until the runs live then have finished). At a run's finish, a
  version published during its life whose added lines are all gone from the
  file now is acted on (at most 8 per finish):
  - **Hollow's own write** is merged back (line-level diff3: base = its
    before, ours = now, theirs = its after; both sides only inserting at one
    place keeps both, ours first, with a line break between) and written
    compare-and-swap, so the merge is in the run's own History change.
  - **Another run's version** is never written back into the file (the run
    may have removed those lines on purpose): it is kept as
    `<name> (conflicted copy <stamp> <random id>).md` beside it, created
    exclusively, which Waiting on you offers. A later lost version that holds
    an earlier one's lines covers it (one copy). A Hollow write that won't
    merge is kept the same way.

  Either way the run gets a `sharedDoc` flag whose detail says what happened
  (merged, copied, or the copy failed): History's run view shows it apart from
  protected changes, and the notification says "A shared workspace file was
  written over". Limits (by design): a version only some of whose lines are
  gone counts as a deliberate edit; mid-turn edits aren't observed, so a
  still-running run's line lost to a peer is caught only if the run that lost
  it finished first; a deliberate removal of everything another run added
  still gets a copy and a flag; a Hollow value put back exactly on purpose is
  merged back (flagged); pure deletions aren't detected; the journal is per
  process.
- **The compare-and-swap write itself** (`vault::write_note_expecting`, every
  caller, not only shared docs): the mutate lock keeps Hollow's writers out,
  but agents and Obsidian take no Hollow lock. On macOS the replace is an
  atomic swap (`renamex_np` `RENAME_SWAP`); if the displaced file isn't the
  bytes the check saw (or can't be read), an outside write landed in the
  window, so it is swapped back and the call returns a conflict; `update_doc`
  then reads again. A displaced file is unlinked only when verified as the
  checked bytes or as Hollow's own; anything else is renamed to a conflict
  copy (same inode and mode, folder synced before and after). Hollow
  recognises its own file by inode and by its bytes read through the handle
  it wrote, so a file that inherited a mode it can't open still counts. A
  conflict whose current file can't be read (or isn't UTF-8) is still a
  `Conflict`: empty `content` and a `revision` starting `unreadable:` with the
  error, which no real revision matches. The file editor treats it as its own
  state (`fileSave.ts` `unreadable` outcome): the draft is kept, the panel says
  Hollow can't read the file on disk, and only **Read the file again**
  succeeding leads on to the usual comparison (adopt or merge), never an
  empty "file on disk". Copies Hollow
  writes from text keep the original's mode (0600 when unknown). Only "can't
  swap" errors (ENOTSUP/EINVAL, another OS) fall back to a plain rename with
  that window unverified; other swap errors fail the write. The guarantee is
  narrow on purpose: a replace-style write (Claude Code's Write, Obsidian) or
  an in-place write completed in the window is caught; an in-place writer
  that opened the old file before the swap and writes after the check lands
  in the displaced inode and is not.

See [Sources and workspace context](SOURCE_CONTEXT.md) for the current source
catalog, import destinations, coverage and provenance schema, automation scope,
and whole-app surface disposition. Context is deliberately small. Before each managed-workspace turn, the backend
reads WORKSPACE, PLAN, SOURCES, AGENTS, and optional NOTION, with a **48 KB combined limit**, plus up to **24 KB attached text evidence**
(8 KB/file, 100 source files listed; 128 KB encoded envelope cap). These
limits trim; they never fail a turn, the session record or the workspace page
(see [SOURCE_CONTEXT.md](SOURCE_CONTEXT.md#context-and-provenance-contract)).
Detailed evidence belongs in source files, and completed work belongs in outputs.
In addition, each request retrieves up to eight relevant passages from connected
projects and supported local vault sources, with original paths and coverage; see
[the retrieval contract](DIRECTORIES.md#library-workspaces-and-agent-context).
The global operator profile, page-context payload, and vault-wide learned recall
are omitted from these turns. Sources and previous sessions are evidence, not
instructions. The provider's other installed tools remain available: cwd/context
scoping is **not** an OS sandbox or a new permission mode.

An attachment is an explicit snapshot, not a move or sync. A source can be a
file chosen in the native picker or an existing Library note. Limit: 20 MB per
file. Duplicate basenames are refused, never overwritten. The original stays
untouched; SOURCES records its canonical origin, timestamp, and checksum. The
source pane is read-only, though external tools/agents still have filesystem
access. Snapshot immutability is a workflow convention, not filesystem ACLs. New
attachments also write a hidden JSON provenance sidecar; Sources reports
coverage and original size/time freshness. Older copies remain valid with
unknown origin freshness. Library can attach files directly to a chosen workspace.

Hollow's durable SQLite conversation store remains authoritative for full turns,
provider/native IDs, tools and evidence. `sessions/` contains the most recent
12 user/assistant records, each capped at 1,800 characters, written after a turn.
These are portable summaries, not full transcripts or proof of delivery. A
session file with user-authored replacement content is preserved; writes use
compare-and-swap. Session keys and workspace cwd join exactly, never by title.

## Interaction and recovery

- Workspaces put the conversation first; WORKSPACE.md, PLAN.md, SOURCES.md
  and AGENTS.md open as files in the Preview pane (Project files). New work starts with a desired result. Creating one does not
  send a prompt or incur an agent call.
- Reopening loads the latest workspace conversation, with exact native resume
  when supported. The conversation list opens a specific session or starts another one.
  Provider changes reuse the existing bounded conversation-handoff behavior.
- Conversations run in parallel (build PR B4): opening another conversation
  or workspace while one is working leaves it running in the background, its
  rail row says "working" (or "needs you" when it waits for an approval), and
  opening it again shows everything it did meanwhile. Every workspace's
  turns run side by side, build workspaces and ordinary ones alike: since
  2026-10-20 up to Settings → Chat → **Chats running at once** (default
  10, at most 60 since 2026-11-16, counting automations and swarm parts too)
  across Hollow, and up to **Chats at once in one workspace** (default 5, at
  most 25 since 2026-11-16) in one workspace's own folder, and up to
  **Subagents per chat** (default 6, at most 20) of one chat's subagents at
  once; a build worktree still runs one turn at a time. Only conversations
  at the vault root (no workspace) take turns on the vault lease. While macOS
  memory pressure is warning or worse, the first three turns may start;
  additional turns recheck every second, subject to all other limits. SE10
  replaces the saved overall count when memory is measurable: free plus inactive
  memory covers a 2 GB floor and live/next turn reservations, up to 60 turns.
  Unknown samples use the saved overall count; workspace/subagent limits stay.
  **Working, not
  Queued (2026-11-16):** a chat reads Working, with "Preparing" for the part
  before the agent runs, from the moment the queue has its row: admission,
  recall, retrieval, prompt assembly and the agent's boot all count as
  working. A queue item carries `working: true` for a claimed or running row
  and for a queued chat row nothing truly holds back. Known tracker syncs
  commit mirrors atomically per file and no longer hold scoped admission.
  Internal bookkeeping holds still read Working (`run_pump::waiting`,
  `is_internal_holder_text`; that holder is not named on the item). Every
  surface (chat badge and working line, sidebar, top bar dock, floating card,
  Subagents rows, `sessions.overview`, `start_session`/`send_to_session`)
  reads it. Only a turn that truly can't start reads Queued, and the badge
  names why. A turn over a
  limit reads "Queued · waiting for …" (memory, "a free chat slot: Hollow
  is at its limit of 10 chats at once (Settings → Chat → Chats at once)", the same for this
  workspace, "a free subagent slot: this chat is at its limit of 6 subagents at once (…)", …; the
  numbers are the person's Settings, 10, 5 and 6 by default). Behind a build worktree's turn it names that
  conversation and how long it has held the worktree, with **Open it**
  (`run_pump::folder_holder`; the queue item's `waitingSession`, P1-46).
- Stop ends the turn's whole process tree: the agent, its tool shells, and the
  processes they started, including ones that moved into their own process
  group. Each gets SIGTERM, then SIGKILL after 3 seconds, before the run
  releases the vault. The transcript shows a neutral "Stopped." notice, and the
  receipt says whether Hollow verified that nothing it could find remains
  (macOS hides some system tools' environments, so a detached `/bin/sleep`-style
  job can still escape). When a turn or automation finishes normally, Hollow
  ends the background processes it started too, so long-running servers such as
  `npm run dev` must be started outside Hollow's agent runs. A Stop pressed in
  the first moments of a turn, while it is still gathering workspace context
  and before it holds the vault, is not remembered; press it again once the turn
  is running.
- Composer text is stored per conversation, with a separate New chat draft
  per workspace. Retention is 100 drafts / one million characters; oversized
  current drafts stay whole in memory with an unsaved notice. Image drafts
  remain in memory by conversation and are not promised across restart.
- Markdown editing is explicit: Edit → Save. Drafts survive navigation and
  reload through sessionStorage. A conflicting external edit blocks the save
  and preserves the draft. There is no force-overwrite button; copy the draft
  before reconciling with the original in an external editor.
- Text previews are capped at 1 MB; larger/binary files use Show file. HTML
  previews use the existing sandbox. Library folds evidence/archive folders and
  previews 12 rows per folder (Show all expands one). Shared material excludes workspace/session/calendar/
  recording folders by default; All files exposes retained supported notes.
- Existing project folders become workspaces automatically through the Hollow
  folder's `Projects/` (the manual “Bring in existing work” panel was removed
  2026-09-26); tracked projects come from the tracker ([Tracker sync](TRACKER_SYNC.md)).
- The app no longer seeds/enables standard routines during startup. Existing
  configured schedules continue unchanged, and routines are edited like any
  other automation. Hollow-folder setup automatically prepares `workspaces/`
  and `library/` in `Vault/`; the rescaffold wizard was removed 2026-09-26.

## Navigation and implementation

The rail is Projects (Work until L1, 11-12; internal route `today`), Tasks,
Library (`vault`), Automations (`workflows`), project rows and Settings.
Inbox and Approvals are removed.
Links to the long-retired Projects page id open the Projects page (`today`). Calendar/People links open the corresponding
Library collection; Graph/Ingest links open Library. Those standalone loaders and
palette entries are removed. Conversation history (old sessions, archived
ones, artifact continuity) is Library → Conversations since S4 (2026-11-03);
Library → History → Conversation history, a project's ⋯ on Work or in the top bar → Open in Conversation history,
`g p` and the `chat` page id open it there
([details](reference/surfaces.md#library--conversations-s4-2026-11-03)). Cmd+1 opens Projects,
Cmd+2 Tasks, Cmd+3 Library, Cmd+4 Automations, Cmd+, Settings (contiguous
since frontier F6). Other digit bindings are unbound. Primary surfaces skip entrance animations.

- `src/pages/Work.tsx`, `Library.tsx`, `components/WorkFileEditor.tsx`: new UI.
- `src/store/workspaces.ts`: guarded session switching and draft ownership;
  `src/store/conversationDrafts.ts` owns bounded per-conversation persistence.
- `src/styles/workbench.css`: shell, split workspace, reader, responsive layout.
- `src-tauri/src/workspaces.rs`: additive creation, bounded context/files,
  snapshot provenance, session records; `commands/workspaces_api.rs` exposes
  `list_workspaces`, `create_workspace(title, goal)`, `read_workspace(id)`,
  `attach_workspace_source(id, source)`, `read_work_file(path)`, and
  `plan_task_candidates(id)` (Paperclip PR 11), plus (2026-09-26, the rail
  tree) `workspace_sessions(pinned?)` (pinned chats listed however old, overnight
  11-04), `update_workspace(id, title?, status?)` and
  `trash_workspace(id)`; `commands/sessions_api.rs` adds
  `rename_agent_session(key, title)`.
- **Pinned workspaces (2026-10-21).** See [Pinned workspaces](#pinned-workspaces).
- **Rail tree and right-click menus (2026-09-26).** The rail lists only
  workspaces with a conversation (`src/railWorkspaces.ts`; F4's every-workspace
  list was reverted 2026-09-29), the ones a conversation was started or opened in first (newest first; ids in
  localStorage `hollow.railWorkspaces`, per machine), each with its unarchived conversations nested beneath
  (`workspaces::sessions_by_workspace`: one session-store read, grouped by the
  canonical cwd, 25 per workspace plus a `total`). `+` and New conversation run
  `selectWorkspace(id, undefined, true)` and focus the composer; the
  conversation joins the list after its first turn. Rename and status edits
  (`update_meta`) patch only `title` / `status` (with a status, also `pinned`
  and `archived_from`) in `WORKSPACE.md` through
  `frontmatter::patch`, compare-and-swap (`shared_docs::update_doc`), keeping
  the file's mtime so list order doesn't jump unless a chat runs there;
  Hide from sidebar (offered only for a workspace in the rail; was "Archive")
  removes it and its conversations from the rail (localStorage
  `hollow.railHidden`) and changes nothing on disk: status, brief and
  conversations stay, and the next conversation started or picked there brings it back
  (P1-38: `pickConversation`, which rail rows, the palette, Open conversation,
  and every open by exact key (`openSessionByKey`:
  Overnight, Review, Runbooks, a canvas's source) use; an open from Work, a launch restore,
  a canvas Refresh or the COO's way back leaves it hidden). Archive…
  (`status: archived`) also leaves the rail (`railWorkspaceOrder` skips
  archived workspaces, even the one open); `update_meta` and a merge record
  the status it had as `archived_from` in the brief (`Workspace.archivedFrom`),
  dropped on any other status and by Undo merge. An archived
  workspace's menu has no Mark as items: its one status item is
  **Unarchive (back to <status>)**, which restores `archived_from` (else
  `active`) and puts it back in the rail. **Mark as done** flashes with Undo
  (the status it had). Delete (`trash`) moves the whole folder to
  the vault's `.trash/{stamp}-workspace-{id}` under the short `workspace-edit-`
  lease, so it is refused while any run holds the vault and never lands in a
  run's diff; conversations stay in Conversation history (Library →
  Conversations). A tracker-bound workspace refuses rename, status, archive
  and delete (change it through the tracker); a legacy `notion-*` one no longer
  refuses (2026-11-10); `local-*` (connected project) workspaces refuse delete
  only, because the directory worker would recreate them. Conversation rename
  sets `sessions.title` (set once from the first request, never regenerated);
  conversation archive is the existing reversible `archive_agent_session`, and
  archiving the open one starts a fresh conversation in that workspace. The
  rename field is `showPrompt` (the shared confirm dialog with an input).
  **One conversation menu (P1-39, 2026-10-25):** `workspaceMenu.ts`'s
  `conversationMenuItems` builds every home's items (rail rows via
  `sessionMenuItems`, Conversation history rows and the ⋯ beside the open
  conversation's title via `historyMenuItems`); a disabled item carries its
  full reason (`conversationBusy`). `archiveConversation` /
  `restoreConversation` (`conversationActions.ts`, loaded on first use) flash with Undo; `archive_session` refuses
  `ARCHIVE_BUSY` while a turn is live or queued (as moves refuse
  `MOVE_BUSY`), so a background turn's services aren't stopped under it.
  `moveSessionTo` resolves whether it moved and offers Undo (moves it back to
  `SessionMove.from`); `newWorkspaceFromChat` (same module) trashes the workspace it made
  when that move fails.
- **Chats (2026-09-30; called Quick chats until 2026-11-04).** `workspaces/quick-chats/` is a built-in
  workspace (`QUICK_CHATS_ID`), created on first use by `ensure_quick_chats`
  with only `WORKSPACE.md` (`kind: quick`, title "Chats"; an older brief's "Quick chats" reads as Chats too, its title decided by the id) and
  `sessions/`. `session_dir_for(vault, id)` is the cwd a conversation in a
  workspace must have (a live build worktree, else the folder; Chats is
  ensured); the move resolves targets through it.
  Its conversations are scoped runs like any workspace's, but their envelope
  is `QUICK_CHAT_ENVELOPE` (one sentence, no documents) and their system
  prompt says there is no project: answer directly, no brief/plan/outputs
  unless asked. `update_meta` and `trash` refuse it. The UI lists it only as
  the sidebar's Chats section (always shown; New chat at the sidebar's top
  starts one), never on Work or in the automation pickers.
- **Moving a conversation (2026-09-30).** `move_agent_session(key,
  workspaceId)` changes only `sessions.cwd` in the session store, which is all
  that ties a conversation to a workspace. In one IMMEDIATE transaction it
  checks the run queue for a queued/claimed/running chat row (the live record
  is checked first), updates the cwd, closes the open stint with reason
  `moved`, opens a fresh one and expires the conversation's open question
  cards. The native session is never resumed in the new folder: a composer
  still holding its id gets `ResumePlan::Reset(Moved)` and the fresh-session
  bundle from `conversation_entries`; one that reopened the conversation holds
  none (`resumable: briefOnly`) and sends its own bundle. Then the bounded
  record is rewritten in the target's `sessions/` and removed from the old one
  (a record without the Hollow header stays; the move reports a warning). The
  rail's command takes the `workspace-edit-` lease, so it is refused while a
  full-vault run holds the vault; the `hollow_app` action runs inside the
  calling run's lease, so the record move lands in that run's History. A
  record write can still land in the diff of another conversation running in
  the source or target workspace, exactly as `record_session` after a turn
  does. Unlike the agent-sessions plan's Phase 5 move (between harnesses, with
  a reviewed handoff brief), this move keeps the agent and relies on the
  existing conversation bundle, so there is no review sheet.
- **Create tasks from plan (Paperclip PR 11).** The Plan view's "Create
  tasks from plan…" preview and its kept batch report went with the workspace
  panel (COO rebuild L4, 2026-11-10); no UI calls it now.
  `plan_task_candidates(id)` stays: it reads `PLAN.md` (1 MB cap; a missing
  plan has no candidates) and returns its unchecked checklist items outside
  closed fences, deduplicated and capped at 100, as `PlanTaskCandidate { text
  (one line, ≤ 240 chars), idempotencyKey, origin, existingUrl }`. The key is
  `plan:workspaces/<id>/PLAN.md\n<sha256 of the item text trimmed,
  whitespace-collapsed and lowercased>`: never the plan's content hash, so
  editing, reordering or re-casing the plan keeps each item's key, and it is
  bounded whatever the item's length. `existingUrl` is the task a create with
  that key already made (`vault_tasks::fingerprint_index`). A task made with
  `origin: plan:workspaces/<id>/PLAN.md` starts as a copy of its item's text
  and is separate afterwards (ticking or editing the item doesn't change the
  task, finishing the task doesn't tick the item). Its **From: plan** opens
  PLAN.md in the Preview pane at that row (`TaskOrigin.tsx`, `planItemLine`),
  or says the item is gone.
- Workspace replies are camelCase. Workspace: id/title/status/path/cwd/modified;
  detail adds files (path/name/kind/size), AgentSessionSummary entries, and
  source coverage/origin/freshness metadata (no excerpt bodies in UI replies).
- Automation permission reviews add no IPC command: they are review-queue items
  of kind `workflow_permissions` / source `workflow` (payload
  `WorkflowPermissionReviewPayload` in `src/types.ts`) listed by
  `list_review_inbox` and resolved by `resolve_review_item` / rejected as usual.
  `resolve_review_item` takes an optional `expectedFingerprint` (the card the
  user acted on): required to approve a permission review, which is refused
  if the card changed since or the queue holds duplicate ids; checked for other
  kinds when supplied. A refused run's error starts with `Automation paused for
  review:`; the notification action `workflow-permissions:<slug>` opens the
  panel, and `settings:history` opens History & recovery. `HistoryRun.flags?:
  RunFlag[]` lists the protected changes an automated run made without
  Hollow's attribution (omitted when empty); History & recovery shows them in a
  "Protected changes in this run" banner. `Workflow.permissionHold?: 'review' |
  'unavailable'` (from `list_workflows` / `read_workflow`, computed read-only by
  `workflow_permissions::permission_holds` with the scheduler's own rule)
  makes the list and detail read "paused until its settings change is
  approved" (or "paused: automation permission records are unavailable")
  instead of any next-run time; a rejected change holds too, and restoring the
  approved file clears it. Run now refuses such an automation at once, before
  anything is queued; at claim the run queue holds its cron and trigger rows
  (`RunQueueItem.heldReason` = the run-log note, shown as the same "paused"
  text), a trigger row keeping its fire, and past its grace a cron occurrence
  is logged `not run: automation settings changed; review them in Automations
  → Permission changes`. The review panel reads its list through `preserveDuringRefetch`,
  so the frequent `workflows/` refreshes never blank the page.
- The automation editor's save is compare-and-swap (Paperclip PR 9, 9a-1).
  `read_workflow` returns `WorkflowDetail.editBase`, a digest of the fields
  the editor owns (title, description, schedule, agent, model, cwd,
  outputGlob, permissionMode, the tool lists and the trimmed prompt; no run
  bookkeeping), and `write_workflow` takes it back as optional
  `WorkflowInput.editBase`. When the file's editor-owned fields changed since
  (an Obsidian or agent edit), the save rejects with `WORKFLOW_EDIT_CONFLICT:
  …` and writes nothing; the editor keeps the draft and says to copy what to
  keep and reopen (reopening loads the current file and discards the unsaved
  edits). A prompt containing `## Run log` is refused on every save. A run
  finishing while the editor is open never conflicts. After an in-place save
  the editor reloads the detail, so the next save sends the new baseline. The
  save patches only those fields: every other key (including `routine`,
  `triggers`, `misfire_grace_min`, `skip_if_quiet`, `watch`, `notify`,
  `created` and every `last_*`) and the run log keep their bytes, and a
  rename writes the new file before trashing the old one and is refused while
  the automation runs. Omitting `editBase` (the new-automation form) skips the
  check. `Workflow` carries `skipIfQuiet`, `watch`, `notify` and
  `lastStartedAt`. Advanced → **Check before each scheduled run** (OC13) edits
  `precheck` (`WorkflowInput.precheck`, sent only when changed, `''` removes it,
  a stamped field, so saving a change asks for approval; see
  [automation](reference/automation.md#precheck-command-oc13-workflow_precheckrs)).
  Advanced → **Notifications** (Every run / Only failures /
  Never, Paperclip PR 9, 9a-2) edits `notify`: `WorkflowInput.notify` is sent
  only when changed, `always` removes the key, anything outside the three is
  refused, and `editBase` now covers it. `lastStartedAt` is stamped right
  before each run's agent spawns. The live run log's first line names the
  run's origin from the `started` event's `source`/`reason` ("Run started ·
  trigger: file changed: notes/a.md"). Advanced → **Skip when nothing
  changed** and **Watch globs** (Paperclip PR 9b) edit `skip_if_quiet` and
  `watch`: `WorkflowInput.skipIfQuiet` / `watch` are sent only when changed
  (None keeps the key; an empty list removes `watch`), a glob may not contain
  a comma, `editBase` covers both, and a save that sends either, creates a
  quiet-skipping automation, or changes what its check would watch (a new
  `cwd` included) is refused when the result is a watch the check cannot
  walk ("Skip when nothing changed: …"); an unrelated edit that leaves an
  already-broken config exactly as it was still saves. The watch defaults to `workspaces/<id>/**`
  for a workspace automation; elsewhere it is required. Scheduled occurrences
  of a workspace automation are skipped while its workspace's status is not
  current (the Work page's Active set; an empty or `Unknown` status counts as
  active), with one in-app notice per episode; set the status by hand in
  `WORKSPACE.md`. Both skips write benign
  `skipped` rows, coalesced (`· N skips`). A transient failure of a scheduled
  or triggered run queues one `retry` row a minute later (visible in the run
  queue, cancellable like any queued row); see [Quiet skip,
  inactive skip and retry](reference/automation.md#quiet-skip-inactive-skip-and-retry-paperclip-pr-9b). No new IPC.
- Settings → Usage (Paperclip PR 6) reads `usage_summary(window)`
  (`commands/usage_api.rs`; `window` is `day` | `week` | `month`, local time)
  and gets a camelCase `UsageSummary`: `window`, `since`, `generatedAt`,
  `total`, `bySurface`, `byWorkflow`, `byAgent` (each a `UsageBucket`: `key`,
  `label`, `runs`, `metered`, `partial`, `unknown`, `notStarted`, `inProgress`,
  `unpriced`, `inputTokens`, `cachedInputTokens`, `outputTokens`,
  `reportedCostUsd`, `estimatedCostUsd`, costs null rather than 0),
  `ledgerSince` and `ledgerError`. Chat emits a `usage` chat event
  (`{usage: TurnUsage, sessionKey}`) on every post-lease exit; the composer's
  cost chip renders it through `src/usageFormat.ts`. The ledger itself is
  described in [Usage ledger](reference/runs.md#usage-ledger-paperclip-pr-6).
- The Settings → Usage "Claude plan" card (Paperclip PR 8c) calls
  `claude_quota(force: bool)` (`commands/claude_quota_api.rs`), only after the
  user opts in. It always resolves: `null` hides the card (sticky only after a
  Keychain or endpoint failure; otherwise the card can appear once a Hollow
  Claude run reports limits), otherwise a
  camelCase `ClaudeQuota`: `source` (`stream` | `endpoint`), `observedAt` and
  `windows` (each a `ClaudeQuotaWindow`: `id` — `five_hour`, `seven_day`,
  `seven_day_opus`, `seven_day_sonnet` or `weekly_scoped:<model>` — `label`,
  `usedPercent` (may exceed 100), `resetsAt` (RFC 3339 or null), `source`,
  `observedAt`). `force` is Refresh, which may ask Claude Code's usage
  endpoint at most once a minute. Display only; see the [backend module map](reference/backend-module-map.md#backend-module-map) →
  `claude_quota.rs`.
- Usage budgets (Paperclip PR 8a, `commands/budgets_api.rs`): `get_budgets()`
  returns a camelCase `BudgetsView` (`budgets: BudgetStatus[]` with each
  budget's window, `observed` (null when the ledger is unreadable or no
  counted run reported the metric: shown as a dash, never 0),
  `percent`, `state` (`ok` | `warning` | `over` | `paused` | `unknown`),
  `pause`, run counts, `enforceability` (`ok` | `partial` | `none`) and a
  reason, `scopeMissing`, `suggestedLimit`; `ledgerDays`,
  `suggestionsAvailable` (7+ days), `blocked {allAutomations, heartbeat,
  workflows}`, `storeError`, `foreignEdit`, `enforcement`);
  `save_budget(input: BudgetInput)` returns the stored `BudgetDef` or a string
  refusal (a budget that could never trip, a duplicate); `delete_budget(id)`;
  `resume_budget(id, newLimit)` (strictly above this window's usage; offered
  for a paused budget and for a hard-stop budget over its limit whose pause no
  claim has opened yet). Writes
  emit `budget-changed`, which refreshes the panel, every review panel and the
  workflow list. `Workflow.budgetHold` (from `list_workflows` /
  `read_workflow`) is the blocking message when a budget holds an automation:
  a "Budget paused" chip and the next-run text "paused by a usage budget",
  below a permission hold. A **Budget pauses** panel (`ReviewInbox` context
  `budgets`, kind `budget_pause`) sits beside Permission changes above the
  Automations split: Raise and resume (a New limit field, enabled only above
  the observed usage; `resolve_review_item(accept, editedPayload.newLimit)`)
  or Keep paused (reject, which holds the pause past its window reset); no
  Edit or Tomorrow, and snoozing is refused. A chat turn may receive
  `{type: 'budgetWarning', sessionKey, notice}` after its `runStarted`; the
  composer shows it only in that conversation. Rules in [Usage
  budgets](reference/runs.md#usage-budgets-paperclip-pr-8a).
- Saved turn feedback (Paperclip PR 17, `commands/feedback_api.rs`, a
  `turn_feedback` table in the session store outside the vault):
  `record_turn_feedback(input: {runId, vote: 'useful' | 'notUseful', reason?})`
  returns the stored camelCase `TurnFeedback` (`id`, `sessionKey`,
  `sessionTitle` (redacted), `turnOrd`, `runId`, `vote`, `reason`, `agent`,
  `model`, `snapshot {asked, answer (≤ 8,000 chars), answerTruncated, tools,
  receiptId, receiptStatus, verificationState, recall, turnStatus}`,
  `createdAt`, `updatedAt`); a re-vote replaces the turn's row, except that the
  same vote with no reason keeps the saved reason. Refusals are
  strings: `FEEDBACK_NO_TURN:` (no turn carries the run id), `FEEDBACK_AMBIGUOUS_RUN:`
  (more than one does), `FEEDBACK_TURN_RUNNING:` (the turn's run still holds
  the vault lease), `FEEDBACK_INVALID:`. `list_feedback(sessionKey?, runIds?,
  limit?)`: newest update first; `runIds` (at most 200) is how a transcript
  loads the votes for the runs it shows; `limit` defaults to 200, at most
  1,000. `export_feedback(dir)` returns `FeedbackExport {jsonPath,
  markdownPath, count}` or `FEEDBACK_EXPORT_IN_VAULT:` / `FEEDBACK_EXPORT_INVALID:`;
  `clear_feedback(sessionKey?)` returns how many votes it removed. ChatBody
  votes only with `ChatTurn.runId` (exact sources; legacy Claude-history
  turns are marked locally, "not saved") and hides the buttons on the run in
  flight. **Settings → Usage → Votes** (the `feedback` id lands there since
  P1-22) listed the newest 50 votes, each opening its turn, with Export… and
  Clear feedback. Votes left the window overnight 11-08 S5; the frontend's
  vote store, turn jump and their `api.ts` wrappers went in the 11-10 sweep,
  and the Rust commands stay.
- Mid-run budget stops (Paperclip PR 8b): `RunLogEntry.exitKind` is `"budget"`
  (else null) on a run a usage budget ended; `lastStatus` stays `error` and
  `lastError` starts `Budget limit reached:`. Workflows shows such a newest run
  as a "Stopped at a budget limit" notice with **Review budget pauses** (it opens
  the Budget pauses panel) instead of the failure debugger and Retry, gives the
  row and cover a warning conduit, and keeps budget stops out of Recent
  failures. A Claude chat turn can also receive `budgetWarning` mid-turn when a
  level is crossed inside the turn. Settings' enforcement note says Claude
  automations stop mid-run at token limits, a spending limit may overshoot by
  one message, and Codex/Antigravity are checked only before they start. Rules
  in [Mid-run enforcement](reference/runs.md#mid-run-enforcement-paperclip-pr-8b).
- The durable automation queue (Paperclip PR 7, `src-tauri/src/run_queue.rs`,
  a `run_queue` table in the session store outside the vault) is exposed by
  `commands/run_queue_api.rs`: `list_run_queue(recent?)` returns
  `RunQueueItem[]` (live rows first, running then claimed then queued in claim
  order with a 1-based `position`, then the newest `recent` finished rows,
  default 20, at most 100; the row payload never crosses IPC) and
  `cancel_queued_run(id)` returns the row after acting by status: a queued
  cron row is marked to be skipped (`cancelRequested`; its occurrence is still
  recorded at claim), any other queued row ends `cancelled`, a claimed row is
  marked so the pump stops it, a running workflow is stopped, and a running
  heartbeat refuses. Like `cancel_workflow`'s queue part, it refuses in a
  Hollow process that does not own the queue. Every change emits `run-queue-event {kind, item}`:
  `enqueued` / `coalesced` from the scheduler, the heartbeat timer and Run
  now; `waiting`, `running`, `finished`, `requeued`, `held` from the pump;
  `cancelled` from a cancel (`cancel_workflow` now also cancels the workflow's
  queued Run now and retry rows and flags a claimed row). `run_workflow` (Run
  now) keeps its signature but now waits in the queue for its row's outcome
  instead of refusing behind a busy vault; it still refuses at once for a
  permission refusal, a stale or unknown lease (with the restart hint), or a
  second Hollow process that owns the queue. The UI
  refetches the list on any event, debounced 250 ms, into a plain signal
  (`src/store/runQueue.ts`, never a resource under the page Suspense).
  Work → Running shows queued and running turns, with Cancel / Skip / Stop
  and each turn's progress. Its Background tasks section retains in-process
  operation details, results, elapsed time and Clear completed. A Settings routine row
  and a Workflows row, cover or editor show `Queued · …` (above the
  catch-up text) unless a permission hold already explains it. Wording
  lives in the pure `src/runQueue.ts`. `tests/smoke/run-queue.spec.ts` covers
  Running, cancel and the event refetch with `localStorage.mockRunQueue`.
- Silent-run indicator (Paperclip PR 15b). A running workflow emits
  `workflow-event` `{type:'activity', slug, runId, queueId, lastEventAt,
  progress}` every 15 s (`lastEventAt`: its last stdout/stderr line, else its
  spawn; `progress`: `researching` | `usingTools` | `workingWithFiles` |
  `coordinating`, from the latest tool NAME), and its `done` now carries
  `runId` and `queueId`. A running chat turn emits the same on `chat-event`
  as `{type:'activity', runId, sessionKey, lastEventAt, progress}` (plus its
  turn tag), and `agent_session_live` adds `lastEventAt` and `progress` to the
  live record (no UI caller yet). The dock's running queue rows show the
  progress and `quiet Nm` (after two minutes) from `src/store/runActivity.ts`
  by `historyRunId`, and a Stop that calls `cancel_queued_run` (workflow and
  chat rows; the heartbeat cannot be stopped); the top bar mounts the dock
  for a running automation even when nothing is queued (`dockVisible`), but
  not for a running chat turn alone; the composer's ember shows the turn's
  quiet time. Pure helpers: `src/runActivity.ts`. Smoke:
  `tests/smoke/run-activity.spec.ts`. Settings → Models adds the
  Codex no-output watchdog (minutes, 1-1440, default 30, Off), saved as
  `codexNoOutputTimeoutMin` (absent = 30, `null` = off) through
  `agentSettingsPayload()`; a Codex chat turn or workflow run quiet that long
  (no output, no process activity) is stopped and recorded as `stalled: no
  output for Nm` (result kind `stalled`, never retried or continued).
  Since merged plan slice 3 every agent has that watchdog (the others in
  `noOutputTimeoutMin`) plus an optional per-run deadline (`deadlineMin`,
  recorded as `timedOut`), and a pending Hollow approval card pauses the
  watchdog (a question card does not: nothing waits on it).
- Phase timings (Paperclip PR 15c), no new command or event: chat, workflow
  and heartbeat evidence receipts gain `phaseTimings`
  (`EvidenceReceiptWire.phaseTimings`: `queueWaitMs`, `baselineSnapshotMs`,
  `spawnMs`, `firstOutputMs`, `turnMs`, `repoCaptureMs`, `finalizeDiffMs`,
  `evidenceMs`, each omitted when unmeasured), carried by
  `executionReceiptFromWire`. The expanded `ExecutionReceipt` lists the
  measured ones under **Timings** (`phaseTimingRows`, formatted `12ms`,
  `2.3s`, `2m 5s`); the compact receipt hides them. The same keys are merged
  into the run-queue row's `phase_timings_json` (never sent over IPC). Smoke:
  the receipt case in `tests/smoke/run-activity.spec.ts`. See [Phase
  timings](reference/runs.md#phase-timings-paperclip-pr-15c).
- Chat IPC since Paperclip PR 7b (D3): `chat_send(prompt, sessionId,
  sessionKey, cwd, model, permissionMode, agent, clientTurnId)` enqueues a
  run-queue row (source `chat`, priority 0, `sourceId` = the session key,
  `detail` = the user's own text) and returns `SendAccepted{sessionKey,
  queueId, queuePosition, phase: 'queued', waitingFor}`; it rejects (nothing
  queued) with `CHAT_CONTEXT_RECOVERED: {path}`, with
  `UNSAFE_WORKING_DIRECTORY: …` for a system or shared folder (Paperclip PR 19:
  `/`, `/tmp`, `/usr`, `/System`, `/Library`, bare `~`, never the vault; the
  conversation and its folder are kept, `src/chatCwd.ts`), while another chat
  turn is queued or running, from a process that does not own the queue, or
  behind a lease only a restart clears. `chat_preflight(cwd)` rejects an
  unsafe folder the same way (the composer, `/cwd <path>`, the Chat page's
  folder picker and artifact-session continuation all call it first). A lease
  whose owner process exited is not one of those since Paperclip PR 18: it
  reads as free and the pump's admission reclaims it. The pump admits the turn when the lease is free
  (never preempting a running automation) and emits `done` only after the row
  is terminal. `chat_cancel(sessionKey?)` stops that session's running turn,
  takes its queued turn out of the queue (emitting `cancelled{queued: true}`
  and `done`), or flags a claimed one; no key stops whatever runs.
  `agent_session_live` also reports a queued chat turn as `{key, agent,
  runId: '', phase: 'queued'}`, so Resume session refuses while a turn waits.
  The composer's Edit and Discard call `cancel_queued_run(id)`, which for a
  chat row also emits the turn's end events and stops a running one through
  its keyed stop slot. Every `chat-event` of a queued turn carries
  `clientTurnId` and `queueId`; `cancelled` adds `queued`, `done` adds
  `recovered` (and then omits `sessionKey`/`sessionId`), `runStarted` adds
  `asked` and `permissionMode` (the mode the turn runs in: an adopted turn's
  native session carries on into the person's next turn only when that mode
  is in the composer's mode's class, `permModes.ts` `modeClass`, 2026-11-04). A turn queued before Hollow quit is interrupted at startup, never
  run, and its text is put back into an empty composer once
  (`restoredQueuedTurns`).
- Question cards (Paperclip PR 13), `commands/interactions_api.rs`:
  `list_pending_interactions(sessionKey)` returns the conversation's open cards
  (`ChatInteraction[]`, status `pending` or `stale`, oldest first);
  `resolve_interaction(id, answer, nativeSessionId)` returns
  `ResolvedInteraction{interaction, prompt, resumes}`, and the composer sends
  `prompt` verbatim as the next turn through `chat_send` with the
  conversation's native session: the card's own (`resumes`), or, since
  2026-11-04, any other or none, when the answers go as an ordinary message
  naming each question in full (`format_answers`, as every answer does). The asking
  session never disables Send; the card says on its first question when its
  answers go as a message, and shows any real blocker (a turn running, the
  conversation loading, a changed target) from that first question.
  `dismiss_interaction(id)` and `reopen_interaction(id)` return the updated
  card. Refusals are plain strings prefixed `INTERACTION_CLOSED`,
  `INTERACTION_STALE` or `INTERACTION_INVALID`. A new `chat-event` member
  `{type: 'interaction', interaction, sessionKey}` (with the turn's
  `clientTurnId` / `queueId` tags) carries a card recorded, superseded,
  reopened or expired by a turn; the store applies it for the open
  conversation only and does not gate it on the queued turn.
  `archive_agent_session(key, true)` now also expires that conversation's
  open cards (no event; Chat.tsx drops them locally). Cards render in ChatBody
  after the approval cards (`components/QuestionCard.tsx`); mock mode seeds
  them through localStorage `mockPendingInteractions`.
- Approval events (Paperclip PR 21): `approval-request` carries
  `{id, title, action, kind, tool, toolUseId, sessionKey}`, where `tool` is
  the bare MCP tool name and `toolUseId` / `sessionKey` name the agent tool
  call the broker bound it to (same tool, JSON-equal arguments, oldest
  first) or are null when it could not bind. `approval-resolved` carries
  `{id, approved, cancelled, reason, toolUseId, sessionKey, tool}` with
  `reason` `allowed | denied | timed_out | cancelled`; `approved` and
  `cancelled` keep their old meaning. `approval_respond(id, approved, reason?)`
  takes the optional typed Decline reason since SA1 (mock: a NO_OP). The tool row badges live only, buffered by id
  (`src/chatApproval.ts`). Each turn's tool calls render through
  `components/ToolLedger.tsx`: its header summarizes the turn's calls
  (`src/toolDescribe.ts`), 3+ consecutive same-family calls fold, and a
  Raw toggle shows them all (windowed past 300).
- Foreground notification scope (Paperclip PR 22):
  `set_foreground_notification_scope(scope: string | null)`
  (`commands/system.rs`; mock: a NO_OP) is sent by Automations with
  `workflow:<slug>` while a workflow is selected and null when it closes. A
  notification whose `actionUrl` is exactly that scope skips its native banner
  while the main window is focused (not for `run-guard` or `budgets`), and
  the webview skips its in-app toast while the window is in front
  (`src/toastPolicy.ts`). It still reaches the notification center.
- Task tools (Paperclip PR 12), `commands/tasks_tools_api.rs`:
  `set_tasks_mcp_enabled(enabled)` persists Settings → Connectors → Built-in agent tools → Task tools
  (default on) in `agent-tools.json` outside the vault and applies it;
  `tasks_mcp_enabled()` returns the live switch, which the `tasksMcpEnabled`
  signal reads at boot (no localStorage: the backend loads the file before the
  run-queue pump starts). The event `review-queue-changed`
  `{kind: 'agent_task', count}` follows a `propose_tasks` call that queued
  suggestions and refreshes the review panels. Kind `agent_task` ("Agent-proposed
  task", source `chat` or `workflow`) lives in Tasks → Task suggestions and
  accepts through `resolve_review_item`. The agent-side tools themselves are
  MCP, not IPC ([Task tools](reference/agent-tools.md#task-tools-hollow_tasks-paperclip-pr-12)); mock mode answers
  `tasks_mcp_enabled` with true and ignores the setter.
- Result kinds and effort (Paperclip PR 14a), no new command:
  `get_agent_settings` / `set_agent_settings` carry an optional
  `efforts: {claude?, codex?}` (omitted when empty; `set` refuses an unknown
  level or any Antigravity effort), and each `agent_descriptors` entry's
  `capabilities.effortLevels` lists the levels its CLI accepts (Claude `low`
  … `max`, Codex `minimal` … `xhigh`, Antigravity none). Settings → Models &
  models → Reasoning effort saves through `agentSettingsPayload`
  (`src/agentSettingsPayload.ts`, the one save-payload builder). The chat
  `result` event gains `resultKind` (the envelope's own kind: an error result
  is labelled in the transcript by `src/resultKind.ts`, a refusal shows as a
  neutral system notice) and the `error` event gains `resultKind` with its
  `message` already labelled by the backend (shown verbatim). Evidence
  receipts gain `resultKind` (`EvidenceReceiptWire.resultKind`, passed through
  by `executionReceiptFromWire`), and a failure kind adds a `result` line to
  `failures`. See [Result kinds](reference/runs.md#result-kinds-and-effort-paperclip-pr-14a).
- Resume hardening (Paperclip PR 14b), no new command: a new chat event
  `sessionReset {reason: 'configChanged' | 'resumeFailed', agent, sessionKey}`,
  plus `changed` on `configChanged` only (OB03: the fingerprint parts that
  changed, `chat_resume::PARTS`, or `unknown`)
  (the conversation moved to a fresh native session; the frontend clears
  `chatSessionId`, drops that key's question cards, which arrive expired as
  `interaction` events just before it, and shows a neutral notice).
  `read_agent_conversation` turns gain `resultKind` (assistant turns, from the
  stored result envelope) and `stoppedBeforeSpawn`; a `cancelled` event of a
  turn stopped before its agent spawned carries `beforeSpawn: true`. A live
  stop notice, or a restored assistant turn that was cancelled or interrupted,
  offers **Continue** (never for a stop before spawn), which sends exactly
  "Your previous run was interrupted. Continue from where you left off." on
  the same conversation through the normal queued send (`dispatchTurn`),
  never automatically. See [Resume hardening](reference/runs.md#resume-hardening-paperclip-pr-14b).
- Repository diff receipts (Paperclip PR 16): `EvidenceReceipt.repoChanges?`
  (`EvidenceRepoChangesWire[]` in `src/executionReceipt.ts`, without patches,
  at most 50 rows per repository with the real `totalFiles`) on every chat
  and workflow receipt whose run had a repository target. A repo-only receipt
  maps to `unverified`, never `no_changes` (`receiptStatusFromWire`), and
  `ExecutionReceipt.tsx` shows a Repository changes section. Its Show patches
  button (not in compact receipts) calls
  `read_history_repo_changes(runId) -> RepoSidecar | null`
  (`commands/repo_receipts_api.rs`, read-only) once through `trackTask`; a
  withheld row says its content was not stored. Mock mode answers for the
  morning-brief fixture run only (`tests/smoke/repo-receipt.spec.ts`). See
  [directories](DIRECTORIES.md#repository-diff-receipts-paperclip-pr-16).
- Startup only warms provider discovery. Legacy resources load when their
  surfaces request them. Chat/editor/graph code remains lazy; no provider-latency
  or memory-budget claim follows solely from this change.

`tests/smoke/workbench.spec.ts` covers create/edit/reload, source attachment,
composer separation, chat/context geometry and focus, conflict preservation, minimal startup, and provider session
restore using fixture IPC. Rust tests exercise real temporary filesystem and
SQLite behavior, path/symlink boundaries, size caps, and source preservation.
Neither is evidence of a live provider completing a real work deliverable.

## Reconciled source and release identity

The previous installed worktree shared commit `2fa8d55` and version 0.4.0 with
`projects/personal/hollow`, but omitted that checkout's uncommitted September
changes. They were three-way reconciled here before this implementation. Both
input trees and patches are saved locally in `.context/source-reconcile/`.
The original checkout remains untouched. Keep this reconciliation in any future
merge; do not replace these files with the old base merely because Git agrees.

Version 0.5.0 and later bundle `build-provenance.json`: SHA-256 over build input paths and
bytes, Git commit, checkout, generation time, version and dirty flag. It contains
hashes/metadata, not source contents or config secrets. `npm run build` and the
Rust build script generate it. The installer checks that the bundle matches the
current checkout's inputs **before** replacing the installed app. A different
checkout with identical source is equivalent; a matching version alone is not.
A passing `npm run gate` on a clean tree also keeps its provenance in
`<git common dir>/hollow-gate-records/<fingerprint>.json` (`scripts/gate-record.mjs`), and
worktree cleanup keeps a clean build's provenance there before removing the worktree, so
`build.install_ready` can still vouch for origin/main's files after the gated worktree is gone.
Use an absolute workspace-specific `CARGO_TARGET_DIR` for this checkout. Keep
stable `Hollow Dev Signing`; never bypass Keychain authentication by resigning.

## Remaining reset work

The outcome workspace and Library are implemented. Build workspaces (B1),
their Changes pane (B2), install from Hollow (B5) and services (B6) are
implemented; parallel runs and the rest of the
build programme are planned in [the build plan](history/HOLLOW_BUILD_PLAN.md). Saved workspace templates, workbook/diagram editors, global durable image
drafts, and the proposed measured latency budgets remain follow-up work.
Automations and Settings reuse their underlying runners and writers with the
new workspace context contract. Existing unscoped automations retain their
configuration until explicitly changed. Historical source modules remain for
compatibility, but are not separate product destinations.

## Chat-first Home

Sam's #1047 Home starts with Ask, then five projects in last-activity order.
All projects exposes origin-aware Find, Include archived, archive suggestions,
row menus, Undo merge and Recently deleted. Pin rank still orders the sidebar,
but does not reorder Home; Home has no Pinned filter or sort menu.
Unique names keep origin metadata in the row tooltip; duplicate names also
show distinguishing metadata beside the title. Project icons stay visible.
Ask retains its chosen scope, saved text, Alt-place and accepted-send guards.
No project coordinator or per-provider new-chat default is introduced.

Work has three primary tabs: Home, Today and Running, plus a visible Tasks
button. Map opens beside Home's project heading and keeps Home selected;
clicking Home returns to its list. The `map` route, palette and shortcuts stay.
Home's shorter Ask keeps its scope, project choice and send action on one
wrapping row. Today/Week recommendations use quieter ruled groups and retain
Preview draft and their explicit draft-only buttons.
Weekly review is Today's Week range, not its own tab. Daily counts and run/result evidence stay in their existing detail
views: Today, Review, Running, Automations → Last night and project sections.
Saved Home canvases move to Today with compatible destination keys and a
matching Show action.
Home no longer polls workspace digests, renders resume cards or doorway art.
Empty-chat artwork and Main's separate page/started-chats list are unchanged.
