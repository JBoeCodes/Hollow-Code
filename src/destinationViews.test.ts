import { describe, expect, it } from 'vitest';
import APP from './App.tsx?raw';
import TASKS from './pages/Tasks.tsx?raw';
import GALLERY from './components/CanvasGallery.tsx?raw';
import LIBRARY_RESULTS from './components/LibraryResults.tsx?raw';
import WORKFLOWS from './pages/Workflows.tsx?raw';
import REVIEW from './pages/Review.tsx?raw';
import { MODAL_SEL } from './modalGuard';

// Consolidation lane B (2026-10-13): overlays that became views of their
// primary destination stay views. A view is a region in the page, has no
// focus trap or modal role, is mounted by its destination rather than by the
// shell, and is out of MODAL_SEL, so the rest of the app keeps working.
describe('views of a destination are not overlays (consolidation lane B)', () => {
  it('Needs you owns render failures without a new Suspense boundary', () => {
    expect(REVIEW).toMatch(/<OutsideSuspense>\{\(\) => <RecoverableView name="Review" compact suspends=\{false\}><WaitingOnYouPanel\b[^\n]*\/><\/RecoverableView>\}<\/OutsideSuspense>/);
    expect(REVIEW).not.toContain('TriageInbox');
  });
  it('Tasks is one list laid out like the tracker, with no Commitments screen', () => {
    expect(APP).not.toContain('CommitmentLedger');
    expect(MODAL_SEL).not.toContain('.cl-');
    expect(TASKS).toContain('<IssueList');
    expect(TASKS).not.toContain('label="Task layout"');
    expect(TASKS).not.toContain('CommitmentLedger');
    expect(TASKS).not.toContain('CommitmentsPanel');
    expect(TASKS).not.toContain("tasksSection()==='commitments'");
  });

  it('Canvases is a region of Library → Results, linked from workspace Results', () => {
    expect(GALLERY).not.toContain('aria-modal="true"');
    expect(GALLERY).not.toContain('holdFocus');
    expect(GALLERY).toContain('role="region"');
    expect(GALLERY).not.toContain('cg-backdrop');
    expect(APP).not.toContain('CanvasGallery');
    expect(MODAL_SEL).not.toContain('.cg-');
    // Current and older versions stay distinguishable; Add to Notion went with Notion writes (11-10).
    expect(GALLERY).toContain("' (current)'");
    expect(GALLERY).not.toContain('Add to Notion page…');
    expect(LIBRARY_RESULTS).toContain('label="Result type"');
    expect(LIBRARY_RESULTS).toContain("import('./CanvasGallery')");
  });

  it('Automations is the list; Runbooks, Tonight and Last night are gone', () => {
    expect(APP).not.toContain('RunbooksPanel');
    expect(WORKFLOWS).not.toContain('RunbooksPanel');
    expect(WORKFLOWS).not.toContain('label="Automations views"');
    expect(WORKFLOWS).toContain('New automation');
    expect(WORKFLOWS).not.toContain('runbooksQueue');
    expect(APP).not.toContain('runbooksQueue');
  });
});
