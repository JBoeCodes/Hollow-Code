import { describe, expect, it } from 'vitest';
import { chatCanvasView, canvasFixRequest } from './chatCanvas';
import { parseCanvasFence } from './fence';
import { escapeRowTags, unclosedCause, unclosedReason, ROW_ELEMENTS } from './strayTags';

const canvas = (inner: string, attrs = '') => `<hc-canvas id="c" title="C"${attrs}>\n${inner}\n</hc-canvas>`;
const RAW = ['title', 'textarea', 'style', 'xmp', 'noembed'];

describe('escapeRowTags: a stray < in a data row is text (overnight A2)', () => {
  it('the 10-11 20:54 case: an ask-canvas matrix row with <title> renders', () => {
    const body = canvas('<hc-ask title="Check" send="Send">\n<hc-matrix name="m" label="Checks" options="Pass | Fail">\nBack to canvas: <title>\nChip by keyboard\n</hc-matrix>\n</hc-ask>', ' ask');
    expect(parseCanvasFence(body, true).state).toBe('invalid');
    const fixed = escapeRowTags(body);
    expect(fixed).toContain('Back to canvas: &lt;title>');
    const p = parseCanvasFence(fixed, true);
    expect(p.state).toBe('ready');
    expect(p.state === 'ready' && p.inner).toContain('Chip by keyboard');
  });

  for (const name of RAW) {
    it(`<${name}> inside every data-row element stays text`, () => {
      for (const row of ROW_ELEMENTS) {
        const body = canvas(`<${row} label="L">\nA <${name}> row: 1\nB: 2\n</${row}>\n<p>after</p>`);
        const fixed = escapeRowTags(body);
        expect(fixed, row).toContain(`A &lt;${name}> row: 1`);
        expect(parseCanvasFence(fixed, true).state, row).toBe('ready');
      }
    });
  }

  it('a comment opener and an unknown <name> inside rows are text', () => {
    const body = canvas('<hc-rank name="r" label="Order">\nShip <!-- later\nDocs <draft> first\n</hc-rank>');
    const fixed = escapeRowTags(body);
    expect(fixed).toContain('Ship &lt;!-- later');
    expect(fixed).toContain('Docs &lt;draft> first');
    expect(parseCanvasFence(fixed, true).state).toBe('ready');
  });

  it('keeps the tags rows may use: a table, list items, inline phrasing, kit elements', () => {
    const body = canvas('<hc-table label="T"><table><tr><th>A</th></tr><tr><td><b>1</b> &lt;ok</td></tr></table></hc-table>\n<hc-choice name="c" label="Q"><ul><li>One</li><li><em>Two</em></li></ul></hc-choice>');
    expect(escapeRowTags(body)).toBe(body);
  });

  it('keeps images, icons, headings and option elements authors put in rows (review)', () => {
    const body = canvas('<hc-table label="T"><table><tr><td><img src="data:image/png;base64,AA" alt="x"><svg viewBox="0 0 8 8"><path d="M0 0"/></svg></td><td><h3>Hi</h3><pre>a</pre><hr></td></tr></table></hc-table>\n<hc-choice name="c" label="Q">\n<option>A</option>\n<option>B</option>\n</hc-choice>');
    expect(escapeRowTags(body)).toBe(body);
  });

  it('a row element left open closes with its parent, so nothing after it is escaped (review)', () => {
    const body = canvas('<hc-tabs label="T"><div label="A"><hc-chart label="C">\nA: 1\n</div><div label="B"><h3>Notes</h3><style>p{}</style><img src="data:," alt=""></div></hc-tabs>');
    expect(escapeRowTags(body)).toBe(body);
    // A stray end tag that closes nothing stays text.
    expect(escapeRowTags(canvas('<hc-bars label="B">\nA </title> 1: 2\n</hc-bars>'))).toContain('A &lt;/title> 1: 2');
  });

  it('hc-code and hc-file are text: every < but their own end tag shows as written (review)', () => {
    const body = canvas('<hc-code lang="html"><div class="x"><hc-chart>A: 1</hc-chart></div></hc-code>\n<hc-file path="a.md">a <b>.md</hc-file>');
    const fixed = escapeRowTags(body);
    expect(fixed).toContain('<hc-code lang="html">&lt;div class="x">&lt;hc-chart>A: 1&lt;/hc-chart>&lt;/div></hc-code>');
    expect(fixed).toContain('a &lt;b>.md</hc-file>');
    expect(parseCanvasFence(fixed, true).state).toBe('ready');
  });

  it('escapes a <title> inside a table cell of an hc-table', () => {
    const body = canvas('<hc-table label="T"><table><tr><td>Use <title> here</td></tr></table></hc-table>');
    expect(escapeRowTags(body)).toContain('<td>Use &lt;title> here</td>');
  });

  it('leaves everything outside data rows alone, including look-alikes in scripts and comments', () => {
    const body = canvas('<p>a <b>b</b></p>\n<script>const s = "<hc-chart><title>";</script>\n<!-- <hc-bars> <title> -->\n<hc-callout title="x">Keep <i>this</i></hc-callout>');
    expect(escapeRowTags(body)).toBe(body);
  });

  it('is idempotent, keeps a row text with < and numbers, and leaves the row element attributes alone', () => {
    const body = canvas('<hc-bars label="a <b> c" unit="%">\nUnder <5: 12\nArrow a <- b: 3\n</hc-bars>');
    const once = escapeRowTags(body);
    expect(once).toContain('label="a <b> c"');
    expect(once).toContain('Under &lt;5: 12');
    expect(once).toContain('Arrow a &lt;- b: 3');
    expect(escapeRowTags(once)).toBe(once);
  });

  it('while streaming, a tag name cut off at the end waits for the next flush', () => {
    const body = '<hc-canvas id="c" title="C">\n<hc-pick name="p" label="P">\nWeek <tit';
    expect(escapeRowTags(body)).toBe(body);
    expect(escapeRowTags(`${body}le>`)).toContain('Week &lt;title>');
  });

  it('returns the same string when there is no data-row element or raw-text tag', () => {
    const body = canvas('<p>Hi <b>there</b></p>');
    expect(escapeRowTags(body)).toBe(body);
  });

  it('a bare <title> outside rows is a word once the canvas has its end (PER-1)', () => {
    for (const name of ['title', 'textarea', 'xmp', 'plaintext']) {
      const body = canvas(`<p>Back to canvas: <${name}></p>\n<hc-callout title="x">Still here</hc-callout>`);
      const fixed = escapeRowTags(body);
      expect(fixed, name).toContain(`Back to canvas: &lt;${name}></p>`);
      expect(chatCanvasView(fixed, true).state, name).toBe('ready');
      expect(escapeRowTags(fixed)).toBe(fixed);
    }
    // A closed one is an element, and a streaming fence waits for its end.
    const closed = canvas('<style>p { color: red }</style>');
    expect(escapeRowTags(closed)).toBe(closed);
    const streaming = '<hc-canvas id="c" title="C">\n<p><title>';
    expect(escapeRowTags(streaming)).toBe(streaming);
  });
});

