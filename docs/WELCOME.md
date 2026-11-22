# Welcome

`FirstRunSheet.tsx` is one welcome page in `firstRunMount.tsx`'s lazy root.
It replaces H6's sequential steps while retaining its config `setup` schema.
Existing installs without a setup block stay complete. Close leaves setup open.
Start without an agent completes only open fields, marking optional fields
skipped and Start done; saved progress survives and a failed write can retry.
Create workspace and Open Chats finish only after their destination accepts.
Workspace creation keeps its created id for retry after a failed open.

The page gets agent membership and capability flags from `agentDescriptors`.
CLI and account probes enrich those entries; they never introduce agents.
Ready requires both availability and verified sign-in. Unknown sign-in stays
Details with its reason. Details lists supported capabilities and Accounts.
Check again refreshes the probes. Add more agents opens the official
[ACP registry](https://agentclientprotocol.com/registry) on click. Registry
discovery does not install binaries or claim unsupported adapters are available.

Import conversations and context, Connect your context, and Where your work
lives are native disclosures. Their panels mount on first open and stay mounted
on collapse, preserving drafts, selections and in-flight imports. They retain
their own scan consent, review, save and tracker controls. Start retains the
optional workspace name, import summary and Open Chats.

Settings About has Open welcome page; Sources' existing Run setup again remains.
The Settings registry makes About and its welcome control searchable in the
palette. Reopen resets setup progress only, not accounts, imports or settings.

Verification: `src/firstRun.test.ts` covers registry membership, unknown sign-in
and retryable completion. `tests/smoke/first-run-setup.spec.ts` covers the page,
disclosures, imports and reopen; browser execution belongs to the end suite.

## Pi history, scan coverage and phone pairing

Pi folders are discovered at `~/.pi/agent/sessions`. Look stages JSONL
conversations through the existing import walker; commit uses the existing
transaction, hash dedupe and provenance. Pi is recorded as `imported`, with
no native resume identity. Continue in Hollow uses the normal quoted-history
handoff. The source is never edited and tool calls remain historical excerpts.

`session_import/pi.rs` reads a bounded 64 MB snapshot, at most 100,000 tree
entries. Version 1 is linear; versions 2 and 3 follow the last entry's parent
chain. Competing branches stay in the source and their count is visible.
Malformed ancestry or duplicate ids refuse the file. Reasoning, system
messages and extension state are omitted. The common builder retains its
message, tool-excerpt and conversation bounds. Parse failures are counted
in the scan notice; incomplete final lines and shortened history are visible.
The format was checked against Pi's official [session format](https://github.com/earendil-works/pi-mono/blob/main/packages/coding-agent/docs/session-format.md)
and [session manager](https://github.com/earendil-works/pi-mono/blob/main/packages/coding-agent/src/core/session-manager.ts).
The fixture is synthetic; no user's history was read.

Scan-limit, stopped-walk, unreadable-file and skipped-link notices appear
beside usable import results. Limits never look like a complete empty scan.

Set up your iPhone lazily mounts the existing phone section. It retains the
server, tailnet publishing, explicit Pair an iPhone, expiring QR and Cancel
pairing controls. Opening the section only reads status and registers mode
reporting. A refresh timer exists only while its component is mounted.
