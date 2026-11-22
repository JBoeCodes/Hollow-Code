---
hollow_workspace: 1
title: "hollow"
status: active
created: 2026-09-24T21:24:35.121529+00:00
---

# hollow

## Outcome

Develop Hollow as a local workspace for projects, tasks, notes, recordings and Calendar. This vault workspace holds plans, sources, conversations and deliverables; application code stays in the connected repository. The original checkout at `/Users/<user>/Documents/Documents - <name>’s MacBook Pro/projects/personal/hollow` is intentionally stale and must remain untouched.

## Current context

**2026-10-05 09:01: Canvas wave 1 installed and verified; wave 2 running.** `08c4b83d` is installed (receipt
`20261005-090057-e6923357` ok, matchesRunning, fingerprint = bed's, journal healthy, rollback `61acf499`;
[verification](outputs/release-install-verification/08c4b83d.md)), recorded in [#351](https://github.com/<github-account>/Hollow/pull/351)
(`ed6d264`, source → 0.5.9). The first build was lost in a Hollow restart and rebuilt; nothing was installed
from it. Wave 2 (workflow `wf_0f53a1a9-86a`): lane C = C3 chat side (inline render, revisions + state, frame
budget + bed rows, "Visual answers in Hollow" prompt + Canvases setting + receipts), lane P = C5a Overview,
Brief, Plan (with a CAS row toggle). Paid probes (1 Claude + 1 Codex reply) are run by the orchestrator after the bed.

**2026-10-05: Canvas wave 1 merged and bed-tested.** C2 kit (#344–#346), C3
foundations (#347–#349: parser, `outputs/canvases/` store, canvas Results) and a **P0 repair found on the way**
(#350: hash-only `script-src` let agent HTML in default Offline request any URL via `<script integrity=sha256-…>`,
present since C0; now a per-document nonce, native red 8 → green 0). main `08c4b83`. Bed on `08c4b83d`
(fp `d14f4599…`): **40 pass / 13 skipped / 0 fail**. Everyday build started 00:58 EDT. Nothing renders
differently in chat yet: wave 2 does the chat side. [C2](outputs/html-canvas/c2/README.md) ·
[C3](outputs/html-canvas/c3/README.md) · [SRI](outputs/html-canvas/sri-repair/README.md). Baseline was verified first
(installed `61acf499`, main `ff8f548`, no orphaned turns; small plan line drift, recorded in C2's evidence).

**2026-10-04: Canvas C1 installed and verified (15:49:42 EDT), recorded in #343.** Receipt
`20261004-154937-f8c98e68` is ok and matchesRunning. The installed fingerprint `09b5b2f7…` equals the bed's.
The journal ends healthy with no relaunch; codesign passes; rollback is `3f333cd3`. Repo record
[#343](https://github.com/<github-account>/Hollow/pull/343) merged `ff8f5484` (source → 0.5.8; the running app is 0.5.7).
[Verification](outputs/release-install-verification/61acf499.md).

**2026-10-04: Canvas C1 merged (#339–#342, main `61acf499`).**
[Evidence](outputs/html-canvas/c1/README.md). #339 adds **Canvas** mode: scripts run, network denied, opaque
origin, no forms or popups. `artifacts.rs` now picks each mode's CSP (the frontend sends a mode, not a policy).
Prefetch is off. Outside Tauri, Canvas falls back to Offline. #340 adds bridge v1 (`src/canvasBridge.ts`:
sender frame, null origin, 64 KB cap, exact fields), plus host → child theme, so a theme switch restyles mounted
frames. #341 makes the quick-chat prompt match the sandbox. #342 extends the native probe. **Offline stays the
default**; scripts-by-default is a later slice. Each PR had a red test first, except #339, whose golden-table
test caught a canvas `img-src https:` before commit. Every merge used the local gate; GitHub jobs were refused
for billing. Bed on `27d03f2c` (fingerprint `09b5b2f7…`): **38 pass / 13 skipped / 0 fail**. Canvas made 0
attempts on all 14 vectors. The theme moved in the same document. Rollback was tried against `3f333cd3`. DNS
prefetch is skipped (not observable). The bound build `20261004-154806-ae9149e2` matches the fingerprint and
codesign passes. Installed and verified (entry above). C2 has not started.

**2026-10-04: Canvas C0 installed, verified and recorded.**
[Evidence](outputs/html-canvas/c0/README.md). #335 (`8133bad5`) closes scripted-link
confirmation bypass; #336 (`d7e8dc8b`) blocks WebRTC UDP and popup constructor recovery.
Each repair had a failing regression first. Native C0: 5 pass / 1 skipped / 0 fail;
full candidate bed: 34 pass / 12 skipped / 0 fail, including rollback/roll-forward.
Prefetch is unsupported; system-wide WebKit memory cannot be attributed. Thirty
frames mounted; ten text-card samples saved 18.4% tokens (charts/inputs unmeasured).
Harness #337 merged as `3f333cd3`; **installed 13:33:51 EDT and verified after restart**.
Receipt `20261004-133346-e4211f0d` is ok and matchesRunning; bound build
`20261004-133222-784aff03` and installed fingerprint match the bed. Journal ends
healthy with no corrective relaunch; codesign and embedding checks pass. Rollback
copy is `5f79d86a`. [Verification](outputs/release-install-verification/3f333cd3.md).
Repo record [#338](https://github.com/<github-account>/Hollow/pull/338) merged `339127cd` (source → 0.5.7) on the
full local gate; GitHub jobs were refused for billing. Running app stays 0.5.6. C1 has not started.
Latest direction: a canvas only drafts into
the composer. An independent clone preserves the original malformed-ref checkout.

**2026-10-04: HTML Canvas programme planned, now the top feature priority.** You asked for agents to answer with
beautiful, concise HTML canvases (interactive ones too, including for answering the agent's questions), and for the
panel sections to show canvases while the markdown stays the source of truth. [Plan](outputs/html-canvas/README.md),
[mock](outputs/html-canvas/preview.html), [session prompt](outputs/html-canvas/SESSION_PROMPT.md). The plan was checked against
`5f79d86a`, and no code changed. It found three gaps in today's HTML artifacts:
- the quick-chat prompt says scripts run, but the default mode blocks them;
- "Interactive" turns on scripts and network together;
- a scripted frame can open a URL it built after one click.

Slices C0–C6. **Decisions answered 2026-10-04** (see Decisions below). C0 native checks are complete; see the current entry and evidence above.

**2026-10-03 night: Results and Plan moved into the side panel, and the top bar's Conversation · Results · Plan switch
is gone (PR #333 `f11f2242`, plus a docs fix in #334 `5f79d86a`).** It passed the bed (29/11/0), and the install was
scheduled ~23:36 EDT. **Verified 2026-10-04:** receipt ok, matchesRunning, journal ends `healthy` with no relaunch, so #332's fix works ([evidence](outputs/release-install-verification/5f79d86a.md)). I had misread your card answer in
#331, which removed them from the side panel instead of the top bar. Now:
- the pills read Overview · Brief · Plan · Results · Sources · Activity, with the full Results (review/accept/compare)
  and Plan views inside them;
- Library, search, Overview and "waiting on you" links open the panel on that section.

The local gate passed (full smoke 457/0).

**2026-10-03 evening: `3b40f87e` (Workspace section pills) is installed and verified, 21:30 EDT.** The "build failed"
you saw was a false alarm. The installer's relaunch check compared `ps`'s whole-second start time with the
millisecond swap time, so it called the new app "from before the swap", quit it once, and reported a mismatch.
Fixed in PR #332 (`2991fe34`), which also records the install (version → 0.5.6).
[Evidence](outputs/release-install-verification/3b40f87e.md).

**2026-10-03 afternoon: the side panel's Workspace view uses section pills now (PR #331, merged `3b40f87e`).** The accordion is gone. A row of pills (Overview · Brief · Sources · Activity) keeps one section open,
and Results and Plan are only in the top-bar views. This was your choice on the question card. The local gate
passed, including the full smoke. On the bed it scored 29 pass / 11 skipped / 0 fail, and the native candidate window
shows the pills. Built and **scheduled to install** (~21:33 EDT); verify after the restart
([evidence](outputs/release-install-verification/3b40f87e.md)). [Build log](BUILD_LOG.md).

**2026-10-03: the merged plan's slices 2–6 are complete and installed. `d1eb295e` (waves 2 and 3, #296–#328) was verified
installed at 13:16 EDT:** receipt ok, matchesRunning with no manual reopen, installer journal complete, provenance =
the bed's, rollback copy `184a8600` ([evidence](outputs/release-install-verification/d1eb295e.md), repo record PR #330,
version → 0.5.5). Wave 3 merged 10 PRs, #319–#328: Wave 3 delivered:
- install approval bound to an immutable build, with the card showing commit, diffstat and files;
- broker proofs off argv, and peer-checked broker and approval sockets;
- a candidate-only bed driver, with a scripted journey and failing launch, typing and RSS budgets;
- TS types generated from Rust;
- CLAUDE.md cut from 856 to 117 KiB (history in `docs/history/`, plus a CI path check);
- decision 7's single "waiting on you" surface and "Results" naming;
- the wave-1 and wave-2 nits.

The hidden Chat and Imports routes stay, because both are still used. Every wave-3 merge used the local gate, since
CI was still refused for billing. Candidate `d1eb295e`: 30 pass / 10 skipped / 0 fail, including the journey and
wave 2's four checks; **D5b is proven** on the candidate. [Evidence](outputs/slice6-wave3/README.md).

**2026-10-03 early: wave 2 (slices 3 and 4) merged, 23 PRs #296–#318, main `0575b6a3`; not installed yet.**
GitHub Actions stopped starting jobs at ~22:55 (billing/spending limit). By your decision, #300–#318 merged on the
local gate, not CI. **Behaviour changes:**
- every agent gets the deadline and no-output watchdog (cards pause it);
- workflows no longer block workspace chats;
- concurrent runs share one budget reservation and fall back visibly at a ceiling;
- agent processes stop when Hollow quits or crashes;
- History shows what it can't restore;
- only a person accepts Results or resolves review items, proposals and the profile builder;
- writes fsync and use one writer contract with a written lock order;
- an exhaustive "needs you" query with exact-id reopening.

Candidate `0575b6a3`: 16 pass / 14 skipped / 0 fail. [Evidence](outputs/slice3-4-wave2/README.md).

**2026-10-02 night: wave 1 of slices 2–6 merged (18 PRs #277–#294) and installed as `184a8600`, verified 21:44 EDT** (receipt ok, matchesRunning without a manual reopen, provenance `9105c245f1d5…` = the bed's, codesign ok, rollback copy `30f3896e`; [install evidence](outputs/release-install-verification/184a8600.md), repo record PR #295, merged `a1d24817`, version → 0.5.4).
Slice 2 (permission parity) and slice 5 (integrations) are complete. Slice 6's installer, CI and hygiene
parts are done: pre-swap re-check and relaunch, installer journal, CI fmt/clippy/smoke/budget, toolchain
1.96.0, mockApi dev-only, one version source. **Behaviour changes:**
- auto services are bounded (decision 1);
- **auto is removed for Antigravity**, because its file tools escaped the sandbox in the probe matrix;
- Grok is sandboxed in plan/default/accept-edits, because headless it asked nothing;
- Grok/Antigravity chats say Hollow cards are absent;
- auto no longer preapproves library connectors;
- links can't navigate the main window.

Candidate `184a8600`: 16 pass / 10 skipped / 0 fail on the bed. [Evidence](outputs/slice2-5-6-wave1/README.md).

**2026-10-02: slice 1 and Track U of the merged plan are merged (main `d5c2ab14`) and installed, verified 14:35 EDT** (receipt ok, provenance `49c9902b9b2f…` = the bed-tested source, codesign ok, rollback copy `6dfe8806`; [install evidence](outputs/release-install-verification/d5c2ab14.md), repo record PR #274, merged `23d096a4`).
#265 delegation cap (a teammate runs at its parent's mode and keeps its tool limits, Antigravity
included), #267 protected paths on the destination, #268 the Codex-stall finalizer, #266 History
never trashes an uncaptured file, #272 remote images click-to-load, #270 redacted artifact reads,
#269 fail-closed cancel, #271 Track U (ultracode per chat; `--effort ultracode` works on Claude
Code 2.1.286, probes cost about $4.76 at list price), #273 the bed's teammate check. Every PR has a
regression test shown to fail without its fix, and CI is green. Behaviour change: under computer
control, an auto or bypass Claude chat's Codex and Antigravity teammates now run sandboxed, and
Grok teammates are refused until slice 2. [Evidence and test-bed results](outputs/slice1-repairs/README.md). Follow-up #275 moved Ultracode into the model menu's Effort list ("this chat"); installed as `30f3896e`, verified 15:09 EDT after a manual reopen, because a Hollow launched mid-install kept the old binary ([evidence](outputs/release-install-verification/30f3896e.md)).

**2026-10-02: step 0 of the merged plan done (test bed and rollback).** Four PRs merged: #259
`db736cb2` (the installer keeps the previous verified app; one-step `--rollback`), #260 `aae5083d`
and #261 `22bf9052` (two installer bugs the bed found: Spotlight still listing the kept backup,
and `pgrep` not seeing a Hollow that is its ancestor), and #262 `b3ff868b`. In #262, a candidate
(`Hollow Candidate.app`, `com.<user>.hollow.candidate`) gets every Hollow store from its bundle
identifier. It refuses the everyday vault and self-install, and starts no background agent
unless asked. `scripts/test-bed.mjs` drives the bed in `~/Hollow-candidate`. Candidate
`b3ff868b` passed 10/10 automated checks and window-opens beside the running everyday app. The
rollback was tried on the bed with real signed bundles. `refuses-everyday-vault` is skipped
(the run's permission check refused that launch). [Evidence](outputs/step0-test-bed/README.md).
**Installed `6dfe8806` (= `b3ff868b` app code + #263's installer wait), verified 2026-10-02 12:46 EDT:** receipt ok, provenance `960e9d6e…` clean, codesign ok, and `/Applications/.Hollow.app.previous` keeps `3476ad61` for `--rollback` ([evidence](outputs/release-install-verification/6dfe8806.md), repo PR #264, merged `417ee64c`). The first attempt failed safely; #263 fixed why.

**2026-10-02: three reviews merged into one plan.** You pasted three reviews and asked for one plan. They are review A (six-lane), review B (Meter) and a "Grok plan" that is word for word this workspace's own [review](outputs/adversarial-review-2026-10-02.md), so only two are independent. The Notion page "Hollow next steps" is now readable; its ~60 KB snapshot is in [sources](sources/notion-hollow-next-steps-2026-10-02.md). Read-only agents checked the claims against `411c0cea`. Nearly all were confirmed, including A's merge cadence (148 PRs in 10 days; #245 merged 2 min after opening with 0 reviews), which B had not verified. **Revision 2** takes ChatGPT's critique (ChatGPT wrote review B): a test bed and rollback come first (step 0; the candidate needs its own bundle id), then narrow urgent repairs. Ultracode moves to a parallel track. Deny lists accumulate. Auto/services becomes a gating decision. Result: [merged plan](outputs/merged-plan-2026-10-02/README.md) (step 0, six slices, Track U) and [verification](outputs/merged-plan-2026-10-02/verification.md). The earlier request to append the review to that Notion page is still not done; nothing has been written to Notion.

**2026-10-02: adversarial review, plan only.** #257 and #258 use the same mode and effort names on every agent. They do not make the guarantees match, ultracode cannot be selected, and neither PR is installed (`3476ad61` is still the app). Antigravity and Grok never hit Hollow's confirm broker. An Antigravity teammate always starts at bypass. Full write-up: [adversarial review](outputs/adversarial-review-2026-10-02.md). No code was changed.

**2026-10-02: auto and bypass mean the same in every agent; effort works with every model.** PR #257 (merged
`88a9a3c1`): bypass shows no Hollow card (app actions, computer tier 2); auto asks only before leaving this
Mac, deleting, secrets/accounts or giving agents more power; Codex auto uses its own auto-reviewer; Antigravity
auto runs every tool in its sandbox (it denied every command before); auto/bypass preapprove computer
control and services; teammates inherit the chat's auto/bypass. PR #258 (merged `411c0cea`): one effort list for every agent,
mapped per model. Not built or installed yet (installed is still `3476ad61`).

**2026-10-01 night: Durability & trust slice merged (D0–D5) and installed.** main had not
moved from `180ce6f1` when it started. Merged in order: #250 install records (`0c9be0e8`), #251 board
continuity (`9b236f8f`: board `id`, first edit in a workspace chat saves to `outputs/boards/`, an agent's
re-emitted board rebases your layout), #252 Enter places a suggestion after a reply that read web,
browser or connector content (`f583f282`), #253 Overview shows when agents get a cut brief/plan
(`9a70b17a`), #254 numbered recording headings (`e193d7f0`), #255 D5 evidence (docs, `3476ad61`). D5: cancel in
the installed app left nothing ([evidence](outputs/durability-slice/D5-supervisor-check.md)); the
deadline case needs you present. Built `3476ad61` clean (198 s). **Installed 23:59:44 EDT, verified after restart:** receipt ok=true and matchesRunning, installed provenance `3476ad61` clean with fingerprint `81b4ff3a…`, codesign ok ([evidence](outputs/release-install-verification/3476ad61.md)); repo record PR #256, merged `76db9a15`.

**2026-10-01 late: composer suggestions for every reply (PR #249, merged `180ce6f1`).** The agent ends each reply
with a hidden `hollow-next` block of likely next messages; the empty composer offers the
first (Enter sends, Tab edits, Esc dismisses), others as links; merge/install offers remain
the fallback. Built clean (185 s, codesign ok) 22:35 EDT. **Installed 22:42:24 EDT**, verified: receipt ok=true,
running app reports `180ce6f1` clean, fingerprint `842414eb…`, codesign ok. [Evidence](outputs/release-install-verification/180ce6f1.md).
Native check per provider still open.

**2026-10-01 late: connectors popover (PR #247 `e178cbdf`) and composer ship suggestions + failed-check wording (PR #248 `273c5543`) merged and installed.**
Popover rows stay inside it and name their source. Merge/install offers are now the empty composer's suggested next step (Enter sends),
including "Build and install it" after a merge. Failed claude.ai checks read as not checked, not red Unavailable.
**Installed `273c5543` 22:07:54 EDT**, verified: receipt ok=true, running app reports `273c5543` clean, fingerprint `8a6738cb…`, codesign ok. [Evidence](outputs/release-install-verification/273c5543.md).

**2026-10-01 evening: next-level programme merged and installed.** PR #245 (P00–P21)
merged as `0f859d4c` on green CI, built clean in Hollow (189 s) and **installed 18:49:10 EDT**:
receipt `ok=true`, installed provenance `0f859d4c` clean with the candidate's fingerprint
`ffcb9b44…`, `codesign --verify` passes. [Evidence](outputs/release-install-verification/0f859d4c.md).
The install is recorded in the repo (PR #246, merged `fc3f290e`, docs only). Cleanup removed
the six next-level worktrees, their branches and the remote branch. Native-app and live-service
checks are still open; P14 and P21 have gaps. [Handoff](outputs/hollow-next-level/HANDOFF.md).

**2026-10-01: active-session icon merged and installed.** PR #244 merged as
`63c79a88`; the persistent installer receipt reports success at 14:44:47 EDT,
and installed provenance and the clean release checkout match. [Verification](outputs/release-install-verification/63c79a88.md).
The 14px line doorway traces its edge and folds into each session's provider
mark; app/system reduced motion keeps a still doorway. Existing provider
marks remain approximations, not exact model logos.

Repository session `s-20261001-deda018c` is finished. Its recorded validation
includes 652 unit tests, 16 Chromium rail smoke tests, TypeScript/CSS checks,
Chromium/WebKit motion samples and both CI jobs passing. The native build
took 194 seconds. Native visual and non-Claude running-fixture checks remain
open. The outcome session verified the installed identity and receipt directly;
it did not rerun release tests or reinstall. [Patch history](outputs/session-icon-morph/README.md).

**2026-10-01: full product pass and proposed next programme.** The user requested sweeping changes across every feature and UI/UX. The [programme](outputs/hollow-next-level/README.md) covers 22 work packages, with [source audit](outputs/hollow-next-level/audit.md), [backlog](outputs/hollow-next-level/backlog.csv) and [verification contract](outputs/hollow-next-level/verification.md). This is a plan; application code has not been changed by this audit.

**Current installed baseline:** `61acf499fab42bff5a78ccdb6f8b1210c1c403e9`,
clean, 0.5.7 (rollback copy `3f333cd3`, verified 2026-10-04), built from
`/Users/<user>/Hollow/Worktrees/hollow-release/canvas-c1-20261004`.
The full-product audit below remains anchored to historical `7a325ac2`.
The install-result action returns null once the restart notice is dismissed
(by design; `build.self_install_receipt` is the record). Installer signature checks passed; the earlier standalone
trust error was not reinvestigated. [Latest evidence](outputs/release-install-verification/63c79a88.md).

**Audit findings:** file/result discovery is split across multiple views; manual editor conflict recovery and content-keyed whiteboard overlays limit durable work; connected-project inventory caps affect coverage; Calendar and recordings lack integrated daily workflows. A synthetic call reproduced an enabled connector with no probe result displaying a healthy “can use it” status. Existing capabilities include quick chats, importers, editable boards, task Board/Week, browser annotation, run receipts, recovery and connector diagnostics; extend these rather than rebuilding them.

**2026-10-01 late: Gemini adversarial review, checked.** [Gemini's plan](outputs/adversarial-review-and-feature-plan.md) mostly describes pre-#245 code (clipboard conflicts, 2,500-file cap, unprobed-green connectors, no process groups are all stale at `180ce6f1`). Still valid: board edits stay in app data, keyed by fence hash, unless saved to the workspace, and PLAN.md (~35 KB) is trimmed in the envelope. A second Gemini review, grounded in the current source, was checked the same day (same file). [Verification](outputs/adversarial-review-verification.md); plan from that table, not the original.

**Full-product audit verification boundary:** source reviewed across active product areas and five historical mock captures inspected. The sandbox blocked a fresh preview server and Playwright CLI retrieval. Current native UI, live connector behavior and runtime performance were not exercised. Test counts in older entries are historical, not this audit's results.

**Next:**
- C1 is installed and recorded; its by-hand native checks are in PLAN.md. C2–C6 run from the [session prompt](outputs/html-canvas/C2-C6_SESSION_PROMPT.md) (four waves, two lanes, bed + install per wave) ([C1 evidence](outputs/html-canvas/c1/README.md));
- native checks of the new surfaces in the everyday app (PLAN.md lists them);
- port the last 10 bed checks to the driver;
- the wave-3 reviewer follow-ups;
- decision 4 (first colleague and task) before the features list.

CI is advisory since #329; merges use the local gate.

**Run the durability native checks on the installed `3476ad61`** (PLAN.md: D1 board save/rebase and
id retention, D2 Enter after outside content, D3 the cut-plan line) and the D5 deadline check with you present.
The install is verified and recorded in the repo (PR #256, merged `76db9a15`). PLAN.md was archived on 2026-10-01 ([full copy](outputs/plan-archive/PLAN-2026-09-26-to-10-01.md)) and holds only open work.

Run the native-app checks on the installed build (browser pane under dialogs/palette,
freed global shortcuts, traffic-light inset, VoiceOver, glass contrast, launch time and memory)
and the live-service checks in the [handoff](outputs/hollow-next-level/HANDOFF.md), then
finish the P14 and P21 gaps.

A [paste-ready implementation prompt](outputs/hollow-next-level/IMPLEMENTATION_PROMPT.md) is prepared for a new session to tackle all P00–P21 packages. It requires refreshing the newer baseline and producing a tested local candidate; this prompt-writing turn did not start implementation or authorize a release.

The session-icon release is complete; do not repeat its merge or installation.
Remaining follow-ups are native visual checks and the install-result reporting
mismatch, tracked in [PLAN.md](PLAN.md).

Review the proposed programme, then begin the first implementation slice when requested: establish truthful baseline/status (P00), review six core visual compositions (P01), and implement workspace re-entry/continuity plus a thin Results view and file-conflict recovery (P02/P03/P05/P08). Native and accessibility verification accompany each release.

Continue tracking existing native checks in [PLAN.md](PLAN.md): shortcuts/menu/global keys, Library, quick chats/imports, browser/annotation, concurrent sessions and permission-card behavior. The new plan does not authorize publishing, merging, installing, deleting or spending.

## Decisions and constraints

- **HTML Canvas decisions (2026-10-04, user, question card):**
  - **1 · Frequency:** *When it helps* (the default; overridable per chat).
  - **2 · Panel sections:** Hollow draws them natively from the markdown, **plus** the optional agent-written curated layer (C6, hash-bound, hidden when stale).
  - **3 · Ask-canvas answers (superseded by latest implementation request, 2026-10-04):** a canvas never sends a message; `draft` only fills the composer. This replaces the earlier "send straight from the canvas" answer and the proposed host Send button.
  - **4 · Network:** never by default; Interactive stays an explicit per-canvas switch.
  - **5 · Saving:** every workspace canvas is saved to `outputs/canvases/` with up to 30 revisions.
- **2026-10-03, user: "stop running as many CI, I'm not too worried about it."** Decision 2 is revised: the **local gate is the merge standard** (the checks listed below, run on the rebased head), and CI is a cheap second look, not a gate. CI runs only on PRs, never on the push to main, skips docs-only changes, and is split by path: `ci.yml` (frontend, macOS, under 1 min), `smoke.yml` (Linux), `rust.yml` (macOS, Rust only). The pre-release lane runs by hand only. Applied in #329 (`aeda52e6`). A red CI run on a PR is still worth a look, but it doesn't block a merge.
- **2026-10-02 ~23:00, user (wave 2 card):**
  - **Decision 2, temporary exception:** GitHub Actions stopped starting jobs ("recent account payments have failed or your spending limit needs to be increased"). Until CI runs again, the agent merges on a **local gate**: the rebased head is re-run in a toolchain slot before merging (fmt --check, clippy -D warnings, `cargo test --lib`, vitest, typecheck, test:scripts, plan:check, frontend build, and the Playwright smoke when frontend files changed). The PR body says CI was unavailable. Once billing is fixed, CI must pass on main again, and any red result gets a fix PR.
  - **Decision 3 extended:** only a person resolves review items, commitment and learning proposals, and the profile builder, as well as Results. Agents may propose. Agents lose the resolve/accept actions in every mode; Bypass still shows no cards, the actions are simply not callable by agents.
- **Decisions 2, 3, 5, 6, 7 (2026-10-02, user, answered together for the slices 2–6 run):**
  - **2 · Merge cadence:** as now. The agent merges every PR on green CI, whatever its size. Installs stay gated by the test bed.
  - **3 · Result acceptance:** only a person accepts a Result. Agents and automations may mark one ready for review. Acceptance records actor, run, mode and the exact bytes.
  - **5 · History budget:** keep 512 MB / 200 runs / 90 days and 2 MB per file. A file History can't capture shows as not recoverable, and a run containing one never claims to be fully recoverable.
  - **6 · Budget ceilings:** allow the fallback. When a hard-stop ceiling is reached, work may continue on another account or agent with room, instead of stopping. Concurrent runs still share one reservation, so the ceiling is measured honestly and the switch is visible.
  - **7 · Surfaces:** keep Jev, and keep Grok and Antigravity as agents. Move to one "waiting on you" surface, replacing Activity, Needs you and the five review inboxes. Call outputs "Results" everywhere. Remove the hidden old Chat and Imports pages.
- **Decision 1 (2026-10-02, user):** Auto runs bounded services (worktree-confined cwd, ends with its chat or an explicit stop, command on the receipt, grant tied to the run); unrestricted services are Bypass-only. Bypass still asks nothing. This revises part of the 2026-10-02 "auto approves everything" wording; implemented in slice 2.
- Keep the doorway identity, shared token system, compact menus, matched chat/document typography and slim collapsed rail. The proposed design improves composition and flow across existing features.
- Proposed navigation keeps Work, Tasks, Library and Automations; Agenda belongs within Tasks, Recordings within Library, and Results within a workspace. These recommendations are not yet implemented.
- Local files stay canonical. Preserve source snapshots, stable citation paths, task idempotency/sync locks, conflict protection and provider-neutral execution. Only shared connectors may access external services; confirm external effects. No built-in messaging, retired message-folder ingestion or message approvals.
- No candidate reaches the everyday app before it passes on the test bed (`scripts/test-bed.mjs`, `~/Hollow-candidate`); a skipped check is not a pass. A candidate runs no background agent unless launched with `HOLLOW_CANDIDATE_BACKGROUND=1` (2026-10-02, so the bed never bills unprompted).
- A scheduled self-install runs after its requesting turn ends. Verify receipt and installed provenance after restart; do not call a scheduled action completed.
- Do not revive old roadmaps based on historical unchecked rows. Current source and newer user direction outrank old programme text.

## History and references

The earlier long brief is preserved unchanged in [pre-audit workspace history](outputs/hollow-next-level/archive/WORKSPACE-before-audit.md). It contains historical status snapshots and decisions, not current install evidence or new authorization. [BUILD_LOG.md](BUILD_LOG.md) records the narrative; [PLAN.md](PLAN.md) retains detailed unfinished checks.

Earlier programme: [next programme](outputs/next-programme-plan.md). Release reconciliation: [report](outputs/hollow-release-reconciliation.md). Install evidence: [records](outputs/release-install-verification/README.md). Attached source provenance: [SOURCES.md](SOURCES.md).
