import { describe, expect, it } from 'vitest';
import { CANVAS_FOCUS_PAD, createCanvasFocus, replyCanvas, type FocusView } from './canvasFocus';

/** A scroller of `content` px showing `client` px, with one canvas at
 *  `canvasAt` (content offset) of `canvasH` px, or none. Smooth scrolls land
 *  at once; `smooth` records how the last one was asked for. */
function fakeView(o: { content: number; client: number; canvasAt: number | null; canvasH?: number }) {
  const v = {
    top: Math.max(0, o.content - o.client),
    content: o.content,
    client: o.client,
    canvasAt: o.canvasAt as number | null,
    canvasH: o.canvasH ?? 300,
    smooth: null as boolean | null,
  };
  const view: FocusView = {
    scrollTop: () => v.top,
    setScrollTop: (top, smooth) => { v.top = top; v.smooth = smooth; },
    clientHeight: () => v.client,
    scrollHeight: () => v.content,
    canvasBox: () => (v.canvasAt === null ? null : { top: v.canvasAt - v.top, bottom: v.canvasAt + v.canvasH - v.top }),
  };
  return { v, view };
}

const canvasReply = (id = 'release-status', title = 'Release status') =>
  `Here it is.\n\n\`\`\`hollow-canvas\n<hc-canvas id="${id}" title="${title}">\n<p>x</p>\n</hc-canvas>\n\`\`\`\n\nThe closing prose.`;

describe('replyCanvas: the last canvas of the latest reply', () => {
  it('finds the last canvas after the last user turn, with its title', () => {
    const turns = [
      { role: 'user', text: 'one' },
      { role: 'assistant', text: canvasReply('old', 'Old') },
      { role: 'user', text: 'two' },
      { role: 'assistant', text: canvasReply('first', 'First') },
      { role: 'assistant', text: canvasReply('second', 'Second') },
      { role: 'assistant', text: 'Done.' },
    ];
    expect(replyCanvas(turns)).toEqual({ turnIndex: 4, title: 'Second' });
  });
  it('ignores a canvas from before the run that just ended (a failed send leaves only a notice)', () => {
    const turns = [
      { role: 'user', text: 'one' }, { role: 'assistant', text: canvasReply() },
      // The send of "two" failed: its message was taken back and a notice posted.
      { role: 'system', text: 'Not sent: the agent is offline.' },
    ];
    expect(replyCanvas(turns)).toEqual({ turnIndex: 1, title: 'Release status' });
    expect(replyCanvas(turns, 2)).toBeNull();
    // A reply that streamed in this run still counts.
    expect(replyCanvas([...turns, { role: 'user', text: 'three' }, { role: 'assistant', text: canvasReply('n', 'New') }], 3)?.title).toBe('New');
  });
  it('is null for a reply without a canvas, even when an earlier reply had one', () => {
    expect(replyCanvas([
      { role: 'user', text: 'one' }, { role: 'assistant', text: canvasReply() },
      { role: 'user', text: 'two' }, { role: 'assistant', text: 'Plain text.\n\n```ts\nconst a = 1;\n```' },
    ])).toBeNull();
    expect(replyCanvas([])).toBeNull();
  });
  it('counts an ask-canvas and a broken (unclosed) canvas fence', () => {
    const ask = 'Pick one:\n\n```hollow-canvas\n<hc-canvas id="pick" title="Pick" ask>\n<hc-ask></hc-ask>\n</hc-canvas>\n```';
    expect(replyCanvas([{ role: 'user', text: 'q' }, { role: 'assistant', text: ask }])).toEqual({ turnIndex: 1, title: 'Pick' });
    const cut = 'Here:\n\n```hollow-canvas\n<hc-canvas id="cut" title="Cut">\n<p>x';
    expect(replyCanvas([{ role: 'user', text: 'q' }, { role: 'assistant', text: cut }])?.turnIndex).toBe(1);
  });
});

