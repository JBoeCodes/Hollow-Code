import { describe, expect, it } from 'vitest';
import { registerRichField, richFieldAt, visibleRichField, type RichComposerField } from './composerField';
import editor from './editor/MarkdownEditor.tsx?raw';
import body from './components/ChatBody.tsx?raw';

describe('rich composer ownership', () => {
  it('finds only the current registered field and excludes retained hidden views', () => {
    let hidden = false;
    const child = {} as Element;
    const element = { isConnected: true, contains: (el: Element) => el === child, closest: () => hidden ? {} : null, getClientRects: () => [1] } as unknown as HTMLElement;
    const old = { element } as RichComposerField;
    const unregisterOld = registerRichField(old);
    const current = { element } as RichComposerField;
    const unregister = registerRichField(current);
    unregisterOld();
    expect(richFieldAt(child)).toBe(current);
    expect(visibleRichField()).toBe(current);
    hidden = true;
    expect(visibleRichField()).toBeNull();
    unregister();
    expect(richFieldAt(element)).toBeNull();
  });
  it('resets the editor state on a conversation change and keeps notes-only behavior scoped', () => {
    expect(editor).toContain('const reset = key !== docKey');
    expect(editor).toContain('if (reset) props.composer.onIdentityChange()');
    expect(editor).toContain('syncComposerDocument(view, doc, reset, extensions)');
    expect(editor).toContain('if (!props.composer) view.contentDOM.blur()');
    expect(editor).toContain('...(props.composer ? [] : [hideFrontmatter])');
    expect(body).toContain("lazy(() => import('../editor/MarkdownEditor')");
    expect(body).toContain('get key() { return chatConversationVersion(); }');
    expect(body).toContain('dictationSent(textareaEl)');
  });
});
