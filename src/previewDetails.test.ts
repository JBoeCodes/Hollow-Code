import { describe, expect, it } from 'vitest';
import { fileDetailsLine, fileTabTitle, fmtSize } from './previewDetails';
import previewPane from './components/PreviewPane.tsx?raw';
import chatPreview from './components/ChatPreview.tsx?raw';

// UI clutter pass (2026-11-06): the Preview pane had four header rows before
// any content, and named the file three or four times. Now one row holds the
// tabs and the file's controls; kind, path, size and time are the tab's
// tooltip and rows of the file's ••• menu.
describe('what a Preview tab says about its item', () => {
  it('names the kind, then the size and time once the file is read', () => {
    expect(fileDetailsLine('markdown')).toBe('Markdown');
    const at = new Date(2026, 10, 6, 15, 6).getTime();
    const line = fileDetailsLine('markdown', { path: '/v/a.md', size: 1004032, modifiedMs: at });
    expect(line.startsWith('Markdown · 980.5 KB · ')).toBe(true);
    expect(line).toMatch(/\b(3|15):06\b/);
    expect(fileDetailsLine('video', { path: '/v/a.mov', size: 12, modifiedMs: null })).toBe('Video · 12 B');
    expect(fmtSize(3 * 1024 * 1024)).toBe('3.0 MB');
  });

  it('puts the full path last, and the size only on the file it belongs to', () => {
    const facts = { path: '/v/out/a.md', size: 2048, modifiedMs: null };
    expect(fileTabTitle('/v/out/a.md', 'markdown', facts)).toBe('Markdown · 2.0 KB\n/v/out/a.md');
    expect(fileTabTitle('/v/out/b.md', 'markdown', facts)).toBe('Markdown\n/v/out/b.md');
    expect(fileTabTitle('data:image/png;base64,AAAA', 'image', null)).toBe('Image from the conversation');
  });
});

describe('the Preview pane header', () => {
  it('has no kind word and no second copy of the file name or path', () => {
    expect(previewPane).not.toContain('KIND_LABEL');
    expect(previewPane).not.toContain('preview-pane-kind');
    expect(chatPreview).not.toContain('chat-preview-meta');
    expect(chatPreview).not.toContain('chat-preview-name');
    expect(chatPreview).not.toContain('chat-preview-path');
  });

  it('keeps the file controls in the tab row, with Copy path and the details one click away', () => {
    expect(previewPane).toContain('<div class="preview-pane-tools" ref={setTools} />');
    expect(previewPane).toContain('<ChatPreview toolbar={tools()} onFacts={setFacts} />');
    expect(chatPreview).toContain('<Portal mount={slot()}>{tools()}</Portal>');
    expect(chatPreview).toContain("{ label: 'Copy path', action: () => void copyPath() },");
    expect(chatPreview).toContain('{ get label() { return fileDetailsLine(kind(), facts()); }, action: () => {}, informational: true, separatorBefore: true },');
    expect(chatPreview).toContain("{ label: path, action: () => {}, informational: true },");
    // Made again only for another file, so a live edit never drops a focused row.
    expect(chatPreview).toContain('<ActionMenu label="File actions" items={fileActions()} />');
    // Unmounted, it clears the size and time it handed up.
    expect(chatPreview).toContain('onCleanup(() => props.onFacts?.(null));');
    // The ⌄ file picker keeps its name and its list.
    expect(chatPreview).toContain('label="Files"');
  });
});
