import { describe, expect, it } from 'vitest';
import { chipFolders, chipLabel, chipTarget, durationText, endsReply, fileChips, finishedText, isListedResult, notesUpdate, replyNoteBody, replyNotePath, replyNoteSlug, replyNoteTitle, replyQuestion, replyText, runDuration, tildeHome, typicalLine } from './turnFooter';
import type { ExecutionReceipt } from './executionReceipt';

function receipt(over: Partial<ExecutionReceipt> = {}): ExecutionReceipt {
  return {
    status: 'unverified', ordering: 'known', files: [], checks: [], failures: [],
    eventCount: 0, ...over,
  };
}

const file = (path: string, additions?: number, deletions?: number) => ({
  path, tool: 'Write', sequence: 0, changeType: 'modify',
  ...(additions != null ? { additions } : {}), ...(deletions != null ? { deletions } : {}),
});

describe('fileChips', () => {
  it('shows two chips and folds the rest with summed counts', () => {
    const s = fileChips(receipt({ files: [file('CLAUDE.md', 35, 10), file('docs/BUILD_LOG.md', 39, 0), file('a.md', 5, 1), file('b.md', 2, 3)] }));
    expect(s.shown.map(c => c.name)).toEqual(['CLAUDE.md', 'BUILD_LOG.md']);
    expect(s.shown[0]).toMatchObject({ additions: 35, deletions: 10, path: 'CLAUDE.md' });
    expect(s.more).toEqual({ count: 2, additions: 7, deletions: 4 });
    expect(s.total).toBe(4);
  });

  it('leaves another chat\'s files out of the chips and the folded totals (chat concurrency F1)', () => {
    const shared = (path: string, additions: number) => ({ ...file(path, additions, 0), tool: 'changed by another chat', attribution: 'shared' });
    const s = fileChips(receipt({ files: [shared('report.md', 90), file('a.md', 1, 0), shared('peer.md', 50), file('b.md', 2, 0), file('c.md', 3, 1), file('d.md', 4, 0)] }));
    expect(s.shown.map(c => c.name)).toEqual(['a.md', 'b.md']);
    expect(s.more).toEqual({ count: 2, additions: 7, deletions: 1 });
    expect(s.total).toBe(4);
    const reviewer = fileChips(receipt({ files: [shared('report.md', 90)] }));
    expect(reviewer).toMatchObject({ shown: [], more: null, total: 0 });
  });

  it('never folds a single chip into "+1 more"', () => {
    const s = fileChips(receipt({ files: [file('a.md'), file('b.md'), file('c.md')] }));
    expect(s.shown).toHaveLength(3);
    expect(s.more).toBeNull();
  });

  it('keeps counts unknown when the backend had none', () => {
    const s = fileChips(receipt({ files: [file('a.md'), file('b.md'), file('c.md'), file('d.md')] }));
    expect(s.shown[0].additions).toBeNull();
    expect(s.more).toEqual({ count: 2, additions: null, deletions: null });
  });

  it('adds repository rows after vault files and counts unlisted ones', () => {
    const repoFile = (path: string, extra = {}) => ({
      path, changeType: 'modify', status: ' M', untracked: false, gitlink: false, additions: 4, deletions: 2,
      binary: false, oversized: false, patchTruncated: false, withheld: false, dirtyBefore: false,
      attribution: 'observed', tool: null, ...extra,
    });
    const s = fileChips(receipt({
      files: [file('notes/a.md', 1, 0)],
      repoChanges: [{
        root: '/repo', label: 'repo', target: 'cwd', status: 'captured', headBefore: null, headAfter: null,
        files: [repoFile('src/x.ts'), repoFile('src/clean.ts', { status: 'clean' }), repoFile('img.png', { binary: true })],
        totalFiles: 5, truncated: false,
      }],
    }));
    expect(s.shown.map(c => c.key)).toEqual(['vault:notes/a.md', 'repo:/repo:src/x.ts']);
    expect(s.shown[1].root).toBe('/repo');
    // img.png (binary: no counts) plus two rows past the listed three.
    expect(s.more).toEqual({ count: 3, additions: null, deletions: null });
    expect(s.total).toBe(5);
  });

  it('is empty without a receipt', () => {
    expect(fileChips(undefined)).toEqual({ shown: [], folded: [], more: null, total: 0 });
  });
});

