import { describe, expect, it } from 'vitest';
import { applyTranscript, beginAnchor, lastKeyOf, endsHold, isRepeat, middleFor, statusTitle, HOLD_MS } from './dictation';

describe('dictated text in a field', () => {
  it('writes at the caret and rewrites only its own middle as the transcript grows', () => {
    let a = beginAnchor('Hello world', 5, 5);
    let r = applyTranscript(a, 'Hello world', 'there')!;
    expect(r.value).toBe('Hello there world');
    expect(r.caret).toBe('Hello there'.length);
    a = r.anchor;
    r = applyTranscript(a, r.value, 'there my friend')!;
    expect(r.value).toBe('Hello there my friend world');
  });

  it('replaces a selection', () => {
    const a = beginAnchor('Send the deck', 9, 13);
    expect(applyTranscript(a, 'Send the deck', 'numbers')!.value).toBe('Send the numbers');
  });

  it('adds no space at the start of an empty box or before punctuation', () => {
    expect(applyTranscript(beginAnchor('', 0, 0), '', 'hi')!.value).toBe('hi');
    expect(middleFor(beginAnchor('Hello.', 5, 5), 'there')).toBe(' there');
    expect(middleFor(beginAnchor('Hello ', 6, 6), 'there')).toBe('there');
  });

  it('stops writing once the person edits around it', () => {
    const a = beginAnchor('abc', 3, 3);
    const r = applyTranscript(a, 'abc', 'one')!;
    expect(applyTranscript(r.anchor, r.value + ' typed', 'one two')).toBeNull();
  });

  it('an empty transcript leaves the field as it was', () => {
    const a = beginAnchor('abc', 1, 1);
    expect(applyTranscript(a, 'abc', '  ')!.value).toBe('abc');
  });
});

describe('hold to talk', () => {
  it('a release after the hold time ends it; a quick tap does not', () => {
    expect(endsHold({ key: 'd' }, 'D', 1000, 1000 + HOLD_MS + 1)).toBe(true);
    expect(endsHold({ key: 'D' }, 'D', 1000, 1000 + HOLD_MS + 1)).toBe(true);
    expect(endsHold({ key: 'd' }, 'D', 1000, 1100)).toBe(false);
    expect(endsHold({ key: 'x' }, 'D', 1000, 5000)).toBe(false);
    expect(endsHold({ key: 'd' }, 'D', null, 5000)).toBe(false);
  });

  it('releasing a modifier ends a hold too (no keyup for the letter while ⌘ is down)', () => {
    expect(endsHold({ key: 'Meta' }, 'D', 1000, 1000 + HOLD_MS + 1)).toBe(true);
    expect(endsHold({ key: 'Shift' }, 'D', 1000, 1100)).toBe(false);
  });

  it('a press right after the last one is the key repeating', () => {
    expect(isRepeat(1000, 1225)).toBe(true);
    expect(isRepeat(1000, 1400)).toBe(false);
    expect(isRepeat(null, 1000)).toBe(false);
  });

  it('another keyboard layout still matches by physical key', () => {
    expect(endsHold({ key: 'в', code: 'KeyD' }, 'D', 0, 1000)).toBe(true);
  });

  it('reads the chord key', () => {
    expect(lastKeyOf('CmdOrCtrl+Shift+D')).toBe('D');
    expect(lastKeyOf('')).toBe('');
  });
});

describe('status words', () => {
  it('says why it is off and never offers a network fallback', () => {
    const off = statusTitle({ state: 'unavailable', locale: 'en-US', reason: 'On-device recognition isn’t available for en-US on this Mac' }, '⌘⇧D');
    expect(off).toContain('On-device');
    expect(statusTitle({ state: 'ready', locale: 'en-US', reason: null }, '⌘⇧D')).toContain('on this Mac');
    expect(statusTitle(null, '')).toBe('Dictate');
  });
});
