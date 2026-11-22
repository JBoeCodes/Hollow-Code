# Dependency triage

The current reachability record for dependency advisories (review F13). It
supersedes nothing in the dated [review](review-2026-10-13/DEPENDENCIES.md);
that file stays the 0.5.15 baseline. Refresh this file whenever a lockfile
changes for an advisory, and rerun the checks listed under "How to refresh".

**Last refreshed:** 2026-11-20 (reachability re-triage for `quick-xml` and new-dependency inventory; last full `npm audit` + OSV `Cargo.lock` scan was 2026-10-13 on branch `feat/consolidation-20261013` — re-run "How to refresh" online when updating lockfiles).

Reachable means an attacker-controlled input can reach the vulnerable code in
the **installed app** under the advisory's stated preconditions. Development
exposure (a dev server or test runner on this Mac) is recorded separately and
is not called an app vulnerability.

## npm (`package-lock.json`)

`npm audit` before: 14 entries (5 moderate, 5 high, 4 critical). After the
patch-level updates below (`npm audit fix`, no `--force`): **6 entries
(3 moderate, 1 high, 2 critical), all in the Vite / Vitest / esbuild
development toolchain.**

| Package | Locked → now | Path | Advisories | Precondition | Assessment | Action |
|---|---|---|---|---|---|---|
| dompurify | 3.4.10 → 3.4.17 | Runtime, direct (`MarkdownView.tsx`, `NoteRenderer.tsx`) | GHSA-cmwh-pvxp-8882, GHSA-55q2-fjhq-7xh7, GHSA-c2j3-45gr-mqc4, GHSA-6688-9rhm-gjv2 | `setConfig()` with hooks; `IN_PLACE`; `CUSTOM_ELEMENT_HANDLING` | **Not reachable**: both callers pass per-call options (`USE_PROFILES`, `FORBID_TAGS`, `ADD_ATTR`); no `setConfig`, `addHook`, `IN_PLACE`, `RETURN_DOM` or custom-element handling anywhere in `src/` | Upgraded (runtime sanitizer stays current) |
| solid-js / seroval / seroval-plugins | 1.9.13 / 1.5.4 → 1.9.17 / 1.6.9 | Runtime framework; seroval only from `solid-js/web/dist/server.js` | GHSA-p6vx-979v-rg4c, GHSA-jp82-f5mq-hwhp | Deserializing attacker JSON with `fromJSON` (SSR/hydration) | **Not reachable**: Hollow is client-rendered; no `seroval` code in `dist/` (searched); no app `fromJSON` | Upgraded (patch) |
| postcss, nanoid, source-map-js, browserslist, baseline-browser-mapping, update-browserslist-db, caniuse-lite | patch updates | Build tooling | GHSA-fxqj-rqcc-2cmp, GHSA-r28c-9q8g-f849, GHSA-28wg-ghj8-5hjv, GHSA-2v37-7h3g-55p8, GHSA-68fv-2mgg-jv7q, GHSA-c83g-rgw3-j3cx, GHSA-73wf-gq98-2v4g, GHSA-w5vr-8v7q-w6rv | Attacker-controlled CSS/source maps/stats files at build time | **Not reachable** in the app (not shipped); build inputs are this repository | Upgraded (patch) |
| vite | 5.4.21 (unchanged) | Dev server + build, direct | GHSA-4w7w-66w2-5vf9, GHSA-v6wh-96g9-6wx3, GHSA-fx2h-pf6j-xcff | Requests to a running dev server; two are Windows-only | **Not reachable** in the installed app (the production bundle has no dev server). Dev exposure: Vite's default host is `localhost` (`vite.config.ts` sets no `host`), Playwright binds `127.0.0.1`, and Vite 5.4.12+ limits CORS to local origins | Deferred: fix needs Vite 7–8 (major) — its own toolchain change |
| esbuild | 0.21.5 (unchanged) | Build-time `buildSync`/`transformSync` (`scripts/build-canvas-kit.mjs`) and Vite's minifier | GHSA-67mh-4wv8-2f99 | esbuild's own `serve` API running | **Not reachable**: no `serve` call anywhere; Vite owns the dev server | Deferred: fix is 0.25+ (breaking; Vite 5 pins its own esbuild) |
| vitest, @vitest/mocker, vite-node, tinypool | 2.1.9 / 1.1.1 (unchanged) | Unit tests only | GHSA-5xrq-8626-4rwp, GHSA-82fw-gwwq-j7x9, GHSA-5gmw-xhrv-c9v3, GHSA-85c8-ppgw-ccpr | Vitest UI/API server listening; attacker-controlled mock redirects; prototype pollution feeding worker options | **Not reachable**: `@vitest/ui` is not installed and no `--ui`/`api` option is used; tests run `vitest run` on repository code | Deferred: fix is Vitest 5 (major, needs Vite 6+) |

