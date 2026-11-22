import { describe, expect, it } from 'vitest';
import { markdownOutline } from './docOutline';

describe('document outline (next-level P08)', () => {
  it('lists headings and skips code and frontmatter', () => {
    const doc = '---\ntitle: x\n# not a heading\n---\n# Review\n\n## 1. **Summary**\n```md\n## fenced\n```\n### Detail ##\n##### too deep\n';
    expect(markdownOutline(doc).map(h => [h.level, h.text])).toEqual([[1, 'Review'], [2, '1. Summary'], [3, 'Detail']]);
  });
});
