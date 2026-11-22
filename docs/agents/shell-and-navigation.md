# Shell, navigation and notifications

Part of the agent manual; [AGENTS.md](../../AGENTS.md) is the entry point.

## Sidebar

- The sidebar holds the primary pages (Projects, Tasks, Library, Automations), then Recent: pinned and the 8 newest projects. Search chats is the top bar's icon (⌘⇧K); New chat is the COO's page; Needs you opens from the bell. See [workspaces](../WORKSPACES.md).
- Only sidebar rows call `visitWorkspaceConversation`; ordinary workspace visits keep requested sections and do not take composer focus. View selection commits after navigation guards accept.
- Drag and drop ([sidebar interactions](../reference/sidebar-interactions.md)): a drop runs the source row's current enabled menu command, never a new operation; a pin protects against settlement. File drops capture the exact row, version and files synchronously, never auto-send, and refuse remote rows.
- Running activity (`historyRunId`) stays launch-owned and never borrows another run's progress after reload.

## Windows and layout

- Main-window lookups use the native window lookup, not Tauri's single-webview wrapper, because browser tabs and canvas export attach child webviews to main. See [Tray and global shortcuts](../reference/architecture.md#tray-and-global-shortcuts).
- Navigation and inline-card guards share lifecycle's live main-window snapshot and `window_state.rs` policies. Only successful explicit hide/show requests stamp the tray flag; focus clears it. Tray repairs compare the version observed before native reads, never an unconditional store.
- Layouts share the root `hollow-window` container: compact at 53.75em, medium through 68.75em, wide above. `windowLayout.ts` reads the same tokens; keep it aligned with CSS. The minimum window is 720x480. See [Window widths](../reference/styling.md#window-widths).
- `PreviewPane` mounts in `.canvas-split` beside the page, only while something is previewed.

## Top bar, palette and build labels

- Native build labels share `BuildIdentity::short_text()` (window title, tray tooltip, first tray row). Keep `tray-copy-build` distinct from the app menu's `copy-build`: Tauri delivers menu events to both handlers.
- Palette section requests publish only after a successful matching workspace open and are consumed once; stale requests never apply.
- Palette previews are lazy, read-only excerpts; closing or changing the query cancels delivery.

## Notifications and Needs you

- Everything waiting on the person (approvals, questions, Results to review, proposals, failures, sync conflicts, unsaved drafts) is in one surface, Needs you, with Waiting and Incoming. Only a person resolves what it lists. There is no Approvals page. See [Waiting on you](../reference/surfaces.md#waiting-on-you-merged-plan-wave-3-decision-7).
- Notification history lives only in the bell. Routine completions stay in history; only failures and time-critical notices toast. Unread failures stay individual and survive bulk actions; Settings Clear all removes everything.
- Notice lifetime and the App messages log: [Notice lifetime](../reference/frontend-state.md#notice-lifetime-and-history). Placement: `ViewToastRegion.tsx` and `toastPlacement.ts`.
- View recovery: [View recovery](../reference/frontend-state.md#view-recovery).

## Preview pane and browser

- Preview has one mixed document/web tab strip: files capped at 12, web tabs at 8, history at 30. PDF pages are read-only native pixels with bounded text. Closing a tab never loads a history URL. See [Preview pane](../reference/frontend-state.md#preview-pane-and-its-web-mode).
- The agent cursor ([Preview player](../PREVIEW_PLAYER.md)) is published only after `ui.inspect_tab`'s final share and host-scope checks, for the original share and navigation generation; it adds no input access.
- Annotation marks use stored snapshot pixels; whole-page comments have no coordinates.

## Quick entry

Quick entry keeps drafts and errors on refocus and owns its pending request across hiding and resummoning; Add keeps unsent text in sessionStorage. See [Quick entry](../reference/product.md#quick-entry).
