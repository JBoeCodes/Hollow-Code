# Empty views

Shared empty, filtered and loading StatePanels use borderless neutral surfaces.
Increase Contrast and forced colours restore their visible border.
EmptyState stays centred and borderless. Both action groups wrap, including
long labels; StatePanel's title uses the same leading as EmptyState. Attention
states keep their borders and tone: failed/conflicted remain alerts, and
offline/partial/blocked/stale keep their existing recovery presentation.
This is styling only; readiness, artwork eligibility and actions are unchanged.

The eleven sketches share an angular sunken plate, fine material edge and
static architectural stipple. The shared data-texture-active policy selects
dotted scene strokes; solid accents retain the scene's doorway or action motif.
Plain strokes remain when texture is off or accessibility vetoes it.
Each rendered SVG stays under 2 KB, including that common plate; no
external image, new scene loader or eager asset is added. The 160×112 and
compact 80×56 footprints stay fixed. Existing instruction lines and actions
remain beside the illustration, never embedded in it. What's new reuses the
shared ThresholdArt and field engine for its 2026-10-27 cover inside the lazy
reader, behind a local lazy/error boundary and a reserved slot. Plain cover
strokes do not scale below 1.5px. The historical 2026-10-26 cover stays. Filtering hides both decorations
so results keep the reading space. Neither makes an installed-build claim.

The Inbox, Commitments, Mission Control idle view, palette search and Waiting on you
share architectural line-art around the doorway. `src/components/EmptyArt.tsx` is a small lazy loader with its own
Suspense and error boundary; `EmptyArtDrawing.tsx` and `styles/empty-art.css` load only for an eligible empty view.
An unavailable artwork chunk never hides the surrounding text or actions. Reserved dimensions match the loaded SVG.
`src/emptyScene.ts` gates decoration on complete reads; filtered emptiness uses the neutral lens.

The shared `EmptyState` accepts optional `art: {kind, evidence}`. Its stricter
`emptyStateScene` gate suppresses filtered artwork and keeps the original icon,
copy and actions. The sketch uses the compact 80×56 slot; Tasks trims its
decorated state's vertical padding to keep its title in view at 720px/Largest
text. Existing direct EmptyArt callers retain the neutral lens.
Tasks' “Nothing here” gets a checklist sketch only when the entire task list
is empty, task and mirror reads are ready, and no sync/record/filter error is
present. Search, project, saved filters and non-default scopes suppress it.
Automations' detailed “No automations yet” gets a small linked-process sketch
only after a ready, unfiltered empty list. Its list placeholder stays unchanged.
Loading and retained failed refreshes never qualify as confirmed emptiness.
The workflow API returns only an array and does not expose per-file omissions;
this decoration follows that existing API's coverage, not a filesystem audit.

The Preview pane's canvas view uses the static visuals drawing and one line
when its canvas belongs to a conversation that isn't open now (the Files and
Visuals shelves were removed with the side panel in run 15 P1). A history load
suppresses that artwork until settled.

The drawings are decorative and add no focus stops. Existing copy and actions
carry the meaning. Token strokes adapt in light/dark/glass. The former drift
and breathe animations, IntersectionObserver, body MutationObserver and
visibility listener are removed. App/system Reduce Motion, Reduce Transparency,
Increase Contrast and forced colours select plain strokes; forced colours use
system ink/paper. No clock, observer, network request, polling or external asset
is added.

Coverage remains visible: Inbox and Waiting on you say which portion loaded rather than claiming all clear;
Commitments distinguishes a failed list or proposal read; Mission Control tracks queue read readiness/failure instead
of treating the initial empty array as idle. Results errors retain their retry state. Sources with omissions or
unmatched manifest entries do not get empty-source artwork. Sources' Markdown view retains its attachment controls.
Sources' Visual view uses Add source, including when empty. A changed-only filter
with no matches says no listed originals changed or became unavailable; it never
claims the workspace has no sources or hides omissions/read errors.

Mock study: `?mock=1&window=smoke-artifact&empty-art=1` (excluded from production). Tests: `src/emptyScene.test.ts` and
`tests/smoke/empty-art.spec.ts` and `tests/smoke/empty-state-pages.spec.ts`. Browser themes and lifecycle checks do not establish native vibrancy or VoiceOver.
