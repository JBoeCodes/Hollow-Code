# Sidebar interactions

## Moving conversations

Local quick, organized and nested conversation rows drag to the temporary
Pinned, Active and Settled destinations. Each destination names the exact
visible menu command it will invoke. Drop captures the conversation key and
uses the source row's current enabled menu entry, preserving organization
revision checks, activity checks, fresh-chat handling, focus and Undo.

A pin protects a conversation from settlement. Drop into Active unpins it;
then Settled can settle it. Work in progress also prevents settlement.
No combined unpin-and-settle operation is introduced. Existing project-pin
ordering and conversation-to-project drops retain their own handlers.

`railChatDrag.ts` owns the source and native ghost. Its cleanup only clears
its own drag. `RailDropZones.tsx` loads lazily; `railDrag.ts` chooses the
existing command. `railListMotion.ts` measures keyed rows across shelves,
animating reorder and entry fade with the shared panel duration. Zero,
reduced motion and navigation suppression cancel owned animations. There
are no cloned exit rows or duplicate focus targets.

`railProjectChildren.ts` preserves the full descendant graph while row
visibility follows organization. A moved parent does not hide its active
descendants; orphaned subagent roots remain reachable after refresh.
Workspace rollups retain their existing full-session inputs.

## Details and row attachments

Local quick, organized, nested and loaded project-lead rows show the same
metadata on hover and keyboard focus. Details in the row menu pins the card
open, with a visible Close details button and focus return. The card shows
stored title, status, turn count, activity time, folder and available
organization/parent facts. It does not generate a transcript summary.
Menus, modals and inline editors keep Escape ownership. Sidebar collapse,
row disposal and drag start dismiss the card.

`railRowDetails.ts` owns window-only hover/focus state; the portalled
`RailRowDetails.tsx` card loads lazily. `railRowFiles.ts` captures the exact
row key, conversation version, page and DataTransfer files synchronously,
before lazy imports. `railFileDrop.ts` waits for that exact destination's
composer, including history hydration, and checks identity before each file.
Files use existing composer limits and never auto-send. A rejected file does
not prevent later valid files in the same drop from attaching.

Remote rows and projects without a loaded lead consume and explicitly refuse
file drops. They never fall through to the open local conversation. Handlers
prevent default but allow propagation so the window clears its drag veil.
Project pin and conversation-drop routing retain their existing handlers.

## Verification

Unit coverage exercises destination guards, source ownership, policy changes,
motion cancellation and descendant reachability. The deferred browser spec
is `tests/smoke/rail-drag-motion.spec.ts`; run it in the final shared browser
suite, never as a per-PR run. Details and attachment coverage lives in
`tests/smoke/rail-details-files.spec.ts`; its unit tests cover navigation races,
history hydration, missing leads, remote refusal and capture before imports.
