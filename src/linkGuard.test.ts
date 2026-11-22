import { describe, expect, it } from 'vitest';
import { FrameOpenGate, linkKind, linkToTakeOver } from './linkGuard';

/** A click whose target sits inside `<a href={href}>` (or no link at all). */
const click = (href: string | null, over: Partial<{ defaultPrevented: boolean; button: number; editable: boolean }> = {}) => ({
  defaultPrevented: over.defaultPrevented ?? false,
  button: over.button ?? 0,
  target: {
    closest: (sel: string) => {
      const found = { getAttribute: (n: string) => (n === 'href' ? href : null) };
      if (sel === 'a[href]') return href !== null ? found : null;
      if (sel.startsWith('[contenteditable]')) return over.editable ? found : null;
      return null;
    },
  } as unknown as EventTarget,
});

describe('main-window link guard', () => {
  it('classifies web and mail links and nothing else', () => {
    expect(linkKind('https://example.com/a')).toBe('web');
    expect(linkKind('HTTP://localhost:5173')).toBe('web');
    expect(linkKind('mailto:a@b.c')).toBe('mail');
    for (const h of ['#section', '/notes/a.md', 'notes/a.md', 'javascript:alert(1)', 'file:///etc/passwd', 'obsidian://open', '', null, undefined]) {
      expect(linkKind(h), String(h)).toBeNull();
    }
  });

  it('takes over a plain click on a web or mail link', () => {
    expect(linkToTakeOver(click('https://example.com/a'))).toEqual({ href: 'https://example.com/a', kind: 'web' });
    expect(linkToTakeOver(click(' mailto:a@b.c '))).toEqual({ href: 'mailto:a@b.c', kind: 'mail' });
  });

  it('leaves a click another handler took, a non-primary button, an editor, and non-links alone', () => {
    expect(linkToTakeOver(click('https://example.com', { defaultPrevented: true }))).toBeNull();
    expect(linkToTakeOver(click('https://example.com', { button: 1 }))).toBeNull();
    expect(linkToTakeOver(click(null))).toBeNull();
    expect(linkToTakeOver(click('#top'))).toBeNull();
    expect(linkToTakeOver(click('https://example.com', { editable: true })), 'a link inside an editor').toBeNull();
    expect(linkToTakeOver({ defaultPrevented: false, button: 0, target: null })).toBeNull();
    // A text node target has no closest().
    expect(linkToTakeOver({ defaultPrevented: false, button: 0, target: {} as EventTarget })).toBeNull();
  });
});

describe('artifact frame links', () => {
  it('opens only while the sending frame has focus, at most three a minute', () => {
    // Review finding: an Interactive artifact script posting from a timer
    // must not open a page per tick, including while the user types in the
    // composer (which activates the main window but moves focus off the frame).
    const gate = new FrameOpenGate();
    expect(gate.admit(0, false, true), 'main is active (typing in the composer), frame not focused').toBe(false);
    expect(gate.admit(0, false, undefined), 'frame not focused, no userActivation API').toBe(false);
    expect(gate.admit(0, true, false), 'frame focused, no user activation').toBe(false);
    expect(gate.admit(1, true, true)).toBe(true);
    expect(gate.admit(2, true, undefined), 'WebView without userActivation').toBe(true);
    expect(gate.admit(3, true, true)).toBe(true);
    expect(gate.admit(4, true, true), 'fourth in a minute').toBe(false);
    expect(gate.admit(59_999, true, true)).toBe(false);
    expect(gate.admit(60_002, true, true), 'the first open has aged out').toBe(true);
  });
});
