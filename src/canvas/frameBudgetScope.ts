import { createContext } from 'solid-js';

/** A transcript's frame budget scope (Canvas C3): the transcript provides
 *  one, and every canvas or html frame inside it shares its budget
 *  (`frameBudget.ts`, which loads with the frame, so the App chunk carries
 *  only this). A frame with no scope (the side panel, the smoke harness)
 *  is always live. */
export interface FrameBudgetScope {
  /** The transcript's scroller, which visibility is measured against. */
  root: () => Element | null | undefined;
  /** Frames under this scope are outside the transcript `of` but share its
   *  budget, each counted as on screen (`SlotOptions.pinned`; Canvas C6 ·
   *  K2 made it for the curated section views, gone since COO rebuild L4). */
  of?: FrameBudgetScope;
}

export const FrameBudgetContext = createContext<FrameBudgetScope | undefined>(undefined);
