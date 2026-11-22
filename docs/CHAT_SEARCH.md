# Chat search

Search chats (the top bar's icon or ⌘⇧K, shown in the sidebar), the command palette, `sessions.search_sessions` and the
phone's `GET /v1/search` share the derived `chat_search_fts` index in Hollow's
session database. It holds one document per chat title and one per turn: the
person's own message, the assistant reply, errors and short tool result
summaries. Tool inputs are excluded. Secrets are redacted before a document
reaches the index. Redaction placeholders are not searchable words, and a
summary cut inside a placeholder drops the placeholder's start, so no fragment
such as `[redac` is left to match.

Layout v4 (2026-11-08). `chat_search_docs` keeps each document's scrubbed
display text, keyed by chat and ordinal; its `id` is the document's FTS rowid,
so replacing a document never scans the FTS table. Titles use ordinal -1;
imported turn zero remains a message. Snippets and neighbours read
`chat_search_docs`. A store with another layout version drops its documents at
open and marks every chat for the backfill. Replay integrity checks open only
the private snapshot write-capable for FTS5 validation; the source connection
remains read-only.

`session_store.rs` owns the source connection and `session_search.rs` is its
full-text child module. SQLite triggers mark what changed in the same
transaction as the change. A title change marks the title document; a turn's
request, stored transcript or trace events mark that turn only
(`chat_search_dirty_turns`); a deleted or re-keyed chat and the backfill mark
the whole chat (`chat_search_dirty`). Trigger inserts test whether the mark
exists instead of relying on OR IGNORE, because an outer upsert can override a
trigger's conflict policy. The index can be rebuilt; transcripts and the
usage ledger remain authoritative. No provider's private logs or retired
message stores are read.

Each search first reindexes at most 128 marked documents (expanding a whole
chat counts as one), changed turns first, then whole chats by newest
activity. It asks its own connection whether anything is marked first, and
skips the refresh when nothing is. A write transaction covers at most 32 documents of one chat, so the
store's writer is released between them. Each mark is read once per search,
so a streaming chat costs its live turn, not the whole chat. When marks
remain, the desktop and phone search routes finish them on one background
thread per vault, right after answering. Until a document is reindexed, its
chat matches on the text it had at its last indexing, or does not match yet if
it was never indexed (the first searches after the v4 backfill starts). A
reindexed document's old text is deleted in the same transaction that writes
the new text, so a result never shows stale text for a document once it is
reindexed.

Every query keeps the first 32 distinct words, matched literally. The first
eight words of three characters or more are FTS5 phrases in SQL; candidate
chats stream newest first from `idx_sessions_recent`, and Rust checks every
word again, in the title or one indexed turn. A query whose words are all
shorter than three characters reads titles only (the trigram index has no
entries for them); in a longer query, a short word must be in the title or a
turn of a chat the longer words found. Archived conversations stay excluded.
Only conversations whose folders map to a workspace have an open route.

Search accepts an optional `workspaceId`. Sidebar and palette pass the current
workspace; agent search defaults to its caller's workspace. Results prefer
that workspace, then work-classified workspaces, then other chats, with recency
and session key breaking ties. Work classification reuses the companion's
existing local policy. Opaque cursors carry priority, date and key, and remain
independent of page size; a cursor with no priority (the phone's, and the
format before ranking) reads as priority 0.

A page is bounded by its size, whatever the query matches. The search lists
the working folders of unarchived chats once, stepping through
`idx_sessions_cwd` one folder at a time, and asks each folder's priority
once (a folder with no workspace, or one the caller hides, has none). Each
priority is then its own query, from the cursor's priority on: its folders,
the cursor's keyset (in the cursor's own priority only) and `LIMIT limit + 1`
in SQL, still streamed from `idx_sessions_recent` (the folder test is written
`+s.cwd` so it never takes the index). The search stops as soon as a page
plus one hit is held, so a page reads about one page of rows per priority it
reaches; a row Rust rejects costs one more row. Until the second 2026-11-08
review one newest-first stream held every priority, and it could stop only
once the cursor's priority filled the page: with no preferred workspace
(priorities 1 and 2 only) a broad query read every matching chat on every
page.

Each chat result has a redacted excerpt, up to one nearby turn on either side,
its opening request, latest outcome and handover successor keys. Session
metadata supplies dates. The request and outcome show their first 2,000
characters, and only a head of each is redacted
(`claude_runner::redact_prefix`): it runs to a whitespace character a quarter
past the cut, and redacting it alone gives what redacting the whole text gives
up to there, since no redaction crosses whitespace except a PEM block (a head
holding `-----BEGIN` redacts the whole text) and the key-value ones, which
replace only the value after the whitespace. Before 2026-11-08 each hit
redacted its whole request and outcome, up to 80 KB each in the timing
fixture, and a first page took about 20 times longer. The desktop's order
reads each workspace's work flag from a cache kept while its brief's stamp
holds (`companion_docs::work_ids`, at most 60 s), not from every brief on
each search. Matching saved memories appear before raw chats on a
first page only (not after a cursor) and take precedence over raw
conversation claims. Reading search results never accepts, promotes or
restores memory.

The phone's search lists chats newest first, with no memories, and fails
closed: a chat is listed only when its workspace is on the allow-list
`companion_docs::shown_on_devices` builds, the companion mirror's work-data
rule. With the companion's "include work data" setting
(`<support>/Companion/scope.json`) off, that is a workspace whose brief was
read, carries no `merged_into` and is not work (`scope: work`, a `notion-*`
id or a work marker in its id or title). A merged workspace (a merge marks its
source before it moves the chats, so an interrupted merge leaves them there),
a missing or unreadable brief and front matter that never closes all count as
work. The brief's front matter is read from its first 48 KB, as the workspace
list reads it, so a long brief keeps its `scope: work`. With the setting on,
every workspace but `linear-*` ones. A `linear-*` workspace's chats never
reach the phone: `companion_docs::never_on_devices` is checked on each chat's
own workspace as well. An absent or unreadable scope file counts as off.
Chats in no workspace (started in the vault root) have no open route and are
searched nowhere, on the Mac or the phone; Chats (`quick-chats`) and the Main
agent are workspaces and follow the rule. The phone's chat list and
workspace routes apply no work-data rule, so its search never shows a chat
they don't. Before the second 2026-11-08 review the rule was a deny list
built from the companion's workspace list, which skips merged folders, so an
interrupted merge showed a work or `linear-*` workspace's chats and
snippets, and a brief over 256 KB lost its `scope: work`.
Contract: [phone-api/search.md](phone-api/search.md).

Tests use temporary vaults and a SQLite snapshot of a fixture store. Nothing in
this feature requires writing the live vault or changing provider settings.
The timing fixture drains the index before it times a search.
