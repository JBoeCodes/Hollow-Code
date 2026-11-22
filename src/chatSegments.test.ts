import { describe, it, expect } from 'vitest';
import { parseSegments, sameSeg, reuseFinishedCanvases, reuseSettledSegments, splitHiddenFence, splitNextFence, type Segment, RETIRED_BOARD_FENCE, RETIRED_BOARD_NOTE } from './chatSegments';
import HIDDEN from './canvas/hiddenFences.golden.json?raw';

describe('parseSegments', () => {
  it('pairs CRLF fences without swallowing later prose', () => {
    expect(parseSegments('```ts\r\nx\r\n```\r\nAfter')).toEqual([
      { kind: 'code', lang: 'ts', code: 'x\r' }, { kind: 'md', text: 'After' },
    ]);
  });
  it('pairs ordinary no-space info strings and keeps the first word as language', () => {
    for (const lang of ['ts:src/a.ts', 'js{1,3}', '12:15:app/x.tsx']) {
      expect(parseSegments('```' + lang + '\nx\n```\nAfter')).toEqual([
        { kind: 'code', lang, code: 'x' }, { kind: 'md', text: 'After' },
      ]);
    }
  });

  it('reads the first info word without swallowing following prose', () => {
    expect(parseSegments('```ts title="a"\nconst x = 1;\n```\nAfter')).toEqual([
      { kind: 'code', lang: 'ts', code: 'const x = 1;' },
      { kind: 'md', text: 'After' },
    ]);
    expect(parseSegments('~~~~ts title="a"\nx\n~~~\ny\n~~~~\nAfter')).toEqual([
      { kind: 'code', lang: 'ts', code: 'x\n~~~\ny' }, { kind: 'md', text: 'After' },
    ]);
  });

  it('keeps attributed reserved fences visible as code, never hidden or executable', () => {
    for (const lang of ['hollow-next', 'hollow-questions', 'hollow-canvas', 'hollow-whiteboard']) {
      expect(parseSegments('```' + lang + ' title="a"\nsource\n```\nAfter')).toEqual([
        { kind: 'code', lang, code: 'source' }, { kind: 'md', text: 'After' },
      ]);
    }
  });

  it('drops hollow-questions fences (the card renders them)', () => {
    expect(parseSegments('Pick one:\n```hollow-questions\n{"questions":[]}\n```\nThanks')).toEqual([
      { kind: 'md', text: 'Pick one:' },
      { kind: 'md', text: 'Thanks' },
    ]);
    expect(parseSegments('```hollow-questions\n{}\n```')).toEqual([]);
    // Other fences are unchanged.
    expect(parseSegments('```json\n{}\n```')).toEqual([{ kind: 'code', lang: 'json', code: '{}' }]);
  });

  it('keeps plain prose as one markdown segment', () => {
    expect(parseSegments('hello\n\nworld')).toEqual([{ kind: 'md', text: 'hello\n\nworld' }]);
  });

  it('splits a top-level fence out of the surrounding prose', () => {
    expect(parseSegments('before\n```bash\nls -l\n```\nafter')).toEqual([
      { kind: 'md', text: 'before' },
      { kind: 'code', lang: 'bash', code: 'ls -l' },
      { kind: 'md', text: 'after' },
    ]);
  });

  it('routes html and svg fences to the artifact surface, other langs to code', () => {
    expect(parseSegments('```html\n<p>hi</p>\n```')).toEqual([
      { kind: 'html', lang: 'html', code: '<p>hi</p>' },
    ]);
    expect(parseSegments('```SVG\n<svg/>\n```')).toEqual([
      { kind: 'html', lang: 'svg', code: '<svg/>' },
    ]);
    expect(parseSegments('```ts\nconst a = 1;\n```')).toEqual([
      { kind: 'code', lang: 'ts', code: 'const a = 1;' },
    ]);
  });

  it('extracts a fence even when indented under a list', () => {
    // The open pattern allows leading whitespace, so an indented fence is
    // pulled out too. Pinned because it surprises: the surrounding list item
    // ends up split around the code block.
    expect(parseSegments('- item\n  ```js\n  x\n  ```\n')).toEqual([
      { kind: 'md', text: '- item' },
      { kind: 'code', lang: 'js', code: '  x' },
    ]);
  });

  it('splits markdown only at fences, not at blank lines', () => {
    const segs = parseSegments('para one\n\npara two');
    expect(segs).toHaveLength(1);
    expect(segs[0]).toEqual({ kind: 'md', text: 'para one\n\npara two' });
  });

  it('treats an unterminated fence as a complete segment (mid-stream)', () => {
    expect(parseSegments('intro\n```py\nprint(1)')).toEqual([
      { kind: 'md', text: 'intro' },
      { kind: 'code', lang: 'py', code: 'print(1)' },
    ]);
  });

  it('handles tilde fences and longer-than-3 fences', () => {
    expect(parseSegments('~~~\nraw\n~~~')).toEqual([{ kind: 'code', lang: '', code: 'raw' }]);
    expect(parseSegments('````md\n```\n````')).toEqual([{ kind: 'code', lang: 'md', code: '```' }]);
  });

  it('returns fresh objects on every call', () => {
    // Load-bearing for the ```html → live-artifact upgrade: when a turn
    // finalizes, ChatMessage re-parses with reuse disabled and relies on the
    // all-new references to make <For> recreate every row (and so re-evaluate
    // the `!streaming` branch). Memoizing inside parseSegments would silently
    // strand a finished artifact as a code block.
    const a = parseSegments('same text');
    const b = parseSegments('same text');
    expect(a).toEqual(b);
    expect(a[0]).not.toBe(b[0]);
  });
});

