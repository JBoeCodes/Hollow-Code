import { createSignal } from 'solid-js';
import { LEADER, WHICH_KEY_DELAY_MS, isTypingTarget, leaderMap, leaderStep } from '../keyboardLeader';
import { recordingShortcut, shortcutSettings, shortcutsEnabled } from '../shortcuts';
import { modalOpen } from '../modalGuard';
import { shortcutsOverlayOpen } from './runtime';
import { runGoTarget } from '../goTo';
import { flashFooter } from './runtime';

// The go-to leader's state (five asks A4). `leaderKeydown` runs in the
// window's capture phase (App.tsx), ahead of every page's own keys, so the
// key after `g` is the leader's, never a list's j or x.

/** `g` was pressed and the leader waits for its key. */
export const [leaderPending, setLeaderPending] = createSignal(false);
/** The which-key card is showing (the leader has waited a moment). */
export const [whichKeyOpen, setWhichKeyOpen] = createSignal(false);

let reveal: ReturnType<typeof setTimeout> | undefined;

/** The pointer, a focus move or leaving the window ends a waiting leader,
 *  so its next key is never taken from where you went (App.tsx wires them). */
export function cancelLeaderOnMove() {
  if (leaderPending()) cancelLeader();
}

export function cancelLeader() {
  if (reveal) clearTimeout(reveal);
  reveal = undefined;
  setLeaderPending(false);
  setWhichKeyOpen(false);
}

/** One keydown; whether the leader took it. */
export function leaderKeydown(e: KeyboardEvent): boolean {
  const step = leaderStep(e, {
    pending: leaderPending(),
    enabled: shortcutsEnabled(),
    typing: isTypingTarget(e.target),
    // The ? card isn't a modal, but its keys are its own.
    modal: modalOpen() || shortcutsOverlayOpen(),
    recording: recordingShortcut(),
    keys: leaderMap(shortcutSettings().leader ?? {}),
  });
  if (step.kind === 'ignore') return false;
  // A chord or a field that took over: the leader ends, the key goes on.
  if (step.kind === 'pass') { cancelLeader(); return false; }
  e.preventDefault();
  e.stopPropagation();
  if (step.kind === 'start') {
    setLeaderPending(true);
    // Once the card shows, it waits for a key, Esc or a click (P1-49): it
    // used to vanish after 2 s while it was still being read.
    reveal = setTimeout(() => setWhichKeyOpen(true), WHICH_KEY_DELAY_MS);
    return true;
  }
  cancelLeader();
  if (step.kind === 'run') runGoTarget(step.target);
  else if (step.kind === 'unknown') flashFooter(`${LEADER} then ${step.key} isn’t a shortcut. Press ? for the list.`, 3000);
  return true;
}
