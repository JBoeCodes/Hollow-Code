# Tracker sync and the phone

Part of the agent manual; [AGENTS.md](../../AGENTS.md) is the entry point. Full references: [Tracker sync](../TRACKER_SYNC.md), [phone tracker API](../phone-api/tracker.md).

## Engine

- The tracker engine uses a durable outbox and create journal, one patching mirror renderer and scoped History leases. Known mirror syncs admit scoped chats; each mirror commit uses atomic CAS/create with History host attribution. Root writers and unknown sync owners use the full-vault exclusion. See [the engine](../TRACKER_SYNC.md#the-engine-a5).
- Both sync engines use the immutable running config; saved mode changes apply only after restart. Setup registers the running writer policy before the broker and phone inbox start; broker edits refuse an unregistered policy or a ledger with a different provider/team.
- Task and phone edits enter `tracker/seams.rs` before legacy parsing. The editor shares the outbox, takes sync exclusion before History and keeps owned comment blocks. The ledger caches the adapter's writable fields and the editor refuses others; an explicitly empty field list stays read-only.
- Agent creates use External actions with stable keys; archive is `ALWAYS_ASK`. Project creates and archives journal the attempt before any provider call and never resend an unconfirmed attempt. Create scope errors keep the attempted intent and never resend.
- Staged local metadata carries before/after values and refuses an intervening edit. Before accepting another editor request, replay any earlier staged write under the same lease; persist conflict acknowledgements with that journal. A complete offline revert keeps its staged journal until the mirror write lands, so crash recovery never resends a reverted edit.
- Successive offline edits keep every pending field and unrelated conflict notes. Failed items keep retry snapshots so unrelated cursors advance. Full reconciliation catches up after 3 AM. Local project status edits are kept as conflict notes until provider status ids are available.
- Tracker claims are local ownership on active issues and never change provider status or body. Recurring completions queue the next due date and an unstarted state together.

## Workspace identity and mirror paths

- Tracker workspaces use `tracker_project` identity, surviving folder renames. New projects get provider/slug/id8 folders and a generated `TRACKER.md`; stored mirror paths stay unchanged. Only marked generated context can be rewritten.
- Workspace creation journals an initializing record before writing files, then marks it active after publication; only that phase may finish a missing initial mirror, and its recovery rechecks any file that appeared since the ledger read before a provider patch or replacement.
- Mirror paths bind provider and issue/project identity before any read or write. Never use a `PROJECT.md` filename match as a status-move fallback. Allocate all new project paths before the History lease (short-id collisions add the full id, then a numeric suffix), reserve them case-insensitively, and pass that exact allocation into reconciliation.
- Create receipts expose only an active recorded mirror path, never a predicted folder. Unavailable tasks are never recreated from an empty template; only owned state, pending and conflict fields change on an unavailable file.
- Generated workspace `AGENTS.md` writes are registered as host writes; seeding a dated build log never replaces an existing log or authored instructions.
- The gardener leaves tracker-linked test tasks (`Task.tracker`) for manual review.
- Tracker guidance in prompts stays within the existing context budgets; read-only environment text never advertises create tools.

## Phone

- The phone's Send settings use [chat controls](../phone-api/chat-controls.md). Agent and model carry forward; permission follows the chat's recorded mode unless the Mac has a saved choice; only effort is one-send and rides with the queued payload. No phone choice changes app-wide settings, and the send and native-resumption guards stay in force.
- Send settings compose with attachment sends; both are validated before queueing. Canvas answers use the same send path with inherited settings.
