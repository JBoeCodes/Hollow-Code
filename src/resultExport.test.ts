import { describe, expect, it } from 'vitest';
import { canExportMarkdown, exportFileName, exportedText } from './resultExport';
import RESULTS from './components/ResultReviewBar.tsx?raw';

describe('export a Result as Markdown', () => {
  it('offers text Results only', () => {
    for (const name of ['outputs/plan.md', 'outputs/a.MARKDOWN', 'outputs/t.csv', 'outputs/canvases/x.canvas.html', 'outputs/n.txt']) {
      expect(canExportMarkdown(name), name).toBe(true);
    }
    for (const name of ['outputs/deck.pdf', 'outputs/chart.png', 'outputs/book.xlsx', 'outputs/README']) {
      expect(canExportMarkdown(name), name).toBe(false);
    }
  });

  it('names the file from the title, safely', () => {
    expect(exportFileName('Q3 plan: draft/v2')).toBe('Q3 plan- draft-v2.md');
    expect(exportFileName('  ')).toBe('Result.md');
    expect(exportFileName('.hidden notes')).toBe('hidden notes.md');
    expect(exportFileName(`${'a'.repeat(79)} b`)).toBe(`${'a'.repeat(79)}.md`);
    expect(exportFileName('x'.repeat(200))).toHaveLength(83);
  });

  it('says where it went and that nothing was sent', () => {
    expect(exportedText('/Users/x/Q3.md', 3)).toBe('Saved as Markdown to /Users/x/Q3.md, with 3 sources listed. Nothing was sent.');
    expect(exportedText('/p.md', 0)).toBe('Saved as Markdown to /p.md. Nothing was sent.');
  });

  it('is in the review bar’s More menu, for text Results only (COO rebuild L4)', () => {
    expect(RESULTS).toContain("...(canExportMarkdown(r().name) ? [{ label: 'Export as Markdown…', action: () => void exportResultMarkdown(props.workspaceId, r().path, r().title) }] : []),");
  });
});