describe('sameSeg', () => {
  it('compares content, not identity', () => {
    expect(sameSeg({ kind: 'md', text: 'a' }, { kind: 'md', text: 'a' })).toBe(true);
    expect(sameSeg({ kind: 'md', text: 'a' }, { kind: 'md', text: 'b' })).toBe(false);
  });

  it('separates kinds even when the payload matches', () => {
    const code: Segment = { kind: 'code', lang: 'html', code: '<p/>' };
    const html: Segment = { kind: 'html', lang: 'html', code: '<p/>' };
    expect(sameSeg(code, html)).toBe(false);
  });

  it('compares lang as well as code', () => {
    expect(sameSeg(
      { kind: 'code', lang: 'ts', code: 'x' },
      { kind: 'code', lang: 'js', code: 'x' },
    )).toBe(false);
  });
});

describe('reuseSettledSegments', () => {
  it('hands back the previous object for each unchanged leading segment', () => {
    const src = 'prose\n```js\nx\n```\nmore';
    const prev = parseSegments(src);
    const next = reuseSettledSegments(prev, parseSegments(src));
    expect(next).toHaveLength(3);
    expect(next[0]).toBe(prev[0]);
    expect(next[1]).toBe(prev[1]);
    expect(next[2]).toBe(prev[2]);
  });

  it('keeps a settled paragraph reference stable across a streaming answer', () => {
    // Three flushes of one growing answer. The finished prose above the fence
    // must be the SAME object every time (so <For> keeps its row and
    // marked/DOMPurify never re-run on it); only the growing tail is new.
    let prev: Segment[] = [];
    const flush = (text: string) => {
      prev = reuseSettledSegments(prev, parseSegments(text));
      return prev;
    };
    const f1 = flush('The answer is:\n```bash\nls');
    const settled = f1[0];
    const tail1 = f1[1];

    const f2 = flush('The answer is:\n```bash\nls -l');
    expect(f2[0]).toBe(settled);
    expect(f2[1]).not.toBe(tail1);

    const f3 = flush('The answer is:\n```bash\nls -l\n```\nDone.');
    expect(f3[0]).toBe(settled);
    expect(f3).toHaveLength(3);
  });

  it('stops at the first difference so a changed segment is never stale', () => {
    const prev = parseSegments('one\n```js\nx\n```\ntail');
    const next = reuseSettledSegments(prev, parseSegments('ONE\n```js\nx\n```\ntail'));
    expect(next[0]).not.toBe(prev[0]);
    expect(next[0]).toEqual({ kind: 'md', text: 'ONE' });
    // Segments 1 and 2 match by content but sit past the mismatch, so they are
    // rebuilt rather than reused — reuse must never jump a difference.
    expect(next[1]).not.toBe(prev[1]);
    expect(next[2]).not.toBe(prev[2]);
  });

  it('stops when a fence changes kind mid-stream', () => {
    // `code` → `html` is exactly the transition that must reach the artifact
    // branch; reusing the code-kind object across it would strand it.
    const prev: Segment[] = [{ kind: 'code', lang: 'htm', code: '<p/>' }];
    const next = reuseSettledSegments(prev, [{ kind: 'html', lang: 'html', code: '<p/>' }]);
    expect(next[0]).not.toBe(prev[0]);
    expect(next[0].kind).toBe('html');
  });

  it('never produces a duplicate reference', () => {
    // <For> requires unique items; two byte-identical code blocks must not
    // collapse onto one object.
    const src = '```js\nsame\n```\ntext\n```js\nsame\n```';
    const prev = parseSegments(src);
    expect(prev).toHaveLength(3);
    const next = reuseSettledSegments(prev, parseSegments(src));
    expect(new Set(next).size).toBe(next.length);
  });

  it('tolerates an empty previous list and a shorter new list', () => {
    expect(reuseSettledSegments([], parseSegments('a'))).toEqual([{ kind: 'md', text: 'a' }]);
    const prev = parseSegments('a\n```js\nx\n```\nb');
    expect(prev).toHaveLength(3);
    const next = reuseSettledSegments(prev, parseSegments('a\n```js\nx'));
    expect(next).toHaveLength(2);
    expect(next[0]).toBe(prev[0]);
    expect(next[1]).toBe(prev[1]);
  });
});

