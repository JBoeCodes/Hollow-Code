import { afterEach, describe, expect, it, vi } from 'vitest';
import css from './styles/motion-kit.css?raw';
import ui from './components/MotionKit.tsx?raw';
import { kitDuration, measureMotionSize, observeKitMotion } from './motionKit';

afterEach(() => vi.unstubAllGlobals());

function harness() {
  const system = new EventTarget() as EventTarget & { matches: boolean };
  system.matches = false;
  const forced = new EventTarget() as EventTarget & { matches: boolean };
  forced.matches = false;
  let token = '200ms', size = 40, displayed = 40;
  let intersect!: (entries: { isIntersecting: boolean }[]) => void;
  let mutate!: () => void, resize!: () => void;
  const disconnect = vi.fn();
  vi.stubGlobal('IntersectionObserver', class { constructor(cb: typeof intersect) { intersect = cb; } observe() {} disconnect = disconnect; });
  vi.stubGlobal('MutationObserver', class { constructor(cb: typeof mutate) { mutate = cb; } observe() {} disconnect = disconnect; });
  vi.stubGlobal('ResizeObserver', class { constructor(cb: typeof resize) { resize = cb; } observe() {} disconnect = disconnect; });
  const doc = Object.assign(new EventTarget(), {
    hidden: false,
    documentElement: { dataset: { motion: 'full' } },
    defaultView: {
      matchMedia: (query: string) => query.includes('forced-colors') ? forced : system,
      getComputedStyle: () => ({ getPropertyValue: (key: string) => key === '--panel-duration' ? token : 'cubic-bezier(0, 0, 0.2, 1)' }),
    },
  });
  const styles = new Map<string, string>();
  const animations: { cancel: ReturnType<typeof vi.fn>; onfinish?: () => void; oncancel?: () => void }[] = [];
  const outer = {
    ownerDocument: doc, dataset: {} as Record<string, string>,
    style: { setProperty: (key: string, value: string) => styles.set(key, value), removeProperty: (key: string) => styles.delete(key) },
    getBoundingClientRect: () => ({ height: displayed, width: displayed }),
    animate: vi.fn((_frames: Keyframe[], _options: KeyframeAnimationOptions) => { const animation = { cancel: vi.fn() }; animations.push(animation); return animation; }),
  };
  const inner = { getBoundingClientRect: () => ({ height: size, width: size }) };
  return {
    outer: outer as unknown as HTMLElement, inner: inner as unknown as HTMLElement, doc, system, forced,
    styles, animations, animate: outer.animate, disconnect,
    visible: (value = true) => intersect([{ isIntersecting: value }]),
    token: (value: string) => { token = value; mutate?.(); },
    resize: (value: number) => { size = value; resize(); },
    displayed: (value: number) => { displayed = value; },
    mutate: () => mutate(),
  };
}

describe('motion kit preference precedence', () => {
  it('reads the shared setting, with no independent default', () => {
    const h = harness();
    expect(kitDuration(h.outer)).toBe(200);
    h.token('0ms'); expect(kitDuration(h.outer)).toBe(0);
    h.token(''); expect(kitDuration(h.outer)).toBe(0);
    h.token('0.3s'); expect(kitDuration(h.outer)).toBe(300);
    h.system.matches = true; expect(kitDuration(h.outer)).toBe(0);
    h.system.matches = false; h.doc.documentElement.dataset.motion = 'reduced'; expect(kitDuration(h.outer)).toBe(0);
  });

  it('keeps cues static offscreen, hidden, forced-colour or reduced, and disconnects', () => {
    const h = harness(), changed = vi.fn();
    const stop = observeKitMotion(h.outer, changed);
    expect(h.outer.dataset.motionPlaying).toBe('false');
    h.visible(); expect(h.outer.dataset.motionPlaying).toBe('true');
    h.forced.matches = true; h.forced.dispatchEvent(new Event('change'));
    expect(h.outer.dataset.motionPlaying).toBe('false');
    h.forced.matches = false; h.forced.dispatchEvent(new Event('change'));
    h.system.matches = true; h.system.dispatchEvent(new Event('change'));
    expect(changed).toHaveBeenLastCalledWith(0);
    h.system.matches = false; h.system.dispatchEvent(new Event('change'));
    h.doc.hidden = true; h.doc.dispatchEvent(new Event('visibilitychange'));
    expect(changed).toHaveBeenLastCalledWith(0);
    h.doc.hidden = false; h.doc.dispatchEvent(new Event('visibilitychange'));
    h.visible(false); expect(changed).toHaveBeenLastCalledWith(0);
    stop(); expect(h.disconnect).toHaveBeenCalledTimes(2);
    expect(h.outer.dataset.motionPlaying).toBeUndefined();
    changed.mockClear(); h.visible(); h.mutate(); h.system.dispatchEvent(new Event('change'));
    expect(changed).not.toHaveBeenCalled(); expect(h.outer.dataset.motionPlaying).toBeUndefined();
  });
});

