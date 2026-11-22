import { describe, expect, it } from 'vitest';
import {
  fileHrefFromCode, fileLinkFromHref, fileRefFromCode, footerChipPath, isInlineImage, joinPath, localFileFromHref,
  normalizePath, splitLineSuffix, videoRefsFromText,
} from './chatFileLinks';
import replySource from './components/ReplyMarkdown.tsx?raw';
import chatRuntimeSource from './store/chatRuntime.ts?raw';
import fileOpenSource from './store/fileOpen.ts?raw';

describe('normalizePath', () => {
  it('collapses dot segments and repeated slashes', () => {
    expect(normalizePath('/a//b/./c/../d.md')).toBe('/a/b/d.md');
    expect(normalizePath('/../x')).toBe('/x');
  });
  it('joins a base and a relative path', () => {
    expect(joinPath('/ws/', 'outputs/r.pdf')).toBe('/ws/outputs/r.pdf');
    expect(joinPath('/ws/a', '../b.md')).toBe('/ws/b.md');
  });
});

describe('localFileFromHref', () => {
  const base = '/Users/me/vault/workspaces/w1';
  it('expands only a leading home slash using the OS home, not the workspace', () => {
    expect(localFileFromHref('~/My%20Notes/a.md:12:3', base, '/Users/me')).toBe('/Users/me/My Notes/a.md');
    expect(localFileFromHref('~/outputs/a.png', null, '/Users/me')).toBe('/Users/me/outputs/a.png');
    expect(localFileFromHref('~someone/a.md', base, '/Users/me')).toBeNull();
    expect(localFileFromHref('~/a.md', base, null)).toBeNull();
    expect(localFileFromHref('outputs/a.png', base)).toBe(`${base}/outputs/a.png`);
  });
  it('keeps absolute paths and strips line suffixes', () => {
    expect(localFileFromHref('/repo/src/app.ts:42', base)).toBe('/repo/src/app.ts');
    expect(localFileFromHref('/repo/src/app.ts:42:7', base)).toBe('/repo/src/app.ts');
    expect(localFileFromHref('/repo/src/app.ts#L12', base)).toBe('/repo/src/app.ts');
    expect(localFileFromHref('/repo/src/app.ts:10-20', base)).toBe('/repo/src/app.ts');
  });
  it('resolves relative links against the conversation folder', () => {
    expect(localFileFromHref('outputs/weekly.pdf', base)).toBe(`${base}/outputs/weekly.pdf`);
    expect(localFileFromHref('./PLAN.md', base)).toBe(`${base}/PLAN.md`);
    expect(localFileFromHref('outputs/a.md', null)).toBeNull();
  });
  it('decodes file URLs and percent-encoding', () => {
    expect(localFileFromHref('file:///Users/me/My%20Doc.pdf', base)).toBe('/Users/me/My Doc.pdf');
    expect(localFileFromHref('file://localhost/tmp/x.png', base)).toBe('/tmp/x.png');
    expect(localFileFromHref('Alex%E2%80%99s/notes.md', '/v')).toBe('/v/Alex’s/notes.md');
  });
  it('leaves web links, mail links, anchors and home paths alone', () => {
    expect(localFileFromHref('https://example.com/a.pdf', base)).toBeNull();
    expect(localFileFromHref('mailto:x@y.z', base)).toBeNull();
    expect(localFileFromHref('#section', base)).toBeNull();
    expect(localFileFromHref('~/notes.md', base)).toBeNull();
    expect(localFileFromHref('', base)).toBeNull();
  });
});

describe('footerChipPath', () => {
  it('uses the repository root, else the vault root', () => {
    expect(footerChipPath({ path: 'src/a.ts', root: '/repo/' }, '/vault')).toBe('/repo/src/a.ts');
    expect(footerChipPath({ path: 'workspaces/w/PLAN.md' }, '/vault')).toBe('/vault/workspaces/w/PLAN.md');
    expect(footerChipPath({ path: 'workspaces/w/PLAN.md' }, null)).toBeNull();
    expect(footerChipPath({ path: '/abs/x.md' }, null)).toBe('/abs/x.md');
  });
});

