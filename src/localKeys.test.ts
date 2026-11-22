import { describe, expect, it } from 'vitest';
import { LOCAL_COMMANDS, localUsesOf } from './localKeys';
import { CHORD_KEYS, COMMANDS, RESERVED } from './shortcuts';

describe('local-command keyboard policy (next-level P18)', () => {
  it('no in-surface key takes a default app or system-wide chord, or an Edit/macOS one', () => {
    const app = new Map(COMMANDS.filter(c => c.default).map(c => [c.default, c]));
    for (const c of LOCAL_COMMANDS) {
      const taken = app.get(c.chord);
      // Only the same command for that surface: the browser panel's Back on
      // Hollow's own Back key (P1-47).
      if (taken && c.sameAs === taken.id) expect(c.label.startsWith(taken.label), `${c.label} isn't ${taken.label}`).toBe(true);
      else expect(taken?.label, `${c.label} (${c.chord}) collides with ${taken?.label}`).toBeUndefined();
      // A macOS chord only for its macOS meaning (Save on ⌘S, Find on ⌘F).
      const what = RESERVED[c.chord];
      if (what) expect(c.label.startsWith(what), `${c.label} (${c.chord}) is macOS's ${what}`).toBe(true);
    }
  });
  it('every in-surface chord is a well-formed chord', () => {
    for (const c of LOCAL_COMMANDS) {
      expect(c.chord).toMatch(/^CmdOrCtrl\+(Alt\+)?(Shift\+)?[^+]+$/);
      expect(CHORD_KEYS).toContain(c.chord.split('+').pop());
    }
  });
  it('the browser panel lists its page keys: Back, Forward and Reload (P1-47)', () => {
    const browser = LOCAL_COMMANDS.filter(c => c.surface === 'Browser panel').map(c => [c.label, c.chord]);
    expect(browser).toEqual([
      ['Go to the address bar', 'CmdOrCtrl+L'], ['Back', 'CmdOrCtrl+['], ['Forward', 'CmdOrCtrl+]'], ['Reload', 'CmdOrCtrl+R'],
    ]);
    // Recording Hollow's own Back on ⌘[ doesn't warn about the panel's Back.
    expect(localUsesOf('CmdOrCtrl+[').map(c => c.label)).toEqual(['Back']);
    expect(localUsesOf('CmdOrCtrl+[', 'go-back')).toEqual([]);
    expect(localUsesOf('CmdOrCtrl+[', 'search').map(c => c.label)).toEqual(['Back']);
  });
  it('finds what else answers to a chord', () => {
    expect(localUsesOf('CmdOrCtrl+F').map(c => c.label)).toEqual(['Find in the conversation']);
    // The board editor's keys went with whiteboards (2026-10-07).
    expect(localUsesOf('CmdOrCtrl+D')).toEqual([]);
    expect(localUsesOf('CmdOrCtrl+K')).toEqual([]);
    expect(localUsesOf('')).toEqual([]);
  });
});