describe('notesUpdate (COO notes)', () => {
  it('folds the COO\'s brief and plan into one line and keeps other files', () => {
    const r = receipt({ files: [file('workspaces/hollow/WORKSPACE.md', 4, 1), file('workspaces/hollow/PLAN.md', 2, 0), file('workspaces/hollow/outputs/note.md', 8, 0)] });
    const folded = notesUpdate(r, 'hollow');
    expect(folded.updated).toBe(true);
    expect(folded.notePath).toBe('workspaces/hollow/WORKSPACE.md');
    expect(folded.chips.shown.map(c => c.name)).toEqual(['note.md']);
    expect(folded.chips.total).toBe(1);
    // A project chat still shows the notes as chips.
    expect(notesUpdate(r).updated).toBe(false);
    expect(notesUpdate(r).chips.total).toBe(3);
    // Another chat's copy of the brief is not this reply's notes.
    const shared = notesUpdate(receipt({ files: [{ ...file('workspaces/hollow/PLAN.md', 1, 0), attribution: 'shared' }] }), 'hollow');
    expect(shared.updated).toBe(false);
    expect(shared.chips.total).toBe(0);
  });
  it('keeps another project and nested same-name notes as file chips', () => {
    const r = receipt({ files: [file('workspaces/main/PLAN.md', 1, 0), file('workspaces/project/WORKSPACE.md', 2, 1), file('workspaces/main/sources/WORKSPACE.md', 3, 0)] });
    const folded = notesUpdate(r, 'main');
    expect(folded.updated).toBe(true);
    expect(folded.notePath).toBe('workspaces/main/PLAN.md');
    expect(folded.chips.shown.map(c => c.path)).toEqual(['workspaces/project/WORKSPACE.md', 'workspaces/main/sources/WORKSPACE.md']);
    const other = notesUpdate(receipt({ files: [file('workspaces/project/PLAN.md', 2, 0)] }), 'main');
    expect(other.updated).toBe(false);
    expect(other.chips.total).toBe(1);
    expect(notesUpdate(r).chips.total).toBe(3);
  });
});

describe('durations and times', () => {
  it('formats whole seconds', () => {
    expect(durationText(8_400)).toBe('8s');
    expect(durationText(713_000)).toBe('11m 53s');
    expect(durationText(3_840_000)).toBe('1h 4m');
    expect(durationText(-5)).toBe('0s');
    expect(typicalLine(240_000, 'code')).toBe('typically 4m 0s for code');
    expect(typicalLine(null, 'code')).toBeNull();
    expect(typicalLine(240_000, '  ')).toBeNull();
    expect(typicalLine(Number.NaN, 'code')).toBeNull();
  });

  it('reads a run duration from its receipt', () => {
    expect(runDuration({ startedAt: '2026-09-26T19:00:07Z', finishedAt: '2026-09-26T19:12:00Z' })).toBe('11m 53s');
    expect(runDuration({ startedAt: 'nope', finishedAt: '2026-09-26T19:12:00Z' })).toBeNull();
    expect(runDuration(undefined)).toBeNull();
  });

  it('adds a date only for another day', () => {
    const at = new Date(2026, 8, 26, 19, 12).toISOString();
    const sameDay = new Date(2026, 8, 26, 23, 0).getTime();
    const later = new Date(2026, 8, 28, 9, 0).getTime();
    expect(finishedText(at, sameDay, 'en-US')).toBe('7:12 PM');
    expect(finishedText(at, later, 'en-US')).toBe('Sep 26, 7:12 PM');
    expect(finishedText(undefined)).toBeNull();
  });
});

