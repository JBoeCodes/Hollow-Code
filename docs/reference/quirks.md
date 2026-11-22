# Quirks: runs, processes, frontend and build

Part of the agent manual. [AGENTS.md](../../AGENTS.md) is the entry point and
links every reference doc; this one holds the "Runs and processes" and
"Frontend and build" groups of CLAUDE.md's "Quirks to know about", moved there
word for word on 2026-10-13 (only relative links were re-based). Read it
before touching runs, leases, processes, MCP servers, Jev, the CSP, artifacts,
canvases or the Preview pane's project files. Keep it current with the code in the
same change, as AGENTS.md asks.

This continues CLAUDE.md's `## Quirks to know about`; its lead-in and the
**Data safety** group moved here on 2026-11-21 and come first, under
[Quirks to know about](#quirks-to-know-about). [AGENTS.md](../../AGENTS.md#data-safety) keeps the binding summary of the data-safety rules.

## Quirks to know about

The full list (about 70 KB, many entries with their history) is in the
quirks archive.
The ones that bite most often:

**Data safety**

- **Found context is accepted one item at a time** (CX20): `context_review.rs`, no batch accept, never overwrites ([surfaces](surfaces.md)).
- **Turn checkpoints only in a chat's own worktree** (CP1, 2026-11-18): `turn_checkpoints.rs` writes
  `refs/hollow/checkpoints/<key>/<ord>/before|after`; Restore saves `restore-<time>` first and is refused
  in a shared folder or main checkout before any file changes. Never prune those refs by hand.
- **Shared tab reads** (CD3, GU1): WebKit has no `responseStatus`, so `ui.read_tab` timing names only failed
  images; `ui.inspect_tab` is the selector read.
- **The window's `read_agent_conversation` is lean** (WD1, 2026-11-18): tool results over 1,200 chars come as
  previews (`resultChars`), fetched whole on expand; agents read `read_agent_conversation_full`. Budget test: `wire_tests`.
- **Side chat** (7.6, 2026-11-20): `side_chat.rs` + `SideChatPane.tsx`, a second chat beside a chat (not the COO's). `forkKind` `side`, off every list, seeded via `continued_from`, mode capped at its parent's ([SIDE_CHAT](../SIDE_CHAT.md)).
- **A branch shares its original's folder** (CH1, 2026-11-18): `chat_branch.rs` copies turns into a new chat
  (`forkKind` `branch`), never writes the original, and keeps a checkpoint as `refs/hollow/branches/<key>`.
- **Native resume of an imported Claude Code chat** (H8, 2026-11-19): `session_import/native_resume.rs` forks the source
  session by the CLI (`--resume <id> --fork-session`, flags probed from `claude --help`, cached); Hollow never writes `~/.claude`.
  Offered only if the jsonl, id and cwd still match; else the handoff. [Surfaces](surfaces.md#session-import).
- **Binaries between the Macs come over only when asked** (V4 reworked to
  N15, 2026-11-18): `sync_files.rs`; no timer copies anything. `vault.fetch_peer_file`
  never overwrites. [Details](../DIRECTORIES.md#binary-copy-between-the-macs-v4-2026-11-15).
- **Folder abstracts are off by default** (CX23, `context.folderAbstracts`): when on, a
  refresh sends redacted text of non-secret workspace and org files to the model; the
  index is derived and rebuildable. [Details](../DIRECTORIES.md#folder-abstracts-cx23-2026-11-18).
- **Never re-render an existing task, project or workflow file.** The models
  are lossy (`frontmatter::parse` skips `- ` list lines, `render` emits only
  modelled keys). Edit through `vault_tasks::patch_task` /
  `frontmatter::patch` / `patch_workflow` (which re-parses its own output);
  `render` is for new files. Task steps go through
  `vault_tasks::splice_subtasks`, never a recompose. New task creators that
  must not mint a `slug-2.md` twin use `vault_tasks::create_idempotent`.
- **Tracker sync patches through `frontmatter::patch` and re-parses**
  (`KeyEdit::Quoted`, `verify_patch`); `frontmatter::unquote` must stay a
  single left-to-right pass.
- **A write made on a run's behalf goes through
  `automated_history::with_live_run`**, which re-reads the lease under
  `HISTORY_LOCK` and writes while holding it. Never authorize with
  `lease_holder` and then write. Scoped admissions hold it shared
  (SE5); never upgrade to exclusive.
- **Never write tasks or `.review-queue.json` from an MCP subprocess**; it
  can't take the GUI's locks. Subprocesses proxy writes to a GUI broker.
- **A read-modify-write of a vault text file goes through
  `vault::write_merging`** (or `replace_if_unchanged` for JSON and HTML), never
  read then `atomic_write`: an outside edit is merged, overlapping ones are
  reported ([Verified writes](vault.md#verified-writes-2026-11-14)). An agent's
  `vault_write_note` that clashes gets both versions back to merge itself
  (`vault_clash.rs`); an item only if it gives up.
- **Never write a watched dot-state file with `atomic_write` directly**; use
  `run_guard::write_host_state`, and route Hollow's own writes of protected
  vault files through `run_guard::note_host_vault_write`.
- **State stores fail closed.** `state_file.rs` stores, permission stamps
  (deleting the store refuses every automation until restart), `budgets.json`
  (a corrupt one holds every automation; reread before each run) and the rest
  are never read as defaults or overwritten. Unattended readers never move a
  damaged file; only a write or the screen the user opened does.
- **Workflow vault writers hold `workflow_permissions::write_guard()`**, and a
  workflow a test writes with `fs::write` is refused: seed fixtures with
  `vault_workflows::seed_workflow(_with)`.
- **`walk_notes` prunes a dot-prefixed root**, so a fixture rooted at
  `tempfile::TempDir::new()` (`.tmpXXXX`) is silently empty. Use
  `tempfile::Builder::new().prefix("vault-")`.
- **OC5 lane rows:** [attempt ids, circuit breaker, stale base](agent-tools.md#oc5-lane-rows).

The **Runs and processes** and **Frontend and build** groups continue
below.

**Runs and processes**

- **A Dock- or Finder-launched Hollow has launchd's bare PATH.** Agents still find Homebrew, cargo and every
  folder `~/.zshrc` adds (gcloud, bun, grok) because `agent::cli_path` folds in the login shell's PATH, read once
  at launch (`shell_path.rs`, B03, 2026-10-16). It arrives a moment after launch: a run started in the first
  second, or on a Mac whose shell rc hangs past 5 s, gets Hollow's own folders only. Never read
  `std::env::var("PATH")` for an agent's PATH; use `cli_path()` (B02 was a build workspace doing exactly that;
  the self-install, Plaud and install-readiness helpers still build theirs with `self_install::tool_path`).
- **Only the lease says whether the vault is held.** Automations enqueue;
  the pump is the only automation caller of `try_begin_run`. `lease_holder`
  and `busy_reason` only name a holder (racy). A stale-looking lease is
  reclaimed only when its lock is free and its pid is gone (Paperclip PR 18);
  otherwise restart Hollow rather than deleting `.hollow-history/active-run.nosync`.
- **The lease file is `active-run.nosync` (night ST6, 2026-10-14).** It is
  created and removed on every run; under its old name `active-run`, iCloud
  made a conflict copy on almost every run (`active-run 2` … `active-run 71`
  by 10-12). iCloud Drive skips `.nosync` names (widely relied on, not
  documented by Apple; check `.hollow-history/` for new copies after an
  install). A legacy `active-run` from an older build is busy while the
  vault's lease lock is held or its recorded pid is alive, removed only when
  the lock is free and its owner exited (or it has no pid), and spared at
  startup as the live owner; an unknown lock state with no live pid doesn't
  block. Older builds don't know the new name, so don't run a pre-ST6 build
  against the vault at the same time as a newer one. Startup removes
  `active-run <n>` / `active-run <n>.nosync` conflict copies, which are never
  read as a lease.
- **A workspace's turn holds no vault lease:** a chat whose cwd is a live
  build worktree or exactly `vault/workspaces/<id>` is a scoped run, tracked
  and diffed for that folder only, beside automations. Every agent turn, the
  vault lane's run and swarm parts included, counts against Settings →
  Chat → Chats at once (default 10 across Hollow, 5 in one workspace's
  folder, settable to 25 and 10 since 2026-11-07; `run_pump::QueueFacts::hold`,
  2026-10-20), and none starts while
  macOS memory pressure is warning or worse. A build worktree takes one turn at a time (`try_begin_scoped`,
  exclusive); a workspace's own folder, Chats included, takes several
  at once (`try_begin_scoped_shared`, 2026-10-07). Overlapping runs in one
  folder each diff the whole folder, so the second to finish marks the files
  both changed (`HistoryChange.also_changed_by`, `HistoryRun.overlaps`) and
  restore refuses those unless forced; every turn in a workspace's own folder
  gets a `<hollow-workspace-shared>` note (since 2026-10-20, peer live or not),
  and a shared doc written over from an older copy gets a conflict copy (or,
  for Hollow's own change, a merge back) at the run's finish ([Shared docs](../WORKSPACES.md#shared-docs-while-chats-run-in-parallel)).
  Its agent can still write elsewhere with shell tools, unattributed. A restore
  refuses only a path inside a live scoped run's folder or in its
  `broker_changes` (`live_writer_refusal`, checked under `HISTORY_LOCK`, F5
  2026-10-20; paths compared by `disk_key`, the case-folded on-disk spelling,
  since APFS is case-insensitive). Restore writes are attributed to Hollow
  (`run_guard::note_host_vault_write`); a clear keeps live scoped runs' records
  and objects (`clear_all_but`) and refuses on an unreadable live pin file.
  `scoped_run_live` now only gates the install. A restore can therefore land in
  a folder-less path a live agent later writes with shell tools; that write is
  the agent's, unattributed, as before. Since 2026-10-20 a run finishing beside a live
  peer marks the overlap from the peer's baseline, and a chat turn that shared
  its folder is credited only with the files its own structured writes named
  (`chat::finish_observed_run` → `automated_history::note_own_writes`); the
  rest are `HistoryChange.unattributed` and `shared` on its receipt. Hollow's
  session records there are `host_write`s, left out of live peers' diffs.
- **Chat is one turn at a time per conversation, not per vault.** Leaving a
  running conversation snapshots it into the background and buffers its
  events (`src/chatRouting.ts`); reopening replays them.
- **Process cleanup (`process_group.rs`):** a run's processes are its
  group, its descendants, what sampling recorded and what carries its
  `HOLLOW_RUN_TAG`. Finishing a turn also ends servers the agent started, so
  start long-running servers through `hollow_services` or outside Hollow.
  Runs that outlive a crashed or quitting Hollow are recorded in the run
  ledger and ended by the boot sweep and the crash reaper
  (`process_group/orphans.rs`, `process_group/reaper.rs`). After
  `cargo test --lib`, `ps -axo pid,ppid,pgid,etime,command | grep -E
  'sleeper_helper|/bin/sleep 60' | grep -v grep` must print nothing.
- **Hollow overrides `TMPDIR`/`TEMP`/`TMP` for agent children** (the run's
  scratch folder, deleted when the run ends). Run `cargo test` from inside a
  Hollow turn with `TMPDIR` set to a neutral folder.
- **Toggling a Hollow MCP server, the model, the permission mode or the
  working folder costs one fresh native session** (the configuration
  fingerprint); library connectors never do, nor does a guidance edit
  (fingerprint v3, OB04) unless it bumps `chat_resume::PROMPT_CONTRACT`. Keep
  guidance constants byte-stable anyway: a changed appended prompt rewrites
  Claude's whole cached prefix on the next resumed call.
- **The Sessions DB has writers in more than one process:** every
  read-then-write goes through `session_store::with_write` (`BEGIN
  IMMEDIATE`); usage timestamps are fixed-width UTC milliseconds
  (`session_store::db_time`).
- **Chat `401 Invalid authentication credentials` is the `claude` CLI's own
  login**: re-auth in a terminal (`claude` → `/login`); never refresh Claude
  Code's OAuth token from Hollow (it would log the CLI out).
- **`claude_runner::redact` is the only redactor**; extend it, never add a
  second. Hollow has no session delete.
- **Every Jev request goes to `jev::endpoint()`**, never a literal URL. A
  candidate launched with `HOLLOW_CANDIDATE_JEV_ENDPOINT=http://127.0.0.1:…`
  (`instance::candidate_jev_endpoint`; the removed test bed set it) sends Jev
  requests there and never reaches TypeSafe. Only a plain-http URL to
  `127.0.0.1` or `[::1]` is honoured (not `localhost`), and the everyday app
  ignores the variable (`instance.json` records it as `jevEndpoint`).
- **`hollow jev-study` is the one way to call Jev outside a turn, and it
  refuses by default** (`jev_study.rs`, agent effectiveness E3). It runs the
  judged fixtures through the production Jev paths, Jev off and on, and
  needs either `--endpoint` with the same loopback rule (sending a
  placeholder key) or an explicit `--typesafe`, which reads this instance's
  saved key from the Keychain, bills its TypeSafe account and never prints
  the key. Only the orchestrator runs `--typesafe`. `--max-calls` (200)
  stops before a request that would pass it; `run.json`'s tally counts a
  request before it is sent (`inFlight`) and is rewritten after it, and a
  folder holding any study file is refused unless `--resume` (which
  continues it from the larger of the tally and the arm files' count, the
  cap covering the total across resumes). It is compiled into every
  build, everyday included, so the refusal is the safety, not the build.

**Frontend and build**

- **Tauri `invoke` rejects with a plain string**, so `e?.message` is
  `undefined`; use `errMsg` / `taskErrorText` (`utils.ts`).
- **The app CSP is enforced only in installed builds** (`tauri dev` sets no
  `devCsp`): verify anything CSP- or sandbox-related in a release bundle.
  Artifact documents load through the `artifact://` protocol with their own
  CSP; `scripts/sync-sandbox-csp.mjs` keeps the runtime's and the Canvas
  Kit's hashes (exactly those two) in `tauri.conf.json`'s `script-src`, for
  the srcdoc fallback that inherits it. No artifact policy lists them.
- **Video plays only through `hollow-media` (media.rs, 2026-10-17).** The main
  CSP's `media-src` is local-scheme only: never add `https:`, `data:` or
  `blob:`, and never enable `use_https_scheme` (the Windows media URL would
  become `https://hollow-media.localhost`, which the Https and Interactive
  artifact modes' `media-src https:` match). WKWebView's media loader needs
  `206` Range answers; wry hands WebKit one body per request, so each is capped
  at 2 MiB and WebKit asks again. Never answer it with a 5xx (fatal, and
  `Retry-After` is ignored): queue. JS can't see a media response's status,
  so the player classifies a failure by granting again (`mediaKinds.ts`
  `classifyFailure`). WebM isn't served: WebKit's WebM player asks once, with
  no `Range`, and stalls after a short answer (spike, 2026-10-17). Verify
  playback in an installed build. Recordings' remote `<audio>`
  (`RecordingsView.tsx`) is probably blocked by the CSP already; that is a
  separate issue and must not be fixed by widening `media-src`. `qlmanage -t`
  writes a **PNG** named `<file>.png` whatever you ask for: posters convert it
  with `sips` (media.rs `quicklook_jpeg`). QuickLook does its work in
  launchd-owned XPC helpers: the 5 s bound and group kill stop `qlmanage`,
  not that work.
- **`artifacts.rs` owns the served artifact policy (Canvas C1).** The frontend
  sends a mode (`offline` default, `canvas`, `images`, `https`, `interactive`),
  never a CSP string; an unknown mode is refused. **No surface shows the modes
  (10-11 B1):** a document opens in its surface's default, and a refused remote
  load or locked script (bridge `blocked`, from the runtime's
  `securitypolicyviolation` listener) raises one permission line whose Allow is
  remembered per canvas in localStorage (`src/canvas/canvasGrants.ts`; rules in
  `docs/CANVAS.md` § Sandbox modes). Editing `sandbox-doc.js` changes its hash:
  run `node scripts/sync-sandbox-csp.mjs` (the build does too). `canvas` runs artifact scripts
  with no network (`script-src 'unsafe-inline'`, everything else `'none'` or
  `data:`/`blob:`) and lists **no hash or nonce**: either beside
  `'unsafe-inline'` disables it. **Offline, Images and HTTPS list no hash
  either** (SRI repair, 2026-10-05): under CSP3 a hash-source also admits an
  external `<script src integrity="sha256-…">`, which the browser requests
  before the integrity check fails, and the runtime and kit hashes are public.
  They allow `'nonce-…'` only. `artifact_put(parts, mode)` takes the document
  as host-built parts (`sandboxDocParts`); Rust draws a fresh 144-bit nonce
  per document (OS CSPRNG) and joins the parts with it, so it lands only in
  the slots the host left (policy meta, runtime tag, kit tag, all before the
  agent's body) and the text is never searched. The part count must match the
  mode (3–4 for nonce modes, 1 for Canvas/Interactive). The srcdoc fallback
  seals the same parts with its own `crypto.getRandomValues` nonce
  (`sealDoc`). Never add a hash, and never fill a nonce by text replace over a
  document holding agent HTML. Rust tests still hash `sandbox-doc.js` and the
  generated kit (`src/canvas/kit/generated/kit.min.js`) and fail if the copies
  in `htmlSandbox.ts` or the app CSP are stale. After editing kit source run `npm run kit:build`;
  `scripts/canvas-kit.test.mjs` fails on a stale generated kit or one over its
  84 KB budget (flow routing/motion/viewport; 72 KB after the SVG instruments).
  Flow helpers in `src/canvas/kit/flowRoute.ts`, `src/canvas/kit/flowMotion.ts`
  and `src/canvas/kit/flowViewport.ts` keep routing bounded and pure, animate
  transforms/opacity with per-element visibility gates, and implement
  user-driven pan/zoom. Private kit members are named `_x`:
  the build mangles them (`mangleProps`), so they can't shadow an
  `HTMLElement` property. `src/components/artifactCsp.golden.json` pins every
  mode for both `artifacts.rs` and `childCsp` (`NONCE` stands for the
  document's nonce). Responses send
  `X-DNS-Prefetch-Control: off` and `Referrer-Policy: no-referrer`. srcdoc
  inherits the app CSP, so outside Tauri Canvas renders Offline and says so.
- **The `hollow-canvas` fence has one rule set in two places (Canvas C3).**
  `src/canvas/fence.ts` (`parseCanvasFence`: pending / drawing / ready /
  invalid, for the chat skeleton) and `canvas_fence.rs` (`validate`, which the
  canvas store calls before saving) are both tested against
  `src/canvas/fence.golden.json`; change a rule in all three. Root
  `<hc-canvas id title [ask] [height]>` (C6's `section` went with the curated views, COO rebuild L4), id `[a-z0-9-]{1,64}`, body ≤128 KB
  UTF-8 (measured after the streaming typing-fence strip), unknown root
  attributes ignored. `chatSegments.ts` makes it a `canvas` segment
  (`closed` says whether its fence ended); `ChatCanvas` renders it. Rules and
  status: CANVAS_C3.md.
- **Saved canvases: the sidecar is the commit point (Canvas C3).**
  `workspace_canvases.rs` keeps `outputs/canvases/<id>.canvas.html` (the fence
  body, a readable copy) and `<id>.canvas.json` (`hollowCanvas: 1`, `id`,
  `title`, `kit`, `session`, `fenceSha256`, `revision`, `state` ≤16 KB,
  `revisions` ≤30, last = current; `retired` ≤500, number and hash of
  aged-out revisions); every kept body lives in the hidden
  `.revisions/<id>/<sha256>.canvas.html`, which Hollow renders from. Outside a
  workspace, `<app data>/canvases/<sessionKey>/`, with pruned conversations'
  bodies remembered in `canvases/.pruned`. Saves are compare-and-swap
  on the sidecar's SHA-256 (`CANVAS_CHANGED:`). **Re-rendering never writes:**
  a kept body is `unchanged`, an aged-out one `retired`, a pruned
  conversation's body `CANVAS_RETIRED:`, all looked up before any folder is
  made. `record_canvas` is plumbing and `save_canvas_state` user-only, never
  app actions. No folder on the way may be a symlink. **In Results a canvas
  is one item** (`kind: canvas`, `canvas: {id, revision}`): the visible copy
  is what is listed, hashed and accepted (person only, exact bytes); the
  sidecar is never listed and can't be reviewed (`workspace_results::is_listed`),
  and `.revisions/` is skipped as a dot folder. A new revision rewrites the
  visible copy, so an accepted canvas reads `changed`. In the review it opens
  in Canvas mode with its version menu (`ResultCanvas`, C5b); an earlier
  revision is read-only and holds Accept, which records the current file.
  With no store record, or a visible copy edited outside Hollow, it opens the
  file in the HTML document view and says why. **The chat saves only live:** a canvas
  that becomes ready while its turn streams is recorded once
  (`resolveCanvas`, deduplicated per store and body, so the remount as the
  turn settles makes no second call); a transcript re-render, a reopened
  conversation and the Preview pane's Expand copy only call `read_canvas`.
  `retired` and `CANVAS_RETIRED:` render the fence itself, unsaved. The
  browser mock implements the store in `src/mockCanvases.ts`.
- **PLAN.md's checkbox is a person's one plan write (Canvas C5a P3; in the
  Files tab since COO rebuild L4).** A workspace's PLAN.md in the Preview
  pane's Markdown view has live boxes (`NoteRenderer`'s `tasks`): the nth box
  is the nth row of `planRowsInOrder` (`readPlan`), and only when the drawn
  boxes and the rows agree in number. A tick calls `toggle_plan_row`
  (`plan_rows.rs`, user-only, never a `hollow_app` action) with the row's
  `{line, sha256}` and the SHA-256 of the text the pane drew; under the vault
  mutate lock it flips that one state byte through
  `write_note_expecting_locked`, writes nothing for the state a row already
  has, and refuses with `PLAN_CHANGED:` when the file's revision or the line
  moved on. The pane then re-reads and a toast says so. Keep the Rust line
  rules (`find_row`, `box_offset`) in step with `sectionMarkdown.sourceLines`
  and `readPlan`'s row pattern.
- **Workspace activity is a timeline of existing records (Canvas C5b Q3).**
  `workspace_activity` (`workspace_activity.rs`, a read action) joins the
  workspace's conversations, each turn's receipt (`.evidence-receipts.json`)
  and usage row by run id, other runs' receipts that changed a workspace
  file, result events (a receipt's change under `outputs/`, a canvas
  sidecar's revisions, the review store's history, and a file no record
  explains: "Added/Changed, no kept run record names it" from its birth
  and modified times, never "outside a run", since the run's receipt may
  have aged out; `pending`, "Changed during a running turn", for a change
  since one of the workspace's turns started, because a turn's receipt is
  only written when it ends), the in-memory app build and the last install receipt
  when it names the workspace. **Nothing new is recorded**, so retention
  is the records' own (500 receipts per vault, usage 400 days, 50 review
  events per result, 30 canvas revisions, only the last install, builds
  only while Hollow runs); `ActivityGaps` counts what aged out or couldn't
  be read and the view says it (`resultsWithoutReceipt`: once the receipt
  store is full, files changed before its oldest kept receipt; an
  unreadable store is an error in words, `receiptsError` / `reviewsError` /
  `usageError`, never counted as records that aged out). Reading writes nothing: the review store is
  read with `read_store_peek` (a damaged one is reported, never moved).
  Agents and the phone read it; its History section view went with the
  workspace panel (COO rebuild L4). **Incremental and paged (C6 P3):** `list` keeps one workspace's
  reads (`Kept`) and re-reads only what changed (a conversation's turns by
  turn count and last activity, or while a turn runs; its runs' usage in one
  query, `usage_rows_for_sessions`; the receipt store by file stamp; other
  usage rows from `usage_changes_since` the ledger's `UsageStamp`); the
  timeline is rebuilt only when an input changed and draws a turn only when
  a page shows it. `cursor`/`nextCursor` page it by keyset on its own order;
  a re-read chains every opened page. A new
  event source belongs in `build_timeline` with a test, a gap if it has
  retention, and a change check in `refresh` so a re-read sees it.
  CANVAS_C5.md.
- **The main window loads no remote images** (`img-src 'self' data: blob:`):
  rendered markdown turns remote images into click-to-load placeholders
  (`src/remoteImages.ts`, `fetch_remote_image`); a new remote image must go
  through the same command.
- **The chat's working folder** defaults to the vault and is
  conversation-scoped; `chat_preflight` / `chat::accept_turn` refuse a missing
  folder (`CHAT_CONTEXT_RECOVERED`) and an unsafe one (`/`, `/tmp`, `/usr`,
  `/System`, `/Library`, bare home: `UNSAFE_WORKING_DIRECTORY`).
- **`WaitingLink` reads the review queue through `preserveDuringRefetch`**
  under its own `<Suspense>`, or every refetch blanks the page.
- **`npm test` needs a worker floor here:** run
  `VITEST_MIN_FORKS=1 VITEST_MIN_THREADS=1 npm test`; lower the floor, never
  raise the cap.
- **A candidate build's Keychain item prompts.** Hollow is self-signed, so a
  rebuilt candidate's first read of its item (`com.hollow.secrets.candidate`)
  raises a login-password prompt, and `secret_store` holds its lock while it
  waits: every chat turn hangs (2026-10-07). Recovery: stop the candidate, then
  `security delete-generic-password -s com.hollow.secrets.candidate -a bundle`
  (never `com.hollow.secrets`, the everyday item).
- **Repointing `vaultPath` at a temp dir does not sandbox a dev run**, and a
  dev build shares `localStorage` with every other `localhost:1420` run. Use
  the mock harness or isolated tests.
- Port 1420 is hard-coded for Vite ↔ Tauri. `.codex/` and `.claude/` are
  tool-side metadata directories, not edited by the app. Never write a secret
  to its own Keychain item; add a key to `secret_store.rs`. A secret may also be read from 1Password (`op://` reference, off by default); never log or cache what `op read` returns, and keep new secret keys behind `secret_store::get` so the reference path and redaction apply.
- **A smoke spec's `page.evaluate(async () => (await import(path)).x())` can
  fail with "Execution context was destroyed" with no navigation at all
  (2026-11-07).** The browser's own error is "Promise was collected": the
  awaited `import()` promise was garbage collected before it settled, which
  happens while the page is busy (a view just mounted and its chunks loading,
  a dialog opening) and Playwright words as a destroyed context. It cost
  `day-ring.spec.ts:110` about a third of its runs and `restart-work-guard`
  its "before save" case. Import the module while the page is quiet, keep it
  (`window.__x = await import(path)`), and call the kept module in later
  evaluates with no `import()` and nothing awaited; do not retry the evaluate.

---