describe('fileRefFromCode', () => {
  it('expands a home path only when the OS home is known', () => {
    expect(fileRefFromCode('~/My Notes/a.md:42', '/Users/me')).toBe('/Users/me/My Notes/a.md');
    expect(fileRefFromCode('~someone/a.md', '/Users/me')).toBeNull();
    expect(fileRefFromCode('~/a.md')).toBeNull();
  });
  it('reads paths and known file names as files', () => {
    expect(fileRefFromCode('outputs/weekly-exec-landscape-2026-09-26.pdf')).toBe('outputs/weekly-exec-landscape-2026-09-26.pdf');
    expect(fileRefFromCode('src/components/ToolCall.tsx:42')).toBe('src/components/ToolCall.tsx');
    expect(fileRefFromCode('src/app.ts:42:7')).toBe('src/app.ts');
    expect(fileRefFromCode('docs/BUILD_LOG.md#L10-L20')).toBe('docs/BUILD_LOG.md');
    expect(fileRefFromCode('src/a.ts:10-20')).toBe('src/a.ts');
    expect(fileRefFromCode('/Users/me/Documents - Alex’s MacBook Pro/a.md')).toBe('/Users/me/Documents - Alex’s MacBook Pro/a.md');
    expect(fileRefFromCode('WORKSPACE.md')).toBe('WORKSPACE.md');
    expect(fileRefFromCode('package.json')).toBe('package.json');
    // A bare video name is a file too (inline video, 2026-10-17).
    expect(fileRefFromCode('clip.mp4')).toBe('clip.mp4');
    expect(fileRefFromCode('render.MOV')).toBe('render.MOV');
  });
  it('leaves code, commands, URLs and globs alone', () => {
    for (const code of [
      'console.log', 'e.target', 'Node.js', 'npm run build', 'claims-dashboard-landscape-0928',
      'git log --oneline src/a.ts', 'https://example.com/a.pdf', 'src/**/*.ts', '~/notes.md',
      '.gitignore', 'src/components', 'rm -rf .next', 'cat src/a.ts', 'v1.2.3', '$HOME/a.md',
    ]) expect(fileRefFromCode(code), code).toBeNull();
  });
});

// S1b (2026-11-03): a reply's cited line opens at that line.
describe('splitLineSuffix', () => {
  it('reads the line suffixes agents write', () => {
    expect(splitLineSuffix('src/a.ts:12')).toEqual({ ref: 'src/a.ts', lines: { line: 12 } });
    expect(splitLineSuffix('src/a.ts:12:7')).toEqual({ ref: 'src/a.ts', lines: { line: 12 } });
    expect(splitLineSuffix('src/a.ts:10-20')).toEqual({ ref: 'src/a.ts', lines: { line: 10, end: 20 } });
    expect(splitLineSuffix('src/a.ts#L12')).toEqual({ ref: 'src/a.ts', lines: { line: 12 } });
    expect(splitLineSuffix('src/a.ts#L10-L20')).toEqual({ ref: 'src/a.ts', lines: { line: 10, end: 20 } });
    expect(splitLineSuffix('src/a.ts#L10-20')).toEqual({ ref: 'src/a.ts', lines: { line: 10, end: 20 } });
  });
  it('cites nothing without a suffix, at line 0 or past a safe number, and one line for a backwards range', () => {
    expect(splitLineSuffix('src/a.ts')).toEqual({ ref: 'src/a.ts', lines: null });
    expect(splitLineSuffix('src/a.ts:0')).toEqual({ ref: 'src/a.ts', lines: null });
    expect(splitLineSuffix('src/a.ts:99999999999999999999')).toEqual({ ref: 'src/a.ts', lines: null });
    expect(splitLineSuffix('src/a.ts:20-10')).toEqual({ ref: 'src/a.ts', lines: { line: 20 } });
    expect(splitLineSuffix('src/a.ts:12-12')).toEqual({ ref: 'src/a.ts', lines: { line: 12 } });
    // A colon inside the path, or a name ending in digits after a dot, is not a line.
    expect(splitLineSuffix('/Users/me/a:b/c.md')).toEqual({ ref: '/Users/me/a:b/c.md', lines: null });
    expect(splitLineSuffix('C:')).toEqual({ ref: 'C:', lines: null });
    expect(splitLineSuffix('notes/v1.2')).toEqual({ ref: 'notes/v1.2', lines: null });
    expect(splitLineSuffix('a.md#Lsection')).toEqual({ ref: 'a.md#Lsection', lines: null });
  });
});