describe('reply grouping', () => {
  const turns = [
    { role: 'user', text: 'do it' },
    { role: 'assistant', text: 'Reading.' },
    { role: 'assistant', text: 'Done.' },
    { role: 'system', text: 'Stopped.' },
    { role: 'user', text: 'again' },
    { role: 'assistant', text: 'Ok' },
  ];

  it('ends a reply only on its last assistant turn', () => {
    expect(turns.map((_, i) => endsReply(turns, i))).toEqual([false, false, true, false, false, true]);
  });

  it('joins the reply text since the last user message', () => {
    expect(replyText(turns, 2)).toBe('Reading.\n\nDone.');
    expect(replyText(turns, 5)).toBe('Ok');
  });

  it('reads an index past the end as no reply instead of throwing (a row read after the turns were cleared)', () => {
    expect(replyText([], 3)).toBe('');
    expect(replyQuestion([], 3)).toBeNull();
  });

  it('keeps the first line’s indentation, so an indented code block stays code (Codex read)', () => {
    const code = [
      { role: 'user', text: 'Show it' },
      { role: 'assistant', text: '\n    const x = 1;\n    run(x);\n\n' },
      { role: 'assistant', text: 'That runs.\n' },
    ];
    expect(replyText(code, 2)).toBe('    const x = 1;\n    run(x);\n\nThat runs.');
  });
});

describe('chipTarget (matrix N8)', () => {
  const ws = { id: 'local-q3-review' };
  const chip = (path: string, extra: { root?: string; changeType?: string } = {}) => ({ path, changeType: 'modify', ...extra });
  const dir = 'workspaces/local-q3-review/';

  it('opens a Result in its workspace outputs/ in Results, by its vault path', () => {
    expect(chipTarget(chip(`${dir}outputs/q3-regional-review.md`), ws))
      .toEqual({ to: 'results', workspaceId: 'local-q3-review', path: `${dir}outputs/q3-regional-review.md` });
    expect(chipTarget(chip(`${dir}outputs/tables/variance.csv`, { changeType: 'create' }), ws).to).toBe('results');
    expect(chipTarget(chip(`${dir}outputs/reviews/home.annotations.json`), ws).to).toBe('results');
    expect(chipTarget(chip(`${dir}outputs/canvases/q3-plan.canvas.html`), ws).to).toBe('results');
    expect(chipTarget(chip(`${dir}outputs/x.md`), { id: 'main', kind: 'main' }).to).toBe('files');
    expect(chipTarget(chip('workspaces/main/outputs/x.md'), { id: 'main', kind: 'main' }).to).toBe('results');
  });

  it('keeps sources, workspace files and other folders in Files', () => {
    for (const path of [`${dir}sources/regional-pnl-q3.csv`, `${dir}WORKSPACE.md`, `${dir}PLAN.md`, `${dir}outputs`, `${dir}outputsx/a.md`, 'CLAUDE.md', 'outputs/report.md', 'notes/outputs/a.md']) {
      expect(chipTarget(chip(path), ws), path).toEqual({ to: 'files' });
    }
  });

  it('keeps another workspace, no workspace and Quick chats in Files', () => {
    expect(chipTarget(chip('workspaces/other/outputs/a.md'), ws).to).toBe('files');
    expect(chipTarget(chip('workspaces/local-q3-review-2/outputs/a.md'), ws).to).toBe('files');
    expect(chipTarget(chip(`${dir}outputs/a.md`), null).to).toBe('files');
    expect(chipTarget(chip(`${dir}outputs/a.md`), undefined).to).toBe('files');
    expect(chipTarget(chip('workspaces/quick-chats/outputs/a.md'), { id: 'quick-chats', kind: 'quick' }).to).toBe('files');
    expect(chipTarget(chip('workspaces/q/outputs/a.md'), { id: 'q', kind: 'quick' }).to).toBe('files');
  });

  it('keeps a deleted file and a repository row out of Results', () => {
    expect(chipTarget(chip(`${dir}outputs/a.md`, { changeType: 'delete' }), ws).to).toBe('files');
    expect(chipTarget(chip(`${dir}outputs/a.md`, { root: '/Users/me/Projects/hollow' }), ws).to).toBe('files');
    expect(chipTarget(chip('/Users/me/vault/workspaces/local-q3-review/outputs/a.md'), ws).to).toBe('files');
  });

  it('leaves what Results never lists in Files: canvas records, curated canvases, dot folders, archives, frames', () => {
    for (const rel of [
      'outputs/canvases/q3-plan.canvas.json',
      'outputs/canvases/_plan.canvas.html', 'outputs/canvases/_overview.canvas.json',
      'outputs/.revisions/a.md', 'outputs/.hidden.md', 'outputs/../WORKSPACE.md', 'outputs//a.md',
      'outputs/workspace-archive/2026-09.md',
      'outputs/process-captures/onboarding/frames/0001.png',
      'outputs/reviews/home/capture-1.png',
      'outputs/a/b/c/d/e/f/g.md',
    ]) expect(chipTarget(chip(dir + rel), ws), rel).toEqual({ to: 'files' });
  });
});

