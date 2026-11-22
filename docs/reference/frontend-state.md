# Frontend state model

Completed provider-reported reasoning appears in a native Thinking disclosure,
closed by default. `AgentCapabilities.reasoning` gates capture; redacted blocks
and subagent reasoning are omitted. A bounded `hollow:reasoning` panel snapshot
is accepted only for the live run and selected conversation. It updates the
first assistant row of that run. `ConversationTurn.reasoning` restores the same
separate field from durable transcript JSON (older records default to empty).
Reasoning is never added to answer text or conversation context reconstruction.
The disclosure remains reachable inside a completed run's expanded steps.
Transcript host warnings have native details/summary disclosure. Exact repeated
messages group within one user turn, with an occurrence count; different causes
and later turns remain separate. The 120-character summary and scrollable
384px detail keep long errors bounded without cutting the stored text. Normal
notices and assistant prose retain their existing rendering. A standalone
host error envelope also discloses; an answer with an appended error and any
turn carrying tools retain their full rendering. The live footer
shows Working for plus elapsed time, then current activity; queued work says
Queued and does not claim execution time.

Automatic settlement (2026-11-16): `conversationSettlement.ts` owns the menu's
explicit choices and minute checks while the window is open. Policies are off
by default and apply once to the saved activity and revision. The selected
conversation and every non-idle row are excluded; native checks remain the
authority. Later activity or manual organization changes disable the policy.
The saved reason remains in Settled, with Wake and exact-revision Undo.

Conversation organization (2026-11-16): `store/conversationOrganization.ts`
holds keyed rows and the clock; `conversationOrganizationLoad.ts` owns reads
and accepted writes. Unchanged rows retain identity. Menu mutations load through
`conversationOrganizationActions.ts`, compare saved revision and activity, and
refresh workspace rows after acceptance. Undo uses the returned revision and
activity, never overwriting later work. Wake opens the exact durable key.
`RailOrganizedChats.tsx` owns the clock and counted Pinned chats, Snoozed and
Settled folds, plus Ready chats for expired snoozes outside the normal project
lead and Quick chats routes. Current rows and attention beat snooze; project
pins keep their existing home. Settle is the archive action, including explicit
question-card expiry. A failed organization read exposes Retry.

Workspace list refreshes expose `workspaceListLoading` and `workspaceListError`
independently of the selected workspace's banner. Failure preserves the last
successful array and `workspaceListLoaded`; success clears only the list error.
Rail and Work's first-list fallback share this state and the coalesced refresh.

Part of the agent manual. [AGENTS.md](../../AGENTS.md) is the entry point and
links every reference doc; this one holds CLAUDE.md's "Frontend state model",
moved there word for word on 2026-10-13 (only relative links were re-based).
Read it before changing the store, chat state and routing, the Preview pane
and its web mode, page context or the activity dock. Keep it current with the code in
the same change, as AGENTS.md asks.

## Frontend state model

### Workspace draft ownership

Composer details are below; persistence retains its existing ownership rules.

### Composer mentions

The + menu also offers Use live Markdown / Use plain text. Markdown mode lazily
loads `editor/MarkdownEditor.tsx`; notes keep their frontmatter and completion
behavior, while the composer owns Enter, paste, mentions and source selection.
`composerField.ts` supplies the field contract and rich-field dictation registry.
The existing draft store remains the only owner of the Markdown string. Switching
chats resets the editor's undo history; external draft changes preserve focus.
Plain text is the default, with live Markdown an explicit per-view option.

The message box completes `@` project/conversation files and `$` Library skills
at the caret, alongside slash commands. The + menu exposes Mention a file and
Use a skill. Completion replaces only its token; Escape preserves the draft.
File chips preview or remove a Markdown reference in draft text, without copying
the file or granting access. Skills use the catalog's provider-aware copy and
existing read-first prompt. Async reads cancel on trigger or conversation
changes; loading, empty and failure stay distinct.

### Workspace draft persistence

`store/conversationDrafts.ts` stores composer text under
`hollow.conversationDrafts.v1`: an ordered array of `{id,text,legacy?,stashOwner?}` rows.
IDs are `session:<key>` or `new:<workspaceId>`; session drafts follow a chat
when it moves projects. At most 100 drafts and one million text characters
are retained, expiring oldest entries first. A larger current draft stays
whole in memory with an unsaved notice. Oversized text is excluded from the
persisted budget and write so other drafts survive; the overall 100-entry bound
still applies to memory. Attachments share those owners and retention but are
never serialized. A chip still saving is stored too; its save finishes into
the left draft by chip id (`settle`). Each write re-reads storage first and
keeps another writer's keys, overriding only the ids this store changed since
its last good write. Failed writes keep the newest memory
copy; legacy workspace slots are removed only after successful migration.

The composer's + menu stashes up to five drafts per owner as `stash:<uuid>`
rows with `stashOwner` pointing at that owner. Stash and restore each move
both text and in-memory chips in one write. Restore prepends to newer text
and refuses combined attachment/text overflow. First acceptance transfers
New chat's stashes to the assigned session. Stashes share draft retention
and window-only attachment behavior; failed writes keep memory and report it.
Clipboard text of at least 12,000 characters attaches as `pasted-text.txt`
through the existing count, size and safe-cwd gates. FileReader progress is
labelled Reading; after 100 percent it says Saving until the local save ends.
Send in background uses normal send acceptance, then opens a new chat in the
same workspace only while its original view still owns the screen. No new chord.
Typed usage, turn and budget result kinds expose a recovery notice by the box.
The model/effort chip stays capability-driven; no speed capability exists.

Migration selects the currently open conversation in that cwd, otherwise
the workspace's default session (New chat if none), before applying a new
selection. An existing per-chat draft wins over the legacy text.

`chatRuntime` calls the first-send hook only for the local queued turn's
accepted reply or matching start/end event. It transfers the current unsent
composer to the assigned key and clears New chat in the same write; repeated,
foreign, adopted and recovered events cannot transfer it. Reset remembers
the departing owner before clearing signals. Workspace opens suppress draft
writes through reset/restore, then establish the new owner after their stale
request and navigation guards pass. Permission restore is unchanged.
An owned workspace/cwd/session tuple must match before every write; changing
only a key cannot claim another draft. Empty composers clear their slot rather
than consuming retention. Main's /new and /clear use the guarded fresh-workspace
route, restoring its New chat draft after the command itself is consumed.
When the chat on screen turns out to have continued elsewhere (a handover;
`ChatChainBar` hides its composer), `carry` moves its unsent text and chips
to the successor's draft, after any text already there, and clears the
composer.

`store/workspaceDraft.ts` routes Results, Plan, brief/file Ask and workspace
passage Ask through the workspace's conversation before appending. It captures
both workspace ID and cwd, rechecks them after focus, and leaves the draft
untouched on failure or navigation. Identical pending requests coalesce;
`addToDraft` prevents a later duplicate. No send occurs. Library passages keep
the current conversation; pure `workspaceRelative` decides file ownership,
independent of canvas settings. Library passes `askInCurrentConversation` even
for workspace files. Successful workspace drafts reveal the composer by closing
the covering panel at narrow widths; failures leave the panel open.
Keep the foreign-draft guard in workspaces.ts.

New workspace creation passes an ownership callback through
`newWorkspaceSessionOpened` to the pre-apply point of the asynchronous open.
Check it before remembering or resetting the composer, not only after opening.
The form retains its creation receipt across page remounts, and cancels only
its own unconsumed pending-send object when navigation changes or it unmounts.
Later conversations in the same cwd must not inherit that first goal.

### View recovery

`components/RecoverableView.tsx` wraps each page's Suspense inside its Show.
Work gets the error boundary only (`suspends={false}`): it is eager, and
its section reads must never sit inside a page-wide Suspense boundary
(`pages/Work.tsx`), or a pick swaps the page out for the fallback and drops
focus (overnight 12 A0b). The Preview pane has its own compact
`RecoverableView` (`suspends={false}`) beside the page in `App.tsx`, outside
every page's Suspense, for the same reason. A page's render or lazy-load error names the page, offers Copy
details and Reload page, and leaves the rail and top bar outside the failure.
Leaving the page disposes the failed owner; returning creates a fresh one.
The Work host remains eager, but Home and the destination tabs in `YourWork`
load as one lazy view when no workspace is selected, outside Suspense. This
keeps their code out of the launch shell without changing resource ownership.
SettingsSection uses the same boundary with Retry inside its body, so its
header and the Settings index remain usable.

An `OutsideSuspense` root has no owner, so no boundary above it sees its
errors: put a compact `RecoverableView` (`suspends={false}`) inside the root.
Work's Today (each of its Day and Week ranges), Map and Running tabs do: a
failing view names itself ("Today", "Weekly review") and offers Retry while the
tabs and the Day / Week switch stay usable (run 13 FIX-EB).
Review and Incoming use the same compact boundary inside their isolated
roots (Runbooks did until it was cut, overnight 11-08 S4). Their normal resource-level failures stay local; an unexpected
render failure offers Retry and Copy details without removing destination tabs.

A page boundary is the last resort, not a file view's error state. Reading an
errored Solid resource (`r()` or `r.latest`) throws into it, so one missing
file once took all of Work down (10-23). A view that reads a file shows
its own failure: read through `unlessErrored` / `latestUnlessErrored`
(`stableResource.ts`) and draw `r.error` in place with Retry. The Preview
pane's file view (`ChatPreview`), a Result's preview and its Compare with
accepted, a page review in Results, the source picker and Plan's Text view do.

Value, Routing and Agent updates guard errored resource reads and
offer local Retry through `ResourceReadError`. Baseline inputs stay unavailable
when their data could not be read. Activity reads spend without suspending,
shows an unknown dash with the failure reason, and offers separate spend and
run-list retries. A retained run list is explicitly labelled after a failure.

Memory and History have local Suspense and `preserveDuringRefetch` resources.
Refresh failures identify retained stale data; first-read failures never become
empty lists or zero counts. History remounts its resource owner when notePath
changes, preventing one note's retained history from appearing under another.

`store/customIcons.ts` shares app and Main agent thumbnails with separate stale-read
guards, one initial read per target and one native change listener. App updates also
set the existing favicon; Reset restores its original URL/type. HollowMark and the
Main agent indicator consume those signals without adding reads per instance.

[`store.ts`](../../src/store.ts) is a compatibility facade: it re-exports
`store/runtime.ts` and `store/theme.ts`. `store/runtime.ts` in turn re-exports
the domain controllers moved out of it in plan L3 X19 (2026-10-14):
`store/chatRuntime.ts` (the on-screen conversation), `store/chatApprovals.ts`,
`store/overlays.ts`, `store/snackbars.ts`, `store/activityDock.ts` and
`store/plaud.ts`, so `store/runtime` imports keep working. New domain state goes
in a narrow module under `src/store/` (`workspaces.ts`, `browser.ts`,
`waitingOnYou.ts`, `runQueue.ts`, `topBar.ts` …). Conventions:

- **Remote datasets are `createResource`** with a manually-triggered fetcher,
  exported alongside a `refetchX()` helper. Pages mount, the resource lazily
  fetches, every component that reads the signal updates automatically. Shared
  page resources pass through `preserveDuringRefetch`: the initial load still
  suspends normally, but a later live refetch reads the last resolved value so
  the app-level Suspense boundary cannot blank the active page.
