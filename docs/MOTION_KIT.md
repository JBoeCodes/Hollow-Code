# Motion kit

The kit adds measured dimensions and operation cues without introducing data or navigation state. Existing controls own disclosure, search, loading and activity. `src/components/MotionKit.tsx` owns reusable presentation; `src/motionKit.ts` owns DOM lifecycle; `src/styles/motion-kit.css` owns layered appearance.

Initial measurements snap. Subsequent height/width changes rebase from the current displayed size and cancel stale completion callbacks. Clipping lasts only through a size transition, with a 2px focus margin. Native details own disclosure visibility, keyboard behavior and focus; the wrapper animates their measured outer size without delaying accessibility changes. Reopening cancels exit. Unmount releases observers and animations.

The shared panel-duration policy controls geometry and enables cues. Zero duration, application or system reduced motion, and navigation suppression produce immediate final states. Repeating decorative cues run only while visible. Text, operation labels, focus and static loading shapes remain available. Forced colours suppress overlays.

Reference values: height 200ms, width 150ms, skeleton opacity 2400ms with stepped ramps, tool/activity shine 2200ms steps(30), search inset ring 650ms twice, progress scale .04 to .9 over 5300ms, drag label reveal 200ms. Shared palette and easing tokens come from the base layer. No branded artwork is copied.

Callers (parity 6.1, 2026-11-20). The working beam, agent state marks and mosaic image load are held by Alex (4:03 PM) on branch `held/libraries-dev-effects` and are not on main:

- Callers added: running tool names use `MotionCue kind="tool"`, tool call bodies sit in `AnimatedHeight`, and the Preview pane's load state shows `LoadingProgress`.

Integration: Settings help disclosures and search target, Library loading rows, rail search loading. Transcript, composer and shelf reordering remain their lanes' callers.

The stylesheet is registered in the global manifest so existing rail drop labels
work before any lazy view opens; the component and DOM helper remain lazy.
`SkeletonRows detail` matches Library All's title/metadata line heights, icon
lane, padding and row rule. Compact callers can omit `detail`.

| Reference family | Hollow use |
|---|---|
| Measured height/width | Native Settings More/Less help; Library All count |
| Skeleton rows | Library All initial load |
| Running-tool shine | Settings tool-path read status; reusable `MotionCue kind="tool"` for transcript callers |
| Activity focus | Library's partial-load status, stationary opaque text above a decorative band |
| Settings target pulse | Existing search landing, preserving its static mark and focus |
| Loading progress | Rail Search chats; indeterminate, no fabricated percentage |
| Drag-label reveal | Existing pinned reorder and conversation-to-project drop classes; menu actions unchanged |
| Stash pulses | No matching shipped stash control in this lane; composer ownership remains with its lane |
