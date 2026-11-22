# Preview mini player and inspection cursor

The Preview header's Mini player button changes the existing pane into a small
floating player. Expand restores its normal layout and complete toolbar. Close
and Escape retain the pane's existing close behavior. The content stays mounted
through both layouts, preserving native browser tabs and media playback. Opening
a new target expands it. Cmd+L expands a minimized browser before focusing its
address field. Mini state is local to the mounted pane and is never persisted.

The inspection cursor marks the first visible, uncovered match whose centre is
inside the viewport after an agent's successful `ui.inspect_tab` call. The label
says Agent inspected. This adds no clicking, hovering, typing or focus access.
The existing share, caller and host-scope rules still own access. No provider
name participates in the path.

The native browser owns the short-lived marker coordinates and share identity.
The main window receives a `browser-inspection` invalidation with tab, navigation
generation and sequence, never page text. The optional `cursor` presentation
argument on `browser_show` carries the effective panel duration and Hollow's
primary, surface and ink colours. No cursor argument means clear the marker.
The native renderer rechecks the original share and navigation before drawing.

Each browser tab is a native webview above the parent DOM. The decoration is
therefore rendered in WebKit's isolated client world, in a closed shadow root
with pointer events disabled and no accessible controls. It never dispatches an
input event or calls a page function. URL, viewport and scroll must still match
the inspection. The marker clears on scroll, resize, navigation (including
same-document URL changes), expiry, Stop sharing or hiding the tab. It lasts at
most three seconds; policy refreshes do not renew that lifetime.

Cursor motion uses the shared panel-duration policy. Zero duration, app or
system reduced motion and navigation suppression snap immediately. Forced
colours use system colours without motion. The first marker snaps; subsequent
inspections can move between positions. Mini-player layout changes are immediate
so the native page and the app frame do not animate independently.

Local-server discovery already belongs to BrowserPane: running services have
Open and Stop routes and appear on the browser start page. Expand exposes those
controls while the mini player is open.

Tests: `src/previewCursor.test.ts`, `src/previewCursorScript.test.ts`, native
`browser_pane::agent_cursor::tests`, and end-run smoke coverage in
`tests/smoke/preview-player.spec.ts`. Mock smoke tests cover layout and IPC
placement only; native WebKit painting still needs an installed-app check.