- **Only the latest request answers:** a view that reads on its own (a
  signal, not a resource) drops a stale response with `latestRequest()` from
  `src/latestRequest.ts` (plan L3 X21a, 2026-10-15), the one way to do it:
  `const t = reads.start()` before the call, `if (!reads.isLatest(t)) return`
  after it and in its `catch`/`finally`, `reads.cancel()` in `onCleanup` (or
  when the input clears), and `reads.peek()` for work that belongs to the
  current request without replacing it (WorkFileEditor's save against the
  open file). Debounced lookups use `runLatest(order, request, deliver)`. Use
  one instance per independent stream (BuildChanges keeps status and diff
  apart). The stores use it too (X21b, 2026-10-15): one module-level
  instance per stream (`workspaces.ts` keeps the workspace open, the rail's
  git states and its session lists apart), `cancel()` where a store drops
  what is in flight (`showWorkOverview`, closing a prep pack,
  `resetFeedbackState`, a triage mark), and `peek()` where a read joins the
  current request rather than replacing it (`refreshWorkspace`, the launch
  restore, feedback reads and saves, triage's arrivals state, the "needs
  you" list's Show more). So do the pure factories `createConversationSearch`,
  `createNeedsYouList` and Settings' jump-to-control. `createNeedsYouList`'s
  Show more joins the current ticket so a later reload drops its page, and
  does nothing while a reload is in flight (`reloading()`, S01d). Don't hand-roll `const mine = ++gen` (or `= gen++`,
  `= (gen += 1)`): `scripts/check-latest-request.mjs` fails on one in the
  migrated files. A
  counter that is an id or an ordered sequence number (`nextChatAttachmentId`,
  `gitFreshVersion`, feedback's `saveEpoch`, which `mergeFeedback` compares
  with `>`) is not a request guard and stays a number.
- **Lazy enables**: most resources gate on an `_enabled` signal (e.g.
  `_tasksEnabled`). The relevant page flips it on mount; once enabled it stays
  enabled for the session. This means a drag from Tasks → Projects sidebar
  still has data after navigating away.
- **Startup warm-up is minimal:** `warmUpAllInBackground()` schedules only
  the available-agents step, and each view enables its own data. Don't add
  top-level fetches without checking boot order.
- **Nav history**: `setActivePage()` pushes onto a 50-entry back/forward stack.
  `navBack/navForward`, mouse aux buttons, and `Cmd+[`/`Cmd+]` all use it.
  Vault notes can stamp their path into the current entry via
  `requestOpenVaultNote()` so back/forward restores the specific note.
- **localStorage persistence** for: theme, sidebar collapsed, vault/chat
  aside widths (`vaultAsideWidth` / `chatAsideWidth`), chat
  panel open + width, chat session id, chat cwd, chat model,
  chat permission mode, the modes for new chats (per agent since 2026-11-12:
  `newClaudePermissionMode`, `newCodexPermissionMode`, `newGrokPermissionMode`,
  `newAntigravityPermissionMode`, written only by Settings; the older single
  `newChatPermissionMode` is retired and no longer read), chat prompt history (200-cap), chat context toggle,
  palette recent commands (`paletteRecents`, 24-cap, in CommandPalette.tsx).
  Pattern is `try { localStorage.setItem(...) } catch {}` — never let storage
  failure break a setter.

The mounted automation form registers `store/navigationGuard.ts` before a
page/history/workspace transition. The store owns destination request identity;
the lazy `pages/workflows/draftGuard.ts` owns confirmation and full-form revision
checks. Clean navigation remains synchronous. Keep mutation of workspace and
history state inside the guarded callback, and dispose the guard with its form.
Work's destination selection and the retained activity-opening action commit
at the start of the accepted page jump, before exposing the page to reactive
observers. Refusal or a superseding request preserves
the prior selection. Deferred view transitions use the
post-guard overview helper; the runtime page setter still checks its own guard.
Choosing the already-active non-overview page stays a no-op without a discard
prompt. The inherited clean-to-dirty race during an animated jump remains open.

### Chat (global, survives navigation)

Live proposed plans use `hollow:proposedPlan {runId,text}` panels. The backend
captures only explicit `ExitPlanMode` / `exit_plan_mode` tool events with a
nonblank `input.plan` payload behind the `planProposals`
adapter capability, main thread only. A Plan-mode successful or permission-denied
finished turn may publish the bounded/redacted proposal. Failed-resume probation
is discarded; accepted account continuations retain the candidate. Arbitrary
prose and todo progress never create a proposal card. Adapters without an
explicit text payload show no card, including file-only proposal events. No
arbitrary preceding narration is used as a plan. This live affordance is not reconstructed on
reload; the original reply text remains in the saved transcript.

`planProposal.ts` checks the latest completed proposal, idle/loading state and
the absence of a later user turn. `components/ProposedPlan.tsx` uses native
details. Accept/Refine prepare visible, source-run-specific composer drafts,
preserving existing draft text and the current permission mode. They never
send, execute or grant access; the normal composer Send remains the final act.
Local reply thumbnails and attached user images open the lazy `ImageZoom`
native modal dialog. It shows already-loaded data URLs only, with zoom controls,
Fit, actual size and Open in Preview. Native focus trapping and Escape retain
keyboard ownership; the existing modal guard recognizes its modal class.

HEIC byte signatures trigger real AppKit PNG conversion before Mac or phone
attachment saves. Input MIME/name do not decide conversion. Output is bounded
to 8192px per side, 40 million pixels and the shared 10 MB preview limit.
Mac saves mark actual conversions with `-heic-converted.png` after the generated
unique suffix. The composer uses that marker to replace original bytes, MIME,
thumbnail and byte accounting on every input route, then rechecks total limits.
Phone saves describe the converted bytes as image/png. Invalid or oversized
conversions fail explicitly before saving. Other attachments keep existing caps.
Reply-only `replyRichText.ts` adds `$inline$` and `$$` block math, GitHub alert
blockquotes and exact `hollow://conversation/<key>` link chips. KaTeX runs with
trust disabled, bounded input/expansion/size, and native MathML output. The
existing sanitizer keeps its HTML profile with an explicit MathML tag/attribute
allowlist; SVG, embedding and network math tags are not enabled. Raw authored
HTML and remote images still pass the original sanitize/consent pipeline.
Thread chips call the guarded exact-key conversation opener on a click only.
One shared click ticket spans reply components. Conversation ownership and the
workspace request token are checked across target lookup and workspace reads;
intentional source unmount on navigation does not cancel the accepted request.
The ordinary note/file renderers keep their existing Markdown behavior.
Context preparation emits `hollow:handoff` panels for fresh follow-ups and failed
resume retries. `runId`, `state` (pending, complete, unavailable) and `entries`
are compiler evidence only. Complete means a nonempty continuity block survived
prompt fitting, not agent receipt. The selected live run owns one replaceable
native-details divider; foreign and stale run snapshots are ignored. This live
progress is not reconstructed on reload; the run record retains compiler evidence.

`chatTurns`, `chatSessionId`, `chatCwd`, `chatModel`, `chatPermissionMode`,
`chatBusy`, `chatInput`, `chatHistory`, etc. live in `store/chatRuntime.ts` and
reach components through `store.ts`. A single
`listen<ChatEventPayload>('chat-event', …)` at module init keeps streaming
turns landing in the store **even when Conversation history is unmounted**,
e.g. the user leaves Library → Conversations while `claude` is still generating.

**Subagent attribution (AG7, 2026-11-17).** A Claude subagent's `toolUse`,
`toolResult`, `toolImages`, `assistantText` and `assistantDelta` events carry
`parentToolUseId`: the id of the `Task` call that spawned it, read by
`chat::subagent_parent` from the stream line's top-level `parent_tool_use_id`.
A main-thread event, and every event of an agent that doesn't report the id
(Codex, Antigravity), has no such key, so old readers see what they always saw.
The run and conversation are already on every event (`queueId`,
`conversationKey`, `clientTurnId`). The stored `toolUse`/`toolResult` events keep
it too, so `read_conversation`'s tools (`ConversationTool.parentToolUseId`) and
the live transcript row (`ChatToolUse.parentToolUseId`) both have it. Not yet
done: ChatBody folding a subagent's calls under its `Task` row (lane U's file),
and the phone feed's `tools` (unchanged; `docs/PHONE_CHAT_API.md` has no new field).

**The chat's name and ↑ recall (overnight 11-07, D1).** `components/ConversationTitle.tsx`
sits at the left of ChatBody's `.chat-navigation` row (the tools' slim row, so a chat with
tools gains no height) for a workspace conversation, not in the Main agent's space, the
compact panel or Library → Conversations (which keep their own heading). It reads the open
chat from `workspaceSessions()` (the store carries the open one past each project's newest
25) and shows nothing when the list doesn't hold it, so it never names the wrong chat
(`chatTitleShown`). Its ⋯ is `MoreButton` over `sessionMenuItems`, the one conversation
menu; a double-click on the name, or Rename… in that menu, turns it into a field that
saves on Enter or blur through `renameConversationTo` and cancels on Escape (which stops
there, not at Esc-stops-the-run); Enter and Escape give the composer its focus on a
microtask and the rename runs behind it (`renamedTitle` decides empty or unchanged is no
change). The field reads its start once and keeps its draft in `ConversationTitle`, never as a
bound `value` (Solid would write the saved name over the typing on every refresh of the
sessions; a draft handed back after the row left the list and returned keeps its caret at the
end instead of being selected), and opening another conversation ends the edit. In an empty box ↑ walks
`recallList(chatTurns, chatHistory())` (`messageActions.ts`): this conversation's own
messages newest first, skipping Hollow's own turns and any message whose text holds an
attachment block, then the earlier prompts from every chat. It is read per key press, never
as a memo (a memo would read every turn on every streamed token), and the history place
resets when another conversation opens.

**Retry with another model (overnight 11-07, D1, gap 10).** `ChatBody` reads the current
agent's catalog into `catalog` (a signal fed by `loadCatalog`, the cache the model
menu fills for every installed agent; never a resource under the transcript) and
`retryModels()` is `retryModelChoices(models, chatModel(), { newestByFamily })` (`modelMenu.ts`:
one row per family, since `opus[1m]` and the pinned ID an alias resolves to are the same
model; the family in use left out, however it is named; with Claude's `newestByFamily` the
families come first, then self-hosted models, then the rest, so the four rows are Opus,
Fable, Sonnet and Haiku less the one in use; an empty model, the CLI's own default, excludes
nothing, since Hollow doesn't know what it resolves to; four at most). The latest
reply's `TurnFooter` takes them as `retryModels` (More: "Retry with <model>", after the
agents), and the failed-turn row has a "Retry with another model" `ActionMenu`. Both end in
`retry(agent?, at, model?)`, which calls `setChatModelPersistent(model)` before the send, so the
chat keeps the model (the model button shows it) exactly as an agent retry keeps the agent.

**Quote selected text (overnight 11-07, D1, gap 9).** `QuoteSelection` (a child of
`.chat-scroll-wrap`, after the scroller) reads the selection on a document `mouseup` (a drag
may end over the composer) and 120 ms after a `selectionchange` with no mouse button down
(Shift and the arrows, Select All; a press whose mouse-up never arrives, a window `blur`
mid-drag or a ctrl-click's `contextmenu`, stops counting as a button down), and follows
`scroll` on the transcript scroller. The
selection is cut to the scroller (`within`), ignored in `.chat-find`, `.chat-input` or a
field, and its passage leaves out what a drag crosses but isn't the text (`CHROME`: buttons,
a code block's or table's header, the turn footer, your message's actions, a canvas, `.sr-only`
words) and a folded long message whose clipped lines it reaches (`.chat-user-clipped`; Show
more first). With text left, a Quote button sits centred 8px above the first selected line in
view (below the last one within 44px of the scroller's top, else just inside the top of a
selection taller than the view; hidden while none of it is in view). Its click reads the
selection again, takes each kept piece's text from `Selection.toString` (block line breaks
kept), calls `quoteIntoBox` in `ChatBody` with
`quoteMarkdown(text)` (`selectionQuote.ts`, which cuts at 4,000 characters but never between the
two halves of an emoji): `addToDraft` puts the quote after any draft, the box
takes focus and the caret goes to the end, under the quote. While a reply streams, a
selection in any finished block holds (smooth streaming keeps those blocks mounted, gap 8);
one in the block still being written goes at its next flush (~80 ms).

