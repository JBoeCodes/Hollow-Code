# Phone API v1.2: search

2026-11-07 (overnight lane I3; chats from the Mac's full-text index since 2026-11-08). One read-only route for the
iPhone app's Search tab: chats (a word in the title or in one of the chat's turns: the person's message, the reply, an
error or a short tool summary, secrets redacted) and workspaces (a word in the title, status, Outcome or Next step). The
same file lives in both repositories (`docs/phone-api/search.md`), with the same fixtures in
`docs/phone-api-fixtures/search/`. Change it only in both at once. v1 and v1.1 are unchanged.

The route needs the bearer token and passes the same Host and Tailscale checks as v1 (Who may call, and Deviations 2
in [../PHONE_CHAT_API.md](../PHONE_CHAT_API.md)). **Nothing here writes a file or a chat**; the Mac brings its derived
search index up to date before it reads ([../CHAT_SEARCH.md](../CHAT_SEARCH.md)). A Hollow without this route answers
404 `not_found`; the app then shows "Update Hollow on your Mac to use this".

## `GET /v1/search?q=<words>[&limit=<n>][&cursor=<cursor>]`

- `q` (required): the words to find, separated by whitespace, percent-encoded (a `+` is a space here, so send a
  literal plus as `%2B`). Whitespace is folded to single spaces; at most 200 characters after that, and only the first
  32 distinct words (compared after lowercasing) are read. **Every word must match**, in any order, as literal text.
  Case is ignored in titles, turns and workspace text ("москва" finds "Москва", "grösse" finds "GRÖSSE"). 400
  `bad_request` when `q` is missing, blank or too long.
- `limit`: chats per page, 1 to 50, default 20. 400 `bad_request` otherwise.
- `cursor`: the `nextCursor` of the previous response, opaque. 400 `bad_request` when it is not one Hollow gave.

200 (fixtures `search/results.json`, `search/results-more.json`):

```json
{
  "query": "release notes",
  "workspaces": [Card],
  "workspacesTotal": 1,
  "chats": [{"session": Session, "workspaceId": "hollow-companion-app-cdc399a4852fa14c", "snippet": null}],
  "nextCursor": "WyIyMDI2LTExLTAzVDE0OjIwOjQxKzAwOjAwIiwicy0yMDI2MTEwMy05MWFiMDdkZSJd"
}
```

- `query`: the words as Hollow read them (whitespace folded).
- `workspaces`: each a Card as in [workspaces.md](workspaces.md): every workspace that is not in the trash, archived
  ones included (their `status` says so), the chat folders and the Main agent left out, whose title, status, brief
  Outcome and Next step together hold every word (a word may sit in a different field than another). The ones whose
  title holds every word come first; each group is in the overview's order (pinned by rank, then the newest
  conversation, then title). At most 10, and **only on the first page** (a request with a `cursor` gets `[]`).
- `workspacesTotal`: how many workspaces matched, which can be more than the 10 sent; 0 on a page with a `cursor`.
- `chats`: the Mac's chat search (`sessions` in every workspace, newest activity first): a chat matches when every
  word is in its title or in one of its turns: the person's own message, the assistant's reply, an error or a short
  tool result summary (tool inputs never). Secrets are redacted before a turn is indexed, and a redaction placeholder
  holds no word. When every word of `q` is shorter than three characters, only titles are searched; a short word next
  to a longer one is also found in the turns of the chats the longer words find. Archived chats and chats in no
  workspace (started in the vault root) are not searched. **Work chats stay on the Mac:** a chat is listed only when
  its workspace is classified as personal, unless "include work data" is on in the companion settings on the Mac,
  and a chat in a `linear-*` workspace never is. A workspace counts as work when its brief says `scope: work` (read
  from the brief's start, however long it is), it is a `notion-*` workspace or its id or title has a work marker;
  and also while it is merged into another (an interrupted merge can leave its chats behind) or its brief is
  missing or can't be read. `session` is the v1 Session (`GET /v1/sessions`), `workspaceId` the workspace the chat opens in
  (null only for a chat in no workspace, which the search leaves out today), `snippet` the part of the earliest turn
  that holds the first word the title lacks, at most 240 characters, else null. Snippets are redacted text: a pasted
  secret shows as a placeholder such as `[api-key]`. A turn written in the last moment can be missing from the
  results or match on its previous text until the Mac reindexes it, which it does on each search and in the
  background right after.
- `nextCursor`: pass back as `cursor` for the next page of chats; null on the last page. The pages keep going from
  the last chat of the previous page by its activity time and key, so a chat that gets a new message meanwhile can
  move past a page boundary: a client keeps what it has and adds new rows by `session.key`.

## Notes for the client

- Chats are newest first, not ranked by relevance (the Mac's palette ranks its own workspace and work chats first;
  the phone does not).
- Search as the person types after a short pause (the app waits 300 ms and drops an answer that arrives after a newer
  question); an empty or blank query is not sent.
- A tap on a chat opens it in the Chats tab by `session.key`; a tap on a workspace opens its detail
  (`GET /v1/workspaces/{id}`).
- Every response carries `Cache-Control: no-store`, as in v1. Nothing here is a stream.
