import { beforeEach, describe, expect, it, vi } from 'vitest';

// Per-machine prefs (PR A4): each has one signal and one setter, the only
// writer of its localStorage key. The signals read their key once at module
// load, so every test loads a fresh copy over a stubbed store.
const saved = vi.hoisted(() => {
  const map = new Map<string, string>();
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => map.get(key) ?? null,
    setItem: (key: string, value: string) => { map.set(key, value); },
    removeItem: (key: string) => { map.delete(key); },
  });
  return map;
});

const load = async () => { vi.resetModules(); return import('./prefs'); };

beforeEach(() => saved.clear());

describe('palette recents', () => {
  it('puts the newest first and moves a repeat to the front without duplicating it', async () => {
    const p = await load();
    p.recordPaletteRecent('a');
    p.recordPaletteRecent('b');
    p.recordPaletteRecent('a');
    expect(p.paletteRecents()).toEqual(['a', 'b']);
    expect(JSON.parse(saved.get('paletteRecents')!)).toEqual(['a', 'b']);
  });

  it('keeps only the newest 24', async () => {
    const p = await load();
    for (let i = 0; i < 30; i++) p.recordPaletteRecent(`id${i}`);
    expect(p.paletteRecents()).toHaveLength(24);
    expect(p.paletteRecents()[0]).toBe('id29');
    expect(p.paletteRecents()[23]).toBe('id6');
  });

  it('reads a saved list back, and drops non-strings or unparseable text', async () => {
    saved.set('paletteRecents', JSON.stringify(['x', 3, null, 'y']));
    expect((await load()).paletteRecents()).toEqual(['x', 'y']);
    saved.set('paletteRecents', '{"not":"a list"}');
    expect((await load()).paletteRecents()).toEqual([]);
    saved.set('paletteRecents', 'not json');
    expect((await load()).paletteRecents()).toEqual([]);
  });
});

describe('sidebar width', () => {
  it('clamps to the min and max and rounds', async () => {
    const p = await load();
    expect(p.clampRailWidth(50)).toBe(p.RAIL_WIDTH_MIN);
    expect(p.clampRailWidth(9000)).toBe(p.RAIL_WIDTH_MAX);
    expect(p.clampRailWidth(250.6)).toBe(251);
  });

  it('does not save while dragging; persistRailWidth saves once', async () => {
    const p = await load();
    p.setRailWidth(300);
    expect(p.railWidth()).toBe(300);
    expect(saved.has('hollow.railWidth')).toBe(false);
    p.persistRailWidth();
    expect(saved.get('hollow.railWidth')).toBe('300');
  });

  it('forgets the key when the width is the default or reset to null', async () => {
    const p = await load();
    p.setRailWidth(300);
    p.persistRailWidth();
    p.setRailWidth(p.RAIL_WIDTH_DEFAULT);
    p.persistRailWidth();
    expect(saved.has('hollow.railWidth')).toBe(false);
    p.setRailWidth(300);
    p.persistRailWidth();
    p.setRailWidth(null);
    expect(p.railWidth()).toBeNull();
    p.persistRailWidth();
    expect(saved.has('hollow.railWidth')).toBe(false);
  });

  it('clamps an out-of-range saved width and ignores junk or non-positive ones', async () => {
    saved.set('hollow.railWidth', '9999');
    expect((await load()).railWidth()).toBe(420);
    saved.set('hollow.railWidth', '12');
    expect((await load()).railWidth()).toBe(200);
    saved.set('hollow.railWidth', 'wide');
    expect((await load()).railWidth()).toBeNull();
    saved.set('hollow.railWidth', '-5');
    expect((await load()).railWidth()).toBeNull();
  });
});

describe('folded sidebar sections', () => {
  it('toggles a section in and out, and removes the key when none are folded', async () => {
    const p = await load();
    p.toggleRailSection('projects');
    expect(p.railFoldedSections()).toEqual(['projects']);
    expect(saved.get('hollow.railSections.v1')).toBe('["projects"]');
    p.toggleRailSection('projects');
    expect(p.railFoldedSections()).toEqual([]);
    expect(saved.has('hollow.railSections.v1')).toBe(false);
  });
});

describe('notification filter', () => {
  it('saves a choice and falls back to all for an unknown saved one', async () => {
    const p = await load();
    p.setNotifFilter('routines');
    expect(saved.get('notifFilter')).toBe('routines');
    expect((await load()).notifFilter()).toBe('routines');
    saved.set('notifFilter', 'bogus');
    expect((await load()).notifFilter()).toBe('all');
  });
});

describe('composer send key', () => {
  it('is on by default and stores only the off state', async () => {
    const p = await load();
    expect(p.enterSends()).toBe(true);
    p.setEnterSends(false);
    expect(saved.get('hollow.chat.enterSends')).toBe('0');
    expect((await load()).enterSends()).toBe(false);
    p.setEnterSends(true);
    expect(saved.has('hollow.chat.enterSends')).toBe(false);
  });
});

describe('opt-ins', () => {
  it('stores quota opt-in as 1 and removes it when off', async () => {
    const p = await load();
    p.setQuotaOptIn(true);
    expect(saved.get('claudeQuotaOptIn')).toBe('1');
    expect((await load()).quotaOptIn()).toBe(true);
    p.setQuotaOptIn(false);
    expect(saved.has('claudeQuotaOptIn')).toBe(false);
  });

  it('keeps the backup flag as the words true and false', async () => {
    const p = await load();
    p.setBackupManagedElsewhere(true);
    expect(saved.get('backupManagedElsewhere.v1')).toBe('true');
    expect((await load()).backupManagedElsewhere()).toBe(true);
    p.setBackupManagedElsewhere(false);
    expect(saved.get('backupManagedElsewhere.v1')).toBe('false');
    expect((await load()).backupManagedElsewhere()).toBe(false);
  });
});

describe('swipe to go back and forward (parity 4.6)', () => {
  it('is on by default, off saves the one key, on again removes it', async () => {
    const p = await load();
    expect(p.swipeNav()).toBe(true);
    p.setSwipeNav(false);
    expect(saved.get('hollow.nav.swipe')).toBe('0');
    expect((await load()).swipeNav()).toBe(false);
    (await load()).setSwipeNav(true);
    expect(saved.has('hollow.nav.swipe')).toBe(false);
  });
});
