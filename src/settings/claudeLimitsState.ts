/**
 * Settings, Usage, Limits (Settings PR 8): the Claude accounts view the
 * Claude meters and Spread usage share. A plain signal loaded when the tab
 * opens, never a resource, so an update can't suspend the Settings page.
 */
import { createSignal } from 'solid-js';
import { api } from '../api';
import { flashFooter } from '../store';
import type { ClaudeAccountsView } from '../types';
import type { LoginPin } from '../claudeAccounts';
import { loginOverrideLine, loginPinFrom } from '../connectorsLibrary';
import { errMsg } from '../utils';

export type ClaudeLimitsState = ReturnType<typeof createClaudeLimitsState>;

// The accounts view and its saves, for the module (overnight 11-13 S3 and
// the review of #1369): leaving Limits for another Usage tab and back
// remounts the panel. Per mount, busy came back false while a save from the
// last mount still ran, and that save's result went to the unmounted copy
// while the new one showed what it loaded before the save landed.
const [view, setView] = createSignal<ClaudeAccountsView | null>(null);
const [error, setError] = createSignal('');
const [savesRunning, setSavesRunning] = createSignal(0);
const running = new Set<Promise<boolean>>();
/** How long a mount's first read waits for running saves before reading anyway. */
export const SAVE_WAIT_MS = 15_000;

/** Save a checkbox's change; when the save fails, the box shows the saved
 *  value again (overnight 11-13 S3: In rotation and Resume on another
 *  account stayed flipped, since the view didn't change and Solid doesn't
 *  rewrite a binding whose value is the same). */
export async function saveCheckbox(box: { checked: boolean }, saved: () => boolean, save: () => Promise<boolean>): Promise<boolean> {
  const ok = await save();
  if (!ok) box.checked = saved();
  return ok;
}

export function createClaudeLimitsState() {
  const busy = () => savesRunning() > 0;
  const [pin, setPin] = createSignal<LoginPin | null>(null);
  const [loginLine, setLoginLine] = createSignal<string | null>(null);

  function run(action: () => Promise<ClaudeAccountsView>, failure?: string): Promise<boolean> {
    const done = save(action, failure);
    running.add(done);
    void done.finally(() => running.delete(done));
    return done;
  }

  async function save(action: () => Promise<ClaudeAccountsView>, failure?: string): Promise<boolean> {
    setSavesRunning(n => n + 1);
    try {
      setView(await action());
      setError('');
      return true;
    } catch (e) {
      const message = errMsg(e);
      if (failure) flashFooter(`× ${failure}: ${message}`, 5000);
      else setError(message);
      return false;
    } finally {
      setSavesRunning(n => n - 1);
    }
  }

  function load() {
    // A save still running from an earlier mount answers with the newer
    // view: read only once it settles, so the read can't be older than it.
    // Bounded: a save that never settles must not keep the read waiting.
    setError('');
    const settled = Promise.allSettled([...running]);
    void Promise.race([settled, new Promise(r => setTimeout(r, SAVE_WAIT_MS))]).then(() => run(() => api.claudeAccounts()));
    // Login override (2026-10-09) and Spread usage across accounts (2026-10-21):
    // best effort, no line when the connector library can't load.
    api.listConnectors(false).then(
      v => { setLoginLine(loginOverrideLine(v.connectors)); setPin(loginPinFrom(v)); },
      () => { setLoginLine(null); setPin(null); },
    );
  }

  return { view, error, busy, pin, loginLine, run, load };
}
