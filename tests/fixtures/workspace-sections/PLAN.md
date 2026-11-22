# Plan

Open work only. The full history up to 2026-10-01 (every PR, build and install row) is kept
verbatim in [plan archive](outputs/plan-archive/PLAN-2026-09-26-to-10-01.md). Keep this file
under ~15 KB so agents receive all of it. Move finished sections to `outputs/plan-archive/` (latest: the Durability & trust slice, 2026-10-03; the 2026-10-01 "closed while archiving" list moved there 2026-10-04).

## HTML Canvas programme (top feature priority, planned 2026-10-04)

[Plan](outputs/html-canvas/README.md) · [mock](outputs/html-canvas/preview.html) · [session prompt](outputs/html-canvas/SESSION_PROMPT.md). Original plan baseline `5f79d86a`; C0 and C1 results below, C2–C6 still planned.
Agents answer with sandboxed HTML canvases (a host-owned kit, no network), ask-canvases fill the composer for you to send,
and the six panel sections render as canvases from the markdown, which stays the source of truth.

- [x] Decisions 1–5 answered 2026-10-04: when it helps · native panels + curated layer · draft into the composer (latest implementation instruction) · no network by default · save every canvas (WORKSPACE.md)
- [x] C0 implementation: #337 harness merged (`3f333cd3`); native spike complete; #335 scripted-link confirmation and #336 WebRTC/popup repair merged. Native C0 5 pass / 1 skipped / 0 fail; full bed 34 / 12 / 0, including rollback/roll-forward. Ten text cards: 18.4% token savings; 30 frames mounted, WebKit memory unattributed. Signed build passed; installation tracked below. [Evidence](outputs/html-canvas/c0/README.md).
- [x] C0 installation: verified `3f333cd3`, receipt `20261004-133346-e4211f0d` ok / matchesRunning, bound build `20261004-133222-784aff03`, installed fingerprint = bed, journal healthy without corrective relaunch; rollback `5f79d86a`. [Evidence](outputs/release-install-verification/3f333cd3.md).
- [x] C0 repository release record: [#338](https://github.com/<github-account>/Hollow/pull/338) merged `339127cd`, source version → 0.5.7; full local gate passed (Rust 2,070 / 12 ignored, unit 998, scripts 129). CI jobs refused for billing. Supersedes the unpublished `5f79d86a` bookkeeping. Running app remains 0.5.6.
- [x] C1 canvas sandbox mode + bridge v1: #339 `94f5376` (canvas mode, Rust-served CSP, prefetch off), #340 `594ef3f` (bridge v1, live theme), #341 `c53c25b` (prompt), #342 `61acf49` (native probe). Local gate each PR (CI refused for billing). Bed 38 pass / 13 skipped / 0 fail on `27d03f2c` fp `09b5b2f7…` (DNS prefetch skipped). Offline stays default. [Evidence](outputs/html-canvas/c1/README.md)
- [x] C1 installation: verified `61acf499` 15:49:42 EDT, receipt `20261004-154937-f8c98e68` ok / matchesRunning, bound build `20261004-154806-ae9149e2`, installed fingerprint = bed, journal healthy, no relaunch, codesign ok; rollback `3f333cd3`. Repo record [#343](https://github.com/<github-account>/Hollow/pull/343) merged `ff8f5484` (source → 0.5.8). [Evidence](outputs/release-install-verification/61acf499.md)
- [ ] C1 native checks by hand: Canvas mode via Settings/custom/glass theme paths (bed flipped `data-theme` only); a real chart canvas in Canvas mode
- [ ] C2–C6 in one programme run: [session prompt](outputs/html-canvas/C2-C6_SESSION_PROMPT.md) (4 waves × 2 lanes, bed + install per wave). **Started 2026-10-04 20:45 EDT** on main `ff8f548`. Wave 1 merged (#344–#350), installed `08c4b83d`, recorded #351 (main `ed6d264`). Wave 2 running. Helpers: `~/Hollow/Worktrees/hollow-release/c2c6-tools/` (dev-server lock, smoke halves, gate logs); lane worktrees `c2c6-lane-a/-b`, bed worktree `c2c6-bed`
- [x] C2 Hollow Canvas Kit v1: #344 `e3a4a31b`, #345 `45866aaf`, #346 `23cbdf0c` (kit 38.7 of 40 KB, API frozen: plain-text data rows; tokens −86% vs raw HTML on 11 real samples). [Evidence](outputs/html-canvas/c2/README.md)
- [x] C3 foundations (wave 1): #347 parser `0bd78f71`, #348 store `d60820e7`, #349 canvas Results `cd9b44f1`. [Evidence](outputs/html-canvas/c3/README.md)
- [x] P0 SRI repair found in wave 1: hash-only `script-src` let `<script src integrity=sha256-…>` request any URL from Offline (since C0). #350 `08c4b83` serves a per-document nonce; native red 8 → green 0. [Evidence](outputs/html-canvas/sri-repair/README.md)
- [x] Wave 1 bed: candidate `08c4b83d` fp `d14f4599…` **40 pass / 13 skipped / 0 fail** (rollback ↔ `cd9b44f1` tried). Everyday build `20261005-010324-b07eec68` done 01:03 EDT (279 s; build workspace `canvas-w1-release-fbcc9b38b6fe13e4`): clean `08c4b83d`, source fingerprint = the bed's, bundle `42dd9d2a…`, codesign ok
- [x] Wave 1 install: **verified** `08c4b83d` 09:01:02 EDT, receipt `20261005-090057-e6923357` ok / matchesRunning, bound build `20261005-090021-5bc10150` (rebuilt: the 01:03 build's state was lost in a Hollow restart), installed fp = bed, journal healthy, no relaunch, codesign ok; rollback `61acf499`. Repo record [#351](https://github.com/<github-account>/Hollow/pull/351) merged `ed6d264` (source → 0.5.9). [Evidence](outputs/release-install-verification/08c4b83d.md)
- [ ] Follow-up: persist a finished build's state across Hollow restarts (the install refused a finished build after a restart)
- [ ] Wave 2: C3 chat canvases (inline render, revisions, bridge `state`, frame budget, prompt, setting, receipts) + C5a panel views (Overview, Plan, Brief)
- [ ] C4 ask-canvases (wave 3)
- [ ] C5 panel canvases (Overview, Plan, Brief, then Results, Sources, Activity) · [ ] C6 curated layer + polish

## Merged plan from three reviews (2026-10-02)

[Merged plan](outputs/merged-plan-2026-10-02/README.md) · [verification at `411c0cea`](outputs/merged-plan-2026-10-02/verification.md) · [session prompt](outputs/merged-plan-2026-10-02/SESSION_PROMPT.md) (resumable; one stage per session, Opus 5.5 1M at max effort, auto) · **[parallel prompt for slices 2–6](outputs/merged-plan-2026-10-02/PARALLEL_SESSION_PROMPT.md)** (three waves of lanes, 2 toolchain slots, a bed candidate and install per wave; decisions 2/3/5/6/7 answered and built in).
Built from review A (six-lane), review B (Meter, by ChatGPT) and C (the pasted "Grok plan", identical to
[this workspace's review](outputs/adversarial-review-2026-10-02.md)). Keeps the 2026-10-02 decision: bypass asks nothing, auto asks only for the sensitive list.
#257 (`88a9a3c1`) and #258 (`411c0cea`) are merged, not installed; installing them does not close the permission work. Revision 2 follows ChatGPT's critique.

- [ ] Slice 1 native checks on the candidate window (manual; this run couldn't click): `me//Guardrails.md` refused, stalled Codex turn has a receipt, 2 MiB → small restore leaves the file, remote image makes no request until clicked
- [ ] Each slice's test-bed checks are listed in the merged plan; record pass/fail/skipped per candidate
- [ ] Decision 4 (first colleague and task, PDF or workbook first): not needed until the features after slice 6
- Finished rows (step 0, slice 1, Track U, waves 1–3, slices 2–6, decisions): [archive](outputs/plan-archive/PLAN-merged-plan-2026-10-02-to-10-03.md)

## Native checks on the installed app

- [x] `3b40f87e` (PR #331, Workspace section pills) **installed and verified 21:30 EDT** (receipt `20261003-213008-c1956811` ok, matchesRunning, bound build, fingerprint `1a3c3a4f…` = the bed's, codesign ok, `.previous` = `d1eb295e`; [evidence](outputs/release-install-verification/3b40f87e.md)). The "failed" notice was a false `running-mismatch` from a whole-second time comparison. Fixed, and the install recorded, in PR #332 (`2991fe34`, version → 0.5.6)
- [x] `5f79d86a` (PR #333 + #332's installer fix): bed 29 pass / 11 skipped / 0 fail, candidate window shows no top switch and six pills ([evidence](outputs/release-install-verification/5f79d86a.md)). **Installed and verified** (23:33 EDT, receipt `20261003-233256-6f24ce5f` ok, matchesRunning, journal ends `healthy`, no relaunch, `.previous` = `3b40f87e`, codesign ok). Prior pending repo bookkeeping superseded by the C0 release record
- [ ] Pills, natively: light mode and the glass theme, six pills wrapping in a narrow panel, Results accept/compare and Plan inside the panel, Library/search/"waiting on you" links opening the right section
- [x] Next install ends `healthy` with no relaunch: confirmed on `5f79d86a` (#332's fix works)

Installed: `d1eb295e` (verified 2026-10-03 13:16 EDT; waves 2 and 3). New to check natively: the "waiting on you" panel (top-bar bell; replaces Activity, Needs you and the review inboxes), "Results" naming, the watchdog settings for every agent (Settings → Agents), Antigravity's "Auto (unavailable)", the "no Hollow cards" notice in a Grok/Antigravity chat, History's "not fully recoverable" box, the next install card showing commit and diffstat. From `30f3896e`: Ultracode at the bottom of Model menu → Effort, the receipt's "Effort" line, remote-image placeholders, teammate notes. The durability slice came in with `3476ad61`. Each item was built and tested in CI or browser; none has been looked at in the native app.

- [ ] Durability slice (installed in `3476ad61`): D1 edit a board in a workspace chat → file in `outputs/boards/`; ask the agent to revise it → layout kept, notice shows; real Claude/Codex replies keep board and element ids. D2 after a reply that used the browser or a connector, Enter fills the box. D3 the cut-plan line in Overview. Native sessions restart once (prompt changed)

- [ ] Composer suggestions after real Claude and Codex replies; nothing leaks into the transcript or session record; connectors popover
- [ ] Next-level programme: browser pane under dialogs/palette, freed global shortcuts, traffic-light inset, VoiceOver, glass contrast, launch time and memory ([handoff](outputs/hollow-next-level/HANDOFF.md))
- [ ] Keyboard shortcuts: remap shows in the menu bar; off switch frees ⌘⇧T; recording ⌘N doesn't open a conversation
- [ ] Library layout
- [ ] Quick chats and imports: row drag in WKWebView, tray/File → New Quick Chat, ⌘N from Home, a real ChatGPT and claude.ai export, continuing an adopted conversation, Downloads-access prompt
- [ ] In-app browser and Annotate: placement under the title bar, hiding under the palette, a real site, snapshot and element names
- [ ] Session icon and doorway: light/dark, glass/custom themes, a running non-Claude session
- [ ] Session-recovery UI; product-quality list (`outputs/product-quality-native-test-list.md`); Recap/Overview; older B1–B7/M5 threads

## Live-service checks

- [ ] claude.ai connector checks under two logins; M365 "Permission missing"; Plaud reads; scheduled automation delivery; moved import's next turn
- [ ] `hollow_app` checklist remainder: 5, 9, 10 re-run with Deny, 13–28 (`outputs/hollow-app-checklist-results.md`)
- [ ] Codex: a confirm card waits past 60 s from a Codex chat (open question 4)

## Waiting on the user

- [ ] Everyday `--rollback` + roll-forward, deferred by you 2026-10-02 (do it when convenient, not a slice-1 blocker): quit Hollow, then in Terminal from the release checkout `node scripts/install-macos-app.mjs --rollback` (→ `d5c2ab14`), open it, check, quit, `--rollback` again (→ `30f3896e`)
- [ ] Connect Notion/M365/Superhuman/Wispr/Zoom on claude.ai as `<email>` (<org-account>), then Settings → Connectors → Check again. Notion and M365 also have Hollow's own built-in connectors (#241)
- [ ] Microsoft 365 built-in sign-in (IT approval for Files.Read.All + Sites.Read.All, or "Sign in for calendar only")
- [ ] OK to close superseded PRs #178–#183 and delete `conductor-style-chat-display` + the kyoto worktree (landed as `e07c625` via #169)
- [ ] OK to drop `git stash@{0}` (an accidental pop of the Jev stash) once the Jev owner confirms
- [ ] Go/no-go: side-panel terminal (P3); exact model logos (P4: logo source/policy); "Live now" card when an agent opens its dev server

## Other open follow-ups

- [x] CI cost cut per your call (2026-10-03): #329 `aeda52e6`, CI is PR-only, path-filtered, smoke on Linux, no main-push runs; the local gate is the merge standard ([why](outputs/ci-billing/README.md)). Optional when billing is back: one manual run of each workflow to see the Linux smoke go green
- [ ] Wave-3 reviewer nits: [list](outputs/slice6-wave3/README.md#reviewer-findings-not-fixed-follow-ups); remaining bed checks to port to the driver (slice 1/2 GUI checks); "Failed" receipt label possibly in success colour (see wave-3 bed)

- [x] Flakes `run_permit::…` and `run_pump::queue_ownership…`: root cause fixed in #310 (flock released explicitly; stress 1,560/10,000 failures → 0); not seen since
- [ ] Candidate window says "Hollow" in the rail; only the menu-bar name tells it apart (optional badge)
- [ ] P14 gaps: board snapping/routing, detail/source-label editing, system-clipboard paste (legacy overlay retention moves to D1)
- [ ] P21 gaps: Rust search timings on a real 10,000-file vault, native memory, controller extraction from Work/ChatBody
- [ ] Standalone codesign trust error ([evidence](outputs/release-install-verification/63c79a88.md), [earlier](outputs/release-install-verification/7a325ac2.md)). The install-result null row is closed: by design after the notice is dismissed ([184a8600](outputs/release-install-verification/184a8600.md))
- [ ] Fresh browser/native visual pass of the six core compositions (P01)
