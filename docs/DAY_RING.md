# Day ring

The day-ring screen was cut overnight 11-08 S3, with Today and Agenda. Local-day geometry in `src/dayRing.ts` stays. The stylesheet went with the screen. The rest of this file is how the dial worked.

Today (palette **Open Today**, `g d`) and Tasks → Agenda (`g c`) shared a local-day SVG dial.
Meeting arcs, inner dotted focus arcs, due-time ticks and a now hand use existing source data.
Calendar remains read-only; focus blocks remain local. Each mark has a native button in the adjacent
scrolling list: hover/focus reveals the title and time, click in Agenda opens its existing detail.
All-day and date-only items are listed separately; no time is invented. Cancelled meetings stay marked.
Calendar unavailable/disconnected/partial and focus-read failures remain explicit. A Microsoft 365
sign-in that is over (`calendarNeedsSignIn`) says "Microsoft 365 needs you to sign in again" instead of
"Calendar unavailable", and Today's Calendar and the Agenda's status offer **Open Connectors** (overnight H6).

`src/dayRing.ts` projects `src/agenda.ts`'s local-day layout into geometry. A revolution is 23, 24 or
25 elapsed hours. DST days label the changed duration and item times include their zone. Unlike the
vertical timeline's minimum hit height, arcs encode the actual clipped duration of short meetings.
Zero-duration reminders use ticks; malformed timed items remain listed with “Time unavailable”.
Overlaps use up to three lanes per kind; all items remain in the list and shared lanes are disclosed.
The dial component reconciled rows by kind/id to preserve keyboard focus across data refreshes.
Its stylesheet used existing theme tokens. The component was shared by lazy views and added no
new launch read. Today loaded local focus blocks only while its view was open, keyed by day.
The dial is 240px on wide and compact windows; SVG text compensates for the
smaller viewBox scale so the labels retain their normal readable size. Compact
windows stack the complete textual detail/list below it. The list remains
scrollable and every mark retains its keyboard button.

The clock aligns its updates to minute boundaries. The minute clock stops offscreen, in a hidden document or an away window; observers, listeners and timers
are disposed on unmount. A subtle opacity pulse uses the same suspension plus both reduced-motion gates.
The full static information remains when animation is off. Native VoiceOver/WKWebView checks are separate.

Verification: `src/dayRing.test.ts` (geometry, DST, times and coverage), `tests/smoke/day-ring.spec.ts`
(the actual Today/Agenda flows, focus, themes, responsive layout, axe and lifecycle). The lane report in
the outcome workspace holds screenshots and final gate results.