describe('isListedResult (Rust workspace_results::is_listed)', () => {
  it('lists outputs/ files to the walk depth, a canvas once and a capture note', () => {
    expect(isListedResult('outputs/a.md')).toBe(true);
    expect(isListedResult('outputs/a/b/c/d/e/f.md')).toBe(true);
    expect(isListedResult('outputs/a/b/c/d/e/f/g.md')).toBe(false);
    expect(isListedResult('outputs/canvases/q3-plan.canvas.html')).toBe(true);
    // Not a canvas id (capitals): a plain data file, listed like any other.
    expect(isListedResult('outputs/canvases/Q3 Plan.canvas.json')).toBe(true);
    expect(isListedResult('outputs/canvases/_results.canvas.html')).toBe(true);
    expect(isListedResult('outputs/process-captures/onboarding/notes.md')).toBe(true);
    expect(isListedResult('outputs/reviews/home.annotations.json')).toBe(true);
    expect(isListedResult('sources/a.md')).toBe(false);
    expect(isListedResult('outputs')).toBe(false);
  });
});

describe('chipFolders (EH4: same file name, different folders)', () => {
  const vault = (path: string) => ({ name: path.slice(path.lastIndexOf('/') + 1), path });

  it('names folders among thousands of files without rescanning the list per chip (A10 review)', () => {
    const many = Array.from({ length: 3000 }, (_, n) => ({ name: `f${n}.md`, path: `workspaces/w/outputs/f${n}.md` }));
    const twins = [vault('workspaces/w/outputs/notes.md'), vault('workspaces/w/sources/notes.md')];
    // Same short folder: the vault's own outputs/ and the workspace's.
    const alike = [vault('outputs/x.md'), vault('workspaces/w/outputs/x.md')];
    const folders = chipFolders([...many, ...twins, ...alike], 'w');
    expect(folders.slice(0, 3000).every(f => f === null)).toBe(true);
    expect(folders.slice(3000)).toEqual(['outputs', 'sources', 'outputs', 'workspaces/w/outputs']);
  });

  it('names no folder when every name is different', () => {
    expect(chipFolders([vault('workspaces/w/outputs/a.md'), vault('workspaces/w/sources/b.md')], 'w')).toEqual([null, null]);
    expect(chipFolders([])).toEqual([]);
  });

  it('names the folder, relative to the workspace, when two chips share a name', () => {
    const chips = [vault('workspaces/w/outputs/report.md'), vault('workspaces/w/outputs/tables/report.md'), vault('workspaces/w/sources/notes.md')];
    expect(chipFolders(chips, 'w')).toEqual(['outputs', 'outputs/tables', null]);
    expect(chips.map((c, i) => chipLabel(c.name, chipFolders(chips, 'w')[i]))).toEqual(['report.md (outputs)', 'report.md (outputs/tables)', 'notes.md']);
  });

  it('is vault-relative outside the workspace, and a repository row starts with its folder', () => {
    expect(chipFolders([vault('notes/a.md'), vault('workspaces/w/outputs/a.md')], 'w')).toEqual(['notes', 'outputs']);
    expect(chipFolders([vault('notes/a.md'), vault('docs/a.md')])).toEqual(['notes', 'docs']);
    expect(chipFolders([vault('README.md'), { name: 'README.md', path: 'README.md', root: '/Users/me/Projects/hollow' }])).toEqual(['', 'hollow']);
  });

  it('gives the full folder when the short ones would still read alike', () => {
    const repo = (root: string) => ({ name: 'README.md', path: 'docs/README.md', root });
    expect(chipFolders([repo('/a/hollow'), repo('/b/hollow/')])).toEqual(['/a/hollow/docs', '/b/hollow/docs']);
    expect(chipFolders([repo('/Users/me/a/hollow'), repo('/Users/me/b/hollow/')])).toEqual(['~/a/hollow/docs', '~/b/hollow/docs']);
    expect(chipFolders([vault('outputs/x.md'), vault('workspaces/w/outputs/x.md'), vault('workspaces/w/sources/x.md')], 'w'))
      .toEqual(['outputs', 'workspaces/w/outputs', 'sources']);
  });

  it('names a shown chip\'s folder when its only twin is folded into +N more (A10)', () => {
    const s = fileChips(receipt({ files: [file('outputs/report.md'), file('a.md'), file('sources/report.md'), file('b.md')] }), 2);
    expect(s.shown.map(c => c.name)).toEqual(['report.md', 'a.md']);
    expect(s.folded.map(c => c.path)).toEqual(['sources/report.md', 'b.md']);
    expect(chipFolders(s.shown)).toEqual([null, null]);
    expect(chipFolders([...s.shown, ...s.folded]).slice(0, s.shown.length)).toEqual(['outputs', null]);
  });

  it('keeps the bare name for a file at the top of the folder', () => {
    expect(chipLabel('a.md', '')).toBe('a.md');
    expect(chipLabel('a.md', null)).toBe('a.md');
    expect(chipLabel('a.md', 'outputs')).toBe('a.md (outputs)');
  });
});

