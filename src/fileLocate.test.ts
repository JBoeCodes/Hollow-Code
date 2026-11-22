import { describe, expect, it } from 'vitest';
import { linkBases, locateCandidates, missingFileText } from './fileLocate';
import { fileLinkFromHref, isFileWithLine, localFileFromHref } from './chatFileLinks';
import { prosePathRefs } from './proseFileRefs';

const VAULT = '/Users/j/hollow-vault';
const WS = `${VAULT}/workspaces/local-abc`;
const REPO = '/Users/j/Documents/projects/atlas';
const read = (href: string, bases: string[]) => bases.flatMap(base => { const l = fileLinkFromHref(href, base); return l ? [{ base, path: l.path }] : []; });

describe('where a file a chat names may be (2026-11-04)', () => {
  it('reads the chat’s folders best first: its own, the worktree, the workspace folder, repos, then the vault', () => {
    expect(linkBases({ cwd: WS, repoRoots: [REPO, REPO, null], vault: VAULT })).toEqual([WS, REPO, VAULT]);
    expect(linkBases({ cwd: '/w/tree', worktree: '/w/tree/', workspaceFolder: `${VAULT}/workspaces/b`, vault: `${VAULT}/` }))
      .toEqual(['/w/tree', `${VAULT}/workspaces/b`, VAULT]);
    expect(linkBases({ cwd: null, vault: null })).toEqual([]);
  });

  it('tries a vault-relative path against the vault after the chat’s folder', () => {
    const bases = linkBases({ cwd: WS, vault: VAULT });
    expect(locateCandidates(read('workspaces/main/outputs/plan.md', bases)))
      .toEqual([`${WS}/workspaces/main/outputs/plan.md`, `${VAULT}/workspaces/main/outputs/plan.md`]);
  });

  it('tries a repository-relative path in the project’s repository', () => {
    const bases = linkBases({ cwd: WS, repoRoots: [REPO], vault: VAULT });
    expect(locateCandidates(read('src/app.ts', bases))).toContain(`${REPO}/src/app.ts`);
  });

  it('maps a Mac mini runner path back onto the chat’s folders', () => {
    const remote = '/Users/runner/hollow-remote/folders/w1-9f3a/outputs/report.md';
    const bases = linkBases({ cwd: WS, vault: VAULT });
    expect(locateCandidates(read(remote, bases))).toEqual([remote, `${WS}/outputs/report.md`, `${VAULT}/outputs/report.md`]);
    const session = '/Users/runner/hollow-remote/sessions/s-77/src/a.ts';
    expect(locateCandidates(read(session, ['/w/tree']))).toEqual([session, '/w/tree/src/a.ts']);
  });

  it('adds the conversation’s own files whose path ends with the link’s, last', () => {
    const bases = linkBases({ cwd: WS, vault: VAULT });
    const known = [`${REPO}/docs/notes/q3.md`, `${REPO}/docs/notes/q3.md.bak`, '/elsewhere/notes/q3.md'];
    expect(locateCandidates(read('notes/q3.md', bases), known, { relative: true }))
      .toEqual([`${WS}/notes/q3.md`, `${VAULT}/notes/q3.md`, `${REPO}/docs/notes/q3.md`, '/elsewhere/notes/q3.md']);
  });

  it('never trades an absolute path for another file of the same name (review)', () => {
    const known = ['/repoB/report.md'];
    expect(locateCandidates(read('/repoA/report.md', ['/repoA']), known)).toEqual(['/repoA/report.md']);
  });

  it('takes a bare name’s match only when it is the only one', () => {
    const bases = linkBases({ cwd: WS, vault: VAULT });
    expect(locateCandidates(read('q3.md', bases), ['/a/q3.md'], { relative: true })).toContain('/a/q3.md');
    expect(locateCandidates(read('q3.md', bases), ['/a/q3.md', '/b/q3.md'], { relative: true })).not.toContain('/a/q3.md');
  });

  it('never repeats a place, and drops a relative link with no folder to read it in', () => {
    expect(locateCandidates([{ base: WS, path: `${WS}/a.md` }, { base: WS, path: `${WS}/./a.md` }])).toEqual([`${WS}/a.md`]);
    expect(locateCandidates([])).toEqual([]);
  });

  it('says why no place holds it: scratch, the Mac mini, or moved or deleted', () => {
    expect(missingFileText('/private/tmp/hollow-run-501/abc/chart.png')).toMatch(/^chart\.png was in the turn's scratch folder/);
    expect(missingFileText('/Users/runner/hollow-remote/folders/w1/out.csv')).toMatch(/^out\.csv was on the Mac mini/);
    expect(missingFileText(`${WS}/outputs/gone.md`)).toBe("Hollow couldn't find gone.md in this chat's folder, its project or the vault. It may have been moved or deleted since the reply named it.");
  });
});

describe('a file and its line is not a link scheme (2026-11-04)', () => {
  it('reads README.md:12 as a file at line 12', () => {
    expect(isFileWithLine('README.md:12')).toBe(true);
    expect(isFileWithLine('app.ts:3:9')).toBe(true);
    expect(isFileWithLine('notes.txt:4-9')).toBe(true);
    expect(isFileWithLine('mailto:a@b.c')).toBe(false);
    expect(isFileWithLine('vnd.app:open')).toBe(false);
    expect(isFileWithLine('src/app.ts:3')).toBe(false); // has a folder: no scheme reading to undo
    expect(localFileFromHref('README.md:12', WS)).toBe(`${WS}/README.md`);
    expect(fileLinkFromHref('README.md:12', WS)).toEqual({ path: `${WS}/README.md`, line: 12 });
  });
});

describe('paths written in a reply’s prose are files too (2026-11-04)', () => {
  const found = (text: string, home: string | null = null) => prosePathRefs(text, home).map(r => [text.slice(r.start, r.end), r.href]);
  it('finds relative, absolute and bare document names, with their lines', () => {
    expect(found('I saved it to outputs/q3-review.md. See PLAN.md and src/app.ts:12-20 too.'))
      .toEqual([['outputs/q3-review.md', 'outputs/q3-review.md'], ['PLAN.md', 'PLAN.md'], ['src/app.ts:12-20', 'src/app.ts#L12-20']]);
    expect(found('Open /Users/j/Downloads/Invoice 1.pdf or ../main/outputs/plan.md'))
      .toEqual([['../main/outputs/plan.md', '../main/outputs/plan.md']]);
    expect(found('Written to ~/Desktop/chart.png', '/Users/j')).toEqual([['~/Desktop/chart.png', '/Users/j/Desktop/chart.png']]);
    expect(found('Written to ~/Desktop/chart.png')).toEqual([]);
  });
  it('leaves words, libraries, addresses and web hosts alone', () => {
    expect(found('Built on Node.js and Vue.js, e.g. version 2.0; mail dev@example.com')).toEqual([]);
    expect(found('Visit example.com/docs/page.html or https://x.dev/a/b.md')).toEqual([]);
    expect(found('A sentence. And/or words without an extension.')).toEqual([]);
  });
});
