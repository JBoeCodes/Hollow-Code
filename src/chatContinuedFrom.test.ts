import { describe, expect, it } from 'vitest';
import { MAX_CONTINUED_FROM, continuedFromAfter } from './chatContinuedFrom';
import chatRuntimeSource from './store/chatRuntime.ts?raw';
// The send moved out of ChatBody.tsx (plan L3 X17): the composer's dispatch.
import chatSendSource from './components/chat/chatSendController.ts?raw';
import apiSource from './api.ts?raw';

/**
 * Agent effectiveness F4 (K1 review). The backend's context compiler builds
 * the conversation bundle from the session store, for the turn's own session
 * key. Two composer paths start a new key while the window still shows an
 * earlier conversation: Sessions -> Continue from summary, and the reset when
 * the working folder vanished (`recoverChatContext`). Before K1 the composer
 * bundled the turns it showed; now the send names the earlier conversations
 * (`continuedFrom`) and the compiler bundles their recorded turns first.
 */
const flat = (text: string) => text.replace(/\s+/g, '');
function body(source: string, signature: string): string {
  const text = flat(source);
  const start = text.indexOf(flat(signature));
  expect(start, signature).toBeGreaterThan(-1);
  let depth = 0;
  for (let i = text.indexOf('{', start); i < text.length; i += 1) {
    if (text[i] === '{') depth += 1;
    if (text[i] === '}') { depth -= 1; if (depth === 0) return text.slice(start, i + 1); }
  }
  throw new Error(`unbalanced ${signature}`);
}

describe('the conversations a new one continues from', () => {
  it('appends the left key once, oldest first, keeping the newest few', () => {
    expect(continuedFromAfter([], null)).toEqual([]);
    expect(continuedFromAfter([], 's-a')).toEqual(['s-a']);
    expect(continuedFromAfter(['s-a'], 's-b')).toEqual(['s-a', 's-b']);
    expect(continuedFromAfter(['s-a', 's-b'], 's-a')).toEqual(['s-b', 's-a']);
    const many = Array.from({ length: MAX_CONTINUED_FROM + 3 }, (_, i) => `s-${i}`);
    const kept = many.reduce<string[]>((list, key) => continuedFromAfter(list, key), []);
    expect(kept).toHaveLength(MAX_CONTINUED_FROM);
    expect(kept.at(-1)).toBe(`s-${MAX_CONTINUED_FROM + 2}`);
  });

  it('the folder-vanished reset keeps the left conversation as a source', () => {
    const recover = body(chatRuntimeSource, 'export function recoverChatContext(previousCwd: string)');
    const seed = recover.indexOf('continuedFromAfter(');
    expect(seed).toBeGreaterThan(-1);
    expect(seed).toBeLessThan(recover.indexOf('setChatAgentSessionKeyPersistent(null)'));
  });

  it('a new chat forgets the sources, and a backgrounded conversation keeps its own', () => {
    expect(body(chatRuntimeSource, 'export function resetChatState()')).toContain('setChatContinuedFrom([])');
    expect(body(chatRuntimeSource, 'export function backgroundRunningChat()')).toContain('continuedFrom:[...chatContinuedFrom()]');
    expect(body(chatRuntimeSource, 'export function restoreBackgroundChat(key: string)')).toContain('setChatContinuedFrom(s.continuedFrom');
  });

  it('every composer send names them', () => {
    expect(flat(chatSendSource)).toContain('chatUltracode(),chatCanvases(),chatContinuedFrom(),turn.canned??null,null,chatEffort())');
    expect(flat(apiSource)).toContain("continuedFrom:continuedFrom?.length?[...continuedFrom]:null");
  });
});
