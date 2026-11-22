import { describe, expect, it } from 'vitest';
import { chipFolders, chipLabel, chipName, fileKind } from './fileChip';

describe('file chips (2026-11-10)', () => {
  it('gives each common type its own icon', () => {
    const icon = (p: string) => fileKind(p).icon;
    expect(icon('outputs/IT-REQUEST.md')).toBe('markdown');
    expect(icon('data/rows.csv')).toBe('fileSpreadsheet');
    expect(icon('Budget.XLSX')).toBe('fileSpreadsheet');
    expect(icon('memo.docx')).toBe('fileType');
    expect(icon('deck.pptx')).toBe('presentation');
    expect(icon('report.pdf')).toBe('filePdf');
    expect(icon('site/index.html')).toBe('globe');
    expect(icon('config.json')).toBe('fileJson');
    expect(icon('notes.txt')).toBe('fileText');
    for (const p of ['a.png', 'b.jpg', 'c.svg', 'd.gif']) expect(icon(p)).toBe('fileImage');
    for (const p of ['src/a.ts', 'b.js', 'c.py', 'd.rs', 'e.sh']) expect(icon(p)).toBe('fileCode');
    expect(icon('outputs/drafts/')).toBe('folder');
    expect(icon('LICENSE')).toBe('file');
    expect(icon('blob.weird')).toBe('file');
    // A cited line doesn't hide the type.
    expect(icon('src/app.ts:42')).toBe('fileCode');
    expect(icon('README.md#L3-L9')).toBe('markdown');
  });

  it('shows the name, never the path, with cited lines apart', () => {
    expect(chipName('workspaces/notion-390b/outputs/location-manager/email-intake/IT-REQUEST.md')).toEqual({ name: 'IT-REQUEST.md', lines: '' });
    expect(chipName('/Users/me/vault/report.pdf')).toEqual({ name: 'report.pdf', lines: '' });
    expect(chipName('src/app.ts:42')).toEqual({ name: 'app.ts', lines: ':42' });
    expect(chipName('src/a.ts:10-20')).toEqual({ name: 'a.ts', lines: ':10-20' });
    expect(chipName('README.md#L3-L9')).toEqual({ name: 'README.md', lines: ':3-9' });
    expect(chipName('outputs/drafts/')).toEqual({ name: 'drafts', lines: '' });
    expect(chipName('PLAN.md')).toEqual({ name: 'PLAN.md', lines: '' });
  });

  it('names a folder only when two files in a reply share a name', () => {
    expect(chipFolders(['outputs/a/IT-REQUEST.md', 'outputs/b.csv'])).toEqual(['', '']);
    // The same file twice is one file.
    expect(chipFolders(['outputs/x.md', 'outputs/x.md'])).toEqual(['', '']);
    expect(chipFolders(['outputs/email-intake/IT-REQUEST.md', 'outputs/phone/IT-REQUEST.md', 'z.md']))
      .toEqual(['email-intake', 'phone', '']);
    // The fewest folders that tell them apart, without case.
    expect(chipFolders(['a/outputs/PLAN.md', 'b/outputs/plan.md'])).toEqual(['a/outputs', 'b/outputs']);
    // A bare name beside a path: the path shows its folder, the bare one has none.
    expect(chipFolders(['PLAN.md', 'workspaces/main/PLAN.md'])).toEqual(['', 'main']);
  });

  it('labels a chip with what a click does, its name and its path', () => {
    expect(chipLabel('outputs/email-intake/IT-REQUEST.md')).toBe('Open IT-REQUEST.md, outputs/email-intake/IT-REQUEST.md');
    expect(chipLabel('PLAN.md')).toBe('Open PLAN.md');
    expect(chipLabel('src/app.ts:42')).toBe('Open app.ts:42, src/app.ts:42');
  });
});
