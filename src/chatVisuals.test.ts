import { describe, expect, it } from 'vitest';
import { canvasVisualKey, collectVisuals, finishedCanvasTitle, htmlVisualTitle, pickVisual, visualsInText } from './chatVisuals';
import { canvasRootId, canvasTurnIndex } from './canvasTurn';

const board = (title: string) => '```hollow-whiteboard\n' + JSON.stringify({
  version: 1, title, layout: 'auto', nodes: [{ id: 'a', label: 'A' }], edges: [], steps: [],
}) + '\n```';
const html = (body: string) => '```html\n' + body + '\n```';
const preview = (title: string) => html(`<title>${title}</title><p>x</p>`);
const canvas = (id: string, title: string) => '```hollow-canvas\n' + `<hc-canvas id="${id}" title="${title}"><p>x</p></hc-canvas>` + '\n```';

describe('chat visuals', () => {
  it('finds html previews, not code, prose or an old whiteboard fence (removed 2026-10-07)', () => {
    const text = `Here it is.\n\n${board('Flow')}\n\n\`\`\`ts\nconst x = 1;\n\`\`\`\n\n${html('<title>Report &amp; notes</title><p>hi</p>')}`;
    const found = visualsInText(text);
    expect(found.map(v => [v.kind, v.title])).toEqual([['html', 'Report & notes']]);
    expect(visualsInText('no fences here')).toEqual([]);
  });

  it('keys by content, so a reload finds the same visual', () => {
    expect(visualsInText(preview('Same'))[0].key).toBe(visualsInText(`intro\n\n${preview('Same')}`)[0].key);
    expect(visualsInText(preview('One'))[0].key).not.toBe(visualsInText(preview('Two'))[0].key);
  });

  it('titles html from <title>, then <h1>, then a fallback', () => {
    expect(htmlVisualTitle('<h1 class="x">Launch plan</h1>', 'html')).toBe('Launch plan');
    expect(htmlVisualTitle('<p>plain</p>', 'html')).toBe('HTML preview');
    expect(htmlVisualTitle('<svg></svg>', 'svg')).toBe('SVG image');
  });

  it('collects settled assistant visuals only, oldest first, one per key', () => {
    const turns = [
      { role: 'user', text: preview('User pasted') },
      { role: 'assistant', text: preview('First') },
      { role: 'assistant', text: preview('Second') },
      { role: 'assistant', text: preview('First') },
      { role: 'assistant', text: preview('Streaming'), partial: true },
    ];
    expect(collectVisuals(turns).map(v => v.title)).toEqual(['Second', 'First']);
  });

  it('lists a finished, well-formed canvas under its title; a broken one stays in place only', () => {
    const found = visualsInText(`Status.\n\n${canvas('release-status', 'Release status')}\n\n${canvas('Bad Id', 'Broken')}`);
    expect(found.map(v => [v.kind, v.title])).toEqual([['canvas', 'Release status']]);
    expect(found[0].key).toBe(canvasVisualKey(found[0].code));
    expect(visualsInText('```hollow-canvas\n<hc-canvas id="a" title="A">')).toEqual([]);
  });

  it('reads a finished canvas title lightly, without the full parser', () => {
    expect(finishedCanvasTitle('\n<HC-CANVAS data-id="x" id=plan title=\'Plan &amp; risks\'>\n<p>x</p>\n</hc-canvas >\n')).toBe('Plan & risks');
    expect(finishedCanvasTitle('<hc-canvas id="plan" title="Plan"><p>x</p>')).toBeNull();
    expect(finishedCanvasTitle('<hc-canvas data-id="plan" title="Plan"></hc-canvas>')).toBeNull();
    expect(finishedCanvasTitle('<hc-canvas id="plan" title=""></hc-canvas>')).toBeNull();
    expect(finishedCanvasTitle('<p>x</p><hc-canvas id="plan" title="Plan"></hc-canvas>')).toBeNull();
  });

  it('keeps the selected visual while it exists, else shows the newest', () => {
    const visuals = collectVisuals([{ role: 'assistant', text: `${preview('A')}\n\n${preview('B')}` }]);
    expect(pickVisual(visuals, visuals[0].key)?.title).toBe('A');
    expect(pickVisual(visuals, 'gone')?.title).toBe('B');
    expect(pickVisual([], null)).toBeNull();
  });
});

describe('canvasTurnIndex (Canvas C4 A3: "waiting on you" reopens a conversation at its canvas)', () => {
  const fence = (id: string, extra = '') => `\`\`\`hollow-canvas\n<hc-canvas id="${id}" title="T"${extra}>\n<p>x</p>\n</hc-canvas>\n\`\`\``;
  const turns = [
    { role: 'user', text: 'Plan it' },
    { role: 'assistant', text: `First.\n\n${fence('release-choices', ' ask')}` },
    { role: 'user', text: fence('release-choices', ' ask') },
    { role: 'assistant', text: `Again.\n\n${fence('other')}\n\n${fence('release-choices', ' ask')}` },
    { role: 'assistant', text: `Unfinished:\n\n\`\`\`hollow-canvas\n<hc-canvas id="late" title="T">` },
    { role: 'assistant', text: '```html\n<hc-canvas id="html-only" title="T"></hc-canvas>\n```' },
  ];

  it('finds the newest assistant turn whose canvas fence has that root id', () => {
    expect(canvasTurnIndex(turns, 'release-choices')).toBe(3);
    expect(canvasTurnIndex(turns, 'other')).toBe(3);
    expect(canvasTurnIndex(turns.slice(0, 3), 'release-choices')).toBe(1);
  });

  it('skips user turns, unfinished canvases, other fences and other ids', () => {
    expect(canvasTurnIndex(turns, 'late')).toBe(-1);
    expect(canvasTurnIndex(turns, 'html-only')).toBe(-1);
    expect(canvasTurnIndex(turns, 'release')).toBe(-1);
    expect(canvasTurnIndex([], 'release-choices')).toBe(-1);
    expect(canvasRootId(`<hc-canvas id='quoted-id' title="T" ask>\n</hc-canvas>`)).toBe('quoted-id');
    expect(canvasRootId('<hc-canvas id="Bad" title="T"></hc-canvas>')).toBeNull();
  });
});
