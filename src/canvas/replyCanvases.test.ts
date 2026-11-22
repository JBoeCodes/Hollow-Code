import { describe, expect, it } from 'vitest';
import { parseSegments } from '../chatSegments';
import { utf8Length } from './fence';
import golden from './replyCanvases.golden.json';

/** Canvas C3 wave 2 C4: the receipt's canvas count and bytes per reply
 *  (`canvas_fence::reply_canvases` in Rust) are what the chat itself shows:
 *  the canvas segments `parseSegments` finds. One golden pins both. */
describe('reply canvases (the receipt count)', () => {
  it('matches the chat segmenter on every golden case', () => {
    expect(golden.cases.length).toBeGreaterThanOrEqual(10);
    for (const c of golden.cases) {
      const canvases = parseSegments(c.text).filter(s => s.kind === 'canvas');
      const bytes = canvases.reduce((n, s) => n + (s.kind === 'canvas' ? utf8Length(s.code) : 0), 0);
      expect({ name: c.name, count: canvases.length, bytes }).toEqual({ name: c.name, count: c.count, bytes: c.bytes });
    }
  });
});