describe('createCanvasFocus: keep a finished turn’s canvas in view', () => {
  it('a finished turn with a canvas leaves the canvas top in view, not the bottom', () => {
    const { v, view } = fakeView({ content: 3000, client: 800, canvasAt: 1600, canvasH: 400 });
    const f = createCanvasFocus(view);
    expect(f.turnEnded({ hasCanvas: true, following: true })).toBe('anchor');
    expect(f.held()).toBe(true);
    expect(f.anchor(true)).toBe(true);
    expect(v.top).toBe(1600 - CANVAS_FOCUS_PAD);
    expect(v.smooth).toBe(true);
    const box = view.canvasBox()!;
    expect(box.top).toBe(CANVAS_FOCUS_PAD);
    expect(f.chip()).toBe(false);
  });

  it('a turn without a canvas keeps today’s behaviour (the caller scrolls to the bottom)', () => {
    const { v, view } = fakeView({ content: 3000, client: 800, canvasAt: null });
    const f = createCanvasFocus(view);
    expect(f.turnEnded({ hasCanvas: false, following: true })).toBe('follow');
    expect(f.held()).toBe(false);
    expect(v.top).toBe(2200);
  });

  it('tall canvases align their top, never their centre', () => {
    const { v, view } = fakeView({ content: 5000, client: 800, canvasAt: 1000, canvasH: 2400 });
    const f = createCanvasFocus(view);
    f.turnEnded({ hasCanvas: true, following: true });
    f.anchor(false);
    expect(v.top).toBe(1000 - CANVAS_FOCUS_PAD);
  });

  it('late layout shifts re-anchor while held', () => {
    const { v, view } = fakeView({ content: 3000, client: 800, canvasAt: 1600 });
    const f = createCanvasFocus(view);
    f.turnEnded({ hasCanvas: true, following: true });
    f.anchor(false);
    // A frame above settles 220 px taller: the canvas moves down.
    v.content += 220; v.canvasAt = 1820;
    f.resized();
    expect(view.canvasBox()!.top).toBe(CANVAS_FOCUS_PAD);
    expect(v.smooth).toBe(false);
  });

  it('a user scroll releases the anchor, never fights them, and shows the chip until the canvas is back', () => {
    const { v, view } = fakeView({ content: 3000, client: 800, canvasAt: 1600 });
    const f = createCanvasFocus(view);
    f.turnEnded({ hasCanvas: true, following: true });
    f.anchor(false);
    f.userScrolled();
    expect(f.held()).toBe(false);
    // They scroll to the end; the canvas leaves the view.
    v.top = 2200; f.scrolled();
    expect(f.chip()).toBe(true);
    // A late shift no longer moves them.
    v.canvasAt = 1700; v.content += 100;
    f.resized();
    expect(v.top).toBe(2200);
    // The chip brings it back and holds it again.
    f.backToCanvas(false);
    expect(view.canvasBox()!.top).toBe(CANVAS_FOCUS_PAD);
    expect(f.held()).toBe(true);
    expect(f.chip()).toBe(false);
    // Scrolled away and back by hand: the chip hides once it is in view.
    f.userScrolled(); v.top = 2300; f.scrolled();
    expect(f.chip()).toBe(true);
    v.top = 1500; f.scrolled();
    expect(f.chip()).toBe(false);
  });

  it('the next send releases the anchor and the chip', () => {
    const { v, view } = fakeView({ content: 3000, client: 800, canvasAt: 1600 });
    const f = createCanvasFocus(view);
    f.turnEnded({ hasCanvas: true, following: true });
    f.anchor(false);
    f.released();
    expect(f.held()).toBe(false);
    v.canvasAt = 1900; f.resized();
    expect(v.top).toBe(1600 - CANVAS_FOCUS_PAD);
    v.top = 2200; f.scrolled();
    expect(f.chip()).toBe(false);
  });

  it('someone who scrolled away mid-stream is not moved; the chip offers the canvas instead', () => {
    const { v, view } = fakeView({ content: 3000, client: 800, canvasAt: 1600 });
    v.top = 100;
    const f = createCanvasFocus(view);
    expect(f.turnEnded({ hasCanvas: true, following: false })).toBe('stay');
    expect(f.held()).toBe(false);
    expect(v.top).toBe(100);
    expect(f.chip()).toBe(true);
  });

  it('a canvas not rendered yet keeps the hold for a retry; abandoning it lets go', () => {
    const { v, view } = fakeView({ content: 3000, client: 800, canvasAt: null });
    const f = createCanvasFocus(view);
    f.turnEnded({ hasCanvas: true, following: true });
    expect(f.anchor(false)).toBe(false);
    expect(f.held()).toBe(true);
    // Before the first anchor lands, size changes and scrolls don't move or release it.
    f.resized(); f.scrolled();
    expect(v.top).toBe(2200);
    expect(f.held()).toBe(true);
    v.canvasAt = 1600;
    expect(f.anchor(false)).toBe(true);
    expect(view.canvasBox()!.top).toBe(CANVAS_FOCUS_PAD);
    f.abandon();
    expect(f.held()).toBe(false);
    expect(f.chip()).toBe(false);
  });

  it('clamps to the scroll range near the end of the transcript', () => {
    const { v, view } = fakeView({ content: 1000, client: 800, canvasAt: 900, canvasH: 80 });
    const f = createCanvasFocus(view);
    f.turnEnded({ hasCanvas: true, following: true });
    f.anchor(false);
    expect(v.top).toBe(200);
  });

  it('a scroll nobody announced (scroll chaining out of a frame) releases a held anchor', () => {
    const { v, view } = fakeView({ content: 3000, client: 800, canvasAt: 1600 });
    const f = createCanvasFocus(view);
    f.turnEnded({ hasCanvas: true, following: true });
    f.anchor(false);
    // A few px of jitter is not a scroll of theirs.
    v.top += 10; f.scrolled();
    expect(f.held()).toBe(true);
    v.top = 2200; f.scrolled();
    expect(f.held()).toBe(false);
    expect(f.chip()).toBe(true);
    v.canvasAt = 1700; f.resized();
    expect(v.top).toBe(2200);
  });

  it('a smooth anchor on its way is not mistaken for the person scrolling', () => {
    const { v, view } = fakeView({ content: 3000, client: 800, canvasAt: 1600 });
    const f = createCanvasFocus(view);
    f.turnEnded({ hasCanvas: true, following: true });
    // The smooth scroll starts from the bottom and reports intermediate positions.
    view.setScrollTop = (top, smooth) => { v.smooth = smooth; void top; v.top = 2000; };
    f.anchor(true);
    f.scrolled();
    expect(f.held()).toBe(true);
    v.top = 1600 - CANVAS_FOCUS_PAD; f.scrolled();
    expect(f.held()).toBe(true);
    // Arrived: now a big move is the person's.
    v.top = 2200; f.scrolled();
    expect(f.held()).toBe(false);
  });

  it('a size change during a smooth move waits for it to arrive, then catches up', () => {
    const { v, view } = fakeView({ content: 3000, client: 800, canvasAt: 1600 });
    const f = createCanvasFocus(view);
    f.turnEnded({ hasCanvas: true, following: true });
    let moves = 0;
    const set = view.setScrollTop;
    view.setScrollTop = (top, smooth) => { moves++; if (smooth) { v.smooth = true; v.top = 2000; } else set(top, smooth); };
    f.anchor(true);
    v.canvasAt = 1700; v.content += 100;
    f.resized();
    expect(moves).toBe(1);
    expect(v.top).toBe(2000);
    f.settle();
    expect(view.canvasBox()!.top).toBe(CANVAS_FOCUS_PAD);
    expect(f.held()).toBe(true);
  });

  it('a canvas briefly missing on a resize (skeleton swapping for the frame) keeps the anchor', () => {
    const { v, view } = fakeView({ content: 3000, client: 800, canvasAt: 1600 });
    const f = createCanvasFocus(view);
    f.turnEnded({ hasCanvas: true, following: true });
    f.anchor(false);
    v.canvasAt = null; f.resized();
    expect(f.held()).toBe(true);
    v.canvasAt = 1650; f.resized();
    expect(view.canvasBox()!.top).toBe(CANVAS_FOCUS_PAD);
  });
});
