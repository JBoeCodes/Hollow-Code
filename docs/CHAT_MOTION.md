# Chat motion

The live footer uses a provider-neutral SVG doorway/orbit beside its existing activity words. Queued is static.
Finished-run disclosures retain all narration, tool details, grouping, Raw view and failure words; their existing
tool-kind icons now sit on a vertical timeline with a transform-only reveal. The reveal moves the decorative
line, never text or canvases, so canvas anchors retain their geometry.

`src/components/ChatMotion.tsx` supplies the shared visibility lifecycle and canvas preparation surface;
`src/styles/chat-motion.css` uses theme tokens. Every instance observes intersection, document visibility and
Hollow's away class, releases observers on removal and remains static when observation is unavailable. System
Reduce Motion and Settings → General → Motion both leave the final content visible. There is no frame loop.

Canvas drawing keeps the root's reserved height; artifact preparation reuses its authored or last reported frame
height. Paused frames retain their existing placeholder and Show action. Loading does not read a pending resource
or suspend the transcript. The shimmer reports no percentage or invented progress. A frame's actual reported
height may differ from its authored reservation; this is not a claim that arbitrary canvas layout never changes.

Verification: `tests/smoke/chat-motion.spec.ts` covers real chat events, tool disclosure, preparation height and
motion gates. Keep `canvas-focus.spec.ts` and long-chat/windowing assertions intact. Browser checks are mock
Chromium evidence, not installed WKWebView or VoiceOver verification.

Follow requests coalesce to one animation frame, retaining the strongest pending request. A newer human scroll
cancels an ordinary/send follow; pending blocking question/approval reveals retain their existing forced behavior.
Unmount cancels the frame. The deterministic browser test controls a pending frame and late layout notification:
old runtime gap661, repaired at bottom. This proves the coalescing defect in the browser, not that it was the sole
cause of the earlier intermittent gap1042. Existing #549/A1/P2 tests remain unchanged.

The overnight rule intentionally pauses this working mark while Hollow is away, overriding the older generic
live-work animation exception. Activity words remain visible. Shared observers stop with the last mounted mark.
Kit import and artifact-storage failures already resolve to light-DOM/offline fallbacks; V8 preserves that policy,
with a loading-placeholder regression. Missing ask-kit controls under import failure are an existing limitation.

A delayed programmatic scroll event at an unchanged top no longer clears following after layout growth. A pending
follow also tolerates its observed clamp, including a short tail moving the view up by under 80px (`FOLLOW_CLAMP_PX`,
the drop, not the position: it was "within the first 80px" until a narrow window's tail sat at 85 and its clamp, 85→49,
released the follow, 2026-11-07) as the reply remounts
(browser traces: desktop7→0, narrow47→11, subsequent growth367/396px). Physical scroll intent or transcript focus
(focus moving into a transcript frame included, H10) cancels the pending follow; larger unannounced movement still
releases it. Deferred Find/reveal navigation uses an intent version so a newer send wins, while newer reader navigation wins over an older send. This is browser
ordering evidence; native assistive cursor-only movement without focus remains unverified.

