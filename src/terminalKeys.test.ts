import { describe, expect, it } from 'vitest';
import { keyToBytes, pasteBytes, type KeyFacts } from './terminalKeys';

const key = (k: string, over: Partial<KeyFacts> = {}): KeyFacts => ({ key: k, ctrlKey: false, altKey: false, metaKey: false, shiftKey: false, ...over });
const bytes = (k: string, over: Partial<KeyFacts> = {}, appCursor = false) => keyToBytes(key(k, over), { appCursor });

describe('terminal keys', () => {
  it('types printable characters, including an astral one', () => {
    expect(bytes('a')).toBe('a');
    expect(bytes('A', { shiftKey: true })).toBe('A');
    expect(bytes('é')).toBe('é');
    expect(bytes('\u{1f600}')).toBe('\u{1f600}');
  });

  it('sends the control keys a shell reads', () => {
    expect(bytes('Enter')).toBe('\r');
    expect(bytes('Tab')).toBe('\t');
    // Shift+Tab is how keyboard focus leaves the terminal: never a shell key.
    expect(bytes('Tab', { shiftKey: true })).toBeNull();
    expect(bytes('Escape')).toBe('\x1b');
    expect(bytes('Backspace')).toBe('\x7f');
    expect(bytes('Backspace', { altKey: true })).toBe('\x1b\x7f');
  });

  it('maps Ctrl with a letter to its control code, and the punctuation that has one', () => {
    expect(bytes('c', { ctrlKey: true })).toBe('\x03');
    expect(bytes('D', { ctrlKey: true, shiftKey: true })).toBe('\x04');
    expect(bytes('z', { ctrlKey: true })).toBe('\x1a');
    expect(bytes(' ', { ctrlKey: true })).toBe('\x00');
    expect(bytes('[', { ctrlKey: true })).toBe('\x1b');
    expect(bytes('\\', { ctrlKey: true })).toBe('\x1c');
    expect(bytes('1', { ctrlKey: true })).toBeNull();
  });

  it('sends arrows in the mode the program asked for, with modifiers', () => {
    expect(bytes('ArrowUp')).toBe('\x1b[A');
    expect(bytes('ArrowUp', {}, true)).toBe('\x1bOA');
    expect(bytes('ArrowRight', { ctrlKey: true })).toBe('\x1b[1;5C');
    expect(bytes('ArrowLeft', { shiftKey: true })).toBe('\x1b[1;2D');
    expect(bytes('ArrowLeft', { altKey: true })).toBe('\x1bb');
    expect(bytes('ArrowRight', { altKey: true })).toBe('\x1bf');
    expect(bytes('Home')).toBe('\x1b[H');
    expect(bytes('End', {}, true)).toBe('\x1bOF');
  });

  it('sends editing and function keys', () => {
    expect(bytes('Delete')).toBe('\x1b[3~');
    expect(bytes('PageUp')).toBe('\x1b[5~');
    expect(bytes('PageDown', { ctrlKey: true })).toBe('\x1b[6;5~');
    expect(bytes('F1')).toBe('\x1bOP');
    expect(bytes('F5')).toBe('\x1b[15~');
    expect(bytes('F12', { shiftKey: true })).toBe('\x1b[24;2~');
  });

  it('treats Option as Meta by the physical key, not the character macOS would compose', () => {
    expect(keyToBytes({ ...key('ƒ', { altKey: true }), code: 'KeyF' }, { appCursor: false })).toBe('\x1bf');
    expect(keyToBytes({ ...key('Å', { altKey: true, shiftKey: true }), code: 'KeyA' }, { appCursor: false })).toBe('\x1bA');
    expect(keyToBytes({ ...key('1', { altKey: true }), code: 'Digit1' }, { appCursor: false })).toBe('\x1b1');
  });

  it('leaves every Cmd chord, bare modifier, dead key and IME step alone', () => {
    for (const k of ['c', 'v', 'w', 'j', 'k', 't', '1']) expect(bytes(k, { metaKey: true })).toBeNull();
    for (const k of ['Shift', 'Control', 'Alt', 'Meta', 'CapsLock', 'Dead', 'Process', 'Unidentified']) expect(bytes(k)).toBeNull();
    expect(bytes('ArrowUp', { metaKey: true })).toBeNull();
  });
});

describe('terminal paste', () => {
  it('turns line endings into Enter and passes tabs', () => {
    expect(pasteBytes('a\nb\r\nc\rd\te', false)).toBe('a\rb\rc\rd\te');
  });

  it('wraps a paste in the bracket when the program asked for one', () => {
    expect(pasteBytes('ls', true)).toBe('\x1b[200~ls\x1b[201~');
  });

  it('drops escape and control characters so a paste cannot close the bracket or drive the terminal', () => {
    const evil = 'x\x1b[201~rm -rf ~\x03\x04\x00\x7fy';
    const out = pasteBytes(evil, true);
    expect(out).toBe('\x1b[200~x[201~rm -rf ~y\x1b[201~');
    expect(out.slice(6, -6)).not.toContain('\x1b');
  });
});
