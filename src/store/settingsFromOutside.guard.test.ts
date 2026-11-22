import { afterEach, describe, expect, it, vi } from 'vitest';

// Overnight 11-13 S6: a jump into a Settings section from another page is one
// guarded navigation. An unsaved draft answered Keep editing leaves no section
// request pending (a later Settings open would jump to it), and `opened`
// says the jump was refused, so ui.navigate doesn't report a landing.
vi.hoisted(() => {
  const values = new Map<string, string>();
  vi.stubGlobal('localStorage', { getItem: (k: string) => values.get(k) ?? null, setItem: (k: string, v: string) => values.set(k, v), removeItem: (k: string) => values.delete(k) });
});
vi.mock('../nativeEvents', () => ({ listen: () => Promise.resolve(() => {}) }));
// runtime.ts builds resources at load, which Solid's server build (Node) refuses.
vi.mock('solid-js', async original => ({ ...(await original<typeof import('solid-js')>()), createResource: () => [Object.assign(() => undefined, { loading: false, error: undefined, state: 'unresolved' }), { refetch: () => {}, mutate: () => {} }] }));
// The stub api answers null, which the operator switch read can't adopt.
vi.mock('../operatorSwitch', async original => ({ ...(await original<typeof import('../operatorSwitch')>()), adoptOperatorSwitch: async () => null }));
vi.mock('../api', () => ({ api: new Proxy({}, { get: () => vi.fn(async () => null) }) }));
import { activePage, openSettingsFromOutside, setActivePage, setSettingsSectionRequest, settingsSectionRequest } from './runtime';
import { registerNavigationGuard } from './navigationGuard';

const release: (() => void)[] = [];
afterEach(() => { release.splice(0).forEach(off => off()); setSettingsSectionRequest(null); setActivePage('today'); });

describe('openSettingsFromOutside with an unsaved-draft guard', () => {
  it('a refused jump leaves no pending section request and opened resolves false', async () => {
    let answer!: (ok: boolean) => void;
    release.push(registerNavigationGuard(() => new Promise<boolean>(r => { answer = r; })));
    const { opened, direct } = openSettingsFromOutside('usage');
    expect(direct).toBe(false);
    expect(settingsSectionRequest()).toBeNull();
    answer(false);
    expect(await opened).toBe(false);
    expect(settingsSectionRequest()).toBeNull();
    expect(activePage()).toBe('today');
  });

  it('a synchronous refusal does the same', async () => {
    release.push(registerNavigationGuard(() => false));
    expect(await openSettingsFromOutside('usage', 'Overnight cap').opened).toBe(false);
    expect(settingsSectionRequest()).toBeNull();
    expect(activePage()).toBe('today');
  });

  it('an accepted jump opens Settings on the section and opened resolves true', async () => {
    let answer!: (ok: boolean) => void;
    release.push(registerNavigationGuard(() => new Promise<boolean>(r => { answer = r; })));
    const { opened } = openSettingsFromOutside('usage', 'Overnight cap');
    answer(true);
    expect(await opened).toBe(true);
    expect(activePage()).toBe('settings');
    expect(settingsSectionRequest()).toMatchObject({ id: 'usage', control: 'Overnight cap' });
  });

  it('already in Settings with a guard registered: no prompt, the request goes to Settings', async () => {
    setActivePage('settings');
    const guard = vi.fn(() => false);
    release.push(registerNavigationGuard(guard));
    const { opened } = openSettingsFromOutside('usage', 'Overnight cap');
    expect(guard).not.toHaveBeenCalled();
    expect(settingsSectionRequest()).toMatchObject({ id: 'usage', control: 'Overnight cap' });
    expect(await opened).toBe(true);
  });

  it('a jump replaced by another navigation during the prompt resolves opened false and leaves no request', async () => {
    const answers: ((ok: boolean) => void)[] = [];
    release.push(registerNavigationGuard(() => new Promise<boolean>(r => { answers.push(r); })));
    const { opened } = openSettingsFromOutside('usage');
    setActivePage('tasks');
    answers.forEach(answer => answer(true));
    expect(await opened).toBe(false);
    expect(settingsSectionRequest()).toBeNull();
    await Promise.resolve();
    expect(activePage()).toBe('tasks');
  });

  it('with no guard it opens at once', async () => {
    const { opened } = openSettingsFromOutside('usage');
    expect(activePage()).toBe('settings');
    expect(settingsSectionRequest()).toMatchObject({ id: 'usage' });
    expect(await opened).toBe(true);
  });
});
