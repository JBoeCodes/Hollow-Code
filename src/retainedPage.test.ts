import { describe, expect, it } from 'vitest';
import { retainedOffscreen } from './retainedPage';
import app from './App.tsx?raw';
import chatBody from './components/ChatBody.tsx?raw';
import shellCss from './styles/shell-workspace.css?raw';

// Overnight 11-04 U4: the Work page kept mounted behind the other pages. (The
// workspace panel beside the conversation went with COO rebuild L4.)
describe('the retained Work page', () => {
  it('is offscreen inside a hidden page or an inert (covered) conversation, and when missing or detached', () => {
    // The suite runs in node: a composer whose ancestors carry the given
    // attributes, answering closest() as the DOM would for these selectors.
    const asked: string[] = [];
    const composer = (ancestors: string[][], isConnected = true) => ({
      isConnected,
      closest: (selector: string) => {
        asked.push(selector);
        const wanted = selector.split(',').map(s => s.trim().replace(/^\[|\]$/g, ''));
        return ancestors.find(attrs => attrs.some(a => wanted.includes(a))) ?? null;
      },
    }) as unknown as Element;
    expect(retainedOffscreen(composer([['class'], []]))).toBe(false);
    expect(retainedOffscreen(composer([[], ['hidden']]))).toBe(true);
    expect(retainedOffscreen(composer([['inert'], []]))).toBe(true);
    expect(asked.every(s => s === '[hidden], [inert]')).toBe(true);
    expect(retainedOffscreen(composer([[]], false))).toBe(true);
    expect(retainedOffscreen(null)).toBe(true);
    expect(retainedOffscreen(undefined)).toBe(true);
  });

  it('Work is mounted once opened and hidden by its wrapper, recovery included', () => {
    expect(app).toContain('<OnceOpened when={activePage() === \'today\'}><div class="page-retained" hidden={activePage() !== \'today\'}><RecoverableView name="Work" suspends={false}><Work /></RecoverableView></div></OnceOpened>');
    expect(app).not.toContain('<Show when={activePage() === \'today\'}><RecoverableView name="Work"');
    // The wrapper has no box of its own, so the page lays out as before.
    expect(shellCss).toContain('.page-retained { display: contents; }');
    expect(shellCss).toContain('.page-retained[hidden] { display: none; }');
  });

  it('takes no focus and no Escape from the page on screen', () => {
    expect(chatBody).toContain("import { retainedOffscreen } from '../retainedPage';");
    expect(chatBody).toContain("import { fieldElement, isRichField, type ComposerField } from '../composerField';");
    expect(chatBody).toContain('queueMicrotask(() => { if (textareaEl && !retainedOffscreen(fieldElement(textareaEl))) textareaEl.focus(); });');
    // A lazy rich editor may finish mounting after its retained page is hidden.
    expect(chatBody).toContain('if (textareaEl !== field || retainedOffscreen(fieldElement(field))) return;');
    // The Escape check comes before any stop decision.
    const esc = chatBody.indexOf('if (retainedOffscreen(conversation)) return;');
    expect(esc).toBeGreaterThan(0);
    expect(esc).toBeLessThan(chatBody.indexOf('if (!escStopsRun(e, {'));
    // ⌘F already asks for a displayed scroller (a covered or hidden chat has none).
    expect(chatBody).toContain('if (modalOpen() || !el || !el.offsetParent || chatTurns.length === 0) return;');
  });
});