describe('a canvas that never reaches </hc-canvas> says why (overnight A2)', () => {
  for (const name of RAW) {
    it(`<${name}> outside rows: which tag, which line, the fix`, () => {
      const body = canvas(`<p>One</p>\n<p>Back to canvas: <${name}></p>`);
      expect(unclosedCause(body)).toEqual({ kind: 'raw', name, line: 3 });
      const v = chatCanvasView(body, true);
      expect(v.state).toBe('invalid');
      const reason = v.state === 'invalid' ? v.reason : '';
      expect(reason).toContain(`Line 3 of the canvas: <${name}> is never closed`);
      expect(reason).toContain(`write &lt;${name}&gt; or put it in words`);
      // "Ask to fix" carries it.
      expect(canvasFixRequest(v.state === 'invalid' ? v.root : null, reason)).toContain(`<${name}> is never closed`);
    });
  }

  it('an open comment, <plaintext>, a declaration and a tag whose quote never closes', () => {
    expect(unclosedCause(canvas('<p>a</p>\n<!-- note'))).toEqual({ kind: 'comment', line: 3 });
    expect(unclosedCause(canvas('<plaintext>x'))).toEqual({ kind: 'plaintext', line: 2 });
    expect(unclosedCause(canvas('<!DOCTYPE x'))).toEqual({ kind: 'declaration', line: 2 });
    expect(unclosedCause(canvas('<div class="a>x</div>'))).toEqual({ kind: 'tag', name: 'div', line: 2 });
    expect(unclosedCause(canvas('<p>a</p>\n<p class="x"\n'))).toEqual({ kind: 'tag', name: 'p', line: 3 });
    // A raw-text element closed only after the root's end tag still hid it.
    expect(unclosedCause(`${canvas('<title>x')}\n</title>`)).toEqual({ kind: 'raw', name: 'title', line: 2 });
    for (const c of [{ kind: 'comment', line: 3 }, { kind: 'plaintext', line: 2 }, { kind: 'declaration', line: 2 }, { kind: 'tag', name: 'div', line: 2 }] as const) {
      expect(unclosedReason(c)).toMatch(/^Line \d of the canvas: /);
    }
  });

  it('a canvas cut off mid-construct (Stop, a dropped fence) keeps the plain reason (review)', () => {
    for (const body of ['<hc-canvas id="a" title="A">\n<script>const x = 1;', '<hc-canvas id="a" title="A">\n<hc-stat label="Synced', '<hc-canvas id="a" title="A">\n<!-- note', '<hc-canvas id="a" title="A">\n<title>x']) {
      expect(unclosedCause(body), body).toBeNull();
      const v = chatCanvasView(body, true);
      expect(v.state === 'invalid' && v.reason, body).toBe('The canvas ended before </hc-canvas>.');
    }
  });

  it('an end tag missing its > names itself without blaming a quote', () => {
    const cause = unclosedCause(canvas('<p>a</p\n'));
    expect(cause).toEqual({ kind: 'tag', name: '/p', line: 2 });
    expect(unclosedReason(cause!)).toContain('the </p> tag never ends (its > is missing)');
    expect(unclosedReason({ kind: 'declaration', line: 2 })).not.toContain('"');
  });

  it('a canvas simply missing its end tag keeps the plain reason', () => {
    const body = '<hc-canvas id="c" title="C">\n<p>hi</p>';
    expect(unclosedCause(body)).toBeNull();
    const v = chatCanvasView(body, true);
    expect(v.state === 'invalid' && v.reason).toBe('The canvas ended before </hc-canvas>.');
  });

  it('a closed raw-text element is fine and other errors keep their own reason', () => {
    expect(chatCanvasView(canvas('<style>p { color: red }</style>'), true).state).toBe('ready');
    const v = chatCanvasView('<hc-canvas id="Bad" title="C"></hc-canvas>', true);
    expect(v.state === 'invalid' && v.code).toBe('bad-id');
  });

  it('while the fence streams, an open <title> is still drawing, not an error', () => {
    expect(chatCanvasView(canvas('<p><title>').replace('\n</hc-canvas>', ''), false).state).toBe('drawing');
  });
});