**A new conversation's pane (overnight 11-07, D1, gap 7).** A project conversation with no
turns centres its greeting, message box and starters by CSS alone
(`styles/chat-workspace.css`, last block), scoped to
`.work-conversation.chat-main:has(> .chat-scroll-wrap > .chat-scroll > .chat-empty)`: the
wrapper, scroller and empty block take `display: contents`, their children and the form
(a sibling of the wrapper) take `order` in the host's column, and `::before` / `::after`
spacers centre the group. The box keeps the pane's width (the 32px gutters only). The
Main agent's page, compact hosts and Library's reader are outside the selector; under
520px of window height the old layout stays. Nothing in `ChatBody.tsx` knows about it, so
a change to the empty block's children needs its `order` row. `chat-empty.spec.ts` pins it.
The whole group fits the pane from 521px of height up (measured; the box ends at most 355px
down). What opens upward from a mid-pane box is capped to fit: the + menu and the usage
menu are capped to fit. The ring card measures its available height and closes
on resize or a conversation switch. Folder and connectors uses the shared
action menu with two rows. Known limits: Tab and a screen reader follow the DOM (greeting, Runs on,
starters, box) while the box is painted before the starters; the empty scroller has no box,
so `chatScrollController.ts:644` reads it as hidden.

**Save conversation as note and Read aloud (overnight 11-07, D1).** The Conversation tools'
fourth button saves what Copy conversation copies (`conversationMd` in ChatBody, the same
`conversationMarkdown`) through `store/replyNote.ts`'s `saveConversationAsNote`: one shared
`saveNote` with the reply's Save as note, so the same path (`replyNotePath`: Results'
`outputs/answers/`, or `library/` outside a workspace), `-2`, `-3`… rather than overwrite, one
save at a time and the same toast with Open. The note is titled by `conversationNoteTitle` (the
saved name, else the first thing asked, else "Conversation") and `replyNoteBody(…, 'conversation')`
says what it was saved from. Both save handlers (`saveConversation`, `saveReplyNote`) read the
text, title and workspace at the click, before awaiting the lazy `store/replyNote` module, so a
conversation opened while it loads is never saved under this one's name. A reply's row button
**Read aloud** / **Stop reading** (in More actions until 11-12)
(`TurnFooter` props `onReadAloud`, `reading`; absent when `speechSynthesis` is) is
`store/readAloud.ts`: `speakableText` turns the reply's Markdown into words (a code block
becomes "Code block skipped.", a link its text, an address "link", headings, list items and
table rows end like sentences), `speechChunks` cuts it at sentences into pieces of at most 240
characters that share one utterance when short, and `readAloud(key, md)` queues them on the
system voice, one reply at a time (`readingKey` is a signal of `<session>:<turn>`; a new read
or `stopReading()` bumps a run counter so the cancel's `onerror` is ignored). WebKit
(238189) speaks what is queued right after a `cancel()` but never reports its end, so a read
that interrupts another waits 80 ms after the cancel, and the end is also read from the voice
(`speaking` / `pending` idle for three looks 400 ms apart). A reading reply's Read aloud button says Stop
reading and its actions stay at full opacity (`.turn-footer.reading`); the chat's top
row (`.chat-navigation > .chat-nav-reading`) shows one too, for a reply that has scrolled out of
the rendered window; the voice stops when another conversation opens or the page changes
(the Work page stays mounted behind the others), whichever view started it, and when the view
that started it is hidden (a `ResizeObserver` on the transcript sees it
with no size) or unmounts. Those last two pass the view's own token (`readAloud(key, md, by)`,
`stopReading(by)`): the quick-chat popover closing, or a page staying mounted and hidden, never
cuts off a read another ChatBody started. A reply's words come
from `speakableText`: inline code is set aside before any rewrite and read as typed, an address
keeps the punctuation after it, a list number with a full stop has at most three digits ("2024."
is a year; "1000)" is an item), emphasis hugs its words, and a fence (no backtick in its info
string) is skipped: indented four or more only under a list item, where it ends at a line
indented less than the item's text (`itemText`, not the fence's own indent).

**Code blocks and tables in a reply (overnight 11-07, D1).** `CodeBlock.tsx` has a **wrap**
toggle beside copy: one setting for every block (`aria-pressed`, `.is-wrapped` →
`pre-wrap` with `overflow-wrap: anywhere`), kept per machine in `localStorage`
`hollow.codeWrap` (`'1'` or absent). It lives in the module, not `store/prefs.ts`, because
`prefs.ts` is in the launch chunk (under 1 KB of its budget left) and only a reply reads it.
Copy still copies the code as written. `ReplyMarkdown.tsx`'s `transform` wraps each `<table>`
in `.reply-table`: a bar with **Copy table** (the button rides in the HTML, its click is
caught in `open`) above `.reply-table-scroll` (a focusable `role="region"` named "Table 2 of 3"
only when a reply has several, so a wide table scrolls by keyboard too). Cells of more than 60
characters get `.long` (they wrap, in a column of at least 20em); every other cell is
`nowrap`, so short values never break mid-token and a wide table scrolls instead. Copy table
(`replyTable.ts` `copyTable`) writes tab-separated text and the HTML table together
(`ClipboardItem`; plain text only where the webview refuses that); a cell's tabs, line
breaks, `<br>`s and the start and end of each block (`BLOCKS`: divs, list items, paragraphs…)
become spaces (`cellContent` swaps each `<br>` and pads each block with a space first:
`textContent` alone would join "first<br>second"). A finished table's box stays mounted while the reply
streams on (smooth streaming, below), so it keeps its scroll, focus and "Copied" label by itself
(the label's 1.2 s revert timer is one per button in `ReplyMarkdown`, so a second copy inside the
window keeps its label for its own full time). A table
still being written is rebuilt with its block each flush: `ChatMessage` owns the offsets
(`tableScroll`, one array per segment index, as `expandedCode` does for code) and hands each
segment its own (`scroll`); `keepTableScroll` (`replyTable.ts`, called from `decorate`) puts a
new box's `scrollLeft` back by order and watches each box once (a `WeakSet`, so a kept box gets
no second listener). The header row keeps its rule (`.reply-table tbody tr:last-child` names the body: the
header's only row is also a `tr:last-child`; a header-only table drops the header's rule, as the
box closes it).

