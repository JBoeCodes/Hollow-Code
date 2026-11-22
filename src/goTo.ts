// What each go-to target does (five asks A4): the leader (`g` + key) and the
// welcome tour call this. Every target is a shared action (actionDefs.ts
// `go`), so a `g` key runs the same dispatcher as its palette row, ⌘ key and
// menu item (actionRun.ts `runGo`); the leader adds a way in, not a page.
// The dispatcher loads on first use, keeping the action table out of the
// launch bundle (plan completion L4).
import { GO_TARGET_BY_ID } from './keyboardLeader';

/** Run go-to target `id`; false when there is no such target. */
export function runGoTarget(id: string): boolean {
  if (!GO_TARGET_BY_ID[id]) return false;
  void import('./actionRun').then(m => m.runGo(id));
  return true;
}