describe('fileLinkFromHref (a reply link opens at its cited lines)', () => {
  const base = '/Users/me/vault/workspaces/w1';
  it('keeps the line and the end line', () => {
    expect(fileLinkFromHref('docs/X.md:42', base)).toEqual({ path: `${base}/docs/X.md`, line: 42 });
    expect(fileLinkFromHref('src/a.ts:10-20', base)).toEqual({ path: `${base}/src/a.ts`, line: 10, end: 20 });
    expect(fileLinkFromHref('/repo/src/app.ts:42:7', base)).toEqual({ path: '/repo/src/app.ts', line: 42 });
    expect(fileLinkFromHref('/repo/src/app.ts#L12', base)).toEqual({ path: '/repo/src/app.ts', line: 12 });
    expect(fileLinkFromHref('/repo/src/app.ts#L12-L30', base)).toEqual({ path: '/repo/src/app.ts', line: 12, end: 30 });
    expect(fileLinkFromHref('file:///Users/me/My%20Doc.md#L3', base)).toEqual({ path: '/Users/me/My Doc.md', line: 3 });
    expect(fileLinkFromHref('/repo/a.ts?plain=1#L5', base)).toEqual({ path: '/repo/a.ts', line: 5 });
    expect(fileLinkFromHref('/repo/a%20b.ts%3A9', base)).toEqual({ path: '/repo/a b.ts', line: 9 });
    expect(fileLinkFromHref('~/notes/a.md:4-6', base, '/Users/me')).toEqual({ path: '/Users/me/notes/a.md', line: 4, end: 6 });
  });
  it('opens a link with no line at the top, as before', () => {
    expect(fileLinkFromHref('outputs/weekly.pdf', base)).toEqual({ path: `${base}/outputs/weekly.pdf` });
    expect(fileLinkFromHref('/repo/a.md#section', base)).toEqual({ path: '/repo/a.md' });
    expect(fileLinkFromHref('/repo/a.md#L', base)).toEqual({ path: '/repo/a.md' });
    expect(fileLinkFromHref('/repo/a.md:0', base)).toEqual({ path: '/repo/a.md' });
    expect(fileLinkFromHref('/Users/me/a:b/c.md', base)).toEqual({ path: '/Users/me/a:b/c.md' });
  });
  it('is not a file for web links, other schemes, Windows drives, anchors or an unresolvable path', () => {
    for (const href of [
      'https://example.com/a.ts#L12', 'http://localhost:3000/a.ts:12', 'mailto:x@y.z', 'C:\\Users\\me\\a.ts:12',
      'C:/Users/me/a.ts:12', '#L12', '', '~/a.md:3',
    ]) expect(fileLinkFromHref(href, base), href).toBeNull();
    expect(fileLinkFromHref('src/a.ts:12', null)).toBeNull();
  });
});

