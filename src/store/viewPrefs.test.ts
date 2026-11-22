import { beforeEach, describe, expect, it, vi } from 'vitest';
import css from '../styles/build-changes.css?raw';
import browser from './browser.ts?raw';
import app from '../App.tsx?raw';

const env = vi.hoisted(() => {
  const map = new Map<string, string>();
  const dataset: Record<string, string> = {};
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => map.get(key) ?? null,
    setItem: (key: string, value: string) => { map.set(key, value); },
    removeItem: (key: string) => { map.delete(key); },
  });
  vi.stubGlobal('document', { documentElement: { dataset } });
  return { map, dataset };
});
const load = async () => { vi.resetModules(); return import('./viewPrefs'); };
beforeEach(() => { env.map.clear(); for (const k of Object.keys(env.dataset)) delete env.dataset[k]; });

describe('diff view preferences', () => {
  it('wrap and line numbers are on by default and set no attribute', async () => {
    const p = await load();
    expect(p.diffWrap()).toBe(true);
    expect(p.diffNumbers()).toBe(true);
    expect(env.dataset.diffWrap).toBeUndefined();
  });
  it('turning one off saves it, sets the root attribute the stylesheet reads, and survives a reload', async () => {
    const p = await load();
    p.setDiffWrap(false);
    p.setDiffNumbers(false);
    expect(env.dataset.diffWrap).toBe('off');
    expect(env.dataset.diffNumbers).toBe('off');
    expect(env.map.get('hollow.diff.wrap')).toBe('0');
    const again = await load();
    expect(again.diffWrap()).toBe(false);
    expect(env.dataset.diffWrap).toBe('off');
    again.setDiffWrap(true);
    expect(env.map.has('hollow.diff.wrap')).toBe(false);
    expect(env.dataset.diffWrap).toBeUndefined();
  });
  it('the stylesheet answers both attributes', () => {
    expect(css).toContain(":root[data-diff-wrap='off'] .bc-code");
    expect(css).toContain(":root[data-diff-numbers='off'] .bc-no");
  });
});

describe('where a web link opens', () => {
  it('defaults to the Preview pane, saves the browser choice and ignores junk', async () => {
    const p = await load();
    expect(p.linkTarget()).toBe('preview');
    p.setLinkTarget('browser');
    expect(env.map.get('hollow.links.target')).toBe('browser');
    expect((await load()).linkTarget()).toBe('browser');
    env.map.set('hollow.links.target', 'elsewhere');
    expect((await load()).linkTarget()).toBe('preview');
  });
  it('openWebLink reverses the default with ⌘ or Shift, and never sends a non-web link to the pane', () => {
    expect(browser).toContain("linkTarget() === 'browser' ? !reversed : reversed");
    expect(browser).toContain('!isWebUrl(url) ||');
  });
  it('the preferences load with the app, not only with Settings', () => {
    expect(app).toContain("import './store/viewPrefs';");
    expect(app).toContain("import './store/fonts';");
  });
});
