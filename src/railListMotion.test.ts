import { afterEach, describe, expect, it, vi } from 'vitest';
import { observeRailListMotion } from './railListMotion';

const policy = vi.hoisted(() => ({ change: (_duration: number) => {}, stop: vi.fn() }));
vi.mock('./motionKit', () => ({ observeKitMotion: (_root: HTMLElement, change: (duration: number) => void) => { policy.change = change; change(0); return policy.stop; } }));
afterEach(() => { vi.unstubAllGlobals(); vi.clearAllMocks(); });

function harness() {
  let update!: () => void;
  const disconnect = vi.fn();
  vi.stubGlobal('MutationObserver', class { constructor(cb: () => void) { update = cb; } observe() {} disconnect = disconnect; });
  const row = (key: string, top: number) => ({
    dataset: { railMotionKey: key }, top, visible: true,
    getClientRects() { return this.visible ? [{}] : []; },
    getBoundingClientRect() { return { top: this.top, left: 0 }; },
    animate: vi.fn(() => ({ cancel: vi.fn(), onfinish: null as (() => void) | null, oncancel: null as (() => void) | null })),
  });
  const a = row('s-a', 0), b = row('s-b', 40);
  let rows = [a, b];
  const root = { scrollTop: 0, scrollLeft: 0, querySelectorAll: () => rows };
  const stop = observeRailListMotion(root as unknown as HTMLElement);
  return { a, b, row, root, stop, disconnect, update: () => update(), rows: (value: typeof rows) => { rows = value; }, duration: policy.change };
}

describe('sidebar list motion', () => {
  it('keeps first layout still and animates a reorder by stable identity', () => {
    const h = harness(); h.duration(180);
    expect(h.a.animate).not.toHaveBeenCalled();
    h.a.top = 40; h.b.top = 0; h.update();
    expect(h.a.animate).toHaveBeenCalledWith([{ transform: 'translate(0px, -40px)' }, { transform: 'translate(0, 0)' }], { duration: 180, easing: 'ease-out' });
    h.stop(); expect(h.disconnect).toHaveBeenCalledOnce();
  });
  it('moves recreated rows between shelves without cloning interactive content', () => {
    const h = harness(); h.duration(160);
    const recreated = h.row('s-a', 120); h.rows([h.b, recreated]); h.update();
    expect(recreated.animate).toHaveBeenCalledWith([{ transform: 'translate(0px, -120px)' }, { transform: 'translate(0, 0)' }], { duration: 160, easing: 'ease-out' });
    expect(h.a.animate).not.toHaveBeenCalled(); h.stop();
  });
  it('fades newly visible rows and cancels all owned work when policy disables motion', () => {
    const h = harness(); h.duration(200);
    const added = h.row('s-c', 80); h.rows([h.a, h.b, added]); h.update();
    expect(added.animate).toHaveBeenCalledWith([{ opacity: 0 }, { opacity: 1 }], { duration: 200, easing: 'ease-out' });
    h.duration(0);
    expect(added.animate.mock.results[0].value.cancel).toHaveBeenCalledOnce();
    h.a.top = 80; h.update(); expect(h.a.animate).not.toHaveBeenCalled();
    h.stop(); h.duration(0); expect(policy.stop).toHaveBeenCalledOnce();
  });
  it('ignores scrolling and does not animate hidden rows', () => {
    const h = harness(); h.duration(200);
    h.root.scrollTop = 40; h.a.top -= 40; h.b.top -= 40; h.update();
    expect(h.a.animate).not.toHaveBeenCalled(); expect(h.b.animate).not.toHaveBeenCalled();
    h.a.visible = false; h.a.top = 200; h.update(); expect(h.a.animate).not.toHaveBeenCalled(); h.stop();
  });
  it('leaves layout usable when animation is unavailable and ignores late updates', () => {
    const h = harness(); h.duration(200);
    h.a.animate.mockImplementation(() => { throw new Error('unavailable'); });
    h.a.top = 90; expect(h.update).not.toThrow();
    h.stop(); h.b.top = 500; h.update(); expect(h.b.animate).not.toHaveBeenCalled();
  });
});
