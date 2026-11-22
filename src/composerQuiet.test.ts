import { describe, expect, it } from 'vitest';
import { busyHint, busyPlaceholder, stopTitle } from './chatSteer';
import { permissionsTriggerLabel } from './permModes';
import { chatUltracodeStorageKey, parseStoredUltracode, ultracodeOnKeyChange } from './ultracode';
import chatRuntimeSource from './store/chatRuntime.ts?raw';
import waitingSource from './components/WaitingOnYouPanel.tsx?raw';
import apiSource from './api.ts?raw';
import chatBodySource from './components/ChatBody.tsx?raw';
// The permissions button folded into the model menu (2026-11-01).
import modelMenuSource from './components/ChatModelMenu.tsx?raw';

const flat = (text: string) => text.replace(/\s+/g, '');

// P1-10: a quieter composer, the fixes only (D13's Agent menu fold is not built).
describe('Ultracode belongs to the conversation (P1-10)', () => {
  it('is kept under the conversation key and read back like Canvases', () => {
    expect(chatUltracodeStorageKey('s-20261024-aaaa0001')).toBe('hollow.ultracode.s-20261024-aaaa0001');
    expect(parseStoredUltracode('true')).toBe(true);
    expect(parseStoredUltracode(null)).toBeNull();
    expect(parseStoredUltracode('false')).toBeNull();
    // On in chat A, then New chat: off (the saved effort shows).
    expect(ultracodeOnKeyChange('A', null, true, true, false).on).toBe(false);
    // Reopening A reads A's own stored value back.
    expect(ultracodeOnKeyChange(null, 'A', false, true, false).on).toBe(true);
    // A new chat switched on before its first send keeps it under its new key.
    expect(ultracodeOnKeyChange(null, 'B', true, false, true)).toEqual({ on: true, persist: true });
    // A forward to the chain's successor on its own turn keeps it.
    expect(ultracodeOnKeyChange('A', 'B', true, false, true)).toEqual({ on: true, persist: true });
    // Same conversation: unchanged.
    expect(ultracodeOnKeyChange('A', 'A', true, false, false)).toEqual({ on: true, persist: false });
  });
  it('never moves from an unsent new chat into a conversation opened from it (review M1)', () => {
    // New chat, Ultracode on, nothing sent, then open chat B: B's own value.
    expect(ultracodeOnKeyChange(null, 'B', true, false, false)).toEqual({ on: false, persist: false });
    // Only the conversation's own turn carries it to the minted key.
    const text = flat(chatRuntimeSource);
    expect(text).toContain('setChatAgentSessionKeyPersistent(accepted.sessionKey,{ownTurn:true})');
    for (const open of ['setChatAgentSessionKeyPersistent(s.key)', 'setChatAgentSessionKeyPersistent(row.sourceId)', 'setChatAgentSessionKeyPersistent(null)'])
      expect(text).toContain(open);
  });
  it('is no longer one global flag that follows every new chat', () => {
    const text = flat(chatRuntimeSource);
    expect(text).not.toContain("lsGet('chatUltracode')");
    expect(text).toContain('setChatUltracode(ultracode.on)');
    expect(text).toContain('chatUltracodeStorageKey(');
  });
  it('resumes each interrupted chat with its own value', () => {
    expect(flat(waitingSource)).toContain('ultracode:storedChatUltracode(key)');
    expect(flat(waitingSource)).toContain('effort:storedChatEffort(key)');
    expect(flat(waitingSource)).not.toContain('chatUltracode()');
    expect(flat(apiSource)).toContain('resumeInterruptedChats:(chats:{sessionKey:string;canvases:string|null;ultracode:boolean;effort:string|null}[])=>');
  });
});

describe('Stop names its key only when it has one (P1-10)', () => {
  it('builds the title from the stop-run chord', () => {
    expect(stopTitle('Esc')).toBe('Stop (Esc)');
    expect(stopTitle('')).toBe('Stop');
    expect(flat(chatBodySource)).toContain('title={stopTitle(shownStopKey())}');
    expect(chatBodySource).not.toContain('title="Stop (Esc)"');
  });
});

describe('the permissions trigger says what it is (P1-10)', () => {
  it('reads "Permissions: <mode>", with (unrestricted) in bypass', () => {
    expect(permissionsTriggerLabel('Bypass all (danger)', true)).toBe('Permissions: Bypass all (unrestricted)');
    expect(permissionsTriggerLabel('Edits only', false)).toBe('Permissions: Edits only');
    expect(modelMenuSource).not.toContain('aria-label="Chat settings"');
  });
  it('lives in the model button’s name and its Permissions row (2026-11-01)', () => {
    const text = flat(modelMenuSource);
    expect(text).toContain('aria-label={`Model:${AGENT_LABELS[chatAgent()]}·${chatModel()||\'default\'}·${permissionsTriggerLabel(');
    expect(text).toContain('aria-label={`${permissionsTriggerLabel(permLabels()[permMode()],permDanger())}.Change`}');
    expect(text).toContain('confirmPermissionModeChange(next,chatAgent())');
    // The composer bar has no separate permissions button any more.
    expect(chatBodySource).not.toContain('ChatSettingsMenu');
  });
});

describe('the busy hint is the placeholder while the box is empty (P1-10)', () => {
  it('names Enter and the Stop key in one line', () => {
    expect(busyPlaceholder('steer', 'Claude', true, 'Enter', 'Esc')).toBe('Enter steers Claude at its next step · Esc stops this turn');
    expect(busyPlaceholder('queue', 'Codex', false, '⌘Enter', '')).toBe('⌘Enter queues it for after this turn · Stop ends this turn');
  });
  it('shows the hint line only once something is typed', () => {
    const text = flat(chatBodySource);
    expect(text).toContain('<Showwhen={chatBusy()&&composerHasMessage()}><divclass="chat-busy-hint"');
    expect(text).toContain('busyPlaceholder(');
    // The line itself is unchanged for a typed message.
    expect(busyHint('queue', 'Claude', '')).toContain('Enter queues it for after this turn');
    // Esc doesn't stop while the box holds text (FIX-ESC), so the typed line
    // names the stop key only when it is a ⌘ chord.
    expect(text).toContain("constshownStopKey=()=>(activeChord('stop-run')==='Escape'&&composerHasMessage()?'':keyFor('stop-run'));");
    expect(text).toContain("enterSends()?'Enter':'⌘Enter',shownStopKey())}</div>");
    expect(text).toContain('composerEmpty:!composerHasMessage()');
  });
});
