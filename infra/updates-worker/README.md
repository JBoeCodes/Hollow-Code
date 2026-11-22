# Hollow updates Worker

A Cloudflare Worker (modules syntax, plain ESM with JSDoc types, no npm
dependencies) that serves Hollow's update channels, builds and teammate install
pages out of the R2 bucket `hollow-updates`. Design: updates design sections 4
and 11; this is row U3. Deployment is row U11: nothing here is deployed from
the repository, and no agent runs `wrangler`.

| File | What it does |
|---|---|
| `src/worker.js` | routes, the token check, responses (`export default { fetch }`) |
| `src/auth.js` | token shape, SHA-256 hashing, constant-time compare, `people.json` parsing and its cache |
| `src/manifest.js` | channel manifest checks, "complete in the bucket" check, URL rewrite |
| `src/install-page.js` | the install page HTML (everything escaped) |
| `src/layout.js` | the R2 key layout and allowlists (channels, version pattern, build file names) |
| `src/publish-order.js` | the upload order contract for the release builder (U2) and promotion (U8) |
| `wrangler.toml` | placeholder config: bucket `hollow-updates`, route `updates.example.com/*` |

Tests: `node --test scripts/updates-worker.test.mjs` (part of
`npm run test:scripts`, so the gate runs them). They import these modules
directly and use an in-memory stand-in for the R2 binding and a fake clock.

## Routes

Only `GET` and `HEAD` (anything else is 405). Every response carries
`X-Content-Type-Options: nosniff` and `Referrer-Policy: no-referrer`.

| Route | Auth | Answer |
|---|---|---|
| `/channels/<main\|nightly>.json` | bearer token, channel allowed | the manifest, `application/json`, `Cache-Control: no-store`; 403 when the person may not use that channel; 404 when the channel is empty **or incomplete** (below) |
| `/builds/<0.6.n>/<file>` | bearer token (DMG only: or `?token=`) | the file, `Cache-Control: private, max-age=3600` (`no-store` when the token came from `?token=`), `Accept-Ranges: bytes`, a single `Range` gets 206 (or 416); 403 for a person with no channels |
| `/install/<token>` | the token in the path | the install page (`text/html`, `no-store`, strict CSP, `X-Frame-Options: DENY`); 403 for a person with no channels |
| `/me` | bearer token | `{"name": "...", "channels": [...]}`, `application/json`, `Cache-Control: no-store` (row U9) |

`<file>` is one of `Hollow.app.tar.gz`, `Hollow.app.tar.gz.sig`, `Hollow.dmg`,
`notes.json`. The build's stored manifest, `builds/<v>/manifest.json` (row
U8), is not on that list, so it is a 404 like any other name. `<0.6.n>` matches `^0\.6\.(0|[1-9][0-9]{0,8})$`. A path with a
percent escape, a backslash, an empty or dot segment, or any part outside its
allowlist is 404 before the token is looked at. R2 keys are rebuilt from the
matched parts (`layout.js`), never taken from the request, so `people.json`
and anything else in the bucket can't be reached.

**Failures don't say why.** A missing header, a malformed one (anything but
exactly `Bearer <43 base64url characters>`), an unknown token, a revoked
person and an invalid `people.json` all get the same 401 (`Unauthorized`,
`WWW-Authenticate: Bearer realm="hollow-updates"`). Bodies are fixed strings;
nothing echoes the token. A bucket error is 503 and logs one fixed line (no
token, hash, URL or R2 message).

**The `?token=` exception.** The install page's DMG link has to work from a
browser, which can't send a header, so `/builds/<v>/Hollow.dmg` alone also
accepts exactly one `?token=` (and refuses a request with both a query token
and a header). A query token anywhere else is a 401, even beside a valid
header, so clients never get into the habit. The cost: that URL can land in
browser history and in request logs. Mitigations: `Referrer-Policy:
no-referrer` everywhere, `Cache-Control: no-store` on every response served
under `?token=` (and on the install page), Workers observability off in
`wrangler.toml`, and the link only reaches the DMG, which is the same
notarized app anyone with the link could install. Revoking the person ends it.

**`/me` (row U9).** The app asks it after connecting and on every check to
learn the person's name and allowed channels, so its channel picker offers
only those. The body is built from two fields only: `name` and `channels`
(in `CHANNELS` order, Main first; `[]` means none, still a 200, and every
other route then answers 403). Never the id, the hash, `revoked`, `createdAt`
or any other person. It authenticates exactly like the manifest routes: the
same identical 401 for a missing, malformed, unknown or revoked token, and a
`?token=` is a 401 here too. So a revoked token gets that 401 at its next
check, within the 60 s cache; the app words it as "did not accept this token
(it may have been revoked)", since the Worker never says which failure it was.
A path near it (`/me/`, `/me.json`, `/ME`) is 404 before the token is looked at.

**Install page.** `/install/<token>` checks the token first (bad or revoked:
the plain 401 above, never the token back). Then it shows the person's name,
a download link `/builds/<v>/Hollow.dmg?token=<token>` for the newest complete
build on the first channel they may use (Main before Nightly) when that build
has a DMG, and a **Connect Hollow** button linking
`hollow://updates/connect?token=<token>`. Every value is HTML-escaped and the
token URL-encoded. A request to it with a header or a `?token=` is a 401.

**Builds and channels.** Channel limits decide which manifest a person is
offered, not which build they may fetch: **any build in the bucket is readable
by any active person** (with at least one channel) who knows its version.
Decided on 2026-11-15 not to scope builds to channels for now; it is a known
limit (see Deploying).

