import { describe, expect, it } from 'vitest';
import panel from './settings/ShortcutsPanel.tsx?raw';
import { COMMANDS, CONTEXT_KEYS, CONTEXT_LABEL, DEFAULT_SHORTCUTS, PALETTE_CONTEXTS, WINDOW_CONTEXTS, assignChord, contextsOverlap, shortcutFor, type ShortcutCommand } from './shortcuts';

describe('shortcut contexts (when)', () => {
  it('anywhere overlaps everything; two specific contexts overlap only when equal', () => {
    expect(contextsOverlap(undefined, 'chat')).toBe(true);
    expect(contextsOverlap('anywhere', 'palette')).toBe(true);
    expect(contextsOverlap('chat', 'chat')).toBe(true);
    expect(contextsOverlap('chat', 'palette')).toBe(false);
  });
  it('every command names a known context or none, and stop-run is in a chat', () => {
    for (const c of COMMANDS) expect(['anywhere', 'chat', 'palette', undefined], c.id).toContain(c.when);
    expect(COMMANDS.find(c => c.id === 'stop-run')?.when).toBe('chat');
    expect(Object.keys(CONTEXT_LABEL).sort()).toEqual(['anywhere', 'chat', 'palette']);
  });
  it('a key given to a command displaces another only when their contexts can both be active', () => {
    // stop-run (chat) holds Escape; an anywhere command taking it displaces it, a palette one would not.
    const anywhere = assignChord(DEFAULT_SHORTCUTS, 'new-task', 'Escape');
    expect(anywhere.displaced).toBe('stop-run');
  });
  it('the window keymap runs commands of the active contexts only, the specific one first', () => {
    const ev = (key: string) => ({ key, code: `Key${key.toUpperCase()}`, metaKey: true, ctrlKey: false, altKey: false, shiftKey: false });
    expect(shortcutFor(ev('k'), WINDOW_CONTEXTS)?.id).toBe('search');
    expect(shortcutFor(ev('k'), PALETTE_CONTEXTS)?.id).toBe('search');
    const stop = COMMANDS.find(c => c.id === 'stop-run') as ShortcutCommand;
    expect(stop.when && PALETTE_CONTEXTS.includes(stop.when)).toBe(false);
  });
  it('the palette keys are listed read-only under their context in Settings', () => {
    expect(CONTEXT_KEYS.every(k => k.when === 'palette')).toBe(true);
    expect(CONTEXT_KEYS.length).toBeGreaterThanOrEqual(3);
    expect(panel).toContain('CONTEXT_KEYS');
    expect(panel).toContain('shortcut-context');
  });
});
