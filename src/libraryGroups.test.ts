import { describe, expect, it } from 'vitest';
import { groupLibrary, libraryTitle, matchesLibraryQuery, wikilinkStep } from './libraryGroups';
import type { NoteRef } from './types';
import LIBRARY from './pages/Library.tsx?raw';
import EDITOR from './components/WorkFileEditor.tsx?raw';

const note = (path: string, title?: string, summary?: string): NoteRef =>
  ({ path, basename: path.split('/').pop()!.replace(/\.md$/, ''), title, summary });

describe('Library groups (2026-10-01)', () => {
  const notes = [
    note('AGENTS.md'),
    note('drafts/heartbeat/2026-10-01-thursday-due.md', 'Thursday 2026-10-01 — what’s due'),
    note('library/atlas/Atlas.md', 'Atlas', 'Acme’s revenue-reconciliation platform.'),
    note('library/atlas/sources/emails/scan.md', 'Email scan'),
    note('library/atlas/archive-v1/README.md', 'v1 archive'),
    note('library/atlas/sources/meetings/2026-06-30-vendor-migration.md', 'Vendor migration'),
    note('library/atlas/sources/meetings/2026-07-10-advisor-call.md', 'Consultation: Jordan Blake'),
    note('library/atlas/archive-v1/synthesis/01-problem.md', '01 — The problem and the mandate'),
    ...['P046be4a221cb', 'P2c54e77a3f63', 'P3e2faa523f5d'].map((id, i) =>
      note(`library/work/reference/evidence/plaud-2026-09-11/${id}.md`, `Meeting ${i}`, 'Selected work-related excerpts.')),
    note('library/work/articles/default.md', 'Engineering', 'The team remit.'),
    note('library/work/articles/company-budget-and-scale.md', '2027 budgets and company scale'),
    note('library/work/people/apex-partners.md', 'Apex Partners', 'Apex Partners (advisory): Morgan Cho.'),
    note('learnings/provider-agnostic.md', 'Hollow work must not depend on…', 'Hollow work must not depend on a particular provider.'),
  ];
  const groups = groupLibrary(notes);

  it('titles rows from the note instead of the file name', () => {
    const ev = groups.find(g => g.key.endsWith('plaud-2026-09-11'))!;
    expect(ev.entries[0]).toMatchObject({ title: 'Meeting 0', basename: 'P046be4a221cb' });
    expect(libraryTitle({ basename: 'notes' })).toBe('notes');
  });

  it('drops a summary every note in the folder repeats', () => {
    const ev = groups.find(g => g.key.endsWith('plaud-2026-09-11'))!;
    expect(ev.entries.every(e => e.detail === '')).toBe(true);
    const articles = groups.find(g => g.key === 'library/work/articles')!;
    expect(articles.entries.map(e => e.detail)).toEqual(['', 'The team remit.']);
    const learnings = groups.find(g => g.key === 'learnings')!;
    expect(learnings.entries[0].detail).toBe('');
    const people = groups.find(g => g.key === 'library/work/people')!;
    expect(people.entries[0].detail).toBe('Apex Partners (advisory): Morgan Cho.');
  });

  it('labels folders for people and folds evidence and archives', () => {
    expect(groups.map(g => [g.label, g.folded])).toEqual([
      ['Atlas', false],
      ['Atlas › Sources › Meetings', false],
      ['Work › Articles', false],
      ['Work › People', false],
      ['Atlas › Archive v1', true],
      ['Work › Reference › Evidence › Plaud 2026-09-11', true],
      ['Drafts › Heartbeat', false],
      ['Learnings', false],
      ['Vault', false],
    ]);
  });

  it('folds a one-note folder into its parent, but never an archive into open notes', () => {
    expect(groups.find(g => g.key === 'library/atlas')!.entries.map(e => e.title)).toEqual(['Atlas', 'Email scan']);
    // The archive's own README has no listed ancestor that is also folded.
    expect(groups.some(g => g.key === 'library/atlas/archive-v1' && g.folded)).toBe(true);
  });

  it('lists dated folders newest first', () => {
    const meetings = groups.find(g => g.key === 'library/atlas/sources/meetings')!;
    expect(meetings.entries.map(e => e.title)).toEqual(['Consultation: Jordan Blake', 'Vendor migration']);
    expect(meetings.entries[0].detail).toBe('2026-07-10');
    const drafts = groups.find(g => g.key === 'drafts/heartbeat')!;
    expect(drafts.entries[0].detail).toBe('');
  });

  it('finds a note by its title as well as its file name', () => {
    const p = notes.find(n => n.basename === 'P2c54e77a3f63')!;
    expect(matchesLibraryQuery(p, 'meeting 1')).toBe(true);
    expect(matchesLibraryQuery(p, 'p2c54')).toBe(true);
    expect(matchesLibraryQuery(p, 'vendor')).toBe(false);
  });
});

describe('Library wikilinks (P0-26)', () => {
  const notes = [note('library/atlas/Atlas.md', 'Atlas platform'), note('library/Launch plan.md'), note('people/Morgan Reyes.md')];
  it('a link opens the note it names, by file name, title or path', () => {
    expect(wikilinkStep('Atlas', notes)).toEqual({ kind: 'open', path: 'library/atlas/Atlas.md' });
    expect(wikilinkStep('launch plan', notes)).toEqual({ kind: 'open', path: 'library/Launch plan.md' });
    expect(wikilinkStep('Atlas platform', notes)).toEqual({ kind: 'open', path: 'library/atlas/Atlas.md' });
    expect(wikilinkStep('people/Morgan Reyes.md', notes)).toEqual({ kind: 'open', path: 'people/Morgan Reyes.md' });
  });
  it('a link to no note offers to create it', () => {
    expect(wikilinkStep('Q4 budget', notes)).toEqual({ kind: 'create', title: 'Q4 budget' });
  });
  it('Library hands its handler to the reader and editor', () => {
    expect(LIBRARY).toMatch(/<WorkFileEditor [^>]*onWikilinkClick=\{openWikilink\}/);
    expect(LIBRARY).toMatch(/actionLabel:'Create note'/);
    expect(EDITOR).toMatch(/<NoteRenderer content=\{text\(\)\} onWikilinkClick=\{props\.onWikilinkClick\}/);
    expect(EDITOR).toMatch(/<MarkdownEditor [^>]*onWikilinkClick=\{props\.onWikilinkClick\}/);
  });
});