**Ranged reads.** The span is worked out from a `head` and the bytes read with
`onlyIf: { etagMatches }` on that etag, and every header (size,
`Content-Range`, `ETag`) comes from the object actually read. A file
overwritten in between is retried once; a second mismatch is a 503 with
`Retry-After: 1`, never bytes of one upload under another's headers.

**Not done here:** conditional requests (`If-None-Match`), and `If-Range` (a
ranged request with `If-Range` gets the whole file, which RFC 9110 allows).
No rate limiting.

## Tokens and `people.json`

A token is 32 random bytes, shown once as **base64url without padding (43
characters)**. The bucket stores only `tokenHash`: the lowercase hex SHA-256
of the token's UTF-8 text (`hashToken` in `src/auth.js`). The Worker hashes
the presented token and compares it with every entry in constant time (no
early exit, revoked entries checked after the loop).

```json
{
  "people": [
    {
      "id": "p-sam",
      "name": "Sam",
      "tokenHash": "<64 lowercase hex>",
      "channels": ["main", "nightly"],
      "revoked": false,
      "createdAt": "2026-11-15T12:00:00Z"
    }
  ]
}
```

- `channels` absent means both; `[]` means none: 403 on every route, the
  install page included.
- `revoked` must be the boolean `false` for the person to be let in.
- The file is all or nothing: one malformed entry, a duplicate `id` or
  `tokenHash`, an unknown channel, or a file that isn't this shape lets nobody
  in (fail closed). A missing file is an empty list.
- **Revocation delay:** the Worker keeps `people.json` in memory for **60
  seconds** (`DEFAULT_PEOPLE_TTL_MS`), per isolate. A revoke (set `revoked`
  or remove the entry) is effective for every request that starts 60 s after
  the write. A read error is not cached. This holds for a revoke made with
  `scripts/release/people.mjs` (Hollow's People) too: it is effective within
  60 s, not at once.
- **Editing it:** `scripts/release/people.mjs` (`docs/UPDATES.md`, People)
  writes with a conditional PUT (`If-Match` on the ETag it read), so two
  writers can't silently lose a change, and creates a missing file only when
  told to (`init`).

## Channel manifest format

Tauri's static updater JSON plus Hollow's fields, at
`channels/<channel>.json`:

```json
{
  "version": "0.6.1234",
  "notes": "What's new, as plain text",
  "pub_date": "2026-11-15T12:00:00Z",
  "platforms": {
    "darwin-aarch64": {
      "signature": "<contents of Hollow.app.tar.gz.sig>",
      "url": "https://updates.example.com/builds/0.6.1234/Hollow.app.tar.gz"
    }
  },
  "channel": "nightly",
  "commit": "<40 hex, the built commit>",
  "buildId": "<release builder's id for the run>",
  "schema": { "sessions": 42, "tasks": 7 }
}
```

`schema` maps each store to its `PRAGMA user_version` (U6 reads it for Go
back). Required: `version`, `channel` (equal to the file's channel),
`platforms["darwin-aarch64"]` with a non-empty `signature` and a `url` whose
path is exactly `/builds/<version>/Hollow.app.tar.gz` (https, no query). Checked
when present: `notes` string, `pub_date` a date, `commit` 40 lowercase hex,
`buildId` non-empty, `schema` values non-negative integers.

The Worker serves a manifest only when it is **complete in the bucket**:
`builds/<v>/Hollow.app.tar.gz` and `notes.json` exist and are not empty, and
`builds/<v>/Hollow.app.tar.gz.sig` exists and equals the manifest's
`signature` (whitespace trimmed). Otherwise the channel answers the same 404
as an empty channel. The served copy keeps only `darwin-aarch64` and rewrites
its `url` to this Worker's own origin.

## Publish order

`publishOrder({ version, channels })` in `src/publish-order.js`:

1. `builds/<v>/Hollow.app.tar.gz`, `Hollow.app.tar.gz.sig`, `Hollow.dmg`
2. `builds/<v>/manifest.json` (kind `record`): the exact channel manifest the
   build was first published with. Stored, never served (`BUILD_RECORD`,
   `buildRecordKey` in `layout.js`); promotion copies it, so any kept Nightly
   can become Main, not only the newest. Builds published before this record
   existed can't be promoted (none had been published when it was added).
3. `builds/<v>/notes.json`
4. `channels/<channel>.json` for each channel, **last**

One write at a time, stop at the first failure: the channel stays on its
previous build. Promotion to Main (U8, `scripts/release/promote.mjs`) uploads
no build files: it checks `manifestPrerequisites(version)` exist and that the
stored `.sig` is the record's signature, then writes `channels/main.json`
(the record with `channel: "main"`).

## Deploying (U11)

- R2 bucket `hollow-updates`, bound as `UPDATES`.
- Route `updates.example.com/*` on the `example.com` zone; `workers_dev`
  stays off and so do preview URLs (`preview_urls = false`).
- No Worker secrets. The account id comes from the deploying shell, not this
  repo. The R2 write credential lives only with the release builder (design
  section 4), never in the Worker.
- **URLs carry tokens** (`/install/<token>`, the DMG's `?token=`). Before
  deploying, check that nothing records or caches them: Workers observability
  stays off unless URLs are redacted; no Logpush job for this zone or Worker
  includes the request URL (or it is redacted); `wrangler tail` sessions show
  URLs, so treat their output as secret; and no Cache Rule or Page Rule
  ("Cache Everything") covers `updates.example.com`, since the Worker's
  `Cache-Control` is what keeps these responses out of shared caches.
- **Known limit:** builds are not scoped to channels: any build in the bucket
  is readable by any active person who knows its version (decided
  2026-11-15, no scoping now).