describe('fileHrefFromCode (an inline-code path carries its cited lines)', () => {
  it('keeps the lines as a #L fragment, and leaves a path with no line as it was', () => {
    expect(fileHrefFromCode('docs/X.md:42')).toBe('docs/X.md#L42');
    expect(fileHrefFromCode('src/a.ts:10-20')).toBe('src/a.ts#L10-20');
    expect(fileHrefFromCode('src/app.ts:42:7')).toBe('src/app.ts#L42');
    // `#`, `?` and `%` in the path survive the round trip through the click.
    expect(fileLinkFromHref(fileHrefFromCode('src/C#/a.cs:5')!, '/ws')).toEqual({ path: '/ws/src/C#/a.cs', line: 5, end: undefined });
    expect(fileLinkFromHref(fileHrefFromCode('docs/50%20off.md:3')!, '/ws')).toEqual({ path: '/ws/docs/50%20off.md', line: 3, end: undefined });
    // A `?` in inline code isn't read as a path at all, so nothing to escape.
    expect(fileHrefFromCode('notes/why?.md')).toBeNull();
    expect(fileHrefFromCode('docs/BUILD_LOG.md#L10-L20')).toBe('docs/BUILD_LOG.md#L10-20');
    expect(fileHrefFromCode('README.md:12')).toBe('README.md#L12');
    expect(fileHrefFromCode('~/My Notes/a.md:3', '/Users/me')).toBe('/Users/me/My Notes/a.md#L3');
    expect(fileHrefFromCode('outputs/summary.md')).toBe('outputs/summary.md');
    expect(fileHrefFromCode('src/a.ts:0')).toBe('src/a.ts');
    for (const code of ['npm run build', 'console.log', 'https://example.com/a.ts:12', '12:30-14:00', 'C:12'])
      expect(fileHrefFromCode(code), code).toBeNull();
  });
  it('reads back through the click as the file and its lines', () => {
    const base = '/ws';
    const click = (code: string) => fileLinkFromHref(fileHrefFromCode(code, '/Users/me')!, base);
    expect(click('docs/X.md:42')).toEqual({ path: '/ws/docs/X.md', line: 42 });
    expect(click('src/a.ts:10-20')).toEqual({ path: '/ws/src/a.ts', line: 10, end: 20 });
    // A bare name with a line is a file, not a `README.md:` scheme.
    expect(click('README.md:12')).toEqual({ path: '/ws/README.md', line: 12 });
    expect(click('~/a.md:5')).toEqual({ path: '/Users/me/a.md', line: 5 });
    expect(click('notes/plan.md')).toEqual({ path: '/ws/notes/plan.md' });
    // A Windows path still reads as a `C:` link, which Hollow can't open.
    expect(fileHrefFromCode('C:\\a\\b.ts:12')).toBe('C:\\a\\b.ts#L12');
    expect(click('C:\\a\\b.ts:12')).toBeNull();
  });
  it('the reply marks code with it and its click hands the lines to the Preview pane', () => {
    const flat = (t: string) => t.replace(/\s+/g, '');
    expect(replySource).toContain("const ref = fileHrefFromCode(code.textContent ?? '', homePath);");
    // Since 2026-11-04 the click reads the link against each of the chat's
    // folders (store/fileOpen.ts) and opens the first that holds the file.
    expect(flat(replySource)).toContain(flat('await openNamedFile(ref, homePath, { owner: click });'));
    expect(flat(fileOpenSource)).toContain(flat(': fileLinkFromHref(href, base, home);'));
    expect(flat(fileOpenSource)).toContain(flat('if (found) { openFileInSidePane(found, { line: first.line, end: first.end }); return true; }'));
    // openFileInSidePane → openChatPreview → openPreview with the lines.
    expect(flat(chatRuntimeSource)).toContain(flat("openPreview({ kind: 'file', path: target, line: lines?.line, end: lines?.end });"));
    expect(flat(chatRuntimeSource)).toContain(flat('openChatPreview(target, lines);'));
  });
});

describe('isInlineImage', () => {
  it('recognizes image data URLs only', () => {
    expect(isInlineImage('data:image/png;base64,AAAA')).toBe(true);
    expect(isInlineImage('data:text/html,hi')).toBe(false);
    expect(isInlineImage('/a/b.png')).toBe(false);
  });
});