describe.each(['height', 'width'] as const)('measured %s lifecycle', axis => {
  it('snaps the initial measurement then animates a resize and releases clipping', () => {
    const h = harness();
    const stop = measureMotionSize(h.outer, h.inner, axis);
    h.visible(); expect(h.animate).not.toHaveBeenCalled();
    h.resize(81.2);
    expect(h.animate).toHaveBeenLastCalledWith([{ [axis]: '40px' }, { [axis]: '82px' }], { duration: axis === 'height' ? 200 : 150, easing: 'cubic-bezier(0, 0, 0.2, 1)' });
    expect(h.styles.get('overflow-clip-margin')).toBe('calc(var(--focus-offset, 2px) + 2px)');
    h.animations[0].onfinish?.(); expect(h.styles.size).toBe(0);
    stop(); expect(h.disconnect).toHaveBeenCalledTimes(3);
  });

  it('rebases interruption, ignores stale completion, and handles close/reopen', () => {
    const h = harness();
    const stop = measureMotionSize(h.outer, h.inner, axis); h.visible();
    h.resize(100); h.displayed(63); h.resize(0);
    expect(h.animations[0].cancel).toHaveBeenCalledOnce();
    expect(h.animate.mock.calls[1][0]).toEqual([{ [axis]: '63px' }, { [axis]: '0px' }]);
    h.animations[0].onfinish?.(); expect(h.styles.get(axis)).toBe('0px');
    h.displayed(24); h.resize(100);
    expect(h.animate.mock.calls[2][0]).toEqual([{ [axis]: '24px' }, { [axis]: '100px' }]);
    stop(); expect(h.animations[2].cancel).toHaveBeenCalledOnce(); expect(h.styles.size).toBe(0);
    h.resize(130); expect(h.animate).toHaveBeenCalledTimes(3);
  });

  it.each(['zero', 'system', 'app', 'hidden'] as const)('cancels immediately when %s disables motion', reason => {
    const h = harness();
    const stop = measureMotionSize(h.outer, h.inner, axis); h.visible(); h.resize(80);
    if (reason === 'zero') h.token('0ms');
    if (reason === 'system') { h.system.matches = true; h.system.dispatchEvent(new Event('change')); }
    if (reason === 'app') { h.doc.documentElement.dataset.motion = 'reduced'; h.mutate(); }
    if (reason === 'hidden') { h.doc.hidden = true; h.doc.dispatchEvent(new Event('visibilitychange')); }
    expect(h.animations[0].cancel).toHaveBeenCalledOnce(); expect(h.styles.size).toBe(0);
    h.resize(120); expect(h.animate).toHaveBeenCalledTimes(1);
    stop();
  });

  it('leaves natural dimensions without Web Animations or observers', () => {
    const h = harness();
    h.outer.animate = undefined as unknown as HTMLElement['animate'];
    const stop = measureMotionSize(h.outer, h.inner, axis); h.visible(); h.resize(90);
    expect(h.styles.size).toBe(0); stop();
    vi.stubGlobal('ResizeObserver', undefined); vi.stubGlobal('IntersectionObserver', undefined);
    measureMotionSize(h.outer, h.inner, axis)(); expect(h.styles.size).toBe(0);
  });

  it.each(['zero', 'system', 'app'] as const)('starts and stays still under %s', reason => {
    const h = harness();
    if (reason === 'zero') h.token('0ms');
    if (reason === 'system') h.system.matches = true;
    if (reason === 'app') h.doc.documentElement.dataset.motion = 'reduced';
    const stop = measureMotionSize(h.outer, h.inner, axis); h.visible(); h.resize(200);
    expect(h.animate).not.toHaveBeenCalled(); expect(h.styles.size).toBe(0); stop();
  });

  it('does not replay a hidden resize when the content becomes visible', () => {
    const h = harness();
    const stop = measureMotionSize(h.outer, h.inner, axis);
    h.resize(120); h.visible(); expect(h.animate).not.toHaveBeenCalled();
    h.resize(150); expect(h.animate.mock.calls[0][0]).toEqual([{ [axis]: '120px' }, { [axis]: '150px' }]);
    stop();
  });

  it('releases geometry when the animation engine rejects an effect', () => {
    const h = harness();
    h.animate.mockImplementation(() => { throw new Error('Unsupported easing'); });
    const stop = measureMotionSize(h.outer, h.inner, axis); h.visible();
    expect(() => h.resize(90)).not.toThrow(); expect(h.styles.size).toBe(0); stop();
  });
});

it('gates every CSS effect behind visibility, the shared duration and both accessibility policies', () => {
  expect(css).toContain('@media (prefers-reduced-motion: no-preference) and (forced-colors: none)');
  const animationRules = [...css.matchAll(/([^{}]+)\{\s*(?:opacity: 1;\s*)?animation:/g)];
  expect(animationRules.length).toBe(7);
  for (const [, selector] of animationRules) {
    expect(selector).toContain(':root:not([data-motion="reduced"])');
    if (!selector.includes('.work-tree-head')) expect(selector).toContain('[data-motion-playing="true"]');
  }
  expect(css).toContain('var(--dur-shine)'); expect(css).toContain('var(--dur-skeleton)');
  expect(css).toContain('animation: sidebar-drag-label-reveal var(--panel-duration) step-end');
  expect(ui).toContain('aria-label={props.label}');
  expect(ui).not.toContain('aria-valuenow');
});
