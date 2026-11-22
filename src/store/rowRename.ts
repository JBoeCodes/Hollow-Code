// The sidebar row being renamed in place (3.6): one at a time, by chat key or project id.
import { createSignal } from 'solid-js';

export const [renamingKey, setRenamingKey] = createSignal<string | null>(null);
