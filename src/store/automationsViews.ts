// Automations is one list (overnight 11-08 S4). Runbooks, Tonight and Last
// night were views of this page; those screens are gone. Automations stay.
//
// Kept free of component imports so a caller can open the page without
// pulling in a lazy chunk.
import { createComputed, createRoot, createSignal } from 'solid-js';
import { activePage } from './runtime';
import { goToPage } from './workspaces';
import { guardNavigation } from './navigationGuard';

export type AutomationsView = 'automations';

const [view, setView] = createSignal<AutomationsView>('automations');
export const automationsView = view;

/** Open Automations. */
export function openAutomationsView(next: AutomationsView = 'automations') {
  if (activePage() === 'workflows' && view() === next) return;
  return guardNavigation(() => {
    setView(next);
    if (activePage() !== 'workflows') goToPage('workflows');
  });
}

/** Leaving Automations: the next visit starts at the automations list. */
export function resetAutomationsView() { setView('automations'); setMainActionsRevealRequest(0); }

// "How background work runs" (plan completion L2): the explainer on
// Automations, opened from elsewhere (the Main agent's side band) as well as
// by its own disclosure.
const [backgroundOpen, setBackgroundOpen] = createSignal(false);
export const backgroundWorkOpen = backgroundOpen;
export const setBackgroundWorkOpen = setBackgroundOpen;

/** Go to Automations with the background-work explainer open. */
export function openBackgroundWork() {
  setBackgroundOpen(true);
  openAutomationsView('automations');
}

// Main's action evidence has one home on Automations, with no additional read.
const [mainActionsOpen, setActionsOpen] = createSignal(false);
const [mainActionsRevealRequest, setMainActionsRevealRequest] = createSignal(0);
export { mainActionsOpen, mainActionsRevealRequest };
let revealSequence = 0;
let disposeReveal: (() => void) | undefined;
export function setMainActionsOpen(open: boolean) {
  setActionsOpen(open);
  if (!open) setMainActionsRevealRequest(0);
}
/** A lazy destination consumes only the accepted visit it was asked to reveal. */
export function consumeMainActionsReveal(request: number) {
  if (!request || request !== mainActionsRevealRequest() || activePage() !== 'workflows'
    || view() !== 'automations' || !mainActionsOpen()) return false;
  setMainActionsRevealRequest(0);
  return true;
}
export function openMainAgentActions() {
  return guardNavigation(() => {
    setMainActionsOpen(true);
    setView('automations');
    if (activePage() !== 'workflows') goToPage('workflows');
    disposeReveal?.();
    setMainActionsRevealRequest(++revealSequence);
    // Own cancellation before either lazy destination chunk has mounted.
    // Like Waiting's item request, this observer lives only while pending.
    disposeReveal = createRoot(dispose => {
      createComputed(() => {
        if (!mainActionsRevealRequest()) { dispose(); return; }
        if (activePage() !== 'workflows') { setMainActionsRevealRequest(0); dispose(); }
      });
      return dispose;
    }, null);
  });
}

// Automation windows and the run queue (Settings PR 4, overnight 11-12): they
// moved from Settings, Background work to a fold beside the schedule dial on
// Automations. Opened from Settings' link row, an old `Automation windows` or
// `Run queue` deep link, and the background-work explainer's window jobs row.
const [windowsFoldOpen, setWindowsFoldOpen] = createSignal(false);
const [windowsFoldRequest, setWindowsFoldRequest] = createSignal<{ tab: 'windows' | 'queue'; nonce: number } | null>(null);
export const automationWindowsOpen = windowsFoldOpen;
export const setAutomationWindowsOpen = setWindowsFoldOpen;
/** The fold scrolls to the part a request names, then clears it. */
export const automationWindowsRequest = windowsFoldRequest;
export const clearAutomationWindowsRequest = () => setWindowsFoldRequest(null);

/** Go to Automations with the windows and run queue fold open on `tab`. */
export function openAutomationWindows(tab: 'windows' | 'queue' = 'windows', onOpen?: () => void) {
  return guardNavigation(() => {
    onOpen?.();
    setView('automations');
    setWindowsFoldOpen(true);
    setWindowsFoldRequest({ tab, nonce: Date.now() });
    if (activePage() !== 'workflows') goToPage('workflows');
  });
}
