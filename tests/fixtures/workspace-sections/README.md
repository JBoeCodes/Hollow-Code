# Workspace section fixtures (Canvas C5a)

Real workspace files for the section-model tests (`src/briefModel.test.ts`,
`src/planModel.test.ts`, `src/overviewModel.test.ts`, `src/sourcesModel.test.ts`; see
[CANVAS_C5.md](../../../docs/history/CANVAS_C5.md)).

| File | Source | Copied |
| --- | --- | --- |
| `WORKSPACE.md` | the Hollow development workspace's brief (`workspaces/local-f9ebfd0d1c780db6a000fcf2/WORKSPACE.md` in the owner's vault), 295 lines, last modified 2026-10-05 09:19 EDT, SHA-256 `5ee21b35…aeb7a5` before sanitizing | 2026-10-05 |
| `PLAN.md` | the same workspace's `PLAN.md`, 88 lines, last modified 2026-10-05 09:16 EDT, SHA-256 `039d6711…ad8c0c` before sanitizing | 2026-10-05 |
| `SOURCES.md` | the same workspace's `SOURCES.md` (C5b), 10 lines, last modified 2026-10-02 08:52 EDT, SHA-256 `49b5d19c…774d27`; it had nothing to sanitize, so it is byte for byte the original. Its one entry names `sources/notion-hollow-next-steps-2026-10-02.md` (60,229 bytes, no `.provenance` sidecar), whose SHA-256 matches the entry's | 2026-10-05 |

**Sanitized, nothing else changed.** Email addresses and account names are
replaced by placeholders, line for line, so every line number, heading,
checkbox and date is the original's:

| Original | Placeholder |
| --- | --- |
| any email address | `<email>` |
| the GitHub account in PR links | `<github-account>` |
| the organisation account a connector signs in as | `<org-account>` |
| the macOS account name (in paths and the candidate bundle id) | `<user>` |
| the account holder's first name (in a stale checkout path) | `<name>` |

They are snapshots: the live files keep changing, and the tests pin counts
taken from these copies. Replace them together, re-sanitize, and update the
counts in the tests and the table above.
