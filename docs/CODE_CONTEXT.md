# Code context (`hollow_code_context`, CC2, CC3, CC4, CC5)

Agents in a git checkout ask "who calls this" and "what breaks if I change it" in
one small call instead of reading whole files and grepping. Design and spike
(CC1): the hollow workspace's `outputs/code-context-2026-11/design.md`. Built in
CC2 (2026-11-18); worktree overlays and the shared base index in CC3
(2026-11-18, [below](#worktree-overlays-and-the-base-index-cc3)); slices with
handles and deltas in CC4 (2026-11-18, [below](#slices-handles-and-deltas-cc4)); `changes`
and `affected_tests` in CC5 (2026-11-18, [below](#changes-and-affected-tests-cc5)); see the
[build log](BUILD_LOG.md).

## Tools

Nine tools, one surface (`code_context.rs`: `specs()`, `call(name, args)`), shared
by every transport, so each returns the same bytes:

| Tool | Arguments | Answers |
|---|---|---|
| `map` | `path?` | a folder's children with file and definition counts, or a file's definitions |
| `symbols` | `query`, `kind?`, `languages?`, `limit?` (1 to 100, default 20) | definitions by split name words (`skill valid` finds `valid_skill_name`); a near miss gets `didYouMean` |
| `callers` | `name`, `qualifier?`, `languages?`, `precision?` (`all`, `resolved`), `includeTests?` | every name-scan hit, grouped by file, each `resolved` with its `how` |
| `callees` | `name`, `qualifier?`, `languages?` | the calls inside the definition, each resolved to its target where a rule allows |
| `impact` | `name`, `qualifier?`, `languages?`, `depth?` (1 to 4, default 2), `includeTests?` | functions that reach it through resolved calls, by depth; `unresolvedHits` counted |
| `slice` | `symbol` or `task`, `qualifier?` (with `symbol`), `languages?`, `depth?` (1 to 2, default 1), `handle?` | cards for the symbol (or a task's 3 best matches), its callers and callees; with an earlier `handle`, only what changed, or `notModified` ([below](#slices-handles-and-deltas-cc4)) |
| `read` | `path`, `fromLine?`, `toLine?`, `symbol?` | numbered lines of an indexed file, at most 600 lines and 16,384 bytes counted as JSON escapes them; one line over that comes back cut, flagged `truncatedLine`, and `more` moves to the next line |
| `changes` | `base?` (a git rev; default the merge-base with `origin/HEAD`), `paths?` | the files changed since `base` (work tree and untracked files included), each with the definitions its hunks touch (`edited`, `added`, `deleted`); files it can't map under `unmapped` with a reason |
| `affected_tests` | `base?`, `paths?`, or `symbols?` (names, or `Type::name`), `depth?` (1 to 4, default 2) | `cargo test --lib` filters and test files that cover the change, each with `why` and `confidence`, a joined `run`, `cannotMap`, `missRate` and `fullGateStillRequired: true` |

Every input schema has `additionalProperties:false`; an unknown argument is
refused `unknown argument "qualifer"; did you mean "qualifier"?` (code
`INVALID_ARGUMENT`, `didYouMean`). Refusals use BI5's closed set with
`Server::Code` (`mcp_envelope.rs`). The tool list is held to a cap in
`tool_list_bytes_stay_under_their_caps` and in TC1's snapshot.

## The contract

- **Name scan is the floor.** `callers` lists every edge whose name matches; a
  failed resolution never removes a hit, it sets `resolved:false` and a reason
  (`ambiguous(n)`, `ambiguous-method(n)`, `qualifier-no-match(external?)`,
  `no-def`, `bare-ref-may-be-local`). `totals` always counts the whole scan;
  `precision:"resolved"` drops only unresolved hits.
- **Resolution only annotates** (`code_index/resolve.rs`): `qualifier`
  (`a::b::N`, `Type::N`, `Self::N`), `self-type` (`self.N()` in an impl with one
  N), `same-file` then `unique-name` (bare calls; a bare non-call reference only
  `same-file` and never to a method), `unique-method`, `import` (TS and Python
  relative imports), `invoke-string` (TS `invoke('n')` to the Rust fn, preferring
  `#[tauri::command]`). Rust macro arguments are scanned token by token and their
  hits carry `macro-args:` before the rule. A hit that resolves to another
  definition of the same name is `resolves-elsewhere`, counted in
  `totals.resolvesElsewhere`.
- **Checkable.** Hits are grouped by file with its 12-hex `fileHash`; each hit
  has its line and its caller's name and line range. Every result has
  `repo.head`, `repo.fingerprint`, `repo.indexedAt`, `repo.dirtyFiles`; read
  through an overlay (CC3), also `repo.base` (the base commit, 9 hex) and
  `repo.overlayFiles` (files the overlay replaces, adds or deletes). The files
  a result returns are re-hashed; a changed one is `stale:true` and the envelope
  gets `capReasons:["stale_files"]` (`complete:false`).
- **Budget.** OB1's envelope (`mcp_envelope::fit`), default 16,384 bytes here
  (hard max 80,000), resolved hits first, then path and line, paged by the `c1.`
  cursor with no hit lost or repeated. `maxBytes` missing or `null` is 16,384.
  The cursor's query hash folds in the index fingerprint, so after a re-index
  (an edited file) an old cursor is refused (`CURSOR_INVALID`, "drop cursor to
  start over"), never re-sliced.
- **Definitions.** `callers`, `callees` and `impact` list at most 20
  `definitions` (by path and line) with `definitionsTotal`; past 20 a
  `definitionsNote` asks for a `qualifier`. A resolved hit names its definition
  by index (`def`), or by `defAt` (`path:line`) when it is past the 20 shown.
- **Language filter.** Hits are limited to the definitions' language family
  (Rust, TS and TSX, Python) unless `languages` says otherwise; a Rust target
  also gets TS `invoke` hits; `totals.otherLanguages` counts the hits left out.
  Three or more definitions of the name and no `qualifier` adds `advice`.
- **Impact cap.** Past 200 functions `impact` refuses and names `callers` on the
  busiest definitions.

## The index

`code_index/` (`mod.rs`, `parse.rs`, `resolve.rs`, `store.rs`): tree-sitter
0.26.13 with the Rust 0.24.2, TypeScript 0.23.2 and Python 0.25.0 grammars (all
MIT, pinned exactly in `src-tauri/Cargo.toml`). One SQLite file per checkout
(`files`, `names`, `nodes`, `edges`, `sym_fts` FTS5 over split identifiers,
`meta`), in the OS cache folder under `hollow/code-index/<12-hex of the root>`,
never inside a tracked tree. A call rebuilds it when the listed files' paths,
sizes or modified times changed (`ensure`): a full rebuild into a new file,
swapped in by atomic rename. Since CC3 that full index is the fallback: a
checkout with a commit reads a shared base plus its own overlay (next section).

What it reads: only files `git ls-files --cached --others --exclude-standard`
lists, with a `.rs .ts .mts .cts .tsx .py` extension (`.mjs` and `.js` are left
out); never a hidden or secret-looking path (`directory_connections::
secret_name`), never a symlink, never a path that resolves outside the root (a
symlinked parent folder: listed in `repo.skippedOutsideRoot`; files are opened
by their canonical path with `O_NOFOLLOW`), never a file over 2 MB (skipped and
listed in `repo.skippedOverCap`). Each parse has a 3 s timeout
(`repo.timedOut`).

Nothing in the repo runs. The only subprocess is `git`, built by
`repo_diff::hardened_git` (`code_index::Git`): `GIT_*` stripped,
`--no-optional-locks` (status never takes `index.lock` under an agent's
commit), fsmonitor, hooks, external diff, submodule recursion and every filter
driver the repo defines off through `GIT_CONFIG_COUNT`, git dir and work tree
pinned after one `rev-parse`, `status --ignore-submodules=all`, stdin closed,
30 s per call.

**Bounds** (`code_index::LIMITS`): a checkout over 40,000 code files or 400 MB
of them is refused before anything is read, and a build stops at 180 s or when
the MCP request that asked for it is cancelled (`shared_mcp::request_current`,
checked between files). Those refusals lead with `REFUSED:` and name the
limit. **One build per index file:** an in-process mutex per index coalesces
callers (the second waits and reuses the first's index), a `.<index>.lock` file
created exclusively keeps other processes out (a lock older than 10 minutes is
broken; a waiter gives up at the deadline), the stamp is re-checked after each
wait, each build writes its own `.<index>.<pid>.<n>.building` temp file, and a
successful build prunes this index's temp files older than an hour.

## Worktree overlays and the base index (CC3)

`code_index/overlay.rs`, design 2.7. A call reads `code_index::open`:

- **Base**: the index of one commit, built from that commit's blobs (`git
  ls-tree -r -l` and `git cat-file --batch` through the hardened `Git`, never
  the work tree; same file filter, byte cap and limits as a work tree). One
  file per repository and commit, `base-<12 hex of the canonical git common
  dir>-<commit>.sqlite` in the index folder, so every worktree of a repository
  shares it; immutable once swapped in (built under the same lock, temp file
  and atomic rename as the full index). Meta `kind=base`, `base=<commit>`;
  `indexedAt` is the commit's time, so a build is a pure function of the commit
  and the same commit gives the same bytes (tested), unless a file's parse hits
  its 3 s limit (then named in `timedOut`). At most 3 generations per
  repository are kept, the least recently used pruned (a read marks one used),
  but never one used or written in the last 30 minutes. Base builds take a
  per-repository lock (`.base-<key>.lock`) and then pick again, so a second
  worktree waits and takes the first's base when its diff is small. Thrash
  left: a repository whose worktrees sit on more than 3 merge-bases at once,
  each with a diff over 500 files from the others, still builds a base per
  merge-base and keeps up to all of them for 30 minutes past the cap.
- **Which base**: the merge-base of HEAD and `origin/HEAD` (HEAD without
  one). When that file is missing, a kept generation whose `git diff
  --name-only` from the work tree is at most 500 files stands in for it, so a
  fresh worktree answers without a build; with none, the base is built (as
  long as a full index takes, once per repository).
- **Overlay**: per worktree (`<12 hex of the root>.overlay.sqlite`), rows only
  for files whose content differs from the base (`git diff --name-only
  --no-renames <base>`, plus every assume-unchanged or skip-worktree file from
  `git ls-files -v`, whose in-place edits that diff misses, plus anything
  listed here and not in the base, or the reverse, then the 12-hex SHA-256
  compared) and untracked, unignored files. A
  deleted or now unreadable file is a tombstone. Tables as the full index plus
  `hidden` (base paths it replaces or deletes) and `edge_fix` (base edges whose
  annotation changes). Node ids continue after the base's. Rebuilt when CC2's
  stamp of the work tree or the base changed. The base's identity (its
  highest definition id and edge rowid, stamp, fingerprint and timed-out
  files, read from the open connection) is in the overlay's stamp and its
  `baseIdentity` meta; `open_overlay` refuses an overlay built on another base
  file (one swapped in between check and attach) and the call rebuilds it
  once. An "overlay unusable" verdict is cached per overlay stamp.
- **Resolution**: overlay edges resolve against the merged definitions
  (overlay definitions win: the base's from a changed file are gone). Base
  edges are re-resolved from their stored rows, a name lookup with no
  re-parse, only for names defined in changed files (old and new versions)
  and names a base relative import now finds in another file (an added or
  deleted path); those whose target or `how` changed become `edge_fix` rows.
  A test checks that every answer (defs, edges, callers' targets, search,
  names, files) and the fingerprint equal a full index of the same work tree.
- **Query**: base rows whose path is not in `hidden`, plus overlay rows, merged
  in the same order a full index gives (`store::Index::open_overlay`, the
  overlay attached read only). `fingerprint` is CC2's over the merged files,
  so the cursor binds to base plus overlay: an edit refuses an old cursor.
- **Fallback**: no commit, an overlay over 2,000 files and over half the
  base, or any other failure to get a base reads CC2's full index; a limit
  refusal is returned as is.
- **Reads**: the stale check and `read` read an index path only through
  `code_index::read_index_path`: a readable code path (relative, no `..` or
  hidden part, nothing secret-looking), canonical inside the root, a regular
  file opened `O_NOFOLLOW|O_NONBLOCK`, at most the byte cap. A cache file
  can't steer a read. Every index connection sets `trusted_schema=OFF`.

### Building the base on the Mac mini

`hollow code-context build-base --repo <path> --commit <sha> --out <file>`
indexes that commit into `<file>` (temp file, then rename) and prints one JSON
line: `commit`, `out`, `sha256`, `bytes`, `files`, `nodes`, `edges`,
`skippedOverCap`, `timedOut`, `schema`, `parsers`, `seconds` (exit 1 on a
refusal, 2 on usage). Its deadline is 900 s. `hollow code-context import-base
<file>` copies a base into this Mac's index folder for the current checkout's
repository. The source must be a regular file (not a symlink or FIFO) of at
most 2 GiB; the copy is 0644. It checks that private copy: exactly a base's
tables and indexes with the same SQL (no view, no trigger), definition ids
1..=n and contiguous per file, every edge's file, source and target present,
SQLite `quick_check`, `kind=base`, this Hollow's schema and parser versions, a
commit this repository has, and every file path readable and of its language,
the (path, size) set equal to the commit's filtered `git ls-tree`. It refuses
otherwise ("nothing was imported"), then swaps it in by atomic rename and
prints `imported`, `commit`, `sha256`, `bytes`, `replaced`. Both commands
refuse the home folder and a root that holds or sits inside the vault.

The runner has no installed Hollow, so a lane or the orchestrator builds it
there from the checkout with `cargo run` (a debug build of the branch's code;
the runner keeps that worktree's build folder), from the checkout's top level:

```sh
base=$(git merge-base HEAD origin/HEAD)
hollow-slot --remote --fetch src-tauri/target/code-index/base.sqlite \
  sh -c "cd src-tauri && cargo run --quiet -- code-context build-base --repo .. --commit $base --out target/code-index/base.sqlite"
/Applications/Hollow.app/Contents/MacOS/hollow code-context import-base src-tauri/target/code-index/base.sqlite
```

`src-tauri/target/` is ignored, so the file never reaches git; `--fetch`
brings it back over the same path here. On the work tailnet use `--on
runner-host-lan` for `--remote`. Until the installed Hollow has CC3, run the last
line as `(cd src-tauri && hollow-slot --here cargo run --quiet -- code-context
import-base target/code-index/base.sqlite)`. Nothing is installed on the mini
and no job or service runs there: a scheduled base build, or a `hollow` binary
on the runner, is a manual step left for Alex.

## Slices, handles and deltas (CC4)

`code_context_slice.rs` (a child of `code_context`). `slice` answers "what do
I need to know to change this" as a list of cards, and lets an agent that
already holds them ask again for only what changed.

- **Targets.** `symbol` takes the definitions `callers` would (`qualifier`,
  `languages`), the first 5 by path and line (more adds a `note` asking for a
  qualifier); no definition is `NOT_FOUND` with a did-you-mean. `task` is free
  text: its words, less common ones (`fix`, `the`, ...), each go through the
  name search (split identifiers, a prefix each), and the 3 definitions that
  match the most words win (a repeated word counts once; then non-test,
  shorter name, path, line);
  `target.matched` names them. One of the two is required, not both;
  `qualifier` goes with `symbol` only.
- **Cards.** The targets, their callers (`edges_to`) and their callees
  (resolved calls only) to `depth` hops, each definition once, in order:
  targets, then by distance, path and line. A card: `id` (`path:Container::
  name`, `#2` on a repeat), `name`, `kind`, `path`, `lines`, `container`,
  `module`, `test`, `tauriCommand` as `def_json` writes them, `rel` (`target`,
  `caller`, `callee`, `both`), `distance`, `signature` (the first lines up to
  the body's `{` or a Python header's `:`, at most 4 lines and 300
  characters), `bodyHash` (the definition's own lines), `cardHash`, `fileHash`
  and `stale` when the file changed since the index. `cardHash` hashes the
  card's content less `lines` and `fileHash`: an edit elsewhere in a file
  never changes it, and a definition that only shifted is a move. A delta
  therefore does not resend a card whose file changed elsewhere, so a held
  card's `fileHash` can be older than the file's. More than 300 definitions
  is refused ("over the slice cap", `INVALID_ARGUMENT`).
- **Handle.** Every answer carries `handle`, `h1.` and 16 hex of SHA-256 over
  the checkout root, the arguments as used (`symbol`, `qualifier` and `task`
  trimmed, `depth` resolved, `languages` sorted; not `handle`, `maxBytes`,
  `cursor`) and each card's id, hash and lines: the same slice gives the same
  handle, through MCP or the CLI, so `{"symbol":"a"}` and
  `{"symbol":"a","depth":1}` share one. Its state (the arguments' hash and the card
  list) is a JSON file in the index folder's `slices/`, leased 30 minutes
  from its last use (a read refreshes it), at most 64 kept (expired ones and
  then the least recently used are pruned on each write), written through a
  temp file unique per call, then a rename. Called with
  `handle`: nothing changed is one text block, `{"handle":"h1...","notModified":true}`
  (no envelope). `notModified` means no card changed, not that no file did:
  an edit elsewhere in a card's file leaves the held `fileHash` (and `stale`)
  as they were; call without `handle` for fresh ones (tested). Otherwise `cards` holds only added and changed cards (each
  `change: "added"|"changed"`), `moved` lists `{id, lines}` of cards whose
  content is the same and whose lines moved, `removed` lists the ids gone,
  `since` is the old handle and `handle` the new one; `totals` counts each. A
  handle that is malformed, unknown or past its lease ("this handle is not
  one a slice gave here, or its 30-minute lease ran out; drop handle ...")
  or was given with other arguments ("belongs to another slice") is refused
  with `CURSOR_INVALID`; an expired state file is deleted. The CLI twin is a
  new process per call, which is why the state lives on disk, not in memory.
- **Budget.** Cards page with the envelope cursor like every list here, whole
  cards only, bound to the index fingerprint (an edit refuses an old cursor).
  A delta's pages page its changed cards; `moved` and `removed` ride on the
  first page, each at most 100 entries and a quarter of `maxBytes`, so that
  page stays within budget; when either is cut, `listsCut` says how many of
  `totals.moved`/`totals.removed` are listed and to drop `handle` for the
  whole slice.
- **Overlays.** Through CC3's overlay a worktree edit changes only the cards
  of the definitions whose lines changed (tested on a committed repo); a
  definition below an edit that adds lines comes back in `moved`.

## Changes and affected tests (CC5)

`code_context_changes.rs` (a child of `code_context`). A lane that changed
three functions should not have to run the whole suite to learn it broke one:
`affected_tests` names the tests worth running first. It never replaces the
full gate.

- **`changes`.** `git diff --name-status -z --no-renames` and `git diff -U0`
  from `base` (through the hardened `Git`, plus `--no-ext-diff --no-textconv
  --no-relative`, fixed `a/` `b/` prefixes and no inter-hunk context; stdin
  closed) and `git ls-files --others --exclude-standard` for untracked files.
  A hunk's new lines map to the index's definitions (the file now), its old
  lines to the definitions of the base's blob, parsed in process (`git
  cat-file --batch`, at most 2 MB a file; nothing runs). Innermost: a `mod`,
  `trait`, `class` or `iface` drops out when a definition inside it is touched
  too. A definition only the new side has is `added`, only the old side
  `deleted` (with `oldLines`); a rename is a delete plus an add; a deleted
  file lists its old definitions. Lines outside every definition are counted
  (`linesOutsideDefinitions`). `unmapped` gives a reason per file: `.mjs`
  and `.js` (not indexed), config or data (`.toml`, `.json`, `.yaml`, ...),
  fixtures, generated or lock files, binaries and assets, docs, scripts, and
  indexed-language files the index skipped. Rows page by file (at most 100
  symbols a file, `symbolsTotal` past that). Over 2,000 changed files is
  refused ("over the changes cap"); an unknown `base` is `NOT_FOUND`.
- **`affected_tests`.** From the changed definitions (or `symbols`), a walk
  over callers: resolved calls; a helper in test code is followed at the same
  level; `depth` counts levels of non-test callers; a Rust name with at most
  3 definitions also follows its unresolved calls (`name-match`; not in
  TypeScript, where it listed 213 files for one PR). Confidence, best first:
  `direct` (a test calls it), `transitive` (through callers), `import` (a
  TypeScript test file importing a changed file, one level), `same-file` (the
  changed file's own `#[cfg(test)] mod`), `same-module` (a `#[path]` test file
  the changed file declares), `name-match`. Rust tests are `cargo test --lib`
  filters whose module path follows `#[path]` attributes (`b_tests.rs`
  declared as `mod tests` in `b.rs` is `b::tests`), run from the crate folder;
  one test in a module is its full path, more (or a whole test module) are
  `module::`. `run` joins them into one cargo command per crate and one
  `npx vitest run` (when under 3,000 bytes). `cannotMap` (15 shown, all
  counted) names what the walk can't follow: references inside macro
  arguments that don't resolve, method calls on receivers of unknown type
  (trait objects, generics), Tauri commands called by `invoke` string,
  deleted definitions, config and fixture changes, files outside the index,
  callers past `depth` or the 2,000-function cap. `missRate` is the measured
  rate below; `fullGateStillRequired` is always `true`.
- **How a lane uses it.** Before its gate, a lane calls `affected_tests`
  (no arguments: its diff against the merge-base) and runs `run` through the
  night's slot, for example `bash local.sh <lane> sh -c 'cd src-tauri &&
  cargo test --lib -- a::tests:: b::tests::gamma_is_two'`, then the full gate
  as before. `scripts/gate-run.mjs` is unchanged: the gate stays whole.

### Evaluation (2026-11-18)

`code_context::changes::tests::evaluate_on_merged_prs` (ignored; reads only
git objects): for 10 merged PRs, the index of the merge commit, the diff from
its first parent with test files and test functions left out, and as truth
the tests each PR added or changed (Rust `#[test]` functions, TypeScript test
files). Listed rows are with the tool's rules (name-match in Rust only, one
level of import walk).

| PR | Change | Listed rows | Truth | Hits | By call graph | Misses | Unmapped files |
|---|---|---|---|---|---|---|---|
| #1584 | settings sections (TS and Rust) | 23 | 2 | 2 | 2 | 0 | 3 |
| #1563 | budgeted handoff | 32 | 8 | 8 | 8 | 0 | 2 |
| #1564 | recall trace | 28 | 7 | 7 | 7 | 0 | 3 |
| #1554 | peer link rules | 16 | 6 | 6 | 6 | 0 | 5 |
| #1537 | parallel admission | 46 | 8 | 8 | 8 | 0 | 4 |
| #1573 | fetch on demand | 32 | 8 | 8 | 8 | 0 | 6 |
| #1557 | serve script runner | 37 | 5 | 5 | 4 | 0 | 7 |
| #1555 | versioned hello | 3 | 5 | 5 | 4 | 0 | 6 |
| #1575 | stale demotion | 114 | 9 | 9 | 9 | 0 | 4 |
| #1585 | branch chat | 101 | 5 | 5 | 5 | 0 | 5 |

0 of 63 missed; 2 of them were found only by a same-file or same-module
filter (3.2% without those rules). Rules compared (rows, misses): name-match
everywhere and import walk to `depth` 725 rows, 0; name-match in Rust only
495, 0; import walk one level 679, 0; both (the tool) 432, 0. Truth is a
proxy: a PR's own tests usually sit beside its change, so this measures
recall on tests near the change, not on distant ones. The listing is not
always lean: #1575 and #1585 list over 100 rows (a hub file in TypeScript,
many callers in Rust), against about 6,500 Rust tests and 456 TypeScript test
files in all.

Breaks on purpose (a scratch commit in the worktree, reverted, never
pushed; the listing from `list_for_worktree` against `HEAD~1`):

| Break (PR) | What broke | Run | Failed | Listed |
|---|---|---|---|---|
| 1 (#1557) | `app_mcp::broker_wait_secs` gives a read action the change timeout | `cargo test --lib app_mcp` (29 tests) | 1, `app_mcp::tests::wait_for_session_is_a_bounded_read` | yes: `app_mcp::tests::` (`direct`), the only row |
| 2 (#1573) | `sync_files::here` inverted | `cargo test --lib -- sync_files core_boot` (41) | 7, all in `sync_files::tests` | yes: `sync_files::tests::` (`transitive`); 4 more rows (`sync_engine::vault_tests::` and 3 `name-match`) passed |
| 3 (#1584) | `resolveSettingsTarget` loses its moved-control branch | `npx vitest run src/settingsResolve.test.ts` (20) | 8 | yes (`direct`), one of 10 files listed |

All 16 failing tests were listed. Break 3 ran one file, so failures in the
other 9 listed files were not checked. It also showed a bug: a Playwright
spec (`tests/smoke/provider-usage.spec.ts`) was listed as a vitest file; a
`.spec.` file is now `npx playwright test`.

## Transports and attach

- **MCP** `hollow_code_context`: the hidden `code-context-mcp` subcommand
  (`code_context_mcp.rs`, stdio), and the shared MCP endpoint
  (`shared_mcp_host.rs`), for Claude and Codex.
- **CLI twin** `hollow code-context <tool> '<json>'` (`hollow code-context tools`
  prints the schemas): stdout is the MCP result's text blocks, one per line,
  byte for byte (exit 1 on a refusal); the root is `HOLLOW_CODE_ROOT`, else the
  cwd's git top level, refused like the attached server's (below; the vault is
  `HOLLOW_CODE_VAULT`, else config.json's). The Settings toggle is app state
  the CLI can't read; its help says so. Grok and Antigravity are told the command in the
  capability contract's `NO_OWNED` line. Never reads stdin.
- **Attach** (`mcp_attach::server_map_for`): chats and workflows of an owned-MCP
  agent (Claude, Codex) whose cwd is inside a git work tree and not inside the
  vault, whose git root is not the home folder and does not hold the vault
  (`code_context::root_refusal`, also checked by the server on every call with
  `HOLLOW_CODE_VAULT` from the entry), while Settings → Connectors → "Look up code: callers and impact"
  (`codeContextMcpEnabled`, action
  `settings.agent-tools.set_code_context_mcp_enabled`) is on. Read only, so plan
  and unattended runs keep it and it is preapproved in every mode; a bare run
  drops it with everything else. `HOLLOW_CODE_ROOT` is the cwd's git top level.

## Left for later

A scheduled base build on the mini (held for Alex); a larger
`affected_tests` evaluation (distant tests, not only a PR's own). Not done here: Rust `use`
resolution, TS path aliases, receiver-type hints for `.method()` calls.
