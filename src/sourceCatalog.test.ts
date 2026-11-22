import { describe, expect, it } from 'vitest';
import { SOURCE_COLLECTIONS, SOURCE_LIMITS, inSourceCollection, sourcePickerWindow } from './sourceCatalog';

// P1-04: one limits sentence for the Sources section and its list, and it
// says Hollow reads PDF and Office files itself (doc_text.rs: pdf, docx,
// pptx, xlsx); only images and other formats need a reader.
describe('Sources limits', () => {
  it('says Hollow reads PDF and Office files, and only images need a reader', () => {
    expect(SOURCE_LIMITS).toContain('Hollow reads the text of PDF, Word, PowerPoint and Excel files');
    expect(SOURCE_LIMITS).toContain('images and other formats need a runtime with a file reader, or a text export');
    expect(SOURCE_LIMITS).not.toMatch(/PDF, workbook and image attachments need/);
  });
});

describe('source collections', () => {
  it('keeps imported material shared, with connector records separate until selected', () => {
    expect(inSourceCollection('library/imports/report/source.md','shared')).toBe(true);
    for(const path of ['calendar/meeting.md','recordings/call.md','people/person.md','workspaces/a/WORKSPACE.md','me/Identity.md']) expect(inSourceCollection(path,'shared')).toBe(false);
    expect(inSourceCollection('tasks/next.md','projects')).toBe(true);
    expect(inSourceCollection('projects/active/review.md','projects')).toBe(true);
  });
  it('finds reusable results without treating source snapshots as outputs', () => {
    expect(inSourceCollection('workspaces/review/outputs/result.md','outputs')).toBe(true);
    expect(inSourceCollection('briefings/old-report.md','outputs')).toBe(true);
    expect(inSourceCollection('workspaces/review/sources/result.md','outputs')).toBe(false);
    expect(inSourceCollection('old-folder/archive.md','all')).toBe(true);
  });
  it('names workspace outputs Result files; only the id and folder keep "outputs" (decision 7)', () => {
    const results = SOURCE_COLLECTIONS.find(c => c.id === 'outputs');
    expect(results?.label).toBe('Result files');
    for (const c of SOURCE_COLLECTIONS) expect(`${c.label} ${c.description}`.replace(/outputs\//g, '')).not.toMatch(/\boutputs?\b/i);
  });
  it('reuses no Library tab name, and the two that hold a tab’s files link to it (P1-19)', () => {
    for (const c of SOURCE_COLLECTIONS) expect(['Notes', 'Results', 'Recordings', 'Connected projects']).not.toContain(c.label);
    const linked = Object.fromEntries(SOURCE_COLLECTIONS.filter(c => 'tab' in c).map(c => [c.label, (c as { tab: string }).tab]));
    expect(linked).toEqual({ 'Result files': 'results', 'Kept transcripts': 'recordings' });
  });
});

describe('the source picker count is exact (review F04)', () => {
  it('says it is truncated only when it is', () => {
    expect(sourcePickerWindow(0)).toEqual({ shown: 0, message: null });
    expect(sourcePickerWindow(60)).toEqual({ shown: 60, message: null });
    expect(sourcePickerWindow(61)).toEqual({ shown: 61, message: null });
    expect(sourcePickerWindow(100)).toEqual({ shown: 100, message: null });
    expect(sourcePickerWindow(101).message).toBe('Showing the first 100 of 101 matching files. Type in Find to narrow them.');
    expect(sourcePickerWindow(401)).toMatchObject({ shown: 100 });
    expect(sourcePickerWindow(2600, 100).message).toContain('of 2,600');
  });
});
