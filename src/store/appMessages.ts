import { createSignal } from 'solid-js';

export interface AppMessage {
  id: number;
  message: string;
  detail?: string;
  failed: boolean;
  at: number;
}
let nextId = 0;
// Session-local text only: no retained callbacks, undo authority or IPC payloads.
export const [appMessages, setAppMessages] = createSignal<AppMessage[]>([]);
export function recordAppMessage(message: string, failed: boolean, detail?: string) {
  const entry = { id: ++nextId, message, detail, failed, at: Date.now() };
  setAppMessages(previous => [entry, ...previous].slice(0, 50));
}
