// Budgets the perf scripts hold the app to, kept in a file with no imports so a unit test can read
// them next to the constants they bound (`src/transcriptWindow.test.ts`).

/** The ceilings parity 0.3 holds the transcript to (Chromium mock, the 1,000-turn thread; the
 *  baseline that set them is in `profile.md`). `scripts/perf-transcript.test.mjs` pins the shape. */
export const TRANSCRIPT_BUDGET = {
  /** Turns in the DOM at any moment, however far back the person has read (TURN_CAP is 200). */
  maxTurnsInDom: 240,
  /** DOM nodes with the 1,000-turn thread read back to its start (4,726 measured with the cap; 21,132 before it). */
  maxDomNodes: 9000,
  /** Frame p95 and the worst frame in a wheel fling over the whole thread, 600 px a frame, ms (34.4 and
   *  45 measured; the fling pays for each 20-turn load, the cost of not holding 1,000 turns). */
  frameP95Ms: 40,
  frameMaxMs: 60,
  /** Heap growth over Home with the thread read back to its start, MB (21.2 measured; 59.3 before the cap). */
  maxHeapOverHomeMB: 30,
  /** Opening the thread (click to its latest turn), ms. */
  openMs: 200,
};