Regression evidence for the npm update: frontend gate (typecheck, 2,160 unit
tests, 207 script tests, production build, bundle budget, no-mock-bundle check)
and Chromium smoke over sanitizing, canvas and editor paths: remote-images,
canvas-inline, canvas-kit, canvas-kit-ask, canvas-a11y, artifact-sri,
next-level-library, app-smoke, workbench (61 passed; the 6 skips are
narrow-only cases on the desktop project).

## Rust (`src-tauri/Cargo.lock`)

No `cargo audit`/`cargo deny` is installed here; the 717 registry crates in
`Cargo.lock` were checked against the public OSV database
(`api.osv.dev/v1/querybatch`, ecosystem crates.io) on 2026-10-13.

| Crate | Locked → now | Used by (macOS) | Advisory | Assessment | Action |
|---|---|---|---|---|---|
| rustls (+ rustls-webpki) | 0.23.40 → 0.23.45 | reqwest → every HTTPS call (Notion, connectors, sign-in) | RUSTSEC-2026-0285 (TLS 1.3 handshake messages accepted across encryption levels) | **Reachable** in principle: a hostile or intercepting TLS peer | Upgraded (semver-compatible) |
| anyhow | 1.0.102 → 1.0.103 | tauri-utils and others | RUSTSEC-2026-0190 (unsound `downcast_mut`) | Unsound API, no attacker input; Hollow does not call `downcast_mut` | Upgraded |
| crossbeam-epoch | 0.9.18 → 0.9.20 | ignore, rayon (candle) | RUSTSEC-2026-0204 (`fmt::Pointer` deref) | Only when formatting `Atomic`/`Shared` with `{:p}`; not done | Upgraded |
| event-listener | 5.4.1 → 5.4.2 | not in the macOS normal graph | RUSTSEC-2026-0221 (unsound) | Not reachable on macOS | Upgraded (compatible) |
| serde_with | 3.19.0 → 3.21.0 | tauri-utils | GHSA-7gcf-g7xr-8hxj (panic serializing an empty `KeyValueMap`) | Tauri's own config types; no attacker input | Upgraded |
| quinn-proto | 0.11.14 (unchanged) | none (`cargo tree -i quinn-proto --target all` prints nothing) | RUSTSEC-2026-0185, -0338, -0339, -0340, -0342, GHSA-4w2j-m93h-cj5j | **Not reachable**: not compiled into any target; a lock-only entry | Left; updating it pulls `rand 0.10` for no effect |
| quick-xml | =0.39.4 (direct) / 0.37.5 (unchanged) | 0.39.4: **direct** in `src-tauri/src/custom_icons.rs` (`svg_dimensions`, validating UI Settings SVG uploads up to 8 MiB and generated logo SVGs) plus transitive `plist` → `tauri-utils`; 0.37.5: Windows notifications only | RUSTSEC-2026-0194, RUSTSEC-2026-0195 (quadratic/unbounded XML DoS) | **Reachable on local UI SVG uploads (DoS only, no RCE/XSS)**: no longer plist-only. `custom_icons::prepare` parses user-uploaded SVGs up to 8 MiB (`MAX_INPUT`) via `quick_xml::Reader::from_str` / `read_event()` before its 10,000-element / 32-depth cap runs (COO `load_file` rejects `.svg`, so remote/unattended callers cannot reach it). The fix (0.41) is a breaking release upstream `plist` has not adopted yet. | Re-triaged 2026-11-20; plan upgrade to `quick-xml >= 0.41` (decoupled from `plist` or alongside a Tauri/plist bump) or add tighter pre-parse XML bounds in `custom_icons.rs` |
| glib | 0.18.5 | Linux GTK builds only | RUSTSEC-2024-0429 (unsound iterator) | Not in the macOS build | Left |
| derivative, instant, paste, proc-macro-error, unic-* | unchanged | build-time macros / transitive | RUSTSEC "unmaintained" notices | Informational, no vulnerability | Left |

