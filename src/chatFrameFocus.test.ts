import { describe, expect, it } from 'vitest';
import { isTranscriptFrameFocus } from './chatFrameFocus';

type Fake = { tagName: string };
/** A scroller holding exactly `inside`. */
const scroller = (...inside: Fake[]) => ({ contains: (n: Fake) => inside.includes(n) });

describe('isTranscriptFrameFocus: focus moving into a transcript frame is the person’s', () => {
  it('a canvas frame inside the transcript counts', () => {
    const frame = { tagName: 'IFRAME' };
    expect(isTranscriptFrameFocus(frame, scroller(frame))).toBe(true);
  });

  it('a frame outside the transcript does not (the side panel’s copy of a canvas)', () => {
    const transcriptFrame = { tagName: 'IFRAME' }, panelFrame = { tagName: 'IFRAME' };
    expect(isTranscriptFrameFocus(panelFrame, scroller(transcriptFrame))).toBe(false);
  });

  it('anything focused that is not a frame does not: the window lost focus to another app', () => {
    const body = { tagName: 'BODY' }, button = { tagName: 'BUTTON' };
    expect(isTranscriptFrameFocus(body, scroller(button))).toBe(false);
    // Inside the transcript but no frame: `focusin` already covers it.
    expect(isTranscriptFrameFocus(button, scroller(button))).toBe(false);
  });

  it('nothing focused, or no transcript mounted, does not', () => {
    const frame = { tagName: 'IFRAME' };
    expect(isTranscriptFrameFocus(null, scroller(frame))).toBe(false);
    expect(isTranscriptFrameFocus(undefined, scroller(frame))).toBe(false);
    expect(isTranscriptFrameFocus(frame, undefined)).toBe(false);
    expect(isTranscriptFrameFocus(frame, null)).toBe(false);
  });
});
