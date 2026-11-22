// Computer control in a chat (Cua CU8): whether the chat has used the
// desktop, and the live state its chip and side-panel card show (paused or
// taken over, the newest actions, the latest frame). Nothing is read until
// the chat has made a desktop tool call, so a chat that never uses it costs no
// IPC. One poll serves every consumer; it stops with the last one.
import { createRoot, createSignal, onCleanup, createEffect } from 'solid-js';
import { api } from '../api';
import { chatAgentSessionKey, chatTurns } from './chatRuntime';
import type { ComputerChatView } from '../types';
export { actionWords } from '../computerWords';

/** The chat has made a call to Hollow's desktop tools (the MCP server is
 *  `hollow_computer`; its tools show as `mcp__hollow_computer__<tool>`). */
export const chatUsesComputer = (): boolean =>
  chatTurns.some(t => t.tools.some(u => u.name.includes('hollow_computer')));

const POLL_MS = 2000;

export type ComputerControlAction = 'pause' | 'resume' | 'takeover' | 'handback' | 'stop';

const state = createRoot(() => {
  const [view, setView] = createSignal<ComputerChatView | null>(null);
  const [frame, setFrame] = createSignal<string | null>(null);
  return { view, setView, frame, setFrame };
});

let consumers = 0;
let timer: ReturnType<typeof setInterval> | undefined;
let shownFrame: number | null = null;
let shownFor: string | null = null;

export async function refreshComputerChat(): Promise<void> {
  // Before the chat's first reply it has no key yet; the backend answers an
  // empty state for it.
  const key = chatAgentSessionKey() ?? '';
  // Another chat: nothing of the last one shows while this one loads.
  if (shownFor !== key) { shownFor = key; shownFrame = null; state.setFrame(null); state.setView(null); }
  const v = await api.computerChatState(key).catch(() => null);
  if (key !== (chatAgentSessionKey() ?? '')) return;
  state.setView(v);
  if (v?.frameId != null && v.frameId !== shownFrame) {
    shownFrame = v.frameId;
    const f = await api.computerChatFrame(key, v.frameId).catch(() => null);
    if (key === (chatAgentSessionKey() ?? '')) state.setFrame(f);
  } else if (v?.frameId == null) {
    shownFrame = null; state.setFrame(null);
  }
}

/** Run the poll while `active()` holds, for as long as the caller lives. */
export function useComputerChat(active: () => boolean) {
  createEffect(() => {
    if (!active()) return;
    consumers += 1;
    void refreshComputerChat();
    if (!timer) timer = setInterval(() => { if (!document.hidden) void refreshComputerChat(); }, POLL_MS);
    onCleanup(() => {
      consumers -= 1;
      if (consumers <= 0 && timer) { clearInterval(timer); timer = undefined; consumers = 0; }
    });
  });
  return { view: state.view, frame: state.frame };
}

/** The person's controls on this chat's desktop. */
export async function controlComputer(action: ComputerControlAction): Promise<void> {
  const key = chatAgentSessionKey() ?? '';
  const v = await api.computerChatControl(key, action).catch(() => null);
  if (v && key === (chatAgentSessionKey() ?? '')) state.setView(v);
  if (action === 'stop') { shownFrame = null; state.setFrame(null); }
  void refreshComputerChat();
}
