import { describe, expect, it } from 'vitest';
import { marked } from 'marked';
import { markdownOptions, NOTE_MARKDOWN } from './markdownOptions';
import NOTE_RENDERER from './components/NoteRenderer.tsx?raw';
import MARKDOWN_VIEW from './components/MarkdownView.tsx?raw';
import MD_PREVIEW from './MdPreview.tsx?raw';
import SKILLS from './pages/Skills.tsx?raw';
import REPLY from './components/ReplyMarkdown.tsx?raw';

// One soft line break, as a person types it in a note.
const FIXTURE = 'First line of the paragraph\nsecond line of the same paragraph.\n\nNext paragraph.';

describe('soft line breaks render one way for files (P1-18)', () => {
  it('a file renders the same in the Library reader and in MarkdownView', () => {
    const reader = marked.parse(FIXTURE, NOTE_MARKDOWN) as string;
    const view = marked.parse(FIXTURE, markdownOptions(false)) as string;
    expect(view).toBe(reader);
    expect(reader).not.toContain('<br');
    // Chat keeps a typed line break.
    expect(marked.parse(FIXTURE, markdownOptions(true)) as string).toContain('<br');
  });

  it('the reader, the preview window and Skills use file breaks; chat keeps its own', () => {
    expect(NOTE_RENDERER).toMatch(/marked\.parse\(preprocessed, NOTE_MARKDOWN\)/);
    expect(MARKDOWN_VIEW).toMatch(/marked\.parse\(t, markdownOptions\(props\.breaks \?\? true\)\)/);
    expect(MD_PREVIEW).toMatch(/<MarkdownView [^>]*breaks=\{false\}/);
    expect(SKILLS).toMatch(/<MarkdownView [^>]*breaks=\{false\}/);
    expect(REPLY).not.toMatch(/breaks=\{false\}/);
  });
});
