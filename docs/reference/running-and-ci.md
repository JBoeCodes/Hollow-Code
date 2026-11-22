# Running, testing and CI

Part of the agent manual. [AGENTS.md](../../AGENTS.md) is the entry point and
links every reference doc; this one holds the command table, smoke timing, CI
and the pre-release lane from CLAUDE.md's "Running the app", moved there word
for word on 2026-10-13 (only relative links were re-based). Read it when you
need a command beyond the few AGENTS.md lists, what `npm run gate` runs, how
the Playwright smoke behaves under load, or which CI workflow runs when. Keep
it current with the code in the same change, as AGENTS.md asks.

This continues CLAUDE.md's `## Running the app`; its toolchain, Rust hygiene
and `tauri dev` paragraphs, its short command list and "Gate, merge and ship,
in short" moved to the end of this doc on 2026-11-21, under [Running the app](#running-the-app).
[AGENTS.md](../../AGENTS.md#running-the-app) keeps the binding summary.

| Command | What it does |
|---|---|
| `npx tauri dev` | Vite on `localhost:1420` + the Rust shell: the real end-to-end run |
| `npm run tauri:rebuild` | Build, install, verify and clean up the production `.app` (the normal ship path; see Build & ship) |
| `npm run build` | Frontend bundle (provenance, Canvas Kit build, sandbox CSP hashes, `css:check`, Vite, then the no-mock check) |
| `npm run kit:build` | Rebuild the Canvas Kit and the ask kit (`src/canvas/kit/` → `generated/`) and re-sync the runtime and kit hashes; commit the generated files |
| `npm run build:mock` | The same bundle with `?mock=1` kept, for `playwright.static.config.ts` and `scripts/perf-mock-budgets.mjs`; never install it |
| `npm run bundle:budget` | Raw-size caps on built assets (every CSS file 344 KB, `App-*.js` 340 KB, `MarkdownEditor-*.js` 600 KB, the lazy `MissionControl-*.js` 26 KB, every bundled image 1 MB, the rail's `hollow-mark-96-*.png` 16 KB) and the **launch set** (index + App + App's preload group + their static imports, JS + CSS) 800 KB; a cap that matches no asset fails too. Keep headroom: a sheet that isn't on screen at launch goes behind `OnceOpened`. Bundle images are derived copies (`hollow-doorway-landscape.jpg`, `hollow-mark-96.png`); the PNG masters next to them aren't bundled |
| `node scripts/perf-report.mjs [--modules]` | Five asks B4's repeatable performance report (JSON on stdout, nothing written): every chunk raw + gzip, totals per kind, the launch set and the images its JS names; `--modules` adds an in-memory Vite build's per-module attribution of the launch chunks (a toolchain slot). The browser half is `scripts/perf-mock-budgets.mjs` (shell, long-thread open, palette open, warm nav, typing just after open and settled, search, Tasks, heap; needs `build:mock` + one `vite preview`) |
| `npm run -s perf [-- --skip-build --skip-rust --skip-browser --out f.json --port N]` | Overnight 10-11/12 P1: the whole baseline in one JSON (`scripts/perf-baseline.mjs`): `build` + `perf-report.mjs` (bundle), `build:mock` + one `vite preview` on `PERF_PORT` (default 1459, never 1420) running `perf-mock-budgets.mjs` and `perf-launch-chat.mjs` (first contentful paint, shell, interactive = last long task before a quiet second, launch IPC calls, when the rail stopped changing and its row count, rail show time (a class change plus layout, not a list re-render), the mock's 60-turn "Canvas review thread" with 10 canvases: open, frame times scrolling top to bottom, iframes mounted, heap of the page's own isolate), `perf-switch.mjs` (ST2: cold start to an interactive Home, then 10 cycles of Home / Tasks / Library / Automations / the 1,000-turn chat switches timed inside the page, p50 / p90 / n and long tasks), `perf-transcript.mjs` (profiling night 11-20: the 1,000-turn thread loaded back to turn 0: DOM nodes, heap, turns in the DOM, scroll frames and typing, over `TRANSCRIPT_BUDGET`; `launchAndChat` also holds the launch IPC tally by command and the heap per chat open), then `perf_hot_paths_report` (Rust IPC over a synthetic vault, test profile, time and serialized payload bytes per path; checks `CARGO_TARGET_DIR` first). The browser scripts reach the shell through `scripts/perf-nav.mjs` only; `perf-nav.test.mjs` pins its selectors to the source, so a shell redesign fails the gate instead of the harness rotting (it did, 11-17 to 11-20). It refuses a port something already listens on and runs vite directly, waiting for it to stop. Not in the gate; several minutes; a toolchain slot plus the one dev server. Numbers compare builds, not the native app |
| `PW_PORT=N [RUNS=5] [STREAM_KB=24] [SECTIONS=launch,switch,open,typing,stream] [PROFILE=1] npx playwright test --config=tests/perf/playwright.perf.config.ts` | Run 15 S7: the frontend responsiveness probe in one Playwright test (`tests/perf/frontend.perf.spec.ts`). Its config builds the mock (`vite build --mode mock --sourcemap`) into `node_modules/.cache/hollow-perf-dist`, never `dist/`, and serves it with `vite preview` on `PW_PORT`. Prints one `S7_PERF {json}` line: launch to shell and interactive (a full quiet second observed, 10 s at most; `launchNoQuietSecond` counts runs that never got one), Tasks / Review / Work switches (the click event to the next frame, plus main-thread CPU over a window to 400 ms after it), opening the 1,000-turn thread (Enter to its newest turn painted; the session's first open apart as `openLongFirst`), typing in it (keydown to the next frame; main-thread CPU per key over three bursts), and two 24 KB replies streamed on a fixed 60-a-second schedule, `stream*` with a code fence every third section and `streamProse*` with none (main-thread CPU, wall-clock task time and its script / layout / style parts, layout and style recalculation counts, frames over 50 ms, delivery time, and last delta to screen, which includes the reply's 80 ms render throttle). Every `*Cpu` figure is the main thread's CPU time (`ThreadTime`): wall-clock task time grows with other work on the machine. `SECTIONS` runs only some measures (typing and the streams open the thread once). Results go to `node_modules/.cache/hollow-perf-results`, never `test-results/`. `PROFILE=1` adds sourcemapped self time for one extra switch of each page, a typing burst and one extra stream of each kind. Not in the gate or the smoke suite; a toolchain slot plus the one server. Numbers compare builds, not the native app |
| `BASE=http://127.0.0.1:N SOAK_MIN=30 npm run -s perf:soak [-- --out f.json]` | Overnight 10-11/12 P6: a memory soak against a served `build:mock` (`scripts/perf-soak.mjs`; start one `vite preview` yourself, under the dev lock). Each cycle opens the 60-turn chat with 10 canvases and scrolls it end to end, visits two workspaces, Tasks and Work, and opens the palette; after each it collects garbage and samples the JS heap, documents, DOM nodes, event listeners and frames. Prints each counter's growth after a 3-cycle warm-up (median of three at each end) and its slope per cycle. Not in the gate |
| `npm run plan:check` | Both Hollow 5.0 ledgers, settled decisions, and one release version (`scripts/version.mjs check`); reads this file for its ledger reference |
| `npm run gate` | The local merge gate in one command: `gate:frontend` (`check-installed-lock.mjs`, then `plan:check`, `typecheck`, `npm test`, `test:scripts`, `build`, `bundle:budget`) then `gate:rust` (the `CARGO_TARGET_DIR` check, fmt, clippy `-D warnings`, `cargo test --lib`), framed by `gate-record.mjs` (`--start` snapshots the tree first; at the end, inside `npm run gate` only, on a clean tree the same at start, build and end, it keeps the provenance in `<git common dir>/hollow-gate-records/<fingerprint>.json` (survives removing the worktree; `build.install_ready` finds origin/main's tree there, so a gated branch counts for its merge commit) and prints the PR line with the gate fingerprint). The whole chain runs inside `scripts/gate-run.mjs '<chain>'` (OF11): `/bin/sh -c` with the environment unchanged (so `npm_lifecycle_event` is still `gate`), stdout and stderr teed through unchanged (so the `[gate-record]` and libtest `test result:` lines still reach the log you grep; the steps see pipes, not a TTY, so no colours in a terminal), the chain's exit code kept (128 + n for a signal). The chain runs in its own process group with stdin `/dev/null`: Ctrl-C, SIGTERM or SIGHUP to the wrapper reaches the running step (vitest, cargo), not only `/bin/sh`, and leftovers in the group get SIGTERM when the wrapper exits; the exit code is taken when `/bin/sh` exits, and a leftover still holding the pipes (a test's `/bin/sleep` child) gets 5 s before the record is written. A caller that stops reading (`npm run gate | head`) just gets no more output: the chain, the log and the record finish and the exit code is still the chain's. It writes one run record per gate, failed ones too, to `<git common dir>/hollow-gate-runs/<start>-<fp8>.json` (written at the start with no `end`, rewritten at the end; a sibling `build.install_ready` never reads): commit, fingerprint, branch, start/end/seconds, exit code, steps with seconds (npm banners; the Rust half split at cargo's lines into fmt, clippy, test build, tests, gate-record; a refusal from `check-installed-lock` or `check-cargo-target` is a step of that name) and which failed, tool durations (vitest `Duration`, node:test `duration_ms`, vite `built in`, cargo `Finished` per profile, every libtest `finished in` summed), failing test names (libtest `---- name stdout ----`, vitest `FAIL`), `>60 s` names, load average at start and end, and other checkouts' gate-record start snapshots (from the last hour, checkout still on disk: a failed gate leaves its snapshot behind). Never the log text: the log goes to `hollow-gate-runs/logs/`, newest 20 kept across all lanes (records: newest 1,000). Both halves run under CI's `TZ=America/New_York`, and the first step refuses a `node_modules` that differs from what CI's `npm ci` would install (package.json out of sync with the lock, a missing or other-version package, a hand-installed one at any depth; it checks, never installs). Vitest refuses `.only` everywhere (`allowOnly: false`), as it does under CI. `ci-gates.test.mjs` fails if it drops a CI step or either parity. Run the halves in separate slots if you like (that records no gate record, so `build.install_ready` won't vouch for the tree; run the whole `npm run gate` for that); set `TMPDIR` for the Rust half inside a Hollow turn |
| `npm run css:check` | Undefined tokens without fallback, raw hex / px font sizes outside `tokens.css`, `!important` outside `a11y`, dangling `animation-name`, stray declarations, infinite animations without the `--ambient-play` opt-in |
| `npm run typecheck` | `tsc --noEmit` |
| `cargo check` / `cargo test --lib` (in `src-tauri/`) | Rust typecheck / unit tests. In tests hold a `flock` as `flock::HeldLock`, never poll for a release; inside a Hollow turn set `TMPDIR` to a neutral folder first. Test folders under the temp dir must be unique per run (`tempfile`, or the process id): `test-temp-paths.test.mjs` refuses a fixed name. A per-process folder named with one of `test_temp.rs`'s prefixes (`hollow-test-<name>-<pid>` for new ones) is swept by the next unfiltered `cargo test --lib` once its process is gone; any other per-process name is never swept, so use that prefix or `tempfile`. A test must hold at any hour and in any time zone (overnight D2 ran Rust in Pago Pago and Vitest in four zones): when the code under test reads the clock, accept the day before or after the call, build local times, or start with `test_clock::clear_of_midnight(margin)`; never read "today" once after it. Unit tests skip macOS's `F_FULLFSYNC` drive-cache flush in `vault::full_sync` (`vault::DRIVE_FLUSH`, overnight OB13): it serializes across the whole machine, and with it libtest took 113 s instead of 57 s. A new durable-write path should go through `vault::atomic_write`/`full_sync`, not std's `sync_all` (which is `F_FULLFSYNC` on Apple targets, in tests too) |
| `npm test` | vitest over `src/**/*.test.ts` (node environment, no DOM, nothing that imports the store at runtime); `navigation.test.ts` pins the Cmd+digit map and the hidden pages |
| `npm run test:scripts` | `node --test scripts/*.test.mjs`: script tests, the CI gate pins (`ci-gates.test.mjs`), the manual path check (`check-claude-paths.mjs` over AGENTS.md, `docs/agents/` and `docs/reference/`), the no-new-sleep-or-clock check for tests (`check-test-sleeps.mjs`, TS1, 2026-11-16: a test may not add a fixed sleep (`waitForTimeout(`, `new Promise(r => setTimeout(r, …))`, `await delay(`, and in Rust tests `thread::sleep(` or `tokio::time::sleep(`) or a bare `Date.now()` / `new Date()` (frontend and script tests only); the counts present when it landed are `scripts/test-sleeps-baseline.json` (path, kind, count), which may only go down: a rise or a new file fails, and so does a count below its baseline until `node scripts/check-test-sleeps.mjs --update` lowers it; a line that must sleep or read the clock says `// sleep-ok: <reason>` on it or the line above; Rust production code outside `#[cfg(test)]` may sleep),  the no-fixed-temp-folder check for Rust tests (`test-temp-paths.test.mjs`), the every-`#[ignore]`-says-why check (`check-ignore-reasons.test.mjs`: `#[ignore = "why; how to run it"]`, also inside `cfg_attr`; the reason may start on the next line), the tight-timing-bound check (`check-timing-asserts.mjs`, S23: an `assert!` putting an upper bound below 2 s, or 5 s in seconds, on an elapsed time fails unless its line or the line above says `// timing: <reason>`. It reads a `Duration`, `.as_millis()` / `.as_secs()` / `.as_secs_f64()`, a `.ms` or `…_ms` field, or a name `let`-bound from `.elapsed()`, bounded by a literal or by the end of a `(A..B).contains(&…)` range. Assert the cause instead (the timeout flag, the result variant, a `Notify` only the test opens) and keep a ≥ 10× "no hang" bound; a pure-timer test could use `#[tokio::test(start_paused = true)]`, but tokio's `test-util` is added only with the first such test. The offenders left at landing are in `scripts/timing-asserts-allowlist.json`, keyed by path and line text, which only shrinks: a stale entry fails too, and the test pins its count) and the no-conflict-markers check for every tracked text file except `tests/fixtures/` (`conflict-markers.test.mjs`; resolve `docs/BUILD_LOG.md` rebases without leaving `|||||||` lines), What's new's install wording (`check-whats-new.mjs`: no "installed yet", and no "not / isn't (yet) in the installed app") and the every-smoke-spec-loads check (`smoke-specs-load.test.mjs`: `playwright test --list --forbid-only` must exit 0 with a non-zero total, about 1 s, no browser; one spec that failed to load emptied every whole-suite run from 10-27 to 11-03) |
| `npm run test:smoke` | Playwright over `?mock=1`: desktop + narrow Chromium for every spec, plus the two supported-minimum projects for the required specs (see Browser fixtures below). Locally use your own `PW_PORT` and `--workers=2`. Whole suite green on main 10-12 (overnight B1, ~22 min at one worker; run it in batches under 10 min in a Hollow turn). Context-menu items (`.ctx-menu`) are `role="menuitem"` and the palette input is a `combobox`: query them by those roles |
| `npm run smoke:critical` | A desktop-only, one-worker subset (app shell, top bar, rail, chats in Chats, canvases, side pane, conversation tools, shortcuts, the launch IPC budget in `launch-ipc.spec.ts`), about 1.5 min with its own dev server on `PW_PORT` (default 1431; never the shared 1420, where Playwright would reuse another checkout's server). Not in `npm run gate` (two lanes gate at once and only one dev server runs machine-wide); run it before merging a UI change while CI is off, when no other dev server is up (`lsof -iTCP -sTCP:LISTEN`). Pinned by `ci-gates.test.mjs` |
| `node scripts/overnight-summary.mjs <folder> [--out FILE]` | Overnight D8: turns an overnight folder's `lane-*.md` reports into one short summary. It lists merged PRs one line each with their main commit, what wasn't merged, notes for Alex, decisions taken, open review findings ("Left", "Documented…", "Accepted", "Not built"…) and what nobody tried by hand. An item is a `##`/`###` heading that starts with its id; a label is a bold or bulleted phrase ending in a colon; the lane letter tells two lanes' same id apart. Local files only |
| `node scripts/gate-stats.mjs --since <iso> [--until <iso>] [--json] [--ref origin/main]` | Gate health (OF11), $0 and read-only: joins the run records (`hollow-gate-runs/`, from 10-17 on), the passing records (`hollow-gate-records/`) and `git log --first-parent --merges <ref>`. A passing gate belongs to the PR whose merge first brought its commit to main (`rev-list <head> ^<main before>`); it is useful when no fingerprint input differs from the merged head (a docs-only commit after it keeps it), else discarded as main moved (main changed an input after its base), own fix (a non-merge branch commit changed an input), mixed (both) or other. A failed run is cancelled (a signal, exit ≥ 128), environment (a pre-check refused before any test failed: `gate-record --start`, `check-installed-lock`, `check-cargo-target`), fixture (a regenerate-and-commit drift test failed: `*_fixture_is_current`, `generated_types_are_current`, `ts_bindings::fixture_*`, the contract table), flake (a test failed and a passing run has its fingerprint) or other. Also runs per merged PR, step wall p50/p95 (completed steps), and >60 s names. Reads no message text. `--json` for scripts |
| `npm run gate -- --digest` (through a runner: `hollow-slot --on runner-host npm run gate -- --digest`) | GD1, 2026-11-16: prints only a digest of the gate instead of every line, under 60: the verdict, the failed step, counts per tool (vitest, `node:test`, cargo, tsc, clippy, fmt), every failing test's name, the first real error with a few lines of context (secrets masked) and the full log's path, which `gate-run.mjs` still writes to the run's `hollow-gate-runs/logs/` (a temp file when the run couldn't be recorded). The exit code is the gate's. Without the flag, or with `--raw`, the output is whole as before. A runner's log path is on the runner (the digest says which host). `node scripts/gate-digest.mjs <log> [--exit N]` digests any saved log. Not yet the default, and `hollow-slot` itself (an installed shell tool) doesn't digest other commands: both wait for an install |
| `node scripts/agent-eval/probe-agent-cli.mjs [--json out.json] [--agent claude\|codex\|grok]... [--keep]` | Agent-CLI facts (OF22), $0 on the isolation of OB23's `scripts/agent-eval/cli-probe.mjs` (allowlisted env, temp homes, dummy keys, a local stub, every proxy at a tripwire). Each CLI gets the stub MCP server `scripts/agent-eval/fixtures/probe/stub-mcp.mjs` (three `probe_*` tools, `_meta {"anthropic/alwaysLoad": true}` on `probe_flagged` only). Claude runs twice with `ENABLE_TOOL_SEARCH=true`, acceptEdits then bypassPermissions: `alwaysLoaded`/`deferred` per run (the HB05 fact behind `app_mcp::ALWAYS_LOAD`) and `modeDiff`, the byte delta of the system text (each run's temp root and its slug as `<ROOT>`, UUIDs as `<UUID>`; `systemFirstDiffOffset`/`systemDiffBlock` say where it first differs) and of each tool description that differs; a run counts only with `probe:connected`, tool search on and the asked mode in force, else `inconclusive`. Codex runs against the stub's Responses API from a temp `CODEX_HOME` (only when `codexPinned` passes; `--strict-config`, analytics and OTel exporters off, `update_plan` forced on and reported as `updatePlanListed`; env `PROBE_CODEX_DISABLE=probe-only` keeps only the probe's four `--disable` flags, dropping Hollow's inherited 24), Grok through a custom `[model]` on the stub's Messages API; both also get a control server (`stub-mcp.mjs --no-meta`) and report, from the tools actually listed in the request, `tolerated`, `drift` (control tools without `probe_flagged`) or `inconclusive` (no control tools), plus `metaForwarded`. Names, offsets and byte sizes only, never request text. Exit 0 clean, 1 a run failed or was inconclusive, an agent asked for wasn't probed, or Codex's Statsig telemetry (`CONNECT ab.chatgpt.com:443` from the Codex run only, `blockedTelemetry`) hit the tripwire, 3 `drift` (a relied-on fact changed; outranks 1), 4 the tripwire saw anything else (which stops the remaining agents). Spawns real CLIs, so it is never in the gate; its unit tests (`agent-eval-probe.test.mjs`) use fixtures |

**Browser fixtures and required checks (review F05–F07, 2026-10-13).**
- **Fixtures:** `?mock=1` (empty vault), `?mock=1&demo=1` (populated: nine
  workspaces, tasks, results, a waiting count), `?mock=1&demo=large` (5,000
  tasks, 10,000 Library files, a 60-turn canvas thread). Add `&main=1` for a
  Main agent conversation and `&missions=1` for running/queued work. The mock
  creates the built-in Main agent workspace on demand, as the backend does,
  so every fixture opens it. Failure switches live in `localStorage`
  (`mockDigestFail`, `mockTaskWriteFail`, `mockCalendarOff`,
  `mockWorkspaceReadError`, `__mockFailCreateTaskOnce` on `window`); each spec
  that uses one names it.
- **Native events:** subscribe through `src/nativeEvents.ts`, never
  `@tauri-apps/api/event` (`check-native-events.mjs`). On the mock it is an
  in-page bus (`window.__mockEmit(name, payload)`); a spec that installs its
  own fake `__TAURI_INTERNALS__` still gets its events.
- **Page errors fail tests:** specs import `test` from
  `tests/smoke/fixtures/test.ts` (`check-smoke-imports.mjs`). An uncaught
  exception or unhandled rejection fails the test unless it calls
  `pageErrors.allow(pattern, reason)`; only a spec's own init script throwing
  inside a sandboxed canvas frame is ignored.
- **Screenshots never go into the checkout:** save them with
  `testInfo.outputPath(...)` (under the ignored `test-results/`) or `attach`
  them; the same check refuses a fixed `screenshot({ path: '…' })`, since a
  smoke run that dirties the tree makes `gate-record` skip its record.
  `.context/` is ignored scratch.
- **Projects:** `chromium-desktop` (1440 × 920) and `chromium-narrow`
  (390 × 844) run every spec. `chromium-minimum` (720 × 480, the native
  window minimum) and `chromium-minimum-large-text` (720 × 600 at the Largest
  text size) run only the required specs, `*.minimum.spec.ts` and the
  click-only `*.journey.spec.ts`, which may not skip
  (`check-required-specs.mjs`). `fixture-parity.spec.ts` mounts every Settings
  section and opens the Main agent in all three fixtures.
  `a11y-sweep.minimum.spec.ts` (overnight 10-15/16 S02) walks 16 main views by
  their route ledger walks at both minimum sizes: axe WCAG 2.1 AA in light and
  dark once nothing moves, and a Tab walk (every stop visible and in the
  window; reaching the body is the end of one cycle, not a lost focus).

**Smoke timing under load (C6 P3).** A "Show N more" button that a list's
IntersectionObserver also reaches by scrolling (the removed `CappedFor` did)
can move before a Playwright click lands, since the click scrolls first:
dispatch the click (`dispatchEvent('click')`) when a test counts steps. A component that focuses something "next frame" must not take focus
from a field already in use (TaskEditor's title did, so a test's typing went
into the title). Reproduce load with Chrome's CPU throttling
(`Emulation.setCPUThrottlingRate` through a CDP session), not sleeps.
`tests/smoke/canvas-scale-measure.spec.ts` (`HOLLOW_MEASURE=1`) prints the
30-canvas first-paint and section re-read numbers.

**Responsive smoke routes.** Expanded Preview canvases keep revision controls
behind Canvas actions → Versions and pin. Open that route before checking the
version label; opening controls must not record another canvas. A test-local
responsive chat helper is a function, so call it on every Cancel/Replace path.
Retained Work headings are hidden behind other pages; count visible page
headings and scope Work attachment checks to `.page-retained`. A return from
Tasks or Library uses its sidebar project row, not Home's hidden Open button.
Seed a project before checking its panel and tracks before checking Running's
cost coverage. Tasks hides its verified-empty suggestion link; seed a supported
local suggestion when following that link. Notification folding fixtures use
local routine sources, because retired message sources are filtered. Wait for
`.palette-backdrop.is-closing` to disappear before reopening search after a
failed destination; that error can arrive during the previous exit hold.
Use `openProjectFiles` / `openProjectFile` in `tests/smoke/choose.ts` (the
top bar's Project files, then the pane's Files picker) for a project's notes,
`openWorkspaceResult` and `resultReviewBar` for a Result, and `openChanges`
for a build workspace's Changes. The Preview pane
takes the page's place (`paneCovers`) in a compact or medium window (up to 1100 px at
Default text size); close it (× or `closeProjectFiles`) before clicking a reply or editing the composer.
Cold-file tests check the fallback, return to edit the retained draft, reopen
to finish loading, then return again to verify preservation. At widths up to
860 px Settings uses its Settings section combobox instead of the hidden index.
The Mermaid and Present specs exercise these paths without skips or forced clicks.
Work stays mounted but hidden behind other pages. Check attachment and hidden
state instead of expecting its DOM to disappear. Work Home clears the workspace
and unmounts the transcript; use that route for fresh-mount settling regressions.

**Cloned dependencies.** Matching donor and worktree lockfiles does not prove
the donor's installed packages match either lock. After copying `node_modules`,
run `node scripts/check-installed-lock.mjs` before building or running browser
checks. If it refuses, repair only the new worktree with `npm ci --no-audit
--no-fund` and `npm rebuild --ignore-scripts`, then check again. Revalidate any
browser evidence collected before a dependency repair.

**Where a command runs (smart routing, 2026-11-03).** Where a remote runner is set up, write plain
`hollow-slot <command>`: it decides between this Mac and the runner and prints one line saying which and why
(`hollow-slot: on the runner (gate, usually 3 min; this Mac 72% free)`). Requirements come first (a command
that uses this Mac's database variables, gcloud, `gh`, `git` or `ssh`, the keychain or signing, installs
packages, writes into the checkout such as `cargo fmt`, `npm run lint:fix`, a `>` redirect or
`HOLLOW_REGENERATE_PROMPTS=1`, an app build, a benchmark, `cargo run`, a dev server, media work without
`--send`, Python tests outside `uv run`, an absolute path into the checkout (write paths relative), or a file
outside the checkout stays here), then the command's kind (gates, `cargo build/test/check/clippy`, whole vitest or `npm
test` runs, script tests, typechecks, lints, builds, Playwright and Python tests go there; one test file or a
lint of named files stays here), then its history in this repository (passing runs only: a usual time of 10 s
or more sends a quick command there, and one of no known kind after two such runs; a heavy one under 5 s
stays here; a command that changed the checkout, or of no known kind that failed there and passes here,
stays here), then pressure (this Mac under its memory floor, both slots taken or on battery sends a command
that may go). The runner's preparation reports its tools, load and system: a command needing a tool it lacks
(today Postgres) runs here, so does one that needs macOS when the runner is Linux (a Tauri checkout's Rust
build or gate, Apple's tools, MLX, a webkit browser; [REMOTE_SESSIONS.md](../REMOTE_SESSIONS.md), Linux
runner), and an automatic one comes back when the runner's slots or Rust lock are taken while this
Mac is free; a runner that didn't answer in the last 5 minutes is skipped. `--here` keeps it here, `--remote`
sends it there, and Settings → Devices → Builds and tests (Auto / Mac mini / This Mac; each turn gets it
as `HOLLOW_SLOT_ROUTE`) sets the default and shows the latest decisions. A run there that
changed files in its copy of the checkout says which (they don't come back), and a build there says what
ignored output stayed there. Rules, files and tests: [WORKSPACES.md](../WORKSPACES.md) (Smart routing). A test suite that needs a database
takes `--postgres`: a throwaway Postgres wherever it runs, `DATABASE_URL` set to its `hollow_test`, vitest on
one worker, gone when the command ends (Portal's integration suite: `hollow-slot --postgres sh -c 'npx prisma
migrate deploy && npx vitest run --config vitest.integration.config.ts'`, 1469 passed on the Mac mini).
Measured 2026-11-03: about 1 s of overhead for a worktree the mini already has, about 4 s for a new one; full
gate median 193 s there (76 runs) against 221 s here (191 runs).

**Gates and tests on the Mac mini (2026-11-01).** Where a remote runner is set up (Alex's MacBook:
`runner-host`, the Mac mini as `runner`), plain `hollow-slot npm run gate` runs the gate there (above), and the
same for `cargo test --lib <filter>` (from `src-tauri/`), `npm test`, `npm run test:scripts` and Playwright
(`hollow-slot --fetch test-results npx playwright test --workers=2 …`; the host sets its own
`PW_PORT`; `--fetch` brings back only what this run wrote, so a run that failed before Playwright started
says `test-results` is older than this run instead of returning the mini's last copy, 2026-11-03; for the
same reason an incremental tool with nothing to do, such as a no-op `make` or `tsc -b`, brings nothing back,
so delete its output in the command first, `sh -c 'rm -rf dist && make'`). The worktree's exact state goes along, committed or not; output streams back, the exit code is
the gate's, and a clean tree's gate record comes back to this checkout's `hollow-gate-records/`, so it counts
for `build.install_ready` and the merge tooling exactly like a gate run here. Every gate's run record comes
back too (`hollow-gate-runs/`, with `host` and the caller's HEAD), so `node scripts/gate-stats.mjs` here
counts mini gates and prints runs by machine. When the mini can't take it
(unreachable, under its 20 GB or 30% memory floor, or a Codex sandbox with no network), the call says why and
runs here, where `hollow-slot` itself now keeps this Mac's 30% memory floor: a build or dev server waits while
less is free (once it holds its slot) and refuses with exit 69 after 5 minutes (`HOLLOW_SLOT_MIN_FREE_PCT`,
`HOLLOW_SLOT_MEMORY_WAIT`). Setup before the command normally takes 1 to 4 s; when it takes 5 s or more
the call names the slow steps in one line (`setup took … s before the command on <host>: …`), so a slow
push, prune, checkout or `npm ci` there shows instead of looking like a slow gate. Long media jobs (a
whisper transcription, a HyperFrames render) go there too with `hollow-slot --remote --send <file or folder>
--fetch <output> <command>`, from any folder, one job at a time there with at most 2 render workers
([WORKSPACES.md](../WORKSPACES.md), Jobs on the runner). Nothing of this
Mac's environment goes along (`env NAME=value <cmd>` does) except the checkout's git identity, so a
commit made there is this Mac's (2026-11-02). Never run the same gate here and on the mini at
once. Stays on this Mac: builds whose output you need here (`tauri:rebuild`, `npm run build` for an install:
run those with `--here`), dev servers, and checks of the installed app. Mechanics and limits: [WORKSPACES.md](../WORKSPACES.md) (Remote
runs); timings in the build log (2026-11-01). Needs a Hollow built from
2026-11-01's main or later: an older installed `hollow-slot` fails `--remote` at once (exit 2,
`exec: --: invalid option`); run the gate here then.

**Checking an agent-built workbook (UN2, 2026-11-18).** `node scripts/workbook-recalc.mjs <book.xlsx>
[--expect 'Sheet!B10=1234.56']... [--json]` recomputes the formulas it understands (arithmetic, comparisons,
`&`, references and ranges across sheets, shared formulas, SUM, AVERAGE, MIN, MAX, COUNT, COUNTA, PRODUCT,
ROUND/ROUNDUP/ROUNDDOWN, ABS, IF, IFERROR, AND, OR, NOT, SUMPRODUCT) and reports error values, `#REF!` in a
formula, circular references, cached values that differ from the recomputed ones (within max(0.005, 1e-9 x
the value)) and each `--expect` (the invoice's stated total) that the cell doesn't recompute to. Anything
else is listed as unchecked, never guessed. Exit 0 clean, 1 findings, 2 unreadable file or bad arguments.
Books written by openpyxl carry no cached values, so pass the stated total with `--expect`. On the Mac mini,
from the checkout with the book inside it (a job folder holds only what is sent): `hollow-slot --remote
--send scripts/workbook-recalc.mjs --send <book.xlsx> node scripts/workbook-recalc.mjs <book.xlsx>
--expect 'Invoice!B5=383.75'` (on the work tailnet `--on runner-host-lan` instead of `--remote`). The script
has no imports outside Node's standard library, so a scratch folder holding a copy of it and the book works
the same way.

**CI is a cheap second look, not the merge gate (owner, 2026-10-03).** Merges are gated on `npm run gate`
(or its two halves) run on the rebased head before merging; it is never a hand-picked subset, because on
2026-10-09 a gate that skipped `npm run build` let 9 `css:check` violations reach main and the install build
failed. Its steps are in the `npm run gate` row of the table above (the installed-lock check opens the
frontend half, the `CARGO_TARGET_DIR` check the Rust half); run the Playwright smoke too, separately, when
frontend files changed. The PR body lists what ran. Two CI lanes stay out of `npm run gate` on purpose: the Playwright smoke (it starts its own
Vite dev server, and dev servers run one at a time machine-wide) and the pre-release `tauri build` (a release
build). Otherwise the gate covers CI's steps, its time zone and its `.only` refusal, and checks `node_modules` against the lock.
One known difference: CI runs Node 22 (`.nvmrc`) and this Mac runs a newer Node, so Intl/ICU output can differ. CI runs **only on pull requests** (never again on the push to `main`), can be run by hand
from the Actions tab, and is path-filtered so a PR pays only for what it touches:

| Workflow | Runner | Runs when | Does |
|---|---|---|---|
| `.github/workflows/ci.yml` (`frontend`) | macOS (under a minute; a few script tests drive macOS tools) | any non-docs PR (`paths-ignore` `**/*.md`, `docs/**`) | `plan:check`, `typecheck`, `npm test`, `test:scripts`, `build`, `bundle:budget` |
| `.github/workflows/smoke.yml` | Linux | PRs touching `src/`, `tests/`, `index.html`, package files, Playwright/Vite config | Playwright suite (`--workers=2`, `PW_PORT=4417`, `TZ=America/New_York`) |
| `.github/workflows/rust.yml` (`backend`) | macOS (keyring/FSEvents) | PRs touching `src-tauri/` or `rust-toolchain.toml` | toolchain from `rust-toolchain.toml`, `cargo fmt --check`, clippy, `cargo test --lib` |

Why: the repo is private and macOS minutes come out of the account's quota at a large multiple; after the
smoke job landed, a run averaged 13 macOS minutes and Actions started refusing jobs (2026-09-26/27 and from
2026-10-03; the workspace's CI-billing note has the numbers). `scripts/ci-gates.test.mjs` pins the gates
and these triggers. The runner is slow, so a smoke spec must wait for state, never sample it.

**Pre-release build lane (2026-10-02):** `.github/workflows/prerelease-build.yml`
runs the release-mode `tauri build --ci --bundles app` on macOS, then
`codesign --verify --deep --strict`, the provenance `--check` and the bundle's
`embed-selftest`. It never runs on a PR. It runs by hand (Actions →
prerelease-build → Run workflow) and on pushes to `release/**`; run it by hand
after editing the lane or `rust-toolchain.toml`. It uses no secrets: the shipped config
signs with the local "Hollow Dev Signing" identity, so the lane passes
`--config '{"bundle":{"macOS":{"signingIdentity":"-"}}}'` (ad-hoc). It uploads
nothing. A green lane means the bundle builds and verifies; it is not an
install.

## Smoke notes moved from CLAUDE.md (2026-11-13)

Test conventions that sat at the top of CLAUDE.md until the 2026-11-13
accuracy pass.

- The prompt fixtures `tests/fixtures/prompts/context-budget.json`,
  `turn-budget.json` and `envelope.json` record the measured size and shape
  of a turn's prompt ([Agent context](../AGENT_CONTEXT.md)); regenerate them
  when what a turn carries changes.
- `tests/smoke/automation-drafts.spec.ts` covers guarded result and canvas
  links in real draft prompts.
- `tests/smoke/launch-ipc.spec.ts` pins the clock (`page.clock`) and asserts
  that no launch reads the cut weekly review.
- `tests/smoke/claude-accounts.spec.ts` reads routing and quota failover from
  the Usage limits panel.
- Usage specs check the whole panel except built-in caps; the overnight
  summary names costs that were not reported.

## Gate time (overnight D7 profile)

Runner SDK gotcha (2026-11-16): the mini's default macOS 27.2 SDK fails even a
C++11 `#include <locale>` syntax check, blocking `esaxx-rs`. Its installed
26.5 SDK passes that probe. For this environment, pass
`SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk` to the gate
through `hollow-slot --remote env`; do not change the machine's selected tools.

Moved from CLAUDE.md on 2026-11-13.

- **Where it goes:** the frontend half takes about 18 s. Clippy takes about
  32 s, and `cargo test --lib` about 90 s to run plus its compile. The Rust
  tests are throughput-bound, not waiting on one slow test.
- **To see per-test times** without nightly, run the built test binary with
  `RUSTC_BOOTSTRAP=1 <target>/debug/deps/hollow_lib-<hash> -Z unstable-options --report-time --format json`.
  The `exec_time` field is per test.
- **Under contention**, "running for over 60 seconds" warnings on the G2
  budget tests mean contention, not slow code: alone they take 3 s.

## Tests sharing process state (build run 11-18)

`cargo test` runs a module's tests on parallel threads in one process, so a
`static` (`OnceLock`, `LazyLock`) is shared by every test running at once.
State with a cap or a per-caller count must be keyed by something each test
owns, usually its vault: `chat_move::staging(vault)` gives each vault its
own staging area (the app serves one vault, so its caps hold process-wide).
Before 11-18 it was one area for the process, capped at 4 moves and 2 per
Mac, and every move test sends as one Mac id: tests running alongside, and
parts a lossy test left staged for 10 minutes, made other moves answer
`busy`, so `move_send_tests` failed 5 of 5 runs on this Mac and 2 of 4 gates.
A retry loop or another Mac id only moves such a race; fix the key.
Exact-size comparisons across seeded chats need fixed times:
`to_rfc3339()` prints 0, 3, 6 or 9 fractional digits by the value
(`chat_move_slices_tests::pin_times`).

## Timing-sensitive tests (overnight 11-13 N3)

- `runner_coo::tests::with_no_runner_the_single_coo_is_unchanged` compares status
  paths using one shared instant. Separate `Utc::now()` inputs differ when a
  check-in is due now, even though both implementations are correct (11-16).

A gate failure in one of these is a reason to look at the clock or the load first, then rerun once.

- **`run_pump::tests::a_transient_failure_enqueues_one_delayed_retry_and_its_claim_is_validated`** (failed once on #1367's gate, "admitted instead of handled"). Cause: it writes budget usage at the real time and claims the retry row a minute later, so a gate that straddled local midnight saw the usage on one day and the claim on the next, and the day budget no longer held. Fixed 11-13: every `run_pump_tests.rs` test that calls `budget_over` now starts with `test_clock::clear_of_midnight(3 minutes)`. A gate that fails there within 4 minutes of midnight local time on an older tree is this.
- **`perf_hot_paths`** judges each path on the calling thread's CPU time with several tries (ceilings about 3 times the measured value, OF10), so machine load rarely fails it; a failure that reproduces on a quiet mini is a real regression, one that does not is noise: rerun once before debugging.
- **Agent budget tests** (`budgets_tests.rs`, `chat_g2_budget_tests.rs`, `cfo/budgets_tests.rs`) use day windows too but write and check in the same instant, so only a failure within a second of local midnight is the clock; they have no guard. Any other failure there is a defect, or load: rerun once before debugging.

## Running the app

Use Rust **1.89+** (standard-library file locks serialize imports across app
processes) and Node.js **20+**. `rust-toolchain.toml` pins the exact release
(**1.96.0** since 2026-10-02) for CI and local builds alike; a machine that only
has `stable` installed needs `rustup toolchain install 1.96.0 --profile minimal
--component rustfmt,clippy` once (recent rustups do it on the first cargo
command). Bump the pin on purpose, in its own change.

**Rust hygiene, enforced by CI:** the crate is rustfmt-clean (default
rustfmt, no `rustfmt.toml`), so run `cargo fmt` in `src-tauri/` before
committing Rust. CI also runs `cargo clippy --lib --tests -- -D warnings`:
any warning not allowed in `Cargo.toml`'s `[lints.clippy]` table fails. That
table is a recorded baseline to shrink (`type_complexity`,
`large_enum_variant`, `result_large_err`, `too_many_arguments`,
`await_holding_lock`): fix a lint's sites and drop its line, never add one to
get a change through.
- **Dead code warns** (the `dead_code` baseline line was removed by overnight
  E6), in two builds: the app and the tests.
- **A helper only tests use** is `#[cfg(test)]`.
- **Gate time** (where it goes, per-test times, contention warnings):
  [Gate time](running-and-ci.md#gate-time-overnight-d7-profile).
- **An item kept on purpose** carries a targeted `#[allow(dead_code)]` with its
  reason: a guard held for its `Drop`, a field parsed from older configs, an
  enum variant written by SQL and the ts-rs-only `ts_bindings`. A
source-text test that pins code with `include_str!` matches through
`crate::source_pins` (`find`/`contains`/`count`), never a raw one-line
`str::find`. The frontend has no linter or formatter. If a build fails on a
generated Tauri permission path under an old checkout, clean the `tauri*` and
`hollow` Cargo packages (`cargo clean --release -p …` for packaging).

`npx tauri dev` is the only entry point that works end-to-end. It
spins up Vite on `localhost:1420` AND the Rust shell embedding the WebView
pointed at Vite.

**Do NOT just run `npm run dev` to verify real app behavior:** in a plain
browser Tauri's `invoke()` is undefined. The exception is `/?mock=1`, which
activates the `mockApi.ts` fixtures for Playwright. It is honoured only by
`npm run dev` and `npm run build:mock`: `src/mockMode.ts` owns the check, a
plain `npm run build` tree-shakes the mock out, and
`scripts/check-no-mock-bundle.mjs` fails the build if any of it reaches
`dist/`. Import `mockMode.ts`, never `mockApi.ts`, to ask "am I mocked?".
`mockApi.ts` throws on a command with no fixture (`NO_OP_COMMANDS`: ignorable
mutations), logging it in `__mockApiMisses` (specs assert it empty). Dates
are local (CI: `TZ=America/New_York`).

### The commands you need on most tasks

The full table (perf, soak, kit, bundle budget, overnight summary) is in
[Running, testing and CI](running-and-ci.md), with the CI
workflows, smoke timing under load and the pre-release lane.

| Command | Use |
|---|---|
| `npm run gate` | The merge gate: `gate:frontend` (installed-lock check, `plan:check`, `typecheck`, `npm test`, `test:scripts`, `build`, `bundle:budget`) then `gate:rust` (`CARGO_TARGET_DIR` check, fmt, clippy, `cargo test --lib`). Fingerprint only for a clean committed tree; every run is recorded for `node scripts/gate-stats.mjs --since <iso>` |
| `npm run typecheck` / `npm test` / `npm run test:scripts` | The frontend halves one at a time (`npm test` with `VITEST_MIN_FORKS=1 VITEST_MIN_THREADS=1`) |
| `cargo fmt` / `cargo clippy --lib --tests -- -D warnings` / `cargo test --lib` | The Rust half, in `src-tauri/` (set `TMPDIR` to a neutral folder inside a Hollow turn) |
| `npm run test:smoke` / `npm run smoke:critical` | Playwright over `?mock=1`, only with no other dev server up. `test:smoke`: own `PW_PORT` (never 1420), `--workers=2`; `smoke:critical`: one worker, `PW_PORT` 1431 |
| `node scripts/agent-eval/scorecard.mjs summary --vault <v> [--since <iso>] [--overlap] --json <f>` / `compare <A> <B>` | Free efficiency scorecard; read-only, [Build & ship](build-and-ship.md). `trace-assert <stream.jsonl>...` (NV5) flags unread skills and `run_action` without `describe_action`. `skill-routing.mjs score|run` (BI1): per-agent skill routing accuracy and failed confusable pairs; `run` is paid, `--live` only. `judge-qualify.mjs run|score` (NV6): a judge's self-agreement and label agreement on the pilot's judged items; paid, `--live` only. `skill-lift.mjs score|run` (NV4): a skill's lift, with against without on the same cases and rules, (paired bootstrap); provisional under 10 cases or with no negative case; `run` is paid, `--live` only. `scenario-gate.mjs status|admit` (ND3, ND4): every scenario must fail a do-nothing answer; admission is 5 of 5 per fingerprint, an edit goes stale. NV7 (`eval_candidates.rs`, action `feedback.file_eval_candidate`): a chat that missed a skill becomes a redacted `eval_candidate` card; accepting needs a reviewer-written expected skill and writes a proposed fixture to Main's `outputs/eval-cases/` |
| `node scripts/agent-eval/my-context.mjs [--session <key>] [--json]` | The running chat's own main-thread context (marker, ledger, transcript); hand over at 350K. Exit 3: a source is missing |
| `node scripts/agent-eval/morning-report.mjs --since <launch> --hw <HW> --vault <v> --out <md>` | The overnight morning sheet (tonight's lanes against the evening lanes); read-only, see [Build & ship](build-and-ship.md) |
| `node scripts/agent-eval/run-facts.mjs <transcript>` | Run facts (LF3); lane tools `lane-preflight`, `lane-triage`, `lane-finish`, `lane-extract`, `lane-outcome`, `anti-goals`: [Build & ship](build-and-ship.md) |
| `node scripts/agent-eval/replay-bench.mjs <bench.json> [<other>]` | Replay bench (S07): recorded vs replayed at HEAD; read-only, [Build & ship](build-and-ship.md) |
| `node scripts/workbook-recalc.mjs <book.xlsx> [--expect 'Sheet!B10=1.5'] [--json]` | Recompute an agent-built workbook's formulas (UN2); on the mini: [Running and CI](running-and-ci.md) |
| `CARGO_TARGET_DIR="$HOME/Library/Caches/hollow-release-target" npm run tauri:rebuild` | Build, install, verify and clean up the production app; Alex's OK only |

### Gate, merge and ship, in short

These summarise rules given in full in the reference docs, which win
where they differ.

- **The merge gate is `npm run gate` on the rebased head**, never a
  hand-picked subset; add the Playwright smoke when frontend files changed,
  and list what ran in the PR body. CI runs only on pull requests, is
  path-filtered, a cheap second look, not the gate
  ([Running, testing and CI](running-and-ci.md)).
- **Cargo targets:** an absolute `CARGO_TARGET_DIR` with no `~` segment (checks
  refuse one), and `CARGO_PROFILE_DEV_DEBUG=0
  CARGO_PROFILE_TEST_DEBUG=0` for repeated gates, or a dev target grows
  without bound ([Build & ship](build-and-ship.md#build--ship-how-a-change-reaches-the-installed-app)).
- **Machine limits:** at most two toolchain slots run at once, only one of
  them a Rust build (`hollow-slot`'s `cargo.lock`: two compiles peak near
  13 GB; only the Mac mini runner has a second Rust lane, `cargo-1.lock`, 5
  cargo jobs each, off while under 30% memory free), and only one dev server runs machine-wide, so
  `npm run gate` leaves the smoke out ([WORKSPACES.md](../WORKSPACES.md),
  Build settings).
- **Builds and tests route themselves:** plain `hollow-slot <cmd>` picks
  this Mac or the runner and says why (gates and suites go there; quick
  checks, writers and what needs this Mac stay); `--here`/`--remote` force
  it ([Running, testing and CI](running-and-ci.md)).
  Workspace turns can run there (this Mac's git identity), overnight and
  worker chats when chosen; each records where, why and any fallback
  (`Route`, [Runs on](../REMOTE_SESSIONS.md)).
  Runner access: `runner_access.rs` (no credential in the header); owner: `runner_owner.rs`.
- **Rust tests:** temp folders unique per run (`tempfile` or
  `hollow-test-<name>-<pid>`; `test-temp-paths.test.mjs` refuses a fixed
  name), tests that hold at any hour or time zone
  (`test_clock::clear_of_midnight`), no `static` tests share
  (per vault: `chat_move::staging`), a held `flock` as `flock::HeldLock`,
  never polled, `#[ignore = "why"]`, never a bare `#[ignore]`
  (`check-ignore-reasons.test.mjs`), and no tight wall-clock upper bound in
  an assert: assert the cause and keep a ≥ 10× "no hang" bound, or say
  `// timing: <reason>` (`check-timing-asserts.mjs`); no new test sleep or clock read (`check-test-sleeps.mjs`) ([Running, testing and CI](running-and-ci.md)).
- **Orphan check:** after `cargo test --lib`, the `sleeper_helper` /
  `/bin/sleep 60` process grep must print nothing ([Quirks](quirks.md)).
- **Dev runs aren't sandboxed:** pointing `vaultPath` at a temp dir doesn't
  isolate one, and dev runs share `localStorage`; use the mock harness or
  isolated tests ([Quirks](quirks.md)).
- **Gating inside a Hollow turn:** service-lifetime tests reject `hollow-run`
  paths, replay tests reject checkout-contained temp folders, and socket and
  prompt-budget tests need short paths. Use a fresh `/tmp/v.XXXXXX`
  outside the checkout, keeping gate logs in turn scratch
  ([Build & ship](build-and-ship.md#build--ship-how-a-change-reaches-the-installed-app)).
- **Installs:** never stop after the raw bundle (macOS indexes it as a second
  Hollow); check Hollow is idle with `pgrep -a` (keep the `-a`) and fall back
  to `TERM`, never `KILL` ([Build & ship](build-and-ship.md#build--ship-how-a-change-reaches-the-installed-app)).
- **Fetching by URL:** `git fetch --no-prune <url> main:refs/remotes/origin/main`
  (without `--no-prune` the tracking ref can be pruned; a plain URL fetch
  updates only `FETCH_HEAD`) ([Build & ship](build-and-ship.md#build--ship-how-a-change-reaches-the-installed-app)).
- **Shipping:** commit → PR → merge to `main` → rebuild → install. Installs:
  Update button or overnight; Go back: Settings ([UPDATES.md](../UPDATES.md)).
  No test bed: a change ships on a green gate and an install Alex OKs. Release builds: `0.6.<first-parent
  count>`; no version bumps. The **Current install**
  paragraph (each install replaces it) is in
  [Build & ship](build-and-ship.md#build--ship-how-a-change-reaches-the-installed-app).