A layout clamp at the end is not the reader leaving it (chat jump, 2026-11-10). WebKit clamps `scrollTop` when the
transcript is shorter for a moment inside one task: a finished run folding its steps into "Worked for…" while the
composer drops its busy row (a 203px drop that left the view 78px short), or every reply swapping its loading view on
the first open (5,250 to 8,700px short). No script writes that move, so a write probe sees nothing; a MutationObserver
reading `scrollTop` at each batch shows it inside the batch. `onScroll` read the move up as the reader's, so nothing
followed again and each later turn (a check-in) left the view a reply further up, under "Finished". Now an upward move
while following, not a jump, reveal or held canvas, goes back to the end when `layoutMove` holds: the transcript's DOM
changed since the last frame (a MutationObserver sets a flag that the next animation frame clears; a frame's scroll
events come before its animation frames, so the clamp's event always sees its change, however late the frame), no
reader input in the transcript for `READER_INPUT_MS` (any press, key, wheel, touch or focus moving in), no press held,
and neither the pointer nor focus on a canvas frame (Chromium's frames send the parent only a `pointerout` with no
`relatedTarget`, so its point inside the scroller's box decides; focus counts only while the window has it). The
observer never reads layout: a first cut read `scrollTop` at each mutation batch to catch the clamp exactly, and that
read itself clamped a send mid-update (34px short). A move up in a quiet chat (an assistive scroll) stays the reader's;
one during a streaming reply, in a frame that also flushed text and with no input event, is taken for a clamp. With the
pointer or focus left on a canvas frame the rule stands aside (the old behaviour). Specs that model the person's scroll
as a bare `scrollTop` write send a wheel first. `tests/smoke/chat-jump.spec.ts` reproduces the
clamp by hand in Chromium (it doesn't flush layout mid-task the way WebKit does) and fails without this.

## Async conversation loading

A reading pin is also evaluated when session loading finishes, even if the person
remained above the bottom through the loading boundary. This keeps the rendered
window from dropping old turns when background messages arrive. Explicit Show earlier
and navigation floors remain intact; only returning to the bottom releases a reading pin.


## Reading through late canvas growth

`src/chatReadingAnchor.ts` holds the viewport offset of a visible paragraph, code block,
list item, image, control or canvas while reading above the end. Visible children
win over tall list ancestors. It observes rendered turns, so a frame
growing earlier in the same reply is covered too. Growth below the reference does not
move the view. Native browser anchoring can act first; Hollow corrects only the remaining
viewport displacement. A canvas remains an opaque box, with no access to its document.

Wheel, touch, scroll keys, scrollbar and focus navigation (a click into a canvas
frame too, H10) drop the old reference; the next changed-position scroll captures
the new place. Delayed notifications at an
unchanged scrollTop retain the pre-growth reference (WebKit can deliver these after
new layout); actual native anchoring and input still move or reset it. A pending reset never replaces an intervening
scroll's reference. Disabled correction still samples geometry, so resuming reading
starts with a reference. Replaced Markdown blocks reuse a unique tag/text prefix (at least16 characters) only
inside the same surviving turn; ambiguous or removed content is recaptured.
Find/outline navigation suppresses corrections through its scroll animation (120ms
quiet, 700ms fallback). Find, reveal, sends, opening/restoration, smooth
latest jumps and held-canvas focus retain priority. Conversation changes reset the
reference; unmount disconnects both observers and cancels the single pending frame.
No continuous animation or new shortcut is introduced. Cold scroll-memory restoration
still starts from the recorded pixel position; it is not a persisted text anchor. Bare inline text without its own block falls back to its ancestor;
height changes inside that ancestor cannot preserve an exact text line.

A long message's Show more and Show less (overnight 11-04, `toggleInPlace` in
`components/chat/chatScrollController.ts`) change height around the reference.
Either press is the reader's own action, as a scroll is (a press on a button isn't
one to `onScrollIntent`): a pending follow, a held canvas and any deferred jump let
go. Show more also takes the place again once the words have opened (the reference
held when the button was pressed can be that button, under the words, and holding it
would scroll the words just shown off the top), and when the growth leaves the end
the view stops following, so Jump to latest shows and the next turn doesn't pull the
reader down. A growth that stays inside the 80 px near-bottom band (a message just
past the fold, at small text or in a very wide window) leaves the view following a
streaming reply, so those few opened lines can still move with it.
Show less scrolls by however far its button moved, so the button stays where it was,
under the pointer and in view with keyboard focus; a key's or a pointer's reset leaves
no hold to rely on there. Smoke: `tests/smoke/long-user-message.spec.ts`.

Coming back to a workspace conversation from another page is a show, not a mount
(overnight 11-04 U4 keeps it mounted and hidden), so the controller watches its
scroller with a ResizeObserver instead (`chatScrollController.ts`). Where the reader
was is taken as it hides, since a scroll event as it hides or shows can move it;
while hidden its scroll events are ignored (its offsets are zeros). Shown again at
the end, it goes back to the end and settles there while the frames draw, as a mount
does; for the first second, focus the page moves in and the frames' own script
re-anchoring as they unpark don't end that settle, while the reader's wheel, keys or
press still do, and so does a scroll with the pointer over a canvas frame (it chains
out of the frame, so no wheel reaches the transcript), once the pointer has really
moved over the window since the show: its screen position differs from the last move
the window saw or, when the window may have missed that one (the pointer went into a
frame or out of the window, a press that can drag a scrollbar, or the window lost
focus), movementX or movementY says so.
WebKit's still pointer's synthetic move after a scroll comes at the pointer's real spot
with no movement, so it never counts, and moves inside a frame never reach the window.
A frame hovered under a pointer that hasn't
moved is the pointer resting where it was: frames unpark and grow after a show, so a
frame comes to sit under a still pointer, and until 11-04 its own re-anchor then ended
the settle short of the end (back through a narrow window's Preview pane close, or
a shortcut with the mouse mid-window). Recording which frame sat under the pointer at
the show didn't hold: at that moment the point was still message text. The trade-off:
a wheel over a frame the pointer already rested on, in that second, with no move over
the window first, is put back at the end. Shown again further up, it
keeps the reader's place. A held canvas, a
busy turn (the follow has it) or a loading conversation are left alone.

`tests/smoke/canvas-open-settle.spec.ts` measures content position, not raw scrollTop:
compensation necessarily changes scrollTop when earlier content grows. Native WKWebView,
VoiceOver and trackpad inertia remain separate hand checks.