Regression evidence for the Rust update: `npm run gate:rust` (rustfmt, clippy
`-D warnings`, `cargo test --lib`: 3,187 passed, 0 failed, 24 ignored).

## Crates and packages added since the refresh

### Rust (`src-tauri/Cargo.toml`)

| Crate | Version | Added by | Used for | Lock change | Advisories |
|---|---|---|---|---|---|
| objc2-event-kit | 0.3.2 (macOS only, selected event features, no reminders) | P4-26, 2026-10-24 | Read-only EventKit source behind Agenda; calendar metadata, occurrence events and participants on a blocking thread | One new package; existing objc2/framework family reused | Advisory lookup not performed in the unattended no-external-service run. Native system framework; no parser or calendar writer added. Fake-reader tests and compilation cover the adapter; native permission remains an attended check. |
| objc2-pdf-kit | 0.3.2 (`default-features = false`, `std`, `PDFDocument`, `PDFPage`; macOS target only) | repo features R14a, 2026-10-18 | `doc_text.rs`: the text layer of PDFs in connected projects and workspace `sources/` (`PDFDocument::initWithData`, `isLocked`, `pageCount`, `pageAtIndex`, `PDFPage::string`), on a blocking thread inside an autorelease pool | One new package; its dependencies (`objc2` 0.6.4, `objc2-foundation` 0.3.2, `bitflags` 2) were already locked. The same objc2 family main pins. | None in OSV (`api.osv.dev/v1/query`, crates.io, 0.3.2) on 2026-10-18. Input is any PDF in a connected folder: PDFKit is Apple's system parser, the bytes never leave the Mac, and files over 64 MB are refused before parsing. |
| quick-xml | `=0.39.4` (promoted from transitive `plist` dep to direct dep) | Custom icons, 2026-10/11 | `custom_icons.rs::svg_dimensions`: strict static-SVG allowlist check for UI Settings icon uploads (up to 8 MiB) and generated logo options | Already locked via `plist`; no new transitive packages | **RUSTSEC-2026-0194, RUSTSEC-2026-0195** (quadratic/unbounded XML DoS). Now reachable on local UI SVG uploads (re-triaged in the Rust table above; COO `load_file` rejects `.svg`). |
| qrcode | `0.14` (`default-features = false`, `features = ["svg"]`) | Phone chat API, 2026-11-04 | `phone_access.rs`: renders the local `hollow-companion://pair` pairing link as an SVG QR code | Zero additional dependencies with `image` feature off | OSV lookup pending next online refresh. Encodes only Hollow-generated pairing URLs; no untrusted input parsed. |
| tauri-plugin-updater | `~2.12` (`default-features = false`, `features = ["rustls-tls"]`; pinned for Tauri 2 minor parity) | Updates lane U5, 2026-11-14 | `update_client/fetch.rs`: channel manifest check and minisign-verified `.app.tar.gz` download only (`install()` is never called; `tar` 0.4 + `flate2` 1 unpack with path checks; 400 MiB / 25 min cap) | Adds `tauri-plugin-updater` plus `reqwest` 0.13 (and `tar`/`flate2` directly declared in `Cargo.toml`) | OSV lookup pending next online refresh. Talks only to configured update host over rustls TLS; verifies minisign signature before unpack. |
| reqwest_updater (`reqwest`) | `0.13` (`default-features = false`, renamed) | Updates lane U5, 2026-11-14 | `update_client/fetch.rs`: exposes the redirect-policy type matching `tauri-plugin-updater`'s `reqwest` 0.13 so update-token requests follow redirects on the update host alone | Already pulled into lock graph by `tauri-plugin-updater` (leaves dual `reqwest` 0.12 + 0.13 in workspace until unified) | OSV lookup pending next online refresh. |
| rquickjs | `0.14.0` (`features = ["allocator"]`; MIT, bundled QuickJS-ng C compiled via `cc`) | Code mode CM1a, 2026-11-17 | `codemode.rs`: sandboxed JavaScript execution for agent tool scripts (no `os`/`std`/network/timers/`quickjs-libc`, no `loader`/`futures`, `SharedArrayBuffer`/`Atomics` removed, 32 MiB counting allocator cap, 512 KiB stack, 10 s CPU / 10 min overall deadline, max 50 calls; inner calls re-authorized through `app_broker`) | Adds `rquickjs`, `rquickjs-core`, `rquickjs-sys`, and optional `relative-path` 2.0.3 (locked, not built) | OSV lookup pending next online refresh. Executes agent-authored JS inside the isolated QuickJS-ng runtime; monitor QuickJS-ng / `rquickjs` advisories closely. |
| tree-sitter, tree-sitter-rust, tree-sitter-typescript, tree-sitter-python | `=0.26.13`, `=0.24.2`, `=0.23.2`, `=0.25.0` (MIT; pinned for MSRV 1.89; default features, no wasm/bindgen) | Code context CC2, 2026-11-18 | `code_index/`: AST definition and name-scan indexing for Rust, TypeScript/TSX, and Python in `code-context-mcp` / `code-context` subprocesses and the shared MCP blocking worker (3 s per-file timeout, 2 MiB file cap, 40k file / 400 MiB index cap, skips hidden/secret/symlink paths) | Adds `tree-sitter`, `tree-sitter-language` 0.1.7, and the three C-grammar crates (built with `cc`) | OSV lookup pending next online refresh. Parses repository source files in connected git checkouts under timeout and byte caps. |
| portable-pty | `0.9` (MIT) | Terminal drawer parity 7.2, 2026-11-20 | `terminal.rs`: user-driven Preview pane shells (user-only plumbing commands; gated by chat permission mode; allowlisted environment via `env_clear()`, max 4 shells/chat, 12 total) | Adds `portable-pty`, `downcast-rs`, `filedescriptor`, `serial2` (unused), `shell-words`, `shared_library` (Windows only), `cfg_aliases`, and `nix` 0.28 | OSV lookup pending next online refresh. Local PTY management only; no network or telemetry code. |

### npm (`package.json`)

| Package | Version | Added by | Used for | Advisories |
|---|---|---|---|---|
| katex (+ `@types/katex`) | `^0.16.25` (`^0.16.7` dev) | Reply rich text, 2026-11 | `src/replyRichText.ts` (lazy reply chunk): bounded reply-only math rendering (`output: 'mathml'`, `trust: false`, `strict: 'ignore'`, `throwOnError: false`, `maxExpand: 1000`, `maxSize: 20`; inline ≤ 2,000 chars, display ≤ 8,000 chars; output still passes through `MarkdownView` DOMPurify sanitization) | `npm audit` lookup pending next online refresh. Untrusted model/reply text reaches `katex.renderToString` under the bounds above and `trust: false`. |

## How to refresh

```sh
npm audit --json                       # npm advisories
npm audit fix --dry-run                # what a non-breaking fix would change
node <script> src-tauri/Cargo.lock     # OSV querybatch over registry crates (see build log 2026-10-13)
cargo tree -i <crate> --target all -e normal   # is a flagged crate compiled in at all?
```

Never apply `npm audit fix --force`; a major toolchain upgrade (Vite, Vitest,
esbuild, Tauri) is its own change with its own build, canvas and smoke checks.
