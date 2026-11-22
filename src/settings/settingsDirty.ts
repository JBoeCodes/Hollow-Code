// Settings' unsaved drafts and saved cues (P1-21). A form that keeps a local
// draft until its own Save registers here, so a section switch can ask
// before it unmounts the form and drops the draft. Controls that save as
// you change them say so with `flashSaved`, shown in the section's header.
import { createSignal, getOwner, onCleanup } from 'solid-js';

interface DirtyEntry { id: string; label: string; isDirty: () => boolean }

const [entries, setEntries] = createSignal<readonly DirtyEntry[]>([]);

/** Register a form's unsaved state under `id` (one entry per id; a later
 *  registration replaces an earlier one). Called inside a component, the
 *  entry goes when the component unmounts; the returned function removes
 *  it sooner. */
export function registerDirty(id: string, isDirty: () => boolean, label: string): () => void {
  const entry: DirtyEntry = { id, label, isDirty };
  setEntries(list => [...list.filter(e => e.id !== id), entry]);
  const stop = () => setEntries(list => list.filter(e => e !== entry));
  if (getOwner()) onCleanup(stop);
  return stop;
}

/** The labels of registered forms holding unsaved changes, in registration order. */
export function dirtyLabels(): string[] {
  return entries().filter(e => e.isDirty()).map(e => e.label);
}

const [saved, setSaved] = createSignal('');
let savedTimer: ReturnType<typeof setTimeout> | undefined;

/** Say "Saved" for a control that saves as you change it. */
export function flashSaved(label: string) {
  setSaved('');
  queueMicrotask(() => setSaved(`Saved: ${label}`));
  clearTimeout(savedTimer);
  savedTimer = setTimeout(() => setSaved(''), 2500);
}

/** The current saved cue ('' when none). */
export const savedCue = saved;

/** Drop the cue (a section switch: it belongs to the section it was said in). */
export function clearSavedCue() {
  clearTimeout(savedTimer);
  setSaved('');
}
