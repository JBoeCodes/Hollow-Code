import { describe, expect, it } from 'vitest';
import { CAPTURE_LABEL, classifyFile, classifyText, ingestSourceFor, isWebAddress, noteName } from './captureClassify';

describe('unified capture (next-level P12)', () => {
  it('routes text predictably', () => {
    expect(classifyText('Call Dana tomorrow about the budget').kind).toBe('task');
    expect(classifyText('Book the board room p2').kind).toBe('task');
    expect(classifyText('Garage 14 gate').kind).toBe('task');
    // A web address is a Library source, the kind Import a source opens on (S5).
    expect(classifyText('https://example.com/report').kind).toBe('source');
    expect(classifyText('https://web.plaud.ai/file/abc123').kind).toBe('recording');
    expect(classifyText('Notes from the site visit\nGate arm sticks after rain\nTwo lights out').kind).toBe('note');
    expect(classifyText('x'.repeat(200)).kind).toBe('note');
  });
  it('routes every file to a workspace source, exports included', () => {
    // Conversation exports went to From other apps until run 15 P4 removed
    // it; a picked export is now a source snapshot like any file.
    expect(classifyFile('conversations.json').kind).toBe('file');
    expect(classifyFile('chatgpt-export-2026.zip').kind).toBe('file');
    expect(classifyFile('Q3 variance.xlsx').kind).toBe('file');
  });
  it('builds what Import a source hands ingest_run (S5)', () => {
    expect(ingestSourceFor('', '/Users/me/Downloads/Q3 report.pdf')).toEqual({ kind: 'file', path: '/Users/me/Downloads/Q3 report.pdf', displayName: 'Q3 report.pdf' });
    // A picked file wins over typed text, which stays the draft.
    expect(ingestSourceFor('https://example.com', '/tmp/a.md')?.kind).toBe('file');
    expect(ingestSourceFor('  https://example.com/report  ', null)).toEqual({ kind: 'url', url: 'https://example.com/report' });
    // A web address with more text after it is text, not a page to fetch.
    expect(ingestSourceFor('https://example.com/report says the gate is late', null)?.kind).toBe('text');
    expect(ingestSourceFor('Gate notes\nArm sticks after rain', null)).toEqual({ kind: 'text', content: 'Gate notes\nArm sticks after rain', title: 'Gate notes' });
    expect(ingestSourceFor('x'.repeat(90), null)).toMatchObject({ title: 'x'.repeat(60) });
    expect(ingestSourceFor('   ', null)).toBeNull();
    expect(isWebAddress(' https://example.com/a ')).toBe(true);
    expect(isWebAddress('example.com')).toBe(false);
    // The kind pills say where each choice goes.
    expect([CAPTURE_LABEL.source, CAPTURE_LABEL.file]).toEqual(['Library source', 'Workspace file']);
  });
  it('names a note from its first line', () => {
    expect(noteName('# Site visit: Garage 14\nmore')).toBe('Site visit  Garage 14'.replace('  ', ' '));
    expect(noteName('   ')).toBe('Untitled note');
  });
});