describe('the suggested next message block', () => {
  it('is hidden from the transcript, closed or still streaming', () => {
    expect(parseSegments('Done.\n\n```hollow-next\nShip it\n```')).toEqual([{ kind: 'md', text: 'Done.\n' }]);
    expect(parseSegments('Done.\n```hollow-next\nShi')).toEqual([{ kind: 'md', text: 'Done.' }]);
    expect(parseSegments('Done.\n```hollow-ne')).toEqual([{ kind: 'md', text: 'Done.' }]);
    expect(parseSegments('```hollow\nx\n```')).toEqual([{ kind: 'code', lang: 'hollow', code: 'x' }]);
  });

  it('splits out of the text for copying, leaving other fences whole', () => {
    expect(splitNextFence('Done.\n\n```hollow-next\nShip it\n```\n')).toEqual({ text: 'Done.', blocks: [['Ship it']] });
    const quoted = 'Example:\n````md\n```hollow-next\nShip it\n```\n````';
    expect(splitNextFence(quoted)).toEqual({ text: quoted, blocks: [] });
    expect(splitNextFence('plain')).toEqual({ text: 'plain', blocks: [] });
  });
});

describe('hidden blocks: the golden cases Rust shares (Canvas C6 P1)', () => {
  const golden = JSON.parse(HIDDEN) as { cases: Array<{ name: string; lang: string; text: string; expect: { text: string; blocks: string[][] } }> };
  it('has the cases', () => expect(golden.cases.length).toBeGreaterThan(12));
  for (const c of golden.cases) {
    it(c.name, () => expect(splitHiddenFence(c.text, c.lang)).toEqual(c.expect));
  }
});

