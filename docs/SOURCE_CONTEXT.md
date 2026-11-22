# Sources and workspace context

Current implementation, 2026-09-24. This completes the shell/source-connection
pass of the [product reset](history/HOLLOW_PRODUCT_RESET.md); it does not claim the
reset's deferred rich editors, parallel runs, or latency targets are complete.

## One path from evidence to work

Hollow folder's Projects → live Library files → query-selected agent evidence → vault outputs.
Vault Library material can still be attached as an explicit workspace snapshot.
An output can be attached to another workspace through the same Library picker.
Each request retrieves relevant local evidence instead of inserting the entire collection.
Account authentication and connector sync remain separate.
See [Projects and Vault directories](DIRECTORIES.md) for the current connection contract.

| Material | Local home | How it enters work | Update behavior |
| --- | --- | --- | --- |
| External repositories and project documents | The Hollow folder's `Projects/` (or a linked existing folder) | Auto-discovered workspaces; Library → Connected projects; cited query-selected evidence | Startup/minute reconciliation, manual refresh, live filtered reads |
| Reference notes and imports | `library/`, `library/imports/<slug>/`; retained older folders | Query-selected local evidence; optional Library → Use in this workspace or Sources → From Library | Explicit snapshot; originals unchanged; reviewed refresh |
| Native local files | User-selected file | Sources → Choose local file | Snapshot, 20 MB/file maximum |
| Tracked projects / tasks | Linear mirrors (`PROJECT.md`, `TRACKER.md`, task notes); legacy Notion notes and retained `NOTION.md` | Linked workspaces use generated `TRACKER.md` and still read a retained `NOTION.md`; other records can be retrieved or attached | [Tracker sync](TRACKER_SYNC.md); Notion two-way sync removed 2026-11-10, Notion is a read-only archive (Notion's official connector reads it once signed in); attachments remain snapshots |
| Meetings | Calendar: the source connected in Settings → Connectors (not in the vault); retained `calendar/` notes | Agenda; a shared connector, when the request is about a meeting | Outlook and the `calendar/` sync were removed on 2026-09-26; one wording in `src/meetingsCopy.ts` (P1-20) |
| Plaud | Plaud (not in the vault) | Two accesses: Recordings in Hollow (Hollow's own sign-in, read on demand; Keep a transcript copy writes one into a workspace's sources) and Plaud tools for agents (the connector, its own sign-in) | The `recordings/` import was removed on 2026-09-26; its earlier files are Library's Kept transcripts |
| People | `people/` | Library People collection → attach | Relevant local retrieval, no separate CRM surface |
| Results | `workspaces/<id>/outputs/`, retained `briefings/` | Library Result files collection → attach to another workspace | Reusable captured evidence |
| Profile and learned recall | `me/` and retained learning records | Attach specific notes when needed | No automatic injection into workspace chat or workspace automations |
| Conversation history | Hollow SQLite and bounded `sessions/` records | Workspace Activity; Library → Conversations (also Library → History → Conversation history) | Exact provider resume where supported; historical context is evidence |

The vault Library view uses the existing supported-note index. Connected projects
use a separate ignore-aware metadata inventory over supported text/code and
document formats, without importing originals. Binary attachments use the native file picker. Collection filters
are centralized in `src/sourceCatalog.ts`; retired message folders stay excluded.
Sources → From Library has one search (review F04, 2026-10-13): **Find**
filters every Library file by name or path, the file list (not itself
searchable) shows the first `SOURCE_PICKER_LIMIT` (100) matches, and
"Showing the first 100 of N matching files" appears only when the list is
truncated (`sourcePickerWindow`). A chosen file that the filter hides is
unchosen, so Attach snapshot never attaches something off screen.
Import a source (the Add sheet set to a Library source since S5,
2026-11-03) runs the existing guarded extraction engine and now writes new
notes under `library/imports/`. Old `ingested/` material is retained and readable.

**Links, for any agent** (repo ideas 2026-10-27 #6). A link that isn't YouTube
is fetched by Hollow itself (`src-tauri/src/public_page.rs`), not by the
agent: `http`/`https` on the default port with no user name or password;
every host and redirect hop (at most 5, each re-vetted) a public name that
resolves only to public addresses, the connection pinned to the addresses
checked (`remote_image.rs`'s gate: no loopback, private, link-local and so
cloud metadata, CGNAT or reserved space; no proxy, cookies or referer); HTML
or plain text only, at most 2 MB, 20 s in all; one URL per import, never a
crawl. A sign-in wall or a browser check ("Just a moment…") is refused as
content. Its readable text (its `main`/`article` when one holds 40+ words,
else the body; scripts, styles, navigation, footers and forms dropped; at
most 60,000 characters) becomes a `page` source, which the frontend can't
send: the agent gets it as "public page, signed out", untrusted page content,
with no web tool (`Read` and `Write` only), so Claude Code and Codex's
tool-free transform import a link alike. Grok and Antigravity can't be
limited that way yet, so Hollow refuses to hand them a page, before
fetching, and says to use Claude Code or Codex. YouTube is recognised by its
own hosts and paths only (a parsed URL, never a substring) and stays on its
own path (yt-dlp through Claude Code). The page's import folder is its
link's, plus a hash of the whole link, so two links never share one. Cancel drops the fetch at once. A sign-in wall is a password field
on a page under 5,000 characters, a sign-in title under 3,000, or a short
page asking to sign in; a browser check is a page under 2,000 characters
that says so. The gate also refuses local-use NAT64 (`64:ff9b:1::/48`),
discard, IETF-assignment, the newer documentation range and SRv6 IPv6 space
(shared with click-to-load images). Agents reach link import only through
`ingest_run` (Confirm tier), one URL per call, through the same gate.

## Context and provenance contract

Each managed-workspace chat or automation loads the following explicit context,
plus the bounded query-selected evidence described in [directory retrieval](DIRECTORIES.md)
(since repo features R14b, connected project files are chosen by their contents through a local
BM25 index when Settings → Advanced → Search & indexes → Search inside project files is on; it is off by default):

- `WORKSPACE.md`, `PLAN.md`, `SOURCES.md`, `AGENTS.md`, optional `NOTION.md`:
  48,000 bytes combined. Size never fails a turn (since 2026-09-29): smaller
  files stay whole, the largest are cut to an equal share at a UTF-8 boundary,
  and a cut file ends with `[Hollow: cut to fit. Only the first N of M bytes
  are included; read FILE in the workspace folder for the rest.]`. The user
  sees it too (durability D3, 2026-10-01): `read_workspace` returns
  `contextHealth`, one `{name, bytes, included, limit}` per document present,
  computed from the listing's file sizes with the same `context_shares`
  arithmetic (no extra reads; empty for Chats). The old composer explanation
  panel is removed; the usage ring shows recorded message shares and links to
  detailed diagnostics. The 128,000-byte envelope trim below is not reflected.
- Attached UTF-8 text evidence, chosen for the request (agent effectiveness
  F3 part 2, 2026-10-06; until then, name-ordered prefixes of at most 8,000
  bytes per file, so a fourth file by name could get nothing): each readable
  file (at most 1 MB read, 16 MB per request) is cut into chunks of at most
  40 lines and 3,000 bytes, 8 lines shared with the next; chunks are ranked
  against the request and the names its references resolved to, matching
  chunks first, then each file's opening chunks in turn for what is left,
  into one 24,000-byte budget across files (`retrieval_broker::
  select_attached`). Each file says what the agent has in a one-line `note`
  ("excerpt: lines 120–160 of 400"; a file with no matching passage gets a
  presence note with its size) and carries its `excerpts` as
  `{from, to, text}` line ranges. Cuts respect line and UTF-8 boundaries.
- An on-demand reader for the rest: in chats with owned MCP (Claude, Codex),
  `hollow_vault`'s `read_source {path, from_line?, to_line?}` returns numbered
  lines (at most 600 lines or 24,000 bytes per call) of the run's own
  workspace files, vault notes retrieval may cite and in-scope connected
  project files, nothing else. Without it (Antigravity, Grok, automations)
  the preamble names the workspace folder for the agent's own file tools.
- Source coverage, local origin, capture date and freshness metadata for at
  most 100 source files (the first 100 in path order). Files past that, and
  entries that can't be read, are counted in `omittedSourceFiles` instead;
  the walk stops counting at 10,000 entries. Attaching refuses the 101st file,
  but agents and Finder can write into `sources/` directly, so the context
  never assumes the cap held. Total encoded envelope is capped at 128,000
  bytes: past it, source excerpts go first (each file then says
  `budget_exceeded`, and the run manifest records the trim), then the source
  list (counted as omitted), then the largest document is halved until it
  fits.
- When anything was cut or omitted, and only then, the preamble adds
  `CUT_NOTE` (`workspaces.rs`) telling the agent to read those files directly;
  every other workspace's context is byte-identical to before.
- Optional `goalAlignment` (Paperclip PR 10): why the work matters, from the
  workspace's project and that project's linked goals. At most 2,048 bytes
  after the envelope's escaping, NOT counted in the 48,000-byte file total, and
  omitted (logged) rather than pushing the envelope past 128,000 bytes. The
  project is found by a legacy `notion_project_path`, then an optional
  hand-set `project_url:` in `WORKSPACE.md`, then a unique exact title match;
  flat Notion notes (`projects/notion-<id>.md`) resolve, and a path left stale
  by a status move is found by its filename. Only goals that parse are used
  (active first, then paused, achieved, dropped); unreadable links are listed
  (or counted) and never read as content. The block is absent when the
  workspace resolves to no project or no linked goal parses; a hand-set
  `project_url` that resolves to nothing, with no title match, sends only a
  flag telling the user to restore the note or correct the link. A preamble
  sentence, added only with the block, frames it as reference evidence: not an
  instruction and no licence to search `projects/` or `goals/`.

Coverage per request is `complete`, `excerpt`, `not_selected` (no passage
matched; the file is there), `budget_exceeded` (its matching passages did not
fit, or the 16 MB read limit was reached) or `requires_reader`, and the run
manifest's `attachedSources` records each file's coverage and every kept or
matching chunk (path, line range, score, kept, bytes; never text). The
Sources panel has no request, so it shows what any request can get (since
Canvas C5b as cards joined with the `SOURCES.md` entries; see
[CANVAS_C5.md](history/CANVAS_C5.md#sources-as-cards-c5b--q2)): `complete` when all
attached text fits 24 KB together, `excerpt` ("Sent when relevant": the
passages that best match each request) otherwise, and `requires_reader`.

Attached PDFs and Word, PowerPoint and Excel files (repo features R14a) are
read by Hollow: a file with that extension whose first bytes are the format's
(`%PDF-`, or a zip's `PK`) is searched through its extracted text
(`doc_text.rs`: `## Page N` per PDF page, one paragraph per line for Word,
`## Slide N` per slide, `xlsx_cells` for a workbook), at most 1 MB of it per
file and counted against the same 16 MB per request, at most 40 documents and
16 MB of them on disk per request (the rest are `budget_exceeded`, their note
saying how many were left unread), and `read_source`
(`sources/…`) numbers the same text, so an excerpt's lines and a read agree.
The Sources panel never parses a document (it may run on an async worker): it
shows `excerpt` until a request's read finds otherwise, then what that read
found (`requires_reader` for an image-only or unreadable PDF). A source whose
path has a hidden or secret-looking name (`passwords.xlsx`,
`credentials.md`, a `secrets/` folder: `read_source`'s `forbidden_component`
rule) is listed as `requires_reader` and its text, document or not, is never
sent; its note says it was withheld because its name looks like a secret and
that `read_source` refuses it, not that it needs a reader (which would invite
an agent to open it with its own tools). A renamed
non-document, an image, invalid UTF-8 and other unsupported content is not
silently treated as read. A runtime needs suitable tools or a text export for
that material. This particularly matters for the currently verified tool-free
Codex adapter. Clipping is disclosed, and missing evidence is never verified
zero.

New attachments retain the human-readable `SOURCES.md` ledger and add
`sources/.provenance/<basename>.json` with camelCase `original`, `captured`,
`sha256`, `size`, and `modified`. The sidecar records the original's size/time
and the captured bytes' SHA-256. It is additive; older attachments work without
it and report `unknown` origin freshness. Filenames are never used to infer
connector identity. Hidden sidecars are excluded from file browsing.

Freshness is a cheap size/mtime comparison against the original, not a live
connector check or a repeated hash verification. Values are `unchanged_metadata`,
`changed`, `unavailable`, and `unknown`. A source can be stale remotely even when
its local metadata is unchanged. Snapshot checksums remain provenance, not a
claim that an external editor has never changed the copy. Snapshots are read-only
in the workspace and Library UI, not protected by filesystem ACLs. Attaching
never overwrites: a second attach of the same name is refused.

### Snapshot, live, synced: one vocabulary (plan L3, 2026-10-14)

Every attachment and read point says what the material is, a plain summary
first and the exact provenance in a secondary line (`src/sourceAuthority.ts`,
`sourceStateLine`):

| Kind | Where | Summary |
| --- | --- | --- |
| Snapshot | a workspace's `sources/` | "Snapshot captured 2 Oct; doesn’t update on its own" / "…; the original has changed since" / "Snapshot; its original can't be found" |
| Live | Library → Connected projects | "Live file from <project>: read each time, never copied" |
| Tracked | Tracker-mirrored tasks and projects | "Tracked in Linear: its fields come from there" (was Synced with Notion until 2026-11-10) |
| Output | `outputs/`, Results | "Made in <workspace>" |
| Note | Library notes | "Library note: edits save here" |

The Sources cards, today's Sources list, the Library reader, the Connected
projects reader, the palette ("Snapshot · <workspace>") and Library →
Results use it, and the file toolbar names the state ("Viewing",
"Reviewing" for a Result, "Editing", "Unsaved draft", "Saved", "Conflict:
not saved", "Snapshot · read only"). Nothing says a snapshot is current
without a content check.

Each snapshot offers **View snapshot** (the copy, read only), **Open
original** (in the Library reader when it is in the vault, else in its own
app; disabled with the reason when it can't be found) and **Check for
changes**. The Library reader's **Use in this workspace** attaches the open
file; when the chosen workspace already holds a snapshot of it, **View
snapshot** replaces the button.

### Reviewed refresh

Check for changes calls `preview_source_refresh {id, path}` (reads only):
the snapshot, the recorded capture and the original are hashed, and both
texts come back when both are UTF-8 and at most 1 MB, for the changed lines
(`lineDiff`). "The original matches the snapshot (same content)" is a
content check, stronger than the size/date freshness. Replace snapshot calls
`apply_source_refresh {id, path, expectedOriginSha, expectedSnapshotSha}`
under the vault mutate lock: if either side no longer hashes to what the
preview showed, it refuses with `SOURCE_CHANGED:` and writes nothing; it
also refuses an original that can't be read or already matches. Otherwise
the earlier snapshot is copied to `sources/.previous/<sha12>-<name>` (a
user click has no History run to record it in History & recovery), the
snapshot is replaced atomically, the sidecar gets the new
`sha256`/`size`/`modified`/`captured` and a `previous: [{sha256, captured,
kept}]` list, and SOURCES.md gets a new entry for the file with a
`- Refreshed:` line (same 48 KB bound and unchanged-file check as attach),
so the cards show the refresh as current and the capture before it as
earlier. A failed write restores the snapshot and sidecar. Both commands
are user-only: the preview returns the text of a file anywhere on this Mac,
and the sidecar naming that file is agent-writable, so the person sees the
original's path before Replace copies it into the vault.

The snapshot, `sources/`, `.provenance/`, the sidecar and `.previous/` are
taken literally and refused when any of them is a symbolic link (an
existing path through `safe_join` resolves to the link's destination, so a
linked snapshot would have replaced that file instead). A file already at
the kept copy's name is reused only when it hashes to the replaced bytes.
When the original still hashes to the capture and only the copy was edited,
the preview says so ("Replacing restores the original"), the sidecar's
`previous` entry also carries `capturedSha256`, and the `- Refreshed:` line
names the edited copy. **Open original** opens a vault original in Hollow's
Library reader and any other document in its app, but only reveals an app,
installer, script or extensionless file in Finder (`opensAsDocument`). The
backend decides again on the symlink-resolved target (`open_guard.rs`, H4):
`chat_preview_open` reveals anything but a regular file, an executable, or
a runnable extension (the same list, held together by a test) instead of
opening it, so a link named `report.pdf` that leads to an app or a script is
shown, not launched, and the card says "Shown in Finder". An executable (any execute bit) still opens only when its
extension is on `DOCUMENT_EXTENSIONS`, viewer/editor types that never run (pdf, txt, md, images, audio, video,
docx/xlsx/pptx, csv/json/yaml, zip…; only types macOS itself declares, so no `mkv` or iWork; no web pages or SVG,
and an executable XML or macro-carrying `doc`/`xls`/`ppt`/OpenDocument file is revealed too, though a 644 one opens),
because exFAT and SMB report every bit set (on such a volume any type not listed is revealed rather than opened).
A file with a per-file "Open with" binding or a resource fork is revealed whatever its name (`open` honours it). Anything else executable, including `notes.34` mode 755 with or without
a `#!` line, is revealed: Launch Services types it as a Unix executable and Terminal runs it (R1). No file content is
read. A Finder alias is revealed too. A document is opened by its canonical path, so a link in the path given can't
be repointed between check and open, but `open` resolves that path again by name: whatever is swapped in at it in
that moment is what opens. This is defence in depth, not a sandbox.

Source/history strings are serialized as evidence, not instructions. The existing
provider permission model remains authoritative: context scoping is not a sandbox.

## Automations and compatibility

New automations require an existing workspace. Hollow saves its absolute `cwd`
and a vault-relative `output_glob` of `workspaces/<id>/outputs/*`. The procedure
starts manual and can be scheduled explicitly. Templates now continue the plan
or review attached evidence. Every run rebuilds current workspace context and,
since 2026-09-26, also carries About you and approved Memory recall. Output discovery and filesystem receipts
continue using the existing runner.

Older automations retain their schedules, cwd and prompts; the editor labels
custom/storage-root scope and lets the user deliberately select a workspace.
Nothing enables,
reschedules, or runs existing automations as part of this source change.

Workflow permission stamps (Paperclip PR 5): every automation present at the
first launch of that build was trusted once, as-is. From then on a change to an
automation's execution fields (permission mode, tool lists, working folder,
agent, model, triggers, schedule) or its prompt made outside Hollow's editor —
in Obsidian, the Library editor, by a move or by an agent — needs approval in
Automations → Permission changes. An automation deleted outside the editor loses
its approval, so restoring it (or an agent recreating it) needs one approval too.
Automated edits to `me/` (including a symlink swapped in), root or workspace
agent instructions (`CLAUDE.md`, `AGENTS.md`, `CLAUDE.local.md`,
`AGENTS.override.md`, `.claude/CLAUDE.md`) and agent config
(`.claude/settings*.json`, `.codex/config.toml`) are flagged after the run, not
blocked; revert them in History & recovery. Edits made through the Library
editor or Set up folders during a run are Hollow's own and are not flagged.

Quiet and inactive skips (Paperclip PR 9b): a workspace automation can skip a
scheduled run when nothing in its workspace changed since its last successful
run started (Skip when nothing changed; the default watch is the workspace
folder). Only the previous run's own writes that a completed write tool event
confirmed are ignored, so sources another app or a shell command wrote still
count. While a workspace's status is not current, its scheduled automations
skip; Run now and triggers still run.

## Whole-app surface disposition

| Former surface / entry | Current behavior |
| --- | --- |
| Today dashboard / daily rituals | Work; ritual palette actions removed |
| Projects portfolio | Workspaces; old project route redirects to Work; selective migration retained |
| Graph / Graph3D | No active shell loader, palette entry or shortcut; files remain in Library |
| Calendar / People | Library Meetings / People; retained source records instead of parallel apps |
| Ingest | Library → Import a source, the Add sheet set to a Library source (S5; ⌘⌥⇧I opens it too); no separate page loader or window |
| Standalone Chat | Workspace conversation; explicit historical conversation browser retained for recovery and artifact resume |
| Workflows + Skills | Workspace-scoped Automations; Skills in Settings |
| Global profile / learning | Settings → Context → About you and Memory (2026-09-26): both reach every chat and automation, workspace or not |
| Inbox / Approvals / messaging | No built-in messaging (the email/Slack/Teams modules were deleted 2026-09-26); a user-shared connector in Settings → Connectors is the only path |
| Setup | One Hollow folder (`Projects/` + `Vault/`); connectors added later in Settings → Connectors |

The top-bar conversation control and Cmd+Shift+C return to the current workspace
(or start workspace creation). The old global side panel is no longer mounted.
Choosing a skill returns to the workspace and preserves its existing draft.
Background imports and sync have a top-bar activity disclosure; errors and
completion remain inspectable after the initiating dialog closes. The old
page-arrival and pointer-light effects are removed from the shell.

Only Cmd+1 Work, Cmd+4 Tasks, Cmd+6 Automations, Cmd+8 Library and Cmd+,
Settings remain page shortcuts. Retired digit bindings are unbound. Compatibility
IDs are normalized in `setActivePage` and history replay; historical source modules
and low-level adapters may remain in the repository without being product routes.
Older design programmes and their ledgers are historical references, not a second
active product specification.

## Verification and boundaries

Rust temporary-filesystem tests cover portable provenance, unchanged originals,
read-only-copy convention, duplicate refusal, symlink/path boundaries, UTF-8
clipping, binary omission, missing/changed origins, and unrelated-context exclusion.
Browser checks use fixture IPC. They establish layout and interaction behavior,
not real account freshness, live provider file access, or installation acceptance.
The September 24 installation includes the merged Projects + Vault feature.
Stable signatures, source provenance, staged/installed embedding self-tests and
native Settings/folder-picker inspection passed. Config remained byte-identical;
all 772 task/workspace files remain present, with 770 byte-identical and two
Notion-managed files updated during enabled background sync. Native verification
did not select a Projects root or send a live provider request.
See [Build & ship](reference/build-and-ship.md#build--ship-how-a-change-reaches-the-installed-app)
and the newest [build log](BUILD_LOG.md) for installation and rollback evidence.