**Smooth streaming (overnight 11-07, D1, gap 8).** A streaming reply re-parses about every
80 ms. `ChatMessage` renders its segments with `<Index>`: a Markdown segment's row is a
non-keyed `<Show>` that stays mounted while its text grows (so its `ReplyMarkdown`, its
`images` cache and its `placed` map live through the turn), and every other kind sits in a keyed
`<Show>` that rebuilds when its segment object changes, as the old keyed `<For>` did (a growing
code block, and every non-canvas row once at the turn's end, so an html fence still becomes
its live frame). `MarkdownView`'s `apply` prop replaces the `innerHTML` swap:
`ReplyMarkdown.keepBlocks` parses the new HTML in a `<template>`, keeps each leading top-level
node whose parsed `outerHTML` equals last time's, and replaces the rest (state per root in a
`WeakMap`; any other change to the top level starts it over). The DOM equals a whole swap's,
and `decorate` runs over the whole view each time, so it is idempotent: `mark` skips marked
`code`, `markProsePaths` skips `.file-ref`, the local-image worker takes only buttons not yet
placed, a loaded image's click checks the view's conversation rather than its pass, and
`keepTableScroll` watches each box once. A cached picture read is shared by later passes, so
its slot check is `owned()` (view connected, same folder and conversation), never `current()`
(which is false once the next pass starts: a read still queued behind two others would reject
and the pass that reused its promise would mark the picture unavailable for good). `placed`
is a `WeakMap` that stamps each filled button with the folder and conversation it was filled
for, and a pass fills a button again when its stamp differs: the reset effect and the view's
decoration both run on a change of the conversation version and Solid doesn't order them, so
neither the skip nor the read cache can depend on that effect having cleared anything (it left
a kept picture with the old pass's dead click after an agent switch; the `images` cache is
keyed by folder, version and file, so a pass never reuses a read queued under an older
folder or version). A
picture that loaded for another folder and fails to read in this one is disabled again, with
its label, click and stamp dropped (`failed()`). Only a picture that loaded is
`placed`: a failed one is tried again on the next pass, a flush or the turn's end (`ChatMessage`
hands out new segment objects then, and `decorate` reads `props.text` through them), since the
file may be written later in the turn. Known limit: the prose-path marker splits a finished
paragraph's text node a microtask after the paragraph first paints (and once more when
`homeDir()` first resolves, for a `~/` path), so a selection made in that instant drops; later
passes skip marked paths. What it buys: a selection, focus or "Copied" label in a
finished block holds while the reply streams on (the Quote button needs that), and settled
images and file marks aren't redone. A new table changes every table's label ("Table 2 of
3"), so the blocks from the first table on are rebuilt then (rare). The caret selectors
(`.chat-msg.typing > .md-view:last-child …`) see the same DOM.
`.chat-msg > .md-view` has `max-width: 100%`: the message is
`align-items: flex-start`, so without it a table's unbreakable cells made the whole reply as
wide as the table and pushed the pane sideways. Reply tables had no border or padding at all
before 11-07; the styling is scoped to `.reply-table`, so notes and the Library reader
(`.viewer-rendered table`) are unchanged.

**The transcript reducer (overnight 10-11 B3).** Text, deltas, tool calls and
results, images, receipts and the turn boundary land through
`chatTranscript.ts`'s `createTranscript(setTurns, runId)`; `store/chatRuntime.ts`
keeps `appendAssistantText` and the rest as thin wrappers. `chatHydrate.ts`'s
`turnsFromConversation` maps a saved conversation to `ChatTurn`s for
`openWorkspace` (Conversation history keeps its own copy with its notices). Turn types
live in `chatTurnTypes.ts` (re-exported from the store). A `chat-event` that
neither the conversation on screen nor a backgrounded one claims is dropped,
except that a finished turn still refreshes the session lists. Background
snapshots also retain the permission mode (P0-21). A send still getting ready
(the agent check, the working-folder preflight, a card's answers) holds its
conversation on screen until the turn is queued (`holdConversationForSend`):
`canLeaveConversation` and `backgroundRunningChat` refuse meanwhile ("still
being sent"), so the send can't carry on into the conversation opened next
(run 15 FIX-FINAL-P). Run 15 P3 (2026-10-27)
removed the second conversation engine the Main agent drawer ran beside the
page, with its pinned-event routing and its read-only background mirrors.

`confirmPermissionModeChange()` is the shared danger-mode confirm flow. Every
surface that switches permission mode (Settings dropdown, the composer's
access chip and the model menu's Permissions row in the shared composer, the
`/permissions` slash command) goes through it. Its title names the danger mode
in the chat's agent's words (`confirmPermissionModeChange(next, agent)`, names
from `agentPermModes.ts`; Grok's Auto asks too, its sandbox is off).
Every chat surface names modes from `agentPermModes.ts` (each agent's own
names, chip names, one line each and `hollowCardsLine`); `PERM_MODE_LABELS`
stays only for automations, which pick no agent's words. A person's agent
switch goes through `switchChatAgent` (nearest mode there, one flashed line;
never into a mode that skips every check unless the chat was in one);
restores keep using `setChatAgentPersistent`, which leaves the mode alone. The three
per-chat pickers capture `sameChatAsNow()` (the agent session key and
`chatConversationVersion`) before it opens and do nothing if another chat took
the screen meanwhile. The Settings dropdowns set only the modes for
new chats (`setNewAgentPermissionModePersistent`, applied to an empty chat by
`applyNewChatPermissionMode`); the chat's own mode changes through
`setChatPermissionModePersistent`, which returns why it refused (a running
turn, a conversation still opening) or '' (P0-19). In the composer the agent,
model, effort, permission mode and Canvases are all picked in
`ChatModelMenu.tsx` (the separate `ChatSettingsMenu` permissions button was
folded in on 2026-11-01); don't re-add inline header dropdowns or another
composer button.

Effort saves return a success boolean. A failed model-menu save restores the
previous effort and Ultracode choice only while the same chat remains current,
and shows a short footer error. Writes remain ordered per chat and retain the
existing cache refresh behavior; a chat without a storage key resolves success.

Per-chat permission memory (chat-first port) lives in `permModes.ts`:
one bounded `hollow.chat.modes.v1` record holds the latest 500 choices. The
in-process choice wins over older durable data if storage writes fail. Older
evicted choices fall back to the latest recorded execution mode; unknown modes
start in Accept edits (`acceptEdits`). A one-time migration retains the last visible pre-upgrade
choice; retired drawer choices migrate when that exact conversation opens.
The `AgentConversation.permissionMode` field is optional for older responses.
Workspace and Conversation history restores resume native only when the known
recorded mode is in the selected one's class (`permModes.ts` `modeClass`, the
backend's `chat_resume::mode_class`: Plan, the acting modes, or Grok's sandbox
profiles; 2026-11-04, so an Edits-only check-in and a Bypass chat share one),
otherwise retained Hollow context starts a fresh native session. History is conservative while pending or failed; stale
responses cannot change the newer conversation. Live background snapshots restore
their own mode before buffered events. Settings owns one new-chat mode per agent
(2026-11-12): Auto for Claude and Codex and Accept edits for Grok and
Antigravity until one is saved; the single setting from before is retired (it
was re-seeded from the last chat's mode, never a choice).
Quota failures replace the stale record with a tiny JSON `"plan"` sentinel;
every omitted chat then uses Plan instead of a potentially broader recorded
mode. Later successful writes retain that fallback alongside the choices as
`{fallback: "plan", choices: [...]}`. Normal records remain arrays. The picker
warns on any incomplete save; migration is not marked durable. If storage is
completely unavailable, only this process's memory can retain the policy.
Startup resolves the current key with its paired mode copy before adoption,
and events for an
existing key never author a new choice. An adopted turn (a check-in or wake
this window didn't send) keeps its native id at `done` only when its mode,
which `runStarted` carries and the queued turn keeps, is in the composer's
mode's class; one adopted mid-run, after its `runStarted`, still drops it. A
mode change in the chat restarts the session only across classes. Every mode apply also updates the startup setting.
`ArtifactSessionContext.permissionMode` gives artifact Continue the same
recorded-mode check; Continue from summary uses the new-chat default. Its
preflight holds navigation, then batches a non-busy reset and provider apply.

The composer draft and images survive navigation and a restart. A project's
main agent counts as one of its conversations before its first turn
(`workspaceConversationKeys`, `src/lastConversation.ts`), so launch restore and
"Open workspace conversation" reopen it with its draft instead of a blank New
chat (overnight 11-03 R4). Once that conversation is moved to another
project it keeps this project's key, so both open the newest conversation
instead (`mainAgentMovedAway`, `src/store/workspaces.ts`). "Work on this", "Update
brief", Branch from here and an agent action's prefill (`askClaudeInChat`,
`appendToChatDraft`) put their request after the
draft with a blank line, never over it, and not twice
(`chatQueue::addToDraft`, morning M5); a stopped turn ends as a
neutral system row; `sessionReset` clears the native id and that key's
question cards; the cost chip (`src/usageFormat.ts`) never shows `$0`. A
reply ends in one footer (`components/TurnFooter.tsx`, `src/turnFooter.ts`)
and a finished run's steps fold into one row (`src/runFold.ts`). Overnight
11-12 (C1, C2): the fold row reads "Worked for 1m 10s ›" (`foldSummary`; the
step count is in its tooltip and read out, `foldSteps`; the time runs to the
last tool result's `endedAt`, live only, else the receipt's finish) over one hairline, and
the reply's changed files sit right-aligned above it
(`components/ReplyFileChips.tsx`, #1280's type icons by `fileKind`; fed by the
receipt on the reply's last assistant turn, `foldTail`); an unfolded reply keeps
them above its footer. The footer is one quiet row (`src/replyActions.ts`
`replyRowButtons`): Copy, Useful and Not useful (only with an exact run id,
`canVote`), Read aloud, Retry on the latest reply, More, then the finish time
(`replyTime`: "11:27 PM" today, "Sunday 11:27 PM" this week) with any
duration, usual time, canvas and CFO words. Older replies' rows are at opacity
0 until hover or focus (the box stays). A thumb saves through
`record_turn_feedback` (`store/replyFeedback.ts`, a plain signal of saved
votes by run id, loaded by `list_feedback` 200 runs at a time; saves on one run
go in click order and a failure restores the last confirmed vote unless a later
click took over; a loaded vote is dropped only when a save succeeded after the
load was sent, else it is the confirmed vote and shows once no save is out); Not useful then
offers an optional one-line reason (`.chat-feedback-reason`). A footer
chip opens its file in the Preview pane, except, in a workspace
conversation, a file Results lists in that workspace's `outputs/`
(`chipTarget`, mirroring Rust `workspace_results::is_listed`), which opens
it in the Preview pane with its review bar (`openWorkspaceResult`,
`components/ResultReviewBar.tsx`); **Review result** is on that chip's
right-click menu and **Review <file>** in the reply's More actions. The same menu's **Save as note** (One Ask, 2026-11-03;
`store/replyNote.ts`, loaded on first use) writes the reply's Markdown through
`vault_create_note`: into the workspace's `outputs/answers/`, or `library/`
from a chat in Chats, the Main agent or a chat outside a workspace
(`turnFooter.ts` `replyNotePath`), trying `-2`, `-3`… rather than overwrite,
and its toast's Open shows it in the Preview pane. A reply that could
look verified but isn't gets a check chip there (`receiptAttentionLabel`:
an unbacked pass claim, stale or unverified checks, cites that don't
resolve) that opens and focuses its Run record (P0-23). **Whiteboards
were removed on 2026-10-07** (diagrams are `hc-flow` canvases): an old
`hollow-whiteboard` fence shows as a one-line notice and its JSON
(`chatSegments.ts` `RETIRED_BOARD_FENCE`); there is no board view, editor,
side-panel card, Library Boards view, Results preview, board command or app
action, `jev.whiteboard` setting or Jev layout use. A leftover `.board.json`
in `outputs/` is listed as data. The canvas id rule lives in
`canvas_fence::valid_canvas_id`. History: Whiteboards.
**Canvases** (`hollow-canvas` fences, Canvas C3) render in place in every
chat (`components/ChatCanvas.tsx`, `src/canvas/chatCanvas.ts`): nothing extra
until the root tag closes, then a "Drawing <title>…" skeleton at the root's
`height`, then the canvas in Canvas mode as soon as the fence ends, even
mid-turn. A broken one (invalid, or a frame `error` message) shows its source
and **Ask to fix**, which drafts a fix request in the composer and never
sends. **Stray tags** (overnight 10-11/12 A2, `src/canvas/strayTags.ts`):
ChatCanvas runs `escapeRowTags` on the fence body before it is parsed, saved
or drawn, so inside the kit's data-row elements (`ROW_ELEMENTS`: hc-chart,
hc-bars, hc-table, hc-flow, hc-sparkline, hc-timeline, hc-heatmap and the ask kit's choice, multi, rank, matrix and
pick) a `<` that doesn't open a `ROW_TAGS` tag, an `hc-` element or the row's
own end tag becomes `&lt;`; in hc-code and hc-file every `<` but their own end
tag does. An allowed end tag that closes nothing the row opened ends the row,
as the browser would. The store keeps the escaped body (the Rust validator and
the fence golden are unchanged); `resolveCanvas`'s `written` still finds a
canvas saved raw before, and the Expand key uses the bytes as written. Outside rows, an
`unclosed` canvas names what hid its `</hc-canvas>` (a raw-text element such
as `<title>`, an open comment, `<plaintext>`, a tag whose quote never closes),
its line and the fix (`unclosedCause`/`unclosedReason`, used by
`chatCanvasView`); Ask to fix quotes it. The kit prompt says to write `&lt;`
for a text `<` outside rows, ask options and hc-code (its 3,072-byte budget is full). A fence still open when the reply settles (Stop, an agent error, a
dropped closing fence) reads as ended (`settleCanvasBody`): truncated it is
broken, whole it renders. A fence that becomes ready during a live turn
(streamed, or arrived whole while its run is `chatLiveRunId`, as Codex
replies do) is recorded once in the conversation's canvas store;
re-rendering or reopening only reads (`read_canvas`), so a turn that
finishes while its conversation is not on screen is not saved. Html fences render in place too and open in Canvas mode in
quick and workspace chats, Offline elsewhere. The head shows the canvas's
version ("v3"; "v1 of 3" plus a line saying so on a reply whose fence is an
earlier revision); with more than one kept revision the label opens the
version list, and picking one renders it read-only from `.revisions/`
(`read_canvas_revision`). Picking, "Back to vN" and Escape in the list
hand focus back to the version label (`keepFocus`, once the revision has
loaded) and say the change in the canvas's mounted live region
(`.chat-canvas-live`; Results' canvas too). The person's view state (bridge `state`) is saved
from the current revision and restored through bridge `init`, also on the
next revision. **Ask-canvases** (C4 A2, `src/canvas/askAnswer.ts`): Hollow's
own Send under the frame sends the answers as the person's next message
(`ChatBody`'s `sendFromCanvas`, also for the Preview pane's expanded copy via
`registerCanvasAnswers`; question cards, #628, 2026-10-14). Before that, Send
only filled the composer through `fillFromCanvas`. While the agent is
answering it holds them for after the turn ("Queue answers", P1-33); it
refuses for a locked revision or another conversation, and leaves what the
person typed in the composer as is. `canvas/askHold.ts` keeps one hold per
conversation and revision (`queued` with the held send's id, or `not-sent`
with the text), read by every copy: a send that fails never goes to the
message box; the canvas offers Send now and Take them back
(`canvas/askSend.ts` words the line). Send is off while a send is in flight
(collect through queue) and while the answers are queued. The message is `canvasAnswerText`:
`Answered canvas "<id>":`, one
`- label: value` line per answer (TS
`answerLine` and Rust `push_line` share `src/canvas/answerLines.golden.json`;
a question card's own answers are the question, then the answer)
and the exact JSON `{canvasId, revision, answers}` in a fenced
`hollow-canvas-answer` block. The block is hidden like `hollow-next`
(`splitHiddenFence`): the user bubble shows `visibleUserText`, Rust's
`next_step::strip` drops both blocks from vault session records, the agent's
prompt keeps it; since C6 P1 the learning job's prompt and the About-you
builder's `recent_asks` drop it too, and Rust and TS share one fence rule
(`next_step::split` mirrors `splitHiddenFence`; both pass
`src/canvas/hiddenFences.golden.json`). `fillFromCanvas`
(`registerCanvasComposer`, `composerFill`) has had no caller since the
curated panel Refresh went (COO rebuild L4, 2026-11-10); its rule stands: text the person wrote is never replaced without **Replace** /
**Cancel** over the composer, only the same canvas's own untouched fill is
replaced silently, and only Enter sends it. **The lock is read from the
transcript** (`answeredCanvases`): the first user message carrying a valid
block for `id@revision` locks that revision (frame `answered`, host note
"Answered…"); nothing is written, so it survives restarts with the
conversation, a withdrawn message never locks, and a new revision is a new
question. An open one says "Waiting on your answer" under the frame, and is
listed in Waiting on you (C4 A3) until its answer is sent: `needs_you`
reads the emitting conversation's stored messages with
`canvas_answers::answered`, the same rule as `answeredCanvases` (both pinned
by `src/canvas/answerBlocks.golden.json`). The store's ask heads are read
through `AskCache` (a sidecar is re-read only when its size, modified time or
inode changed) and sorted newest first before the 500 cap (C6 P3).
**Frame budget:** at most 6 live canvas and html frames per
transcript (`src/canvas/frameBudget.ts`; ChatBody provides the scope): host
IntersectionObservers on the scroller pick frames on screen, then within
600 px, newest first; the rest unmount to a placeholder at their last
height (with **Show**) and remount with their state through `init`. The
Preview pane and SmokeHarness have no scope, so their frames are always live.
ChatBody still publishes its scope (`publishWorkspaceFrames`) and
`SlotOptions.pinned` remains, but the curated section view that joined it as
a pinned slot is gone (COO rebuild L4).
**Canvas focus (10-11):** a turn that finishes with a `hollow-canvas` fence
(ask-canvases and broken ones too) scrolls the top of the reply's LAST canvas
to 16 px under the scroller's top instead of the bottom; tall canvases align
their top. The rules are the pure `src/canvasFocus.ts` (`replyCanvas`,
`createCanvasFocus`; unit tests beside it); ChatBody supplies the geometry
and events. Only someone following the stream is moved (someone scrolled away
stays put and gets the chip). Turn end is `chatBusy` true → false in the same
conversation (same `chatConversationVersion`; a key assigned mid-turn counts
as the same; a backgrounded, still-partial turn doesn't count), and only a
canvas at or after the turn count when busy began counts (`replyCanvas(turns,
from)`), so a failed send or withdrawn queued turn never lands on an earlier
reply's canvas; mounted mid-turn, the current reply counts. A canvas folded
into the finished run's "Worked for…" row opens that fold. The canvas may
re-mount as the reply settles, so the anchor looks for it for ~1 s before
ending at the bottom. While held, a ResizeObserver on the scroller and every
rendered turn re-anchors it (only after the first anchor lands; during a smooth
move it catches up on arrival), and `autoScroll` without `force` does nothing;
a forced one (approval or question card, send) releases it. The person's
vertical wheel, touchmove, scroll keys (outside fields; Space not on a
button), scrollbar press, the find/outline jumps, **Jump to latest**, and a
drift over 48 px that no listener saw (scroll chaining out of a frame) release
it; the next user turn, a new busy turn or another conversation release it and
the chip. **Back to canvas** (`.chat-back-canvas`, styled like **Jump to
latest** and stacked above it) shows while the released canvas is out of view;
Escape on it hides it until the next canvas; from the keyboard it focuses the
canvas, and if it hides while focused, focus goes to the composer. Reduced motion makes every move
instant. Smoke: `tests/smoke/canvas-focus.spec.ts` (runs with
`overflow-anchor: none`, as WebKit has no scroll anchoring to hide a missing
re-anchor). **Streaming follow** (overnight 10-11/12 A1): while a turn runs, a
ResizeObserver on the scroller and every rendered turn calls `autoScroll()`
when the content height changes, because ChatMessage draws streamed text up to
80 ms after its delta and canvas frames settle later still; following only the
deltas pinned a stale height (a canvas in the first delta left the chat at its
top). Same rules as the delta follow: only at the bottom, never past a held
canvas or a smooth jump; `autoScroll`'s frame callback re-checks `atBottom()`,
so a find, outline, canvas-reveal or scroll-memory jump made meanwhile wins.
**Opening settle** (10-12, `src/chatSettle.ts`): opening a conversation, switching to
it, or ChatBody mounting again on a loaded one (back from another page) scrolls
once while its canvases are still placeholders; their growth above the view left
it a canvas or two back from the end (WebKit has no scroll anchoring). Until the
content has been quiet for 3 s after it first changed (10 s at most), a
ResizeObserver puts the view back on its target: the end, or a scroll-memory
position. It ends on any interaction the transcript sees (wheel, key, pointer,
focus), including focus moving into one of its frames: a click inside a canvas
or HTML frame (H10, 2026-10-15) reaches no listener, since the frames are
sandboxed without same-origin, but it blurs the window with the `<iframe>` left
active (`src/chatFrameFocus.ts`, checked again one task later for WebKit). That
is handled as transcript focus: it also ends a reveal's or jump's reading hold
and cancels its deferred second look. It ends on a move it didn't make (a scroll
chained out of a canvas frame, which those listeners don't see), when something else lets go of the end (a reveal, a
find jump), and on a send, a new turn or a held canvas.
**Reading anchor** (`src/chatReadingAnchor.ts`): while reading above the end, hold a
visible content block through earlier canvas growth, including within the same reply.
Correct only residual viewport displacement after native anchoring. Input and navigation
recapture the place; opening/restoration, sends and held canvases retain priority.
A long message's Show more and Show less are the reader's own action, as a scroll is (the
follow and a held canvas let go); Show more recaptures the place once the words have opened,
and Show less keeps its button where it was (`toggleInPlace`, overnight 11-04).
Lifecycle and limits: [Chat motion](../CHAT_MOTION.md). Smoke:
`tests/smoke/canvas-open-settle.spec.ts`.
**Never read a pending resource in the transcript** (`resource()` while
loading): the conversation sits inside a Suspense boundary, and the fallback
flash resets its scroll to the top; read `.latest` once `state` is ready
(`settled` in ChatCanvas). **How often** agents draw canvases is Settings →
Chat → Canvases (Off / When it helps / Most replies, `AgentSettings.canvases`)
or the chat's own choice in the model menu's Canvases row
(`src/canvasFrequency.ts`; kept per conversation as
`hollow.canvases.<session key>` and sent as `chat_send`'s `canvases`, which
rides in the queued payload and wins over the setting; a turn a `hollow_app`
agent sends uses the setting). Saving Agents & models carries the live
Canvases value, never resets it. **Receipts** record each reply's
`canvases: {count, bytes}` (its `hollow-canvas` fences and body bytes,
`canvas_fence::reply_canvases`, pinned to `parseSegments` by
`src/canvas/replyCanvases.golden.json`); the footer shows "2 canvases ·
14 KB". `scripts/canvas-real-reply.mjs` is the **billed** real-reply probe
(free dry run by default; `--billed` sends one Claude and one Codex call;
since G1 it sends the fixture's `turn.visualGuide` block ahead of its visual
request, as Hollow would):
never run it without the owner's go-ahead. See CANVAS_C3.md,
revisions and state,
frame budget and
prompt, setting and receipts.

#### Mid-turn steering (2026-10-21)

`runStarted.steerSupport` is the resolved transport capability for that turn.
The foreground signal and background snapshot retain it until the turn ends
or resets. Agent-wide refreshes are request-ticketed and never
overwrite that snapshot. A legacy start event without the field captures the
current foreground agent's cached row when available.

While a turn runs, ChatBody keeps the box open. Its send button names what Enter does: `chatSteer.ts`
`defaultBusyChoice` gives Steer now when `steersNatively(agent)` (from `chat_steer_support`, read once by
`ensureChatSteerSupport`, asked again when a turn starts after a failed read) and `steerBlockedWhy` finds nothing in
the way (P1-30: an image, a draft the backend would refuse as too long, `steerTooLong`: over 3,000 characters or
7,000 bytes as JSON, a visibly queued turn, support not read yet), else Queue for after. **Send options** (an `ActionMenu`) has the
other choices for one send (a ruled-out Steer now says why; an agent without one reads the backend's note), and
**Stop** sits beside it. A one-line `.chat-busy-hint` (`aria-live="polite"`) says it in words, with the reason when
Enter queues. While a turn runs, `send` checks local commands first (`slashCommands.ts` `busyLocalCommand`): a read
runs, one that changes the next turn (`/model x`, `/agent x`, `/cwd x`, `/permissions x`, `/new`) or holds the busy
flag itself (`/mcp`, `/plugin`) is refused with its text kept and a notice to send it again, so none steers.

`chatSendController.ts` `sendWhileBusy`:
- **Steer now** calls `api.chatSteer` and draws the reply's `steer` (`applyOwnSteer`).
- A refusal is worded by `steerRefusalNotice` (never the code): `STEER_ENDING`, `STEER_NOT_RUNNING` ("hasn't started
  yet") or `STEER_UNSUPPORTED` sends it as Queue for after with its own notice; `STEER_INVALID` and `STEER_NOT_STORED`
  put it back in the box.
- A steer row that wasn't taken (with the agent's `reason` when it gave one, kept through a reload by the stored
  `steerState`) offers **Send as next message** (`sendAsNext`; hidden once it went, back if it didn't) and **Put back
  in the box** (`SteerActions.tsx`).
- **Queue for after** and **Stop and send** call `chat_send` with `after`. A `heldId` reply becomes a chip
  (`addHeldChip`, per conversation key). A reply without one means the turn had already ended, and the message is
  tracked as a normal queued turn.

`chatRuntime.ts` events:
- `steer` draws or moves the row (`notice: 'steer'`, `ChatTurn.steer`); a new row ends the reply's bubble.
- `heldSent` turns the chip into the next turn (user bubble, queued-turn state, busy).
- `heldFailed` puts the words back in the box with a notice.

Chips leave only when the event is not buffered for a background conversation. Restored conversations show a
turn's `steers` before its answer (`chatHydrate.ts`, `components/ConversationHistory.tsx`).

### Preview pane and its web mode

**Card and New tabs (11-15 polish):** `ChatBody` mounts the lazy summary
whenever the pane is closed. It does not take focus or have dismissal state.
Wide chats reserve space beside the transcript; below 900px or 60em the
card shrinks with a matching transcript inset. Sections start collapsed and
save independent choices in `hollow.sideCard.sections.v1`. Recall items disclose
their text and feedback individually. Long sections scroll inside the bounded
card; short content takes only its own height. Both motion preferences
remove its short entrance. Subagents uses the shared live read; workspace file
watch events refresh Outputs. Sources and Recall keep their actions. The
top bar has one pane toggle; with no remembered content the keyboard toggle
opens Tools. Find, Outline, Copy and Save live in the conversation title menu,
in the top bar for workspace chats. The strip's + opens `{ kind: 'newtab', id, chat }`, a document
tab owned by the chat on screen (`DRAFT_TAB_OWNER` before its first send;
dropped with that chat's Subagents tabs, so
New tabs are per chat in memory; web tabs stay app-wide by design). A New
tab's tool replaces it in place (`replacePreviewDocument`, strip and history),
except Browser, which closes it and shows the start page; a pick for a tab
closed or a chat switched meanwhile opens nothing (`newTabStillFor`).

**Mixed tabs (chat-first port):** `PreviewPane` owns the sole tab strip
for documents and web pages. `previewDocuments` keeps at most 12 app-wide
file/visual tabs, independently of the 30-item Back/Forward history. Opening
another citation updates that file's tab; history retains the specific lines.
Closing a document removes its historical occurrences, while Back can restore
a tab evicted at the cap. Conversation changes drop visuals, not file tabs.
Web tabs remain in the existing eight-tab store. Selecting one preserves its
live navigation; explicit link opens and history navigation can load an address.
Arrow/Home/End keys focus tabs; Enter/Space activate them, avoiding expensive
PDF reads while moving across the strip. Close buttons sit outside the ARIA
tablist. The file picker, focus return and native browser occlusion remain.

**One header row (UI clutter pass, 2026-11-06):** the strip shares one
`.preview-pane-bar` with Back/Forward, New tab and close; the shown file's
controls (Files as an icon `Select`, grouped by `src/projectFiles.ts`, Preview/Source, Refresh, the
fix wand, File actions) are portalled by `ChatPreview` into the bar's
`.preview-pane-tools` slot, so a file's content starts under that row. The tabs
and New tab (`.preview-pane-tabset`, + right after the last tab) keep at
least 224px; a narrower pane wraps `.preview-pane-end` (the controls and close)
to a second row, in DOM order. The tab on show gets a `max-content` column (up
to 360px) and scrolls into view. File actions' rows are a memo made again only
for another file; the details row's label is a getter, so a live edit never
drops a keyboard-focused row. There is no kind word and no
path line: `previewDetails.ts` builds each tab's tooltip (kind, and for the
file on show its size and time from `onFacts`, then the full path; a data-URL
image says it came from the conversation), and File actions ends with the
same details and the path as `informational` rows under Copy path. A canvas
tab is named by its canvas's title. A web page's address bar is the second row.

**Review notes (OC9, 2026-11-17):** a plain-text or Markdown file (never an
image, PDF, canvas, web tab, HTML, diagram, table or JSON) shows a **Review
notes** toggle in the header row (`ChatPreview` `reviewable()`; `reviewing`
stays on across files, the Preview and Source buttons and a cited-line open turn
it off). On, the body is `PreviewReviewNotes.tsx` (lazy, with
`styles/review-notes.css`): the first 1500 lines (`MAX_REVIEW_LINES`) with a
number gutter, an "Add a note on line N" button per line (opacity 0 until hover
or focus-within, always shown under `(hover: none), (pointer: coarse)`; one tab
stop, arrows/Page/Home/End move between them), an inline labelled editor
(Ctrl or Cmd+Enter saves, Esc cancels) and each note under its line with
Resolve/Reopen and Delete. `src/reviewNotes.ts` is the pure model: a note is
`{ id, path, line, anchorText, before, after, text, resolved, lost, createdAt,
sentAt? }`; `reanchor` runs whenever the file text changes; `composeBatch` makes
the one message (header naming the file, `Line N: "<line, 120 chars>"` plus the
note, a closing sentence; resolved notes never; 12,000 characters). Storage:
`localStorage` `hollow.reviewNotes.v1`, `{ files: [{ path, updatedAt, notes }] }`,
at most 200 notes a file and 40 files (oldest dropped), parsed tolerantly. The
footer's **Send N notes to chat** calls `ChatPreview.sendNotes`: bring a composer
page on screen (as Ask the agent to fix), `appendToChatDraft` (never sends), a
compact window closes the pane, then the notes are marked `sentAt` and the next
send carries only notes not yet sent. Notes are text nodes, never HTML.

`PdfPreview.tsx` loads lazily in file Preview and Results. `pdf_preview_page`
admits only the main webview and canonical media roots for the supplied
conversation. It renders read-only PNG and bounded page text through PDFKit,
with two concurrent readers, 20 MB files, 2,000 pages and bounded pixels/text.
No PDF scripts, links or attachments execute. Private directories and hidden
filenames are refused, while names like `token-budget.pdf` are allowed.
Page, zoom and scroll positions are bounded to 32 session/path entries.
Refresh and Retry preserve the page; an unavailable later page offers
**Go to first PDF page**. Every page result must match path, session, page and
refresh nonce. Browser and native UI validation are separate from unit gates.
An app-wide PDF tab is reauthorized against the current conversation, like
video; a file available only through another conversation's folder can be
refused after switching chats. Previously requested renders finish behind the
two-reader limit, but their stale results cannot replace the selected page.

Run 15 P1 (2026-10-27) replaced the side panel with one app-wide **Preview
pane** (`components/PreviewPane.tsx`, state and API in `store/preview.ts`).
The shell (`App.tsx`) mounts it inside `.canvas-split`, beside whichever page
is open and under the top bar, outside every Suspense boundary, and only
while `previewTarget()` is set: it is hidden until the person clicks a file or
a link (reply chips and links, tool chips, citations, attachments, `hc-file`,
a canvas's Expand) or an agent calls `navigate` with page `browser`. Nothing
opens it when a turn ends. A target is a file (absolute path or image data
URL, with optional lines: a Run record's Cites and, since S1b on 2026-11-03,
a reply's `path:12` / `path:12-20` / `path#L12` link, `chatFileLinks.ts`
`fileLinkFromHref`), an http(s) address or a conversation visual;
`openPreview` keeps a back/forward history of 30. × / Esc in the pane / ⌘\
(`toggle-side-pane`, `togglePreview`) hide it; ⌘\ reopens the last item. Its
width is `sidePaneWidth`. A file shows by type through `ChatPreview.tsx` and
`previewKinds.ts`; `openFileInSidePane` and `openChatPreview` are thin aliases
of `openPreview`. In a compact or medium window (up to 68.75em) the pane
takes the page's place; side by side, the page keeps at least 360px.

**Web mode** (the browser since 2026-09-30) shows real web pages (at most 8
tabs, one app-wide list; at the cap the tab shown longest ago closes,
`lruVictim`). Web links in replies, unclaimed links in the main
window and a service's **Open :port** open here; ⌘- or Shift-click uses the
default browser. A `file:` address or an absolute path typed in the address
field opens that file in the pane's file mode. Agents open a page with
`hollow_app` `navigate` (page `browser`). Each tab is a native child webview
of `main` (`browser_pane.rs`), so `BrowserPane.tsx` measures placement every
frame and hides the page while anything is drawn over it (the sample grid, or
any menu, listbox, tooltip, dialog or popover overlapping it); notices move
left of a live page when 420px is free there (`TOAST_ROOM`, `data-native-page`
on the root), and otherwise hide it as any layer does. **Loads (run 15 P2):**
WebKit sends a load event only once the server answered and wry has no
failed-load hook, so a page stays hidden behind "Loading…" until its first
event (`committed`); a server on this Mac (localhost, 127.x, [::1], 0.0.0.0)
is probed before a tab opens it ("Nothing is answering at localhost:5173");
any other page Hollow opens or reloads gets `NO_ANSWER_MS` (20 s) before
"Couldn't open" with Try again (a navigation the page starts itself, a link
or a script, sends nothing until its server answers, so it has no limit);
Stop ends a load, and one that never answered says so; that load's start
event, if it was already on its way, never restarts the spinner
(`stoppedGen`). On a tab that showed a page, Back and Forward clear a failed
address's message and end a pending reload's limit (a cached page may send no
event; run 15 FIX-FINAL-P). Each open or
reload is one load with a number: `browser_open` and `browser_go` are async
and return it, every `browser-event` carries it as `gen`, and the store
applies only its tab's current `gen` (an early event of a newer load waits in
`early` for its command's reply). The pure half
is `browserReview.ts` (`tabAfterEvent`, `unanswered`, `stoppedTab`,
`layerCovers`). **Security:** a tab's frames may load only
`http(s)`, `about:`, `data:` and `blob:`, never the app's own protocols or the
dev origin, because Tauri gives app-protocol pages every app command. Mail
and phone links open the default app; another app's link (`vscode:`, an
app's sign-in callback) opens only after a native question naming the exact
address (`Verdict::Ask`); `file:`, scripts and Hollow's own schemes never
leave; at most 3 hand-offs a minute. `Info.plist` lifts App Transport
Security for web view content only (`NSAllowsArbitraryLoadsInWebContent`),
so plain-http sites load. Downloads go to `~/Downloads`; `window.open` opens
a new tab (a scripted sign-in popup therefore gets no `window.opener`: next
run).

**Include page text (repo ideas 2026-10-27 #3).** Annotate's bar has an
**Include page text** checkbox beside Add to chat, off each time Annotate
opens. Ticked, Add to chat first reads the tab's text (`browser_page_text`,
Rust `browser_pane.rs`): the page on show now, main frame only, in WebKit's
own client world (`WKContentWorld.defaultClientWorld`, so the page's scripts
can't patch what the extraction calls; not wry's `eval_with_callback`, which
JSON-serialises any answer with an `unwrap`; Annotate's element inspection
uses the same evaluator), and only a string answer of at most 256 K UTF-16
units comes back. It takes the `innerText` of a
rendered `main`/`[role=main]`/`article` holding 200+ characters, else of the
body (rendered text only: no `display: none` or `visibility: hidden`
elements, form values, scripts or other frames; an unrendered element is
never read, since its `innerText` is its hidden text; tables as
tab-separated rows), cut at 24 KB of UTF-8 (`PAGE_TEXT_BYTES`), named by
the address of the document it read without any user name or password.
A page that doesn't answer in 5 s, or has no text, adds nothing and leaves
Annotate open; so does a switch of conversation or workspace while it is
read (the marks stay). The text joins the marked-up picture in
the composer as a removable chip, "Shared by you from the Preview pane ·
visible content only" over "title · host · size" (`ChatAttachment.page`,
`attachSharedPage`; no file is written). On send it goes to the agent as one
leading `<shared-page>` block (`sharedPage.ts` `sharedPageBlock`): a line
saying it is the person's share and untrusted page content, then the page
as JSON with every `<` escaped, so no page text can close the block. The
backend peels it off the person's words like the page-context block
(`sessions::split_injected_context`; manifest layer `sharedPage`), so
titles, recall, retrieval and learning read only what they typed. Like an
image, it waits for a running turn to end (no Steer now). Nothing gives an
agent a way to read a tab: `browser_page_text` is plumbing, not an action.

**Attached files (C5, 2026-11-04).** Any file attaches to a message by the
composer's + → Attach files… (any type, several at once), a drop or a paste;
pasted text still pastes as text. A PNG, JPEG, GIF or WebP keeps its
thumbnail and its `<attached-images>` line (`isImageFile`). Any other file,
other image types included, is a chip with its name and size
(`ChatAttachment.file` and `size`; Saving… and Failed states; its × is named
"Remove <name>"), saved by `chat_save_attachment` with its `name` into the
chat folder's hidden `.hollow-attachments/` under a generated name that keeps
a cleaned stem and extension (`chat::attachment_name_parts`), with the same
20 MB cap, empty refusal and unsafe-folder refusal as an image. The backend
refuses a payload by its base64 length before decoding and refuses a
`.hollow-attachments` that is a link, and every save keeps a `.gitignore` of
`/*` in the folder (`chat::keep_attachments_out_of_git`), so nothing attached
in a build workspace's worktree is ever committed. Not `*`: that is a tool
cache's sign (`remote_folder::self_ignoring`), and a cache stays on this Mac
when the chat runs on the runner; the runner's snapshot adds the folder by
name instead (`remote_session::snapshot`). A chip shows Saving… and blocks
Send from the moment its read starts (`pendingAttachment`,
`attachmentsHoldSend`), and a stored draft leaves out a chip still saving. A folder, an
empty file or one over 20 MB is refused in words before it is read
(`composerFiles.ts` `attachRefusal`; a drop asks `webkitGetAsEntry` which
item is a folder, `composerDrop.ts` `splitDrop`), and so is a file that
would take the message past 10 attachments or 50 MB (`messageCapRefusal`;
Annotate's picture and page text and Ask to fix's picture are held to it
too: `attachChatImage` is false when the composer refuses, and
`attachSharedPage` when the message is full). A dropped folder's notice
offers Add to Sources… in a workspace chat, and so does the + menu there;
either shows why a pick fails (`addToSourcesNotice`). Names lose controls,
separators and bidi controls on the chip and in the prompt
(`displayFileName`, `promptFileName`). On send each file is one line of a
trailing `<attached-files>` block, its original name then the saved path
(`attachedFilesBlock`), which the backend peels off with the images block
when the block closes the message (`sessions::trailing_attachment_blocks`;
manifest layer `attachedFiles`), so a quoted tag stays in the words. The sent
message shows each file as a chip that opens in the Preview pane (live turns
only, like its images). Like an image, a file waits for a running turn to end
(no Steer now).

**Share with chat (repo ideas 2026-10-27 #5; read only in this run).** The
Preview browser's toolbar has **Share with chat** beside Annotate: it shares
the tab on show with the conversation on screen (`chatAgentSessionKey`, once
settled, on the Conversation page or in an open workspace whose centre shows
its own conversation, `workCenterChat`, false only while the workspace
panel covers it in a narrow desk (overnight 11-04 U4); never a key kept from a chat left for
Home or a workspace's documents; named by its first message;
`browser_grant_tab`, main window only). One tab per chat and one chat per tab (a new share moves it).
The tab then shows an **In use by ‹chat›** strip with Stop
(`browser_revoke_tab`). That chat's agent may read the tab with the
`ui.read_tab` action (Rust `tab_grants.rs`): the same main-frame,
client-world read as Include page text (address, title, readable text,
24 KB), handed back as untrusted page content, plus a numbered **outline**
(CD2, 2026-11-17) beside the text: the visible landmarks, headings (with
level), links (visible text and address: origin and path only, so a token
in the path itself passes), buttons and
labelled form fields, numbered 1 to N in page order, at most 8 KB
(`PAGE_OUTLINE_BYTES`; whole lines, so a kept entry never changes number;
`outline_truncated`). The same script builds it (`OUTLINE_JS`, spliced into
`PAGE_TEXT_JS` only for `ui.read_tab`; Include page text is unchanged and
never carries it) and Rust parses, cleans, numbers and caps it
(`parse_page_read`). A field shows its label and type, never what was typed,
chosen or checked; a password field is no different; a link's query,
fragment and any user name or password are dropped. The numbers describe that
read only: nothing can be clicked, typed or opened by number, and the result's
`note` says so and labels the outline untrusted like the text.
Nothing else of the page (cookies, storage, field contents) leaves it.
A grant belongs to one turn: the chat's turn running when shared, else its
next to start (the run pump holds a `TurnScope` around every chat turn). It
ends when that turn ends however it ends, on Stop, or when the tab closes; a
later turn never inherits it, and the read checks the grant again once the
page is read, so a Stop or re-share mid-read hands nothing over. Nothing
persists across a restart. The strip looks again every 3 s while anything is
shared; a failed look keeps the last strip and Stop, says so and retries.
`ui.inspect_tab` (GU1, 2026-11-20; experimental) is the second read-only tool on the same share: a CSS
selector in, the match count and the first five matches out (box in viewport pixels, visible, covered,
disabled, a few computed styles; no field value), in the same client-world pass style as `ui.read_tab`
(`browser_pane.rs` `SELECTOR_JS`, prototype getters only). Tier Read, `Lease::Outside`, same caller and
host-scope checks. Screenshot, console and wait for agents are not built.
After a successful call the pane marks the first visible, uncovered match with an "Agent inspected" cursor
(parity 5.6): drawn by `browser_pane/agent_cursor.{rs,js}` in WebKit's client world because a parent DOM cursor
would sit behind the native page; it is published only after the final share and host-scope checks, for the
original share and navigation generation, and never adds click, hover, typing or focus. `browser_show` takes an
optional `cursor` presentation argument (duration and colours; none clears it). The header's Mini player and
Expand buttons (`PreviewPlayerControls.tsx`, local state, never persisted) shrink the same mounted pane, so native
tabs and media keep running. See [Preview player](../PREVIEW_PLAYER.md).
Whatever was shared, `ui.read_tab` and `ui.inspect_tab` refuse an automation, a child task
(depth > 0), a Main agent check-in or wake, and a turn a wake queued.
Clicking and typing for agents wait for Alex's word.

**Keyboard annotation (P3-10).** Browser → Annotate has a shared Segmented
tool group, a named focusable stage and one numbered list of editable marks.
Arrows move the crosshair 8 stored snapshot pixels, Shift+arrows 40; Enter
places a Comment or each Box corner. Coordinates clamp to the original
image size, including reduced restored images. Escape cancels a pending box
without closing Annotate; otherwise the existing close/undo/redo guards apply.
Draw remains pointer-only. A polite readout names the crosshair and corner state.
Keyboard focus and arrows keep the cursor point within the capture viewport,
scrolling instantly without moving focus or the surrounding document.
Comment on the whole page adds `{ kind: 'page', note }`, with no coordinates.
Inspection and image/review badges omit page marks, preserving original list
indices for all other marks. Entries offer Enter/click to edit and Delete or
Backspace to remove; Done/Escape return focus to the entry, removal to the next
survivor or stage. Add to chat still only appends a draft and one image.

### Workspace conversation and page context

Queue refreshes adopt a turn only when its source session matches
`chatAgentSessionKey`; events for another conversation are routed to its
background snapshot or ignored. `Cmd+Shift+C` returns to the active
workspace's composer. Workspace and history views both use `ChatBody`; keep
input behaviour there. `pageContext()` (`store/runtime.ts`) returns a
`<page-context>` block only in Conversation history (Library → Conversations,
`conversationHistoryShown()`); every
workspace conversation carries its own workspace context. `ChatBody`'s send
composes only the page context, the intent block, the request and the image
list; the conversation bundle and the About-you profile are the backend's
(`context_compiler.rs`), and the "Preferences" chip only reflects the switch.
Long threads: only the latest `TURN_WINDOW` (80) turns render ("Show earlier"
loads 80 more, scrolling to it by wheel, touch, key or scrollbar loads 20 at a time; find and the
outline cover every turn, `src/conversationTools.ts`). Reading back is bounded at both ends (parity 0.3,
`src/transcriptWindow.ts`, in `chat/chatScrollController.ts`): past `TURN_CAP` (200) drawn turns the
window's end gives way (`renderCeil`, never while the person is at the end), "Show N later messages"
at the foot (or scrolling to it) moves it down and drops the oldest turns off screen, `holdPlace` keeps
the first turn on screen at its offset through every load, a jump (find, outline, canvas reveal)
recentres the window on its target instead of loading every turn between (`windowFor`), a send or
Jump to latest from far back returns to the latest turns (`showLatest`, an instant scroll), and `atBottom`
is false while the end is cut so the pill stays. Scroll loading needs reader input within 1 s
(`readerRecently`): a scroll the app made never loads. The budget (turns drawn, DOM nodes, frames,
heap, open time) is `scripts/perf-budgets.mjs`, measured by `scripts/perf-transcript.mjs`. **New since you
were here** (`chatVisitMark.ts`, `chat/visitMark.ts`, `chat/ThreadVisitWatermark.tsx`): leaving a
conversation records how many turns were seen (`hollow.chatVisits.v1` in localStorage, 200
conversations, never fewer than before; at the end of the thread everything drawn counts, otherwise what
was on screen after a 300 ms pause); opening one with more turns shows a divider before the first new
turn (Dismiss clears it; a first visit shows none). Smoke: `tests/smoke/long-chat.spec.ts`.
The outline (E9, 11-12) is `components/ChatOutlineRail.tsx`, lazy: a strip of
dashes in the chat's left gutter, one per prompt (sampled to 40,
`src/chatOutlineRail.ts`), the prompt in view marked from a passive scroll
listener and a ResizeObserver on the scroller and each rendered turn (a
streaming reply moves the prompts too), measured once per frame (read only: it never writes the chat's
scroll; a dash or a row calls `jumpToTurn`). Hovering or focusing the strip
opens the Outline card beside it; Outline in the conversation title menu
opens the same card, and is the click path when the chat column is
650px or narrower (the strip hides) or in the side panel. Esc closes the card
and gives focus back to the strip or the title menu. The strip is one tab stop
(roving tabindex): the dash Up, Down, Home or End moved to while focus stays in
the strip, else the current prompt's (`railTabStop`, #1322 re-review). Smoke:
`tests/smoke/chat-outline-rail.spec.ts`.
While the person is not at the bottom, the window keeps its start (ChatBody's
`readingPin`, overnight P2), so turns arriving below never drop turns above and
move what they read (WebKit has no scroll anchoring); back at the bottom it
trims to the latest 80 again; setting a floor on purpose (Show earlier, find,
the outline, a canvas reveal) clears the pin, so that floor stays; a
conversation switch while scrolled up re-pins after the swap.
Off-screen canvases are already bounded by the six-frame budget
(`src/canvas/frameBudget.ts`). Smoke: `tests/smoke/long-chat.spec.ts`.
A per-row computation must not scan the whole thread: derive it once in a memo
(`lastUserIndex` for the footer's "latest", `foldRoles` for each row's fold role;
overnight C5), since a scan per row also subscribes that row to the turns it reads.
When the window keeps showing turns under a new key (Continue from summary,
`recoverChatContext`), every send names the earlier keys in `continuedFrom`
(`src/chatContinuedFrom.ts`) so the backend bundles them.

### ActivityDock + trackTask

`trackTask(name, fn, opts?)` wraps any async backend call so:
1. It shows in Work → Running → Background tasks as pending → success/warning/error/cancelled.
2. `opts.onSuccessStatus` lets a result that didn't throw still report as a
   partial failure (e.g. a recording import that wrote 4 of 5 notes to the
   vault), or as `cancelled` (a workflow run the user stopped).
3. Errors re-throw so callers can still react. An expected cancellation
   (`isExpectedCancellation` in toastPolicy.ts: `cancelled before it started`
   from the run queue, or the run permit's `was stopped while waiting for a
   background agent slot`) ends the row as a neutral `cancelled` "Stopped"
   (symbol `·`, muted) instead of an error (Paperclip PR 22). Workflows
   and the Settings routine row say "Stopped." rather than "✓ run complete"
   or a failure for a cancelled run, and the live run log shows it as a
   muted `stopped` row.

**Prefer `trackTask` over bare `invoke`** for any operation that takes more
than a couple hundred ms.

`ActivityDock.tsx` now renders that operation log as a flat section in Running.
It retains details, results, elapsed time and Clear completed. The pending
clock runs only while mounted and the window is present. Queue status and
Stop/Cancel remain in Mission Control's turn list. TopBar keeps the startup
queue refresh, with no second queue list or operation popover.
Running receives workflow activity by exact historyRunId from the launch-owned
runActivity store, including activity-only events after reload. Missing exact
activity does not fall back to another run of the same automation; done clears
the phase and quiet time even before the queue refreshes. Chat feed completion
also clears activity while retaining its text. Sidebar visits request composer
focus only after navigation succeeds and the workspace and conversation match.
They use visitWorkspaceConversation; plain visitWorkspace keeps an explicit
section request, including Back to Results, without taking composer focus.
The old activityDockOpen/openRunningActivity and dockVisible helpers remain
temporarily as inert compatibility/test seams; no product dock uses them.

Full-view toast placement: `ViewToastRegion.tsx` registers a bounded normal-flow
shelf inside Today, Mission Control, Inbox, Commitments and Weekly review
(Ask your vault's view had one until One Ask removed it, 2026-11-03). `toastPlacement.ts` chooses the last connected region in DOM
order (their shared stacking level); both global stacks portal into it and
return to their fixed corners when all views close. The registry publishes only
an actual destination change; opening a view underneath cannot remount the
active portal or release its held countdown. The same notices and
countdowns survive moves; held snackbar timers resume when their destination
changes, so a removed focused/hovered element cannot strand them. The region
stays inside each view's keyboard boundary, including loading/error states.
Today, Commitments and Weekly review scroll their content separately, so a
notice remains reachable while reading lower down. Today and Weekly review
focus that inner region on open for keyboard scrolling, while the focus trap
still covers the whole card including notices.
`styles/view-toasts.css` caps overflow; `toast-placement.spec.ts` covers all six
views, nested order, Undo, countdown recovery, themes and narrow screens.

### Notice lifetime and history

Running and Map scope document-capture keys to their own view, so the bell
keeps history navigation and Enter on disclosures. Opening a failure toast
leaves the notice unread, matching history and Incoming; acknowledgment stays
with the explicit Mark read action.

The top-bar Notifications bell opens a bounded nonmodal history, newest first,
without navigating away from the chat. Escape returns focus to the bell;
outside click or focus closes it. It reads the existing notification resource
on opening, with loading/error/Retry distinct from an empty history. Mark listed
read and Clear listed act only on shown ids; both exclude unread failures.
The bell is the single history owner, including saved filters, repeat folding,
missing-target hints and per-row Delete. Repeats must share body, destination
and read state as well as title/source/kind. Unread failures never fold and
Open leaves them unread; their own Mark read acknowledges them. Details keeps
the full title, body and timestamp, outside the Open button. Keyed days and rows
retain all grouped ids: Mark read, x, Open and Delete act on the whole folded
routine row. Unread failure rows always contain one id. Keyed rows
retain open Details and focus after refresh or acknowledgment. History keeps
j/k, arrows, Enter and x (Mark read), respecting Shortcuts Off. An empty note
index remains unknown, never proof that a target moved. Settings Clear all is
separate: its confirmation deletes every notification, including failures.
App messages retains the bounded session text history. Needs you is the other
tab, selected on open when waiting items, drafts or unavailable coverage exist.
Its first eight waiting rows use `openWaitingItem`; See all opens the full
waiting view and its Incoming tab. Only that full view owns decisions. The
bell owns waiting refresh events and the minute timer formerly in Rail.
Its description separates waiting, unsaved drafts and unread notifications;
overlapping failure counts are never added. Failed/partial coverage shows !
while initial loading says Checking without a failure badge,
and the summary names missing sources. Routine completions no longer toast; failures,
run-guard (not `run-guard-docs`, PER-36), budget and permission/failure-streak notices remain time-critical.
Existing Undo/Retry snackbars and quit actions keep their
owners and lifetime. Native notification preferences are independent.

`store/noticeTimers.ts` owns the single `windowAway` signal re-exported by
runtime. The App focus detector sets it; countdowns must not use document.hidden.
Snackbars and notification toasts bank their remaining time while away or held
by hover/focus (independent reasons). Only the last release resumes them, with
at least three seconds. Caps evict the oldest unheld transient, excluding
leaving rows. If every older row is held, the incoming notice temporarily
overflows the cap; release trims it. Eviction disposes timers; moving a portal clears only
interaction holds and leaves an away hold intact. Stack containers have no live
region: each item uses status or alert once.

Bell → Notifications → App messages retains the newest 50 session-local
messages, failures labelled in words, with full text/Details and Copy. Evicted and
dismissed notices remain there; the history keeps no callbacks or Undo authority.
It clears when the app session ends and does not replace persisted notifications.
The unused footerStatus signal is removed.

Install results use sticky (`ttlMs: 0`) snackbar receipts outside the five-item
transient cap. Reveal log calls the supported host opener in reveal mode and
leaves the receipt visible; successful receipts without a running-app warning
also offer What's new. A nonempty result message keeps alert/error treatment
even when installation succeeded. Dismiss
acknowledges the host result. Pending-install cancellation and guarded quit stay
owned by SelfInstallNotice. Its measured banner height reserves space below
the snackbar stack. The global stack has a viewport-height cap and scrolls
focused actions into view; full-view portals retain their own scrolling shelf
without nesting another scroller. Native overlap/VoiceOver checks remain separate.

`flashWithUndo({ message, kind, undo })` shows a toast with Undo;
`flashFooter(msg)` flashes short status in the bottom-left corner. Anything
with a real outcome goes through `trackTask` instead. **Raw errors in plain
language** (overnight C7): `flashFooter` passes every message through
`plainError` (`src/plainErrors.ts`). A message carrying a raw OS, SQLite,
network, JSON or Tauri error (`No such file or directory (os error 2)`,
`database is locked`, `expected value at line 1 column 1`) keeps its lead-in
and gets one sentence saying what happened and what to do. It shows as a
failure, with the raw text under a "Details" disclosure on the snackbar
(`Snackbar.detail`), and stays at least 7 s. A raw error counts only at the
start of the message or right after a `: ` / ` — ` separator, and only in its
raw shape. So a title the person chose (`Moved "database is locked.md"…`), a
plain "timed out" or "Permission denied by the person" is left alone. Hollow's
own error codes (`TASK_CONFLICT: `, `WRITE_CONFLICT: `…, listed one by one in
`HOLLOW_ERROR_CODES`, so a key the person typed such as `GITHUB_TOKEN: ` stays) at a
segment start are dropped from the text, and the message as it came goes under
Details (night ST10); callers keep matching the code on the raw message
(`isTaskConflict`, `isSourceChanged`). **Words:** `scripts/check-plain-words.mjs`
(in `npm run test:scripts`) fails when a sentence the frontend shows says
"receipt", "stdout"/"stderr", "session id/key", "IPC" or "backend call"; the
app says "run record", "output" / "error output", "the conversation" and
"request". A sentence written for an agent goes in its `ALLOWED` list with the
reason. Inline
error lines use `ErrorText` (`components/ui.tsx`, styled in `styles/ui.css`,
morning M6): the same plain sentence, the raw text behind a "Details" toggle
inside the alert (phrasing content only: a `<details>` can't sit in a `<p>`),
the surface's own class. `PlainErrorText` is the same sentence and Details
without an element of its own (overnight S5), for a line that already is the
alert or has its own lead-in (`Couldn’t load: <PlainErrorText text={errMsg(e)} />`),
a list item or a note; its `children` sit between the sentence and Details. A
failed `StatePanel` takes the error itself: `error={x}` renders it the same
way, then the panel's children, then Details. `ErrorText` and `PlainErrorText` take `plain={false}` to show
text as it is: a warning, and a note that is only sometimes an error, which passes its error
flag (`plain={n().error}`), because `plainError` acts after every `: ` and
would rewrite a title the person chose (`shownError` in `plainErrors.ts`
decides). The Work page's banner (`store/workspaces.ts`, A08) is a
`{text, plain}` signal behind the old names: `workspaceError()` is the text,
`workspaceErrorPlain()` the flag the banner passes as `plain`,
`setWorkspaceError(text)` shows an error (plain words, raw text under Details;
`''` clears), and `setWorkspaceWarning(text)` shows a line as written. Only a
build workspace's `warnings` use the warning setter, after the open (opening a
workspace clears the banner). `plainErrors.test.ts` scans every TSX source and fails on a one-line
`<Show when={x()}><p … role="alert">{x()}</p></Show>`; on a raw error
(`{errMsg(…)}`, `{String(…)}`, `{xError()}`, `{x.error}`,
`{plainError(…).text}`) as the first interpolation in a `role="alert"`
element's text or as a `StatePanel` body; on `{message()}` / `{n().text}` in an
element whose `role` turns to `alert` on failure; and on a `PlainErrorText`
without `plain=` as the first child of such a note or a `role="status"` one. It
reads source shapes, so an error line built another way (no role, a
conditional expression, a `role="status"` line such as `CanvasPinButton`'s pin error) can still slip past. Its `LEFT` exceptions, each of
which must still trip the scan: `components/AppBuildPanel.tsx` (the app build's
failure line is a build log), `components/QuestionCard.tsx` and
`components/WorkFileEditor.tsx` (another lane's files on 10-13). A second
test (`RAW_LINES`, overnight A09) names six files whose error lines had no
alert role, each with the raw shapes it must not go back to: the Markdown
preview window's "Couldn't open this file" line, the receipt's patch error,
the Add sheet's picker error (Quick ingest's until S5 folded that sheet into Add), the Plaud token test (a throw goes in its own
`testError` signal; the ✓ / × outcomes stay verbatim), the Claude accounts
notes, and the skill reader (a failed read is `{ text, error }`, shown as
"Couldn’t read this skill: …" above the instructions, never in the Markdown).
`MdPreview.tsx` imports `./ui` now; on 10-13 that import put the launch `App`
chunk a few hundred bytes over its then 330 KB budget (it is 340 KB since
consolidation D/E), so check `App-*` in `check-bundle-budget` if it moves. Each
recognised kind has a stable `code` (`plainError(msg).code`:
`not-found`, `db-busy`, `signed-out`, …): match on that, never on the
sentence. `errMsg` stays the raw text, so the prefixes callers already match
on keep working.
`src/loadingErrorStates.test.ts` pins the remaining `flashRetry` flows
(merge preview, archive scan, Save diagnostics). The map, Runbooks and
pressure-test screens were cut overnight 11-08 S4.
`src/emptyStates.test.ts` pins the Results and Waiting empty lines that stayed.
A failure the
person can simply try again uses `flashRetry(msg, retry)` (overnight G2;
`retryNotice` in `plainErrors.ts`): the same plain language, held 12 s, with
a **Retry** button that runs `retry` (merge preview, archive scan, Save
diagnostics into the same folder). Backend command
strings aren't rewritten.

Archive: frontend state, chat UI, side panel browser, styling and shortcuts.

---

- `store/pairAddress.ts`: Pair a Mac's typed address (two-Macs M-UI, 2026-11-18): `parsePairAddress` reads an ssh alias, `user@host`, a tailnet or dotted name or IPv4 address with an optional `:port` and refuses URLs, paths, IPv6, loopback, half-written IPv4 and bad ports in plain words; a mirror of Rust `peer_machines::parse_pair_address` (the backend decides), used by `settings/MachinesPanel.tsx` and `mockPeers.ts`. Pure, unit-tested.

### Chat announcements: banner and dock badge (0.4, 2026-11-20)

`components/ThreadNotificationCoordinator.tsx` (lazy, renders nothing, mounted in `App.tsx`)
diffs `sessionStatesNow()` (`store/sessionStates.ts`) and applies the pure rules in
[`threadNotifications.ts`](../../src/threadNotifications.ts): a chat going running or queued to idle is a
**finish**, to `failed` a **failure**, anything to `waiting` **needs you**. The first snapshot announces
nothing. Quiet rules: switch off (Settings → General → **Chat announcements**, `store/threadAnnounce.ts`,
localStorage `hollow.chatAnnounce`, default on); never a subagent (a chat the sessions list has not shown yet waits for the flush, after a re-read, and is dropped if still unknown); never the chat on screen while the
window is focused (`windowAway`); one banner per chat per 20 s. Events inside 1.5 s become one banner, re-checked at the flush (a chat that ran again was a flap)
(`bannerFor`: the chat's title for one, counts for several). The banner is the `announce_chat_native`
command (no notification record; it still honours the macOS system notifications switch,
`notifications::native_enabled`); the **dock badge** is `set_dock_badge` (`badgeCount`: chats waiting on
you or failed, plus finished chats not opened, capped at 99; switch off clears it, opening the chat in a
focused window clears its mark). Both commands are webview-bridge only (not Hollow app tools).