describe('hollow-canvas fences (Canvas C3)', () => {
  const body = '<hc-canvas id="release-status" title="Release status">\n  <p>29 checks</p>\n</hc-canvas>';

  it('are a visible canvas segment that knows whether the fence has closed', () => {
    expect(parseSegments(`Status:\n\`\`\`hollow-canvas\n${body}\n\`\`\`\nDone.`)).toEqual([
      { kind: 'md', text: 'Status:' },
      { kind: 'canvas', lang: 'hollow-canvas', code: body, closed: true },
      { kind: 'md', text: 'Done.' },
    ]);
    expect(parseSegments(`Status:\n~~~HOLLOW-CANVAS\n<hc-canvas id="a"`)).toEqual([
      { kind: 'md', text: 'Status:' },
      { kind: 'canvas', lang: 'hollow-canvas', code: '<hc-canvas id="a"', closed: false },
    ]);
  });

  it('never flash as a code block while the language is still being typed', () => {
    expect(parseSegments('Status:\n```hollow-canv')).toEqual([
      { kind: 'md', text: 'Status:' },
      { kind: 'canvas', lang: 'hollow-canvas', code: '', closed: false },
    ]);
    // A finished fence with a near-miss language is just code.
    expect(parseSegments('```hollow-canv\nx\n```')).toEqual([{ kind: 'code', lang: 'hollow-canv', code: 'x' }]);
    // The hidden fences' own typing hold still wins for their prefixes.
    expect(parseSegments('Done.\n```hollow-')).toEqual([{ kind: 'md', text: 'Done.' }]);
  });

  it('change identity when the fence closes, so the row moves from drawing to ready', () => {
    const open: Segment = { kind: 'canvas', lang: 'hollow-canvas', code: body, closed: false };
    const closed: Segment = { kind: 'canvas', lang: 'hollow-canvas', code: body, closed: true };
    expect(sameSeg(open, closed)).toBe(false);
    expect(sameSeg(closed, { ...closed })).toBe(true);
    const next = reuseSettledSegments([open], parseSegments(`\`\`\`hollow-canvas\n${body}\n\`\`\``));
    expect(next[0]).not.toBe(open);
  });

  it('a finished canvas keeps its row when the turn settles; everything else is rebuilt', () => {
    const src = `Intro\n\`\`\`hollow-canvas\n${body}\n\`\`\`\n\`\`\`html\n<p>x</p>\n\`\`\``;
    const prev = parseSegments(src);
    const next = reuseFinishedCanvases(prev, parseSegments(src));
    expect(next[0]).not.toBe(prev[0]);
    expect(next[1]).toBe(prev[1]);
    expect(next[2]).not.toBe(prev[2]);
    // An unfinished canvas is rebuilt, and nothing jumps an index.
    const streaming = parseSegments(`\`\`\`hollow-canvas\n${body}`);
    expect(reuseFinishedCanvases(streaming, parseSegments(`\`\`\`hollow-canvas\n${body}`))[0]).not.toBe(streaming[0]);
    expect(reuseFinishedCanvases(prev, parseSegments('Intro'))).toHaveLength(1);
  });
});

describe('an old whiteboard fence (whiteboards removed 2026-10-07)', () => {
  it('shows as a one-line notice and its JSON source, never drawn and never lost', () => {
    const board = '{"version":2,"title":"Flow","elements":[{"id":"a","type":"rect","label":"A"}]}';
    for (const fence of ['```', '~~~']) {
      const segs = parseSegments(`Before.\n\n${fence}hollow-whiteboard\n${board}\n${fence}\nAfter.`);
      expect(segs).toEqual([
        { kind: 'md', text: 'Before.\n' },
        { kind: 'md', text: RETIRED_BOARD_NOTE },
        { kind: 'code', lang: 'json', code: board },
        { kind: 'md', text: 'After.' },
      ]);
    }
    expect(RETIRED_BOARD_FENCE).toBe('hollow-whiteboard');
    expect(RETIRED_BOARD_NOTE).toMatch(/removed/);
  });
});