describe('videoRefsFromText (inline video)', () => {
  const base = '/vault/workspaces/w';
  it('finds links, file URLs, inline code, bare absolute paths and raw media tags, in order, once', () => {
    const md = [
      'Rendered [the cut](outputs/cut.mp4) and `outputs/clip.MOV`.',
      'Also file:///Users/me/Movies/a.mp4 — no, that one is [here](file:///Users/me/proj/a.m4v).',
      'Bare: /Users/me/proj/render.mp4.',
      '<video src="/Users/me/proj/raw.mp4" controls></video><video><source src="outputs/src.mov"></video>',
      '<a href="outputs/cut.mp4">again</a>',
    ].join('\n');
    expect(videoRefsFromText(md, base)).toEqual([
      '/vault/workspaces/w/outputs/cut.mp4',
      '/vault/workspaces/w/outputs/clip.MOV',
      '/Users/me/Movies/a.mp4',
      '/Users/me/proj/a.m4v',
      '/Users/me/proj/render.mp4',
      '/Users/me/proj/raw.mp4',
      '/vault/workspaces/w/outputs/src.mov',
    ]);
  });
  it('skips fenced blocks, commands, web links, ~ paths and other files', () => {
    const md = [
      '```sh',
      'ffmpeg -i in.mov /tmp/out.mp4',
      '```',
      'Run `ffmpeg -i a -o out.mp4`, see [web](https://x.com/a.mp4), [home](~/a.mp4), `outputs/notes.md`, `clip.webm`.',
    ].join('\n');
    expect(videoRefsFromText(md, base)).toEqual([]);
  });
  it('reads a command in inline code as code, not as bare paths', () => {
    expect(videoRefsFromText('Ran `ffmpeg -i /Users/me/raw.mov /Users/me/out.mp4` and `/Users/me/out.mp4`.', base))
      .toEqual(['/Users/me/out.mp4']);
  });
  it('takes angle-bracket links with spaces, bold or bracketed bare paths, and one path per file whatever its case', () => {
    expect(videoRefsFromText('[cut](<outputs/My Clip.mp4>), **/Users/me/a.mp4**, (/Users/me/b.mov) and /Users/me/A.MP4', base))
      .toEqual(['/vault/workspaces/w/outputs/My Clip.mp4', '/Users/me/a.mp4', '/Users/me/b.mov']);
  });
  it('ignores a fence-like line with an info string inside an open fence', () => {
    expect(videoRefsFromText('```\n```js\n/Users/me/in.mp4\n```\nThen /Users/me/out.mp4', base)).toEqual(['/Users/me/out.mp4']);
  });
  it('needs a base for a relative path, and takes a bare file name in code', () => {
    expect(videoRefsFromText('`clip.mp4`', null)).toEqual([]);
    expect(videoRefsFromText('`clip.mp4`', base)).toEqual(['/vault/workspaces/w/clip.mp4']);
  });
  it('reads inline code and bare paths as written, and links as hrefs (lane S follow-ups)', () => {
    // A literal path keeps `%` and `#`: no percent-decoding, no fragment cut.
    expect(videoRefsFromText('Saved `/a/100%.mov`.', base)).toEqual(['/a/100%.mov']);
    expect(videoRefsFromText('Saved /a/100%.mov.', base)).toEqual(['/a/100%.mov']);
    expect(videoRefsFromText('Saved `/a/take #2.mov` and /a/cut#3.mp4', base)).toEqual(['/a/take #2.mov', '/a/cut#3.mp4']);
    expect(videoRefsFromText('`outputs/50%20off.mov`', base)).toEqual(['/vault/workspaces/w/outputs/50%20off.mov']);
    // A link or a file URL is an href: decoded once, and its target is not
    // read a second time, literally, as a bare path.
    expect(videoRefsFromText('[cut](/a/100%25.mov) and [two](outputs/a%20b.mov)', base)).toEqual(['/a/100%.mov', '/vault/workspaces/w/outputs/a b.mov']);
    expect(videoRefsFromText('See file:///a/100%25.mov and <video src="/a/x%23y.mp4">', base)).toEqual(['/a/100%.mov', '/a/x#y.mp4']);
  });
});
