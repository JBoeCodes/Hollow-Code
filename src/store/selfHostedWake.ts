// Self-hosted models Q6: the app's wake state for self-hosted chats
// (`selfHostedWake.ts` holds the rules and words). Eager and small: the
// composer calls `wakeFromComposer` on input and on send, and the usage ring
// reads `selfHostedChatWindow`; the chip itself is a lazy chunk.
import { createSignal } from 'solid-js';
import { api, onSelfHostedChange } from '../api';
import type { SelfHostedEntry } from '../generated/ipc';
import { selfHostedEntryId } from '../selfHostedModels';
import { createWakes, typingWakes } from '../selfHostedWake';
import { chatModel } from './chatRuntime';

/** Every self-hosted entry's wake, shared by every chat on it. */
export const selfHostedWakes = createWakes({
  wake: id => api.wakeSelfHostedModel(id),
  status: id => api.selfHostedWakeStatus(id),
  now: () => Date.now(),
});

const [entries, setEntries] = createSignal<readonly SelfHostedEntry[] | null>(null);
let reading = false;
let readAgain = false;

/** Read the saved entries again (a local file; no model is called). The
 *  chip does when it mounts; a call during a read reads once more after it,
 *  so a change made meanwhile is never missed. */
export function loadSelfHostedEntries(): void {
  if (reading) { readAgain = true; return; }
  reading = true;
  api.selfHostedModels().then(view => setEntries(view.entries), () => {}).finally(() => {
    reading = false;
    if (readAgain) { readAgain = false; loadSelfHostedEntries(); }
  });
}

// Settings saved, switched or removed an entry (the backend forgot its last
// wake): a chip still on screen gets the new name and window, and the
// entry's wake state and 2-minute wait start over.
onSelfHostedChange(id => {
  loadSelfHostedEntries();
  selfHostedWakes.forget(id);
});

/** The open chat's self-hosted entry id, or null. */
export const selfHostedChatEntry = (): string | null => selfHostedEntryId(chatModel());

export function selfHostedEntry(id: string): SelfHostedEntry | null {
  return entries()?.find(e => e.id === id) ?? null;
}

/** The open chat's context window when it runs on a self-hosted entry (its
 *  Settings entry: 131,072 for Qwen), else null. */
export function selfHostedChatWindow(): number | null {
  const id = selfHostedChatEntry();
  return id ? selfHostedEntry(id)?.contextWindow ?? null : null;
}

/** A keystroke (`draft`: the box's text) or a send (no draft) in the open
 *  chat's composer: wake its self-hosted model, at most once per 2 minutes
 *  per entry. Typing a `/` command doesn't. Nothing else calls this, so
 *  opening, focusing, scrolling or reading a chat never wakes a model. */
export function wakeFromComposer(draft?: string): void {
  if (draft != null && !typingWakes(draft)) return;
  const id = selfHostedChatEntry();
  if (id) selfHostedWakes.nudge(id);
}