describe('tildeHome', () => {
  it('shows a leading /Users/<name> as ~ and passes other paths through unchanged', () => {
    expect(tildeHome('/Users/me/Projects/hollow/docs')).toBe('~/Projects/hollow/docs');
    expect(tildeHome('/Users/me')).toBe('~');
    expect(tildeHome('/Users/Shared/hollow/docs')).toBe('/Users/Shared/hollow/docs');
    for (const path of ['/a/hollow/docs', '/Volumes/Users/me/x', 'Users/me/x', '/Users', '/opt/Users/me', 'outputs/x.md', '']) {
      expect(tildeHome(path), path).toBe(path);
    }
  });
});

describe('Save as note on a reply (One Ask, 2026-11-03)', () => {
  const day = new Date(2026, 10, 3, 9, 30);
  const q3 = { id: 'q3-ops', title: 'Q3 regional operations review', path: 'workspaces/q3-ops' };

  it('saves a workspace conversation’s reply into its outputs/answers/, a Result', () => {
    expect(replyNotePath(q3, 'Who owns the Q3 floor price?', day)).toBe('workspaces/q3-ops/outputs/answers/2026-11-03-who-owns-the-q3-floor-price.md');
    // The workspace's own folder when it has one, else workspaces/<id>.
    expect(replyNotePath({ id: 'beta' }, 'Plan', day)).toBe('workspaces/beta/outputs/answers/2026-11-03-plan.md');
  });

  it('saves a quick chat’s, the COO’s or a vault chat’s reply into the Library', () => {
    for (const w of [{ id: 'quick-chats', kind: 'quick' }, { id: 'main', kind: 'main' }, { id: 'main' }, null, undefined]) {
      expect(replyNotePath(w, 'Who owns the Q3 floor price?', day), JSON.stringify(w)).toBe('library/2026-11-03-who-owns-the-q3-floor-price.md');
    }
  });

  it('never overwrites: each further attempt gets the next number', () => {
    expect(replyNotePath(null, 'Plan', day, 1)).toBe('library/2026-11-03-plan.md');
    expect(replyNotePath(null, 'Plan', day, 2)).toBe('library/2026-11-03-plan-2.md');
    expect(replyNotePath(q3, 'Plan', day, 3)).toBe('workspaces/q3-ops/outputs/answers/2026-11-03-plan-3.md');
  });

  it('names the file from the title, with a fallback for one with no letters or digits', () => {
    expect(replyNoteSlug('Café: déjà vu, 2026!')).toBe('cafe-deja-vu-2026');
    expect(replyNoteSlug('…?')).toBe('reply');
    expect(replyNoteSlug('a'.repeat(80))).toHaveLength(60);
    expect(replyNoteSlug('abcd '.repeat(13))).toBe(`${'abcd-'.repeat(11)}abcd`);
  });

  it('titles the note with the question, else the reply’s first line', () => {
    expect(replyNoteTitle('  Who owns\n the price?', 'Pat does.')).toBe('Who owns');
    expect(replyNoteTitle(null, '## Summary\n\nDave owns it.')).toBe('Summary');
    // A reply keeps its first line's indentation, and a heading may sit three spaces in.
    expect(replyNoteTitle(null, replyText([{ role: 'assistant', text: '  # Summary\nDetails.' }], 0))).toBe('Summary');
    expect(replyNoteTitle('', '')).toBe('Saved reply');
    expect(replyNoteTitle('x'.repeat(200), '')).toHaveLength(120);
    // Cut by code point: an emoji at the cut stays whole.
    const emoji = replyNoteTitle(`${'x'.repeat(118)}😀😀😀`, '');
    expect(Array.from(emoji)).toHaveLength(120);
    expect(emoji).toBe(`${'x'.repeat(118)}😀…`);
    expect(() => encodeURIComponent(emoji), 'no lone surrogate').not.toThrow();
  });

  it('keeps the `#` of code lines and `#123` in a note title (ENG-83, ENG-93)', () => {
    expect(replyNoteTitle(null, '    # TODO\nmore')).toBe('# TODO');
    expect(replyNoteTitle(null, '\t# TODO\nmore')).toBe('# TODO');
    expect(replyNoteTitle(null, '```bash\n# install it\nnpm i\n```')).toBe('# install it');
    expect(replyNoteTitle(null, '#123 is fixed')).toBe('#123 is fixed');
    expect(replyNoteTitle(null, '   ### Plan\nSteps')).toBe('Plan');
    expect(replyNoteTitle(null, '#\nNext line')).toBe('Next line');
  });

  it('cuts a long note title on a whole grapheme (ENG-93)', () => {
    // 118 letters, then e + combining acute at code points 119 and 120.
    const accent = replyNoteTitle(`${'x'.repeat(118)}e\u0301${'y'.repeat(10)}`, '');
    expect(accent).toBe(`${'x'.repeat(118)}…`);
  });

  it('finds the question a reply answers', () => {
    const turns = [{ role: 'user', text: 'First?' }, { role: 'assistant', text: 'A' }, { role: 'user', text: 'Second?' }, { role: 'assistant', text: 'B' }, { role: 'assistant', text: 'C' }];
    expect(replyQuestion(turns, 4)).toBe('Second?');
    expect(replyQuestion(turns, 1)).toBe('First?');
    expect(replyQuestion([{ role: 'assistant', text: 'Hello' }], 0)).toBeNull();
  });

  it('keeps the reply as written under its title', () => {
    expect(replyNoteBody('Plan', '\n- one\n- two\n', day)).toBe('# Plan\n\n*Saved from a chat reply on 2026-11-03.*\n\n- one\n- two\n');
  });

  it('keeps an indented first line as written, and drops only the blank lines around it', () => {
    expect(replyNoteBody('Plan', '  \n    const x = 1;\r\n', day)).toBe('# Plan\n\n*Saved from a chat reply on 2026-11-03.*\n\n    const x = 1;\n');
  });

  it('says what it was saved from: a whole conversation, too', () => {
    expect(replyNoteBody('Plan', '## You\n\nHi\n', day, 'conversation')).toBe('# Plan\n\n*Saved from a conversation on 2026-11-03.*\n\n## You\n\nHi\n');
  });
});
