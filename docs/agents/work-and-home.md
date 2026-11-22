# Work, Home and project notes

Part of the agent manual; [AGENTS.md](../../AGENTS.md) is the entry point. Full references: [Workspaces](../WORKSPACES.md), [product](../reference/product.md).

## Welcome and imports

- Welcome keeps optional panels mounted after their first open so collapsing never discards drafts or imports. Unknown sign-in is never Ready. See [welcome](../WELCOME.md).
- Pi imports follow the latest session branch and leave source files unchanged; reasoning or extension state is never imported as a person's message.

## Work and Home

- Work has Home, Today and Running tabs; Map is a Home action and keeps its route id. Keep `today`, `your-work`, `go-work`, route ids and saved keys compatible. See [Home](../WORKSPACES.md#chat-first-home).
- Work stays mounted, hidden, behind other pages: gate its portals, focus, section arrivals and minute refresh by visibility. The eager Work host loads `YourWork.tsx` only when its overview shows, outside Suspense.
- Workspace result links apply their saved selection inside the navigation guard; a refusal leaves the prior view unchanged.
- Home is Ask plus recent projects; Every workspace folds until Show all or Find. Digest and session changes share a two-second trailing refresh; first load, Retry and the minute clock stay immediate. See [Home overview](../reference/product.md#home-overview).
- Day ring: [Day ring](../DAY_RING.md). Meeting prep: [docs/PREP_PACKS.md](../PREP_PACKS.md); unknown attendees stop a new paid pack, existing packs stay readable.

## Project notes, brief, plan and sources

- The workspace panel and its sections are gone: a project's notes open as files in the Preview pane (the top bar's Project files), and the Preview pane opens only on a click. Earlier versions of a file are in History & recovery.
- `briefModel` stays lossless; presentation never changes the parser or agent context. Plan rows keep their original identities.
- Sources rows keep provenance, checksum and refresh actions inside each row, and read errors in full. Open rows survive refresh; state resets only on a workspace identity change. See [source contract](../SOURCE_CONTEXT.md).
- Results previews and the 24-read allowance: [Canvas sections](../CANVAS.md#workspace-sections-and-the-curated-layer). Canvas sketches never mount agent HTML.

## New workspace

The form (`NewWorkspaceForm.tsx`) keeps a created workspace for open retries and failed submissions across remounts until success or Cancel. Create and start snapshots the goal and queues it only after an owned fresh conversation opens; navigating away never consumes another draft. Repository discovery rejects missing or unreadable roots. See [New workspace form](../WORKSPACES.md#new-workspace-form).
