import { describe, expect, it } from 'vitest';
import { COMMANDS } from './shortcuts';
import { MAC_RESERVED } from './macReserved';
import { SCRIPT_CHORD_KEYS, scriptChord, scriptChordProblem, scriptForChord, type ProjectScript } from './projectScripts';

const ev = (key: string, over: Partial<{ metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean }> = {}) =>
  ({ key, code: /^[0-9]$/.test(key) ? `Digit${key}` : `Key${key.toUpperCase()}`, metaKey: true, ctrlKey: false, altKey: false, shiftKey: true, ...over });
const script = (id: string, chord: string | null, name = id): ProjectScript => ({ id, name, command: 'echo hi', chord });

describe('script chords pass the chord guard (4.5, A10)', () => {
  it('refuses anything that is not ⇧⌘ and one letter or digit', () => {
    for (const bad of ['CmdOrCtrl+T', 'CmdOrCtrl+Alt+Shift+T', 'CmdOrCtrl+Alt+T', 'CmdOrCtrl+Shift+TT', 'CmdOrCtrl+Shift+t', 'CmdOrCtrl+Shift+[', 'Shift+T', '']) {
      expect(scriptChordProblem(bad, []), bad).toBeTruthy();
    }
  });

  it('refuses every macOS chord of that shape and every one a Hollow command holds', () => {
    for (const c of Object.keys(MAC_RESERVED).filter(c => /^CmdOrCtrl\+Shift\+[A-Z0-9]$/.test(c))) {
      expect(scriptChordProblem(c, []), c).toMatch(/on a Mac/);
    }
    expect(scriptChordProblem('CmdOrCtrl+Shift+Z', [])).toMatch(/Redo/);
    for (const c of COMMANDS.filter(c => /^CmdOrCtrl\+Shift\+[A-Z0-9]$/.test(c.default))) {
      expect(scriptChordProblem(c.default, []), c.id).toBeTruthy();
    }
  });

  it('refuses another script’s chord and offers a free one', () => {
    expect(scriptChordProblem('CmdOrCtrl+Shift+Y', [script('a', 'CmdOrCtrl+Shift+Y', 'Deploy')])).toMatch(/runs Deploy/);
    const free = SCRIPT_CHORD_KEYS.filter(k => !scriptChordProblem(scriptChord(k), []));
    expect(free.length).toBeGreaterThan(10);
  });

  it('runs the script whose chord a keystroke is, and never one the guard now refuses', () => {
    const free = SCRIPT_CHORD_KEYS.find(k => !scriptChordProblem(scriptChord(k), []))!;
    const a = script('a', scriptChord(free));
    expect(scriptForChord(ev(free), [a])?.id).toBe('a');
    expect(scriptForChord(ev(free, { shiftKey: false }), [a])).toBeNull();
    expect(scriptForChord(ev(free, { altKey: true }), [a])).toBeNull();
    expect(scriptForChord(ev(free, { metaKey: false }), [a])).toBeNull();
    // A script saved with a chord a Hollow command holds today does not run.
    expect(scriptForChord(ev('Z'), [script('b', 'CmdOrCtrl+Shift+Z')])).toBeNull();
  });
});
