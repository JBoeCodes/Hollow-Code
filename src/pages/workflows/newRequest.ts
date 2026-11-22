import { createSignal } from 'solid-js';

/** Destination-local request, published only inside accepted navigation. */
export const [newAutomationWindow, setNewAutomationWindow] = createSignal<string | null>(null);
